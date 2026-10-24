// SPDX-License-Identifier: GPL-2.0-or-later
// File-picker adapter. Assets keep their original File objects until preloaded;
// the running game reads the engine's virtual files with no network fallback.
export function createLocalGame(files, { playerName = 'player.html' } = {}) {
  const index = new Map();
  let folder = '';
  for (const file of files) {
    const relative = (file.webkitRelativePath || file.name).replaceAll('\\', '/');
    const slash = relative.indexOf('/');
    if (slash < 0) continue;
    folder ||= relative.slice(0, slash);
    if (relative.slice(0, slash) !== folder) throw new Error('Choose a single game folder.');
    const path = relative.slice(slash + 1);
    if (!path || path.split('/').some(part => part.startsWith('.')) || path.toLowerCase() === playerName.toLowerCase()) continue;
    const key = path.toLowerCase();
    if (index.has(key)) throw new Error(`Ambiguous asset names: ${index.get(key).path} and ${path}`);
    index.set(key, { path, file });
  }
  if (!index.size) throw new Error('Choose the game folder itself, containing Game.ini or Data.');
  const archives = [...index.values()].filter(entry => /^game\.rgss(?:ad|2a|3a)$/i.test(entry.path)).map(entry => entry.path);
  const ini = index.get('game.ini');
  return {
    folder,
    releaseSource() { index.clear(); },
    // Full disk paths are deliberately unavailable to browser pages. Folder
    // names partition the save bundles inside this player's local storage.
    storageId: `folder:${folder}`,
    paths: [...index.values()].map(entry => entry.path),
    archives,
    has: path => index.has(path.replaceAll('\\', '/').replace(/^\.\//, '').toLowerCase()),
    async read(path) {
      const entry = index.get(path.replaceAll('\\', '/').replace(/^\.\//, '').toLowerCase());
      if (!entry) throw new Error(`Game file not found: ${path}`);
      return new Uint8Array(await entry.file.arrayBuffer());
    },
    async validate() {
      const text = ini ? await ini.file.text() : '';
      const configured = text.match(/^\s*Scripts\s*=\s*(.*?)\s*$/im)?.[1]?.replaceAll('\\', '/');
      const scripts = configured || 'Data/Scripts.rxdata';
      if (!archives.length && !index.has(scripts.toLowerCase())) {
        throw new Error(`Cannot find ${scripts}. Choose the folder containing Game.ini and Data.`);
      }
    },
    rubyManifest(warm, scriptPath) {
      // Keep Ruby's directory-visible fonts and Ruby sources, while large
      // graphics/audio remain only in the engine VFS, as in the served build.
      const wanted = new Set([...warm, scriptPath, 'Game.ini'].map(path => path.toLowerCase()));
      return [...index.values()].filter(({ path }) => wanted.has(path.toLowerCase()) || /^fonts\//i.test(path) || /\.rb$/i.test(path)).map(entry => entry.path);
    },
  };
}
