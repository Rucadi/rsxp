class Plane
  include NativeResource
  @instances = []
  class << self
    attr_reader :instances
    def __sync_all
      Plane.instances.delete_if do |reference|
        begin
          plane = reference.__getobj__
        rescue RSXP::ResourceRef::RefError
          next true
        end
        plane.__sync unless plane.disposed?
        plane.disposed?
      end
    end
  end
  attr_accessor :viewport,:visible,:z,:ox,:oy,:zoom_x,:zoom_y,:blend_type
  attr_reader :bitmap,:opacity,:color,:tone
  def initialize(viewport=nil)
    raise TypeError, 'expected Viewport or nil' unless viewport.nil? || viewport.is_a?(Viewport)
    __own(RSXP.call('plane_new'),'plane_dispose')
    @viewport=viewport
    @bitmap=nil
    @visible=true
    @z=@ox=@oy=0
    @zoom_x=@zoom_y=1.0
    @opacity=255
    @blend_type=0
    @color=Color.new(0,0,0,0)
    @tone=Tone.new
    Plane.instance_variable_get(:@instances) << RSXP::ResourceRef.new(self)
  end
  def bitmap=(value)
    __check
    raise TypeError, 'expected Bitmap or nil' unless value.nil? || value.is_a?(Bitmap)
    value.__check if value
    @bitmap=value
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
  def __sync
    __check
    unless [0,1,2].include?(blend_type)
      raise NotImplementedError, "plane blend type #{blend_type} is not supported"
    end
    shown=visible && bitmap && !bitmap.disposed?
    clip=Rect.new(0,0,Graphics.width,Graphics.height)
    if viewport
      shown &&= viewport.visible && !viewport.disposed?
      clip=viewport.rect
    end
    RSXP.sync(self,'plane_sync',bitmap ? bitmap.__id : 0,Integer(ox),Integer(oy),
      Float(zoom_x),Float(zoom_y),shown ? opacity : 0,shown ? 1 : 0,Integer(z),Integer(blend_type),
      *@color.to_a,*[@tone.red,@tone.green,@tone.blue,@tone.gray].map(&:to_i),*clip.to_a,
      viewport ? viewport.__id : 0)
  end
end

