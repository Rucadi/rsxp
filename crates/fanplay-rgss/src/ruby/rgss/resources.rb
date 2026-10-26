module NativeResource
  attr_reader :__id
  def __own(id,op)
    @__id, @__dispose_op, @__disposed = Integer(id),op,false
    ObjectSpace.define_finalizer(self,FANPLAY.finalizer(op,@__id))
  end
  def disposed? = @__disposed
  def __check
    raise RGSSError, "disposed #{self.class}" if disposed?
  end
  def dispose
    return if disposed?
    FANPLAY.call(@__dispose_op,@__id)
    @__disposed = true
    ObjectSpace.undefine_finalizer(self)
    nil
  end
end

