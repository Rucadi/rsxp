# SPDX-License-Identifier: GPL-2.0-or-later
# Exercise the actual unmodified Essentials renderer with deterministic assets.
Graphics.resize_screen(96,64)
class Game_Map
  TILE_WIDTH=32; TILE_HEIGHT=32; X_SUBPIXELS=4; Y_SUBPIXELS=4
  attr_accessor :data,:display_x,:display_y,:map_id,:tileset_id,:autotile_names,:tileset_name,:priorities,:terrain_tags
  def initialize(id)
    @map_id=id; @display_x=@display_y=0; @tileset_id=1
    @data=Table.new(8,5,3); @priorities=Table.new(512); @terrain_tags=Table.new(512)
    @autotile_names=['water']; @tileset_name='ground'
    8.times { |x| 5.times { |y| @data[x,y,0]=384+(x+y)%4; @data[x,y,1]=48 if y==1 } }
  end
  def width = 8
  def height = 5
end
module MapFactoryHelper
  def self.hasConnections?(*) = false
end
module GameData
  module TerrainTag
    Entry=Struct.new(:shows_reflections,:bridge)
    def self.try_get(tag) = Entry.new(tag==1,tag==2)
  end
end
def nil_or_empty?(value) = value.nil? || value.empty?
$PokemonGlobal=Struct.new(:bridge).new(0)
$PokemonSystem=Struct.new(:autotile_animations).new(0)
$map_factory=Struct.new(:maps).new([])
$game_map=Game_Map.new(1)
$map_factory.maps=[$game_map]
$clock=0.0
module System
  def self.uptime = $clock
end
load File.expand_path('../../ericlostie/Data/Scripts/016_Map renderer/002_TilemapRenderer.rb',__dir__)
RSXP.install_native_tilemap
viewport=Viewport.new(0,0,96,64)
renderer=TilemapRenderer.new(viewport)
ground=Bitmap.new(256,32)
8.times { |i| ground.fill_rect(i*32,0,32,32,Color.new(20+i*25,160-i*12,40+i*10)) }
water=Bitmap.new(64,32)
water.fill_rect(0,0,32,32,Color.new(20,70,220,180))
water.fill_rect(32,0,32,32,Color.new(60,130,240,180))
renderer.tilesets['ground']=ground
renderer.autotiles.instance_variable_get(:@frame_durations)['water']=0.25
renderer.autotiles['water']=water
character=Sprite.new(viewport)
character.bitmap=Bitmap.new(10,18)
character.bitmap.fill_rect(character.bitmap.rect,Color.new(220,30,70))
character.x=34;character.y=30;character.z=64
checks=[]
30.times do |frame|
  $clock=frame*0.125
  case frame
  when 2 then $game_map.data[1,0,0]=387
  when 3 then $game_map.display_x=4
  when 4 then $game_map.display_x=128
  when 5 then $game_map.display_x=-4
  when 6 then renderer.tone.gray=70
  when 7 then renderer.color.set(70,30,20,90)
  when 8 then viewport.ox=2; viewport.rect.width=80
  when 9 then $game_map.terrain_tags[385]=2; renderer.refresh
  when 10 then $PokemonGlobal.bridge=1
  when 11 then $game_map.priorities[386]=2; renderer.refresh
  when 12 then $game_map.terrain_tags[387]=1; renderer.refresh
  when 13
    connected=Game_Map.new(2);connected.display_x=-256
    $map_factory.maps << connected
  when 14 then $game_map.display_x=768
  when 15 then $map_factory.maps.reverse!
  when 16 then $map_factory.maps=[$game_map]; $game_map.display_x=0
  when 17
    replacement=$game_map.data.dup;replacement[0,0,0]=386;$game_map.data=replacement
  when 18 then $game_map.data.resize(9,5,3)
  when 19 then renderer.refresh
  when 20 then viewport.visible=false
  when 21 then viewport.visible=true
  when 22 then viewport.rect.x=3
  when 23 then ground.fill_rect(0,0,32,32,Color.new(150,40,100))
  end
  previous=RSXP::Retained.stats[:map_changed_cells]
  renderer.update
  if ENV['RSXP_RETAINED_MAP']=='1' && frame==4
    raise 'scroll refreshed retained cells' unless RSXP::Retained.stats[:map_changed_cells]-previous==9
  end
  if ENV['RSXP_RETAINED_MAP']=='1' && frame>=24
    raise 'idle map was traversed again' unless RSXP::Retained.stats[:map_changed_cells]==previous
  end
  coordinates=renderer.instance_variable_get(:@tiles).flat_map { |column| column.flat_map { |stack| stack.flat_map { |tile| [tile.x,tile.y,tile.z] } } }
  checks << Zlib.crc32(coordinates.pack('l<*'))
  Graphics.update
  if ENV['RSXP_TEST_CAPTURE_FRAME'] && frame==Integer(ENV['RSXP_TEST_CAPTURE_FRAME'])
    puts "CAPTURE_FRAME:#{frame}"
    exit(0)
  end
  snap=Graphics.snap_to_bitmap
  bytes=(0...snap.height).flat_map { |y| (0...snap.width).map { |x| snap.get_pixel(x,y).to_a.pack('C*') } }.join
  checks << Zlib.crc32(bytes)
  snap.dispose
end
if ENV['RSXP_NATIVE_STATE']=='1' && ENV['RSXP_RETAINED_MAP']=='1' && ENV['RSXP_NATIVE_SCROLL']=='1'
  raise 'native scrolling did not run' unless RSXP::Retained.stats[:bulk_scrolls]>0
end
if ENV['RSXP_WORLD_CAMERA']=='1'
  raise 'world camera did not run' unless RSXP::Retained.stats[:world_camera_frames]>0
end
if ENV['RSXP_MAP_CHUNKS']=='1'
  cache=renderer.instance_variable_get(:@__rsxp_cell_cache)
  builds,hits,checks_count,evictions,resident=cache.chunk_stats
  raise 'chunk cache was unused' unless builds>0 && hits>0 && resident>0
end
# Plugin writes must materialize lazy coordinates and preserve normal setters.
[:setter,:reflective,:metadata].each do |kind|
  renderer.refresh
  renderer.update
  tile=renderer.instance_variable_get(:@tiles).flatten.find { |candidate| candidate.bitmap && candidate.visible }
  case kind
  when :setter then tile.x=tile.x+7
  when :reflective then tile.instance_variable_set(:@y,tile.y+5)
  when :metadata then tile.priority=2
  end
  coordinates=renderer.instance_variable_get(:@tiles).flatten.flat_map { |candidate| [candidate.x,candidate.y,candidate.z] }
  checks << Zlib.crc32(coordinates.pack('l<*'))
  Graphics.update
  snap=Graphics.snap_to_bitmap
  bytes=(0...snap.height).flat_map { |y| (0...snap.width).map { |x| snap.get_pixel(x,y).to_a.pack('C*') } }.join
  checks << Zlib.crc32(bytes)
  snap.dispose
  $game_map.display_x+=4
  renderer.update
  coordinates=renderer.instance_variable_get(:@tiles).flatten.flat_map { |candidate| [candidate.x,candidate.y,candidate.z] }
  checks << Zlib.crc32(coordinates.pack('l<*'))
end
class TilemapRenderer
  alias __retained_test_update update
  def update
    __retained_test_update
  end
  alias __retained_test_dispose dispose
  def dispose
    __retained_test_dispose
  end
end
renderer.update
renderer.dispose
puts "RETAINED_MAP:#{checks.join(',')}"
