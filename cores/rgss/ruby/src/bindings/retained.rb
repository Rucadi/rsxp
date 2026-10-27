# SPDX-License-Identifier: GPL-2.0-or-later
# Desktop adapter. Game code and the serialized RGSS API stay unchanged.
module FANPLAY::Retained
  ENABLED = ENV['FANPLAY_NATIVE_STATE'] != '0' && Sprite::NATIVE_SYNC
  FIELDS = {
    Sprite => %i[x y z ox oy zoom_x zoom_y visible mirror blend_type angle invert bush_depth bush_opacity
                 pattern pattern_opacity pattern_tile pattern_scroll_x pattern_scroll_y pattern_zoom_x
                 pattern_zoom_y pattern_blend_type viewport bitmap src_rect opacity color tone],
    Viewport => %i[z ox oy visible rect color tone],
    Plane => %i[viewport visible z ox oy zoom_x zoom_y blend_type bitmap opacity color tone]
  }.freeze
  DEPENDENCIES = %i[@src_rect @rect @color @tone @bitmap @viewport @pattern @__fanplay_flash_color].freeze
  @resources = {}
  @compatibility = {}
  @pending = {}
  @polled_viewports = {}
  @signatures = {}
  @value_signatures = {}
  @reclassify = false
  @stats = Hash.new(0)
  class << self
    attr_reader :stats
    def invalidate!
      @reclassify = true
    end
    # SabiRuby's UnboundMethod#bind_call re-resolves a native/attr method by
    # name, which would call the wrapper that replaced it. Keep the original
    # under an alias and call that; aliases also carry attr/native methods.
    def original_alias(klass, name)
      key="__fanplay_retained_#{name.to_s.gsub(/[^A-Za-z0-9_]/,'_')}".to_sym
      unless klass.method_defined?(key) || klass.private_method_defined?(key)
        klass.alias_method(key,name)
      end
      key
    end
    def reclassification_pending? = @reclassify
    def kind(object)
      return Sprite if object.is_a?(Sprite)
      return Viewport if object.is_a?(Viewport)
      Plane
    end
    def eligible?(object, base)
      if defined?(FANPLAY::TilemapSprite) && object.instance_of?(FANPLAY::TilemapSprite)
        return @tilemap_signatures.all? { |name,method| object.method(name).unbind==method }
      end
      supported = object.class == base
      supported ||= base == Sprite && defined?(TilemapRenderer::TileSprite) && object.class == TilemapRenderer::TileSprite
      supported && object.singleton_methods(false).empty? && @signatures.fetch(base).all? { |name, method| object.method(name).unbind == method }
    end
    def register(object)
      base = kind(object)
      id = object.__id
      reference = FANPLAY::ResourceRef.new(object)
      @resources[id] = [base, reference]
      @compatibility[id] = reference unless eligible?(object,base)
      object.instance_variable_set(:@__fanplay_retained,!@compatibility.key?(id))
      # The legacy initialize methods append weak refs; this registry replaces
      # those lists, without introducing a strong reference to any resource.
      base.instance_variable_get(:@instances).clear
      rewire(object)
      __fanplay_touch(object)
      object
    end
    def rewire(object)
      return unless object.instance_variable_defined?(:@__id)
      id = object.__id
      __fanplay_unwatch(id)
      DEPENDENCIES.each do |name|
        value = object.instance_variable_get(name)
        if value.is_a?(Rect) || value.is_a?(Color) || value.is_a?(Tone) || value.is_a?(Bitmap) || value.is_a?(Viewport)
          __fanplay_watch(value,id)
          signatures=@value_signatures[value.class]
          if (value.is_a?(Rect) || value.is_a?(Color) || value.is_a?(Tone)) &&
              (!signatures || !signatures.all? { |name,method| value.method(name).unbind == method })
            __fanplay_volatile(id)
          end
        elsif value
          __fanplay_volatile(id)
        end
      end
    end
    def forget(id)
      @resources.delete(id)
      @compatibility.delete(id)
      @pending.delete(id)
      @polled_viewports.delete(id)
      __fanplay_forget(id)
    end
    def references(base)
      @resources.filter_map do |id,(type,reference)|
        next unless type == base
        begin
          object = reference.__getobj__
          next reference unless object.disposed?
        rescue FANPLAY::ResourceRef::RefError
        end
        forget(id)
        nil
      end
    end
    def flush(base)
      if @reclassify
        @reclassify = false
        @resources.each do |id,(type,reference)|
          begin
            object = reference.__getobj__
            @compatibility[id] = reference unless eligible?(object,type)
            object.instance_variable_set(:@__fanplay_retained,!@compatibility.key?(id))
            rewire(object)
            __fanplay_touch(object)
          rescue FANPLAY::ResourceRef::RefError
            forget(id)
          end
        end
      end
      __fanplay_dirty_resources.each { |id| @pending[id] = true }
      # Only compatibility objects are polled. Native snapshots remain retained
      # until a setter, nested value mutation, or dependency invalidates them.
      # Collect matching ids directly. `keys.select` materialized both the
      # entire pending key list and a second filtered list on every class flush.
      ids = []
      @pending.each_key do |id|
        entry = @resources[id]
        ids << id if entry && entry[0] == base
      end
      unless @compatibility.empty?
        @compatibility.each_key do |id|
          entry = @resources[id]
          ids << id if entry && entry[0] == base && !@pending[id]
        end
      end
      ids.each do |id|
        @pending.delete(id)
        entry = @resources[id]
        next unless entry
        begin
          object = entry[1].__getobj__
          if object.disposed?
            forget(id)
          else
            if base==Viewport && @compatibility.key?(id)
              # Legacy screen-resizers require viewport polling. Polling alone
              # is not a change: dirtying the viewport every frame also dirties
              # every tile sprite that depends on it.
              signature=FIELDS[Viewport].map do |field|
                value=object.instance_variable_get("@#{field}")
                if value.is_a?(Rect) || value.is_a?(Color)
                  value.to_a
                elsif value.is_a?(Tone)
                  [value.red,value.green,value.blue,value.gray]
                else
                  value
                end
              end
              if @polled_viewports[id]!=signature
                @polled_viewports[id]=signature
                __fanplay_touch(object)
              end
            end
            object.__sync
            @stats[:synced] += 1
          end
        rescue FANPLAY::ResourceRef::RefError
          forget(id)
        rescue Exception
          @pending[id]=true
          raise
        end
      end
      # Dependency tokens are not resources, and need no Ruby lookup.
      @pending.delete_if { |id,_| !@resources.key?(id) }
    end
  end
