# Opt-in Continue regression. All save writes/deletes happen in a temporary
# directory; game data is read through directory symlinks and the save is copied.
require 'tmpdir'
game_root=File.expand_path(ENV['RSXP_POKEMONZ_ROOT'] || '../../pokemonz',__dir__)
Dir.chdir(game_root)
$RGSS_SCRIPTS=load_data('Data/Scripts.rxdata')
$RGSS_SCRIPTS.each do |section|
  section[3]=Zlib::Inflate.inflate(section[2]).force_encoding(Encoding::UTF_8)
  next if section[1]=='Main'
  RSXP.eval_script(section[3],section[1])
  RSXP.publish_top_level_methods
end
source_save=File.expand_path(ENV['RSXP_POKEMONZ_SAVE'] || 'Game.rxdata')
original=File.binread(source_save)
Dir.mktmpdir('rsxp-save-load-') do |save_dir|
  %w[Data Graphics Audio Fonts PBS].each do |directory|
    File.symlink(File.join(game_root,directory),File.join(save_dir,directory))
  end
  File.binwrite(File.join(save_dir,'Game.rxdata'),original)
  RTP.define_singleton_method(:getSaveFileName) { |name| File.join(save_dir,name) }
  Dir.chdir(save_dir) do
    $data_system=load_data('Data/System.rxdata')
    $data_common_events=load_data('Data/CommonEvents.rxdata')
    $data_tilesets=load_data('Data/Tilesets.rxdata')
    $data_animations=load_data('Data/Animations.rxdata')
    class SaveLoadTestScene
      def pbChoose(commands)
        commands.index(_INTL('Continuar')) || raise('Continue is unavailable')
      end
      # Exercise the real loader while skipping its animated menu UI.
      def method_missing(*); end
    end
    screen=PokemonLoad.new(SaveLoadTestScene.new)
    preview=screen.pbTryLoadFile(RTP.getSaveFileName('Game.rxdata'))
    screen.pbStartLoadScreen
    raise 'wrong scene after Continue' unless $scene.is_a?(Scene_Map)
    raise 'loaded the wrong map' unless $game_map.map_id==preview[4]
    raise 'trainer changed during load' unless $Trainer.name==preview[0].name
    raise 'player was not loaded' unless $game_player.is_a?(Game_Player)
    # Run the restored scene far enough to construct its spritesets and render
    # frames. Yield after each update so dialogue cannot block the test.
    Graphics.singleton_class.alias_method(:__save_test_update,:update)
    Graphics.define_singleton_method(:update) do
      __save_test_update
      Fiber.yield
    end
    begin
      world=Fiber.new { $scene.main }
      30.times do
        break unless world.alive?
        world.resume
      end
    ensure
      Graphics.singleton_class.alias_method(:update,:__save_test_update)
      Graphics.singleton_class.remove_method(:__save_test_update)
    end
    puts "SAVE_LOAD_OK:#{$game_map.map_id}:#{$Trainer.party.length}"
  end
end
raise 'original save was changed' unless File.binread(source_save)==original
