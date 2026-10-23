// SPDX-License-Identifier: GPL-3.0-or-later
use super::NdsAdapter;
use dust_core::utils::{
    PersistentReadSavestate, PersistentWriteSavestate, ReadSavestate, WriteSavestate,
};
use rsxp_core::{CoreError, CoreResult};
// Bump this when changing the backend revision or the envelope layout.
const MAGIC: &[u8; 12] = b"RSXP-NDS\x01\0\0\0";
fn hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |h, b| {
        (h ^ *b as u64).wrapping_mul(0x100000001b3)
    })
}
impl NdsAdapter {
    pub(super) fn save_state(&mut self) -> CoreResult<Vec<u8>> {
        let emu = self.emu.as_mut().unwrap();
        let mut payload = Vec::new();
        PersistentWriteSavestate::new(&mut payload)
            .store(emu)
            .map_err(|_| CoreError::runtime("could not serialize DS state"))?;
        let save = emu.ds_slot.spi.contents();
        let mut data = Vec::with_capacity(32 + save.len() + payload.len());
        data.extend_from_slice(MAGIC);
        data.extend_from_slice(&hash(&self.rom).to_le_bytes());
        data.extend_from_slice(&(save.len() as u32).to_le_bytes());
        data.extend_from_slice(save);
        data.extend_from_slice(&payload);
        data.extend_from_slice(&hash(&data).to_le_bytes());
        Ok(data)
    }
    pub(super) fn load_state(&mut self, data: &[u8]) -> CoreResult<()> {
        let invalid =
            || CoreError::content("invalid, incompatible, or different-game DS save state");
        if data.len() < 32 || data.len() > 64 * 1024 * 1024 || &data[..12] != MAGIC {
            return Err(invalid());
        }
        let u64_at = |offset| u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap());
        if u64_at(12) != hash(&self.rom) || u64_at(data.len() - 8) != hash(&data[..data.len() - 8])
        {
            return Err(invalid());
        }
        let len = u32::from_le_bytes(data[20..24].try_into().unwrap()) as usize;
        let end = 24usize
            .checked_add(len)
            .filter(|end| *end <= data.len() - 8)
            .ok_or_else(invalid)?;
        // Decode into a fresh machine so a failed load cannot damage the running game.
        let mut candidate = self.build(Some(data[24..end].to_vec()))?;
        PersistentReadSavestate::new(&data[end..data.len() - 8])
            .and_then(|mut reader| reader.load_into(&mut candidate).map_err(drop))
            .map_err(|_| invalid())?;
        candidate.ds_slot.spi.mark_contents_dirty();
        self.emu = Some(candidate);
        self.samples.borrow_mut().clear();
        Ok(())
    }
}
