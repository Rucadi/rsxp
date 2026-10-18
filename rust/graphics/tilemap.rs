// SPDX-License-Identifier: GPL-2.0-or-later
//! Native tile cells and ground/priority quad geometry. Source cells are
//! deduplicated across tiles and animation frames. Geometry is ordered before
//! spatial binning, so transparent ground layers and priority tiles preserve
//! their exact relationship to characters and other scene elements.
use crate::graphics::{Bitmap, Rect};
use std::collections::HashMap;

type CellKey = (u64, i32, i32, u32, u32);
struct Cell {
    revision: u64,
    bitmap: Bitmap,
    id: u64,
    used: u64,
}

#[derive(Default)]
pub struct TileAtlas {
    cells: HashMap<CellKey, Cell>,
    next_id: u64,
    epoch: u64,
}
impl TileAtlas {
    pub fn begin_frame(&mut self) {
        self.epoch += 1;
        // Bound CPU-side cell storage without evicting the currently visible
        // scene. GPU slots have their own frame-based reclamation.
        if self.cells.len() > 4096 {
            let epoch = self.epoch;
            self.cells.retain(|_, cell| epoch - cell.used <= 2);
        }
    }
    pub fn cell(
        &mut self,
        source_id: u64,
        source: &Bitmap,
        area: Rect,
    ) -> Result<(u64, &Bitmap), String> {
        let key = (source_id, area.x, area.y, area.width, area.height);
        if !self.cells.contains_key(&key) {
            self.next_id += 1;
            // Reserve a namespace below the compositor's snapshot/overlay IDs.
            let id = u64::MAX - 1024 - self.next_id;
            let bitmap = Bitmap::new(area.width, area.height).map_err(|e| e.to_string())?;
            self.cells.insert(
                key,
                Cell {
                    revision: 0,
                    bitmap,
                    id,
                    used: self.epoch,
                },
            );
        }
        let cell = self.cells.get_mut(&key).unwrap();
        if cell.revision != source.revision() {
            cell.bitmap.clear(crate::graphics::Color::default());
            cell.bitmap
                .stretch_blt(cell.bitmap.rect(), source, area, 255);
            cell.revision = source.revision();
        }
        cell.used = self.epoch;
        Ok((cell.id, &cell.bitmap))
    }
}

