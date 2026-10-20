# Browser dependencies

Vendored distributions keep the exported player self-contained and the Rust
build independent of Node, npm and CDNs. They are loaded only when used.

| File | Upstream release | Original package file | License |
| --- | --- | --- | --- |
| `zip.js` | `@zip.js/zip.js` 2.24.0 | `dist/zip.min.js` | BSD-3-Clause (`zip.LICENSE`) |
| `webtorrent.js` | `webtorrent` 3.0.21 | `dist/webtorrent.min.js` | MIT (`webtorrent.LICENSE`) |

Sources: https://github.com/gildas-lormeau/zip.js and https://github.com/webtorrent/webtorrent.
The ZIP bundle contains its codecs; the WebTorrent bundle is an ES module.
To update, obtain the pinned npm package tarballs, replace these files and
licenses, update this table and run the import/source tests and full web build.
