# Synthetic Pokemon Essentials-like map classes, so the "map panoramic mode"
# runtime patch's hooks can be exercised without booting the whole game.

module Settings
  SCREEN_WIDTH = 512
  SCREEN_HEIGHT = 384
end

class Game_Map
  TILE_WIDTH = 32
  TILE_HEIGHT = 32
  X_SUBPIXELS = 4
  Y_SUBPIXELS = 4
  REAL_RES_X = TILE_WIDTH * X_SUBPIXELS
  REAL_RES_Y = TILE_HEIGHT * Y_SUBPIXELS
end

class FakeMap
  attr_accessor :display_x, :display_y

  def initialize
    @display_x = 0
    @display_y = 0
  end

  def scrolling? = false
end

$game_map = FakeMap.new

class Game_Player
  attr_accessor :x, :y, :real_x, :real_y, :moved_last_frame, :moved_this_frame

  def initialize
    @x = 4
    @y = 6
    @real_x = 0
    @real_y = 0
    @moved_last_frame = false
    @moved_this_frame = false
  end

  def map = $game_map

  def center(x, y)
    self.map.display_x = (x * Game_Map::REAL_RES_X) -
                         ((Settings::SCREEN_WIDTH / 2 - Game_Map::TILE_WIDTH / 2) * Game_Map::X_SUBPIXELS)
    self.map.display_y = (y * Game_Map::REAL_RES_Y) -
                         ((Settings::SCREEN_HEIGHT / 2 - Game_Map::TILE_HEIGHT / 2) * Game_Map::Y_SUBPIXELS)
  end

  def update_screen_position(_last_real_x, _last_real_y); end
end

$game_player = Game_Player.new

class Spriteset_Map
  @@viewport0 = nil
  @@viewport3 = nil
  def update; end
end

$game_temp = Struct.new(:in_menu, :in_battle, :message_window_showing,
                        :player_transferring, :transition_processing).new(false, false, false, false, false)

class Scene_Map
  attr_reader :rebuilt_count

  def initialize
    @spritesets = {}
    @rebuilt_count = 0
  end

  def disposeSpritesets
    @spritesets = {}
  end

  def createSpritesets
    @spritesets = { 1 => Spriteset_Map.new }
    @rebuilt_count += 1
  end
end

base_width, base_height = Graphics.width, Graphics.height
grown_width = (base_width * 1.5).round
grown_height = (base_height * 1.5).round

# Off the overworld the mode must not be applied at all.
$scene = Object.new
RSXP.call('runtime_patch_set', 'map_panoramic', 1)
RuntimePatches.sync
raise 'panoramic mode did not activate' unless RuntimePatches.active?('map_panoramic')
raise "enlarged off the overworld: #{Graphics.width}x#{Graphics.height}" unless Graphics.width == base_width && Graphics.height == base_height

# Free overworld movement enlarges the frame.
$scene = Scene_Map.new
Graphics.update
raise "frame did not grow: #{Graphics.width}x#{Graphics.height}" unless Graphics.width == grown_width && Graphics.height == grown_height
raise 'map renderer was not rebuilt' unless $scene.rebuilt_count == 1

$game_player.center($game_player.x, $game_player.y)
expected_x = ($game_player.x * Game_Map::REAL_RES_X) -
             ((grown_width / 2 - Game_Map::TILE_WIDTH / 2) * Game_Map::X_SUBPIXELS)
raise "camera not centred on the enlarged frame: #{$game_map.display_x} != #{expected_x}" unless $game_map.display_x == expected_x

# Menus and message boxes must not toggle the mode mid-menu.
$game_temp.message_window_showing = true
Graphics.update
raise "message box toggled the frame: #{Graphics.width}x#{Graphics.height}" unless Graphics.width == grown_width && Graphics.height == grown_height
$game_temp.message_window_showing = false
Graphics.update
raise "frame changed after the message: #{Graphics.width}x#{Graphics.height}" unless Graphics.width == grown_width && Graphics.height == grown_height

# A battle is not the overworld, so the native size returns.
$game_temp.in_battle = true
Graphics.update
raise "battle kept the enlarged frame: #{Graphics.width}x#{Graphics.height}" unless Graphics.width == base_width && Graphics.height == base_height
$game_temp.in_battle = false
Graphics.update
raise "frame did not return after the battle: #{Graphics.width}x#{Graphics.height}" unless Graphics.width == grown_width && Graphics.height == grown_height

# The replaced update_screen_position must keep working while the mode is on.
$game_player.moved_this_frame = true
$game_player.update_screen_position(0, 0)

RSXP.call('runtime_patch_set', 'map_panoramic', 0)
RuntimePatches.sync
raise 'panoramic mode did not revert' if RuntimePatches.active?('map_panoramic')
raise "frame was not restored: #{Graphics.width}x#{Graphics.height}" unless Graphics.width == base_width && Graphics.height == base_height

puts 'PANORAMIC_HOOKS_OK'