#[derive(Clone)]
pub struct TileQuad {
    pub z: i32,
    pub params: [[u32; 4]; 16],
}
impl TileQuad {
    pub const WORDS: usize = 24;
    fn packed(&self) -> [u32; Self::WORDS] {
        let p = &self.params;
        let mut words = [0; Self::WORDS];
        words[0..4].copy_from_slice(&p[1]); // clipped bounds
        words[4] = (f32::from_bits(p[5][0]) as i32) as u32;
        words[5] = (f32::from_bits(p[5][1]) as i32) as u32;
        words[6..8].copy_from_slice(&p[13][0..2]); // destination size
        words[8] = p[0][2]; // atlas cell
        words[9..11].copy_from_slice(&p[2][2..4]); // cell size
        words[11] = p[12][0]; // integer opacity
        words[12] = p[0][1]; // viewport buffer
        words[13] = p[2][0]; // frame width
        words[14] = p[10][0]; // blend
        words[15] = p[10][2] | (p[10][1] << 1) | (u32::from(p[11][3] == 1f32.to_bits()) << 2);
        words[16..20].copy_from_slice(&p[6]); // color
        words[20..24].copy_from_slice(&p[7]); // tone
        words
    }
}
#[derive(Debug, PartialEq, Eq)]
pub struct TileLayer {
    pub z: i32,
    pub first: usize,
    pub count: usize,
}
pub struct TileGeometry {
    pub layers: Vec<TileLayer>,
    pub bounds: [u32; 4],
    pub bin_width: u32,
    pub bin_origin: [u32; 2],
    pub bin_count: u32,
    pub base: u32,
}
impl TileGeometry {
    pub const BIN_SIZE: u32 = 32;
    /// Encode tile quads plus per-bin ordered quad indices. Adjacent z layers
    /// share one dispatch; an intervening non-tile scene element ends the run.
    pub fn append(width: u32, height: u32, quads: &[TileQuad], data: &mut Vec<u32>) -> Self {
        let bounds = quads.iter().fold([width, height, 0, 0], |b, q| {
            let [x, y, x1, y1] = q.params[1];
            [b[0].min(x), b[1].min(y), b[2].max(x1), b[3].max(y1)]
        });
        let bin_origin = [bounds[0] / Self::BIN_SIZE, bounds[1] / Self::BIN_SIZE];
        let bin_width = bounds[2].saturating_sub(1) / Self::BIN_SIZE - bin_origin[0] + 1;
        let bin_height = bounds[3].saturating_sub(1) / Self::BIN_SIZE - bin_origin[1] + 1;
        // Restrict compressed bins to the occupied run, not the full screen.
        // Small priority runs should not allocate a panoramic-sized index.
        let mut counts = vec![0u32; (bin_width * bin_height) as usize];
        let mut layers: Vec<TileLayer> = Vec::new();
        let bins_for = |quad: &TileQuad| {
            let [x, y, x1, y1] = quad.params[1];
            (
                x / Self::BIN_SIZE,
                y / Self::BIN_SIZE,
                (x1.saturating_sub(1)) / Self::BIN_SIZE,
                (y1.saturating_sub(1)) / Self::BIN_SIZE,
            )
        };
        for (index, quad) in quads.iter().enumerate() {
            if let Some(layer) = layers.last_mut().filter(|layer| layer.z == quad.z) {
                layer.count += 1;
            } else {
                layers.push(TileLayer {
                    z: quad.z,
                    first: index,
                    count: 1,
                });
            }
            let [x, y, x1, y1] = quad.params[1];
            if x1 <= x || y1 <= y {
                continue;
            }
            let (bx, by, bx1, by1) = bins_for(quad);
            for row in by..=by1 {
                for col in bx..=bx1 {
                    counts[((row - bin_origin[1]) * bin_width + col - bin_origin[0]) as usize] += 1;
                }
            }
        }
        let base = data.len() as u32;
        let param_base = base + counts.len() as u32 * 2 + counts.iter().sum::<u32>();
        data.resize(param_base as usize + quads.len() * TileQuad::WORDS, 0);
        let mut cursors = Vec::with_capacity(counts.len());
        let mut offset = base + counts.len() as u32;
        for (index, &count) in counts.iter().enumerate() {
            data[base as usize + index] = offset;
            data[offset as usize] = count;
            cursors.push(offset as usize + 1);
            offset += 1 + count;
        }
        for (index, quad) in quads.iter().enumerate() {
            let param = param_base + index as u32 * TileQuad::WORDS as u32;
            for (word, value) in quad.packed().iter().enumerate() {
                data[param as usize + word] = *value;
            }
            let [x, y, x1, y1] = quad.params[1];
            if x1 <= x || y1 <= y {
                continue;
            }
            let (bx, by, bx1, by1) = bins_for(quad);
            for row in by..=by1 {
                for col in bx..=bx1 {
                    let cursor = &mut cursors
                        [((row - bin_origin[1]) * bin_width + col - bin_origin[0]) as usize];
                    data[*cursor] = param;
                    *cursor += 1;
                }
            }
        }
        Self {
            layers,
            bounds,
            bin_width,
            bin_origin,
            bin_count: bin_width * bin_height,
            base,
        }
    }
}

