# SPDX-License-Identifier: GPL-2.0-or-later
# Exercise retained ordering independently of pixel content and geometry.
Graphics.resize_screen(64,48)
bitmap=Bitmap.new(16,16)
bitmap.fill_rect(bitmap.rect,Color.new(230,70,30,180))
back=Bitmap.new(8,8)
back.fill_rect(back.rect,Color.new(30,150,220,190))
v1=Viewport.new(0,0,64,48)
v2=Viewport.new(3,2,55,40)
v2.z=1
sprites=Array.new(3) do |i|
  s=Sprite.new(i==2 ? nil : v1)
  s.bitmap=bitmap
  s.x=8+i*4
  s.y=8
  s
end
plane=Plane.new(v2)
plane.bitmap=back
plane.opacity=170
checks=[]
14.times do |frame|
  case frame
  when 1 then sprites[0].y=12             # reorder equal-z children
  when 2 then sprites[1].z=2
  when 3 then sprites[1].viewport=v2      # reparent
  when 4 then v1.z=3; v2.ox=3; v2.rect.x=6
  when 5 then sprites[0].visible=false
  when 6 then sprites[0].visible=true; sprites[0].x=20; sprites[0].opacity=120
  when 7 then bitmap.fill_rect(bitmap.rect,Color.new(70,230,30,160))
  when 8 then sprites[2].dispose; sprites[2]=Sprite.new(v2); sprites[2].bitmap=back
  when 9 then plane.z=4; plane.ox=7; plane.viewport=v1
  when 10 then v2.dispose; sprites[1].viewport=nil; sprites[2].viewport=nil
  when 11 then Graphics.render_size=[80,56]
  when 12 then plane.dispose
  when 13 then sprites[0].dispose; v1.dispose
  end
  Graphics.update
  snap=Graphics.snap_to_bitmap
  pixels=(0...snap.height).flat_map { |y| (0...snap.width).map { |x| snap.get_pixel(x,y).to_a.pack('C*') } }.join
  checks << Zlib.crc32(pixels)
  snap.dispose
end
# Collected wrappers must not hold their engine resources alive; the traversal
# must also recover from dead weak references without swallowing script errors.
100.times { Sprite.new; Plane.new; Viewport.new }
GC.start
Graphics.update
puts "SCENE_CACHE:#{checks.join(',')}"
