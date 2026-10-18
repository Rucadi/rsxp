# Opt-in integration test using local, unmodified Pokemon Z scripts and data.
# Skip Main's interactive loop and suppress only the starter's dialogue UI.
Dir.chdir(ENV['RSXP_POKEMONZ_ROOT'] || File.expand_path('../../pokemonz',__dir__))
$RGSS_SCRIPTS=load_data('Data/Scripts.rxdata')
$RGSS_SCRIPTS.each do |section|
  section[3]=Zlib::Inflate.inflate(section[2]).force_encoding(Encoding::UTF_8)
  next if section[1]=='Main'
  RSXP.eval_script(section[3],section[1])
  RSXP.publish_top_level_methods
end
$game_system=Game_System.new
$game_switches=Game_Switches.new
$game_variables=Game_Variables.new
$PokemonGlobal=PokemonGlobalMetadata.new
$PokemonTemp=PokemonTemp.new
[:FENNEKIN,:CHESPIN,:FROAKIE].each do |species|
  pokemon=PokeBattle_Pokemon.new(species,5)
  raise 'wrong starter level' unless pokemon.level==5
  raise 'invalid growth rate' unless (0...6).cover?(pokemon.growthrate)
  raise 'invalid stats' unless pokemon.totalhp>0 && pokemon.attack>0
  raise 'missing starter moves' unless pokemon.moves.any? { |move| move.id>0 }
  puts "STARTER_OK:#{species}:#{pokemon.level}:#{pokemon.growthrate}:#{pokemon.totalhp}"
end
$Trainer=PokeBattle_Trainer.new('TEST',0)
class << Kernel
  def pbMessage(*); end
  def pbConfirmMessage(*) = false
end
raise 'starter event failed' unless pbAddPokemon(:FENNEKIN,5)
pokemon=$Trainer.party.last
raise 'starter not stored' unless pokemon && pokemon.species==PBSpecies::FENNEKIN && pokemon.level==5
puts 'STARTER_EVENT_OK:FENNEKIN:5'
original_center=Game_Player.instance_method(:center)
RSXP.call('runtime_patch_set','map_panoramic',1)
RuntimePatches.sync
raise 'legacy patch failed' unless RuntimePatches.active?('map_panoramic')
raise 'legacy camera changed' unless Game_Player.instance_method(:center)==original_center
puts 'POKEMONZ_PANORAMIC_HOOKS_OK'

# The real title-screen planes are proxy subclasses with no native Plane ID.
[LargePlane,AnimatedPlane].each do |klass|
  plane=klass.new(nil)
  raise 'proxy plane unexpectedly owns a native ID' unless plane.__id.nil?
  2.times { plane.dispose }
  raise "#{klass} disposal failed" unless plane.disposed?
end
Graphics.update
puts 'POKEMONZ_PROXY_PLANES_OK'

# Exercise the failing title transition with its real graphics and fade-out.
title=GenTwoStyle.new
3.times { title.update; Graphics.update }
title.dispose
raise 'title disposal failed' unless title.disposed?
puts 'POKEMONZ_TITLE_DISPOSE_OK'
