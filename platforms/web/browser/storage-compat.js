// SPDX-License-Identifier: GPL-2.0-or-later
// Copy pre-rename values on first read; never delete or overwrite a newer value.
export function readStored(storage, key) {
  const current = storage.getItem(key);
  if (current !== null) return current;
  const legacyKey = key.replace(/^fanplay(?=[.-])/, 'rsxp');
  if (legacyKey === key) return null;
  const legacy = storage.getItem(legacyKey);
  if (legacy !== null) {
    try { storage.setItem(key, legacy); } catch { /* Still allow reading old saves. */ }
  }
  return legacy;
}