end

if FANPLAY::Retained::ENABLED
  # Observe nested mutations without changing value identity, subclass behavior,
  # floating point channels, or Marshal payloads.
  { Color => %i[red green blue alpha], Tone => %i[red green blue gray], Rect => %i[x y width height] }.each do |klass,fields|
    observer = klass
    ivars=fields.map { |field| "@#{field}".to_sym }.freeze
    fields.each do |field|
      original=FANPLAY::Retained.original_alias(klass,"#{field}=")
      ivar="@#{field}".to_sym
      observer.define_method("#{field}=") do |value|
        previous = instance_variable_get(ivar)
        result = send(original,value)
        if @__fanplay_dependency_token && previous != instance_variable_get(ivar)
          __fanplay_touch(self)
        end
        result
      end
    end
    original_set=FANPLAY::Retained.original_alias(klass,:set)
    observer.define_method(:set) do |*args|
      a=instance_variable_get(ivars[0])
      b=instance_variable_get(ivars[1])
      c=instance_variable_get(ivars[2])
      d=instance_variable_get(ivars[3])
      result = send(original_set,*args)
      if @__fanplay_dependency_token && (a != instance_variable_get(ivars[0]) ||
          b != instance_variable_get(ivars[1]) || c != instance_variable_get(ivars[2]) ||
          d != instance_variable_get(ivars[3]))
        __fanplay_touch(self)
      end
      result
    end
    original_copy=FANPLAY::Retained.original_alias(klass,:initialize_copy)
    observer.define_method(:initialize_copy) do |other|
      result=send(original_copy,other)
      remove_instance_variable(:@__fanplay_dependency_token) if instance_variable_defined?(:@__fanplay_dependency_token)
      result
    end
    klass.send(:private,:initialize_copy)
    FANPLAY::Retained.instance_variable_get(:@value_signatures)[klass]=(fields.flat_map { |name| [name,"#{name}=".to_sym] } + [:set]).map { |name| [name,klass.instance_method(name)] }
    klass.singleton_class.prepend(Module.new do
      def method_added(name)
        super
        FANPLAY::Retained.invalidate!
      end
    end)
  end

  FANPLAY::Retained::FIELDS.each do |klass,fields|
    observer = klass
    fields.each do |field|
      original=FANPLAY::Retained.original_alias(klass,"#{field}=")
      ivar="@#{field}".to_sym
      reference=%i[bitmap viewport pattern].include?(field)
      owned=%i[src_rect rect color tone].include?(field)
      observer.define_method("#{field}=") do |value|
        previous=instance_variable_get(ivar)
        result=send(original,value)
        unless owned
          current=instance_variable_get(ivar)
          FANPLAY::Retained.rewire(self) if reference && !previous.equal?(current)
          if !reference && !value.nil? && value != true && value != false && !value.instance_of?(Integer) && !value.instance_of?(Float)
            __fanplay_volatile(__id)
          end
          __fanplay_touch(self) unless previous.equal?(current)
        end
        result
      end
    end
    watched_ivars=fields.map { |field| "@#{field}".to_sym }.freeze
    original_ivar_set=FANPLAY::Retained.original_alias(klass,:instance_variable_set)
    observer.define_method(:instance_variable_set) do |name,value|
      __fanplay_world_metadata_changed(self) if [:@x,:@y,:@z].include?(name.to_sym)
      result = send(original_ivar_set,name,value)
      if watched_ivars.include?(name.to_sym)
        FANPLAY::Retained.rewire(self)
        __fanplay_volatile(__id)
        __fanplay_touch(self)
      end
      result
    end
    original_dispose=FANPLAY::Retained.original_alias(klass,:dispose)
    observer.define_method(:dispose) do
      id = __id
      __fanplay_world_unbind(self) if instance_variable_defined?(:@__fanplay_world_group)
      __fanplay_touch(self)
      result = send(original_dispose)
      FANPLAY::Retained.forget(id) if id
      result
    end
    if klass != Plane
      original_flash=FANPLAY::Retained.original_alias(klass,:flash)
      observer.define_method(:flash) do |*args|
        result = send(original_flash,*args)
        FANPLAY::Retained.rewire(self)
        __fanplay_touch(self)
        result
      end
      original_update=FANPLAY::Retained.original_alias(klass,:update)
      observer.define_method(:update) do
        flashing = @__fanplay_flashing
        result = send(original_update)
        __fanplay_touch(self) if flashing
        result
      end
    end
    __fanplay_install_scalar_setters(klass, klass==Sprite ? 0 : (klass==Viewport ? 1 : 2))
    original_initialize=FANPLAY::Retained.original_alias(klass,:initialize)
    klass.define_method(:initialize) do |*args,**kwargs,&block|
      result=send(original_initialize,*args,**kwargs,&block)
      FANPLAY::Retained.register(self)
      result
    end
    klass.send(:private,:initialize)
    constructors = Module.new do
      def method_added(name)
        super
        FANPLAY::Retained.invalidate!
      end
      def prepend(*modules)
        result = super
        FANPLAY::Retained.invalidate!
        result
      end
      def include(*modules)
        result = super
        FANPLAY::Retained.invalidate!
        result
      end
      def method_removed(name)
        super
        FANPLAY::Retained.invalidate!
      end
      def method_undefined(name)
        super
        FANPLAY::Retained.invalidate!
      end
      def instances = FANPLAY::Retained.references(self)
      def __sync_all = FANPLAY::Retained.flush(self)
    end
    klass.singleton_class.prepend(constructors)
    names = fields.flat_map { |field| [field,"#{field}=".to_sym] } + %i[__sync update dispose]
    signatures = names.filter_map { |name| [name,klass.instance_method(name)] if klass.method_defined?(name) }
    FANPLAY::Retained.instance_variable_get(:@signatures)[klass] = signatures
  end
  module FANPLAY::RetainedSingletonChanges
    def singleton_method_added(name)
      super
      FANPLAY::Retained.invalidate!
    end
  end
  [Sprite,Viewport,Plane,Rect,Color,Tone].each { |klass| klass.prepend(FANPLAY::RetainedSingletonChanges) }
  # Render-target resizing affects sprites with no viewport as well.
  graphics_class=Graphics.singleton_class
  %i[resize_screen render_size=].each do |name|
    original=Graphics.method(name)
    graphics_class.define_method(name) do |*args|
      result=original.call(*args)
      FANPLAY::Retained.invalidate!
      result
    end
  end
  original_bitmap_dispose=FANPLAY::Retained.original_alias(Bitmap,:dispose)
  Bitmap.define_method(:dispose) do
    __fanplay_touch(self)
    send(original_bitmap_dispose)
  end
