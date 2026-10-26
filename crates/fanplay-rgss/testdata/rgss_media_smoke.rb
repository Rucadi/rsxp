# SPDX-License-Identifier: GPL-2.0-or-later
require 'tmpdir'
Dir.mktmpdir('fanplay-media') do |dir|
  path=File.join(dir,'fixture.bmp')
  header='BM'+[70,0,54].pack('V3')+[40,2,-2,1,24,0,16,0,0,0,0].pack('VllvvVVllVV')
  pixels=[0,0,255,0,255,0,0,0,255,0,0,255,255,255,0,0].pack('C*')
  File.binwrite(path,header+pixels)
  bitmap=Bitmap.new(path)
  raise 'image dimensions' unless bitmap.width==2 && bitmap.height==2
  raise 'image colors' unless bitmap.get_pixel(0,0).red==255 && bitmap.get_pixel(1,0).green==255
  canvas=Bitmap.new(128,48)
  bounds=canvas.text_size('Añil')
  raise 'UTF-8 text measurement' unless bounds.width>0 && bounds.height>0
  canvas.font.outline=true
  canvas.draw_text(0,0,128,48,'Añil')
  raise 'text rendering empty' unless (0...128).any? { |x| (0...48).any? { |y| canvas.get_pixel(x,y).alpha>0 } }
  Graphics.resize_screen(32,32)
  viewport=Viewport.new(5,5,1,1)
  viewport.z=5
  sprite=Sprite.new(viewport)
  sprite.bitmap=bitmap
  Graphics.update
  capture=Graphics.snap_to_bitmap
  raise 'viewport origin' unless capture.get_pixel(5,5).red==255
  raise 'viewport clipping' unless capture.get_pixel(6,5).green==0
  capture.dispose
  viewport.ox=1
  Graphics.update
  capture=Graphics.snap_to_bitmap
  raise 'viewport scrolling' unless capture.get_pixel(5,5).green==255
  capture.dispose
  sprite.dispose
  viewport.dispose
  bitmap.dispose
  canvas.dispose
end
puts 'MEDIA_OK'
