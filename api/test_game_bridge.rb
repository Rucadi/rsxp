# SPDX-License-Identifier: GPL-2.0-or-later
# Small Ruby-level regression checks for the game-thread adapter.
require 'json'

module RSXP
  def self.call(*) = raise('unexpected native call in adapter test')
end

eval(File.read(File.join(__dir__, 'game_bridge.rb')), TOPLEVEL_BINDING, 'api/game_bridge.rb')

def assert(condition, message)
  raise message unless condition
end

api = RSXP::GameAPI
api.instance_variable_set(:@keys, {})
api.instance_variable_set(:@key_releases, [])

api.dispatch('press_key', ['up', '2'])
state = {}
api.inject_input(state)
assert((state.fetch(82) & 3) == 3, 'first API key frame should be pressed and triggered')
state = {}
api.inject_input(state)
assert((state.fetch(82) & 1) == 1, 'second API key frame should remain pressed')
state = {}
api.inject_input(state)
assert((state.fetch(82) & 4) == 4, 'the frame after key expiry should report release')

api.instance_variable_set(:@keys, {})
api.instance_variable_set(:@key_releases, [])
api.dispatch('press_key', ['action', '5'])
api.inject_input({})
api.dispatch('release_all_keys', [])
state = {}
api.inject_input(state)
assert((state.fetch(40) & 4) == 4, 'release_all_keys should report a released action key')

pokemon = Struct.new(:iv, :ev, :moves).new(
  [31, 30, 29, 28, 27, 26],
  [252, 0, 4, 0, 0, 252],
  []
)
details = api.pokemon_details(pokemon, 0)
assert(details['ivs'] == {
  'hp' => 31, 'attack' => 30, 'defense' => 29,
  'special_attack' => 28, 'special_defense' => 27, 'speed' => 26
}, 'Pokémon IVs should be named by stat')
assert(details['evs'] == {
  'hp' => 252, 'attack' => 0, 'defense' => 4,
  'special_attack' => 0, 'special_defense' => 0, 'speed' => 252
}, 'Pokémon EVs should be named by stat')

command = Struct.new(:code, :parameters).new(201, [0, 2, 5, 7, 2, 0])
page = Struct.new(:list).new([command])
event = Struct.new(:x, :y, :pages).new(18, 12, [page])
map = Struct.new(:events).new({ 1 => event })
assert(api.map_transfer_edges(1, map) == [[1, 18, 12, 2, 5, 7]],
       'direct Essentials transfer commands should expose source and destination cells')

game_root = File.expand_path('../pokemonz', __dir__)
if File.file?(File.join(game_root, 'PBS', 'metadata.txt'))
  Dir.chdir(game_root) do
    metadata = api.world_map_metadata
    assert(metadata[3]['outdoor'], 'world map metadata should identify outdoor maps')
    assert(metadata[3]['position'] == [0, 18, 17], 'world map metadata should preserve regional positions')
  end
end

module GameData
  class MapMetadata
    def self.try_get(id)
      Struct.new(:outdoor_map, :town_map_position).new(false, [1, 2, 3]) if id == 3
    end
  end
end
compiled_metadata = api.world_map_metadata([3])
assert(compiled_metadata[3]['outdoor'] == false,
       'compiled Essentials metadata should override PBS outdoor state')
assert(compiled_metadata[3]['position'] == [1, 2, 3],
       'compiled Essentials MapPosition should be read from GameData')

module GameData
  Encounter = Struct.new(:types, :step_chances) do
    def self.try_get(id, _version = 0)
      return unless id == 12
      new({ Land: [[4, :PIKACHU, 3, 5], [1, :BIDOOF, 2, 4]], OldRod: [[:KRABBY, 4, 6], [:SHELLDER, 4, 6]], Cave: [[1, 25, 2, 2]] }, { Land: 20, OldRod: 10, Cave: 5 })
    end
  end
  EncounterType = Struct.new(:name) do
    def self.try_get(id) = new(id.to_s.capitalize)
  end
  Species = Struct.new(:name, :id_number) do
    RECORDS = [new('Pikachu', 25), new('Bidoof', 399), new('Krabby', 98), new('Shellder', 90)]
    def self.try_get(id)
      raise ArgumentError, 'expected species ID, not integer' if id.is_a?(Integer)
      RECORDS.find { |record| record.name.upcase == id.to_s }
    end
    def self.each(&block) = RECORDS.each(&block)
  end
end
$Trainer = Struct.new(:name, :id, :money, :badges, :party).new('Tester', 123, 500, [], [])
$game_map = Struct.new(:map_id, :name, :events).new(12, 'Route 12', {})
$PokemonEncounters = Struct.new(:encounter_type).new(:Land)
$PokemonGlobal = Struct.new(:encounter_version).new(2)
encounters = api.dispatch('pokemon_essentials_encounters', [])
assert(encounters['encounter_version'] == 2, 'encounter endpoint should use the active encounter version')
assert(encounters['tables'][0]['entries'][0]['selection_percent'] == 80.0,
       'encounter endpoint should respect Essentials weighted slots')
assert(encounters['tables'][0]['entries'][0]['step_percent'] == 16.0,
       'encounter endpoint should scale step chance by slot weight')
assert(encounters['tables'][0]['entries'][0]['dex_number'] == 25,
       'encounter endpoint should include the Pokédex number used to find its battler image')
assert(encounters['tables'][0]['entries'][0]['species'] == 'PIKACHU' &&
       encounters['tables'][0]['entries'][0]['min_level'] == 3 &&
       encounters['tables'][0]['entries'][0]['max_level'] == 5,
       'weighted encounter rows should decode species and level fields in Essentials order')
assert(encounters['tables'][1]['entries'][0]['selection_percent'] == 50.0,
       'legacy repeated entries should represent equal encounter slots')
assert(encounters['tables'][2]['entries'][0]['name'] == 'Pikachu' && encounters['tables'][2]['entries'][0]['dex_number'] == 25,
       'numeric encounter species IDs should resolve through species Pokédex numbers')
assert(api.dispatch('pokemon_essentials_status', [])['available'],
       'Essentials API status should report when Essentials data is present')

command = Struct.new(:code, :parameters).new(401, ['Welcome to Route 12!'])
page = Struct.new(:trigger, :list, :character_name).new(0, [command], 'hiker')
event_data = Struct.new(:name, :pages).new('Hiker', [page])
npc = Struct.new(:event, :id, :name, :x, :y, :direction).new(event_data, 8, 'Hiker', 9, 6, 2)
$game_map.events[8] = npc
npcs = api.dispatch('pokemon_essentials_npcs', [])
assert(npcs['npcs'][0]['pages'][0]['text_excerpt'] == 'Welcome to Route 12!',
       'NPC endpoint should include dialogue excerpts from event commands')
assert(api.dispatch('pokemon_essentials_battle', [])['active'] == false,
       'battle endpoint should clearly report when there is no active battle')

puts 'game API adapter checks passed'
