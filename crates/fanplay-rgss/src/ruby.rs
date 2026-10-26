// SPDX-License-Identifier: GPL-2.0-or-later
//! Ruby source bundles assembled from their maintained, readable units.
//!
//! `concat!` keeps this assembly in Rust's compile-time pipeline: no generated
//! copy of either entry script can drift from the sources below.
pub const RGSS_SOURCE: &str = concat!(
    include_str!("ruby/core/bootstrap.rb"),
    include_str!("ruby/core/game_helpers.rb"),
    include_str!("ruby/core/marshal.rb"),
    include_str!("ruby/rgss/audio.rb"),
    include_str!("ruby/rgss/system.rb"),
    include_str!("ruby/rgss/values.rb"),
    include_str!("ruby/rgss/table.rb"),
    include_str!("ruby/rgss/resources.rb"),
    include_str!("ruby/rgss/bitmap.rb"),
    include_str!("ruby/rgss/viewport.rb"),
    include_str!("ruby/rgss/plane.rb"),
    include_str!("ruby/rgss/sprite.rb"),
    include_str!("ruby/rgss/tilemap.rb"),
    include_str!("ruby/rgss/graphics.rb"),
    include_str!("ruby/rgss/runtime_patches.rb"),
    include_str!("ruby/rgss/input.rb"),
    include_str!("ruby/compat/kernel.rb"),
    include_str!("ruby/compat/scancodes.rb"),
    include_str!("ruby/compat/win32_input.rb"),
    include_str!("ruby/compat/kgl2.rb"),
    include_str!("ruby/compat/win32_impl.rb"),
    include_str!("ruby/compat/win32_api.rb"),
    include_str!("ruby/core/script_loader.rb"),
);

// The game-specific API is now a semi-external Rhai extension package. Keep
// this endpoint as an empty compatibility response for older callers; the
// core Ruby shim still owns the generic request transport and the extension
// injects `extensions/essentials-game-api/game_api.rb` after startup.
pub const GAME_BRIDGE_SOURCE: &str = "";
