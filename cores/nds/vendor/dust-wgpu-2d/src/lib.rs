#![warn(clippy::all)]
#![allow(clippy::missing_safety_doc)]

mod common;
pub mod threaded;
pub use common::gfx::{CpuFrameCapture, Renderer3dRx};
pub use threaded::lockstep_scanlines::FrontendChannels;
