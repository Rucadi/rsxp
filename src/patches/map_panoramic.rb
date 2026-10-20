# SPDX-License-Identifier: GPL-2.0-or-later
#
# Runtime patch: "Map panoramic mode".
#
# Renders the overworld into a larger logical frame and lets the frontend scale
# that frame down to the window, so more of the current map is visible around
# the player. It is written for Pokemon Essentials v21 games (the ericlostie /
# Pokemon Anil build) and stays inert on games that do not define the map
# classes it hooks.
#
# The mode is active for the whole overworld scene: it requires `$scene` to be a
# `Scene_Map` and no battle to be running. Menus, message boxes and transitions
# stay enlarged so the mode never toggles while a menu is open; the title
# screen, the intro and battles run at the native size.
#
# The evaluated value is the patch contract consumed by `RuntimePatches` in
# rgss.rb: a hash of procs that turn the mode on and off and react to the zoom
# slider. Everything else is namespaced under MapPanoramic.

module MapPanoramic
  # Extra map visible along each axis. Kept in the same range as the engine's
  # "Zoom" parameter so the panel slider and the default agree.
  DEFAULT_ZOOM = 1.5
  MIN_ZOOM = 1.0
  MAX_ZOOM = 3.0
  # Clamp so a bad base size cannot allocate a huge frame.
  MAX_WIDTH = 4096
  MAX_HEIGHT = 4096

  # The panel toggle is an intent: the frame is only enlarged while free
  # overworld movement is happening. `@applied` tracks whether it is enlarged.
  def self.applied? = @applied == true
  def self.requested? = @requested == true
  def self.pending? = @rebuild_pending == true
  def self.engaged? = @requested == true || @applied == true || @rebuild_pending == true
  def self.zoom = (@zoom || DEFAULT_ZOOM)

  def self.apply
    @requested = true
    install_hooks
    tick
  end

  def self.revert
    @requested = false
    tick
  end

  # Live zoom control from the panel slider. Before the mode is applied this
  # only records the value for the next apply.
  def self.set_zoom(value)
    @zoom = [[value.to_f, MIN_ZOOM].max, MAX_ZOOM].min
    return unless applied? && @base
    Graphics.render_size = zoomed_size(@base)
    rebuild
  end

  # Called every frame from the patched Graphics.update. Only the overworld is
  # affected: menus, messages, transitions and battles revert to native size.
  def self.tick
    if requested? && active?
      apply_now unless applied?
    elsif applied?
      revert_now
    end
    attempt_rebuild
  end

  def self.on_map? = defined?(Scene_Map) && $scene.is_a?(Scene_Map)
  def self.in_battle?
    temp = (defined?($game_temp) ? $game_temp : nil)
    temp ? (temp.respond_to?(:in_battle) && temp.in_battle) : false
  end
  # Active for the whole overworld scene, menus and messages included, so the
  # mode never toggles mid-menu. Battles and every other scene stay native.
  def self.active? = on_map? && !in_battle?

  def self.apply_now
    return if applied?
    @base ||= [Graphics.width, Graphics.height]
    Graphics.render_size = zoomed_size(@base)
    use_live_graphics_dimensions
    @applied = true
    rebuild
  end

  def self.revert_now
    return unless applied?
    @applied = false
    Graphics.render_size = @base if @base
    restore_graphics_dimensions
    # Off the overworld there is no map renderer to rebuild; the next map scene
    # draws its grid at the restored size.
    on_map? ? rebuild : (@rebuild_pending = false)
  end

  def self.zoomed_size(base)
    width = [(base[0] * zoom).round, 1].max
    height = [(base[1] * zoom).round, 1].max
    [[width, MAX_WIDTH].min, [height, MAX_HEIGHT].min]
  end

  # Some games, including Pokémon Z, replace Graphics.width/height with fixed
  # defaults in their resize script. The engine's render target still grows,
  # but those methods then make the camera and map viewports stay at the old
  # size. While panoramic mode is active, use the live engine dimensions.
  def self.use_live_graphics_dimensions
    return if @original_graphics_dimensions
    @original_graphics_dimensions = [Graphics.method(:width), Graphics.method(:height)]
    Graphics.singleton_class.class_eval do
      define_method(:width) do
        @__size ||= RSXP.call('graphics_size').split("\t").map!(&:to_i)
        @__size[0]
      end
      define_method(:height) do
        @__size ||= RSXP.call('graphics_size').split("\t").map!(&:to_i)
        @__size[1]
      end
    end
  end

  def self.restore_graphics_dimensions
    return unless @original_graphics_dimensions
    Graphics.define_singleton_method(:width, @original_graphics_dimensions[0])
    Graphics.define_singleton_method(:height, @original_graphics_dimensions[1])
    @original_graphics_dimensions = nil
  end

  # Screen centre used to follow the player, derived from the live frame size
  # instead of the fixed Settings constants so the player stays centred while
  # the frame is enlarged.
  def self.center_x
    ((Graphics.width / 2) - (Game_Map::TILE_WIDTH / 2)) * Game_Map::X_SUBPIXELS
  end

  def self.center_y
    ((Graphics.height / 2) - (Game_Map::TILE_HEIGHT / 2)) * Game_Map::Y_SUBPIXELS
  end

  # Hooks are installed once and stay in place; every one of them is a no-op
  # while the mode is off (Graphics.width is then the original size again).
  def self.install_hooks
    return if @hooks_installed
    if defined?(Graphics)
      # Runs in every scene, which is how the mode notices entering and leaving
      # the overworld.
      Graphics.singleton_class.class_eval do
        alias_method :__rsxp_panoramic_update, :update
        def update
          __rsxp_panoramic_update
          MapPanoramic.tick if MapPanoramic.engaged?
        end
      end
    end
    # Older Essentials already computes its camera centre from Graphics.width
    # and height inside update, and has no update_screen_position method.
    # Only replace the separate camera hooks on the newer engine layout.
    if defined?(Game_Player) && defined?(Game_Map) &&
        Game_Player.method_defined?(:center) && Game_Player.method_defined?(:update_screen_position) &&
        %i[TILE_WIDTH TILE_HEIGHT X_SUBPIXELS Y_SUBPIXELS REAL_RES_X REAL_RES_Y].all? { |name| Game_Map.const_defined?(name) }
      Game_Player.class_eval do
        alias_method :__rsxp_panoramic_center, :center
        alias_method :__rsxp_panoramic_update_screen_position, :update_screen_position
        def center(x, y)
          self.map.display_x = (x * Game_Map::REAL_RES_X) - MapPanoramic.center_x
          self.map.display_y = (y * Game_Map::REAL_RES_Y) - MapPanoramic.center_y
        end
        def update_screen_position(last_real_x, last_real_y)
          return if self.map.scrolling? || !(@moved_last_frame || @moved_this_frame)
          center_x = MapPanoramic.center_x
          center_y = MapPanoramic.center_y
          if (@real_x < last_real_x && @real_x < $game_map.display_x + center_x) ||
             (@real_x > last_real_x && @real_x > $game_map.display_x + center_x)
            self.map.display_x += @real_x - last_real_x
          end
          if (@real_y < last_real_y && @real_y < $game_map.display_y + center_y) ||
             (@real_y > last_real_y && @real_y > $game_map.display_y + center_y)
            self.map.display_y += @real_y - last_real_y
          end
        end
      end
    end
    if defined?(Spriteset_Map)
      # The panorama and screen-flash viewports are created once at the original
      # size and never resized by the game, so grow them with the frame.
      Spriteset_Map.class_eval do
        alias_method :__rsxp_panoramic_update, :update
        def update
          if MapPanoramic.applied?
            @@viewport0.rect.set(0, 0, Graphics.width, Graphics.height) if defined?(@@viewport0) && @@viewport0
            @@viewport3.rect.set(0, 0, Graphics.width, Graphics.height) if defined?(@@viewport3) && @@viewport3
          end
          __rsxp_panoramic_update
        end
      end
    end
    if defined?(TilemapLoader) && defined?(SynchronizedTilemap)
      # Pokémon Z's custom tilemap repaints a viewport-sized bitmap whenever
      # the camera scrolls. At 3x, that means roughly nine times the tile work
      # per frame. The engine's native tilemap already supports the data this
      # game supplies and keeps the map rendering on the native path. Keep the
      # game's custom renderer whenever panoramic mode is inactive.
      TilemapLoader.class_eval do
        alias_method :__rsxp_panoramic_update_class, :updateClass
        def updateClass
          if MapPanoramic.applied? && defined?($PokemonSystem) &&
              $PokemonSystem && $PokemonSystem.tilemap == 1
            setClass(SynchronizedTilemap)
          else
            __rsxp_panoramic_update_class
          end
        end
      end
    end
    if defined?(EventHandlers) && EventHandlers.respond_to?(:add)
      # Runs at the end of Scene_Map#updateSpritesets, a safe point to rebuild
      # the map renderer without disturbing the frame's spriteset loop.
      EventHandlers.add(:on_frame_update, :map_panoramic, proc { MapPanoramic.attempt_rebuild })
    end
    @hooks_installed = true
  end

  # The map renderer builds a fixed grid of tile sprites sized to the frame, so
  # it has to be rebuilt whenever the frame grows or shrinks. Rebuilding during
  # a menu or a map transfer would tear down sprites the game is still using, so
  # defer until the next safe frame.
  def self.rebuild
    @rebuild_pending = true
    attempt_rebuild
  end

  def self.attempt_rebuild
    return unless @rebuild_pending
    return unless on_map?
    if busy?
      unless @busy_logged
        RSXP::ERROR_OUTPUT.call("[MapPanoramic] map rebuild waiting for a safe frame\n")
        @busy_logged = true
      end
      return
    end
    @busy_logged = false
    @rebuild_pending = false
    scene = $scene
    scene.disposeSpritesets
    renderer = scene.instance_variable_get(:@map_renderer)
    renderer.dispose if renderer && !renderer.disposed?
    scene.instance_variable_set(:@map_renderer, nil)
    scene.createSpritesets
    $game_player.center($game_player.x, $game_player.y) if $game_player
    display = defined?($game_map) && $game_map ?
      " map_display=#{$game_map.display_x},#{$game_map.display_y}" : ""
    RSXP::ERROR_OUTPUT.call("[MapPanoramic] rebuilt map at #{Graphics.width}x#{Graphics.height}#{display}\n")
  end

  # Only used to defer the map renderer rebuild (never the mode itself), so a
  # rebuild never tears down sprites while a menu, message or transfer is using
  # them.
  def self.busy?
    temp = (defined?($game_temp) ? $game_temp : nil)
    return false unless temp
    return true if temp.respond_to?(:in_menu) && temp.in_menu
    return true if temp.respond_to?(:in_battle) && temp.in_battle
    return true if temp.respond_to?(:message_window_showing) && temp.message_window_showing
    return true if temp.respond_to?(:player_transferring) && temp.player_transferring
    return true if temp.respond_to?(:transition_processing) && temp.transition_processing
    false
  end
end

{ apply: MapPanoramic.method(:apply),
  revert: MapPanoramic.method(:revert),
  value: MapPanoramic.method(:set_zoom) }
