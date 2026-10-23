// Assemble embedded Ruby source units using the same order as Rust's
// compile-time concat!(include_str!(...)) declarations.
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const root = resolve(import.meta.dirname, '..');
const declarations = readFileSync(resolve(root, 'crates/rsxp-rgss/src/ruby.rs'), 'utf8');

export function rubySource(name) {
  const match = declarations.match(new RegExp(`pub const ${name}: &str = concat!\\(([\\s\\S]*?)\\);`));
  if (!match) throw new Error(`Missing Ruby source bundle ${name}`);
  const parts = [...match[1].matchAll(/include_str!\("([^"]+)"\)/g)];
  if (!parts.length) throw new Error(`Ruby source bundle ${name} has no units`);
  return parts.map((part) => readFileSync(resolve(root, 'src', part[1]), 'utf8')).join('');
}
