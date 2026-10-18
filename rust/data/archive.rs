// SPDX-License-Identifier: GPL-2.0-or-later
// Ported from src/crypto/rgssad.cpp, Copyright (C) 2014–2021 Amaryllis Kulla.
//! Bounded readers for RGSS game archives: RGSSAD/RGSS2A (version 1) and RGSS3A
//! (version 3).
//!
//! An archive begins with the 7-byte signature `RGSSAD\0` and a version byte.
//! [`Archive::parse`] reads the encrypted index into [`Entry`] records while
//! borrowing the archive bytes; [`Archive::read`] then decrypts a byte range of
//! one entry. Filenames stay raw bytes because older games may use a non-UTF-8
//! encoding, and `\` is normalised to `/`; empty or NUL-containing names are
//! rejected.
//!
//! Both versions derive a wrapping 32-bit LCG key stream from `advance`
//! (`key = key * 7 + 3`). Version 1 starts from `0xDEADCAFE` and XORs the entry
//! length, each filename byte and the data size with successive key values; the
//! file data is XORed with the stream advanced once per 4 bytes. Version 3
//! starts from the first index word as `word * 9 + 3`; each index record is
//! `(offset, size, data_key, name_len)`, all XORed with that key, an `offset` of
//! zero terminates the index, names are XORed with the key's bytes by position
//! `i % 4`, and entries must not overlap the index. `jump` skips `n` LCG steps in
//! logarithmic time so unaligned `read` offsets are cheap.
//!
//! All cursor movement goes through `Reader`, which uses checked arithmetic and
//! reports truncation/overflow as [`InvalidArchive`]; `read` clamps the request
//! to the remaining entry length and rejects offsets past the end.
use std::{error::Error, fmt, ops::Range};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidArchive(pub &'static str);
impl fmt::Display for InvalidArchive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl Error for InvalidArchive {}

#[derive(Debug)]
pub struct Entry {
    pub name: Vec<u8>,
    range: Range<usize>,
    key: u32,
}
impl Entry {
    pub fn len(&self) -> usize {
        self.range.len()
    }
    pub fn is_empty(&self) -> bool {
        self.range.is_empty()
    }
}

pub struct Archive<'a> {
    bytes: &'a [u8],
    entries: Vec<Entry>,
    pub version: u8,
}

fn advance(key: &mut u32) -> u32 {
    let old = *key;
    *key = key.wrapping_mul(7).wrapping_add(3);
    old
}

/// Jump ahead in the wrapping 32-bit LCG in logarithmic time.
fn jump(mut key: u32, mut count: usize) -> u32 {
    let (mut mul, mut add) = (7u32, 3u32);
    while count > 0 {
        if count & 1 != 0 {
            key = key.wrapping_mul(mul).wrapping_add(add);
        }
        add = add.wrapping_mul(mul.wrapping_add(1));
        mul = mul.wrapping_mul(mul);
        count >>= 1;
    }
    key
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], InvalidArchive> {
        let end = self
            .pos
            .checked_add(count)
            .ok_or(InvalidArchive("offset overflow"))?;
        let data = self
            .bytes
            .get(self.pos..end)
            .ok_or(InvalidArchive("truncated archive"))?;
        self.pos = end;
        Ok(data)
    }
    fn word(&mut self) -> Result<u32, InvalidArchive> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
}

fn normalize(mut name: Vec<u8>) -> Result<Vec<u8>, InvalidArchive> {
    if name.is_empty() || name.contains(&0) {
        return Err(InvalidArchive("invalid filename"));
    }
    for byte in &mut name {
        if *byte == b'\\' {
            *byte = b'/';
        }
    }
    Ok(name)
}

