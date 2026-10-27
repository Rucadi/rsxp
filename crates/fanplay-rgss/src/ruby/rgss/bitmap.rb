class Font
  class << self
    attr_accessor :default_name,:default_size,:default_bold,:default_italic,:default_color,:default_outline,:default_shadow,:default_out_color
  end
  self.default_name='Arial'
  self.default_size=22
  self.default_bold=false
  self.default_italic=false
  self.default_color=Color.new(255,255,255)
  self.default_outline=false
  self.default_shadow=false
  self.default_out_color=Color.new(0,0,0,128)
  attr_accessor :name,:size,:bold,:italic,:color,:outline,:shadow,:out_color
  # mkxp-z exposes short aliases for bold/italic.
  alias_method :b, :bold
  alias_method :b=, :bold=
  alias_method :i, :italic
  alias_method :i=, :italic=
  def initialize(name=nil,size=nil)
    @name=name || Font.default_name
    @size=size || Font.default_size
    @bold,@italic=Font.default_bold,Font.default_italic
    @color=Font.default_color.clone
    @outline,@shadow=Font.default_outline,Font.default_shadow
    @out_color=Font.default_out_color.clone
  end
  def self.exist?(name)
    Dir.glob('Fonts/*').any? { |path| File.basename(path,File.extname(path)).casecmp?(name.to_s) }
  end
  def __payload(text) = Array(name).join("\x1f").b+"\0"+FANPLAY.utf8(text).b
end

