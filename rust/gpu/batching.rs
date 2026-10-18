// SPDX-License-Identifier: GPL-2.0-or-later
//! Draw batching: groups runs of independent sprite/plane writes into a single
//! compute dispatch so their pixels may execute concurrently.
//!
//! `DrawBatch` records a contiguous command range (`start`, `len`) plus the
//! largest destination extent (`width`, `height`) the dispatch must cover.
//!
//! `draw_batches` walks the encoded `Params` list and extends the current batch
//! only when batching is enabled, the draw has a non-empty rectangle, it is an
//! operation-1 sprite/plane (`a[0][0] == 1`), the batch holds fewer than 64
//! draws, and every command already in the batch writes either a different
//! buffer (`a[0][1]`, the main frame vs the scratch layer) or a disjoint
//! rectangle. Overlapping writes, and all non-draw operations (clear,
//! viewport, brightness, native tile geometry), therefore begin a new batch,
//! preserving scratch and ordering dependencies. When `enabled` is false each
//! command becomes its own single-command batch.
use super::Params;

pub(super) struct DrawBatch {
    pub(super) start: usize,
    pub(super) len: usize,
    pub(super) width: u32,
    pub(super) height: u32,
}

// Adjacent, disjoint sprite writes may execute concurrently. Viewport reads
// and all other operations form barriers, preserving scratch dependencies.
pub(super) fn draw_batches(commands: &[Params], enabled: bool) -> Vec<DrawBatch> {
    let mut batches: Vec<DrawBatch> = Vec::new();
    for (i, p) in commands.iter().enumerate() {
        let [x, y, x1, y1] = p.a[1];
        let width = x1.saturating_sub(x);
        let height = y1.saturating_sub(y);
        let append = enabled
            && width > 0
            && height > 0
            && p.a[0][0] == 1
            && batches.last().is_some_and(|batch| {
                batch.len < 64
                    && commands[batch.start..i].iter().all(|other| {
                        let [ox, oy, ox1, oy1] = other.a[1];
                        other.a[0][0] == 1
                            && (other.a[0][1] != p.a[0][1]
                                || x >= ox1
                                || ox >= x1
                                || y >= oy1
                                || oy >= y1)
                    })
            });
        if append {
            let batch = batches.last_mut().unwrap();
            batch.len += 1;
            batch.width = batch.width.max(width);
            batch.height = batch.height.max(height);
        } else {
            batches.push(DrawBatch {
                start: i,
                len: 1,
                width: width.max(1),
                height: height.max(1),
            });
        }
    }
    batches
}
