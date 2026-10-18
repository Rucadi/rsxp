// SPDX-License-Identifier: GPL-2.0-or-later
//! Build script for the root `rsxp` crate.
//!
//! It converts the three checked-in `binding/module_rpg{1,2,3}.rb.xxd` C byte
//! arrays (hex dumps of the per-RGSS-version Ruby bootstrap scripts) into raw
//! `rpg{1,2,3}.rb` files in `OUT_DIR`, which `rust/engine/commands.rs` embeds
//! with `include_bytes!`. It scans for `0x`-prefixed hex bytes, drops a single
//! trailing NUL so the emitted file is exactly the script text, and registers
//! each source file with `cargo:rerun-if-changed`.
use std::{env, path::PathBuf};

fn main() {
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    for version in 1..=3 {
        let path = format!("binding/module_rpg{version}.rb.xxd");
        println!("cargo:rerun-if-changed={path}");
        let text = std::fs::read_to_string(path).unwrap();
        let mut bytes: Vec<_> = text
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter_map(|word| word.strip_prefix("0x"))
            .map(|hex| u8::from_str_radix(hex, 16).unwrap())
            .collect();
        if bytes.last() == Some(&0) {
            bytes.pop();
        }
        std::fs::write(out.join(format!("rpg{version}.rb")), bytes).unwrap();
    }
}
