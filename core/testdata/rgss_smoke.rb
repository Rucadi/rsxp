# SPDX-License-Identifier: GPL-2.0-or-later
require 'tmpdir'
def check(value, message)
  raise message unless value
end
check((1..3).cover?(RGSS_VERSION), 'missing RGSS version')
check(RPG::Actor.new.is_a?(RPG::Actor), 'RPG data classes not loaded')
table=Table.new(2,2)
table[1,1]=-1234
table[0,0]=65535
check(table[0,0]==-1, 'Table value is not signed 16-bit')
check(table[-1,0].nil?, 'out-of-range Table reads must return nil')
table.resize(3,3,2)
check(table[1,1,0]==-1234 && table[2,2,1]==0, 'Table resize lost data')
copy=Marshal.load(Marshal.dump(table))
cloned=table.clone
table[1,1,0]=42
GC.start
check(copy[1,1,0]==-1234 && cloned[1,1,0]==-1234, 'Table copies share storage')
100.times { temp=Table.new(3); temp[1]=12; check(Marshal.load(Marshal.dump(temp))[1]==12, 'Table GC roundtrip') }
Table.allocate.resize(2)
begin
  Table._load('invalid')
  raise 'bad Table payload was accepted'
rescue ArgumentError
end
[Color.new(12,34,56,78),Tone.new(-12,34,-56,78),Rect.new(-3,4,5,6)].each do |object|
  check(Marshal.load(Marshal.dump(object))._dump(0)==object._dump(0), 'RGSS value Marshal mismatch')
end
# RGSS compares values, allowing the map renderer to skip unchanged tint
# updates across clones. Channel changes must still invalidate that comparison.
[Color.new(12.25,34,56,78), Tone.new(-12.25,34,-56,78), Rect.new(-3,4,5,6)].each do |value|
  value_copy=value.clone
  check(value == value_copy && value.eql?(value_copy) && value === value_copy, 'RGSS value equality')
  check(!(value == nil) && !(value == Object.new), 'RGSS value type equality')
  if value.is_a?(Rect)
    value_copy.x+=1
  else
    value_copy.red+=0.25
  end
  check(value != value_copy, 'RGSS value equality missed mutation')
end
color=Color.new('-1',Float::INFINITY,12.75,-Float::INFINITY)
check(color.to_a == [0,255,12,0], 'Color clamp or truncation changed')
tone=Tone.new('-300',300,12.75,300)
check([tone.red,tone.green,tone.blue,tone.gray] == [-255.0,255.0,12.75,255.0], 'Tone clamp changed')
begin
  color.red=Float::NAN
  raise 'NaN Color accepted'
rescue ArgumentError
end
Dir.mktmpdir('rsxp-ruby') do |dir|
  path=File.join(dir,'data.rxdata')
  save_data([copy,RPG::Actor.new],path)
  check(load_data(path)[0][1,1,0]==-1234,'load_data/save_data failed')
end
Graphics.resize_screen(64,48)
check(Graphics.width==64 && Graphics.height==48,'screen size')
Graphics.frame_rate=60
check(Graphics.frame_rate==60,'frame rate')
bitmap=Bitmap.new(16,16)
bitmap.fill_rect(bitmap.rect,Color.new(255,0,0))
bitmap.set_pixel(1,1,Color.new(0,255,0))
duplicate=bitmap.clone
bitmap.set_pixel(1,1,Color.new(255,0,0))
check(duplicate.get_pixel(1,1).green==255,'Bitmap clone shares pixels')
duplicate.dispose
discarded=bitmap.clone
discarded=nil
GC.start
bitmap.set_pixel(1,1,Color.new(255,0,0))
begin
  duplicate.clear
  raise 'disposed bitmap allowed mutation'
rescue RGSSError
end
sprite=Sprite.new
sprite.bitmap=bitmap
# Pokémon Essentials reassigns the same walking charset on every update.
# Preserve the selected animation cell until a different bitmap is assigned.
sprite.src_rect.set(4,4,4,4)
sprite.bitmap=bitmap
check(sprite.src_rect.to_a==[4,4,4,4], 'same bitmap reset animation crop')
sprite.x=4
sprite.y=5
Graphics.update
cropped=Graphics.snap_to_bitmap
check(cropped.get_pixel(7,8).red==255, 'animation cell missing')
check(cropped.get_pixel(8,9).red==0, 'whole charset rendered instead of cell')
cropped.dispose
replacement=bitmap.clone
sprite.bitmap=replacement
check(sprite.src_rect.to_a==[0,0,16,16], 'new bitmap did not reset crop')
sprite.bitmap=bitmap
replacement.dispose
Graphics.frame_count=0
sprite.x=4
sprite.y=5
Graphics.update
Input.update
check(!Input.press?(:C) && !Input.trigger?(Input::C) && !Input.repeat?(:C),'headless input')
check(Input.dir4==0 && Input.dir8==0,'input direction')
snapshot=Graphics.snap_to_bitmap
check(snapshot.get_pixel(4,5).red==255,'sprite not rendered by Rust')
check(snapshot.get_pixel(0,0).red==0,'background not cleared')
snapshot.dispose
Graphics.wait(2)
check(Graphics.frame_count==3,'frame count')
puts "RGSS_OK:#{RGSS_VERSION}"
