# SPDX-License-Identifier: GPL-2.0-or-later
# Ordinary sprites, mutable inputs, late getter overrides, dependencies and GC.
Graphics.resize_screen(32,24)
bitmap=Bitmap.new(8,8)
bitmap.fill_rect(bitmap.rect,Color.new(90,180,30))
sprite=Sprite.new
sprite.bitmap=bitmap
viewport=Viewport.new(Rect.new(1,1,28,20).freeze)
viewport.dispose
viewport=Viewport.new(1,1,28,20)
Graphics.update
if ENV['FANPLAY_NATIVE_STATE']=='1'
  before=FANPLAY::Retained.stats[:synced]
  sprite.color.set(sprite.color)
  sprite.tone.set(sprite.tone)
  sprite.src_rect.set(sprite.src_rect)
  viewport.color.set(viewport.color)
  viewport.tone.set(viewport.tone)
  viewport.rect.set(viewport.rect)
  Graphics.update
  raise 'idle native resources were polled' unless FANPLAY::Retained.stats[:synced]==before
end
checks=[]
18.times do |frame|
  case frame
  when 1 then Thread.new { sprite.x=4 }.join
  when 2 then sprite.color.alpha=80
  when 3 then sprite.tone.gray=60
  when 4 then sprite.src_rect.width=4
  when 5 then sprite.viewport=viewport
  when 6 then viewport.rect.width=7
  when 7 then viewport.visible=false
  when 8 then viewport.visible=true;viewport.ox=2
  when 9 then sprite.flash(Color.new(200,20,60,180),3)
  when 10 then sprite.instance_variable_set(:@x,8)
  when 11
    copy=sprite.color.clone;copy.red=200
    raise 'clone shared storage' if sprite.color.red==200
    raise 'Marshal changed' unless Marshal.load(Marshal.dump(copy))==copy
  when 12 then sprite.x='9'
  when 13 then sprite.x.replace('10')
  when 14 then def sprite.y = 3
  when 15 then viewport.dispose
  when 16 then sprite.viewport=nil
  when 17 then bitmap.dispose
  end
  sprite.update
  GC.start if frame==8
  Graphics.update
  snap=Graphics.snap_to_bitmap
  bytes=(0...24).flat_map { |y| (0...32).map { |x| snap.get_pixel(x,y).to_a.pack('C*') } }.join
  checks << Zlib.crc32(bytes)
  snap.dispose
end
sprite.dispose
puts "RETAINED_STATE:#{checks.join(',')}"

# RPG Maker plugins commonly alias the existing method before replacing it.
# Native observers must remain safe under this pattern (no prepend recursion).
class Sprite
  alias __retained_test_old_x= x=
  def x=(value)
    self.__retained_test_old_x=value
  end
  alias __retained_test_update update
  def update
    __retained_test_update
  end
  alias __retained_test_dispose dispose
  def dispose
    __retained_test_dispose
  end
end
aliased=Sprite.new
aliased.x=2
aliased.update
aliased.dispose

allocated=Sprite.allocate
allocated.send(:initialize)
allocated.bitmap=Bitmap.new(1,1)
allocated.bitmap.set_pixel(0,0,Color.new(255,0,0))
Graphics.update
raise 'allocated sprite was not tracked' unless Graphics.snap_to_bitmap.get_pixel(0,0).red==255
allocated.bitmap.dispose
allocated.dispose

# Essentials proxy planes inherit Plane but own only an internal Sprite. They
# skip Plane#initialize and mark themselves disposed before calling super.
class RetainedProxyPlane < Plane
  def initialize
    @sprite=Sprite.new
    @__disposed=false
  end
  def dispose
    unless @__disposed
      @sprite.dispose
      @sprite=nil
      @__disposed=true
    end
    super
  end
end
proxy=RetainedProxyPlane.new
raise 'proxy unexpectedly owns a native plane' unless proxy.__id.nil?
2.times { proxy.dispose }
raise 'proxy disposal failed' unless proxy.disposed?
Graphics.update