end

# The map adapter is installed only for the supported Essentials renderer shape.
# It calls the game's existing refresh methods, preserving terrain/plugin hooks.
module FANPLAY::RetainedMapAdapter
  def update
    unless FANPLAY.retained_map_supported?(self)
      __fanplay_world_release(@__fanplay_cell_cache.object_id,@tiles) if @__fanplay_world_active && @__fanplay_cell_cache
      @__fanplay_world_active=false
      @__fanplay_cell_cache=nil
      @__fanplay_animated=nil
      return FANPLAY.retained_map_original.bind_call(self) if @__fanplay_map_fallback
      @__fanplay_map_fallback=true
      begin
        return super
      ensure
        @__fanplay_map_fallback=false
      end
    end
    @__fanplay_cell_cache ||= FANPLAYRetainedMap.new
    @__fanplay_animated ||= {}
    @__fanplay_tile_maps ||= {}
    @tilesets.update
    @autotiles.update
    full = @need_refresh
    if @old_tone != @tone || @old_color != @color
      @tiles.each { |col| col.each { |stack| stack.each { |tile| tile.tone=@tone; tile.color=@color } } }
      @old_tone=@tone.clone
      @old_color=@color.clone
    end
    if @viewport.ox != @old_viewport_ox || @viewport.oy != @old_viewport_oy
      @old_viewport_ox=@viewport.ox
      @old_viewport_oy=@viewport.oy
      full=true
    end
    @screen_moved=false
    @screen_moved_vertically=false
    if $PokemonGlobal.bridge != @bridge
      @bridge=$PokemonGlobal.bridge
      @screen_moved_vertically=true
    end
    previous_map_id=@current_map_id
    old_column=@tiles[0]
    old_stack=old_column&.first
    full=true if check_if_screen_moved
    # The game rotates its sprite pool at tile boundaries. Rotate the cached
    # cells by the same amount so only newly exposed strips need refreshing.
    shift_x=0
    shift_y=0
    if old_column && !old_column.equal?(@tiles[0])
      index=@tiles.find_index { |column| column.equal?(old_column) }
      shift_x=@tiles_horizontal_count-index if index
    end
    if old_stack && !old_stack.equal?(old_column[0])
      index=old_column.find_index { |stack| stack.equal?(old_stack) }
      shift_y=@tiles_vertical_count-index if index
    end
    @__fanplay_cell_cache.shift(shift_x,shift_y) if shift_x!=0 || shift_y!=0
    maps=$map_factory.maps
    descriptors=maps.map do |map|
      dx=(map.display_x.to_f/Game_Map::X_SUBPIXELS).round
      dy=(map.display_y.to_f/Game_Map::Y_SUBPIXELS).round
      dx=((dx+(Graphics.width/2))*self.class::ZOOM_X)-(Graphics.width/2) if self.class::ZOOM_X != 1
      dy=((dy+(Graphics.height/2))*self.class::ZOOM_Y)-(Graphics.height/2) if self.class::ZOOM_Y != 1
      [map.object_id,map.data,dx/self.class::DISPLAY_TILE_WIDTH,dy/self.class::DISPLAY_TILE_HEIGHT,map.width,map.height,map.data.object_id]
    end
    # Scrolling currently rotates the game's sprite pool. Force coordinate/z
    # refresh for those slots; idle frames and map edits use retained cell diffs.
    changes=@__fanplay_cell_cache.update(@tiles_horizontal_count,@tiles_vertical_count,descriptors,!!full)
    changes.each do |i,j,layer,map_index,tile_id|
      tile=@tiles[i][j][layer]
      tile.instance_variable_set(:@__fanplay_map_edit,true)
      begin
      if map_index
        map=maps[map_index]
        @__fanplay_tile_maps[tile.__id]=map
        tile.instance_variable_set(:@__fanplay_map_covered,true)
        if full || tile.need_refresh || tile.tile_id != tile_id
          refresh_tile(tile,i,j,map,layer,tile_id)
        else
          refresh_tile_coordinates(tile,i,j) if @screen_moved
          refresh_tile_z(tile,map,j,layer,tile_id) if @screen_moved_vertically
        end
        if tile.animated
          @__fanplay_animated[tile.__id]=tile
        else
          @__fanplay_animated.delete(tile.__id)
        end
      else
        __fanplay_world_unbind(tile) if @__fanplay_world_active
        tile.set_bitmap('',0,false,false,0,nil)
        tile.shows_reflection=false
        tile.bridge=false
        @__fanplay_animated.delete(tile.__id)
        @__fanplay_tile_maps.delete(tile.__id)
        tile.instance_variable_set(:@__fanplay_map_covered,false)
      end
      ensure
        tile.instance_variable_set(:@__fanplay_map_edit,false)
      end
    end
    world=false
    if ENV['FANPLAY_WORLD_CAMERA']!='0' && FANPLAY::Retained::ENABLED && FANPLAY.retained_map_bulk_supported?(self)
      world=__fanplay_world_camera(@__fanplay_cell_cache.object_id,@tiles,changes,
        @tile_offset_x,@tile_offset_y,@pixel_offset_x,@pixel_offset_y,
        self.class::DISPLAY_TILE_WIDTH,self.class::DISPLAY_TILE_HEIGHT,self.class::SOURCE_TILE_HEIGHT,
        @bridge>0,!!(full || previous_map_id!=@current_map_id))
      FANPLAY::Retained.stats[:world_camera_frames]+=1 if world
    end
    if !world && @__fanplay_world_active
      __fanplay_world_release(@__fanplay_cell_cache.object_id,@tiles)
    end
    @__fanplay_world_active=world
    bulk=world
    FANPLAY::Retained.stats[:bulk_scrolls]+=1 if world && (@screen_moved || @screen_moved_vertically)
    if !bulk && !full && (@screen_moved || @screen_moved_vertically) && FANPLAY.retained_map_bulk_supported?(self)
      bulk=__fanplay_scroll_tiles(@tiles,@pixel_offset_x,@pixel_offset_y,
        self.class::DISPLAY_TILE_WIDTH,self.class::DISPLAY_TILE_HEIGHT,self.class::SOURCE_TILE_HEIGHT,
        @screen_moved,@screen_moved_vertically,@bridge>0,FANPLAY::Retained::ENABLED)
      FANPLAY::Retained.stats[:bulk_scrolls]+=1 if bulk
    end
    if !bulk && !full && (@screen_moved || @screen_moved_vertically)
      @tiles.each_with_index do |col,i|
        col.each_with_index do |stack,j|
          stack.each_with_index do |tile,layer|
            map=@__fanplay_tile_maps[tile.__id]
            next unless map
            refresh_tile_coordinates(tile,i,j) if @screen_moved
            refresh_tile_z(tile,map,j,layer,tile.tile_id) if @screen_moved_vertically
          end
        end
      end
    end
    if @autotiles.changed
      @__fanplay_animated.each_value { |tile| refresh_tile_frame(tile,tile.tile_id) }
    end
    @need_refresh=false
    @autotiles.changed=false
    FANPLAY::Retained.stats[:map_changed_cells] += changes.length
    false
  end
  def dispose
    return FANPLAY.retained_map_dispose_original.bind_call(self) if @__fanplay_map_disposing
    @__fanplay_map_disposing=true
    begin
      __fanplay_world_release(@__fanplay_cell_cache.object_id,@tiles) if @__fanplay_world_active && @__fanplay_cell_cache
      @__fanplay_world_active=false
      @__fanplay_cell_cache=nil
      @__fanplay_animated&.clear
      @__fanplay_tile_maps&.clear
      super
    ensure
      @__fanplay_map_disposing=false
    end
  end
