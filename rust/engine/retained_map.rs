// SPDX-License-Identifier: GPL-2.0-or-later
//! Retained Essentials visible-cell selection, independent of Ruby and GPU.
//!
//! `MapView` describes one connected map's visible window into its `Table`
//! (identity, data revision, camera `x`/`y`, size). `RetainedMap::update`
//! recomputes the `width × height × 3`-layer screen grid, later maps
//! overwriting earlier ones where they overlap, and returns only the `Change`s
//! versus the previous frame (`None` cell = uncovered). It skips work when the
//! view keys are unchanged unless `force`. `shift` rotates retained cells to
//! match Essentials' reused sprite columns/rows, so scrolling reports only the
//! newly exposed strips. Cell count is capped at 4,000,000. Optional
//! `MapChunks` caching is toggled with `set_chunks_enabled`.
use crate::table::Table;

pub struct MapView<'a> {
    pub identity: u64,
    pub data_identity: u64,
    pub revision: u64,
    pub data: &'a Table,
    pub x: i64,
    pub y: i64,
    pub width: usize,
    pub height: usize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct ViewKey {
    identity: u64,
    data_identity: u64,
    revision: u64,
    x: i64,
    y: i64,
    width: usize,
    height: usize,
}
#[derive(Clone, Copy, Debug)]
pub struct Cell {
    pub identity: u64,
    pub data_identity: u64,
    pub map: usize,
    pub tile: i16,
    pub map_x: usize,
    pub map_y: usize,
}
impl PartialEq for Cell {
    fn eq(&self, other: &Self) -> bool {
        self.identity == other.identity
            && self.data_identity == other.data_identity
            && self.tile == other.tile
            && self.map_x == other.map_x
            && self.map_y == other.map_y
    }
}
impl Eq for Cell {}
#[derive(Debug, PartialEq, Eq)]
pub struct Change {
    pub x: usize,
    pub y: usize,
    pub layer: usize,
    pub cell: Option<Cell>,
}
#[derive(Default)]
pub struct RetainedMap {
    width: usize,
    height: usize,
    keys: Vec<ViewKey>,
    cells: Vec<Option<Cell>>,
    next: Vec<Option<Cell>>,
    shifted: bool,
    chunks: super::map_chunks::MapChunks,
    uncached: bool,
}
impl RetainedMap {
    pub fn set_chunks_enabled(&mut self, enabled: bool) {
        self.uncached = !enabled;
    }
    pub fn chunk_stats(&self) -> super::map_chunks::ChunkStats {
        self.chunks.stats()
    }
    /// Rotate retained cells exactly as Essentials rotates its reusable sprite
    /// columns/rows. Compare reused world cells, rather than old screen slots.
    pub fn shift(&mut self, x: usize, y: usize) {
        if self.width == 0 || self.height == 0 {
            return;
        }
        let x = x % self.width;
        let y = y % self.height;
        if x == 0 && y == 0 {
            return;
        }
        self.cells.rotate_left(x * self.height * 3);
        if y != 0 {
            for column in self.cells.chunks_exact_mut(self.height * 3) {
                column.rotate_left(y * 3);
            }
        }
        self.shifted = true;
    }
    pub fn update(
        &mut self,
        width: usize,
        height: usize,
        maps: &[MapView<'_>],
        force: bool,
    ) -> Result<Vec<Change>, &'static str> {
        if width > 4_000_000 || height > 4_000_000 {
            return Err("retained map exceeds cell limit");
        }
        let count = width
            .checked_mul(height)
            .and_then(|n| n.checked_mul(3))
            .filter(|&n| n <= 4_000_000)
            .ok_or("retained map exceeds cell limit")?;
        let keys: Vec<_> = maps
            .iter()
            .map(|map| ViewKey {
                identity: map.identity,
                data_identity: map.data_identity,
                revision: map.revision,
                x: map.x,
                y: map.y,
                width: map.width,
                height: map.height,
            })
            .collect();
        let resized = self.width != width || self.height != height;
        if !force && !resized && !self.shifted && keys == self.keys {
            return Ok(Vec::new());
        }
        self.next.clear();
        self.next.resize(count, None);
        if !self.uncached {
            self.chunks.begin();
            for map in maps {
                self.chunks.prefetch(map, width, height);
            }
        }
        // Match the Ruby renderer: later connected maps overwrite earlier maps
        // where their visible regions overlap. Uncovered cells are cleared.
        for (index, map) in maps.iter().enumerate() {
            let start_x = map.x.saturating_neg().max(0).min(width as i64) as usize;
            let start_y = map.y.saturating_neg().max(0).min(height as i64) as usize;
            let end_x = (i64::try_from(map.width)
                .unwrap_or(i64::MAX)
                .saturating_sub(map.x))
            .max(0)
            .min(width as i64) as usize;
            let end_y = (i64::try_from(map.height)
                .unwrap_or(i64::MAX)
                .saturating_sub(map.y))
            .max(0)
            .min(height as i64) as usize;
            for x in start_x..end_x {
                for y in start_y..end_y {
                    let map_x = (x as i64 + map.x) as usize;
                    let map_y = (y as i64 + map.y) as usize;
                    for layer in 0..3 {
                        self.next[(x * height + y) * 3 + layer] = Some(Cell {
                            identity: map.identity,
                            data_identity: map.data_identity,
                            map: index,
                            tile: if self.uncached {
                                map.data.get(map_x, map_y, layer).unwrap_or(0)
                            } else {
                                self.chunks.tile(map, map_x, map_y, layer)
                            },
                            map_x,
                            map_y,
                        });
                    }
                }
            }
        }
        let changes = self
            .next
            .iter()
            .enumerate()
            .filter_map(|(index, &cell)| {
                (force || resized || self.cells.get(index) != Some(&cell)).then_some(Change {
                    x: index / 3 / height.max(1),
                    y: index / 3 % height.max(1),
                    layer: index % 3,
                    cell,
                })
            })
            .collect();
        std::mem::swap(&mut self.cells, &mut self.next);
        self.keys = keys;
        self.width = width;
        self.height = height;
        self.shifted = false;
        if !self.uncached {
            self.chunks.finish();
        }
        Ok(changes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retains_cells_and_detects_edits_camera_connections_and_uncovered_cells() {
        let mut data = Table::new(4, 2, 3).unwrap();
        data.set(0, 0, 0, 7);
        let mut cache = RetainedMap::default();
        fn view(data: &Table, revision: u64, x: i64) -> MapView<'_> {
            MapView {
                identity: 1,
                data_identity: 1,
                revision,
                data,
                x,
                y: 0,
                width: 4,
                height: 2,
            }
        }
        assert_eq!(
            cache
                .update(2, 2, &[view(&data, 0, 0)], false)
                .unwrap()
                .len(),
            12
        );
        assert!(cache
            .update(2, 2, &[view(&data, 0, 0)], false)
            .unwrap()
            .is_empty());
        data.set(0, 0, 0, 8);
        let changes = cache.update(2, 2, &[view(&data, 1, 0)], false).unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].cell.unwrap().tile, 8);
        assert_eq!(
            cache
                .update(2, 2, &[view(&data, 1, 1)], false)
                .unwrap()
                .len(),
            12
        );
        assert_eq!(cache.update(2, 2, &[], false).unwrap().len(), 12);
        assert!(cache.update(2, 2, &[], false).unwrap().is_empty());
        assert_eq!(cache.update(2, 2, &[], true).unwrap().len(), 12);
    }
    #[test]
    fn scrolling_reuses_rotated_world_cells_and_reports_only_exposed_strips() {
        let data = Table::new(12, 8, 3).unwrap();
        let mut cache = RetainedMap::default();
        let view = |x, y| MapView {
            identity: 1,
            data_identity: 1,
            revision: 0,
            data: &data,
            x,
            y,
            width: 12,
            height: 8,
        };
        assert_eq!(cache.update(4, 3, &[view(0, 0)], false).unwrap().len(), 36);
        cache.shift(1, 0);
        let changes = cache.update(4, 3, &[view(1, 0)], false).unwrap();
        assert_eq!(changes.len(), 9);
        assert!(changes.iter().all(|change| change.x == 3));
        cache.shift(1, 1);
        assert_eq!(cache.update(4, 3, &[view(2, 1)], false).unwrap().len(), 18);
        cache.shift(3, 2); // reverse one column and one row
        assert_eq!(cache.update(4, 3, &[view(1, 0)], false).unwrap().len(), 18);
        assert!(cache.update(4, 3, &[view(1, 0)], false).unwrap().is_empty());
    }
    #[test]
    fn connected_map_order_and_resize_are_observed() {
        let mut a = Table::new(1, 1, 3).unwrap();
        let mut b = a.clone();
        a.set(0, 0, 0, 10);
        b.set(0, 0, 0, 20);
        let mut cache = RetainedMap::default();
        let maps = [
            MapView {
                identity: 1,
                data_identity: 1,
                revision: 0,
                data: &a,
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            },
            MapView {
                identity: 2,
                data_identity: 2,
                revision: 0,
                data: &b,
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            },
        ];
        assert_eq!(
            cache.update(1, 1, &maps, false).unwrap()[0]
                .cell
                .unwrap()
                .tile,
            20
        );
        assert_eq!(cache.update(2, 1, &maps, false).unwrap().len(), 6);
        assert!(cache.update(usize::MAX, 2, &maps, false).is_err());
        assert!(cache.update(usize::MAX, 0, &maps, false).is_err());
        let extreme = MapView {
            identity: 3,
            data_identity: 3,
            revision: 0,
            data: &a,
            x: i64::MIN,
            y: i64::MAX,
            width: 1,
            height: 1,
        };
        assert!(cache
            .update(1, 1, &[extreme], false)
            .unwrap()
            .iter()
            .all(|change| change.cell.is_none()));
    }
}
