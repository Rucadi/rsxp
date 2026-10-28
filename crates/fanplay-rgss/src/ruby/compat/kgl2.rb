# KGL2 is a Windows graphics DLL some Pokémon fangames call through Win32API
# for light blending and cast shadows. This module implements its functions on
# top of the Bitmap API so those games work on every platform, mirroring
# mkxp-z's scripts/preload/kgl2_wrap.rb. Arguments arrive as an array because
# that is how fanplay's Win32API wrapper invokes portable implementations.
module KGL2_Impl
	begin
		require 'objspace'
	rescue LoadError
	end

	class << self
		attr_accessor :initialized, :light_blending, :soft_shadows, :framebuffer, :shadowbuffer
	end
	self.initialized=false
	self.light_blending=false
	self.soft_shadows=false
	self.framebuffer=nil
	self.shadowbuffer=nil

	def self.bitmap(id)
		ObjectSpace._id2ref(Integer(id))
	end

	class KglVersion
		def call(_args) = 200
	end

	class KglLoad
		def call(_args)
			return 102 if KGL2_Impl.initialized
			KGL2_Impl.initialized=true
			1
		end
	end

	class KglBlank
		def call(args)
			KGL2_Impl.bitmap(args[0]).clear
			1
		end
	end

	class KglClear
		def call(args)
			packed=Integer(args[1])
			bitmap=KGL2_Impl.bitmap(args[0])
			bitmap.fill_rect(bitmap.rect,Color.new((packed>>16)&0xff,(packed>>8)&0xff,packed&0xff,(packed>>24)&0xff))
			1
		end
	end

	class KglInvert
		def call(args)
			KGL2_Impl.bitmap(args[0])._kgl_invert
			1
		end
	end

	class KglClone
		def call(args)
			dst=KGL2_Impl.bitmap(args[0])
			src=KGL2_Impl.bitmap(args[1])
			rect=dst.rect
			return 112 if rect!=src.rect
			dst.clear
			dst.blt(0,0,src,rect)
			1
		end
	end

	class KglBindFramebuffer
		def call(args)
			return 101 unless KGL2_Impl.initialized
			KGL2_Impl.framebuffer=KGL2_Impl.bitmap(args[0])
			1
		end
	end

	class KglBindShadowbuffer
		def call(args)
			return 101 unless KGL2_Impl.initialized
			KGL2_Impl.shadowbuffer=KGL2_Impl.bitmap(args[0])
			1
		end
	end

	class KglUnbindFramebuffer
		def call(_args)
			KGL2_Impl.framebuffer=nil
			1
		end
	end

	class KglUnbindShadowbuffer
		def call(_args)
			KGL2_Impl.shadowbuffer=nil
			1
		end
	end

	class KglClearFramebuffer
		def call(_args)
			return 103 if KGL2_Impl.framebuffer.nil?
			KGL2_Impl.framebuffer.clear
			1
		end
	end

	class KglCompressAlpha
		def call(args)
			KGL2_Impl.bitmap(args[0])._kgl_compress_alpha
			1
		end
	end

	class KglLightBlending
		def call(args)
			KGL2_Impl.light_blending=!(args[0].nil? || args[0]==0)
			1
		end
	end

	class KglLightShader
		def call(args)
			return 103 if KGL2_Impl.framebuffer.nil?
			bitmap=KGL2_Impl.bitmap(args[0])
			x=Integer(args[1]); y=Integer(args[2]); opacity=Integer(args[3])
			framebuffer_width=KGL2_Impl.framebuffer.width
			framebuffer_height=KGL2_Impl.framebuffer.height
			bitmap_width=bitmap.width
			bitmap_height=bitmap.height
			framebuffer_width-=[x,0].min
			framebuffer_height-=[y,0].min
			bitmap_width+=[x,0].max
			bitmap_height+=[y,0].max
			width=[framebuffer_width,bitmap_width].min-x.abs
			height=[framebuffer_height,bitmap_height].min-y.abs
			return 111 if width<0 || height<0
			opacity=opacity>100 ? 255 : 0 unless KGL2_Impl.light_blending
			KGL2_Impl.framebuffer._kgl_subtract_rect(
				[x,0].max,
				[y,0].max,
				bitmap,
				Rect.new(-[x,0].min,-[y,0].min,width,height),
				opacity)
			1
		end
	end

	class KglSoftShadows
		def call(args)
			KGL2_Impl.soft_shadows=!(args[0].nil? || args[0]==0)
			1
		end
	end

	class KglShadowShaderH
		def call(args)
			return 105 if KGL2_Impl.shadowbuffer.nil?
			KGL2_Impl.shadowbuffer._kgl_shadow_shader_h(Integer(args[0]),Integer(args[1]),Integer(args[2]),KGL2_Impl.soft_shadows)
		end
	end

	class KglShadowShaderV
		def call(args)
			return 105 if KGL2_Impl.shadowbuffer.nil?
			KGL2_Impl.shadowbuffer._kgl_shadow_shader_v(Integer(args[0]),Integer(args[1]),Integer(args[2]),false,KGL2_Impl.soft_shadows)
		end
	end

	class KglShadowShaderW
		def call(args)
			return 105 if KGL2_Impl.shadowbuffer.nil?
			KGL2_Impl.shadowbuffer._kgl_shadow_shader_v(Integer(args[0]),Integer(args[1]),Integer(args[2]),true,KGL2_Impl.soft_shadows)
		end
	end
end

