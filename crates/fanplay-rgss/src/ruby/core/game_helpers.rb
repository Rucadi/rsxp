# Some game helpers are defined or made private after Main starts, beyond the
# script loader's publication pass. Keep legacy Kernel.helper calls working
# only for methods defined directly on Object, leaving Kernel's own private
# methods and unrelated missing methods subject to normal Ruby rules.
module FANPLAY::KernelGameHelpers
  private
  def method_missing(name, *args, **kwargs, &block)
    if Object.private_instance_methods(false).include?(name)
      return TOPLEVEL_BINDING.eval('self').__send__(name, *args, **kwargs, &block)
    end
    super
  end
  def respond_to_missing?(name, include_private = false)
    Object.private_instance_methods(false).include?(name) || super
  end
end
Kernel.singleton_class.prepend(FANPLAY::KernelGameHelpers)

