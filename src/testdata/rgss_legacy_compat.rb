# Standard RGSS APIs used by older Essentials games, on desktop and ruby.wasm.
def assert(value, message)
  raise message unless value
end
assert($MKXP, 'engine detection')
assert(nil.id==nil.__id__,'Ruby 1.8 Object#id') if RGSS_VERSION==1
assert(!Thread.critical,'initial critical state')
Thread.critical=true
assert(Thread.critical,'critical section')
Thread.critical=true
Thread.critical=false
assert(!Thread.critical,'critical section release')
# The byte reader used by older Essentials must feed integers to its dex and
# experience parsers even though modern String#getc-style indexing returns text.
module FileInputMixin
  def fgetb
    each_byte { |byte| return byte }
    0
  end
  def fgetw
    low=fgetb
    high=fgetb
    low | (high << 8)
  end
end
class StringInput
  include FileInputMixin
  def initialize(bytes)
    @bytes=bytes
    @position=0
  end
  def getc
    byte=@bytes[@position]
    @position+=1
    byte
  end
  def each_byte
    yield getc while @position<@bytes.bytesize
  end
end
MKXP.publish_top_level_methods
reader=StringInput.new("\x05\x34\x12".b)
assert(reader.fgetb==5 && reader.fgetw==0x1234 && reader.getc.nil?,'legacy dex-data byte reader')
MKXP.publish_top_level_methods
assert(StringInput.ancestors.count(MKXP::LegacyByteReader)==1,'byte reader installed twice')
# A return from a registered map-change callback must not leave the engine's
# script evaluation. Preserve subsequent callbacks and loose argument counts.
class CompatibilityEvent
  attr_reader :callbacks
  def initialize = @callbacks=[]
  def +(callback)
    @callbacks << callback
    self
  end
  def trigger(sender,*args)
    @callbacks.each { |callback| callback.call(sender,args) }
  end
end
module Events
  class << self
    attr_accessor :onMapChange
  end
end
Events.onMapChange=CompatibilityEvent.new
MKXP.eval_script(<<~'SOURCE','legacy event returns')
  $callback_results=[]
  Events.onMapChange+=proc {|sender,args|
    [1,2].each do |i|
      return :skipped if args && args[0]==2
    end
    $callback_results << [sender,args]
  }
  Events.onMapChange+=proc {|sender,args|
    $callback_results << :continued
  }
SOURCE
Events.onMapChange.trigger(:map,2)
assert($callback_results==[:continued],'map callback return escaped or kept running')
$callback_results.clear
Events.onMapChange.trigger(:map,3)
assert($callback_results==[[:map,[3]],:continued],'map callback arguments changed')
$callback_results.clear
Events.onMapChange.callbacks.first.call
Events.onMapChange.callbacks.first.call(:map,[4],:unused)
assert($callback_results==[[nil,nil],[:map,[4]]],'legacy callback arity became strict')
Events.onMapChange=CompatibilityEvent.new
MKXP.eval_script(<<~'SOURCE','legacy callback argument shapes')
  Events.onMapChange+=proc { return :ignored }
  Events.onMapChange+=proc do |sender|
    $callback_results << sender
  end
