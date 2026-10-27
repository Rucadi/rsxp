// SPDX-License-Identifier: GPL-2.0-or-later
//! Desktop save-copy adapter.
//!
//! The implementation lives in `fanplay-provision::backup` so the launcher UI, the
//! control API follows the same staging and safety
//! rules. This module keeps the historical `crate::saves::copy` path stable for
//! the desktop workspace.

#![allow(unused_imports)]

pub use fanplay_provision::backup::{
    backup_root, copy, create, default_name, delete, list, restore, BackupInfo,
};
