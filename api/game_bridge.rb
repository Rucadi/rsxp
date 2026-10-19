# SPDX-License-Identifier: GPL-2.0-or-later
# Convenience loader for Ruby-level tests and direct tooling. The application
# embeds the same ordered units from rust/ruby.rs at compile time.
require_relative 'game_bridge/game_api'
require_relative 'game_bridge/pokedex'
require_relative 'game_bridge/dex_registries'
require_relative 'game_bridge/inventory'
require_relative 'game_bridge/legacy_essentials'
