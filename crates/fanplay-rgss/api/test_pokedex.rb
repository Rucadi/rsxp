# SPDX-License-Identifier: GPL-2.0-or-later
require 'json'
require_relative 'game_bridge'
def assert(value, message) = (raise(message) unless value)

module GameData
  Species = Struct.new(:id, :species, :name, :form, :form_name, :types, :base_stats, :moves,
                       :tutor_moves, :egg_moves, :abilities, :hidden_abilities, :evolutions,
                       :egg_groups, :pokedex_entry, :height, :weight, keyword_init: true) do
    DATA = {}
    def self.each(&block) = DATA.each_value(&block)
    def self.try_get(id) = DATA[id.to_s.to_sym]
    def get_baby_species = :BULBASAUR
  end
  Encounter = Struct.new(:map, :version, :types) do
    def self.each
      yield new(7, 0, { Land: [[8, :BULBASAUR, 3, 6], [2, :OTHER, 4, 5]] })
      yield new(8, 2, { Cave: [[:BULBASAUR, 7, 9], [:BULBASAUR, 8, 10]] })
    end
  end
  Move = Struct.new(:id, :name, :type, :category, :power, :accuracy, :total_pp, :description) do
    def self.try_get(id) = new(id, 'Game-specific move', :GRASS, 1, 93, 95, 12, 'Custom move description.')
  end
  Ability = Struct.new(:name, :description) do
    def self.try_get(id) = new(id.to_s.capitalize, 'Custom ability description.')
  end
  EncounterType = Struct.new(:name) do
    def self.try_get(id) = new(id.to_s)
  end
end
base = { species: :BULBASAUR, name: 'Bulbasaur', form: 0, types: [:GRASS, :POISON],
         base_stats: { HP: 53, ATTACK: 50, DEFENSE: 49, SPECIAL_ATTACK: 70, SPECIAL_DEFENSE: 65, SPEED: 45 },
         moves: (1..250).map { |level| [level, :VINEWHIP] }, tutor_moves: [:SOLARBEAM], egg_moves: [:CURSE],
         abilities: [:OVERGROW], hidden_abilities: [:CHLOROPHYLL], egg_groups: [:Monster, :Grass],
         pokedex_entry: 'This game has its own Bulbasaur.', height: 7, weight: 69 }
GameData::DATA[:BULBASAUR] = GameData::Species.new(**base, id: :BULBASAUR, evolutions: [[:IVYSAUR, :Level, 16, false]])
GameData::DATA[:IVYSAUR] = GameData::Species.new(**base.merge(name: 'Ivysaur', species: :IVYSAUR), id: :IVYSAUR, evolutions: [[:BULBASAUR, :Level, 16, true]])
GameData::DATA[:BULBASAUR_1] = GameData::Species.new(**base.merge(form: 1, form_name: 'Custom form'), id: :BULBASAUR_1, evolutions: [])
GameData::DATA[:OTHER] = GameData::Species.new(**base.merge(name: 'Other', species: :OTHER), id: :OTHER, evolutions: [])
GameData::DATA[:MYSTERY] = GameData::Species.new(**base.merge(name: 'Mystery', species: :MYSTERY), id: :MYSTERY, evolutions: [])
api = FANPLAY::GameAPI
api.define_singleton_method(:load_game_data) { |_| { 7 => Struct.new(:name).new('Greenwood Trail'), 8 => Struct.new(:name).new('Mossy Cavern') } }
api.define_singleton_method(:trainer) { nil }
index = api.dispatch('pokemon_essentials_pokedex', ['refresh'])
assert(index['species'].length == 5, 'Catalog must preserve custom forms and source-unknown species')
assert(index['species'].find { |row| row['id'] == 'IVYSAUR' }['source'] == 'Evolution', 'Known evolution routes should be indexed')
assert(index['species'].find { |row| row['id'] == 'MYSTERY' }['source'].nil?, 'Undefined availability must not be called obtainable')
entry = api.dispatch('pokemon_essentials_species_entry', ['BULBASAUR'])
assert(entry['stats']['HP'] == 53, 'Use modified game stats instead of canonical external data')
assert(entry['locations'].map { |row| row['map_name'] } == ['Greenwood Trail', 'Mossy Cavern'], 'Search all maps and versions')
assert(entry['locations'][0]['percent'] == 80.0, 'Aggregate weighted table share')
assert(entry['locations'][1]['percent'] == 100.0 && entry['locations'][1]['min_level'] == 7 && entry['locations'][1]['max_level'] == 10, 'Handle legacy rows and merge repeated slots')
assert(entry['moves'].length == 252 && entry['moves'][0]['power'] == 93, 'Preserve the full learnset and game-specific move data')
assert(entry['evolutions'][0]['method'] == 'Level' && entry['evolutions'][0]['parameter'] == 16, 'Preserve exact evolution rules')
assert(api.pokemon_essentials_species_entry('IVYSAUR')['previous'][0]['id'] == 'BULBASAUR', 'Reverse evolution links should navigate to the source')
assert(api.pokemon_essentials_species_entry('BULBASAUR_1')['form_name'] == 'Custom form', 'Preserve form-specific data')
assert(entry['abilities'][0]['description'] == 'Custom ability description.', 'Read ability descriptions from the game')
if ENV['FANPLAY_POKEDEX_FIXTURE']
  File.write(ENV['FANPLAY_POKEDEX_FIXTURE'], JSON.pretty_generate({ 'index' => index, 'entry' => entry }))
end
puts 'Pokédex adapter checks passed'
