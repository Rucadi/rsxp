//! Action Replay DS cheat parsing and execution.
//!
//! The common DS format is a sequence of `XXXXXXXX YYYYYYYY` words.  This
//! module intentionally keeps the interpreter small and deterministic: simple
//! 8/16/32-bit writes, conditionals, offset/data-register operations and
//! repeat blocks cover the codes used by standard cheat databases while the
//! emulator remains the owner of all memory access.

use dust_core::{
    cpu::{arm7, arm9, bus::CpuAccess, interpreter::Interpreter},
    emu::Emu,
};

#[derive(Clone, Debug, Default)]
pub struct ActionReplay {
    codes: Vec<CodeLine>,
    condition_stack: Vec<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CodeLine {
    command: u32,
    value: u32,
}

#[derive(Default)]
struct ExecutionState {
    offset: u32,
    data: u32,
}

impl ActionReplay {
    pub fn add(&mut self, source: &str) -> Result<usize, String> {
        let parsed = parse(source)?;
        if parsed.is_empty() {
            return Err("cheat code did not contain any 16-digit Action Replay lines".into());
        }
        self.codes.extend(parsed);
        Ok(self.codes.len())
    }

    pub fn remove(&mut self, source: &str) -> Result<bool, String> {
        let parsed = parse(source)?;
        if parsed.is_empty() {
            return Err("cheat code did not contain any 16-digit Action Replay lines".into());
        }
        let before = self.codes.len();
        self.codes = remove_sequence(&self.codes, &parsed);
        Ok(before != self.codes.len())
    }

    pub fn clear(&mut self) {
        self.codes.clear();
    }

