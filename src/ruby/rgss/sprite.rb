class Sprite
  include NativeResource
  include RSXP::Flashable
  @instances = []
  class << self
    attr_reader :instances
    def __sync_all
      Sprite.instances.delete_if do |reference|
        begin
          sprite = reference.__getobj__
        rescue RSXP::ResourceRef::RefError
          next true
        end
        next true if sprite.disposed?
        sprite.__sync
        # Disposal already removes the native resource immediately. If a
        # custom sync disposes itself, prune its weak entry on the next pass.
        false
      end
    end
  end
  attr_accessor :x,:y,:z,:ox,:oy,:zoom_x,:zoom_y,:visible
  attr_accessor :mirror,:blend_type,:angle,:invert
  attr_accessor :bush_depth,:bush_opacity
  attr_accessor :pattern,:pattern_opacity,:pattern_tile
  attr_accessor :pattern_scroll_x,:pattern_scroll_y,:pattern_zoom_x,:pattern_zoom_y
  attr_accessor :pattern_blend_type
  # Wave effects are accepted for compatibility; rsxp does not render them.
  attr_accessor :wave_amp,:wave_length,:wave_speed,:wave_phase
  attr_reader :bitmap,:src_rect,:opacity,:color,:tone
  attr_accessor :viewport
  def initialize(viewport=nil)
    raise TypeError, 'expected Viewport or nil' unless viewport.nil? || viewport.is_a?(Viewport)
    __own(RSXP.call('sprite_new'),'sprite_dispose')
    @x=@y=@z=@ox=@oy=0
    @zoom_x=@zoom_y=1.0
    @visible=true
    @opacity=255
    @viewport=viewport
    @src_rect=Rect.new
    @color=Color.new(0,0,0,0)
    @tone=Tone.new
    @mirror=false
    @blend_type=0
    @angle=0
    @invert=false
    @bush_depth=0
    @bush_opacity=128
    @pattern=nil
    @pattern_opacity=255
    @pattern_tile=true
    @pattern_scroll_x=@pattern_scroll_y=0
    @pattern_zoom_x=@pattern_zoom_y=1.0
    @pattern_blend_type=0
    Sprite.instance_variable_get(:@instances) << RSXP::ResourceRef.new(self)
  end
  def initialize_copy(_other)
    raise TypeError, 'Sprite cannot be copied'
  end
  def bitmap=(value)
    __check
    raise TypeError, 'expected Bitmap or nil' unless value.nil? || value.is_a?(Bitmap)
    value.__check if value
    return if @bitmap.equal?(value)
    @bitmap=value
    @src_rect.set(value ? value.rect : Rect.new)
  end
  def src_rect=(value)
    raise TypeError, 'expected Rect' unless value.is_a?(Rect)
    @src_rect.set(value)
  end
  def opacity=(value)
    @opacity=RSXP.channel(value)
  end
  def color=(value)
    @color.set(value)
  end
  def tone=(value)
    @tone.set(value)
  end
  def width = src_rect.width
  def height = src_rect.height
  def __sync
    __check
    blend=blend_type.nil? ? 0 : blend_type
    if ![0,1,2].include?(blend)
      raise NotImplementedError, "sprite blend type #{blend} is not supported"
    end
    zx,zy=Float(zoom_x || 1.0),Float(zoom_y || 1.0)
    shown=visible && bitmap && !bitmap.disposed?
    clip=Rect.new(0,0,Graphics.width,Graphics.height)
    if viewport
      shown &&= viewport.visible && !viewport.disposed?
      clip=viewport.rect
    end
    if pattern && pattern.disposed?
      raise RGSSError, 'disposed pattern Bitmap'
    end
    sync_color=__flash_color_for_sync(@color)
    RSXP.sync(self,'sprite_sync',bitmap ? bitmap.__id : 0,*src_rect.to_a,
      Integer(x || 0),Integer(y || 0),Integer(ox || 0),Integer(oy || 0),zx,zy,Float(angle || 0),mirror ? 1 : 0,
      opacity || 255,shown ? 1 : 0,*clip.to_a,Integer(z || 0),
      Integer(blend),*sync_color.to_a,*[@tone.red,@tone.green,@tone.blue,@tone.gray].map(&:to_i),invert ? 1 : 0,
      pattern ? pattern.__id : 0,Integer(pattern_blend_type || 0),pattern_tile ? 1 : 0,RSXP.channel(pattern_opacity || 255),
      Integer(pattern_scroll_x || 0),Integer(pattern_scroll_y || 0),Float(pattern_zoom_x || 1.0),Float(pattern_zoom_y || 1.0),
      viewport ? viewport.__id : 0,Integer(bush_depth || 0),RSXP.channel(bush_opacity || 128))
  end
