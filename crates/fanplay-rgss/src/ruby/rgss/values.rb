class Color
  attr_reader :red, :green, :blue, :alpha
  def initialize(red=0, green=0, blue=0, alpha=255) = set(red, green, blue, alpha)
  def set(red, green=nil, blue=nil, alpha=255)
    if red.is_a?(Color)
      red, green, blue, alpha = red.red, red.green, red.blue, red.alpha
    end
    self.red, self.green, self.blue, self.alpha = red, green, blue, alpha
    self
  end
  %i[red green blue alpha].each do |key|
    class_eval "def #{key}=(value); @#{key} = Float(value).clamp(0.0,255.0); end", __FILE__, __LINE__
  end
  def ==(other)
    other.is_a?(Color) && red == other.red && green == other.green && blue == other.blue && alpha == other.alpha
  end
  alias === ==
  alias eql? ==
  def to_a = [red.to_i, green.to_i, blue.to_i, alpha.to_i]
  def _dump(_depth) = [red, green, blue, alpha].pack('E4')
  def self._load(bytes)
    raise TypeError, 'bad Color payload' unless bytes.bytesize == 32
    new(*bytes.unpack('E4'))
  end
end

module FANPLAY::Flashable
  def flash(color,duration)
    __check
    duration=Integer(duration)
    return nil if duration<1
    raise TypeError, 'expected Color or nil' unless color.nil? || color.is_a?(Color)
    @__fanplay_flash_color=Color.new(0,0,0,0) unless @__fanplay_flash_color
    @__fanplay_flashing=true
    @__fanplay_flash_duration=duration
    @__fanplay_flash_counter=0
    if color.nil?
      @__fanplay_empty_flash=true
      return nil
    end
    # Color#initialize may be overridden by game scripts with stricter
    # argument handling, so don't rely on its optional zero-argument form.
    @__fanplay_flash_color=Color.new(0,0,0,0).set(color)
    @__fanplay_flash_alpha=@__fanplay_flash_color.alpha
    @__fanplay_empty_flash=false
    nil
  end
  def update
    __check
    return nil unless @__fanplay_flashing
    @__fanplay_flash_counter+=1
    if @__fanplay_flash_counter>@__fanplay_flash_duration
      @__fanplay_flash_color=Color.new(0,0,0,0)
      @__fanplay_flashing=false
      @__fanplay_empty_flash=false
      return nil
    end
    unless @__fanplay_empty_flash
      alpha=@__fanplay_flash_alpha || @__fanplay_flash_color.alpha || 0
      @__fanplay_flash_color.alpha=alpha*(1.0-@__fanplay_flash_counter.to_f/@__fanplay_flash_duration)
    end
    nil
  end
  def __flash_color_for_sync(color)
    @__fanplay_flashing && @__fanplay_flash_color && @__fanplay_flash_color.alpha>color.alpha ? @__fanplay_flash_color : color
  end
end

class Tone
  attr_reader :red, :green, :blue, :gray
  def initialize(red=0, green=0, blue=0, gray=0) = set(red, green, blue, gray)
  def set(red, green=nil, blue=nil, gray=0)
    if red.is_a?(Tone)
      red, green, blue, gray = red.red, red.green, red.blue, red.gray
    end
    self.red, self.green, self.blue, self.gray = red, green, blue, gray
    self
  end
  %i[red green blue].each do |key|
    class_eval "def #{key}=(value); @#{key} = Float(value).clamp(-255.0,255.0); end", __FILE__, __LINE__
  end
  def gray=(value)
    @gray = Float(value).clamp(0.0,255.0)
  end
  def ==(other)
    other.is_a?(Tone) && red == other.red && green == other.green && blue == other.blue && gray == other.gray
  end
  alias === ==
  alias eql? ==
  def _dump(_depth) = [red,green,blue,gray].pack('E4')
  def self._load(bytes)
    raise TypeError, 'bad Tone payload' unless bytes.bytesize == 32
    new(*bytes.unpack('E4'))
  end
end

class Rect
  attr_reader :x, :y, :width, :height
  def initialize(x=0, y=0, width=0, height=0)
    # RGSS's Rect#initialize writes fields directly (no overridable setters).
    @x=Integer(x)
    @y=Integer(y)
    @width=[Integer(width),0].max
    @height=[Integer(height),0].max
  end
  def set(x,y=nil,width=nil,height=nil)
    x,y,width,height = x.x,x.y,x.width,x.height if x.is_a?(Rect)
    # RGSS's Rect#set writes its fields directly and never calls overridden
    # x=/y=/width=/height= methods (WindowCursorRect relies on this).
    @x=Integer(x)
    @y=Integer(y)
    @width=[Integer(width),0].max
    @height=[Integer(height),0].max
    self
  end
  %i[x y].each do |key|
    class_eval "def #{key}=(value); @#{key} = Integer(value); end", __FILE__, __LINE__
  end
  %i[width height].each do |key|
    class_eval "def #{key}=(value); @#{key} = [Integer(value),0].max; end", __FILE__, __LINE__
  end
  def ==(other)
    other.is_a?(Rect) && x == other.x && y == other.y && width == other.width && height == other.height
  end
  alias === ==
  alias eql? ==
  def empty = set(0,0,0,0)
  def to_a = [x,y,width,height]
  def _dump(_depth) = to_a.pack('l<4')
  def self._load(bytes)
    raise TypeError, 'bad Rect payload' unless bytes.bytesize == 16
    new(*bytes.unpack('l<4'))
  end
end

