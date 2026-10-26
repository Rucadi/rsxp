# SPDX-License-Identifier: GPL-2.0-or-later
# Wiring checks for the Pokémon Essentials debug-mode operation behind the
# panel toggle. Uses the newer GameData data-model marker; the flag itself
# (`$DEBUG`) is shared by every Essentials release.
require 'json'
require_relative 'game_bridge'
def assert(value, message) = (raise(message) unless value)

module GameData
  class Species
    def self.exists?(_id) = true
  end
end
$player = Struct.new(:name).new('TEST')
api = FANPLAY::GameAPI

original = $DEBUG
begin
  $DEBUG = false
  assert(api.dispatch('pokemon_essentials_debug', ['query'])['debug'] == false, 'query should report the current flag')
  assert(api.dispatch('pokemon_essentials_debug', ['true'])['debug'] == true, 'true should enable debug mode')
  assert($DEBUG == true, 'the game global should be updated')
  assert(api.dispatch('pokemon_essentials_debug', ['toggle'])['debug'] == false, 'toggle should flip debug mode')
  assert(api.dispatch('pokemon_essentials_debug', ['false'])['debug'] == false, 'false should disable debug mode')
  $DEBUG = true
  assert(api.dispatch('pokemon_essentials_debug', [''])['debug'] == true, 'an empty mode should only report')
  assert(api.dispatch('pokemon_essentials_status', [])['debug'] == true, 'status should report the debug flag')
  begin
    api.dispatch('pokemon_essentials_debug', ['maybe'])
    raise 'an invalid debug mode was accepted'
  rescue ArgumentError
  end
ensure
  $DEBUG = original
end
puts 'Debug mode adapter checks passed'
