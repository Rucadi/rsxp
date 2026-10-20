# RGSS1 tilemap. Visible cells use the same native tile sprites as Essentials;
# map ownership and animation remain here, atlas caching and rendering in Rust.
class Tilemap
  AUTOTILE_PARTS = [
     [ [27, 28, 33, 34], [ 5, 28, 33, 34], [27,  6, 33, 34], [ 5,  6, 33, 34],
       [27, 28, 33, 12], [ 5, 28, 33, 12], [27,  6, 33, 12], [ 5,  6, 33, 12] ],
     [ [27, 28, 11, 34], [ 5, 28, 11, 34], [27,  6, 11, 34], [ 5,  6, 11, 34],
       [27, 28, 11, 12], [ 5, 28, 11, 12], [27,  6, 11, 12], [ 5,  6, 11, 12] ],
     [ [25, 26, 31, 32], [25,  6, 31, 32], [25, 26, 31, 12], [25,  6, 31, 12],
       [15, 16, 21, 22], [15, 16, 21, 12], [15, 16, 11, 22], [15, 16, 11, 12] ],
     [ [29, 30, 35, 36], [29, 30, 11, 36], [ 5, 30, 35, 36], [ 5, 30, 11, 36],
       [39, 40, 45, 46], [ 5, 40, 45, 46], [39,  6, 45, 46], [ 5,  6, 45, 46] ],
     [ [25, 30, 31, 36], [15, 16, 45, 46], [13, 14, 19, 20], [13, 14, 19, 12],
       [17, 18, 23, 24], [17, 18, 11, 24], [41, 42, 47, 48], [ 5, 42, 47, 48] ],
     [ [37, 38, 43, 44], [37,  6, 43, 44], [13, 18, 19, 24], [13, 14, 43, 44],
       [37, 42, 43, 48], [17, 18, 47, 48], [13, 18, 43, 48], [ 1,  2,  7,  8] ]
  ].flatten(1).map(&:freeze).freeze
  @instances=[]
  class << self
    attr_reader :instances
    def __sync_all
      instances.delete_if do |reference|
        begin
          tilemap=reference.__getobj__
        rescue RSXP::ResourceRef::RefError
          next true
        end
        next true if tilemap.disposed?
        tilemap.__sync
        false
      end
    end
  end
  attr_accessor :tileset,:map_data,:flash_data,:priorities,:visible,:ox,:oy
  attr_reader :viewport,:autotiles,:color,:tone
  def initialize(viewport=nil)
    @viewport=viewport
    @tileset=@map_data=@flash_data=@priorities=nil
    @autotiles=Array.new(7)
    @visible=true
    @disposed=false
    @ox=@oy=@animation=0
    @color=Color.new(0,0,0,0)
    @tone=Tone.new
    @sprites=[]
    Tilemap.instances << RSXP::ResourceRef.new(self)
  end
  def color=(value)
    @color.set(value)
  end
  def tone=(value)
    @tone.set(value)
  end
  def disposed? = @disposed
  def dispose
    return if @disposed
    @sprites.each(&:dispose)
    @sprites.clear
    @disposed=true
  end
  def update
    raise RGSSError, 'disposed Tilemap' if disposed?
    @animation+=1
  end
  def __sync
    internal=!!(defined?(RSXP::TilemapSprite) && ENV['RSXP_RETAINED_TILEMAP']!='0' &&
      (!defined?($ResizeFactorMul) || $ResizeFactorMul==100) &&
      (@viewport || ((!defined?($ResizeOffsetX) || $ResizeOffsetX==0) &&
        (!defined?($ResizeOffsetY) || $ResizeOffsetY==0))))
    if @__rsxp_internal_mode!=internal
      @sprites.each(&:dispose)
      @sprites.clear
      @__rsxp_internal_mode=internal
    end
    @__rsxp_animated_tiles=[] if internal
    @__rsxp_tile_plan=internal && @__rsxp_batch_tiles ? [] : nil
    used=0
    if @visible && @map_data && (!@viewport || !@viewport.disposed?)
      width=@viewport ? @viewport.rect.width : Graphics.width
      height=@viewport ? @viewport.rect.height : Graphics.height
      left=[@ox.div(32),0].max
      top=[@oy.div(32),0].max
      right=[(@ox+width-1).div(32),@map_data.xsize-1].min
      bottom=[(@oy+height-1).div(32),@map_data.ysize-1].min
      @map_data.zsize.times do |layer|
        (top..bottom).each do |y|
          (left..right).each do |x|
            id=@map_data[x,y,layer]
            next if id<48
            priority=@priorities ? @priorities[id].to_i : 0
            @__rsxp_tile_priority=priority!=0
            @__rsxp_tile_animation=nil
            z=priority==0 ? 0 : (y+priority)*32+32-@oy
            if id>=384
              next unless @tileset && !@tileset.disposed?
              used=__tile(used,@tileset,(id-384)%8*32,(id-384)/8*32,32,
                x*32-@ox,y*32-@oy,z)
            else
              bitmap=@autotiles[id/48-1]
              next unless bitmap && !bitmap.disposed?
              if bitmap.height==32
                @__rsxp_tile_animation=[32,bitmap.width/32] if internal && bitmap.width>32
                used=__tile(used,bitmap,(@animation/16%[bitmap.width/32,1].max)*32,0,32,
                  x*32-@ox,y*32-@oy,z)
              else
                @__rsxp_tile_animation=[96,bitmap.width/96] if internal && bitmap.width>96
                frame=(@animation/16%[bitmap.width/96,1].max)*96
                AUTOTILE_PARTS[id%48].each_with_index do |part,index|
                  part-=1
                  used=__tile(used,bitmap,frame+part%6*16,part/6*16,16,
                    x*32-@ox+index%2*16,y*32-@oy+index/2*16,z)
                end
              end
            end
          end
        end
      end
    end
    if @__rsxp_tile_plan
      clip=@viewport ? @viewport.rect.to_a : [0,0,Graphics.width,Graphics.height]
      plan=@__rsxp_tile_plan
      @__rsxp_tile_plan=nil
      unless __rsxp_apply_rgss_tiles(@sprites,plan,@color.to_a,
          [@tone.red,@tone.green,@tone.blue,@tone.gray],@viewport,clip)
        plan.each_with_index do |row,index|
          @__rsxp_tile_priority=row[7]
          @__rsxp_tile_animation=nil
          __tile(index,*row[0,7])
        end
        @sprites.drop(used).each { |sprite| sprite.visible=false }
      end
    else
      @sprites.drop(used).each { |sprite| sprite.visible=false }
    end
  end
  def __tile(index,bitmap,sx,sy,size,x,y,z)
    sprite=@sprites[index]
    unless sprite
      klass=@__rsxp_internal_mode ? RSXP::TilemapSprite : Sprite
      sprite=klass.new(@viewport)
      RSXP.call('sprite_tile',sprite.__id)
      @sprites[index]=sprite
    end
    if @__rsxp_tile_plan
      @__rsxp_tile_plan << [bitmap,sx,sy,size,x,y,z,@__rsxp_tile_priority]
      if @__rsxp_tile_animation
        step,frames=@__rsxp_tile_animation
        @__rsxp_animated_tiles << [sprite,sx-(@animation/16%frames)*step,step,frames]
      end
      return index+1
    end
    sprite.instance_variable_set(:@__rsxp_rgss_priority,@__rsxp_tile_priority)
    sprite.bitmap=bitmap
    sprite.src_rect.set(sx,sy,size,size)
    sprite.x=x
    sprite.y=y
    sprite.z=z
    sprite.visible=true
    sprite.color=@color
    sprite.tone=@tone
    if @__rsxp_internal_mode && @__rsxp_tile_animation
      step,frames=@__rsxp_tile_animation
      @__rsxp_animated_tiles << [sprite,sx-(@animation/16%frames)*step,step,frames]
    end
    index+1
  end
end