impl<'a> Archive<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, InvalidArchive> {
        if bytes.get(..7) != Some(b"RGSSAD\0") {
            return Err(InvalidArchive("invalid RGSS archive signature"));
        }
        let version = *bytes.get(7).ok_or(InvalidArchive("missing version"))?;
        let mut reader = Reader { bytes, pos: 8 };
        let mut entries = Vec::new();
        match version {
            1 => {
                let mut key = 0xDEADCAFE;
                while reader.pos < bytes.len() {
                    let len = (reader.word()? ^ advance(&mut key)) as usize;
                    let name = reader
                        .take(len)?
                        .iter()
                        .map(|&b| b ^ advance(&mut key) as u8)
                        .collect();
                    let name = normalize(name)?;
                    let size = (reader.word()? ^ advance(&mut key)) as usize;
                    let start = reader.pos;
                    reader.take(size)?;
                    entries.push(Entry {
                        name,
                        range: start..reader.pos,
                        key,
                    });
                }
            }
            3 => {
                let key = reader.word()?.wrapping_mul(9).wrapping_add(3);
                loop {
                    let offset = (reader.word()? ^ key) as usize;
                    if offset == 0 {
                        break;
                    }
                    let size = (reader.word()? ^ key) as usize;
                    let data_key = reader.word()? ^ key;
                    let len = (reader.word()? ^ key) as usize;
                    let name = reader
                        .take(len)?
                        .iter()
                        .enumerate()
                        .map(|(i, b)| b ^ key.to_le_bytes()[i % 4])
                        .collect();
                    let end = offset
                        .checked_add(size)
                        .ok_or(InvalidArchive("offset overflow"))?;
                    if end > bytes.len() {
                        return Err(InvalidArchive("entry outside archive"));
                    }
                    entries.push(Entry {
                        name: normalize(name)?,
                        range: offset..end,
                        key: data_key,
                    });
                }
                if entries.iter().any(|e| e.range.start < reader.pos) {
                    return Err(InvalidArchive("entry overlaps archive index"));
                }
            }
            _ => return Err(InvalidArchive("unsupported RGSS archive version")),
        }
        Ok(Self {
            bytes,
            entries,
            version,
        })
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Decrypt a byte range, including unaligned reads and reads ending at EOF.
    /// An index prevents callers from supplying an entry from another archive.
    pub fn read(
        &self,
        index: usize,
        offset: usize,
        length: usize,
    ) -> Result<Vec<u8>, InvalidArchive> {
        let entry = self
            .entries
            .get(index)
            .ok_or(InvalidArchive("unknown entry"))?;
        if offset > entry.len() {
            return Err(InvalidArchive("seek past entry"));
        }
        let length = length.min(entry.len() - offset);
        let mut key = jump(entry.key, offset / 4);
        let mut out =
            self.bytes[entry.range.start + offset..entry.range.start + offset + length].to_vec();
        for (i, byte) in out.iter_mut().enumerate() {
            let lane = (offset + i) % 4;
            *byte ^= key.to_le_bytes()[lane];
            if lane == 3 {
                advance(&mut key);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn encrypted(data: &[u8], mut key: u32) -> Vec<u8> {
        data.chunks(4)
            .flat_map(|chunk| {
                let bytes = advance(&mut key).to_le_bytes();
                chunk
                    .iter()
                    .zip(bytes)
                    .map(|(b, k)| b ^ k)
                    .collect::<Vec<_>>()
            })
            .collect()
    }
    fn fixture(version: u8) -> Vec<u8> {
        let mut out = b"RGSSAD\0".to_vec();
        out.push(version);
        let name = b"Data\\test.bin";
        let data = b"hello world!\0\xff";
        if version == 1 {
            let mut key = 0xDEADCAFE;
            out.extend_from_slice(&((name.len() as u32) ^ advance(&mut key)).to_le_bytes());
            for b in name {
                out.push(b ^ advance(&mut key) as u8);
            }
            out.extend_from_slice(&((data.len() as u32) ^ advance(&mut key)).to_le_bytes());
            out.extend(encrypted(data, key));
        } else {
            out.extend_from_slice(&123u32.to_le_bytes());
            let key = 123u32 * 9 + 3;
            let offset = 12 + 16 + name.len() as u32 + 4;
            for n in [offset, data.len() as u32, 0x12345678, name.len() as u32] {
                out.extend_from_slice(&(n ^ key).to_le_bytes());
            }
            out.extend(
                name.iter()
                    .enumerate()
                    .map(|(i, b)| b ^ key.to_le_bytes()[i % 4]),
            );
            out.extend_from_slice(&key.to_le_bytes());
            out.extend(encrypted(data, 0x12345678));
        }
        out
    }
    #[test]
    fn both_formats_and_every_unaligned_read() {
        let plain = b"hello world!\0\xff";
        for version in [1, 3] {
            let bytes = fixture(version);
            let arch = Archive::parse(&bytes).unwrap();
            assert_eq!(arch.entries()[0].name, b"Data/test.bin");
            for offset in 0..=plain.len() {
                for len in 0..=plain.len() + 3 {
                    assert_eq!(
                        arch.read(0, offset, len).unwrap(),
                        plain[offset..(offset + len).min(plain.len())]
                    );
                }
            }
            assert!(arch.read(0, plain.len() + 1, 0).is_err());
            assert!(arch.read(1, 0, 1).is_err());
        }
    }
    #[test]
    fn rejects_truncation_and_bad_headers() {
        for version in [1, 3] {
            let bytes = fixture(version);
            for end in 0..bytes.len() {
                // A header-only v1 archive is a valid empty archive.
                if version == 1 && end == 8 {
                    continue;
                }
                assert!(
                    Archive::parse(&bytes[..end]).is_err(),
                    "v{version}, length {end}"
                );
            }
        }
        assert!(Archive::parse(b"RGSSAD\0\x02").is_err());
        assert!(Archive::parse(b"ABCDEFG\x01").is_err());
    }
    #[test]
    fn lcg_jump_matches_iteration() {
        let mut key = 0xDEADCAFE;
        for n in 0..10000 {
            assert_eq!(jump(0xDEADCAFE, n), key);
            advance(&mut key);
        }
    }
}
