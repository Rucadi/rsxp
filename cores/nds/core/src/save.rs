// SPDX-License-Identifier: GPL-3.0-or-later
use dust_core::{
    ds_slot::spi::{self, Spi},
    utils::BoxedByteSlice,
    SaveContents,
};
use fanplay_core::{CoreError, CoreResult};

pub fn device(rom: &[u8], existing: Option<Vec<u8>>) -> CoreResult<Spi> {
    let code = u32::from_le_bytes(rom[12..16].try_into().unwrap());
    let database: serde_json::Value =
        serde_json::from_str(include_str!("../game_db.json")).expect("bundled Dust database");
    let kind = database
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["code"].as_u64() == Some(code as u64))
        .and_then(|entry| entry["save-type"].as_str())
        .unwrap_or("none");
    let size = match kind {
        "eeprom-4k" => 0x200,
        "eeprom-fram-64k" => 0x2000,
        "eeprom-fram-512k" => 0x10000,
        "eeprom-fram-1m" => 0x20000,
        "flash-2m" => 0x40000,
        "flash-4m" => 0x80000,
        "flash-8m" => 0x100000,
        "none" => 0,
        _ => {
            return Err(CoreError::content(format!(
                "Dust does not support this cartridge's save type: {kind}"
            )))
        }
    };
    let size = existing.as_ref().map_or(size, Vec::len);
    let contents = match existing {
        Some(bytes) => {
            let mut out = BoxedByteSlice::new_zeroed(bytes.len());
            out.copy_from_slice(&bytes);
            SaveContents::Existing(out)
        }
        None => SaveContents::New(size),
    };
    let invalid = || CoreError::content(format!("unsupported DS save size: {size} bytes"));
    Ok(match size {
        0 => spi::Empty::new().into(),
        0x200 => spi::eeprom_4k::Eeprom4k::new(contents, None)
            .map_err(|_| invalid())?
            .into(),
        0x2000 | 0x10000 | 0x20000 => spi::eeprom_fram::EepromFram::new(contents, None)
            .map_err(|_| invalid())?
            .into(),
        0x40000 | 0x80000 | 0x100000 => spi::flash::Flash::new(contents, [0; 20], rom[12] == b'I')
            .map_err(|_| invalid())?
            .into(),
        _ => return Err(invalid()),
    })
}
