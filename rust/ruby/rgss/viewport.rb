class Viewport
  include NativeResource
  include RSXP::Flashable
  @instances = []
  class << self
    attr_reader :instances
    def __sync_all
      Viewport.instances.delete_if do |reference|
        begin
          viewport = reference.__getobj__
        rescue RSXP::ResourceRef::RefError
          next true
        end
        viewport.__sync unless viewport.disposed?
        viewport.disposed?
      end
    end
  end
  attr_accessor :z,:ox,:oy,:visible
  attr_reader :rect,:color,:tone
  def initialize(*args)
    __own(RSXP.call('viewport_new'),'viewport_dispose')
    @rect=args.empty? ? Rect.new(0,0,Graphics.width,Graphics.height) : (args.size==1 ? args.first.clone : Rect.new(*args))
    @z=@ox=@oy=0
    @visible=true
    @color=Color.new(0,0,0,0)
    @tone=Tone.new
    Viewport.instance_variable_get(:@instances) << RSXP::ResourceRef.new(self)
  end
  def rect=(value)
    @rect.set(value)
  end
  def color=(value)
    @color.set(value)
  end
  def tone=(value)
    @tone.set(value)
  end
  def __sync
    __check
    sync_color=__flash_color_for_sync(@color)
    RSXP.sync(self,'viewport_sync',*@rect.to_a,Integer(@ox),Integer(@oy),visible ? 1 : 0,Integer(@z),
      *sync_color.to_a,*[@tone.red,@tone.green,@tone.blue,@tone.gray].map(&:to_i))
  end
end

