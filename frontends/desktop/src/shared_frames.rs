// SPDX-License-Identifier: GPL-2.0-or-later
//! A small file-backed shared-memory ring for cartridge video frames.
//!
//! The file is created by the dashboard and opened by the runner. Frame bytes
//! never cross the session socket; only the slot number and dimensions do.

use memmap2::{MmapMut, MmapOptions};
use std::fs::OpenOptions;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};

pub const SLOT_COUNT: usize = 3;
// Cartridge cores in this frontend are bounded well below 1 MiB; leave room
// for future layouts without reserving hundreds of megabytes per session.
pub const MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;
const MAGIC: &[u8; 8] = b"FPLAYFRM";
const HEADER_BYTES: usize = 64;
const SLOT_BYTES: usize = 4 + MAX_FRAME_BYTES;
const FREE: u32 = 0;
const WRITING: u32 = 1;
const READY: u32 = 2;

pub struct SharedFrames {
    map: MmapMut,
}

impl SharedFrames {
    pub fn create(path: &Path) -> Result<Self, String> {
        let file = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(path)
            .map_err(|e| format!("could not create shared frame file: {e}"))?;
        let length = HEADER_BYTES + SLOT_COUNT * SLOT_BYTES;
        file.set_len(length as u64)
            .map_err(|e| format!("could not size shared frame file: {e}"))?;
        let mut map = unsafe { MmapOptions::new().len(length).map_mut(&file) }
            .map_err(|e| format!("could not map shared frame file: {e}"))?;
        map[..8].copy_from_slice(MAGIC);
        for slot in 0..SLOT_COUNT {
            state_ptr(&map, slot).store(FREE, Ordering::Release);
        }
        Ok(Self { map })
    }

    pub fn open(path: &Path) -> Result<Self, String> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|e| format!("could not open shared frame file: {e}"))?;
        let length = HEADER_BYTES + SLOT_COUNT * SLOT_BYTES;
        let map = unsafe { MmapOptions::new().len(length).map_mut(&file) }
            .map_err(|e| format!("could not map shared frame file: {e}"))?;
        if &map[..8] != MAGIC {
            return Err("invalid shared frame file".into());
        }
        Ok(Self { map })
    }

    pub fn acquire(&self) -> Option<FrameSlot<'_>> {
        for slot in 0..SLOT_COUNT {
            if state_ptr(&self.map, slot)
                .compare_exchange(FREE, WRITING, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                return Some(FrameSlot {
                    frames: self,
                    slot,
                    published: false,
                });
            }
        }
        None
    }

    pub fn read(&self, slot: usize, length: usize) -> Option<&[u8]> {
        if slot >= SLOT_COUNT || length > MAX_FRAME_BYTES {
            return None;
        }
        if state_ptr(&self.map, slot).load(Ordering::Acquire) != READY {
            return None;
        }
        let start = slot_offset(slot) + 4;
        Some(&self.map[start..start + length])
    }

    pub fn release(&self, slot: usize) {
        if slot < SLOT_COUNT {
            state_ptr(&self.map, slot).store(FREE, Ordering::Release);
        }
    }
}

pub struct FrameSlot<'a> {
    frames: &'a SharedFrames,
    slot: usize,
    published: bool,
}

impl FrameSlot<'_> {
    pub fn index(&self) -> usize {
        self.slot
    }

    pub fn write(&mut self, bytes: &[u8]) -> Result<(), String> {
        if bytes.len() > MAX_FRAME_BYTES {
            return Err("frame exceeds shared-memory slot".into());
        }
        let start = slot_offset(self.slot) + 4;
        // SAFETY: this slot is exclusively owned by the FrameSlot state
        // transition (WRITING), so no other process accesses these bytes.
        unsafe {
            std::slice::from_raw_parts_mut(
                self.frames.map.as_ptr().add(start) as *mut u8,
                bytes.len(),
            )
            .copy_from_slice(bytes);
        }
        Ok(())
    }

    pub fn publish(mut self) {
        self.published = true;
        state_ptr(&self.frames.map, self.slot).store(READY, Ordering::Release);
    }
}

impl Drop for FrameSlot<'_> {
    fn drop(&mut self) {
        if !self.published {
            state_ptr(&self.frames.map, self.slot).store(FREE, Ordering::Release);
        }
    }
}

fn slot_offset(slot: usize) -> usize {
    HEADER_BYTES + slot * SLOT_BYTES
}

fn state_ptr(map: &MmapMut, slot: usize) -> &AtomicU32 {
    // HEADER_BYTES and SLOT_BYTES are multiples of four, so every state field
    // is suitably aligned for an inter-process AtomicU32.
    unsafe { &*(map.as_ptr().add(slot_offset(slot)) as *const AtomicU32) }
}

unsafe impl Send for SharedFrames {}
unsafe impl Sync for SharedFrames {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_transfer_bytes_and_return_to_the_ring() {
        let path = std::env::temp_dir().join(format!(
            "fanplay-shared-frame-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let frames = SharedFrames::create(&path).unwrap();
        let mut slot = frames.acquire().unwrap();
        slot.write(&[1, 2, 3, 4]).unwrap();
        let index = slot.index();
        slot.publish();
        assert_eq!(frames.read(index, 4).unwrap(), &[1, 2, 3, 4]);
        frames.release(index);
        assert!(frames.acquire().is_some());
        let _ = std::fs::remove_file(path);
    }
}
