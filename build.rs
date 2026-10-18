// SPDX-License-Identifier: GPL-2.0-or-later
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
