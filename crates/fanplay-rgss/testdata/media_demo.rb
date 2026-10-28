# SPDX-License-Identifier: GPL-2.0-or-later
# Visual smoke test for the media decoders (src/media).
#
# Animated GIF in a window (press B or C to stop):
#   FANPLAY_GIF=/path/to/animated.gif cargo run --release -- run --script src/testdata/media_demo.rb
#
# Theora movie in a window (any .ogv path, absolute or relative to the cwd):
#   FANPLAY_MOVIE=/path/to/intro.ogv cargo run --release -- run --script src/testdata/media_demo.rb
#
# Headless snapshot of the last displayed frame (.ppm):
#   cargo run --release -- run --script src/testdata/media_demo.rb \
#     --headless --frames 30 --output /tmp/media.ppm
movie = ENV['FANPLAY_MOVIE'].to_s

Graphics.resize_screen(512, 384)
sprite = Sprite.new

if !movie.empty?
  volume = (ENV['FANPLAY_MOVIE_VOLUME'] || 90).to_i
  puts "movie=#{movie}"
  Graphics.play_movie(movie, volume, true)
  puts 'MOVIE_DONE'
else
  path = ENV['FANPLAY_GIF'].to_s
  raise 'set FANPLAY_GIF to an animated GIF (or FANPLAY_MOVIE to a movie file)' if path.empty?
  bitmap = Bitmap.new(path)
  raise "not an animated GIF: #{path}" unless bitmap.animated?
  puts "gif=#{path}"
  puts "frames=#{bitmap.frame_count} frame_rate=#{format('%.2f', bitmap.frame_rate)} size=#{bitmap.width}x#{bitmap.height}"
  sprite.bitmap = bitmap
  sprite.x = (Graphics.width - bitmap.width) / 2
  sprite.y = (Graphics.height - bitmap.height) / 2
  frames = Integer(ENV['FANPLAY_DEMO_FRAMES'] || 600)
  frames.times do
    Graphics.update
    break if Input.trigger?(Input::B) || Input.trigger?(Input::C)
  end
  frame = bitmap.current_frame
  bitmap.dispose
  puts "GIF_DONE:frame=#{frame}"
end

sprite.dispose
