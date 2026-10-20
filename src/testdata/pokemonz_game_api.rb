# Opt-in integration test using local, unmodified Pokemon Z scripts and data.
# Loads the game's own scripts, then evaluates the game bridge and exercises the
# Pokédex, Movedex, Itemdex and current-map encounter readers against the
# pre-GameData (PBSpecies/PBMoves/PBItems) layout that Pokemon Z uses.
Dir.chdir(ENV['RSXP_POKEMONZ_ROOT'] || File.expand_path('../../pokemonz', __dir__))
$RGSS_SCRIPTS = load_data('Data/Scripts.rxdata')
$RGSS_SCRIPTS.each do |section|
  section[3] = Zlib::Inflate.inflate(section[2]).force_encoding(Encoding::UTF_8)
  next if section[1] == 'Main'
  RSXP.eval_script(section[3], section[1])
  RSXP.publish_top_level_methods
end
$game_system = Game_System.new
$game_switches = Game_Switches.new
$game_variables = Game_Variables.new
$PokemonGlobal = PokemonGlobalMetadata.new
$PokemonTemp = PokemonTemp.new
$Trainer = PokeBattle_Trainer.new('TEST', 0)
$PokemonEncounters = PokemonEncounters.new
$PokemonEncounters.setup(7)
$game_map = Struct.new(:map_id).new(7)
$PokemonBag = PokemonBag.new
$ItemData = Object.new.__send__(:readItemList, 'Data/items.dat')
$game_temp = Game_Temp.new
class FakeSceneMap < Scene_Map; end
$scene = FakeSceneMap.allocate

bridge = File.expand_path('../../api/game_bridge.rb', __dir__)
eval(File.read(bridge).force_encoding(Encoding::UTF_8), TOPLEVEL_BINDING, 'api/game_bridge.rb', 1)
api = RSXP::GameAPI

def assert(condition, message)
  raise message unless condition
end

raise 'Pokemon Z must use the legacy data path' unless api.legacy_essentials?

# Pokédex: every compiled species, real dex bytes, learnsets and evolutions.
index = api.dispatch('pokemon_essentials_pokedex', ['refresh'])
species = index['species']
raise "incomplete species catalog: #{species.length}" unless species.length > 900
bulba = species.find { |row| row['id'] == 'BULBASAUR' }
raise 'BULBASAUR missing from the catalog' unless bulba
raise "bad name #{bulba['name']}" unless bulba['name'] == 'Bulbasaur'
raise "bad number #{bulba['number']}" unless bulba['number'] == 1
raise "bad types #{bulba['types'].inspect}" unless bulba['types'] == %w[GRASS POISON]
entry = api.dispatch('pokemon_essentials_species_entry', ['BULBASAUR'])
stats = entry['stats']
raise "bad base stats #{stats.inspect}" unless stats['HP'] == 50 && stats['SPECIAL_ATTACK'] == 65 && stats['SPEED'] == 45
raise "bad height/weight #{entry['facts'].inspect}" unless entry['facts']['height_m'] == 0.7 && entry['facts']['weight_kg'] == 6.9
raise "bad abilities #{entry['abilities'].inspect}" unless entry['abilities'].map { |a| a['name'] }.include?('Espesura')
raise 'missing hidden abilities' unless entry['hidden_abilities'].map { |a| a['name'] }.include?('Clorofila')
raise 'missing level-up learns' unless entry['moves'].any? { |m| m['method'] == 'Level-up' && m['level'] == 9 }
raise 'missing TM/tutor learns' unless entry['moves'].any? { |m| m['method'] == 'TM / tutor' }
raise "bad evolutions #{entry['evolutions'].inspect}" unless entry['evolutions'].any? { |e| e['id'] == 'IVYSAUR' && e['method'] == 'Level' && e['parameter'] == 18 }
ivy = api.dispatch('pokemon_essentials_species_entry', ['IVYSAUR'])
raise "bad previous #{ivy['previous'].inspect}" unless ivy['previous'].any? { |e| e['id'] == 'BULBASAUR' }
puts "POKEDEX_OK:#{species.length}:#{entry['moves'].length}"

# Movedex: real move values, categories, flags and descriptions.
movedex = api.dispatch('pokemon_essentials_movedex', ['refresh'])['entries']
raise "incomplete move catalog: #{movedex.length}" unless movedex.length > 600
megahorn = movedex.find { |row| row['id'] == 'MEGAHORN' }
raise 'MEGAHORN missing' unless megahorn
raise "bad MEGAHORN #{megahorn.inspect}" unless megahorn['power'] == 120 && megahorn['accuracy'] == 85 && megahorn['pp'] == 10
raise 'bad MEGAHORN type' unless megahorn['type'] == 'BUG' && megahorn['category_name'] == 'Physical'
raise 'bad MEGAHORN flags' unless megahorn['flags'].include?('Contact')
raise 'missing move description' if megahorn['description'].to_s.empty?
puts "MOVEDEX_OK:#{movedex.length}"

# Itemdex: pockets, use modes, tags and TM-to-move links.
itemdex = api.dispatch('pokemon_essentials_itemdex', ['refresh'])['entries']
raise "incomplete item catalog: #{itemdex.length}" unless itemdex.length > 900
repel = itemdex.find { |row| row['id'] == 'REPEL' }
raise 'REPEL missing' unless repel
raise "bad item name #{repel['name'].inspect}" unless repel['name'] == 'Flor Rep. Pequeña'
raise "bad pocket #{repel['pocket_name'].inspect}" unless repel['pocket_name'] == 'Objetos'
raise "missing item icon #{repel['icon'].inspect}" unless repel['icon'].to_s.start_with?('Graphics/Icons/item')
machine = itemdex.find { |row| row['field_use'] == 3 && row['move'] }
raise 'no TM found' unless machine
raise "TM missing tag #{machine['tags'].inspect}" unless machine['tags'].include?('TM / HM / TR')
raise 'TM must link to a move' unless machine['move']['id'] && machine['move']['name']
puts "ITEMDEX_OK:#{itemdex.length}"

