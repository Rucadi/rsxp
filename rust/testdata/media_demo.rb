# SPDX-License-Identifier: GPL-2.0-or-later
# Visual smoke test for the media decoders (rust/media).
#
# Animated GIF in a window (press B or C to stop):
#   cargo run --release -- run --script rust/testdata/media_demo.rb
#
# Theora movie in a window (any .ogv path, absolute or relative to the cwd):
#   RSXP_MOVIE=/path/to/intro.ogv cargo run --release -- run --script rust/testdata/media_demo.rb
#
# Headless snapshot of the last displayed frame (.ppm):
#   cargo run --release -- run --script rust/testdata/media_demo.rb \
#     --headless --frames 30 --output /tmp/media.ppm
movie = ENV['RSXP_MOVIE'].to_s

Graphics.resize_screen(512, 384)
sprite = Sprite.new

if !movie.empty?
  volume = (ENV['RSXP_MOVIE_VOLUME'] || 90).to_i
  puts "movie=#{movie}"
  Graphics.play_movie(movie, volume, true)
  puts 'MOVIE_DONE'
else
  path = ENV['RSXP_GIF'].to_s
  if path.empty?
    path = File.expand_path('../../ericlostie/Graphics/UI/evolution_bg.gif', __dir__)
  end
  bitmap = Bitmap.new(path)
  raise "not an animated GIF: #{path}" unless bitmap.animated?
  puts "gif=#{path}"
  puts "frames=#{bitmap.frame_count} frame_rate=#{format('%.2f', bitmap.frame_rate)} size=#{bitmap.width}x#{bitmap.height}"
  sprite.bitmap = bitmap
  sprite.x = (Graphics.width - bitmap.width) / 2
  sprite.y = (Graphics.height - bitmap.height) / 2
  frames = Integer(ENV['RSXP_DEMO_FRAMES'] || 600)
  frames.times do
    Graphics.update
    break if Input.trigger?(Input::B) || Input.trigger?(Input::C)
  end
  frame = bitmap.current_frame
  bitmap.dispose
  puts "GIF_DONE:frame=#{frame}"
end

sprite.dispose
