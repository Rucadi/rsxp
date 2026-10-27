//! GBA cheat parsing and execution.
//!
//! Rhai plugins and the desktop control bridge feed cheat text to this module.
//! Three common line formats are supported:
//!
//! - CodeBreaker: 8-digit address plus a 4-digit value (`82005274 0044`).
//! - GameShark / Action Replay: 8-digit words (`02000000 00000063`). Both
//!   plain ("raw") codes and TEA-encrypted GameShark Advance v1/v2 and
//!   Action Replay v3 codes are detected automatically.
//! - VBA: `ADDRESS:VALUE`, where value is 1, 2, or 4 bytes
//!   (`02000000:63`).
//!
//! Codes are parsed into a flat operation list when they are added, so the
//! per-frame interpreter does not allocate. The shared GameShark/PARv3 seed
//! constants come from the publicly documented Datel formats (as also
//! implemented by mGBA); the interpreter itself lives here.
//!
//! Unsupported device features (ROM hooks/patches, button actions, EEPROM
//! commands, and PARv3 block conditions) return an error instead of silently
//! doing nothing. Ordinary RAM/IO writes, fills, arithmetic and one/two-line
//! conditions are supported.

use rustboyadvance_core::arm7tdmi::memory::BusIO;
use rustboyadvance_core::GameBoyAdvance;

const GAMESHARK_SEEDS: [u32; 4] = [0x09F4_FBBD, 0x9681_884A, 0x3520_27E9, 0xF3DE_E5A7];
const PRO_ACTION_REPLAY_SEEDS: [u32; 4] = [0x7AA9_648F, 0x7FAE_6994, 0xC0EF_AAD5, 0x4271_2C57];

const PAR3_COND: u32 = 0x3800_0000;
const PAR3_WIDTH: u32 = 0x0600_0000;
const PAR3_ACTION: u32 = 0xC000_0000;
const PAR3_BASE: u32 = 0xC000_0000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Condition {
    Eq,
    Ne,
    Gt,
    Lt,
    Ge,
    Le,
    And,
}

