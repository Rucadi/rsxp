# SPDX-License-Identifier: GPL-2.0-or-later
# Cached/legacy and native/serialized paths must agree across lifecycle changes.
Graphics.resize_screen(24,20)
class MovingGetterSprite < Sprite
  def x = super + $sync_step
end
class MutableCoordinate
  attr_accessor :value
  def initialize(value) = @value=value
  def to_int = @value
end
$sync_step=0
bitmap=Bitmap.new(7,5)
bitmap.fill_rect(bitmap.rect,Color.new(50,130,210,180))
viewport=Viewport.new(1,2,20,17)
sprite=MovingGetterSprite.new(viewport)
sprite.bitmap=bitmap
sprite.x=3
sprite.pattern_scroll_x='1'
coordinate=MutableCoordinate.new(2)
sprite.y=coordinate
sprite.zoom_y='1.0'
disposed=Sprite.new
disposed.dispose
def forgotten_sprite
  WeakRef.new(Sprite.new)
end
forgotten=forgotten_sprite
GC.start
raise 'registry retained an unreachable sprite' if forgotten.weakref_alive?
checksums=[]
8.times do |frame|
  sprite.pattern_scroll_x.replace('2') if frame==1
  sprite.zoom_y.replace('1.5') if frame==1
  coordinate.value=4 if frame==1
  sprite.src_rect.x=1 if frame==2
  sprite.color.alpha=70 if frame==2
  viewport.visible=false if frame==3
  sprite.y=5 if frame==4
  viewport.visible=true if frame==5
  viewport.rect.width=12 if frame==6
  $sync_step+=1 if frame>=5
  Graphics.update
  snapshot=Graphics.snap_to_bitmap
  pixels=(0...20).flat_map { |y| (0...24).flat_map { |x| snapshot.get_pixel(x,y).to_a } }
  checksums << Zlib.crc32(pixels.pack('C*'))
  snapshot.dispose
end
raise 'disposed registry entries were not pruned' unless Sprite.instances.size==1
# Unsupported properties must still raise on the serialized path while hidden.
# Desktop's existing hidden-sprite fast path intentionally omits geometry.
if ENV['FANPLAY_SPRITE_SYNC']=='ruby' || !Sprite::NATIVE_SYNC
  sprite.visible=false
  Graphics.update
  sprite.blend_type=3
  begin
    sprite.__sync
    raise 'unsupported blend on hidden sprite accepted'
  rescue NotImplementedError
  end
  sprite.blend_type=0
end
puts "SYNC_LIFECYCLE:#{checksums.join(',')}"