class Bitmap
  def self.max_size = 4096
  include NativeResource
  attr_writer :font
  def font = (@font ||= Font.new)
  def initialize(width,height=nil)
    id=height.nil? ? FANPLAY.call('bitmap_load',payload:String(width)) : FANPLAY.call('bitmap_new',Integer(width),Integer(height))
    __own(id,'bitmap_dispose')
  end
  def self.__from_id(id)
    object = allocate
    object.__own(id,'bitmap_dispose')
    object
  end
  def initialize_copy(other)
    other.__check
    defined?(__sabi_disown) ? __sabi_disown(self) : ObjectSpace.undefine_finalizer(self)
    __own(FANPLAY.call('bitmap_clone',other.__id),'bitmap_dispose')
  end
  def width
    __check
    (@__dimensions ||= FANPLAY.call('bitmap_size',__id).split("\t").map!(&:to_i))[0]
  end
  def height
    __check
    (@__dimensions ||= FANPLAY.call('bitmap_size',__id).split("\t").map!(&:to_i))[1]
  end
  def rect = Rect.new(0,0,width,height)
  # Animated GIFs decode in the engine and animate using the host clock.
  def animated?
    __check
    FANPLAY.call('bitmap_animated',__id) != '0'
  end
  def frame_count
    __check
    FANPLAY.call('bitmap_frame_count',__id).to_i
  end
  def current_frame
    __check
    FANPLAY.call('bitmap_current_frame',__id).to_i
  end
  def frame_rate
    __check
    FANPLAY.call('bitmap_frame_rate',__id).to_f
  end
  def frame_rate=(value)
    __check
    FANPLAY.call('bitmap_frame_rate_set',__id,Float(value))
  end
  # Bitmaps here are never "mega" (split) surfaces.
  def mega? = false
  def playing
    __check
    FANPLAY.call('bitmap_playing',__id) != '0'
  end
  def playing=(value)
    __check
    FANPLAY.call('bitmap_playing_set',__id,value ? 1 : 0)
  end
  def goto_and_stop(frame)
    __check
    FANPLAY.call('bitmap_goto_and_stop',__id,Integer(frame))
    nil
  end
  def goto_and_play(frame)
    __check
    FANPLAY.call('bitmap_goto_and_play',__id,Integer(frame))
    nil
  end
  def play
    __check
    FANPLAY.call('bitmap_play',__id)
    nil
  end
  def stop
    __check
    FANPLAY.call('bitmap_stop',__id)
    nil
  end
  # Raw RGBA bytes, matching mkxp-z's Bitmap#raw_data / #raw_data=.
  def raw_data
    __check
    FANPLAY.call('bitmap_raw_data',__id).force_encoding(Encoding::BINARY)
  end
  def raw_data=(value)
    __check
    FANPLAY.call('bitmap_set_raw_data',__id,payload:String(value).b)
    nil
  end
  # KGL2 compatibility operations driven by KGL2_Impl below.
  def _kgl_invert
    __check
    FANPLAY.call('bitmap_kgl_invert',__id)
    nil
  end
  def _kgl_compress_alpha
    __check
    FANPLAY.call('bitmap_kgl_compress_alpha',__id)
    nil
  end
  def _kgl_subtract_rect(x,y,source,area,opacity=255)
    __check
    source.__check
    FANPLAY.call('bitmap_kgl_subtract',__id,Integer(x),Integer(y),source.__id,*area.to_a,FANPLAY.channel(opacity))
    nil
  end
  def _kgl_shadow_shader_h(x1,x2,y,soft)
    __check
    FANPLAY.call('bitmap_kgl_shadow_h',__id,Integer(x1),Integer(x2),Integer(y),soft ? 1 : 0).to_i
  end
  def _kgl_shadow_shader_v(y1,y2,x,wall,soft)
    __check
    FANPLAY.call('bitmap_kgl_shadow_v',__id,Integer(y1),Integer(y2),Integer(x),wall ? 1 : 0,soft ? 1 : 0).to_i
  end
  # Hi-res asset replacement is not implemented; a bitmap is its own hi-res copy.
  def hires = self
  def gradient_fill_rect(*args)
    __check
    vertical = args.last == true || args.last == false ? args.pop : false
    if args.size == 3 && args[0].is_a?(Rect)
      area, color1, color2 = args
    elsif args.size == 6
      area = Rect.new(*args[0, 4])
      color1 = args[4]
      color2 = args[5]
    else
      raise ArgumentError, 'gradient_fill_rect expects a Rect or x, y, width, height'
    end
    raise TypeError, 'expected Color' unless color1.is_a?(Color) && color2.is_a?(Color)
    FANPLAY.call('bitmap_gradient_fill',__id,*area.to_a,*color1.to_a,*color2.to_a,vertical ? 1 : 0)
    nil
  end
  def blur
    __check
    FANPLAY.call('bitmap_blur',__id)
    nil
  end
  def radial_blur(angle,divisions)
    __check
    FANPLAY.call('bitmap_radial_blur',__id,Integer(angle),Integer(divisions))
    nil
  end
  def snap_to_bitmap(position=nil)
    __check
    Bitmap.__from_id(FANPLAY.call('bitmap_snap',__id,position.nil? ? -1 : Integer(position)))
  end
  def to_file(filename)
    __check
    FANPLAY.call('bitmap_to_file',__id,payload:String(filename))
    nil
  end
  def hue_change(value)
    __check
    FANPLAY.call('bitmap_hue',__id,Float(value))
    nil
  end
  def clear
    __check
    FANPLAY.call('bitmap_clear',__id)
    nil
  end
  def fill_rect(*args)
    __check
    color = args.pop
    raise TypeError, 'expected Color' unless color.is_a?(Color)
    area = args.size == 1 && args[0].is_a?(Rect) ? args[0] : Rect.new(*args)
    FANPLAY.call('bitmap_fill',__id,*area.to_a,*color.to_a)
    nil
  end
  def clear_rect(*args) = fill_rect(*args,Color.new(0,0,0,0))
  def get_pixel(x,y)
    __check
    Color.new(*FANPLAY.call('bitmap_get',__id,Integer(x),Integer(y)).split("\t").map(&:to_i))
  end
  def set_pixel(x,y,color)
    __check
    raise TypeError, 'expected Color' unless color.is_a?(Color)
    FANPLAY.call('bitmap_set',__id,Integer(x),Integer(y),*color.to_a)
    nil
  end
  def blt(x,y,source,area,opacity=255)
    stretch_blt(Rect.new(x,y,area.width,area.height),source,area,opacity)
  end
  def stretch_blt(destination,source,area,opacity=255)
    __check
    source.__check
    FANPLAY.call('bitmap_blt',__id,*destination.to_a,source.__id,*area.to_a,FANPLAY.channel(opacity))
    nil
  end
  def text_size(text)
    __check
    w,h=FANPLAY.call('text_size',Integer(font.size),font.bold ? 1 : 0,font.italic ? 1 : 0,payload:font.__payload(text)).split("\t").map(&:to_i)
    Rect.new(0,0,w,h)
  end
  def draw_text(*args)
    __check
    area=args.first.is_a?(Rect) ? args.shift : Rect.new(*args.shift(4))
    value=args.shift
    align=Integer(args.shift || 0)
    raise ArgumentError,'too many draw_text arguments' unless args.empty?
    FANPLAY.call('bitmap_text',__id,*area.to_a,align,Integer(font.size),font.bold ? 1 : 0,font.italic ? 1 : 0,*font.color.to_a,font.outline ? 1 : 0,font.shadow ? 1 : 0,*font.out_color.to_a,payload:font.__payload(value))
    nil
  end
end