    pub fn apply(&mut self, emu: &mut Emu<Interpreter>) {
        if !self.codes.is_empty() {
            self.condition_stack.clear();
            let mut state = ExecutionState::default();
            execute_range(
                &self.codes,
                0,
                self.codes.len(),
                emu,
                &mut state,
                &mut self.condition_stack,
            );
        }
    }
}

fn parse(source: &str) -> Result<Vec<CodeLine>, String> {
    let mut result = Vec::new();
    for (line_number, original) in source.lines().enumerate() {
        let mut line = original.trim();
        if let Some(comment) = line.find("//") {
            line = &line[..comment];
        }
        line = line.trim().trim_start_matches(';').trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let compact: String = line
            .chars()
            .filter(|character| !character.is_ascii_whitespace() && *character != '-')
            .collect();
        if compact.len() == 16
            && compact
                .chars()
                .all(|character| character.is_ascii_hexdigit())
        {
            let command = u32::from_str_radix(&compact[..8], 16).map_err(|error| {
                format!(
                    "invalid Action Replay command on line {}: {error}",
                    line_number + 1
                )
            })?;
            let value = u32::from_str_radix(&compact[8..], 16).map_err(|error| {
                format!(
                    "invalid Action Replay value on line {}: {error}",
                    line_number + 1
                )
            })?;
            result.push(CodeLine { command, value });
        } else if line.chars().all(|character| {
            character.is_ascii_hexdigit() || character.is_ascii_whitespace() || character == '-'
        }) {
            return Err(format!(
                "Action Replay line {} must contain exactly 16 hexadecimal digits",
                line_number + 1
            ));
        }
        // Cheat databases commonly include a title, game ID or a prose
        // comment next to the code. Non-code lines are intentionally ignored.
    }
    Ok(result)
}

fn remove_sequence(source: &[CodeLine], sequence: &[CodeLine]) -> Vec<CodeLine> {
    if sequence.len() > source.len() {
        return source.to_vec();
    }
    let mut result = Vec::with_capacity(source.len());
    let mut index = 0;
    while index < source.len() {
        if index + sequence.len() <= source.len()
            && source[index..index + sequence.len()] == *sequence
        {
            index += sequence.len();
        } else {
            result.push(source[index]);
            index += 1;
        }
    }
    result
}

fn execute_range(
    codes: &[CodeLine],
    start: usize,
    end: usize,
    emu: &mut Emu<Interpreter>,
    state: &mut ExecutionState,
    conditions: &mut Vec<bool>,
) {
    let condition_start = conditions.len();
    let mut index = start;
    while index < end {
        let CodeLine { command, value } = codes[index];
        let kind = command >> 28;
        let address = command & 0x0FFF_FFFF;
        let active = conditions.iter().all(|active| *active);
        match kind {
            0 => {
                if active {
                    write_memory(emu, address.wrapping_add(state.offset), 4, value);
                }
            }
            1 => {
                if active {
                    write_memory(emu, address.wrapping_add(state.offset), 2, value & 0xFFFF);
                }
            }
            2 => {
                if active {
                    write_memory(emu, address.wrapping_add(state.offset), 1, value & 0xFF);
                }
            }
            3..=6 => conditions
                .push(active && compare_32(emu, address.wrapping_add(state.offset), kind, value)),
            7..=10 => {
                let mask = value >> 16;
                let expected = value & 0xFFFF;
                let actual = read_memory(emu, address.wrapping_add(state.offset), 2) as u32;
                let actual = actual & !mask;
                let matched = match kind {
                    7 => actual > expected,
                    8 => actual < expected,
                    9 => actual == expected,
                    _ => actual != expected,
                };
                conditions.push(active && matched);
            }
            0xB => {
                if active {
                    state.offset = read_memory(emu, address.wrapping_add(state.offset), 4) as u32;
                }
            }
            0xC => {
                if active && let Some(repeat_end) = find_repeat_end(codes, index + 1, end) {
                    // Datel's repeat count is inclusive: C...00000000 runs
                    // once, while C...00000001 runs twice.
                    for _ in 0..=value {
                        execute_range(codes, index + 1, repeat_end, emu, state, conditions);
                    }
                    index = repeat_end;
                }
            }
            0xD => match address >> 24 {
                0x00 => {
                    if conditions.len() > condition_start {
                        conditions.pop();
                    }
                }
                0x01 | 0x02 => break,
                0x03 => {
                    state.offset = value;
                }
                0x04 => {
                    state.data = state.data.wrapping_add(value);
                }
                0x05 => {
                    state.data = value;
                }
                0x06..=0x08 if active => {
                    let width = (address >> 24) - 0x06 + 1;
                    write_memory(emu, state.offset.wrapping_add(value), width, state.data);
                    state.offset = state.offset.wrapping_add(width);
                }
                0x09..=0x0B if active => {
                    let width = (address >> 24) - 0x09 + 1;
                    state.data = read_memory(emu, state.offset.wrapping_add(value), width);
                }
                0x0C => {
                    state.offset = state.offset.wrapping_add(value);
                }
                _ => {}
            },
            _ => {}
        }
        index += 1;
    }
    conditions.truncate(condition_start);
}

fn find_repeat_end(codes: &[CodeLine], start: usize, end: usize) -> Option<usize> {
    codes[start..end]
        .iter()
        .position(|line| line.command >> 28 == 0xD && (line.command & 0x0FFF_FFFF) >> 24 == 1)
        .map(|offset| start + offset)
}

fn compare_32(emu: &mut Emu<Interpreter>, address: u32, kind: u32, expected: u32) -> bool {
    let actual = read_memory(emu, address, 4);
    match kind {
        3 => actual < expected,
        4 => actual > expected,
        5 => actual == expected,
        _ => actual != expected,
    }
}

fn is_arm7_address(address: u32) -> bool {
    (0x0380_0000..0x0381_0000).contains(&address)
}

#[inline]
pub(crate) fn read_memory(emu: &mut Emu<Interpreter>, address: u32, width: u32) -> u32 {
    if let Some(value) = read_direct(emu, address, width) {
        return value;
    }
    if is_arm7_address(address) {
        match width {
            1 => arm7::bus::read_8::<CpuAccess, _>(emu, address) as u32,
            2 => arm7::bus::read_16::<CpuAccess, _>(emu, address) as u32,
            _ => arm7::bus::read_32::<CpuAccess, _>(emu, address),
        }
    } else {
        match width {
            1 => arm9::bus::read_8::<CpuAccess, _>(emu, address) as u32,
            2 => arm9::bus::read_16::<CpuAccess, _>(emu, address) as u32,
            _ => arm9::bus::read_32::<CpuAccess, _, false>(emu, address),
        }
    }
}

#[inline]
pub(crate) fn write_memory(emu: &mut Emu<Interpreter>, address: u32, width: u32, value: u32) {
    if write_direct(emu, address, width, value) {
        return;
    }
    if is_arm7_address(address) {
        match width {
            1 => arm7::bus::write_8::<CpuAccess, _>(emu, address, value as u8),
            2 => arm7::bus::write_16::<CpuAccess, _>(emu, address, value as u16),
            _ => arm7::bus::write_32::<CpuAccess, _>(emu, address, value),
        }
    } else {
        match width {
            1 => arm9::bus::write_8::<CpuAccess, _>(emu, address, value as u8),
            2 => arm9::bus::write_16::<CpuAccess, _>(emu, address, value as u16),
            _ => arm9::bus::write_32::<CpuAccess, _>(emu, address, value),
        }
    }
}

#[inline]
pub(crate) fn read_memory_bytes(
    emu: &mut Emu<Interpreter>,
    address: u32,
    length: usize,
) -> Vec<u8> {
    if let Some(bytes) = direct_bytes(emu, address, length) {
        return bytes.to_vec();
    }
    (0..length)
        .map(|offset| read_memory(emu, address.wrapping_add(offset as u32), 1) as u8)
        .collect()
}

#[inline]
fn read_direct(emu: &Emu<Interpreter>, address: u32, width: u32) -> Option<u32> {
    let bytes = direct_bytes(emu, address, width as usize)?;
    Some(match width {
        1 => bytes[0] as u32,
        2 => u16::from_le_bytes([bytes[0], bytes[1]]) as u32,
        4 => u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        _ => return None,
    })
}

#[inline]
fn write_direct(emu: &mut Emu<Interpreter>, address: u32, width: u32, value: u32) -> bool {
    let Some(bytes) = direct_bytes_mut(emu, address, width as usize) else {
        return false;
    };
    bytes.copy_from_slice(&value.to_le_bytes()[..width as usize]);
    true
}

#[inline]
fn direct_bytes<'a>(emu: &'a Emu<Interpreter>, address: u32, length: usize) -> Option<&'a [u8]> {
    if (0x0200_0000..0x0280_0000).contains(&address) {
        let offset = (address & emu.main_mem_mask().get()) as usize;
        let bytes = unsafe { emu.main_mem().as_arr() };
        return bytes.get(offset..offset.checked_add(length)?);
    }
    if is_arm7_address(address) {
        let offset = (address - 0x0380_0000) as usize;
        let bytes = unsafe { emu.arm7.wram.as_arr() };
        return bytes.get(offset..offset.checked_add(length)?);
    }
    None
}

