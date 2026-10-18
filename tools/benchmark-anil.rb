# SPDX-License-Identifier: GPL-2.0-or-later
# Read an existing Pokémon Añil save, bypass the menu, and time scene updates.
# Supply RSXP_BENCH_SAVE and the runtime's --frames limit; this harness never saves.
# RSXP_BENCH_ZOOM_SWEEP=1 drives the "map panoramic mode" zoom slider once per
# measured frame, to time live zoom changes (requires RSXP_PATCH=map_panoramic).
# For repeatable visual captures, also set RSXP_BENCH_SEED=0 and
# RSXP_BENCH_CLOCK_START=0 (with RSXP_BENCH_FIXED_CLOCK=1).
# RSXP_BENCH_ZOOM=3 enables panoramic mode at 3x before entering the map.
srand(Integer(ENV['RSXP_BENCH_SEED'])) if ENV['RSXP_BENCH_SEED']
save = File.expand_path(ENV.fetch('RSXP_BENCH_SAVE'))
warmup = Integer(ENV.fetch('RSXP_BENCH_WARMUP','150'))
raise ArgumentError, 'warmup must be positive' unless warmup.positive?
Dir.chdir(ENV.fetch('RSXP_BENCH_GAME', File.expand_path('../ericlostie', __dir__)))
title = File.read('Game.ini')[/^Title=(.*)$/,1]&.strip
ENV['RSXP_GAME_TITLE'] = title if title

load 'preload.rb'
$RGSS_SCRIPTS=load_data('Data/Scripts.rxdata')
$RGSS_SCRIPTS.each { |s| s[3]=Zlib.inflate(s[2]).force_encoding(Encoding::UTF_8) }
$RGSS_SCRIPTS.each do |s|
  source=s[1]=='Main' ? s[3].split(/^loop do/,2).first : s[3]
  eval(source,TOPLEVEL_BINDING,s[1],1)
  RSXP.publish_top_level_methods
end
MessageTypes.load_default_messages if FileTest.exist?('Data/messages_core.dat')
PluginManager.runPlugins
Compiler.main
# Essentials movement uses System.uptime. A fixed simulation clock makes
# scripted movement comparable even when different binaries run uncapped.
if ENV['RSXP_BENCH_FIXED_CLOCK'] == '1'
  $benchmark_game_time=Float(ENV.fetch('RSXP_BENCH_CLOCK_START') { System.uptime })
  module System
    def self.uptime = $benchmark_game_time
  end
end
Game.initialize
Game.set_up_system
module Game
  def self.save(*) = raise('Saving disabled in profiling harness')
  def self.auto_save(*) = nil
end
Game.load(Marshal.load(File.binread(save)))
if ENV['RSXP_BENCH_ZOOM']
  $benchmark_zoom=Float(ENV['RSXP_BENCH_ZOOM'])
  raise ArgumentError, 'zoom must be between 1 and 3' unless (1.0..3.0).cover?($benchmark_zoom)
  RSXP.call('runtime_patch_value_set','map_panoramic',$benchmark_zoom)
  RSXP.call('runtime_patch_set','map_panoramic',1)
end
puts "BENCH_SCENE map=#{$game_map.map_id} events=#{$game_map.events.size} x=#{$game_player.x} y=#{$game_player.y}"
$benchmark_game_step=1.0/Graphics.frame_rate
$benchmark_warmup=warmup
$benchmark_updates=0
class << Graphics
  alias __bench_update update
  def update
    __bench_update
    $benchmark_game_time+=$benchmark_game_step if $benchmark_game_time
    $benchmark_updates+=1
    if $benchmark_zoom && $benchmark_updates >= $benchmark_warmup
      raise 'benchmark left the panoramic world map' unless MapPanoramic.on_map? && MapPanoramic.applied? && MapPanoramic.zoom==$benchmark_zoom
    end
    if $benchmark_updates==$benchmark_warmup
      if $benchmark_zoom
        raise 'panoramic benchmark did not activate' unless RuntimePatches.active?('map_panoramic') && MapPanoramic.applied?
        raise 'panoramic benchmark has the wrong zoom' unless MapPanoramic.zoom==$benchmark_zoom
      end
      puts "BENCH_RENDER width=#{Graphics.width} height=#{Graphics.height} zoom=#{$benchmark_zoom || 1}"
      Graphics.snap_to_bitmap.dispose
      GC.start
      $benchmark_allocations=GC.stat(:total_allocated_objects)
      $benchmark_gc=GC.stat(:count)
      $benchmark_start=Process.clock_gettime(Process::CLOCK_MONOTONIC)
      $benchmark_cpu_start=Process.clock_gettime(Process::CLOCK_PROCESS_CPUTIME_ID)
    end
  end
end
begin
  $scene.main
ensure
  puts "BENCH_FINAL map=#{$game_map.map_id} x=#{$game_player.x} y=#{$game_player.y}"
  if $benchmark_start
    Graphics.snap_to_bitmap.dispose
    elapsed=Process.clock_gettime(Process::CLOCK_MONOTONIC)-$benchmark_start
    frames=$benchmark_updates-$benchmark_warmup
    cpu=Process.clock_gettime(Process::CLOCK_PROCESS_CPUTIME_ID)-$benchmark_cpu_start
    allocated=GC.stat(:total_allocated_objects)-$benchmark_allocations
    puts "OVERWORLD_BENCH frames=#{frames} elapsed_s=#{elapsed.round(4)} ms_per_frame=#{(elapsed*1000/frames).round(4)} cpu_ms_per_frame=#{(cpu*1000/frames).round(4)} allocations_per_frame=#{(allocated.to_f/frames).round(2)} gc_cycles=#{GC.stat(:count)-$benchmark_gc}"
  end
end
