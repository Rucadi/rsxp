// SPDX-License-Identifier: GPL-3.0-or-later
use super::{NdsAdapter, CORE_ID, STATE_REVISION};
use dust_core::utils::{
    PersistentReadSavestate, PersistentWriteSavestate, ReadSavestate, WriteSavestate,
};
use fanplay_core::{content_hash, CoreError, CoreResult, StateEnvelope};

impl NdsAdapter {
    pub(super) fn save_state(&mut self) -> CoreResult<Vec<u8>> {
        let emu = self.emu.as_mut().unwrap();
        let mut payload = Vec::new();
        PersistentWriteSavestate::new(&mut payload)
            .store(emu)
            .map_err(|_| CoreError::runtime("could not serialize DS state"))?;
        let save = emu.ds_slot.spi.contents();
        let mut data = Vec::with_capacity(4 + save.len() + payload.len());
        data.extend_from_slice(&(save.len() as u32).to_le_bytes());
        data.extend_from_slice(save);
        data.extend_from_slice(&payload);
        StateEnvelope::new(CORE_ID, STATE_REVISION, content_hash(&self.rom), data)
            .encode()
            .map_err(|error| CoreError::runtime(format!("could not encode DS state: {error}")))
    }

    pub(super) fn load_state(&mut self, data: &[u8]) -> CoreResult<()> {
        let invalid =
            || CoreError::content("invalid, incompatible, or different-game DS save state");
        let envelope =
            StateEnvelope::decode_for(data, CORE_ID, STATE_REVISION, content_hash(&self.rom))
                .map_err(|error| CoreError::content(error.to_string()))?;
        let payload = &envelope.payload;
        if payload.len() < 4 {
            return Err(invalid());
        }
        let len = u32::from_le_bytes(payload[..4].try_into().unwrap()) as usize;
        let end = 4usize
            .checked_add(len)
            .filter(|end| *end <= payload.len())
            .ok_or_else(invalid)?;
        // Decode into a fresh machine so a failed load cannot damage the running game.
        let (mut candidate, gpu_output) = self.build(Some(payload[4..end].to_vec()))?;
        PersistentReadSavestate::new(&payload[end..])
            .and_then(|mut reader| reader.load_into(&mut candidate).map_err(drop))
            .map_err(|_| invalid())?;
        candidate.ds_slot.spi.mark_contents_dirty();
        self.emu = Some(candidate);
        self.gpu_output = gpu_output;
        self.flush_audio();
        Ok(())
    }
}