impl Condition {
    fn matches(self, actual: u32, expected: u32) -> bool {
        match self {
            Self::Eq => actual == expected,
            Self::Ne => actual != expected,
            Self::Gt => actual > expected,
            Self::Lt => actual < expected,
            Self::Ge => actual >= expected,
            Self::Le => actual <= expected,
            Self::And => actual & expected == expected,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Write {
        address: u32,
        width: u8,
        value: u32,
    },
    Fill {
        address: u32,
        width: u8,
        value: u32,
        count: u32,
        stride: u32,
    },
    Add {
        address: u32,
        width: u8,
        value: u32,
    },
    Or {
        address: u32,
        width: u8,
        value: u32,
    },
    And {
        address: u32,
        width: u8,
        value: u32,
    },
    Condition {
        address: u32,
        width: u8,
        value: u32,
        condition: Condition,
        lines: u8,
    },
}

#[derive(Clone, Debug, Default)]
pub struct GbaCheats {
    ops: Vec<Op>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Format {
    GameShark,
    ProActionReplay,
}

enum ParsedLine {
    Vba(Op),
    CodeBreaker(u32, u16),
    Words(u32, u32),
    Ignore,
}

impl GbaCheats {
    /// Adds one or more cheat codes. Returns the total number of compiled
    /// operations, matching the NDS helper's shape.
    pub fn add(&mut self, source: &str) -> Result<usize, String> {
        let parsed = parse(source)?;
        if parsed.is_empty() {
            return Err("cheat code did not contain any supported lines".into());
        }
        self.ops.extend(parsed);
        Ok(self.ops.len())
    }

    pub fn remove(&mut self, source: &str) -> Result<bool, String> {
        let parsed = parse(source)?;
        if parsed.is_empty() {
            return Err("cheat code did not contain any supported lines".into());
        }
        let before = self.ops.len();
        self.ops = remove_sequence(&self.ops, &parsed);
        Ok(before != self.ops.len())
    }

    pub fn clear(&mut self) {
        self.ops.clear();
    }

    /// Applies every active code once. The host calls this immediately before
    /// advancing a frame, so button/condition codes see the current keypad.
    pub fn apply(&self, core: &mut GameBoyAdvance) {
        let mut conditions: Vec<(bool, u8)> = Vec::new();
        for op in &self.ops {
            let active = conditions.iter().all(|(active, _)| *active);
            let mut pushed = None;
            match *op {
                Op::Write {
                    address,
                    width,
                    value,
                } => {
                    if active {
                        write_memory(core, address, u32::from(width), value);
                    }
                }
                Op::Fill {
                    address,
                    width,
                    value,
                    count,
                    stride,
                } => {
                    if active {
                        for index in 0..count {
                            write_memory(
                                core,
                                address.wrapping_add(index.wrapping_mul(stride)),
                                u32::from(width),
                                value,
                            );
                        }
                    }
                }
                Op::Add {
                    address,
                    width,
                    value,
                } => {
                    if active {
                        let current = read_memory(core, address, u32::from(width));
                        write_memory(core, address, u32::from(width), current.wrapping_add(value));
                    }
                }
                Op::Or {
                    address,
                    width,
                    value,
                } => {
                    if active {
                        let current = read_memory(core, address, u32::from(width));
                        write_memory(core, address, u32::from(width), current | value);
                    }
                }
                Op::And {
                    address,
                    width,
                    value,
                } => {
                    if active {
                        let current = read_memory(core, address, u32::from(width));
                        write_memory(core, address, u32::from(width), current & value);
                    }
                }
                Op::Condition {
                    address,
                    width,
                    value,
                    condition,
                    lines,
                } => {
                    let actual = read_memory(core, address, u32::from(width));
                    pushed = Some((active && condition.matches(actual, value), lines));
                }
            }

            for (_, remaining) in &mut conditions {
                *remaining = remaining.saturating_sub(1);
            }
            conditions.retain(|(_, remaining)| *remaining > 0);
            if let Some(condition) = pushed
                && condition.1 > 0
            {
                conditions.push(condition);
            }
        }
    }
}

fn parse(source: &str) -> Result<Vec<Op>, String> {
    let mut ops = Vec::new();
    let mut pending_fill: Option<(u32, u8)> = None;
    for (line_number, original) in source.lines().enumerate() {
        let mut line = original.trim();
        if let Some(comment) = line.find("//") {
            line = &line[..comment];
        }
        line = line.trim().trim_start_matches(';').trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let parsed = parse_line(line, line_number + 1)?;
        if let Some((address, width)) = pending_fill.take() {
            match parsed {
                ParsedLine::Words(op1, op2) => {
                    let count = u32::from((op2 >> 16) as u8).max(1);
                    let stride = (op2 & 0xFFFF).wrapping_mul(u32::from(width));
                    ops.push(Op::Fill {
                        address,
                        width,
                        value: mask(op1, width),
                        count,
                        stride,
                    });
                }
                ParsedLine::Ignore => pending_fill = Some((address, width)),
                _ => {
                    return Err(format!(
                        "line {} must continue the previous fill code with an 8+8-digit line",
                        line_number + 1
                    ));
                }
            }
            continue;
        }

        match parsed {
            ParsedLine::Vba(op) => ops.push(op),
            ParsedLine::CodeBreaker(command, value) => {
                ops.extend(parse_codebreaker(command, value)?);
            }
            ParsedLine::Words(op1, op2) => match detect_and_parse(op1, op2)? {
                Parsed::Ops(mut parsed) => ops.append(&mut parsed),
                Parsed::FillNext { address, width } => pending_fill = Some((address, width)),
            },
            ParsedLine::Ignore => {}
        }
    }
    if pending_fill.is_some() {
        return Err("cheat code ended after a fill header without a value line".into());
    }
    Ok(ops)
}

fn parse_line(line: &str, line_number: usize) -> Result<ParsedLine, String> {
    if let Some((left, right)) = line.split_once(':')
        && let Some(op) = parse_vba(left, right, line_number)?
    {
        return Ok(ParsedLine::Vba(op));
    }

    let compact: String = line
        .chars()
        .filter(|character| !character.is_ascii_whitespace() && *character != '-')
        .collect();
    if !compact
        .chars()
        .all(|character| character.is_ascii_hexdigit())
    {
        // Cheat databases commonly include a title, game ID or prose comment.
        // Keep the NDS behaviour: non-code lines are ignored.
        return Ok(ParsedLine::Ignore);
    }

    match compact.len() {
        12 => {
            let command = parse_hex(&compact[..8], line_number, "command")?;
            let value = u16::try_from(parse_hex(&compact[8..], line_number, "value")?)
                .map_err(|_| format!("value on line {line_number} does not fit in 16 bits"))?;
            Ok(ParsedLine::CodeBreaker(command, value))
        }
        16 => {
            let op1 = parse_hex(&compact[..8], line_number, "command")?;
            let op2 = parse_hex(&compact[8..], line_number, "value")?;
            Ok(ParsedLine::Words(op1, op2))
        }
        _ => Err(format!(
            "cheat line {line_number} must contain 12 or 16 hexadecimal digits (or use ADDRESS:VALUE)"
        )),
    }
}

fn parse_vba(left: &str, right: &str, line_number: usize) -> Result<Option<Op>, String> {
    let address_text: String = left
        .chars()
        .filter(|character| !character.is_ascii_whitespace() && *character != '-')
        .collect();
    let value_text: String = right
        .chars()
        .filter(|character| !character.is_ascii_whitespace() && *character != '-')
        .collect();
    if address_text.is_empty()
        || value_text.is_empty()
        || !address_text
            .chars()
            .all(|character| character.is_ascii_hexdigit())
        || !value_text
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Ok(None);
    }
    if address_text.len() != 8 {
        return Err(format!(
            "VBA address on line {line_number} must contain exactly 8 hexadecimal digits"
        ));
    }
    let width = match value_text.len() {
        2 => 1,
        4 => 2,
        8 => 4,
        _ => {
            return Err(format!(
                "VBA value on line {line_number} must contain 2, 4, or 8 hexadecimal digits"
            ));
        }
    };
    Ok(Some(Op::Write {
        address: parse_hex(&address_text, line_number, "address")?,
        width,
        value: parse_hex(&value_text, line_number, "value")?,
    }))
}

fn parse_hex(text: &str, line_number: usize, part: &str) -> Result<u32, String> {
    u32::from_str_radix(text, 16)
        .map_err(|error| format!("invalid {part} on line {line_number}: {error}"))
}

fn parse_codebreaker(command: u32, value: u16) -> Result<Vec<Op>, String> {
    let kind = command >> 28;
    let address = command & 0x0FFF_FFFF;
    let op = match kind {
        // Game ID/checksum lines are informational and never executed.
        0x0 => return Ok(Vec::new()),
        0x2 => Op::Or {
            address: address & !1,
            width: 2,
            value: u32::from(value),
        },
        0x3 => Op::Write {
            address,
            width: 1,
            value: u32::from(value) & 0xFF,
        },
        0x6 => Op::And {
            address: address & !1,
            width: 2,
            value: u32::from(value),
        },
        0x7 => Op::Condition {
            address: address & !1,
            width: 2,
            value: u32::from(value),
            condition: Condition::Eq,
            lines: 1,
        },
        0x8 => Op::Write {
            address: address & !1,
            width: 2,
            value: u32::from(value),
        },
        0xA => Op::Condition {
            address: address & !1,
            width: 2,
            value: u32::from(value),
            condition: Condition::Ne,
            lines: 1,
        },
        0xB => Op::Condition {
            address: address & !1,
            width: 2,
            value: u32::from(value),
            condition: Condition::Gt,
            lines: 1,
        },
        0xC => Op::Condition {
            address: address & !1,
            width: 2,
            value: u32::from(value),
            condition: Condition::Lt,
            lines: 1,
        },
        0xE => Op::Add {
            address: address & !1,
            width: 2,
            value: u32::from(value),
        },
        0xF => Op::Condition {
            address: address & !1,
            width: 2,
            value: u32::from(value),
            condition: Condition::And,
            lines: 1,
        },
        _ => {
            return Err(format!(
                "unsupported CodeBreaker code type {kind:X} (ROM hooks, fills, encrypted \
                 CodeBreaker, and keypad actions are not supported)"
            ));
        }
    };
    Ok(vec![op])
}

enum Parsed {
    Ops(Vec<Op>),
    FillNext { address: u32, width: u8 },
}

fn detect_and_parse(op1: u32, op2: u32) -> Result<Parsed, String> {
    let (game_shark1, game_shark2) = decrypt(op1, op2, GAMESHARK_SEEDS);
    let (action_replay1, action_replay2) = decrypt(op1, op2, PRO_ACTION_REPLAY_SEEDS);
    let candidates = [
        (
            game_shark_probability(game_shark1, game_shark2),
            Format::GameShark,
            game_shark1,
            game_shark2,
        ),
        (
            action_replay_probability(action_replay1, action_replay2),
            Format::ProActionReplay,
            action_replay1,
            action_replay2,
        ),
        (
            game_shark_probability(op1, op2),
            Format::GameShark,
            op1,
            op2,
        ),
        (
            action_replay_probability(op1, op2),
            Format::ProActionReplay,
            op1,
            op2,
        ),
    ];
    let mut best = candidates[0];
    for candidate in &candidates[1..] {
        if candidate.0 > best.0 {
            best = *candidate;
        }
    }
    match best.1 {
        Format::GameShark => parse_game_shark(best.2, best.3),
        Format::ProActionReplay => parse_action_replay(best.2, best.3),
    }
}

fn parse_game_shark(op1: u32, op2: u32) -> Result<Parsed, String> {
    if op2 == 0x001D_C0DE {
        return Ok(Parsed::Ops(Vec::new()));
    }
    let kind = op1 >> 28;
    let address = op1 & 0x0FFF_FFFF;
    let op = match kind {
        0x0 => Op::Write {
            address,
            width: 1,
            value: op2 & 0xFF,
        },
        0x1 => Op::Write {
            address: address & !1,
            width: 2,
            value: op2 & 0xFFFF,
        },
        0x2 => Op::Write {
            address: address & !3,
            width: 4,
            value: op2,
        },
        0x3 => return Err("GameShark assign-list codes are not supported".into()),
        0x6 => return Err("GameShark ROM patch codes are not supported".into()),
        0x8 => return Err("GameShark keypad action codes are not supported".into()),
        0xD => {
            let condition = match op2 >> 20 {
                0 => Condition::Eq,
                1 => Condition::Ne,
                2 => Condition::Le,
                3 => Condition::Ge,
                _ => return Err("GameShark conditional code has an unknown comparison".into()),
            };
            Op::Condition {
                address: address & !1,
                width: 2,
                value: op2 & 0xFFFF,
                condition,
                lines: 1,
            }
        }
        0xE => Op::Condition {
            address: (op2 & 0x0FFF_FFFF) & !1,
            width: 2,
            value: op1 & 0xFFFF,
            condition: Condition::Eq,
            lines: ((op1 >> 16) & 0xFF).max(1) as u8,
        },
        0xF => return Err("GameShark hook codes are not supported".into()),
        _ => {
            return Err(format!(
                "unsupported GameShark/Action Replay code type {kind:X}"
            ));
        }
    };
    Ok(Parsed::Ops(vec![op]))
}

fn parse_action_replay(op1: u32, op2: u32) -> Result<Parsed, String> {
    if op1 == 0x001D_C0DE || op2 == 0x001D_C0DE {
        return Ok(Parsed::Ops(Vec::new()));
    }
    if op1 == 0xDEAD_FACE {
        // The reseed code changes the TEA key used by later encrypted lines.
        // Accepting it as a no-op would make those lines silently wrong, so
        // reject the set with an explicit message instead.
        return Err("Action Replay reseed codes are not supported".into());
    }
    if op1 == 0 {
        return parse_action_replay_special(op2);
    }

    if op1 & PAR3_COND != 0 {
        let action = op1 & PAR3_ACTION;
        let lines = match action {
            0x0000_0000 => 1,
            0x4000_0000 => 2,
            0x8000_0000 => {
                return Err("Action Replay block conditions are not supported".into());
            }
            _ => return Err("Action Replay disable conditions are not supported".into()),
        };
        let condition = match op1 & PAR3_COND {
            0x0800_0000 => Condition::Eq,
            0x1000_0000 => Condition::Ne,
            0x1800_0000 => Condition::Lt,
            0x2000_0000 => Condition::Gt,
            0x2800_0000 => Condition::Lt,
            0x3000_0000 => Condition::Gt,
            0x3800_0000 => Condition::And,
            _ => return Err("Action Replay condition has an unknown comparison".into()),
        };
        let width = width_from_bits(op1)?;
        return Ok(Parsed::Ops(vec![Op::Condition {
            address: par_address(op1),
            width,
            value: mask(op2, width),
            condition,
            lines,
        }]));
    }

    let width = width_from_bits(op1)?;
    let address = par_address(op1);
    let op = match op1 & PAR3_BASE {
        0x0000_0000 => {
            let value = mask(op2, width);
            let count = if width < 4 {
                ((op2 >> (u32::from(width) * 8)) & 0xFF) + 1
            } else {
                1
            };
            Op::Fill {
                address,
                width,
                value,
                count,
                stride: u32::from(width),
            }
        }
        0x4000_0000 => return Err("Action Replay indirect codes are not supported".into()),
        0x8000_0000 => Op::Add {
            address,
            width,
            value: mask(op2, width),
        },
        0xC000_0000 => {
            if op1 & 0xFF00_0000 == 0xC400_0000 {
                return Err("Action Replay hook codes are not supported".into());
            }
            let io_width = if op1 & 0x0100_0000 != 0 { 1 } else { 2 };
            Op::Write {
                address: 0x0400_0000 | (op1 & 0x00FF_FFFF),
                width: io_width,
                value: mask(op2, io_width),
            }
        }
        _ => return Err("Action Replay code has an unknown base type".into()),
    };
    Ok(Parsed::Ops(vec![op]))
}

fn parse_action_replay_special(op2: u32) -> Result<Parsed, String> {
    match op2 & 0xFF00_0000 {
        0x0000_0000 => Ok(Parsed::Ops(Vec::new())),
        0x0800_0000 => Err("Action Replay slowdown codes are not supported".into()),
        0x1800_0000 | 0x1A00_0000 | 0x1C00_0000 | 0x1E00_0000 => {
            Err("Action Replay ROM patch codes are not supported".into())
        }
        0x1000_0000 | 0x1200_0000 | 0x1400_0000 => {
            Err("Action Replay keypad action codes are not supported".into())
        }
        0x4000_0000 => Err("Action Replay ENDIF without a block condition".into()),
        0x6000_0000 => Err("Action Replay ELSE without a block condition".into()),
        0x8000_0000 | 0x8200_0000 | 0x8400_0000 => {
            let width = match op2 & 0xFF00_0000 {
                0x8000_0000 => 1,
                0x8200_0000 => 2,
                _ => 4,
            };
            Ok(Parsed::FillNext {
                address: par_address(op2),
                width,
            })
        }
        _ => Err("unsupported Action Replay special code".into()),
    }
}

fn width_from_bits(op1: u32) -> Result<u8, String> {
    let width = 1u8 << ((op1 & PAR3_WIDTH) >> 25);
    if width > 4 {
        return Err("Action Replay code uses an invalid operand width".into());
    }
    Ok(width)
}

fn par_address(value: u32) -> u32 {
    (value & 0x000F_FFFF) | ((value << 4) & 0x0F00_0000)
}

fn mask(value: u32, width: u8) -> u32 {
    match width {
        1 => value & 0xFF,
        2 => value & 0xFFFF,
        _ => value,
    }
}

fn decrypt(op1: u32, op2: u32, seeds: [u32; 4]) -> (u32, u32) {
    let mut value1 = op1;
    let mut value2 = op2;
    let mut sum = 0xC6EF_3720u32;
    for _ in 0..32 {
        let mix2 = ((value1 << 4).wrapping_add(seeds[2]))
            ^ value1.wrapping_add(sum)
            ^ (value1 >> 5).wrapping_add(seeds[3]);
        value2 = value2.wrapping_sub(mix2);
        let mix1 = ((value2 << 4).wrapping_add(seeds[0]))
            ^ value2.wrapping_add(sum)
            ^ (value2 >> 5).wrapping_add(seeds[1]);
        value1 = value1.wrapping_sub(mix1);
        sum = sum.wrapping_sub(0x9E37_79B9);
    }
    (value1, value2)
}

fn address_probability(address: u32) -> i32 {
    match address >> 24 {
        0x02 => 0x20,
        0x03 => 0x20,
        0x04 => 0x10,
        0x05..=0x0F => -0x08,
        _ => -0xC0,
    }
}

fn game_shark_probability(op1: u32, op2: u32) -> i32 {
    if op2 == 0x001D_C0DE {
        return 0x100;
    }
    let address = op1 & 0x0FFF_FFFF;
    let mut probability = 0;
    match op1 >> 28 {
        0x0 => {
            probability += 0x20;
            if op2 & 0xFFFF_FF00 != 0 {
                probability -= 0x10;
            }
            probability += address_probability(address);
        }
        0x1 => {
            probability += 0x20;
            if op2 & 0xFFFF_0000 != 0 {
                probability -= 0x10;
            }
            probability += address_probability(address);
        }
        0x2 => {
            probability += 0x20;
            probability += address_probability(address);
        }
        0x3 => probability += 0x10,
        0x6 => {
            probability += 0x20;
            if op2 & 0xCFFF_0000 != 0 {
                probability -= 0x10;
            }
        }
        0x8 => probability += 0x10,
        0xD => {
            probability += 0x20;
            if op2 & 0xFFCF_0000 != 0 {
                probability -= 0x10;
            }
            probability += address_probability(address);
        }
        0xE => {
            probability += 0x20;
            probability += address_probability(op2 & 0x0FFF_FFFF);
            if op1 & 0x0F00_0000 != 0 {
                probability -= 0x10;
            }
        }
        0xF => {
            probability += 0x20;
            if op2 & 0xFFFF_0000 != 0 {
                probability -= 0x10;
            }
        }
        _ => probability -= 0x40,
    }
    probability
}

fn action_replay_probability(op1: u32, op2: u32) -> i32 {
    if op2 == 0x001D_C0DE {
        return 0x100;
    }
    if op1 == 0xDEAD_FACE && op2 & 0xFFFF_0000 == 0 {
        return 0x100;
    }
    let mut probability = 0;
    if op1 == 0 {
        probability += 0x40;
        let address = par_address(op2);
        match op2 & 0xFE00_0000 {
            0x8000_0000 | 0x8200_0000 | 0x8400_0000 => {
                probability += address_probability(address);
            }
            0x1800_0000 | 0x1A00_0000 | 0x1C00_0000 | 0x1E00_0000 => {}
            0x0000_0000 | 0x0800_0000 | 0x1000_0000 | 0x1200_0000 | 0x1400_0000 | 0x4000_0000
            | 0x6000_0000 => {
                if op2 & 0x0100_0000 != 0 {
                    probability -= 0x40;
                }
            }
            _ => probability -= 0x40,
        }
        return probability;
    }

    let width_bits = (op1 & PAR3_WIDTH) >> 25;
    if width_bits >= 3 {
        return 0;
    }
    let width = 1u32 << width_bits;
    if op1 & PAR3_COND != 0 {
        probability += 0x20;
        let mask = width_mask(width);
        if op2 & !mask != 0 {
            probability -= 0x10;
        }
    } else {
        let address = par_address(op1);
        probability += 0x20;
        match op1 & PAR3_BASE {
            0x8000_0000 => {
                let mask = width_mask(width);
                if op2 & !mask != 0 {
                    probability -= 0x10;
                }
                probability += address_probability(address);
                if op1 & 0x0100_0000 != 0 {
                    return 0;
                }
            }
            0x0000_0000 | 0x4000_0000 => {
                probability += address_probability(address);
                if op1 & 0x0100_0000 != 0 {
                    return 0;
                }
            }
            _ => {}
        }
    }
    probability
}

fn width_mask(width: u32) -> u32 {
    if width >= 4 {
        u32::MAX
    } else {
        (1u32 << (width * 8)) - 1
    }
}

fn remove_sequence(source: &[Op], sequence: &[Op]) -> Vec<Op> {
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

#[inline]
pub(crate) fn read_memory(core: &mut GameBoyAdvance, address: u32, width: u32) -> u32 {
    let mut bus = core.cpu.bus.borrow_mut();
    match width {
        1 => bus.read_8(address) as u32,
        2 => bus.read_16(address) as u32,
        _ => bus.read_32(address),
    }
}

#[inline]
pub(crate) fn write_memory(core: &mut GameBoyAdvance, address: u32, width: u32, value: u32) {
    let mut bus = core.cpu.bus.borrow_mut();
    match width {
        1 => bus.write_8(address, value as u8),
        2 => bus.write_16(address, value as u16),
        _ => bus.write_32(address, value),
    }
}

#[inline]
pub(crate) fn read_memory_bytes(core: &mut GameBoyAdvance, address: u32, length: usize) -> Vec<u8> {
    (0..length)
        .map(|offset| read_memory(core, address.wrapping_add(offset as u32), 1) as u8)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(source: &str) -> Vec<Op> {
        parse(source).unwrap()
    }

    fn encrypt(mut value1: u32, mut value2: u32, seeds: [u32; 4]) -> (u32, u32) {
        let mut sum = 0u32;
        for _ in 0..32 {
            sum = sum.wrapping_add(0x9E37_79B9);
            let mix1 = ((value2 << 4).wrapping_add(seeds[0]))
                ^ value2.wrapping_add(sum)
                ^ (value2 >> 5).wrapping_add(seeds[1]);
            value1 = value1.wrapping_add(mix1);
            let mix2 = ((value1 << 4).wrapping_add(seeds[2]))
                ^ value1.wrapping_add(sum)
                ^ (value1 >> 5).wrapping_add(seeds[3]);
            value2 = value2.wrapping_add(mix2);
        }
        (value1, value2)
    }

    #[test]
    fn parses_codebreaker_writes_and_conditions() {
        let ops = parsed("82005274 0044\n32000000 0001\n72000000 0001\n82000000 0002");
        assert_eq!(
            ops[0],
            Op::Write {
                address: 0x0200_5274,
                width: 2,
                value: 0x44
            }
        );
        assert_eq!(
            ops[1],
            Op::Write {
                address: 0x0200_0000,
                width: 1,
                value: 1
            }
        );
        assert_eq!(
            ops[2],
            Op::Condition {
                address: 0x0200_0000,
                width: 2,
                value: 1,
                condition: Condition::Eq,
                lines: 1
            }
        );
    }

    #[test]
    fn parses_raw_gameshark_and_vba_lines() {
        let ops = parsed("02000000 00000063\n12000004 0000ABCD\n02000008:01");
        assert_eq!(
            ops[0],
            Op::Write {
                address: 0x0200_0000,
                width: 1,
                value: 0x63
            }
        );
        assert_eq!(
            ops[1],
            Op::Write {
                address: 0x0200_0004,
                width: 2,
                value: 0xABCD
            }
        );
        assert_eq!(
            ops[2],
            Op::Write {
                address: 0x0200_0008,
                width: 1,
                value: 1
            }
        );
    }

    #[test]
    fn ignores_titles_and_rejects_wrong_widths() {
        let ops = parsed("Infinite money\n;82005274 0044\n00000000 001DC0DE");
        assert_eq!(ops.len(), 1);
        assert!(parse("8200 0044").is_err());
    }

    #[test]
    fn removes_a_complete_sequence() {
        let mut cheats = GbaCheats::default();
        cheats.add("82000000 0001\n82000002 0002").unwrap();
        assert!(cheats.remove("82000000 0001").unwrap());
        assert_eq!(cheats.ops.len(), 1);
        assert_eq!(
            cheats.ops[0],
            Op::Write {
                address: 0x0200_0002,
                width: 2,
                value: 2
            }
        );
    }

    #[test]
    fn decrypts_tea_round_trip() {
        // Encrypting with the inverse rounds and decrypting must recover the
        // original words. This guards the fixed-point constants and order.
        let plain = (0x0200_0000, 0x0000_0063);
        let encrypted = encrypt(plain.0, plain.1, GAMESHARK_SEEDS);
        assert_eq!(decrypt(encrypted.0, encrypted.1, GAMESHARK_SEEDS), plain);
    }

    #[test]
    fn autodetects_encrypted_gameshark_codes() {
        let (op1, op2) = encrypt(0x0200_0000, 0x63, GAMESHARK_SEEDS);
        let ops = parsed(&format!("{op1:08X} {op2:08X}"));
        assert_eq!(
            ops,
            vec![Op::Write {
                address: 0x0200_0000,
                width: 1,
                value: 0x63
            }]
        );
    }
}
