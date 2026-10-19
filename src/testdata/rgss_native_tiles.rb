# SPDX-License-Identifier: GPL-2.0-or-later
# Essentials-compatible tile resources exercise the native adapter and fallback.
class TilemapRenderer
  class TileSprite < Sprite; end
end
RSXP.install_native_tilemap
Graphics.resize_screen(96,80)
bitmap=Bitmap.new(96,64)
64.times { |y| 96.times { |x| bitmap.set_pixel(x,y,Color.new(x*2,y*3,120,x%3==0 ? 100 : 255)) } }
viewport=Viewport.new(0,0,96,80)
second=Viewport.new(2,3,88,70)
second.z=1
tiles=Array.new(18) do |i|
  tile=TilemapRenderer::TileSprite.new(viewport)
  tile.bitmap=bitmap
  tile.src_rect.set((i%3)*32,0,32,32)
  tile.x=(i%3)*32
  tile.y=((i/3)%3)*32
  tile.z=i<9 ? 0 : ((i/3)%3)*32+65
  tile
end
character=Sprite.new(viewport)
character.bitmap=bitmap
character.src_rect.set(64,32,24,24)
character.x=20
character.y=25
character.z=70
checks=[]
12.times do |frame|
  tiles[0].src_rect.x=(frame%3)*32
  tiles[2].blend_type=frame%3
  tiles[3].invert=frame.odd?
  tiles[4].opacity=110+frame
  tiles[5].color.set(frame*9,30,60,80)
  tiles[6].tone.gray=frame*5
  viewport.ox=frame
  character.y=25+frame
  case frame
  when 1 then tiles[10].z=0                         # bridge priority
  when 2 then tiles[11].z=-100                      # reflection ground
  when 3 then viewport.color.alpha=40; viewport.tone.gray=30
  when 4 then bitmap.fill_rect(32,0,32,32,Color.new(30,180,90,150))
  when 5 then tiles[12].angle=17                    # plugin effect fallback
  when 6 then tiles[13].src_rect.width=70           # large custom tile fallback
  when 7 then tiles[14].viewport=second
  when 8 then tiles[15].dispose
  when 9 then viewport.rect.x=4; second.oy=3
  when 10 then Graphics.render_size=[112,88]
  when 11 then second.dispose; tiles[14].viewport=nil
  end
  Graphics.update
  snap=Graphics.snap_to_bitmap
  pixels=(0...snap.height).flat_map { |y| (0...snap.width).map { |x| snap.get_pixel(x,y).to_a.pack('C*') } }.join
  checks << Zlib.crc32(pixels)
  snap.dispose
end
puts "NATIVE_TILES:#{checks.join(',')}"
