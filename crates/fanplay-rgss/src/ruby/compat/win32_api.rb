# The desktop binding defines these methods before the shim is loaded.
# ruby.wasm has no OS DLL loader; keep the same alias/fallback contract there.
unless defined?(Win32API)
  class Win32API
    def initialize(*)
      raise RuntimeError, 'native Win32API calls are unavailable on this platform'
    end
    def call(*)
      raise RuntimeError, 'uninitialized native Win32API'
    end
  end
end

class Win32API
	NATIVE_ON_WINDOWS = true unless const_defined?("NATIVE_ON_WINDOWS")
	TOLERATE_ERRORS = true unless const_defined?("TOLERATE_ERRORS")
	LOG_NATIVE = false unless const_defined?("LOG_NATIVE")

	alias_method :fanplay_native_initialize, :initialize
	def initialize(dll, func, *args)
		@dll = dll
		@func = func
		@called = false
		@fanplay_wrap_impl = nil

		dll = dll.sub(/\.(dll|klib)\z/i, "").downcase
		dll = "KGL2" if dll=="kgl2" || dll.end_with?("/kgl2") || dll.end_with?("\\kgl2")
		dll = kappatalize(dll)
		func = kappatalize(func.dup)

		if !System.is_windows? or !NATIVE_ON_WINDOWS
			if /\A[A-Z]\w*\z/.match?(dll) && Win32API_Impl.const_defined?(dll,false)
				dll_impl = Win32API_Impl.const_get(dll)
				if /\A[A-Z]\w*\z/.match?(func) && dll_impl.const_defined?(func,false)
					@fanplay_wrap_impl = dll_impl.const_get(func).new
					return
				end
			end
		end

		@fanplay_native_available = false
		begin
			fanplay_native_initialize(@dll, @func, *args)
			@fanplay_native_available = true
			return
		rescue
		end

	end

	alias_method :fanplay_native_call, :call
	def call(*args)
		if @fanplay_wrap_impl
			return @fanplay_wrap_impl.call(args)
		end

		if @fanplay_native_available
			if LOG_NATIVE
				System.puts("[Win32API] [#{@dll}:#{@func}] #{args.to_s}")
			end
			return fanplay_native_call(*args)
		end

		if TOLERATE_ERRORS
			System.puts("[Win32API] [#{@dll}:#{@func}] #{args.to_s}") if !@called
			@called = true
			return 0
		else
			raise RuntimeError, "[Win32API] [#{@dll}:#{@func}] #{args.to_s}"
		end
	end
	alias_method :Call, :call
end



