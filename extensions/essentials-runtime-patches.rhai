// fanplay:id = essentials-runtime-patches
// fanplay:name = Essentials Runtime Patches
// fanplay:version = 0.1.0
// fanplay:capabilities = ruby.inject,ruby.unload
// fanplay:operation = essentials_runtime_patch
// fanplay:operation-capabilities = essentials_runtime_patch game.read
// fanplay:operation-mutates = essentials_runtime_patch
// fanplay:route = GET /v1/essentials/runtime-patches essentials_runtime_patch payload
// fanplay:route = POST /v1/essentials/runtime-patches essentials_runtime_patch payload
// fanplay:panel = runtime-patches,Essentials Runtime Patches

// The engine only supplies the reversible Ruby injection transport. The patch
// implementations, their state, and the zoom control all belong to this
// plugin. `payload` is intentionally one ordered string because it also makes
// the browser route independent of JSON object key ordering.
fn on_activate() {
    state_set("panoramic", false);
    state_set("cable", false);
    state_set("zoom", 1.5);
    let ruby = text_block("ruby");
    let apply = "{ apply: proc { " + ruby + " }, revert: proc { FANPLAY::EssentialsRuntimePatches.uninstall } }";
    state_set("pending", ruby_inject("essentials-runtime-patches", apply));
}

fn build_ui() {
    let error = last_error();
    if error != "" && state_get("handle") == () {
        return [ui_label("runtime-patches.error", "Essentials runtime patches unavailable: " + error)];
    }
    if state_get("handle") == () {
        return [ui_loading("runtime-patches.installing", "Installing Essentials runtime patches…")];
    }
    let panoramic = state_get("panoramic") == true;
    let cable = state_get("cable") == true;
    let zoom = state_get("zoom");
    [
        ui_group("runtime-patches.panoramic", "Map panoramic mode", [
            ui_checkbox("runtime-patches.panoramic.enabled", "Enable the wider map view", panoramic),
            ui_slider("runtime-patches.panoramic.zoom", "Zoom", zoom, 1.0, 3.0, 0.1, "×"),
            ui_label("runtime-patches.panoramic.note", "The frame expands only while the overworld map is active."),
        ]),
        ui_group("runtime-patches.cable", "Cable Club compatibility", [
            ui_checkbox("runtime-patches.cable.enabled", "Repair Cable Club turn RNG synchronization", cable),
            ui_label("runtime-patches.cable.note", "Enable before starting an online battle."),
        ]),
    ]
}

