// SPDX-License-Identifier: GPL-2.0-or-later
//! Shared RGSS data formats, independent of Ruby and rendering frontends.
//!
//! - [`archive`]: bounded readers for RGSSAD/RGSS2A (version 1) and RGSS3A
//!   (version 3) encrypted game archives.
//! - [`table`]: the signed 16-bit `Table` type and its Ruby Marshal payload
//!   encoding.
//!
//! Both are re-exported at the crate root as `rsxp::archive` and `rsxp::table`.
pub mod archive;
pub mod table;
