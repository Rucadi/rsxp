# Older Essentials computes its camera in update/center, without v21's hook.
class Game_Map
  TILEWIDTH=32
  TILEHEIGHT=32
  XSUBPIXEL=4
  YSUBPIXEL=4
  attr_accessor :display_x,:display_y
end
$game_map=Game_Map.new
class Game_Player
  def x = 10
  def y = 12
  def center(x,y)
    $game_map.display_x=x*128-(Graphics.width/2-16)*4
    $game_map.display_y=y*128-(Graphics.height/2-16)*4
  end
end
$game_player=Game_Player.new
original_center=Game_Player.instance_method(:center)
class Scene_Map
  attr_reader :rebuilds
  def initialize = @rebuilds=0
  def disposeSpritesets; end
  def createSpritesets = @rebuilds+=1
end
Graphics.resize_screen(320,240)
$scene=Object.new
RSXP.call('runtime_patch_set','map_panoramic',1)
RuntimePatches.sync
raise 'panoramic activation failed' unless RuntimePatches.active?('map_panoramic')
raise 'legacy camera replaced' unless Game_Player.instance_method(:center)==original_center
raise 'invented v21 camera method' if Game_Player.method_defined?(:update_screen_position)
$scene=Scene_Map.new
Graphics.update
raise 'legacy map did not resize' unless Graphics.width==480 && Graphics.height==360
raise 'map was not rebuilt' unless $scene.rebuilds==1
raise 'legacy camera not centered' unless $game_map.display_x==384 && $game_map.display_y==880
RSXP.call('runtime_patch_set','map_panoramic',0)
RuntimePatches.sync
raise 'legacy map size not restored' unless Graphics.width==320 && Graphics.height==240
raise 'legacy camera changed on revert' unless Game_Player.instance_method(:center)==original_center
puts 'PANORAMIC_HOOKS_OK'