SOURCE
$callback_results.clear
Events.onMapChange.trigger(:map,2)
assert($callback_results==[:map],'zero/one argument callbacks did not preserve local returns')
dll='USER32.DLL'.freeze
function='FindWindowA'.freeze
window=Win32API.new(dll,function,'pp','l')
assert(window.Call('RGSS Player',nil)==42, 'FindWindowA / Call')
assert(dll=='USER32.DLL' && function=='FindWindowA','mutated API names')
assert(Win32API.new('user32','FindWindowEx','llpp','l').call(0,0,'RGSS Player',0)==42,'FindWindowEx')
assert(Win32API.new('user32','FindWindowEx','llpp','l').call(0,42,'RGSS Player',0)==0,'window enumeration termination')
thread=Win32API.new('kernel32','GetCurrentThreadId','','l').call
process="\0".b*4
assert(Win32API.new('user32','GetWindowThreadProcessId','lp','l').call(42,process)==thread,'window thread identity')
assert(process.unpack1('L')==Process.pid,'process output buffer')
rect="\0".b*16
assert(Win32API.new('user32','GetClientRect','lp','i').call(42,rect)==1,'client rectangle')
assert(rect.unpack('l4')==[0,0,Graphics.width,Graphics.height],'rectangle output buffer')
assert(Win32API.new('user32','GetClientRect','lp','i').call(0,rect)==0,'invalid window')
assert(Win32API.new('user32','GetSystemMetrics','i','i').call(0)==Graphics.width,'screen metrics')
if Sprite::NATIVE_SYNC
  native=Win32API.allocate
  begin
    native.send(:rsxp_native_initialize,'kernel32','GetCurrentProcessId','f','l')
    raise 'unsupported native signature accepted'
  rescue ArgumentError
  end
end
Graphics.update
Input.update
assert(!Input.press?(nil) && !Input.trigger?(nil) && !Input.repeat?(nil) && !Input.release?(nil),'disabled input binding')
keyboard="\0".b*256
assert(Win32API.new('user32','GetKeyboardState','p','i').call(keyboard)==1,'keyboard snapshot')
expected=Input.pressex?(:A) ? 0x80 : 0
assert(keyboard.getbyte(0x41)==expected,'virtual key mapping')
assert(Win32API.new('user32','GetAsyncKeyState','i','i').call(0x41)==(expected==0 ? 0 : 0x8000),'async key state')
text="payload\0".b*20
assert(MKXP.zinflate(MKXP.zdeflate(text))==text,'compression helpers')
# Games replace these class helpers with delegates back to MKXP.
class << Zlib::Inflate
  alias __compat_inflate inflate
  def inflate(data) = MKXP.zinflate(data)
end
class << Zlib::Deflate
  alias __compat_deflate deflate
  def deflate(data,*args) = MKXP.zdeflate(data,*args)
end
assert(Zlib::Inflate.inflate(Zlib::Deflate.deflate(text))==text,'recursive zlib delegation')
class << Zlib::Inflate
  alias inflate __compat_inflate
end
class << Zlib::Deflate
  alias deflate __compat_deflate
