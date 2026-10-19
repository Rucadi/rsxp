# SPDX-License-Identifier: GPL-2.0-or-later
# Visual demo for the KGL2 compatibility layer (light blending).
#
# Window (press B or C to stop):
#   cargo run --release -- run --script src/testdata/kgl2_demo.rb
# Headless snapshot (.ppm):
#   RSXP_DEMO_FRAMES=1 cargo run --release -- run --script src/testdata/kgl2_demo.rb \
#     --headless --frames 2 --output /tmp/kgl2.ppm
Graphics.resize_screen(512, 384)

# The KGL framebuffer starts fully lit; the light shader subtracts darkness.
framebuffer = Bitmap.new(Graphics.width, Graphics.height)
framebuffer.fill_rect(framebuffer.rect, Color.new(235, 235, 225, 255))

# A radial vignette to subtract: black in the centre, white at the edges.
light = Bitmap.new(256, 256)
128.downto(0) do |radius|
  shade = (255.0 * radius / 128).round
  light.fill_rect(
    Rect.new(128 - radius, 128 - radius, radius * 2, radius * 2),
    Color.new(shade, shade, shade, 255)
  )
end

Win32API.new('KGL2.klib', 'kglLoad', '', 'i').call
Win32API.new('KGL2.klib', 'kglBindFramebuffer', 'i', 'i').call(framebuffer.object_id)
Win32API.new('KGL2.klib', 'kglLightBlending', 'i', 'i').call(1)
Win32API.new('KGL2.klib', 'kglLightShader', 'iiii').call(light.object_id, 32, 16, 255)

sprite = Sprite.new
sprite.bitmap = framebuffer

frames = Integer(ENV['RSXP_DEMO_FRAMES'] || 300)
frames.times do
  Graphics.update
  break if Input.trigger?(Input::B) || Input.trigger?(Input::C)
end

sprite.dispose
light.dispose
framebuffer.dispose
puts 'KGL2_DEMO_OK'