# Clone the TM's move so its Movedex row can be checked for the add-TM action.
move_row = movedex.find { |row| row['id'] == machine['move']['id'] }
raise 'TM move missing from the movedex' unless move_row
raise 'TM move must list its machine' unless Array(move_row['machines']).any? { |m| m['id'] == machine['id'] }
puts "MASCHINE_OK:#{machine['id']}:#{machine['move']['id']}"

# Legacy bag additions resolve item constant names and use the game bag methods.
class LegacyTestBag
  def initialize = (@counts = Hash.new(0))
  def pbQuantity(item) = @counts[item]
  def pbCanStore?(item, quantity) = @counts[item] + quantity <= 10
  def pbStoreItem(item, quantity) = (@counts[item] += quantity; true)
end
$PokemonBag = LegacyTestBag.new
change = api.dispatch('pokemon_essentials_add_item', ['REPEL', '3'])
raise "legacy item lookup failed: #{change.inspect}" unless change['added'] == 3 && change['total'] == 3 && change['complete']
raise "unexpected item name #{change['name'].inspect}" unless change['name'] == 'Flor Rep. Pequeña'
tm_change = api.dispatch('pokemon_essentials_add_item', [machine['id'], '1'])
raise "legacy TM lookup failed: #{tm_change.inspect}" unless tm_change['added'] == 1 && tm_change['complete']
$PokemonBag = nil
puts "INVENTORY_OK:#{change['item']}:#{tm_change['item']}"

# Current-map encounters: real slot weights, levels and species.
data = api.dispatch('pokemon_essentials_encounters', [])
raise 'map 7 should have encounters' unless data['available']
tables = data['tables']
raise "bad tables #{tables.map { |t| t['name'] }.inspect}" unless tables.any? { |t| t['name'] == 'Land' }
land = tables.find { |t| t['name'] == 'Land' }
raise "expected 12 land slots, got #{land['entries'].length}" unless land['entries'].length == 12
first = land['entries'][0]
raise "bad first slot #{first.inspect}" unless first['species'] == 'BIDOOF' && first['dex_number'] == 399
raise 'bad slot levels' unless first['min_level'] == 3 && first['max_level'] == 5
raise 'missing slot share' unless first['selection_percent'].to_f.positive?
raise 'missing step chance' unless land['step_chance'].to_f.positive?
puts "ENCOUNTERS_OK:#{tables.map { |t| t['name'] }.join(',')}"

# Cheat encounters validate their options, schedule a battle and apply every
# option to the wild Pokémon the game generates for that battle.
$PokemonBag = PokemonBag.new
result = api.dispatch('pokemon_essentials_wild_encounter',
  ['BULBASAUR', '25', 'true', '0', 'female', 'JOLLY', '65', 'LEFTOVERS', 'true', '200'])
raise "encounter was not scheduled: #{result.inspect}" unless result['scheduled']
raise "bad encounter name #{result['name'].inspect}" unless result['name'] == 'Bulbasaur'
spec = api.instance_variable_get(:@rsxp_pending_encounter)
raise "bad encounter spec #{spec.inspect}" unless spec['level'] == 25 && spec['shiny'] == true && spec['gender'] == 'female'
wild = Object.new.__send__(:pbGenerateWildPokemon, PBSpecies::BULBASAUR, 25)
api.apply_legacy_encounter_options(wild, spec)
raise 'shiny was not forced' unless wild.isShiny?
raise "nature was not forced (#{wild.nature})" unless wild.nature == PBNatures::JOLLY
raise 'gender was not forced' unless wild.isFemale?
raise "ability was not forced (#{wild.ability})" unless wild.ability == 65
raise "item was not set (#{wild.item})" unless wild.item == PBItems::LEFTOVERS
raise 'friendship was not set' unless wild.happiness == 200
raise "IVs were not maxed (#{wild.iv.inspect})" unless wild.iv == [31, 31, 31, 31, 31, 31]
begin
  api.dispatch('pokemon_essentials_wild_encounter', ['NOTASPECIES', '5'])
  raise 'unknown species was accepted'
rescue ArgumentError
end
begin
  api.dispatch('pokemon_essentials_wild_encounter', ['BULBASAUR', '0'])
  raise 'bad level was accepted'
rescue ArgumentError
end
$scene = Object.new
begin
  api.dispatch('pokemon_essentials_wild_encounter', ['BULBASAUR', '5'])
  raise 'non-overworld request was accepted'
rescue RuntimeError
end
puts "WILD_ENCOUNTER_OK:#{result['species']}:#{spec['level']}"

# Debug mode reads and writes the game's own `$DEBUG` flag.
original_debug = $DEBUG
begin
  api.dispatch('pokemon_essentials_debug', ['true'])
  raise 'debug mode was not enabled' unless $DEBUG
  raise 'debug status was not reported' unless api.dispatch('pokemon_essentials_status', [])['debug'] == true
  api.dispatch('pokemon_essentials_debug', ['toggle'])
  raise 'debug mode was not toggled' if $DEBUG
ensure
  $DEBUG = original_debug
end
puts "DEBUG_MODE_OK:#{$DEBUG ? 'on' : 'off'}"

puts 'POKEMONZ_GAME_API_OK'