end
# Retry in a rescue clause is left alone; the obsolete block form is repaired.
legacy="count=0\nloop do\n count+=1\n retry if count<2\n break\nend\nraise 'retry translation' unless count==2\n"
MKXP.eval_script(legacy,'legacy retry')
MKXP.eval_script("retries=0; begin; retries+=1; raise 'again' if retries<2; rescue; retry; end",'valid retry')
MKXP.eval_script('regexp=/"[^"\\x00-\\x1f\\x7f-\\x9f]*"/','legacy byte range')
MKXP.eval_script("case 2\nwhen 1, 2:\n true\nelse\n raise 'numeric when'\nend",'legacy when')
MKXP.eval_script("text=<<~TEXT\nwhen 2:\nTEXT\nraise 'changed literal' unless text.include?('when 2:')",'literal syntax')
MKXP.eval_script("data='A'; raise 'byte comparison' unless data[0]==0x41",'legacy byte indexing')
MKXP.eval_script("class String\n alias getbyte []\n alias bytesize size\nend",'legacy byte polyfills')
assert('A'.getbyte(0)==65 && "é".bytesize==2,'native byte methods overwritten')
Graphics.resize_screen(64,64)
bitmap=Bitmap.new(32,32)
bitmap.fill_rect(bitmap.rect,Color.new(255,0,0))
sprite=Sprite.new
sprite.bitmap=bitmap
sprite.bush_depth=16
assert(sprite.bush_opacity==128,'default bush opacity')
Graphics.update
frame=Graphics.snap_to_bitmap
assert(frame.get_pixel(0,0).red==255,'ordinary sprite pixels')
assert((frame.get_pixel(0,31).red-128).abs<=1,'bush transparency')
frame.dispose
sprite.bush_opacity=0
Graphics.update
frame=Graphics.snap_to_bitmap
assert(frame.get_pixel(0,31).red==0,'bush snapshot invalidation')
frame.dispose
sprite.dispose
# Inherit the base class, as SynchronizedTilemap does in Essentials.
class CompatibleTilemap < Tilemap; end
map=CompatibleTilemap.new
map.tileset=bitmap
map.map_data=Table.new(2,2,1)
map.map_data[0,0,0]=384
map.priorities=Table.new(385)
Graphics.update
frame=Graphics.snap_to_bitmap
assert(frame.get_pixel(1,1).red==255,'base tilemap rendering')
frame.dispose
map.ox=32
Graphics.update
frame=Graphics.snap_to_bitmap
assert(frame.get_pixel(1,1).red==0,'tilemap scrolling and hidden cells')
frame.dispose
map.ox=0
map.visible=false
Graphics.update
frame=Graphics.snap_to_bitmap
assert(frame.get_pixel(1,1).red==0,'hidden tilemap')
frame.dispose
map.visible=true
map.priorities[384]=1
overlay=Sprite.new
overlay.bitmap=Bitmap.new(32,32)
overlay.bitmap.fill_rect(overlay.bitmap.rect,Color.new(0,255,0))
overlay.z=32
Graphics.update
frame=Graphics.snap_to_bitmap
assert(frame.get_pixel(1,1).red==255,'priority tile ordering')
frame.dispose
map.priorities[384]=0
Graphics.update
frame=Graphics.snap_to_bitmap
assert(frame.get_pixel(1,1).green==255,'ground tile ordering')
frame.dispose
overlay.bitmap.dispose
overlay.dispose
autotile=Bitmap.new(64,32)
autotile.fill_rect(0,0,32,32,Color.new(0,255,0))
autotile.fill_rect(32,0,32,32,Color.new(0,0,255))
map.autotiles[0]=autotile
map.map_data[0,0,0]=48
Graphics.update
frame=Graphics.snap_to_bitmap
assert(frame.get_pixel(1,1).green==255,'autotile rendering')
frame.dispose
16.times { map.update }
Graphics.update
frame=Graphics.snap_to_bitmap
assert(frame.get_pixel(1,1).blue==255,'autotile animation')
frame.dispose
quarters=Bitmap.new(96,128)
quarters.fill_rect(32,64,16,16,Color.new(255,0,0))
quarters.fill_rect(48,64,16,16,Color.new(0,255,0))
quarters.fill_rect(32,80,16,16,Color.new(0,0,255))
quarters.fill_rect(48,80,16,16,Color.new(255,255,0))
map.autotiles[0]=quarters
Graphics.update
frame=Graphics.snap_to_bitmap
assert(frame.get_pixel(1,1).red==255 && frame.get_pixel(20,1).green==255 &&
       frame.get_pixel(1,20).blue==255 && frame.get_pixel(20,20).red==255,'autotile quadrant assembly')
frame.dispose
map.dispose
assert(map.disposed?,'tilemap disposal')
assert(!bitmap.disposed?,'tilemap disposed an externally owned bitmap')
bitmap.dispose
autotile.dispose
quarters.dispose
# A game-provided WeakRef replacement cannot change engine resource tracking.
MKXP.eval_script("class WeakRef\n @@id_rev_map={}\n def initialize(*)\n end\n def __getobj__\n nil\n end\nend",'legacy WeakRef base')
reference=WeakRef.new(Object.new)
assert(!(!reference) && reference.__getobj__.nil?,'legacy WeakRef inherited Delegator behavior')
class WeakRef
  def initialize(*) = raise('game WeakRef should not be used by the engine')
end
sprite=Sprite.new
Graphics.update
sprite.dispose
puts 'LEGACY_COMPAT_OK'