/// Retains ground/priority topology and spatial bins when only animation cell
/// selection or atlas pixels change. Geometry is rebuilt for scroll, clipping,
/// priority or membership changes.
#[derive(Default)]
pub struct TileGeometryCache {
    extent: (u32, u32),
    keys: Vec<(i32, [u32; 4])>,
    prefix: Vec<u32>,
    relocations: Vec<usize>,
    layout: Option<TileGeometry>,
}
pub struct TileDrawGeometry {
    pub bounds: [u32; 4],
    pub bin_width: u32,
    pub bin_origin: [u32; 2],
    pub base: u32,
    pub layer_count: usize,
}
impl TileGeometryCache {
    pub fn append(
        &mut self,
        width: u32,
        height: u32,
        quads: &[TileQuad],
        data: &mut Vec<u32>,
    ) -> TileDrawGeometry {
        let changed = self.extent != (width, height)
            || self.keys.len() != quads.len()
            || self
                .keys
                .iter()
                .zip(quads)
                .any(|(key, quad)| *key != (quad.z, quad.params[1]));
        if changed || self.layout.is_none() {
            let mut encoded = Vec::new();
            let layout = TileGeometry::append(width, height, quads, &mut encoded);
            encoded.truncate(encoded.len() - quads.len() * TileQuad::WORDS);
            let bins = layout.bin_count;
            self.relocations.clear();
            for bin in 0..bins as usize {
                self.relocations.push(bin);
                let list = encoded[bin] as usize;
                self.relocations
                    .extend(list + 1..list + 1 + encoded[list] as usize);
            }
            self.prefix = encoded;
            self.layout = Some(layout);
            self.extent = (width, height);
            self.keys.clear();
            self.keys.extend(quads.iter().map(|q| (q.z, q.params[1])));
        }
        let base = data.len();
        data.extend_from_slice(&self.prefix);
        for &index in &self.relocations {
            data[base + index] += base as u32;
        }
        for quad in quads {
            data.extend_from_slice(&quad.packed());
        }
        let layout = self.layout.as_ref().unwrap();
        TileDrawGeometry {
            bounds: layout.bounds,
            bin_width: layout.bin_width,
            bin_origin: layout.bin_origin,
            base: base as u32,
            layer_count: layout.layers.len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphics::Color;
    #[test]
    fn atlas_deduplicates_cells_and_invalidates_changed_pixels() {
        let mut atlas = TileAtlas::default();
        let mut source = Bitmap::new(64, 32).unwrap();
        source.fill_rect(source.rect(), Color::rgba(90, 120, 170, 180));
        let area = Rect::new(32, 0, 32, 32);
        let (id, cell) = atlas.cell(1, &source, area).unwrap();
        assert_eq!(cell.get_pixel(0, 0), source.get_pixel(32, 0));
        let revision = cell.revision();
        assert_eq!(atlas.cell(1, &source, area).unwrap().0, id);
        assert_eq!(atlas.cell(1, &source, area).unwrap().1.revision(), revision);
        source.set_pixel(32, 0, Color::rgb(1, 2, 3));
        let (same_id, cell) = atlas.cell(1, &source, area).unwrap();
        assert_eq!(same_id, id);
        assert_eq!(cell.get_pixel(0, 0), Some(Color::rgb(1, 2, 3)));
        assert_ne!(cell.revision(), revision);
        assert_ne!(
            atlas.cell(1, &source, Rect::new(0, 0, 32, 32)).unwrap().0,
            id
        );
    }
    #[test]
    fn ground_and_priority_geometry_keeps_overlap_order() {
        let mut quads = Vec::new();
        for z in [0, 0, 65, 97] {
            let mut params = [[0; 4]; 16];
            params[1] = [0, 0, 32, 32];
            quads.push(TileQuad { z, params });
        }
        let mut data = Vec::new();
        let geometry = TileGeometry::append(64, 64, &quads, &mut data);
        assert_eq!(
            geometry.layers,
            [
                TileLayer {
                    z: 0,
                    first: 0,
                    count: 2
                },
                TileLayer {
                    z: 65,
                    first: 2,
                    count: 1
                },
                TileLayer {
                    z: 97,
                    first: 3,
                    count: 1
                }
            ]
        );
        let bin = data[0] as usize;
        assert_eq!(data[bin], 4);
        assert_eq!(data[bin + 2] - data[bin + 1], TileQuad::WORDS as u32);
        assert_eq!(data[bin + 4] - data[bin + 3], TileQuad::WORDS as u32);
        let mut cache = TileGeometryCache::default();
        for (phase, offset) in [0, 13, 7].into_iter().enumerate() {
            quads[0].params[0][2] = phase as u32; // animated atlas cell
            let mut cached = vec![99; offset];
            cache.append(64, 64, &quads, &mut cached);
            let mut rebuilt = vec![99; offset];
            TileGeometry::append(64, 64, &quads, &mut rebuilt);
            assert_eq!(
                cached, rebuilt,
                "retained geometry must relocate every index"
            );
        }
    }
}
