# SPDX-License-Identifier: GPL-2.0-or-later
# Uses a temporary copy of the save; RSXP_BENCH_ZOOM=3 enables panoramic mode.
# Set RSXP_INPUT for repeatable walking and --frames above RSXP_BENCH_WARMUP.
# RSXP_BENCH_PIXEL_TRACE=1 prints frame digests; its capture cost invalidates timings.
srand(Integer(ENV.fetch('RSXP_BENCH_SEED','0')))
# Saved-overworld performance harness. All save writes/deletes use a temporary
# directory; game data is read through directory symlinks and the save is copied.
require 'tmpdir'
game_root=File.expand_path(ENV['RSXP_POKEMONZ_ROOT'] || '../pokemonz',__dir__)
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
    zoom=Float(ENV.fetch('RSXP_BENCH_ZOOM','1'))
    if zoom>1
      RSXP.call('runtime_patch_value_set','map_panoramic',zoom)
      RSXP.call('runtime_patch_set','map_panoramic',1)
    end
    warmup=Integer(ENV.fetch('RSXP_BENCH_WARMUP','60'))
    raise 'warmup must be positive' unless warmup.positive?
    count=0
    samples=[]
    positions=[]
    started=cpu=allocated=previous=nil
    Graphics.singleton_class.alias_method(:__bench_pokemonz_update,:update)
    Graphics.define_singleton_method(:update) do
      __bench_pokemonz_update
      count+=1
      positions << [$game_player.x,$game_player.y]
      if ENV['RSXP_BENCH_PIXEL_TRACE']=='1' && count>=warmup
        snap=Graphics.snap_to_bitmap
        puts "POKEMONZ_PIXEL:#{count}:#{Zlib.crc32(snap.raw_data)}:#{$game_player.x}:#{$game_player.y}"
        snap.dispose
      end
      now=Process.clock_gettime(Process::CLOCK_MONOTONIC)
      if count==warmup
        GC.start
        started=previous=Process.clock_gettime(Process::CLOCK_MONOTONIC)
        cpu=Process.clock_gettime(Process::CLOCK_PROCESS_CPUTIME_ID)
        allocated=GC.stat(:total_allocated_objects)
        puts "POKEMONZ_SCENE map=#{$game_map.map_id} events=#{$game_map.events.size} x=#{$game_player.x} y=#{$game_player.y} width=#{Graphics.width} height=#{Graphics.height} tilemap=#{$PokemonSystem.tilemap} classes=#{ObjectSpace.each_object(TilemapLoader).map{|t| t.instance_variable_get(:@tilemap).class}.inspect}"
      elsif started
        samples << (now-previous)*1000
        previous=now
      end
    end
    begin
      $scene.main
    ensure
      if started && !samples.empty?
        elapsed=Process.clock_gettime(Process::CLOCK_MONOTONIC)-started
        sorted=samples.sort
        puts "POKEMONZ_BENCH:#{JSON.generate(frames: samples.size, ms_per_frame: elapsed*1000/samples.size, cpu_ms_per_frame: (Process.clock_gettime(Process::CLOCK_PROCESS_CPUTIME_ID)-cpu)*1000/samples.size, p95_ms: sorted[(sorted.size*0.95).floor], max_ms: sorted.last, allocations_per_frame: (GC.stat(:total_allocated_objects)-allocated).to_f/samples.size, final: [$game_map.map_id,$game_player.x,$game_player.y], positions: positions.uniq, retained: RSXP::Retained.stats)}"
      end
    end
  end
end
raise 'original save was changed' unless File.binread(source_save)==original