fn on_ui_event(widget_id, value) {
    if widget_id == "runtime-patches.panoramic.enabled" {
        let enabled = if value { "1" } else { "0" };
        state_set("panoramic", value);
        state_set("pending", request("essentials_runtime_patch", #{payload: "set|map_panoramic|" + enabled}));
    }
    if widget_id == "runtime-patches.panoramic.zoom" {
        state_set("zoom", value);
        state_set("pending", request("essentials_runtime_patch", #{payload: "value|map_panoramic|" + value.to_string()}));
    }
    if widget_id == "runtime-patches.cable.enabled" {
        let enabled = if value { "1" } else { "0" };
        state_set("cable", value);
        state_set("pending", request("essentials_runtime_patch", #{payload: "set|cable_club_rng|" + enabled}));
    }
}

fn on_operation_result(request_id, result) {
    if request_id == state_get("pending") && state_get("handle") == () && result["patch_id"] != () {
        state_set("handle", result["patch_id"]);
        state_set("pending", ());
        return;
    }
    if request_id == state_get("pending") {
        state_set("pending", ());
        if result["map_panoramic"] != () {
            state_set("panoramic", result["map_panoramic"]["enabled"]);
            state_set("zoom", result["map_panoramic"]["zoom"]);
        }
        if result["cable_club_rng"] != () {
            state_set("cable", result["cable_club_rng"]["enabled"]);
        }
    }
}

fn on_shutdown() {
    let handle = state_get("handle");
    if handle != () { ruby_unload(handle); }
}

/* fanplay:text ruby-begin
module MapPanoramic
  DEFAULT_ZOOM = 1.5
  MIN_ZOOM = 1.0
  MAX_ZOOM = 3.0
  MAX_WIDTH = 4096
  MAX_HEIGHT = 4096

  def self.applied? = @applied == true
  def self.requested? = @requested == true
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

  def self.set_zoom(value)
    @zoom = [[value.to_f, MIN_ZOOM].max, MAX_ZOOM].min
    return unless applied? && @base
    Graphics.render_size = zoomed_size(@base)
    rebuild
  end

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
    on_map? ? rebuild : (@rebuild_pending = false)
  end

  def self.zoomed_size(base)
    width = [(base[0] * zoom).round, 1].max
    height = [(base[1] * zoom).round, 1].max
    [[width, MAX_WIDTH].min, [height, MAX_HEIGHT].min]
  end

  def self.use_live_graphics_dimensions
    return if @original_graphics_dimensions
    @original_graphics_dimensions = [Graphics.method(:width), Graphics.method(:height)]
    Graphics.singleton_class.class_eval do
      define_method(:width) do
        @__size ||= FANPLAY.call('graphics_size').split("\t").map!(&:to_i)
        @__size[0]
      end
      define_method(:height) do
        @__size ||= FANPLAY.call('graphics_size').split("\t").map!(&:to_i)
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

  def self.center_x
    ((Graphics.width / 2) - (Game_Map::TILE_WIDTH / 2)) * Game_Map::X_SUBPIXELS
  end

  def self.center_y
    ((Graphics.height / 2) - (Game_Map::TILE_HEIGHT / 2)) * Game_Map::Y_SUBPIXELS
  end

  def self.install_hooks
    return if @hooks_installed
    if defined?(Graphics)
      Graphics.singleton_class.class_eval do
        alias_method :__fanplay_panoramic_update, :update
        def update
          __fanplay_panoramic_update
          MapPanoramic.tick if MapPanoramic.engaged?
        end
      end
    end
    if defined?(Game_Player) && defined?(Game_Map) &&
        Game_Player.method_defined?(:center) && Game_Player.method_defined?(:update_screen_position) &&
        %i[TILE_WIDTH TILE_HEIGHT X_SUBPIXELS Y_SUBPIXELS REAL_RES_X REAL_RES_Y].all? { |name| Game_Map.const_defined?(name) }
      Game_Player.class_eval do
        alias_method :__fanplay_panoramic_center, :center
        alias_method :__fanplay_panoramic_update_screen_position, :update_screen_position
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
      Spriteset_Map.class_eval do
        alias_method :__fanplay_panoramic_update, :update
        def update
          if MapPanoramic.applied?
            @@viewport0.rect.set(0, 0, Graphics.width, Graphics.height) if defined?(@@viewport0) && @@viewport0
            @@viewport3.rect.set(0, 0, Graphics.width, Graphics.height) if defined?(@@viewport3) && @@viewport3
          end
          __fanplay_panoramic_update
        end
      end
    end
    if defined?(TilemapLoader) && defined?(SynchronizedTilemap)
      TilemapLoader.class_eval do
        alias_method :__fanplay_panoramic_update_class, :updateClass
        def updateClass
          if MapPanoramic.applied? && defined?($PokemonSystem) &&
              $PokemonSystem && $PokemonSystem.tilemap == 1
            setClass(SynchronizedTilemap)
          else
            __fanplay_panoramic_update_class
          end
        end
      end
    end
    if defined?(EventHandlers) && EventHandlers.respond_to?(:add)
      EventHandlers.add(:on_frame_update, :map_panoramic, proc { MapPanoramic.attempt_rebuild })
    end
    @hooks_installed = true
  end

  def self.rebuild
    @rebuild_pending = true
    attempt_rebuild
  end

  def self.attempt_rebuild
    return unless @rebuild_pending
    return unless on_map?
    if busy?
      unless @busy_logged
        FANPLAY::ERROR_OUTPUT.call("[MapPanoramic] map rebuild waiting for a safe frame\n")
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
  end

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

module CableClubRngCompat
  def self.enabled? = @enabled == true

  module RandomInstance
    def srand(seed = (Random.new_seed & 0x7fffffff))
      @fanplay_cable_club_random = Random.new(seed)
      seed
    end

    def rand(*arguments)
      return @fanplay_cable_club_random.rand(*arguments) if CableClubRngCompat.enabled? && @fanplay_cable_club_random
      super
    end
  end

  module BattleAccessor
    def battleRNG
      return super unless CableClubRngCompat.enabled?
      rng = instance_variable_get(:@battleRNG)
      unless rng
        rng = Random.new(Random.new_seed & 0x7fffffff)
        instance_variable_set(:@battleRNG, rng)
      end
      singleton = rng.singleton_class
      singleton.prepend(RandomInstance) unless singleton.ancestors.include?(RandomInstance)
      rng
    end
  end

  def self.apply
    @enabled = true
    if defined?(Battle_CableClub) && !Battle_CableClub.ancestors.include?(BattleAccessor)
      Battle_CableClub.prepend(BattleAccessor)
    end
  end

  def self.revert
    @enabled = false
  end
end

module FANPLAY
  module EssentialsRuntimePatches
    def self.install
      @api = FANPLAY::GameAPI
      @original = @api.method(:essentials_runtime_patch) if @api.respond_to?(:essentials_runtime_patch)
      @api.define_singleton_method(:essentials_runtime_patch) do |args|
        FANPLAY::EssentialsRuntimePatches.dispatch(args)
      end
    end

    def self.uninstall
      MapPanoramic.revert if MapPanoramic.requested?
      CableClubRngCompat.revert if CableClubRngCompat.enabled?
      return unless @api
      if @original
        original = @original
        @api.define_singleton_method(:essentials_runtime_patch) { |*args| original.call(*args) }
      else
        @api.singleton_class.send(:remove_method, :essentials_runtime_patch) rescue nil
      end
      @api = nil
      @original = nil
    end

    def self.state
      {
        'map_panoramic' => { 'enabled' => MapPanoramic.requested?, 'zoom' => MapPanoramic.zoom },
        'cable_club_rng' => { 'enabled' => CableClubRngCompat.enabled? }
      }
    end

    def self.dispatch(args)
      parts = args.fetch(0, 'state').to_s.split('|', -1)
      case parts.shift
      when 'state'
        state
      when 'set'
        patch = parts.fetch(0)
        enabled = parts.fetch(1) == '1'
        case patch
        when 'map_panoramic' then enabled ? MapPanoramic.apply : MapPanoramic.revert
        when 'cable_club_rng' then enabled ? CableClubRngCompat.apply : CableClubRngCompat.revert
        else raise ArgumentError, "unknown Essentials runtime patch: #{patch}"
        end
        state
      when 'value'
        raise ArgumentError, 'only map_panoramic has a value' unless parts.fetch(0) == 'map_panoramic'
        MapPanoramic.set_zoom(parts.fetch(1))
        state
      else
        raise ArgumentError, 'runtime patch payload must be state, set, or value'
      end
    end
  end
end

FANPLAY::EssentialsRuntimePatches.install
fanplay:text ruby-end */
