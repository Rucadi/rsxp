# SPDX-License-Identifier: GPL-2.0-or-later
# Functional smoke test for the KGL2 compatibility layer (KGL2_Impl), ported
# from the corresponding cases in mkxp-z's tests/kgl2 suite.
def kgl(func, proto = '', ret = 'i')
  Win32API.new('KGL2.klib', func, proto, ret)
end
def check(value, message)
  raise "KGL2: #{message}" unless value
end
def bytes(*values)
  values.pack('C*')
end

check(kgl('kglVersion').call == 200, 'version should be 200')
check(kgl('kglLoad').call == 1, 'first load should initialize')
check(kgl('kglLoad').call == 102, 'second load should report already initialized')

# KglClear packs the color as ARGB.
bitmap = Bitmap.new(1, 1)
bitmap.fill_rect(bitmap.rect, Color.new(0xde, 0xad, 0xbe, 0xef))
check(kgl('kglClear', 'ii').call(bitmap.object_id, 0x4d1a2b3c) == 1, 'clear status')
check(bitmap.raw_data == bytes(0x1a, 0x2b, 0x3c, 0x4d), 'clear color')

# KglInvert complements RGB and leaves alpha unchanged.
bitmap = Bitmap.new(1, 1)
bitmap.set_pixel(0, 0, Color.new(0x03, 0x02, 0x01, 0xff))
check(kgl('kglInvert', 'i').call(bitmap.object_id) == 1, 'invert status')
check(bitmap.raw_data == bytes(0xfc, 0xfd, 0xfe, 0xff), 'invert result')

# KglCompressAlpha multiplies RGB by alpha/255 and clears alpha.
bitmap = Bitmap.new(1, 1)
bitmap.fill_rect(bitmap.rect, Color.new(0x12, 0x34, 0x56, 0xff))
check(kgl('kglCompressAlpha', 'i').call(bitmap.object_id) == 1, 'compress status')
check(bitmap.raw_data == bytes(0x12, 0x34, 0x56, 0x00), 'compress opaque result')

# KglLightShader subtracts the source from the bound framebuffer and makes it opaque.
dst = Bitmap.new(1, 1)
dst.fill_rect(dst.rect, Color.new(0x03, 0x02, 0x01, 0xff))
src = Bitmap.new(1, 1)
src.fill_rect(src.rect, Color.new(0x01, 0x02, 0x03, 0xff))
check(kgl('kglBindFramebuffer', 'i').call(dst.object_id) == 1, 'bind framebuffer')
check(kgl('kglLightBlending', 'i').call(1) == 1, 'light blending status')
check(kgl('kglLightShader', 'iiii').call(src.object_id, 0, 0, 255) == 1, 'light shader status')
check(dst.raw_data == bytes(0x02, 0x00, 0x00, 0xff), 'light subtract result')

# KglClone copies between equal-sized bitmaps and rejects size mismatches.
dst = Bitmap.new(2, 1)
src = Bitmap.new(2, 1)
src.set_pixel(0, 0, Color.new(10, 20, 30, 255))
src.set_pixel(1, 0, Color.new(40, 50, 60, 255))
check(kgl('kglClone', 'ii').call(dst.object_id, src.object_id) == 1, 'clone status')
check(dst.raw_data == src.raw_data, 'clone data')
check(kgl('kglClone', 'ii').call(dst.object_id, Bitmap.new(1, 1).object_id) == 112, 'clone size check')

# KglBlank zeroes a bitmap.
check(kgl('kglBlank', 'i').call(dst.object_id) == 1, 'blank status')
check(dst.raw_data == bytes(0, 0, 0, 0, 0, 0, 0, 0), 'blank data')

# Shadow shaders require a bound shadowbuffer and validate their coordinates.
shadow = Bitmap.new(5, 5)
shadow.fill_rect(shadow.rect, Color.new(255, 255, 255, 255))
check(kgl('kglBindShadowbuffer', 'i').call(shadow.object_id) == 1, 'bind shadowbuffer')
check(kgl('kglShadowShaderH', 'iii').call(0, 4, 2) == 111, 'shadow y center rejected')
check(kgl('kglShadowShaderH', 'iii').call(4, 4, 4) == 1, 'shadow cast status')
check(shadow.raw_data[(4 * 5 + 4) * 4, 4] == bytes(0, 0, 0, 0), 'shadow zeroed pixel')
check(kgl('kglUnbindShadowbuffer').call == 1, 'unbind shadowbuffer')
check(kgl('kglShadowShaderH', 'iii').call(4, 4, 4) == 105, 'shadow unbound rejected')

puts 'KGL2_OK'
