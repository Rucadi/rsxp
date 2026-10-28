# SPDX-License-Identifier: GPL-2.0-or-later
Graphics.resize_screen(32,32)
Graphics.frame_rate=30
skippable=ENV.fetch('FANPLAY_MOVIE_SKIPPABLE')=='1'
start=Graphics.frame_count
Graphics.play_movie(File.join(__dir__,'movie_timing.ogv'),0,skippable)
frames=Graphics.frame_count-start
if skippable
  raise "movie did not skip promptly: #{frames} frames" unless (2..5).cover?(frames)
else
  raise "unskippable movie exited early: #{frames} frames" unless frames>=10
end
puts 'MOVIE_INPUT_OK'