end

class Sprite
  # Keep virtual getter calls on the Ruby side. Desktop submits typed values;
  # the browser serializes only changed sprites. A reusable snapshot
  # of the last submitted state is compared first so an unchanged sprite costs
  # no native call; sprites that are not drawn (no bitmap/viewport) take an even
  # shorter path because their geometry is irrelevant.
  NATIVE_SYNC = private_method_defined?(:__rsxp_sprite_sync)
  if ENV['RSXP_SPRITE_CACHE']!='0'
    def __sync
      __check
      bmp=bitmap
      vp=viewport
      pat=pattern
      bmp_id=bmp ? bmp.__id : 0
      vp_id=vp ? vp.__id : 0
      pat_id=pat ? pat.__id : 0
      bmp_disposed=bmp ? bmp.disposed? : false
      pat_disposed=pat ? pat.disposed? : false
      vis=visible
      shown=vis && bmp && !bmp_disposed
      shown &&= vp.visible && !vp.disposed? if vp
      # A fixed snapshot array is reused in place, so an unchanged sprite costs
      # no native call and no allocation.
      state=@__sync_state
      if state.nil?
        state=@__sync_state=Array.new(43)
      elsif NATIVE_SYNC && !shown && state[0]==shown && state[1]==bmp_id && state[2]==vp_id &&
            state[3]==pat_id && state[4]==bmp_disposed && state[5]==pat_disposed
        # Not drawn: only the objects that could make it drawable again matter.
        return
      end
      blend=blend_type
      zx=zoom_x
      zy=zoom_y
      src=src_rect
      sx=src.x
      sy=src.y
      sw=src.width
      sh=src.height
      px=x
      py=y
      pox=ox
      poy=oy
      pang=angle
      pmirror=mirror
      pz=z
      pop=opacity
      pinv=invert
      pblend=pattern_blend_type
      ptile=pattern_tile
      popac=pattern_opacity
      psx=pattern_scroll_x
      psy=pattern_scroll_y
      pzx=pattern_zoom_x
      pzy=pattern_zoom_y
      bush=bush_depth
      bush_alpha=bush_opacity
      # Plugins can leave a sprite property unset. RGSS reads missing numeric
      # properties as their defaults, so coerce nil instead of failing the frame.
      blend=0 if blend.nil?
      zx=1.0 if zx.nil?
      zy=1.0 if zy.nil?
      vis=false if vis.nil?
      px=0 if px.nil?
      py=0 if py.nil?
      pox=0 if pox.nil?
      poy=0 if poy.nil?
      pang=0 if pang.nil?
      pmirror=false if pmirror.nil?
      pz=0 if pz.nil?
      pop=255 if pop.nil?
      pinv=false if pinv.nil?
      pblend=0 if pblend.nil?
      ptile=false if ptile.nil?
      popac=255 if popac.nil?
      psx=0 if psx.nil?
      psy=0 if psy.nil?
      pzx=1.0 if pzx.nil?
      pzy=1.0 if pzy.nil?
      bush=0 if bush.nil?
      bush_alpha=128 if bush_alpha.nil?
      if vp
        clip=vp.rect
        clip_x=clip.x
        clip_y=clip.y
        clip_w=clip.width
        clip_h=clip.height
      else
        clip_x=0
        clip_y=0
        clip_w=Graphics.width
        clip_h=Graphics.height
      end
      sync_color=__flash_color_for_sync(@color)
      cr=sync_color.red
      cg=sync_color.green
      cb=sync_color.blue
      ca=sync_color.alpha
      tone=@tone
      tr=tone.red
      tg=tone.green
      tb=tone.blue
      tgray=tone.gray
      return if state[0]==shown && state[1]==bmp_id && state[2]==vp_id && state[3]==pat_id &&
                state[4]==bmp_disposed && state[5]==pat_disposed && state[6]==blend &&
                state[7]==clip_x && state[8]==clip_y && state[9]==clip_w && state[10]==clip_h &&
                state[11]==sx && state[12]==sy && state[13]==sw && state[14]==sh &&
                state[15]==px && state[16]==py && state[17]==pox && state[18]==poy &&
                state[19]==zx && state[20]==zy && state[21]==pang && state[22]==pmirror &&
                state[23]==pz && state[24]==pop && state[25]==pinv &&
                state[26]==pblend && state[27]==ptile && state[28]==popac &&
                state[29]==psx && state[30]==psy && state[31]==pzx && state[32]==pzy &&
                state[33]==cr && state[34]==cg && state[35]==cb && state[36]==ca &&
                state[37]==tr && state[38]==tg && state[39]==tb && state[40]==tgray && state[41]==bush && state[42]==bush_alpha
      if NATIVE_SYNC
        __rsxp_sprite_sync(self,__id,blend,zx,zy,bmp,vp,pat,vis,
          sx,sy,sw,sh,px,py,pox,poy,pang,pmirror,pz,pop,pinv,
          pblend,ptile,popac,psx,psy,pzx,pzy,cr,cg,cb,ca,tr,tg,tb,tgray,bush,bush_alpha)
      else
        unless blend==0 || blend==1 || blend==2
          raise NotImplementedError, "sprite blend type #{blend} is not supported"
        end
        native_zx,native_zy=Float(zx),Float(zy)
        raise RGSSError, 'disposed pattern Bitmap' if pat_disposed
        # Only changed sprites allocate/convert a command. The snapshot above
        # already performs the comparison, so no second RSXP.sync Array is needed.
        RSXP::SYNC_QUEUE << ['sprite_sync',__id,bmp_id,Integer(sx),Integer(sy),Integer(sw),Integer(sh),
          Integer(px),Integer(py),Integer(pox),Integer(poy),native_zx,native_zy,Float(pang),pmirror ? 1 : 0,
          pop,shown ? 1 : 0,clip_x,clip_y,clip_w,clip_h,Integer(pz),Integer(blend),
          cr.to_i,cg.to_i,cb.to_i,ca.to_i,tr.to_i,tg.to_i,tb.to_i,tgray.to_i,pinv ? 1 : 0,
          pat_id,Integer(pblend),ptile ? 1 : 0,RSXP.channel(popac),Integer(psx),Integer(psy),
          Float(pzx),Float(pzy),vp_id,Integer(bush),RSXP.channel(bush_alpha)].join("\t").b
      end
      state[0]=shown
      state[1]=bmp_id
      state[2]=vp_id
      state[3]=pat_id
      state[4]=bmp_disposed
      state[5]=pat_disposed
      state[6]=blend
      state[7]=clip_x
      state[8]=clip_y
      state[9]=clip_w
      state[10]=clip_h
      state[11]=sx
      state[12]=sy
      state[13]=sw
      state[14]=sh
      state[15]=RSXP.snapshot_number(px)
      state[16]=RSXP.snapshot_number(py)
      state[17]=RSXP.snapshot_number(pox)
      state[18]=RSXP.snapshot_number(poy)
      state[19]=RSXP.snapshot_number(zx)
      state[20]=RSXP.snapshot_number(zy)
      state[21]=RSXP.snapshot_number(pang)
      state[22]=pmirror
      state[23]=RSXP.snapshot_number(pz)
      state[24]=RSXP.snapshot_number(pop)
      state[25]=pinv
      state[26]=RSXP.snapshot_number(pblend)
      state[27]=ptile
      state[28]=RSXP.snapshot_number(popac)
      state[29]=RSXP.snapshot_number(psx)
      state[30]=RSXP.snapshot_number(psy)
      state[31]=RSXP.snapshot_number(pzx)
      state[32]=RSXP.snapshot_number(pzy)
      state[33]=cr
      state[34]=cg
      state[35]=cb
      state[36]=ca
      state[37]=tr
      state[38]=tg
      state[39]=tb
      state[40]=tgray
      state[41]=RSXP.snapshot_number(bush)
      state[42]=RSXP.snapshot_number(bush_alpha)
    end
  end
end

