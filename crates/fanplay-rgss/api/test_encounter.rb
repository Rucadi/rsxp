# SPDX-License-Identifier: GPL-2.0-or-later
# Wiring checks for the Pokédex "Cheats" wild-encounter operation against the
# newer GameData Essentials data model. Older PBS Essentials are covered by the
# opt-in Pokemon Z integration test.
require 'json'
require_relative 'game_bridge'
def assert(value, message) = (raise(message) unless value)

module GameData
  class Species
    DATA = {}
    attr_reader :id, :species, :name, :form
    def initialize(id, species, name, form)
      @id = id
      @species = species
      @name = name
      @form = form
    end
    def self.get(id) = DATA.fetch(id.to_s) { raise "unknown species #{id}" }
    def self.exists?(id) = DATA.key?(id.to_s)
  end
  module Nature
    NAMES = %w[HARDY JOLLY MODEST]
    def self.exists?(id) = NAMES.include?(id.to_s.upcase)
  end
  module Ability
    NAMES = %w[OVERGROW CHLOROPHYLL]
    def self.exists?(id) = NAMES.include?(id.to_s.upcase)
  end
  module Item
    NAMES = %w[LEFTOVERS]
    def self.exists?(id) = NAMES.include?(id.to_s.upcase)
  end
  module Stat
    LIST = %w[HP ATTACK DEFENSE SPECIAL_ATTACK SPECIAL_DEFENSE SPEED]
    def self.each_main(&block) = LIST.each { |stat| block.call(Struct.new(:id).new(stat.to_sym)) }
  end
end
GameData::Species::DATA['BULBASAUR'] = GameData::Species.new(:BULBASAUR, :BULBASAUR, 'Bulbasaur', 0)
GameData::Species::DATA['BULBASAUR_1'] = GameData::Species.new(:BULBASAUR_1, :BULBASAUR, 'Bulbasaur', 1)

class Pokemon
  attr_accessor :form, :shiny, :gender, :nature, :ability, :item, :happiness, :iv
  attr_reader :level, :species
  def initialize(species, level)
    @species = species
    @level = level
    @iv = {}
  end
  def makeMale = (@gender = 0)
  def makeFemale = (@gender = 1)
  def calc_stats = (@calculated = true)
  def calculated? = @calculated
end
class WildBattle
  def self.start(pokemon) = (@started = pokemon)
  def self.started = @started
end
class Scene_Map; end

$player = Struct.new(:name).new('TEST')
$scene = Scene_Map.new
$game_temp = Struct.new(:message_window_showing).new(false)

api = FANPLAY::GameAPI
assert(!api.legacy_essentials?, 'the GameData path should be used')

result = api.dispatch('pokemon_essentials_wild_encounter',
  ['BULBASAUR_1', '30', 'true', '1', 'male', 'JOLLY', 'OVERGROW', 'LEFTOVERS', 'true', '180'])
assert(result['scheduled'], 'the encounter should be scheduled')
assert(result['name'] == 'Bulbasaur' && result['level'] == 30, 'the result should echo the request')
spec = api.instance_variable_get(:@fanplay_pending_encounter)
assert(spec['form'] == 1 && spec['shiny'] == true, 'the options should be parsed')
assert(spec['gender'] == 'male' && spec['nature'] == 'JOLLY', 'gender and nature should be parsed')
assert(spec['ability'] == 'OVERGROW' && spec['item'] == 'LEFTOVERS', 'ability and item should be parsed')

api.start_game_data_wild_encounter(spec)
pokemon = WildBattle.started
assert(pokemon, 'WildBattle.start should receive the built Pokémon')
assert(pokemon.species == :BULBASAUR, 'the base species should be built')
assert(pokemon.form == 1, 'the form should be applied')
assert(pokemon.shiny == true, 'shininess should be applied')
assert(pokemon.gender == 0, 'gender should be applied')
assert(pokemon.nature == 'JOLLY', 'nature should be applied')
assert(pokemon.ability == 'OVERGROW', 'ability should be applied')
assert(pokemon.item == 'LEFTOVERS', 'the held item should be applied')
assert(pokemon.happiness == 180, 'friendship should be applied')
assert(pokemon.iv.values.all? { |value| value == 31 }, 'IVs should be maxed')
assert(pokemon.calculated?, 'stats should be recalculated after options')

# Validation rejects bad input before anything is queued.
[['NOTASPECIES', '5'], ['BULBASAUR', '0'], ['BULBASAUR', '5', '', '', '', 'NOTANATURE'],
 ['BULBASAUR', '5', '', '', '', '', 'NOTANABILITY'], ['BULBASAUR', '5', '', '', '', '', '', 'NOTANITEM']].each do |args|
  begin
    api.dispatch('pokemon_essentials_wild_encounter', args)
    raise "invalid request was accepted: #{args.inspect}"
  rescue ArgumentError
  end
end

# A battle, menu or dialogue open blocks the request.
$game_temp.message_window_showing = true
begin
  api.dispatch('pokemon_essentials_wild_encounter', ['BULBASAUR', '5'])
  raise 'a request with a menu open was accepted'
rescue RuntimeError
end
puts 'Encounter cheat adapter checks passed'
