// Assemble embedded Ruby source units using the same order as Rust's
// compile-time concat!(include_str!(...)) declarations.
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const root = resolve(import.meta.dirname, '..');
const sourceRoot = resolve(root, 'crates/fanplay-rgss/src');
const declarations = readFileSync(resolve(sourceRoot, 'ruby.rs'), 'utf8');

export function rubySource(name) {
  const match = declarations.match(new RegExp(`pub const ${name}: &str = concat!\\(([\\s\\S]*?)\\);`));
  if (!match) throw new Error(`Missing Ruby source bundle ${name}`);
  const parts = [...match[1].matchAll(/include_str!\("([^"]+)"\)/g)];
  if (!parts.length) throw new Error(`Ruby source bundle ${name} has no units`);
  // include_str! paths in ruby.rs are relative to that file's directory.
  return parts.map((part) => readFileSync(resolve(sourceRoot, part[1]), 'utf8')).join('');
}
