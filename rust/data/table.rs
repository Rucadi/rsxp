// SPDX-License-Identifier: GPL-2.0-or-later
// Ported from src/etc/table.cpp, Copyright (C) 2013–2021 Amaryllis Kulla.
//! RGSS signed 16-bit tables and their Ruby Marshal payload format.
use std::{error::Error, fmt};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidTable;
impl fmt::Display for InvalidTable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Marshal: Table: bad file format or dimensions")
    }
}
impl Error for InvalidTable {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    size: [usize; 3],
    data: Vec<i16>,
}

impl Table {
    pub fn new(x: usize, y: usize, z: usize) -> Result<Self, InvalidTable> {
        let count = x
            .checked_mul(y)
            .and_then(|n| n.checked_mul(z))
            .ok_or(InvalidTable)?;
        // The on-disk format uses signed 32-bit sizes. Bound allocation as well.
        if [x, y, z, count].iter().any(|&n| n > i32::MAX as usize) {
            return Err(InvalidTable);
        }
        let mut data = Vec::new();
        data.try_reserve_exact(count).map_err(|_| InvalidTable)?;
        data.resize(count, 0);
        Ok(Self {
            size: [x, y, z],
            data,
        })
    }

    pub fn dimensions(&self) -> [usize; 3] {
        self.size
    }

    fn index(&self, x: usize, y: usize, z: usize) -> Option<usize> {
        let [xs, ys, zs] = self.size;
        (x < xs && y < ys && z < zs).then(|| (z * ys + y) * xs + x)
    }

    pub fn get(&self, x: usize, y: usize, z: usize) -> Option<i16> {
        self.index(x, y, z).map(|i| self.data[i])
    }

    /// Out-of-bounds writes are ignored, as in the original engine.
    pub fn set(&mut self, x: usize, y: usize, z: usize, value: i16) {
        if let Some(i) = self.index(x, y, z) {
            self.data[i] = value;
        }
    }

    pub fn resize(&mut self, x: usize, y: usize, z: usize) -> Result<(), InvalidTable> {
        let mut resized = Self::new(x, y, z)?;
        for k in 0..z.min(self.size[2]) {
            for j in 0..y.min(self.size[1]) {
                for i in 0..x.min(self.size[0]) {
                    resized.set(i, j, k, self.get(i, j, k).unwrap());
                }
            }
        }
        *self = resized;
        Ok(())
    }

    pub fn serialize(&self) -> Vec<u8> {
        let [x, y, z] = self.size;
        let dim = if z > 1 {
            3
        } else if y > 1 {
            2
        } else {
            1
        };
        let mut bytes = Vec::with_capacity(20 + self.data.len() * 2);
        for n in [dim, x, y, z, self.data.len()] {
            bytes.extend_from_slice(&(n as i32).to_le_bytes());
        }
        for n in &self.data {
            bytes.extend_from_slice(&n.to_le_bytes());
        }
        bytes
    }

    pub fn deserialize(bytes: &[u8]) -> Result<Self, InvalidTable> {
        if bytes.len() < 20 {
            return Err(InvalidTable);
        }
        let mut header = [0usize; 5];
        for (i, value) in header.iter_mut().enumerate() {
            *value = usize::try_from(i32::from_le_bytes(
                bytes[i * 4..i * 4 + 4].try_into().unwrap(),
            ))
            .map_err(|_| InvalidTable)?;
        }
        let [dim, x, y, z, count] = header;
        if !(1..=3).contains(&dim)
            || x.checked_mul(y).and_then(|n| n.checked_mul(z)) != Some(count)
            || count.checked_mul(2).and_then(|n| n.checked_add(20)) != Some(bytes.len())
        {
            return Err(InvalidTable);
        }
        let mut table = Self::new(x, y, z)?;
        for (dst, src) in table.data.iter_mut().zip(bytes[20..].chunks_exact(2)) {
            *dst = i16::from_le_bytes(src.try_into().unwrap());
        }
        Ok(table)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn marshal_fixture_and_resize() {
        let bytes = [
            1, 0, 0, 0, 2, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 0xff, 0xff, 0x34, 0x12,
        ];
        let mut t = Table::deserialize(&bytes).unwrap();
        assert_eq!(t.get(0, 0, 0), Some(-1));
        assert_eq!(t.get(1, 0, 0), Some(0x1234));
        assert_eq!(t.serialize(), bytes);
        t.resize(3, 2, 2).unwrap();
        assert_eq!(t.get(1, 0, 0), Some(0x1234));
        assert_eq!(t.get(2, 1, 1), Some(0));
        t.set(1, 1, 1, -123);
        assert_eq!(Table::deserialize(&t.serialize()).unwrap(), t);
        t.resize(1, 1, 1).unwrap();
        assert_eq!(t.get(0, 0, 0), Some(-1));
        assert_eq!(t.get(1, 0, 0), None);
    }
    #[test]
    fn rejects_corrupt_payloads() {
        let t = Table::new(2, 3, 4).unwrap().serialize();
        for len in 0..t.len() {
            assert!(Table::deserialize(&t[..len]).is_err());
        }
        for offset in [0, 4, 8, 12, 16] {
            let mut bad = t.clone();
            bad[offset..offset + 4].copy_from_slice(&(-1i32).to_le_bytes());
            assert!(Table::deserialize(&bad).is_err());
        }
        assert!(Table::new(usize::MAX, 2, 2).is_err());
        assert!(Table::new(0, usize::MAX, 0).is_err());
    }
}