end
module FANPLAY
  @retained_table_read=Table.instance_method(:[])
  @retained_table_get=Table.instance_method(:__get)
  class << self
    alias_method :__fanplay_install_tile_atlas, :install_native_tilemap
    def install_native_tilemap
      __fanplay_install_tile_atlas
      return if ENV['FANPLAY_RETAINED_MAP']=='0' || !defined?(TilemapRenderer)
      klass=TilemapRenderer
      return unless %i[update refresh_tile check_if_screen_moved].all? { |name| klass.method_defined?(name) }
      return unless %i[ZOOM_X ZOOM_Y DISPLAY_TILE_WIDTH DISPLAY_TILE_HEIGHT].all? { |name| klass.const_defined?(name) }
      unless klass.ancestors.include?(FANPLAY::RetainedMapAdapter)
        %i[priority bridge shows_reflection].each do |field|
          original=FANPLAY::Retained.original_alias(klass::TileSprite,"#{field}=")
          klass::TileSprite.define_method("#{field}=") do |value|
            __fanplay_world_metadata_changed(self)
            send(original,value)
          end
        end
        @retained_map_original=klass.instance_method(:update)
        @retained_map_dispose_original=klass.instance_method(:dispose)
        @retained_map_geometry=%i[refresh_tile_coordinates refresh_tile_z].map { |name| [name,klass.instance_method(name)] }
        @retained_tile_metadata=%i[priority bridge shows_reflection].map { |name| [name,klass::TileSprite.instance_method(name)] }
        klass.prepend(FANPLAY::RetainedMapAdapter)
      end
    end
    attr_reader :retained_map_original,:retained_map_dispose_original
    def retained_map_bulk_supported?(renderer)
      ENV['FANPLAY_NATIVE_SCROLL']!='0' && !FANPLAY::Retained.reclassification_pending? && @retained_map_geometry.all? { |name,method| renderer.method(name).unbind==method } &&
        @retained_tile_metadata.all? { |name,method| TilemapRenderer::TileSprite.instance_method(name)==method }
    end
    def retained_map_supported?(renderer)
      # A plugin replacing update keeps full control through super. A subclass
      # can also opt out naturally by overriding update.
      renderer.method(:update).super_method&.unbind == @retained_map_original &&
        $map_factory.maps.all? { |map| map.data.instance_of?(Table) && map.data.method(:[]).unbind == @retained_table_read && map.data.method(:__get).unbind == @retained_table_get }
    end
  end
