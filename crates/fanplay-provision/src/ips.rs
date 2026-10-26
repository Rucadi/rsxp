//! Classic IPS decoder. Never modifies the base ROM.
const LIMIT: usize = 512 * 1024 * 1024;

pub fn apply(base: &[u8], patch: &[u8]) -> Result<Vec<u8>, String> {
    if !patch.starts_with(b"PATCH") || base.len() > LIMIT {
        return Err("Invalid IPS header or oversized base ROM".into());
    }
    let mut out = base.to_vec();
    let mut pos = 5;
    fn take<'a>(data: &'a [u8], pos: &mut usize, n: usize) -> Result<&'a [u8], String> {
        let bytes = data.get(*pos..*pos + n).ok_or("Truncated IPS record")?;
        *pos += n;
        Ok(bytes)
    }
    fn number(bytes: &[u8]) -> usize {
        bytes.iter().fold(0, |n, b| (n << 8) | usize::from(*b))
    }
    loop {
        let offset = take(patch, &mut pos, 3)?;
        if offset == b"EOF" {
            match patch.len() - pos {
                0 => {}
                3 => out.resize(number(&patch[pos..]), 0),
                _ => return Err("Invalid IPS trailer".into()),
            }
            return Ok(out);
        }
        let offset = number(offset);
        let size = number(take(patch, &mut pos, 2)?);
        let (len, value) = if size == 0 {
            let len = number(take(patch, &mut pos, 2)?);
            if len == 0 {
                return Err("Empty IPS RLE record".into());
            }
            (len, Some(take(patch, &mut pos, 1)?[0]))
        } else {
            (size, None)
        };
        let end = offset
            .checked_add(len)
            .filter(|n| *n <= LIMIT)
            .ok_or("IPS output too large")?;
        if out.len() < end {
            out.resize(end, 0);
        }
        if let Some(value) = value {
            out[offset..end].fill(value);
        } else {
            out[offset..end].copy_from_slice(take(patch, &mut pos, len)?);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn literal_rle_growth_and_truncation() {
        assert_eq!(
            apply(b"abcdef", b"PATCH\0\0\x01\0\x02XY\0\0\x06\0\0\0\x03ZEOF").unwrap(),
            b"aXYdefZZZ"
        );
        assert_eq!(apply(b"abcdef", b"PATCHEOF\0\0\x03").unwrap(), b"abc");
    }
    #[test]
    fn rejects_incomplete_and_invalid_records() {
        for patch in [
            b"".as_slice(),
            b"PATCH",
            b"PATCH\0\0\x01\0\x02X",
            b"PATCHEOFx",
            b"PATCH\0\0\0\0\0\0\0xEOF",
        ] {
            assert!(apply(b"base", patch).is_err());
        }
        let patch = b"PATCH\0\0\x01\0\x02XYEOF";
        for n in 0..patch.len() {
            assert!(apply(b"abc", &patch[..n]).is_err());
        }
    }
}
