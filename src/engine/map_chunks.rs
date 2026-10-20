// SPDX-License-Identifier: GPL-2.0-or-later
//! Bounded world-cell chunks, prefetched one chunk beyond the viewport.
//! Hidden chunks retain tile data without allocating Ruby sprites or bitmaps.
//!
//! Tiles are cached in `CHUNK_SIZE` (16) × 16 × 3-layer blocks keyed by the
//! map's `data_identity` plus chunk coordinates. `prefetch` marks every chunk
//! covering the viewport plus a one-chunk margin; `tile` reads a single cell
//! (building its chunk on demand) and `begin`/`finish` bracket a frame.
//! Chunks are validated by `map.revision`; a revision bump re-reads cells but
//! only replaces data that actually changed. `finish` evicts chunks not touched
//! this epoch (oldest first) once `budget` is exceeded, preserving the active
//! viewport working set. `ChunkStats` reports builds/hits/checks/evictions.
use super::retained_map::MapView;
use std::collections::HashMap;
pub const CHUNK_SIZE: usize = 16;
const CHUNK_CELLS: usize = CHUNK_SIZE * CHUNK_SIZE * 3;
const DEFAULT_BUDGET: usize = 128;
#[derive(Clone, Copy, Hash, PartialEq, Eq)]
struct Key {
    table: u64,
    x: usize,
    y: usize,
}
struct Chunk {
    revision: u64,
    data: Vec<i16>,
    used: u64,
}
#[derive(Default, Clone, Copy, Debug)]
pub struct ChunkStats {
    pub builds: u64,
    pub hits: u64,
    pub checks: u64,
    pub evictions: u64,
    pub resident: usize,
}
pub struct MapChunks {
    entries: HashMap<Key, Chunk>,
    epoch: u64,
    budget: usize,
    stats: ChunkStats,
}
impl Default for MapChunks {
    fn default() -> Self {
        Self::new(DEFAULT_BUDGET)
    }
}
impl MapChunks {
    pub fn new(budget: usize) -> Self {
        Self {
            entries: HashMap::new(),
            epoch: 0,
            budget,
            stats: ChunkStats::default(),
        }
    }
    pub fn begin(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
    }
    pub fn prefetch(&mut self, map: &MapView<'_>, width: usize, height: usize) {
        let margin = CHUNK_SIZE as i64;
        let left = map
            .x
            .saturating_sub(margin)
            .max(0)
            .min(i64::try_from(map.width).unwrap_or(i64::MAX)) as usize;
        let top = map
            .y
            .saturating_sub(margin)
            .max(0)
            .min(i64::try_from(map.height).unwrap_or(i64::MAX)) as usize;
        let right = map
            .x
            .saturating_add(width as i64)
            .saturating_add(margin)
            .max(0)
            .min(i64::try_from(map.width).unwrap_or(i64::MAX)) as usize;
        let bottom = map
            .y
            .saturating_add(height as i64)
            .saturating_add(margin)
            .max(0)
            .min(i64::try_from(map.height).unwrap_or(i64::MAX)) as usize;
        if left >= right || top >= bottom {
            return;
        }
        for x in left / CHUNK_SIZE..=(right - 1) / CHUNK_SIZE {
            for y in top / CHUNK_SIZE..=(bottom - 1) / CHUNK_SIZE {
                self.ensure(map, x, y);
            }
        }
    }
    fn ensure(&mut self, map: &MapView<'_>, x: usize, y: usize) {
        let key = Key {
            table: map.data_identity,
            x,
            y,
        };
        if let Some(chunk) = self.entries.get_mut(&key) {
            chunk.used = self.epoch;
            if chunk.revision == map.revision {
                self.stats.hits += 1;
                return;
            }
        }
        self.stats.checks += 1;
        let mut data = Vec::with_capacity(CHUNK_CELLS);
        for dx in 0..CHUNK_SIZE {
            for dy in 0..CHUNK_SIZE {
                for layer in 0..3 {
                    data.push(
                        map.data
                            .get(x * CHUNK_SIZE + dx, y * CHUNK_SIZE + dy, layer)
                            .unwrap_or(0),
                    );
                }
            }
        }
        if let Some(chunk) = self.entries.get_mut(&key) {
            chunk.revision = map.revision;
            // A Table revision causes validation, not blanket invalidation.
            // Only a chunk whose actual cells changed replaces its data.
            if chunk.data == data {
                self.stats.hits += 1;
                return;
            }
        }
        self.stats.builds += 1;
        self.entries.insert(
            key,
            Chunk {
                revision: map.revision,
                data,
                used: self.epoch,
            },
        );
    }
    pub fn tile(&mut self, map: &MapView<'_>, x: usize, y: usize, layer: usize) -> i16 {
        let key = Key {
            table: map.data_identity,
            x: x / CHUNK_SIZE,
            y: y / CHUNK_SIZE,
        };
        if !self.entries.contains_key(&key) {
            self.ensure(map, key.x, key.y);
        }
        let chunk = &self.entries[&key];
        chunk.data[((x % CHUNK_SIZE) * CHUNK_SIZE + y % CHUNK_SIZE) * 3 + layer]
    }
    pub fn finish(&mut self) {
        if self.entries.len() <= self.budget {
            return;
        }
        let mut old: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, chunk)| chunk.used != self.epoch)
            .map(|(&key, chunk)| (chunk.used, key))
            .collect();
        old.sort_unstable_by_key(|entry| entry.0);
        for (_, key) in old {
            if self.entries.len() <= self.budget {
                break;
            }
            self.entries.remove(&key);
            self.stats.evictions += 1;
        }
        // A viewport larger than the budget keeps its active working set. Old
        // chunks are evicted first, avoiding churn in the visible/prefetch ring.
    }
    pub fn stats(&self) -> ChunkStats {
        ChunkStats {
            resident: self.entries.len(),
            ..self.stats
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::table::Table;
    fn view(data: &Table, revision: u64, x: i64) -> MapView<'_> {
        MapView {
            identity: 1,
            data_identity: 1,
            revision,
            data,
            x,
            y: 0,
            width: 256,
            height: 64,
        }
    }
    #[test]
    fn prefetch_reuses_hidden_chunks_and_edits_rebuild_only_changed_data() {
        let mut data = Table::new(256, 64, 3).unwrap();
        let mut cache = MapChunks::new(128);
        cache.begin();
        cache.prefetch(&view(&data, 0, 0), 16, 16);
        cache.finish();
        let builds = cache.stats().builds;
        assert_eq!(builds, 4);
        cache.begin();
        cache.prefetch(&view(&data, 0, 1), 16, 16);
        cache.finish();
        let prefetched = cache.stats().builds;
        cache.begin();
        cache.prefetch(&view(&data, 0, 16), 16, 16);
        cache.finish();
        assert_eq!(
            cache.stats().builds,
            prefetched,
            "revealing warmed tiles must not rebuild chunks"
        );
        data.set(17, 0, 0, 42);
        cache.begin();
        cache.prefetch(&view(&data, 1, 16), 16, 16);
        cache.finish();
        assert_eq!(cache.stats().builds, prefetched + 1);
        assert_eq!(cache.tile(&view(&data, 1, 16), 17, 0, 0), 42);
        cache.begin();
        cache.prefetch(&view(&data, 1, 0), 16, 16);
        cache.finish();
        assert_eq!(
            cache.stats().builds,
            prefetched + 1,
            "returning to hidden chunks must reuse them"
        );
    }
    #[test]
    fn old_chunks_are_evicted_under_a_budget() {
        let data = Table::new(256, 64, 3).unwrap();
        let mut cache = MapChunks::new(8);
        for x in (0..240).step_by(16) {
            cache.begin();
            cache.prefetch(&view(&data, 0, x), 16, 16);
            cache.finish();
            assert!(cache.stats().resident <= 8);
        }
        assert!(cache.stats().evictions > 0);
    }
}