end

if FANPLAY::Retained::ENABLED
  module FANPLAY::RetainedFinalizers
    def finalizer(op,id)
      original=super
      proc do
        begin
          original.call
        ensure
          FANPLAY::Retained.forget(id)
        end
      end
    end
  end
  FANPLAY.singleton_class.prepend(FANPLAY::RetainedFinalizers)
end

# Native RGSS1 tilemaps own their render sprites. Capture the engine's Sprite
# API before game plugins install screen-resizer wrappers on public Sprites.
# This mirrors a real RGSS Tilemap, whose internals are not game Sprite objects.
if FANPLAY::Retained::ENABLED
  class FANPLAY::TilemapSprite < Sprite; end
  (Sprite.instance_methods(false)+[:initialize]).uniq.each do |name|
    FANPLAY::TilemapSprite.define_method(name,Sprite.instance_method(name))
  end
  FANPLAY::TilemapSprite.send(:private,:initialize)
  signatures=(FANPLAY::Retained::FIELDS[Sprite].flat_map { |field| [field,"#{field}=".to_sym] } + %i[__sync update dispose]).map { |name| [name,FANPLAY::TilemapSprite.instance_method(name)] }
  FANPLAY::Retained.instance_variable_set(:@tilemap_signatures,signatures)
end

module FANPLAY::LegacyTilemapCache
  READ=Table.instance_method(:[])
  TILE=Tilemap.instance_method(:__tile)
  def __sync
    supported=ENV['FANPLAY_RETAINED_TILEMAP']!='0' && FANPLAY::Retained::ENABLED &&
      @ox.instance_of?(Integer) && @oy.instance_of?(Integer) &&
      @map_data.instance_of?(Table) && @map_data.method(:[]).unbind==READ &&
      (!@priorities || (@priorities.instance_of?(Table) && @priorities.method(:[]).unbind==READ)) &&
      method(:__tile).unbind==TILE &&
      (!defined?($ResizeFactorMul) || $ResizeFactorMul==100) &&
      (@viewport || ((!defined?($ResizeOffsetX) || $ResizeOffsetX==0) &&
        (!defined?($ResizeOffsetY) || $ResizeOffsetY==0)))
    unless supported
      @__fanplay_layout=nil
      @__fanplay_batch_tiles=false
      return super
    end
    width=@viewport ? @viewport.rect.width : Graphics.width
    height=@viewport ? @viewport.rect.height : Graphics.height
    key=[@visible,@map_data,@map_data.__fanplay_revision,@priorities,@priorities&.__fanplay_revision,
      @viewport,@viewport&.disposed?,width,height,@ox.div(32),@oy.div(32),
      (@ox+width-1).div(32),(@oy+height-1).div(32),
      @tileset,@tileset&.disposed?,@color.to_a,[@tone.red,@tone.green,@tone.blue,@tone.gray],
      *Array.new(7) { |i| bitmap=@autotiles[i]; [bitmap,bitmap&.disposed?] }]
    dx=@ox-(@__fanplay_last_ox || @ox)
    dy=@oy-(@__fanplay_last_oy || @oy)
    if key==@__fanplay_layout && ((dx==0 && dy==0) || __fanplay_scroll_rgss_tiles(@sprites,dx,dy))
      if @__fanplay_last_animation!=@animation/16
        updated=0
        (@__fanplay_animated_tiles || []).each do |sprite,base,step,frames|
          x=base+(@animation/16%frames)*step
          next if sprite.src_rect.x==x
          sprite.src_rect.x=x
          updated+=1
        end
        FANPLAY::Retained.stats[:legacy_tilemap_animated_tiles]+=updated
      end
      FANPLAY::Retained.stats[:legacy_tilemap_reused]+=1
    else
      @__fanplay_batch_tiles=ENV['FANPLAY_NATIVE_TILEMAP_BATCH']!='0' && [Rect,Color,Tone].all? do |klass|
        FANPLAY::Retained.instance_variable_get(:@value_signatures).fetch(klass).all? { |name,method| klass.instance_method(name)==method }
      end
      super
      @__fanplay_layout=key
      FANPLAY::Retained.stats[:legacy_tilemap_rebuilt]+=1
    end
    @__fanplay_last_ox=@ox
    @__fanplay_last_oy=@oy
    @__fanplay_last_animation=@animation/16
  end
end
Tilemap.prepend(FANPLAY::LegacyTilemapCache)
