module NativeResource
  attr_reader :__id
  def __own(id,op)
    @__id, @__dispose_op, @__disposed = Integer(id),op,false
    if defined?(__sabi_own)
      __sabi_own(self,op,@__id)
    else
      ObjectSpace.define_finalizer(self,FANPLAY.finalizer(op,@__id))
    end
  end
  def disposed? = @__disposed
  def __check
    raise RGSSError, "disposed #{self.class}" if disposed?
  end
  def dispose
    return if disposed?
    FANPLAY.call(@__dispose_op,@__id)
    @__disposed = true
    if defined?(__sabi_disown)
      __sabi_disown(self)
    else
      ObjectSpace.undefine_finalizer(self)
    end
    nil
  end
end

