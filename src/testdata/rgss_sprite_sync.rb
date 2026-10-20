# SPDX-License-Identifier: GPL-2.0-or-later
# Render identically through the typed desktop and serialized Ruby paths.
Graphics.resize_screen(64,48)
class GetterSprite < Sprite
  def x
    GC.start if @force_gc
    super.to_f + 2.75
  end
  def y = '7'
  def zoom_x = '1.25'
  def visible = :visible
end
bitmap=Bitmap.new(16,16)
16.times { |y| 16.times { |x| bitmap.set_pixel(x,y,Color.new(x*15,y*15,120,(x%3==0) ? 111 : 255)) } }
pattern=Bitmap.new(4,4)
pattern.fill_rect(pattern.rect,Color.new(170,40,90,130))
viewport=Viewport.new(3,2,55,40)
sprite=GetterSprite.new(viewport)
sprite.bitmap=bitmap
sprite.src_rect.set(2,1,10,12)
sprite.x='4'
sprite.z='7'
sprite.opacity=197
sprite.color.set(30.8,70.2,120.9,45.7)
sprite.tone.set(-12.8,20.5,4,60)
sprite.pattern=pattern
sprite.pattern_opacity='123.9'
sprite.pattern_scroll_x=2.8
sprite.pattern_zoom_x=1.5
raise 'flash did not return nil' unless sprite.flash(Color.new(240,180,80,220),3).nil?
# The game may use generic @flash_* variables of its own; flash state must not
# depend on them and a missing cached alpha should recover from the color.
sprite.instance_variable_set(:@__rsxp_flash_alpha,nil)
summaries=[]
3.times do |frame|
  # Finalizers can reenter the engine while a virtual getter runs.
  10.times { Sprite.new }
  sprite.instance_variable_set(:@force_gc,true)
  sprite.src_rect.x=frame+1
  viewport.ox=frame
  viewport.tone.gray=frame*35
  sprite.color.alpha+=4.8
  sprite.update
  sprite.angle=frame*13
  sprite.mirror=frame.odd?
  sprite.blend_type=frame
  Graphics.update
  snap=Graphics.snap_to_bitmap
  pixels=(0...48).flat_map { |y| (0...64).flat_map { |x| snap.get_pixel(x,y).to_a } }
  summaries << Zlib.crc32(pixels.pack('C*'))
  snap.dispose
end
# Negative zoom mirrors the sprite instead of failing. Render it and fold the
# result into the digest so both sync paths must agree.
sprite.zoom_y=-1
Graphics.update
snap=Graphics.snap_to_bitmap
pixels=(0...48).flat_map { |y| (0...64).flat_map { |x| snap.get_pixel(x,y).to_a } }
summaries << Zlib.crc32(pixels.pack('C*'))
snap.dispose
sprite.zoom_y=1
# Verify error behavior remains recoverable at the Ruby boundary.
sprite.blend_type='0'
begin
  sprite.__sync
  raise 'string blend accepted'
rescue NotImplementedError
end
sprite.blend_type=0
sprite.pattern_opacity=Float::INFINITY
begin
  sprite.__sync
  raise 'non-finite channel accepted'
rescue FloatDomainError
end
sprite.pattern_opacity='123.9'
pattern.dispose
begin
  sprite.__sync
  raise 'disposed pattern accepted'
rescue RGSSError
end
puts "SPRITE_SYNC:#{summaries.join(',')}"