#[inline]
fn direct_bytes_mut<'a>(
    emu: &'a mut Emu<Interpreter>,
    address: u32,
    length: usize,
) -> Option<&'a mut [u8]> {
    if (0x0200_0000..0x0280_0000).contains(&address) {
        let offset = (address & emu.main_mem_mask().get()) as usize;
        let bytes = unsafe { emu.main_mem().as_mut_arr() };
        return bytes.get_mut(offset..offset.checked_add(length)?);
    }
    if is_arm7_address(address) {
        let offset = (address - 0x0380_0000) as usize;
        let bytes = unsafe { emu.arm7.wram.as_mut_arr() };
        return bytes.get_mut(offset..offset.checked_add(length)?);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_standard_codes_and_ignores_titles() {
        let codes = parse("Infinite HP\n;02000000 00000063\nD2000000 00000000").unwrap();
        assert_eq!(codes.len(), 2);
        assert_eq!(codes[0].command, 0x0200_0000);
        assert_eq!(codes[0].value, 0x63);
    }

    #[test]
    fn rejects_hex_like_lines_with_the_wrong_width() {
        assert!(parse("02000000 0000").is_err());
    }

    #[test]
    fn removes_a_complete_code_sequence() {
        let source = parse("02000000 00000001\nD2000000 00000000\n02000004 00000002").unwrap();
        let removed = remove_sequence(&source, &source[..2]);
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].command, 0x0200_0004);
    }
}
