# SPDX-License-Identifier: GPL-2.0-or-later
GC.auto_compact=true if ENV['FANPLAY_TEST_COMPACT']=='1'
Graphics.resize_screen(96,64)
class LegacyPolledViewport < Viewport
  def ox = super
end
viewport=LegacyPolledViewport.new(0,0,96,64)
map=Tilemap.new(viewport)
map.map_data=Table.new(10,8,3)
map.priorities=Table.new(512)
map.priorities[385]=2
map.tileset=Bitmap.new(256,64)
16.times { |i| map.tileset.fill_rect(i%8*32,i/8*32,32,32,Color.new(i*13,180-i*7,40+i*8)) }
map.autotiles[0]=Bitmap.new(192,128)
12.times { |x| 8.times { |y| map.autotiles[0].fill_rect(x*16,y*16,16,16,Color.new(x*19,y*29,190,170)) } }
10.times { |x| 8.times { |y| map.map_data[x,y,0]=384+(x+y)%16;map.map_data[x,y,1]=48+(x+y)%48 if y.even? } }
character=Sprite.new(viewport)
character.bitmap=Bitmap.new(12,20)
character.bitmap.fill_rect(character.bitmap.rect,Color.new(240,30,30))
character.x=35;character.y=25;character.z=65
checks=[]
45.times do |frame|
  map.ox=(frame%22)*3-9
  map.oy=(frame%18)*2-5
  case frame
  when 3 then map.map_data[1,1,0]=385
  when 5 then map.priorities[385]=0
  when 7 then map.color.set(50,20,80,70)
  when 9 then map.tone.gray=70
  when 11 then viewport.ox=3;viewport.rect.width=80
  when 13 then viewport.color.alpha=40
  when 15 then viewport.tone.red=50
  when 17 then map.visible=false
  when 18 then map.visible=true
  when 20 then map.map_data=map.map_data.dup
  when 22 then map.map_data.resize(9,8,3)
  when 24 then map.autotiles[0].fill_rect(0,0,16,16,Color.new(240,20,10))
  when 26 then map.autotiles[0]=nil
  when 28 then map.tileset.dispose
  end
  map.update
  Graphics.update
  exit(0) if ENV['FANPLAY_TEST_CAPTURE_FRAME'] && frame==Integer(ENV['FANPLAY_TEST_CAPTURE_FRAME'])
  sprite_values=map.instance_variable_get(:@sprites).flat_map { |s| [s.x,s.y,s.z,s.visible ? 1 : 0] }
  checks << Zlib.crc32(sprite_values.pack('l<*'))
  snap=Graphics.snap_to_bitmap
  bytes=(0...snap.height).flat_map { |y| (0...snap.width).map { |x| snap.get_pixel(x,y).to_a.pack('C*') } }.join
  checks << Zlib.crc32(bytes)
  snap.dispose
end
map.dispose
character.bitmap.dispose
character.dispose
viewport.dispose
# Returning to the previous snapshot position while forcing a rebuild must
# not skip the native update after an intermediate bulk-only camera move.
reversal=Tilemap.new
reversal.map_data=Table.new(8,1,1)
reversal.priorities=Table.new(512)
reversal.tileset=Bitmap.new(256,32)
32.times { |x| reversal.tileset.fill_rect(x,0,1,32,Color.new(x*7,30,220-x*6)) }
8.times { |x| reversal.map_data[x,0,0]=384 }
[1,2,1,2,1].each_with_index do |offset,index|
  reversal.ox=offset
  reversal.priorities[500]=index if index>=2
  reversal.update
  Graphics.update
  exit(0) if ENV['FANPLAY_TEST_CAPTURE_REVERSAL']==index.to_s
  snap=Graphics.snap_to_bitmap
  checks << Zlib.crc32((0...96).map { |x| snap.get_pixel(x,0).to_a.pack('C*') }.join)
  snap.dispose
end
reversal.tileset.dispose;reversal.dispose
animated=Tilemap.new
animated.map_data=Table.new(1,1,1)
animated.map_data[0,0,0]=48
animated.autotiles[0]=Bitmap.new(64,32)
animated.autotiles[0].fill_rect(0,0,32,32,Color.new(20,220,40))
animated.autotiles[0].fill_rect(32,0,32,32,Color.new(40,20,220))
builds=FANPLAY::Retained.stats[:legacy_tilemap_rebuilt]
updates=FANPLAY::Retained.stats[:legacy_tilemap_animated_tiles]
21.times do |index|
  animated.update
  Graphics.update
  exit(0) if ENV['FANPLAY_TEST_CAPTURE_ANIMATION']==index.to_s
  snap=Graphics.snap_to_bitmap
  checks << snap.get_pixel(0,0).to_a.pack('C*').unpack1('L<')
  snap.dispose
end
if ENV['FANPLAY_RETAINED_TILEMAP']=='1'
  raise 'animation rebuilt the whole map' unless FANPLAY::Retained.stats[:legacy_tilemap_rebuilt]-builds==1
  raise 'animation did not update its tiles' unless FANPLAY::Retained.stats[:legacy_tilemap_animated_tiles]>updates
end
animated.autotiles[0].dispose;animated.dispose
puts "LEGACY_RETAINED:#{checks.join(',')}"

if ENV['FANPLAY_RETAINED_TILEMAP']=='1' && FANPLAY::Retained::ENABLED
  polled=LegacyPolledViewport.new(0,0,96,64)
  child=FANPLAY::TilemapSprite.new(polled)
  child.bitmap=Bitmap.new(2,2)
  child.x=1.5
  polled.ox=0.0;polled.oy=0.0
  Graphics.update
  synced=FANPLAY::Retained.stats[:synced]
  3.times do
    polled.ox=0.0;polled.oy=0.0
    child.x=1.5;child.zoom_x=1.0
    Graphics.update
  end
  raise 'unchanged immediate floats or polled viewport dirtied child' unless FANPLAY::Retained.stats[:synced]-synced==3
  child.x=2.5
  Graphics.update
  raise 'changed float failed to synchronize' unless FANPLAY::Retained.stats[:synced]-synced==5
  child.bitmap.dispose;child.dispose;polled.dispose
end
