# SPDX-License-Identifier: GPL-2.0-or-later
# Ruby-facing RGSS API. Bitmaps, rendering, input and Table storage live in Rust.
require 'zlib'
require 'weakref'
require 'fileutils'
require 'json'
$RSXP = true
$MKXP = $RSXP # Compatibility flag used by older game scripts.
# Older RGSS libraries still use Thread.critical around shared caches. Retain
# their nested critical-section convention with a mutex on modern Ruby.
class Thread
  @__rgss_critical_mutex=Mutex.new
  class << self
    def critical = !!Thread.current[:__rgss_critical]
    def critical=(value)
      value=!!value
      return value if critical==value
      @__rgss_critical_mutex.try_lock || @__rgss_critical_mutex.lock if value
      Thread.current[:__rgss_critical]=value
      @__rgss_critical_mutex.unlock unless value
      value
    end
  end
end
module HTTPLite
  # rsxp exposes HTTPLite::JSON with stringify/parse and ParserError.
  module JSON
    ParserError = ::JSON::ParserError
    def self.parse(string) = ::JSON.parse(string)
    def self.stringify(object) = ::JSON.generate(object)
  end
  # Network access is not ported yet; fail explicitly so callers can rescue.
  def self.get(*_args) = raise(RSXPError,'HTTPLite.get is not ported yet')
  def self.post(*_args) = raise(RSXPError,'HTTPLite.post is not ported yet')
  def self.post_body(*_args) = raise(RSXPError,'HTTPLite.post_body is not ported yet')
end

class RGSSError < StandardError; end
class RGSSReset < Exception; end
class RSXPError < StandardError; end
class RSXPStop < Exception; end
MKXPError = RSXPError unless defined?(MKXPError)
MKXPStop = RSXPStop unless defined?(MKXPStop)

# Ruby 3.2 removed File.exists?/Dir.exists?; older RGSS games (and their
# preload.rb) still use them.
class File
  class << self
    alias_method :exists?, :exist? unless method_defined?(:exists?)
  end
end
class Dir
  class << self
    alias_method :exists?, :exist? unless method_defined?(:exists?)
  end
end

# Game scripts authored for Windows often normalize asset names to uppercase
# before checking them (e.g. the title plugin asks whether TITULO.ogg exists
# when the file is named Titulo.ogg). The native asset loader already resolves
# paths case-insensitively; make Ruby's FileTest preflight match that behavior.
module FileTest
  class << self
    alias_method :__rsxp_case_sensitive_exist?, :exist? unless method_defined?(:__rsxp_case_sensitive_exist?)

    def exist?(path)
      return true if __rsxp_case_sensitive_exist?(path)
      !RSXP.case_insensitive_path(path).nil?
    end

    alias_method :exists?, :exist?
  end
end

module RSXP
  # Resolve a path component by component with Windows-style case folding.
  # Keeping this at the Ruby boundary makes filesystem preflight checks and
  # subsequent File.open calls agree with the host asset loader.
  def self.case_insensitive_path(path)
    path = path.to_path if path.respond_to?(:to_path)
    return nil unless path.is_a?(String) && !path.empty?
    return path if FileTest.__rsxp_case_sensitive_exist?(path)

    absolute = File.expand_path(path.tr('\\', '/'))
    current = File::SEPARATOR
    absolute.split(File::SEPARATOR).reject(&:empty?).each do |part|
      candidate=File.join(current,part)
      if FileTest.__rsxp_case_sensitive_exist?(candidate)
        current=candidate
        next
      end
      entries = begin
        stat=File.stat(current)
        stamp=[stat.mtime,stat.ctime,stat.ino]
        @case_directories ||= {}
        cached=@case_directories[current]
        unless cached && cached[0]==stamp
          # Directory timestamps invalidate both successful and missing names.
          # Bound the index without retaining every directory a game visits.
          @case_directories.clear if @case_directories.size>=256
          names=Dir.children(current).each_with_object({}) { |entry,index| index[entry.downcase(:fold)] ||= entry }
          cached=@case_directories[current]=[stamp,names]
        end
        cached[1]
      rescue SystemCallError
        return nil
      end
      matched = entries[part.downcase(:fold)]
      return nil unless matched
      current = File.join(current, matched)
    end
    FileTest.__rsxp_case_sensitive_exist?(current) ? current : nil
  end

  # Resolve mixed-case game asset paths for direct Ruby reads as well as the
  # native engine's asset requests. Preserve normal File.open mode/kwargs and
  # block semantics by forwarding them to the original implementation.
  class << File
    alias_method :__rsxp_case_sensitive_open, :open unless method_defined?(:__rsxp_case_sensitive_open)

    def open(path, *args, **kwargs, &block)
      resolved = RSXP.case_insensitive_path(path)
      __rsxp_case_sensitive_open(resolved || path, *args, **kwargs, &block)
    end
  end

  # Some games reopen WeakRef with a Ruby 1.8 implementation. Keep the engine's
  # resource registry on the interpreter's GC-aware implementation.
  ResourceRef = WeakRef.dup
  ERROR_OUTPUT = STDERR.method(:write)
  ERROR_FORMAT = Exception.instance_method(:full_message)
  SYNC_QUEUE = []
  def self.call(op, *args, payload: ''.b)
    flush_sync unless op == 'sync_batch'
    result = __rsxp_native(([op, *args].join("\t") + "\n").b + payload.b)
    raise RSXPStop if result.nil?
    result
  end
  # Sprite/plane/viewport snapshots are only consumed by Graphics.update. Batch
  # adjacent snapshots to avoid crossing the Ruby/WASM bridge once per object.
  def self.flush_sync
    return if SYNC_QUEUE.empty?
    payload = SYNC_QUEUE.join("\x1e").b
    SYNC_QUEUE.clear
    result = __rsxp_native("sync_batch\n".b + payload)
    raise RSXPStop if result.nil?
    nil
  end
  # Numeric values are immutable; strings accepted by Integer/Float need a
  # private copy so in-place edits invalidate the snapshot. Other coercible
  # objects are deliberately not cached, since their conversion can change.
  def self.snapshot_number(value)
    return value if value.is_a?(Numeric)
    value.is_a?(String) ? value.dup : nil
  end
  def self.byte_index(object,index)
    object.is_a?(String) ? object.getbyte(index) : object[index]
  end
  def self.integer(value) = Integer(value)
  def self.puts(*messages) = Kernel.puts(*messages)
  def self.sync(object, op, *values)
    return if object.instance_variable_get(:@__native_state)==values
    SYNC_QUEUE << ([op, object.__id, *values].join("\t").b)
    object.instance_variable_set(:@__native_state,values)
  end
  def self.channel(value) = [[Float(value).to_i, 0].max, 255].min
  def self.volume(value) = [[Integer(value), 0].max, 100].min

  # RGSS strings are byte strings; text from Marshal-loaded data is often tagged
  # ASCII-8BIT while holding UTF-8 bytes. Reinterpret, replacing invalid bytes.
  def self.utf8(value)
    string=value.to_s
    string=string.dup.force_encoding(Encoding::UTF_8) unless string.encoding==Encoding::UTF_8
    string.valid_encoding? ? string : string.encode(Encoding::UTF_8,invalid: :replace,undef: :replace)
  end
  def self.finalizer(op, id)
    proc { call(op, id) rescue nil }
  end
  # Extension point: hosts (the browser frontend) may hook here after the RGSS
  # API is defined but before the game's scripts run. Defined only if a host
  # has not already provided one.
  def self.before_scripts; end unless singleton_class.method_defined?(:before_scripts)
  def self.prepare_script(source)
    if RGSS_VERSION==1
      # Binary readers written for Ruby 1.8 expect String#[0] to return a byte.
      source=source.gsub(/\.read\(1\)\[0\]/,'.read(1).getbyte(0)')
      source=source.gsub(/\b([a-z_]\w*)\[([^\[\]\n]+)\](\s*(?:[!=]=\s*0x[\da-fA-F]+|>>\s*\d+))/,'RSXP.byte_index(\1,\2)\3')
      # Old Essentials event handlers use return to leave just the callback.
      # A modern non-lambda proc instead returns out of the script evaluation.
      # Keep the legacy event convention and its permissive arity while giving
      # zero/one/two-argument callbacks local returns, including inside loops.
      source=source.gsub(/(\bEvents\.\w+[ \t]*\+=[ \t]*)proc([ \t]*(?:\{|do\b))[ \t]*\|[ \t]*([a-z_]\w*)[ \t]*,[ \t]*([a-z_]\w*)[ \t]*\|/) do
        "#{$1}lambda#{$2}|#{$3}=nil,#{$4}=nil,*|"
      end
      source=source.gsub(/(\bEvents\.\w+[ \t]*\+=[ \t]*)proc([ \t]*(?:\{|do\b))[ \t]*\|[ \t]*([a-z_]\w*)[ \t]*\|/) do
        "#{$1}lambda#{$2}|#{$3}=nil,*|"
      end
      source=source.gsub(/(\bEvents\.\w+[ \t]*\+=[ \t]*)proc([ \t]*(?:\{|do\b))(?!\s*\|)/,'\1lambda\2|*|')
    end
    # Ruby 1.8 byte-API polyfills must not replace Ruby's native byte methods
    # with modern character-indexing methods.
    source=source.gsub(/^([ \t]*alias[ \t]+(getbyte|setbyte|bytesize)[ \t]+(?:\[\]=?|size))[ \t]*\r?$/,'\1 unless method_defined?(:\2)')
    # Older Essentials supplies a plain WeakRef class. Reopening stdlib's
    # Delegator subclass changes !ref for dead references, causing bitmap
    # caches to return dead wrappers instead of nil. Restore its plain base.
    if source.match?(/^class WeakRef[ \t]*\r?$/) && source.include?('@@id_rev_map') &&
        defined?(WeakRef) && WeakRef < Delegator
      Object.send(:remove_const,:WeakRef)
      Object.const_set(:WeakRef,Class.new)
    end
    source
  end
  # Repair obsolete syntax only on lines rejected by the compiler. Preserve
  # line counts and valid rescue/retry clauses for accurate game backtraces.
  def self.eval_script(source,name)
    source.replace(prepare_script(source))
    begin
      eval(source,TOPLEVEL_BINDING,name,1)
    rescue SyntaxError => error
      message=error.message.gsub(/\e\[[0-9;]*m/,'')
      lines=source.lines
      numbers=message.scan(/(?:^|\n)>\s*(\d+)\s*\|/).flatten
      numbers.concat(message.scan(/#{Regexp.escape(name)}:(\d+):/).flatten)
      changed=false
      numbers.uniq.each do |number|
        index=number.to_i-1
        line=lines[index]
        next unless line
        fixed=line
        if message.include?('Invalid retry') && line.match?(/\A[ \t]*retry\b/)
          fixed=line.sub(/\bretry\b/,'redo')
        elsif message.include?("unexpected ':'")
          fixed=line.sub(/\A([ \t]*when\s+-?\d+(?:\s*,\s*-?\d+)*)[ \t]*:(?!:)/,'\1 then')
        elsif message.include?('invalid multibyte escape')
          fixed=line.gsub('\x7f-\x9f','\u007f-\u009f')
        end
        changed ||= fixed!=line
        lines[index]=fixed
      end
      raise unless changed
      source.replace(lines.join)
      retry
    end
  end
  # Browser hosts use this hook to cooperatively yield during large startup
  # workloads. Desktop execution leaves it as a no-op.
  def self.startup_yield; end unless singleton_class.method_defined?(:startup_yield)
  # rsxp re-tags ASCII-8BIT strings from Marshal data as UTF-8.
  def self.force_utf8(object)
    if object.is_a?(String) && object.encoding == Encoding::ASCII_8BIT
      object.force_encoding(Encoding::UTF_8)
    end
    object
  end
  # RGSS/rsxp lets scripts call top-level helper methods as `Kernel.foo`
  # (e.g. `Kernel.pbShowCommands`). Plain Ruby makes top-level `def`s private
  # methods of Object, so republish the ones the game defined as singleton
  # methods of Kernel that run with the top-level `main` object as self.
  def self.publish_top_level_methods
    main = eval('self', TOPLEVEL_BINDING)
    Object.private_instance_methods(false).each do |name|
      next if Kernel.singleton_class.method_defined?(name)
      Kernel.define_singleton_method(name) { |*args, **kwargs, &block| main.__send__(name, *args, **kwargs, &block) }
    end
    # Pokémon Z closes PokeBattle_Scene before defining raiseSpeciesStats at
    # top level. the engine's patched Ruby bypasses private/protected call checks;
    # stock Ruby rejects the explicit @scene call, so expose this helper on
    # the class that the game intended to receive it.
    if RGSS_VERSION==1 && defined?(PokeBattle_Scene) &&
        Object.private_instance_methods(false).include?(:raiseSpeciesStats) &&
        !PokeBattle_Scene.method_defined?(:raiseSpeciesStats)
      PokeBattle_Scene.define_method(:raiseSpeciesStats,Object.instance_method(:raiseSpeciesStats))
      PokeBattle_Scene.send(:public,:raiseSpeciesStats)
    end
    patch_plugin_manager
    install_native_tilemap
    install_legacy_byte_reader
  end
  # Essentials' Ruby 1.8 StringInput implements each_byte by yielding getc.
  # Restore getc's byte-number contract for that reader without changing the
  # modern String/IO APIs used by the engine and other libraries.
  module LegacyByteReader
    def getc
      value=super
      value.is_a?(String) ? value.getbyte(0) : value
    end
  end
  def self.install_legacy_byte_reader
    return unless RGSS_VERSION==1 && defined?(StringInput) && defined?(FileInputMixin)
    return unless StringInput.ancestors.include?(FileInputMixin) && StringInput.method_defined?(:getc)
    StringInput.prepend(LegacyByteReader) unless StringInput.ancestors.include?(LegacyByteReader)
  end
  # Essentials keeps tile selection, bridge/reflection rules and plugin hooks
  # in Ruby. Register its tile resources so Rust owns their atlas and ordered
  # ground/priority geometry instead of submitting them as ordinary sprites.
  module NativeTileSprite
    def initialize(*args)
      super
      RSXP.call('sprite_tile',__id)
    end
  end

  def self.install_native_tilemap
    return if ENV['RSXP_NATIVE_TILEMAP']=='0'
    return unless defined?(TilemapRenderer) && TilemapRenderer.const_defined?(:TileSprite,false)
    klass=TilemapRenderer.const_get(:TileSprite,false)
    return unless klass.is_a?(Class) && klass < Sprite
    klass.prepend(NativeTileSprite) unless klass.ancestors.include?(NativeTileSprite)

  end
  # Some plugins accidentally define standard RGSS sprite setters (x=, y=, ...)
  # under `private`, which breaks base code such as PictureEx that assigns to
  # them from outside. RGSS guarantees these accessors are public, so restore
  # that after the plugins have loaded.
  def self.republicize_sprite_setters
    return unless defined?(Sprite)
    accessors = %i[x y z ox oy zoom_x zoom_y angle mirror opacity blend_type visible
                   color tone src_rect bitmap viewport invert pattern pattern_opacity
                   pattern_tile pattern_blend_type pattern_scroll_x pattern_scroll_y
                   pattern_zoom_x pattern_zoom_y]
    ObjectSpace.each_object(Class) do |klass|
      next unless klass < Sprite
      accessors.each do |name|
        setter = :"#{name}="
        klass.send(:public, setter) if klass.private_method_defined?(setter)
      end
    end
  end
  # Runs the setter fixup once, after PluginManager has loaded the plugins.
  def self.patch_plugin_manager
    return if $__rsxp_plugin_manager_patched
    return unless defined?(PluginManager) && PluginManager.respond_to?(:runPlugins)
    $__rsxp_plugin_manager_patched = true
    PluginManager.singleton_class.class_eval do
      alias_method :__rsxp_runPlugins, :runPlugins unless method_defined?(:__rsxp_runPlugins)
      define_method(:runPlugins) do
        __rsxp_runPlugins
        RSXP.republicize_sprite_setters
      end
    end
  end
end

# Some game helpers are defined or made private after Main starts, beyond the
# script loader's publication pass. Keep legacy Kernel.helper calls working
# only for methods defined directly on Object, leaving Kernel's own private
# methods and unrelated missing methods subject to normal Ruby rules.
module RSXP::KernelGameHelpers
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
Kernel.singleton_class.prepend(RSXP::KernelGameHelpers)

# rsxp aliases Marshal.load to force ASCII-8BIT strings to UTF-8. Game data
# saved by older RPG Maker tools has no encoding tag, so do the same here.
class << Marshal
  alias __rsxp_load_alias load unless method_defined?(:__rsxp_load_alias)
  def load(port, proc = nil)
    if proc
      __rsxp_load_alias(port, ->(object) { proc.call(RSXP.force_utf8(object)) })
    else
      __rsxp_load_alias(port, RSXP.method(:force_utf8))
    end
  end
end

MKXP = RSXP unless defined?(MKXP) # Compatibility constant for older games.

# Host-backed audio API for the desktop Kira/CPAL backend and browser media
# backend. The host owns playback while Ruby retains the RGSS method surface.
module Audio
  # Native ops; `channel` is one of bgm/bgs/me/se. Positions are seconds and
  # fade times are milliseconds, matching rsxp.
  def self.__play(channel, file, volume, pitch, position = 0.0)
    RSXP.call("audio_play\t#{channel}\t#{RSXP.volume(volume)}\t#{Float(pitch)}\t#{Float(position)}",
              payload: RSXP.utf8(file))
    nil
  end
  def self.bgm_play(file, volume=100, pitch=100, position=0, _track=nil)
    __play('bgm', file, volume, pitch, position)
  end
  def self.bgm_stop(_track=nil)
    RSXP.call('audio_stop','bgm')
    nil
  end
  def self.bgm_fade(time, _track=nil)
    RSXP.call('audio_fade','bgm',Integer(time))
    nil
  end
  def self.bgm_pos(_track=nil) = RSXP.call('audio_pos','bgm').to_f
  def self.bgm_volume = RSXP.call('audio_volume','bgm').to_i
  def self.bgm_set_volume(value)
    RSXP.call('audio_set_volume','bgm',RSXP.volume(value))
    nil
  end
  def self.bgs_play(file, volume=100, pitch=100, _position=0, _track=nil)
    __play('bgs', file, volume, pitch)
  end
  def self.bgs_stop(_track=nil)
    RSXP.call('audio_stop','bgs')
    nil
  end
  def self.bgs_fade(time, _track=nil)
    RSXP.call('audio_fade','bgs',Integer(time))
    nil
  end
  def self.bgs_pos(_track=nil) = RSXP.call('audio_pos','bgs').to_f
  def self.me_play(file, volume=100, pitch=100)
    __play('me', file, volume, pitch)
  end
  def self.me_stop(*_args)
    RSXP.call('audio_stop','me')
    nil
  end
  def self.me_fade(time, *_args)
    RSXP.call('audio_fade','me',Integer(time))
    nil
  end
  def self.se_play(file, volume=100, pitch=100)
    __play('se', file, volume, pitch)
  end
  def self.se_stop(*_args)
    RSXP.call('audio_stop','se')
    nil
  end
  def self.setup_midi = nil
  def self.__reset__
    %w[bgm bgs me se].each { |channel| RSXP.call('audio_stop',channel) }
    nil
  end
end

RGSS_VERSION = RSXP.call('version').to_i
if RGSS_VERSION==1
  class Object
    alias_method :id, :__id__ unless method_defined?(:id)
  end
end
$DEBUG = false
$TEST = RGSS_VERSION >= 2
$BTEST = false

module System
  VERSION="rsxp 0.1.0"
  START_TIME=Process.clock_gettime(Process::CLOCK_MONOTONIC)
  def self.uptime = Process.clock_gettime(Process::CLOCK_MONOTONIC)-START_TIME
  def self.real_uptime = uptime
  def self.platform = RUBY_PLATFORM
  # Host queries used by the Win32API shim and by cross-platform games.
  def self.is_windows? = !!(RUBY_PLATFORM =~ /mingw|mswin|cygwin/)
  def self.is_linux? = !!(RUBY_PLATFORM =~ /linux/)
  def self.is_mac? = !!(RUBY_PLATFORM =~ /darwin/)
  def self.puts(*messages) = Kernel.puts(*messages)
  def self.user_language = ENV.fetch('LANG','en_US').split(/[.@]/).first
  def self.user_name = ENV['USER'] || ENV['USERNAME'] || 'Player'
  def self.game_title = ENV.fetch('RSXP_GAME_TITLE','rsxp')
  def self.power_state
    seconds,percent,discharging = RSXP.call('system_power').split("\t")
    seconds,percent = seconds.to_i,percent.to_i
    {seconds: (seconds < 0 ? nil : seconds),percent: (percent < 0 ? nil : percent),discharging: discharging == '1'}
  end
  def self.set_window_title(title) = RSXP.call('window_title',payload:String(title))
  # mkxp-z System compatibility surface. Values that need OS/host support that
  # rsxp does not expose return safe neutral results instead of raising.
  def self.window_title = (@window_title || game_title)
  def self.window_title=(title)
    @window_title=title.to_s
    set_window_title(@window_title)
  end
  def self.delta = Graphics.delta
  def self.memory
    begin
      File.foreach('/proc/meminfo') do |line|
        if line =~ /\AMemTotal:\s+(\d+)\s+kB/
          return Regexp.last_match(1).to_i * 1024
        end
      end
    rescue StandardError
      # Not Linux, or /proc is unavailable.
    end
    0
  end
  def self.nproc
    begin
      require 'etc'
      Etc.nprocessors
    rescue StandardError
      1
    end
  end
  def self.file_exist?(path) = FileTest.exist?(path)
  def self.launch(*_args) = false
  def self.mount(*_args) = nil
  def self.unmount(*_args) = nil
  def self.reload_cache = nil
  def self.desensitize(path) = path.to_s
  def self.show_settings = nil
  def self.default_font_family=(value)
    @default_font_family=value
  end
  def self.parse_csv(text)
    begin
      require 'csv'
      CSV.parse(text.to_s)
    rescue StandardError, LoadError
      text.to_s.split(/\r?\n/).map { |line| line.split(',') }
    end
  end
  def self.is_bsd? = !!(RUBY_PLATFORM =~ /bsd/)
  def self.is_really_windows? = is_windows?
  def self.is_really_linux? = is_linux?
  def self.is_really_mac? = is_mac?
  def self.is_really_bsd? = is_bsd?
  def self.is_rosetta? = false
  def self.is_wine? = false
  def self.data_directory
    base=ENV['XDG_DATA_HOME'] || (ENV['HOME'] && File.join(ENV['HOME'],'.local','share')) || File.join(Dir.pwd,'.local','share')
    path=File.join(base,'rsxp',game_title.gsub(/[^[:alnum:]_. -]/,'_'))
    begin
      FileUtils.mkdir_p(path)
    rescue StandardError
      # The browser/WASI filesystem may be read-only; saves then fall back to
      # whatever the host provides.
    end
    path
  end
end

# mkxp-z configuration surface. rsxp has no user configuration file, so reads
# return nil (or the caller's default) and writes are kept for the session.
module CFG
  @values={}
  def self.[](key) = @values[key.to_s]
  def self.[]=(key,value)
    @values[key.to_s]=value
  end
end

class Color
  attr_reader :red, :green, :blue, :alpha
  def initialize(red=0, green=0, blue=0, alpha=255) = set(red, green, blue, alpha)
  def set(red, green=nil, blue=nil, alpha=255)
    if red.is_a?(Color)
      red, green, blue, alpha = red.red, red.green, red.blue, red.alpha
    end
    self.red, self.green, self.blue, self.alpha = red, green, blue, alpha
    self
  end
  %i[red green blue alpha].each do |key|
    class_eval "def #{key}=(value); @#{key} = Float(value).clamp(0.0,255.0); end", __FILE__, __LINE__
  end
  def ==(other)
    other.is_a?(Color) && red == other.red && green == other.green && blue == other.blue && alpha == other.alpha
  end
  alias === ==
  alias eql? ==
  def to_a = [red.to_i, green.to_i, blue.to_i, alpha.to_i]
  def _dump(_depth) = [red, green, blue, alpha].pack('E4')
  def self._load(bytes)
    raise TypeError, 'bad Color payload' unless bytes.bytesize == 32
    new(*bytes.unpack('E4'))
  end
end

module RSXP::Flashable
  def flash(color,duration)
    __check
    duration=Integer(duration)
    return nil if duration<1
    raise TypeError, 'expected Color or nil' unless color.nil? || color.is_a?(Color)
    @__rsxp_flash_color=Color.new(0,0,0,0) unless @__rsxp_flash_color
    @__rsxp_flashing=true
    @__rsxp_flash_duration=duration
    @__rsxp_flash_counter=0
    if color.nil?
      @__rsxp_empty_flash=true
      return nil
    end
    # Color#initialize may be overridden by game scripts with stricter
    # argument handling, so don't rely on its optional zero-argument form.
    @__rsxp_flash_color=Color.new(0,0,0,0).set(color)
    @__rsxp_flash_alpha=@__rsxp_flash_color.alpha
    @__rsxp_empty_flash=false
    nil
  end
  def update
    __check
    return nil unless @__rsxp_flashing
    @__rsxp_flash_counter+=1
    if @__rsxp_flash_counter>@__rsxp_flash_duration
      @__rsxp_flash_color=Color.new(0,0,0,0)
      @__rsxp_flashing=false
      @__rsxp_empty_flash=false
      return nil
    end
    unless @__rsxp_empty_flash
      alpha=@__rsxp_flash_alpha || @__rsxp_flash_color.alpha || 0
      @__rsxp_flash_color.alpha=alpha*(1.0-@__rsxp_flash_counter.to_f/@__rsxp_flash_duration)
    end
    nil
  end
  def __flash_color_for_sync(color)
    @__rsxp_flashing && @__rsxp_flash_color && @__rsxp_flash_color.alpha>color.alpha ? @__rsxp_flash_color : color
  end
end

class Tone
  attr_reader :red, :green, :blue, :gray
  def initialize(red=0, green=0, blue=0, gray=0) = set(red, green, blue, gray)
  def set(red, green=nil, blue=nil, gray=0)
    if red.is_a?(Tone)
      red, green, blue, gray = red.red, red.green, red.blue, red.gray
    end
    self.red, self.green, self.blue, self.gray = red, green, blue, gray
    self
  end
  %i[red green blue].each do |key|
    class_eval "def #{key}=(value); @#{key} = Float(value).clamp(-255.0,255.0); end", __FILE__, __LINE__
  end
  def gray=(value)
    @gray = Float(value).clamp(0.0,255.0)
  end
  def ==(other)
    other.is_a?(Tone) && red == other.red && green == other.green && blue == other.blue && gray == other.gray
  end
  alias === ==
  alias eql? ==
  def _dump(_depth) = [red,green,blue,gray].pack('E4')
  def self._load(bytes)
    raise TypeError, 'bad Tone payload' unless bytes.bytesize == 32
    new(*bytes.unpack('E4'))
  end
end

class Rect
  attr_reader :x, :y, :width, :height
  def initialize(x=0, y=0, width=0, height=0)
    # RGSS's Rect#initialize writes fields directly (no overridable setters).
    @x=Integer(x)
    @y=Integer(y)
    @width=[Integer(width),0].max
    @height=[Integer(height),0].max
  end
  def set(x,y=nil,width=nil,height=nil)
    x,y,width,height = x.x,x.y,x.width,x.height if x.is_a?(Rect)
    # RGSS's Rect#set writes its fields directly and never calls overridden
    # x=/y=/width=/height= methods (WindowCursorRect relies on this).
    @x=Integer(x)
    @y=Integer(y)
    @width=[Integer(width),0].max
    @height=[Integer(height),0].max
    self
  end
  %i[x y].each do |key|
    class_eval "def #{key}=(value); @#{key} = Integer(value); end", __FILE__, __LINE__
  end
  %i[width height].each do |key|
    class_eval "def #{key}=(value); @#{key} = [Integer(value),0].max; end", __FILE__, __LINE__
  end
  def ==(other)
    other.is_a?(Rect) && x == other.x && y == other.y && width == other.width && height == other.height
  end
  alias === ==
  alias eql? ==
  def empty = set(0,0,0,0)
  def to_a = [x,y,width,height]
  def _dump(_depth) = to_a.pack('l<4')
  def self._load(bytes)
    raise TypeError, 'bad Rect payload' unless bytes.bytesize == 16
    new(*bytes.unpack('l<4'))
  end
end

class Table
  # Portable storage used when the host does not install native Table methods.
  unless method_defined?(:__initialize)
    def __initialize(x, y, z)
      __check_table_size(x, y, z)
      @x, @y, @z = x, y, z
      @data = Array.new(x * y * z, 0)
    end
    def __get(x, y, z)
      return nil unless @data
      return nil if x < 0 || y < 0 || z < 0 || x >= @x || y >= @y || z >= @z
      @data[(z * @y + y) * @x + x]
    end
    def __set(x, y, z, value)
      return unless @data
      return if x < 0 || y < 0 || z < 0 || x >= @x || y >= @y || z >= @z
      @data[(z * @y + y) * @x + x] = ((value + 32768) & 65535) - 32768
    end
    def initialize_copy(other)
      super
      @data = other.instance_variable_get(:@data)&.dup
    end
    def __check_table_size(x, y, z)
      raise ArgumentError, 'Table exceeds the 16 million element limit' if x * y * z > 16_777_216
      raise ArgumentError, 'invalid Table dimensions' if [x, y, z].any? { |n| n > 2_147_483_647 }
    end
    def __resize(x, y, z)
      __check_table_size(x, y, z)
      return unless @data
      old, ox, oy, oz = @data, @x, @y, @z
      @x, @y, @z = x, y, z
      @data = Array.new(x * y * z, 0)
      if old
        [z, oz].min.times do |k|
          [y, oy].min.times do |j|
            [x, ox].min.times do |i|
              @data[(k * y + j) * x + i] = old[(k * oy + j) * ox + i]
            end
          end
        end
      end
    end
    def __dimensions
      @data ? [@x, @y, @z] : [0, 0, 0]
    end
    def _dump(_depth = 0)
      return [1, 0, 1, 1, 0].pack('l<5') unless @data
      dim = @z > 1 ? 3 : (@y > 1 ? 2 : 1)
      [dim, @x, @y, @z, @data.size].pack('l<5') + @data.pack('s<*')
    end
    def self._load(bytes)
      raise ArgumentError, 'bad Table payload' if bytes.bytesize < 20
      dim, x, y, z, count = bytes.unpack('l<5')
      unless (1..3).cover?(dim) && [x, y, z, count].all? { |n| n >= 0 } && x * y * z == count && bytes.bytesize == 20 + count * 2
        raise ArgumentError, 'bad Table payload'
      end
      table = new(x, y, z)
      data = table.instance_variable_get(:@data)
      bytes[20..].unpack('s<*').each_with_index { |value, i| data[i] = value }
      table
    end
  end
  def initialize(x,y=1,z=1) = __initialize(*[x,y,z].map { |v| [Integer(v),0].max })
  def [](x,y=0,z=0) = __get(Integer(x),Integer(y),Integer(z))
  def []=(*args)
    raise ArgumentError, 'expected one to three coordinates and a value' unless (2..4).cover?(args.size)
    value = Integer(args.pop)
    coords = args.map { |v| Integer(v) }
    coords << 0 while coords.size < 3
    __set(*coords,value)
  end
  def resize(x,y=1,z=1) = __resize(*[x,y,z].map { |v| [Integer(v),0].max })
  def xsize = __dimensions[0]
  def ysize = __dimensions[1]
  def zsize = __dimensions[2]
end

module NativeResource
  attr_reader :__id
  def __own(id,op)
    @__id, @__dispose_op, @__disposed = Integer(id),op,false
    ObjectSpace.define_finalizer(self,RSXP.finalizer(op,@__id))
  end
  def disposed? = @__disposed
  def __check
    raise RGSSError, "disposed #{self.class}" if disposed?
  end
  def dispose
    return if disposed?
    RSXP.call(@__dispose_op,@__id)
    @__disposed = true
    ObjectSpace.undefine_finalizer(self)
    nil
  end
end

class Font
  class << self
    attr_accessor :default_name,:default_size,:default_bold,:default_italic,:default_color,:default_outline,:default_shadow,:default_out_color
  end
  self.default_name='Arial'
  self.default_size=22
  self.default_bold=false
  self.default_italic=false
  self.default_color=Color.new(255,255,255)
  self.default_outline=false
  self.default_shadow=false
  self.default_out_color=Color.new(0,0,0,128)
  attr_accessor :name,:size,:bold,:italic,:color,:outline,:shadow,:out_color
  # mkxp-z exposes short aliases for bold/italic.
  alias_method :b, :bold
  alias_method :b=, :bold=
  alias_method :i, :italic
  alias_method :i=, :italic=
  def initialize(name=nil,size=nil)
    @name=name || Font.default_name
    @size=size || Font.default_size
    @bold,@italic=Font.default_bold,Font.default_italic
    @color=Font.default_color.clone
    @outline,@shadow=Font.default_outline,Font.default_shadow
    @out_color=Font.default_out_color.clone
  end
  def self.exist?(name)
    Dir.glob('Fonts/*').any? { |path| File.basename(path,File.extname(path)).casecmp?(name.to_s) }
  end
  def __payload(text) = Array(name).join("\x1f").b+"\0"+RSXP.utf8(text).b
end

class Bitmap
  def self.max_size = 4096
  include NativeResource
  attr_writer :font
  def font = (@font ||= Font.new)
  def initialize(width,height=nil)
    id=height.nil? ? RSXP.call('bitmap_load',payload:String(width)) : RSXP.call('bitmap_new',Integer(width),Integer(height))
    __own(id,'bitmap_dispose')
  end
  def self.__from_id(id)
    object = allocate
    object.__own(id,'bitmap_dispose')
    object
  end
  def initialize_copy(other)
    other.__check
    ObjectSpace.undefine_finalizer(self)
    __own(RSXP.call('bitmap_clone',other.__id),'bitmap_dispose')
  end
  def width
    __check
    (@__dimensions ||= RSXP.call('bitmap_size',__id).split("\t").map!(&:to_i))[0]
  end
  def height
    __check
    (@__dimensions ||= RSXP.call('bitmap_size',__id).split("\t").map!(&:to_i))[1]
  end
  def rect = Rect.new(0,0,width,height)
  # Animated GIFs decode in the engine and animate using the host clock.
  def animated?
    __check
    RSXP.call('bitmap_animated',__id) != '0'
  end
  def frame_count
    __check
    RSXP.call('bitmap_frame_count',__id).to_i
  end
  def current_frame
    __check
    RSXP.call('bitmap_current_frame',__id).to_i
  end
  def frame_rate
    __check
    RSXP.call('bitmap_frame_rate',__id).to_f
  end
  def frame_rate=(value)
    __check
    RSXP.call('bitmap_frame_rate_set',__id,Float(value))
  end
  # Bitmaps here are never "mega" (split) surfaces.
  def mega? = false
  def playing
    __check
    RSXP.call('bitmap_playing',__id) != '0'
  end
  def playing=(value)
    __check
    RSXP.call('bitmap_playing_set',__id,value ? 1 : 0)
  end
  def goto_and_stop(frame)
    __check
    RSXP.call('bitmap_goto_and_stop',__id,Integer(frame))
    nil
  end
  def goto_and_play(frame)
    __check
    RSXP.call('bitmap_goto_and_play',__id,Integer(frame))
    nil
  end
  def play
    __check
    RSXP.call('bitmap_play',__id)
    nil
  end
  def stop
    __check
    RSXP.call('bitmap_stop',__id)
    nil
  end
  # Raw RGBA bytes, matching mkxp-z's Bitmap#raw_data / #raw_data=.
  def raw_data
    __check
    RSXP.call('bitmap_raw_data',__id).force_encoding(Encoding::BINARY)
  end
  def raw_data=(value)
    __check
    RSXP.call('bitmap_set_raw_data',__id,payload:String(value).b)
    nil
  end
  # KGL2 compatibility operations driven by KGL2_Impl below.
  def _kgl_invert
    __check
    RSXP.call('bitmap_kgl_invert',__id)
    nil
  end
  def _kgl_compress_alpha
    __check
    RSXP.call('bitmap_kgl_compress_alpha',__id)
    nil
  end
  def _kgl_subtract_rect(x,y,source,area,opacity=255)
    __check
    source.__check
    RSXP.call('bitmap_kgl_subtract',__id,Integer(x),Integer(y),source.__id,*area.to_a,RSXP.channel(opacity))
    nil
  end
  def _kgl_shadow_shader_h(x1,x2,y,soft)
    __check
    RSXP.call('bitmap_kgl_shadow_h',__id,Integer(x1),Integer(x2),Integer(y),soft ? 1 : 0).to_i
  end
  def _kgl_shadow_shader_v(y1,y2,x,wall,soft)
    __check
    RSXP.call('bitmap_kgl_shadow_v',__id,Integer(y1),Integer(y2),Integer(x),wall ? 1 : 0,soft ? 1 : 0).to_i
  end
  # Hi-res asset replacement is not implemented; a bitmap is its own hi-res copy.
  def hires = self
  def gradient_fill_rect(*args)
    __check
    vertical = args.last == true || args.last == false ? args.pop : false
    if args.size == 3 && args[0].is_a?(Rect)
      area, color1, color2 = args
    elsif args.size == 6
      area = Rect.new(*args[0, 4])
      color1 = args[4]
      color2 = args[5]
    else
      raise ArgumentError, 'gradient_fill_rect expects a Rect or x, y, width, height'
    end
    raise TypeError, 'expected Color' unless color1.is_a?(Color) && color2.is_a?(Color)
    RSXP.call('bitmap_gradient_fill',__id,*area.to_a,*color1.to_a,*color2.to_a,vertical ? 1 : 0)
    nil
  end
  def blur
    __check
    RSXP.call('bitmap_blur',__id)
    nil
  end
  def radial_blur(angle,divisions)
    __check
    RSXP.call('bitmap_radial_blur',__id,Integer(angle),Integer(divisions))
    nil
  end
  def snap_to_bitmap(position=nil)
    __check
    Bitmap.__from_id(RSXP.call('bitmap_snap',__id,position.nil? ? -1 : Integer(position)))
  end
  def to_file(filename)
    __check
    RSXP.call('bitmap_to_file',__id,payload:String(filename))
    nil
  end
  def hue_change(value)
    __check
    RSXP.call('bitmap_hue',__id,Float(value))
    nil
  end
  def clear
    __check
    RSXP.call('bitmap_clear',__id)
    nil
  end
  def fill_rect(*args)
    __check
    color = args.pop
    raise TypeError, 'expected Color' unless color.is_a?(Color)
    area = args.size == 1 && args[0].is_a?(Rect) ? args[0] : Rect.new(*args)
    RSXP.call('bitmap_fill',__id,*area.to_a,*color.to_a)
    nil
  end
  def clear_rect(*args) = fill_rect(*args,Color.new(0,0,0,0))
  def get_pixel(x,y)
    __check
    Color.new(*RSXP.call('bitmap_get',__id,Integer(x),Integer(y)).split("\t").map(&:to_i))
  end
  def set_pixel(x,y,color)
    __check
    raise TypeError, 'expected Color' unless color.is_a?(Color)
    RSXP.call('bitmap_set',__id,Integer(x),Integer(y),*color.to_a)
    nil
  end
  def blt(x,y,source,area,opacity=255)
    stretch_blt(Rect.new(x,y,area.width,area.height),source,area,opacity)
  end
  def stretch_blt(destination,source,area,opacity=255)
    __check
    source.__check
    RSXP.call('bitmap_blt',__id,*destination.to_a,source.__id,*area.to_a,RSXP.channel(opacity))
    nil
  end
  def text_size(text)
    __check
    w,h=RSXP.call('text_size',Integer(font.size),font.bold ? 1 : 0,font.italic ? 1 : 0,payload:font.__payload(text)).split("\t").map(&:to_i)
    Rect.new(0,0,w,h)
  end
  def draw_text(*args)
    __check
    area=args.first.is_a?(Rect) ? args.shift : Rect.new(*args.shift(4))
    value=args.shift
    align=Integer(args.shift || 0)
    raise ArgumentError,'too many draw_text arguments' unless args.empty?
    RSXP.call('bitmap_text',__id,*area.to_a,align,Integer(font.size),font.bold ? 1 : 0,font.italic ? 1 : 0,*font.color.to_a,font.outline ? 1 : 0,font.shadow ? 1 : 0,*font.out_color.to_a,payload:font.__payload(value))
    nil
  end
end

class Viewport
  include NativeResource
  include RSXP::Flashable
  @instances = []
  class << self
    attr_reader :instances
    def __sync_all
      Viewport.instances.delete_if do |reference|
        begin
          viewport = reference.__getobj__
        rescue RSXP::ResourceRef::RefError
          next true
        end
        viewport.__sync unless viewport.disposed?
        viewport.disposed?
      end
    end
  end
  attr_accessor :z,:ox,:oy,:visible
  attr_reader :rect,:color,:tone
  def initialize(*args)
    __own(RSXP.call('viewport_new'),'viewport_dispose')
    @rect=args.empty? ? Rect.new(0,0,Graphics.width,Graphics.height) : (args.size==1 ? args.first.clone : Rect.new(*args))
    @z=@ox=@oy=0
    @visible=true
    @color=Color.new(0,0,0,0)
    @tone=Tone.new
    Viewport.instance_variable_get(:@instances) << RSXP::ResourceRef.new(self)
  end
  def rect=(value)
    @rect.set(value)
  end
  def color=(value)
    @color.set(value)
  end
  def tone=(value)
    @tone.set(value)
  end
  def __sync
    __check
    sync_color=__flash_color_for_sync(@color)
    RSXP.sync(self,'viewport_sync',*@rect.to_a,Integer(@ox),Integer(@oy),visible ? 1 : 0,Integer(@z),
      *sync_color.to_a,*[@tone.red,@tone.green,@tone.blue,@tone.gray].map(&:to_i))
  end
end

class Plane
  include NativeResource
  @instances = []
  class << self
    attr_reader :instances
    def __sync_all
      Plane.instances.delete_if do |reference|
        begin
          plane = reference.__getobj__
        rescue RSXP::ResourceRef::RefError
          next true
        end
        plane.__sync unless plane.disposed?
        plane.disposed?
      end
    end
  end
  attr_accessor :viewport,:visible,:z,:ox,:oy,:zoom_x,:zoom_y,:blend_type
  attr_reader :bitmap,:opacity,:color,:tone
  def initialize(viewport=nil)
    raise TypeError, 'expected Viewport or nil' unless viewport.nil? || viewport.is_a?(Viewport)
    __own(RSXP.call('plane_new'),'plane_dispose')
    @viewport=viewport
    @bitmap=nil
    @visible=true
    @z=@ox=@oy=0
    @zoom_x=@zoom_y=1.0
    @opacity=255
    @blend_type=0
    @color=Color.new(0,0,0,0)
    @tone=Tone.new
    Plane.instance_variable_get(:@instances) << RSXP::ResourceRef.new(self)
  end
  def bitmap=(value)
    __check
    raise TypeError, 'expected Bitmap or nil' unless value.nil? || value.is_a?(Bitmap)
    value.__check if value
    @bitmap=value
  end
  def opacity=(value)
    @opacity=RSXP.channel(value)
  end
  def color=(value)
    @color.set(value)
  end
  def tone=(value)
    @tone.set(value)
  end
  def __sync
    __check
    unless [0,1,2].include?(blend_type)
      raise NotImplementedError, "plane blend type #{blend_type} is not supported"
    end
    shown=visible && bitmap && !bitmap.disposed?
    clip=Rect.new(0,0,Graphics.width,Graphics.height)
    if viewport
      shown &&= viewport.visible && !viewport.disposed?
      clip=viewport.rect
    end
    RSXP.sync(self,'plane_sync',bitmap ? bitmap.__id : 0,Integer(ox),Integer(oy),
      Float(zoom_x),Float(zoom_y),shown ? opacity : 0,shown ? 1 : 0,Integer(z),Integer(blend_type),
      *@color.to_a,*[@tone.red,@tone.green,@tone.blue,@tone.gray].map(&:to_i),*clip.to_a,
      viewport ? viewport.__id : 0)
  end
end

class Sprite
  include NativeResource
  include RSXP::Flashable
  @instances = []
  class << self
    attr_reader :instances
    def __sync_all
      Sprite.instances.delete_if do |reference|
        begin
          sprite = reference.__getobj__
        rescue RSXP::ResourceRef::RefError
          next true
        end
        next true if sprite.disposed?
        sprite.__sync
        # Disposal already removes the native resource immediately. If a
        # custom sync disposes itself, prune its weak entry on the next pass.
        false
      end
    end
  end
  attr_accessor :x,:y,:z,:ox,:oy,:zoom_x,:zoom_y,:visible
  attr_accessor :mirror,:blend_type,:angle,:invert
  attr_accessor :bush_depth,:bush_opacity
  attr_accessor :pattern,:pattern_opacity,:pattern_tile
  attr_accessor :pattern_scroll_x,:pattern_scroll_y,:pattern_zoom_x,:pattern_zoom_y
  attr_accessor :pattern_blend_type
  # Wave effects are accepted for compatibility; rsxp does not render them.
  attr_accessor :wave_amp,:wave_length,:wave_speed,:wave_phase
  attr_reader :bitmap,:src_rect,:opacity,:color,:tone
  attr_accessor :viewport
  def initialize(viewport=nil)
    raise TypeError, 'expected Viewport or nil' unless viewport.nil? || viewport.is_a?(Viewport)
    __own(RSXP.call('sprite_new'),'sprite_dispose')
    @x=@y=@z=@ox=@oy=0
    @zoom_x=@zoom_y=1.0
    @visible=true
    @opacity=255
    @viewport=viewport
    @src_rect=Rect.new
    @color=Color.new(0,0,0,0)
    @tone=Tone.new
    @mirror=false
    @blend_type=0
    @angle=0
    @invert=false
    @bush_depth=0
    @bush_opacity=128
    @pattern=nil
    @pattern_opacity=255
    @pattern_tile=true
    @pattern_scroll_x=@pattern_scroll_y=0
    @pattern_zoom_x=@pattern_zoom_y=1.0
    @pattern_blend_type=0
    Sprite.instance_variable_get(:@instances) << RSXP::ResourceRef.new(self)
  end
  def initialize_copy(_other)
    raise TypeError, 'Sprite cannot be copied'
  end
  def bitmap=(value)
    __check
    raise TypeError, 'expected Bitmap or nil' unless value.nil? || value.is_a?(Bitmap)
    value.__check if value
    return if @bitmap.equal?(value)
    @bitmap=value
    @src_rect.set(value ? value.rect : Rect.new)
  end
  def src_rect=(value)
    raise TypeError, 'expected Rect' unless value.is_a?(Rect)
    @src_rect.set(value)
  end
  def opacity=(value)
    @opacity=RSXP.channel(value)
  end
  def color=(value)
    @color.set(value)
  end
  def tone=(value)
    @tone.set(value)
  end
  def width = src_rect.width
  def height = src_rect.height
  def __sync
    __check
    blend=blend_type.nil? ? 0 : blend_type
    if ![0,1,2].include?(blend)
      raise NotImplementedError, "sprite blend type #{blend} is not supported"
    end
    zx,zy=Float(zoom_x || 1.0),Float(zoom_y || 1.0)
    shown=visible && bitmap && !bitmap.disposed?
    clip=Rect.new(0,0,Graphics.width,Graphics.height)
    if viewport
      shown &&= viewport.visible && !viewport.disposed?
      clip=viewport.rect
    end
    if pattern && pattern.disposed?
      raise RGSSError, 'disposed pattern Bitmap'
    end
    sync_color=__flash_color_for_sync(@color)
    RSXP.sync(self,'sprite_sync',bitmap ? bitmap.__id : 0,*src_rect.to_a,
      Integer(x || 0),Integer(y || 0),Integer(ox || 0),Integer(oy || 0),zx,zy,Float(angle || 0),mirror ? 1 : 0,
      opacity || 255,shown ? 1 : 0,*clip.to_a,Integer(z || 0),
      Integer(blend),*sync_color.to_a,*[@tone.red,@tone.green,@tone.blue,@tone.gray].map(&:to_i),invert ? 1 : 0,
      pattern ? pattern.__id : 0,Integer(pattern_blend_type || 0),pattern_tile ? 1 : 0,RSXP.channel(pattern_opacity || 255),
      Integer(pattern_scroll_x || 0),Integer(pattern_scroll_y || 0),Float(pattern_zoom_x || 1.0),Float(pattern_zoom_y || 1.0),
      viewport ? viewport.__id : 0,Integer(bush_depth || 0),RSXP.channel(bush_opacity || 128))
  end
end

class Sprite
  # Keep virtual getter calls on the Ruby side. Desktop submits typed values;
  # the browser serializes only changed sprites. A reusable snapshot
  # of the last submitted state is compared first so an unchanged sprite costs
  # no native call; sprites that are not drawn (no bitmap/viewport) take an even
  # shorter path because their geometry is irrelevant.
  NATIVE_SYNC = private_method_defined?(:__rsxp_sprite_sync)
  if ENV['RSXP_SPRITE_CACHE']!='0'
    def __sync
      __check
      bmp=bitmap
      vp=viewport
      pat=pattern
      bmp_id=bmp ? bmp.__id : 0
      vp_id=vp ? vp.__id : 0
      pat_id=pat ? pat.__id : 0
      bmp_disposed=bmp ? bmp.disposed? : false
      pat_disposed=pat ? pat.disposed? : false
      vis=visible
      shown=vis && bmp && !bmp_disposed
      shown &&= vp.visible && !vp.disposed? if vp
      # A fixed snapshot array is reused in place, so an unchanged sprite costs
      # no native call and no allocation.
      state=@__sync_state
      if state.nil?
        state=@__sync_state=Array.new(43)
      elsif NATIVE_SYNC && !shown && state[0]==shown && state[1]==bmp_id && state[2]==vp_id &&
            state[3]==pat_id && state[4]==bmp_disposed && state[5]==pat_disposed
        # Not drawn: only the objects that could make it drawable again matter.
        return
      end
      blend=blend_type
      zx=zoom_x
      zy=zoom_y
      src=src_rect
      sx=src.x
      sy=src.y
      sw=src.width
      sh=src.height
      px=x
      py=y
      pox=ox
      poy=oy
      pang=angle
      pmirror=mirror
      pz=z
      pop=opacity
      pinv=invert
      pblend=pattern_blend_type
      ptile=pattern_tile
      popac=pattern_opacity
      psx=pattern_scroll_x
      psy=pattern_scroll_y
      pzx=pattern_zoom_x
      pzy=pattern_zoom_y
      bush=bush_depth
      bush_alpha=bush_opacity
      # Plugins can leave a sprite property unset. RGSS reads missing numeric
      # properties as their defaults, so coerce nil instead of failing the frame.
      blend=0 if blend.nil?
      zx=1.0 if zx.nil?
      zy=1.0 if zy.nil?
      vis=false if vis.nil?
      px=0 if px.nil?
      py=0 if py.nil?
      pox=0 if pox.nil?
      poy=0 if poy.nil?
      pang=0 if pang.nil?
      pmirror=false if pmirror.nil?
      pz=0 if pz.nil?
      pop=255 if pop.nil?
      pinv=false if pinv.nil?
      pblend=0 if pblend.nil?
      ptile=false if ptile.nil?
      popac=255 if popac.nil?
      psx=0 if psx.nil?
      psy=0 if psy.nil?
      pzx=1.0 if pzx.nil?
      pzy=1.0 if pzy.nil?
      bush=0 if bush.nil?
      bush_alpha=128 if bush_alpha.nil?
      if vp
        clip=vp.rect
        clip_x=clip.x
        clip_y=clip.y
        clip_w=clip.width
        clip_h=clip.height
      else
        clip_x=0
        clip_y=0
        clip_w=Graphics.width
        clip_h=Graphics.height
      end
      sync_color=__flash_color_for_sync(@color)
      cr=sync_color.red
      cg=sync_color.green
      cb=sync_color.blue
      ca=sync_color.alpha
      tone=@tone
      tr=tone.red
      tg=tone.green
      tb=tone.blue
      tgray=tone.gray
      return if state[0]==shown && state[1]==bmp_id && state[2]==vp_id && state[3]==pat_id &&
                state[4]==bmp_disposed && state[5]==pat_disposed && state[6]==blend &&
                state[7]==clip_x && state[8]==clip_y && state[9]==clip_w && state[10]==clip_h &&
                state[11]==sx && state[12]==sy && state[13]==sw && state[14]==sh &&
                state[15]==px && state[16]==py && state[17]==pox && state[18]==poy &&
                state[19]==zx && state[20]==zy && state[21]==pang && state[22]==pmirror &&
                state[23]==pz && state[24]==pop && state[25]==pinv &&
                state[26]==pblend && state[27]==ptile && state[28]==popac &&
                state[29]==psx && state[30]==psy && state[31]==pzx && state[32]==pzy &&
                state[33]==cr && state[34]==cg && state[35]==cb && state[36]==ca &&
                state[37]==tr && state[38]==tg && state[39]==tb && state[40]==tgray && state[41]==bush && state[42]==bush_alpha
      if NATIVE_SYNC
        __rsxp_sprite_sync(self,__id,blend,zx,zy,bmp,vp,pat,vis,
          sx,sy,sw,sh,px,py,pox,poy,pang,pmirror,pz,pop,pinv,
          pblend,ptile,popac,psx,psy,pzx,pzy,cr,cg,cb,ca,tr,tg,tb,tgray,bush,bush_alpha)
      else
        unless blend==0 || blend==1 || blend==2
          raise NotImplementedError, "sprite blend type #{blend} is not supported"
        end
        native_zx,native_zy=Float(zx),Float(zy)
        raise RGSSError, 'disposed pattern Bitmap' if pat_disposed
        # Only changed sprites allocate/convert a command. The snapshot above
        # already performs the comparison, so no second RSXP.sync Array is needed.
        RSXP::SYNC_QUEUE << ['sprite_sync',__id,bmp_id,Integer(sx),Integer(sy),Integer(sw),Integer(sh),
          Integer(px),Integer(py),Integer(pox),Integer(poy),native_zx,native_zy,Float(pang),pmirror ? 1 : 0,
          pop,shown ? 1 : 0,clip_x,clip_y,clip_w,clip_h,Integer(pz),Integer(blend),
          cr.to_i,cg.to_i,cb.to_i,ca.to_i,tr.to_i,tg.to_i,tb.to_i,tgray.to_i,pinv ? 1 : 0,
          pat_id,Integer(pblend),ptile ? 1 : 0,RSXP.channel(popac),Integer(psx),Integer(psy),
          Float(pzx),Float(pzy),vp_id,Integer(bush),RSXP.channel(bush_alpha)].join("\t").b
      end
      state[0]=shown
      state[1]=bmp_id
      state[2]=vp_id
      state[3]=pat_id
      state[4]=bmp_disposed
      state[5]=pat_disposed
      state[6]=blend
      state[7]=clip_x
      state[8]=clip_y
      state[9]=clip_w
      state[10]=clip_h
      state[11]=sx
      state[12]=sy
      state[13]=sw
      state[14]=sh
      state[15]=RSXP.snapshot_number(px)
      state[16]=RSXP.snapshot_number(py)
      state[17]=RSXP.snapshot_number(pox)
      state[18]=RSXP.snapshot_number(poy)
      state[19]=RSXP.snapshot_number(zx)
      state[20]=RSXP.snapshot_number(zy)
      state[21]=RSXP.snapshot_number(pang)
      state[22]=pmirror
      state[23]=RSXP.snapshot_number(pz)
      state[24]=RSXP.snapshot_number(pop)
      state[25]=pinv
      state[26]=RSXP.snapshot_number(pblend)
      state[27]=ptile
      state[28]=RSXP.snapshot_number(popac)
      state[29]=RSXP.snapshot_number(psx)
      state[30]=RSXP.snapshot_number(psy)
      state[31]=RSXP.snapshot_number(pzx)
      state[32]=RSXP.snapshot_number(pzy)
      state[33]=cr
      state[34]=cg
      state[35]=cb
      state[36]=ca
      state[37]=tr
      state[38]=tg
      state[39]=tb
      state[40]=tgray
      state[41]=RSXP.snapshot_number(bush)
      state[42]=RSXP.snapshot_number(bush_alpha)
    end
  end
end

# RGSS1 tilemap. Visible cells use the same native tile sprites as Essentials;
# map ownership and animation remain here, atlas caching and rendering in Rust.
class Tilemap
  AUTOTILE_PARTS = [
     [ [27, 28, 33, 34], [ 5, 28, 33, 34], [27,  6, 33, 34], [ 5,  6, 33, 34],
       [27, 28, 33, 12], [ 5, 28, 33, 12], [27,  6, 33, 12], [ 5,  6, 33, 12] ],
     [ [27, 28, 11, 34], [ 5, 28, 11, 34], [27,  6, 11, 34], [ 5,  6, 11, 34],
       [27, 28, 11, 12], [ 5, 28, 11, 12], [27,  6, 11, 12], [ 5,  6, 11, 12] ],
     [ [25, 26, 31, 32], [25,  6, 31, 32], [25, 26, 31, 12], [25,  6, 31, 12],
       [15, 16, 21, 22], [15, 16, 21, 12], [15, 16, 11, 22], [15, 16, 11, 12] ],
     [ [29, 30, 35, 36], [29, 30, 11, 36], [ 5, 30, 35, 36], [ 5, 30, 11, 36],
       [39, 40, 45, 46], [ 5, 40, 45, 46], [39,  6, 45, 46], [ 5,  6, 45, 46] ],
     [ [25, 30, 31, 36], [15, 16, 45, 46], [13, 14, 19, 20], [13, 14, 19, 12],
       [17, 18, 23, 24], [17, 18, 11, 24], [41, 42, 47, 48], [ 5, 42, 47, 48] ],
     [ [37, 38, 43, 44], [37,  6, 43, 44], [13, 18, 19, 24], [13, 14, 43, 44],
       [37, 42, 43, 48], [17, 18, 47, 48], [13, 18, 43, 48], [ 1,  2,  7,  8] ]
  ].flatten(1).map(&:freeze).freeze
  @instances=[]
  class << self
    attr_reader :instances
    def __sync_all
      instances.delete_if do |reference|
        begin
          tilemap=reference.__getobj__
        rescue RSXP::ResourceRef::RefError
          next true
        end
        next true if tilemap.disposed?
        tilemap.__sync
        false
      end
    end
  end
  attr_accessor :tileset,:map_data,:flash_data,:priorities,:visible,:ox,:oy
  attr_reader :viewport,:autotiles,:color,:tone
  def initialize(viewport=nil)
    @viewport=viewport
    @tileset=@map_data=@flash_data=@priorities=nil
    @autotiles=Array.new(7)
    @visible=true
    @disposed=false
    @ox=@oy=@animation=0
    @color=Color.new(0,0,0,0)
    @tone=Tone.new
    @sprites=[]
    Tilemap.instances << RSXP::ResourceRef.new(self)
  end
  def color=(value)
    @color.set(value)
  end
  def tone=(value)
    @tone.set(value)
  end
  def disposed? = @disposed
  def dispose
    return if @disposed
    @sprites.each(&:dispose)
    @sprites.clear
    @disposed=true
  end
  def update
    raise RGSSError, 'disposed Tilemap' if disposed?
    @animation+=1
  end
  def __sync
    internal=!!(defined?(RSXP::TilemapSprite) && ENV['RSXP_RETAINED_TILEMAP']!='0' &&
      (!defined?($ResizeFactorMul) || $ResizeFactorMul==100) &&
      (@viewport || ((!defined?($ResizeOffsetX) || $ResizeOffsetX==0) &&
        (!defined?($ResizeOffsetY) || $ResizeOffsetY==0))))
    if @__rsxp_internal_mode!=internal
      @sprites.each(&:dispose)
      @sprites.clear
      @__rsxp_internal_mode=internal
    end
    @__rsxp_animated_tiles=[] if internal
    @__rsxp_tile_plan=internal && @__rsxp_batch_tiles ? [] : nil
    used=0
    if @visible && @map_data && (!@viewport || !@viewport.disposed?)
      width=@viewport ? @viewport.rect.width : Graphics.width
      height=@viewport ? @viewport.rect.height : Graphics.height
      left=[@ox.div(32),0].max
      top=[@oy.div(32),0].max
      right=[(@ox+width-1).div(32),@map_data.xsize-1].min
      bottom=[(@oy+height-1).div(32),@map_data.ysize-1].min
      @map_data.zsize.times do |layer|
        (top..bottom).each do |y|
          (left..right).each do |x|
            id=@map_data[x,y,layer]
            next if id<48
            priority=@priorities ? @priorities[id].to_i : 0
            @__rsxp_tile_priority=priority!=0
            @__rsxp_tile_animation=nil
            z=priority==0 ? 0 : (y+priority)*32+32-@oy
            if id>=384
              next unless @tileset && !@tileset.disposed?
              used=__tile(used,@tileset,(id-384)%8*32,(id-384)/8*32,32,
                x*32-@ox,y*32-@oy,z)
            else
              bitmap=@autotiles[id/48-1]
              next unless bitmap && !bitmap.disposed?
              if bitmap.height==32
                @__rsxp_tile_animation=[32,bitmap.width/32] if internal && bitmap.width>32
                used=__tile(used,bitmap,(@animation/16%[bitmap.width/32,1].max)*32,0,32,
                  x*32-@ox,y*32-@oy,z)
              else
                @__rsxp_tile_animation=[96,bitmap.width/96] if internal && bitmap.width>96
                frame=(@animation/16%[bitmap.width/96,1].max)*96
                AUTOTILE_PARTS[id%48].each_with_index do |part,index|
                  part-=1
                  used=__tile(used,bitmap,frame+part%6*16,part/6*16,16,
                    x*32-@ox+index%2*16,y*32-@oy+index/2*16,z)
                end
              end
            end
          end
        end
      end
    end
    if @__rsxp_tile_plan
      clip=@viewport ? @viewport.rect.to_a : [0,0,Graphics.width,Graphics.height]
      plan=@__rsxp_tile_plan
      @__rsxp_tile_plan=nil
      unless __rsxp_apply_rgss_tiles(@sprites,plan,@color.to_a,
          [@tone.red,@tone.green,@tone.blue,@tone.gray],@viewport,clip)
        plan.each_with_index do |row,index|
          @__rsxp_tile_priority=row[7]
          @__rsxp_tile_animation=nil
          __tile(index,*row[0,7])
        end
        @sprites.drop(used).each { |sprite| sprite.visible=false }
      end
    else
      @sprites.drop(used).each { |sprite| sprite.visible=false }
    end
  end
  def __tile(index,bitmap,sx,sy,size,x,y,z)
    sprite=@sprites[index]
    unless sprite
      klass=@__rsxp_internal_mode ? RSXP::TilemapSprite : Sprite
      sprite=klass.new(@viewport)
      RSXP.call('sprite_tile',sprite.__id)
      @sprites[index]=sprite
    end
    if @__rsxp_tile_plan
      @__rsxp_tile_plan << [bitmap,sx,sy,size,x,y,z,@__rsxp_tile_priority]
      if @__rsxp_tile_animation
        step,frames=@__rsxp_tile_animation
        @__rsxp_animated_tiles << [sprite,sx-(@animation/16%frames)*step,step,frames]
      end
      return index+1
    end
    sprite.instance_variable_set(:@__rsxp_rgss_priority,@__rsxp_tile_priority)
    sprite.bitmap=bitmap
    sprite.src_rect.set(sx,sy,size,size)
    sprite.x=x
    sprite.y=y
    sprite.z=z
    sprite.visible=true
    sprite.color=@color
    sprite.tone=@tone
    if @__rsxp_internal_mode && @__rsxp_tile_animation
      step,frames=@__rsxp_tile_animation
      @__rsxp_animated_tiles << [sprite,sx-(@animation/16%frames)*step,step,frames]
    end
    index+1
  end
end

module Graphics
  def self.fullscreen = RSXP.call('window_fullscreen')=='1'
  def self.fullscreen=(value)
    RSXP.call('window_fullscreen_set',value ? 1 : 0)
  end
  def self.scale = RSXP.call('window_scale').to_f
  def self.scale=(value)
    RSXP.call('window_scale_set',Float(value))
  end
  def self.center = RSXP.call('window_center')
  def self.show_cursor=(value)
    RSXP.call('window_cursor',value ? 1 : 0)
  end
  def self.freeze = RSXP.call('graphics_freeze')
  def self.transition(duration=8,filename='',vague=40)
    raise NotImplementedError,'pattern transitions are not ported yet' unless filename.to_s.empty?
    frames=[Integer(duration),0].max
    RSXP.call('graphics_transition',frames)
    frames.times {update}
    nil
  end
  def self.brightness = RSXP.call('graphics_brightness').to_i
  def self.brightness=(value)
    RSXP.call('graphics_brightness_set',RSXP.channel(value))
  end
  def self.fadeout(duration)
    frames=[Integer(duration),1].max
    start=brightness
    frames.times { |i| self.brightness=start*(frames-i-1)/frames;update }
  end
  def self.fadein(duration)
    frames=[Integer(duration),1].max
    start=brightness
    frames.times { |i| self.brightness=start+(255-start)*(i+1)/frames;update }
  end
  def self.frame_reset = RSXP.call('graphics_frame_reset')
  def self.width = (@__size ||= RSXP.call('graphics_size').split("\t").map!(&:to_i))[0]
  def self.height = (@__size ||= RSXP.call('graphics_size').split("\t").map!(&:to_i))[1]
  def self.frame_count = RSXP.call('graphics_frame').to_i
  def self.frame_count=(value)
    RSXP.call('graphics_frame_set',Integer(value))
  end
  def self.frame_rate = RSXP.call('graphics_rate').to_i
  def self.frame_rate=(value)
    RSXP.call('graphics_rate_set',Integer(value))
  end
  def self.resize_screen(width,height)
    result=RSXP.call('graphics_resize',Integer(width),Integer(height))
    @__size=nil
    result
  end
  # Resizes only the logical render target, not the native window. The host
  # scales the target to fit, so a larger target shows more of the scene at a
  # smaller scale. Used by the "map panoramic mode" runtime patch.
  def self.render_size=(size)
    RSXP.call('graphics_resolution_set',Integer(size[0]),Integer(size[1]))
    @__size=nil
    nil
  end
  def self.update
    now=Time.now.to_f
    if @last_update_time
      elapsed=now-@last_update_time
      if elapsed>0.0
        @delta=elapsed
        @average_frame_rate=(@average_frame_rate || frame_rate.to_f)*0.9+(1.0/elapsed)*0.1
      end
    end
    @last_update_time=now
    Viewport.__sync_all
    Plane.__sync_all
    Tilemap.__sync_all
    Sprite.__sync_all
    RSXP.call('graphics_update')
    nil
  end
  def self.wait(frames)
    [Integer(frames),0].max.times { update }
    nil
  end
  def self.snap_to_bitmap = Bitmap.__from_id(RSXP.call('graphics_snap'))
  # Real time since the previous update, in seconds (mkxp-z Graphics.delta).
  def self.delta = (@delta || 0.0)
  def self.average_frame_rate = (@average_frame_rate || frame_rate.to_f)
  def self.display_width = width
  def self.display_height = height
  def self.resize_window(width,height)
    resize_screen(Integer(width),Integer(height))
    nil
  end
  def self.screenshot(filename)
    bitmap=snap_to_bitmap
    begin
      bitmap.to_file(filename)
    ensure
      bitmap.dispose
    end
    nil
  end
  def self.frameskip = (@frameskip || false)
  def self.frameskip=(value)
    @frameskip=value ? true : false
  end
  def self.threadsafe = (@threadsafe || false)
  def self.threadsafe=(value)
    @threadsafe=value ? true : false
  end
  # Plays a Theora (.ogv) movie centred and scaled to the screen, blocking until
  # it ends or (when skippable) the player cancels. Video decodes in the engine
  # on every platform, so no system codec is required.
  def self.play_movie(filename,volume=100,skippable=false)
    info=begin
      RSXP.call('movie_open',Integer(volume),skippable ? 1 : 0,payload:String(filename))
    rescue StandardError
      nil
    end
    return if info.nil? || info.empty?
    id,width,height=info.split("\t").map!(&:to_i)
    return if id.to_i<=0
    width=[width,1].max
    height=[height,1].max
    bitmap=Bitmap.new(width,height)
    sprite=Sprite.new
    sprite.bitmap=bitmap
    sprite.z=5001
    ratio=[Graphics.width.to_f/width,Graphics.height.to_f/height].min
    sprite.zoom_x=ratio
    sprite.zoom_y=ratio
    sprite.x=(Graphics.width/2)-(width*ratio/2)
    sprite.y=(Graphics.height/2)-(height*ratio/2)
    begin
      loop do
        status=RSXP.call('movie_frame',id,bitmap.__id).to_s
        update
        break if status=='0'
        break if skippable && (Input.trigger?(Input::C) || Input.trigger?(Input::B))
      end
    ensure
      sprite.dispose
      bitmap.dispose
      RSXP.call('movie_close',id)
    end
    nil
  end
end

# Runtime patches are small programs the workspace UI can turn on while the
# game is running. The engine owns the catalog, the enabled set and each patch's
# optional numeric parameter; the shim polls it each frame, fetches a patch's
# source the first time it is enabled, evaluates it and calls the returned
# `apply`/`revert`/`value` procs on change. This keeps patching out of the hot
# path: nothing runs until a patch is enabled.
module RuntimePatches
  @patches={}
  @active={}
  @values={}
  @failed={}
  def self.sync
    return if @syncing
    @syncing=true
    begin
      RSXP.call('runtime_patch_states').each_line do |line|
        id,payload=line.strip.split('=',2)
        next if id.nil? || id.empty?
        next if @failed[id]
        enabled,value=payload.to_s.split("\t",2)
        wanted=(enabled=='1')
        value=(value.nil? || value.empty?) ? nil : value.to_f
        active=(@active[id]==true)
        if wanted
          patch=@patches.fetch(id) { @patches[id]=load_patch(id) || false }
          next if patch==false
          # A patch that raises is marked failed and not retried, so a broken
          # patch cannot spam the log every frame.
          begin
            if active
              if value && @values[id]!=value
                patch[:value].call(value) if patch[:value]
                @values[id]=value
              end
            else
              patch[:value].call(value) if patch[:value] && value
              patch[:apply].call
              @active[id]=true
              @values[id]=value
              RSXP::ERROR_OUTPUT.call("[RuntimePatches] enabled #{id}\n")
            end
          rescue StandardError => error
            @failed[id]=true
            RSXP::ERROR_OUTPUT.call("[RuntimePatches] #{id}: #{error.class}: #{error.message}\n")
          end
        elsif active
          patch=@patches[id]
          next if patch.nil? || patch==false
          begin
            patch[:revert].call
            @active[id]=false
            RSXP::ERROR_OUTPUT.call("[RuntimePatches] disabled #{id}\n")
          rescue StandardError => error
            @failed[id]=true
            RSXP::ERROR_OUTPUT.call("[RuntimePatches] #{id}: #{error.class}: #{error.message}\n")
          end
        end
      end
    rescue StandardError => error
      RSXP::ERROR_OUTPUT.call("[RuntimePatches] #{error.class}: #{error.message}\n")
    ensure
      @syncing=false
    end
    nil
  end
  def self.load_patch(id)
    source=RSXP.call('runtime_patch_source',id)
    return nil if source.nil? || source.empty?
    patch=eval(source.dup.force_encoding(Encoding::UTF_8),TOPLEVEL_BINDING,"runtime patch #{id}",1)
    return nil unless patch.is_a?(Hash) && patch[:apply].respond_to?(:call) && patch[:revert].respond_to?(:call)
    patch
  end
  def self.active?(id) = @active[id]==true
end

module Input
  @state={}
  def self.__button(scan, flag) = (@state.fetch(scan,0) & flag)!=0
  # rsxp exposes the raw SDL scancode table (one boolean per scancode); the
  # Win32API GetKeyState/GetAsyncKeyState shim reads it.
  def self.raw_key_states
    states=Array.new(512,false)
    @state.each { |scan,flags| states[scan]=(flags&1)!=0 if scan>=0 && scan<512 }
    states
  end
  def self.raw_keys(key)
    return [Integer(key)] unless key.is_a?(Symbol)
    name=key.to_s
    return [name.ord-'A'.ord+4] if /\A[A-Z]\z/.match?(name)
    return [58+name[1..].to_i-1] if /\AF(?:[1-9]|1[0-2])\z/.match?(name)
    code={RETURN:40,ESCAPE:41,BACKSPACE:42,TAB:43,SPACE:44,GRAVE:53,UP:82,DOWN:81,LEFT:80,RIGHT:79,LSHIFT:225,RSHIFT:229,LCTRL:224,RCTRL:228,LALT:226,RALT:230}[key]
    # rsxp resolves unknown scancode names to 0 (SDL_SCANCODE_UNKNOWN), which
    # is never held, rather than raising.
    [code || 0]
  end
  def self.pressex?(key) = raw_keys(key).any? { |scan| __button(scan,1) }
  def self.triggerex?(key) = raw_keys(key).any? { |scan| __button(scan,2) }
  def self.releaseex?(key) = raw_keys(key).any? { |scan| __button(scan,4) }
  def self.repeatex?(key) = raw_keys(key).any? { |scan| __button(scan,8) }
  MOUSELEFT=1001;MOUSEMIDDLE=1002;MOUSERIGHT=1003;MOUSEX1=1004;MOUSEX2=1005
  DOWN=2; LEFT=4; RIGHT=6; UP=8
  A=11; B=12; C=13; X=14; Y=15; Z=16; L=17; R=18
  SHIFT=21; CTRL=22; ALT=23; F5=25; F6=26; F7=27; F8=28; F9=29
  KEYS={DOWN=>[81],LEFT=>[80],RIGHT=>[79],UP=>[82],A=>[225,229],B=>[27,41],C=>[29,40,44],X=>[4],Y=>[22],Z=>[7],L=>[20,75],R=>[26,78],SHIFT=>[225,229],CTRL=>[224,228],ALT=>[226,230],F5=>[62],F6=>[63],F7=>[64],F8=>[65],F9=>[66],MOUSELEFT=>[-1],MOUSEMIDDLE=>[-2],MOUSERIGHT=>[-3],MOUSEX1=>[-4],MOUSEX2=>[-5]}.freeze
  # Virtual controller actions are independent of physical keyboard state.
  [DOWN,LEFT,RIGHT,UP,A,B,C,X,Y,Z,L,R].each_with_index { |key,i| KEYS[key] << 600+i }
  @held=Hash.new(0)
  def self.code(value)
    # Legacy plugins use nil to disable an optional action binding.
    return 0 if value.nil?
    value.is_a?(Symbol) ? const_get(value) : Integer(value)
  end
  def self.update
    RSXP::GameAPI.poll if defined?(RSXP::GameAPI)
    bytes=RSXP.call('input_poll')
    @state=bytes.unpack('l<C'*(bytes.bytesize/5)).each_slice(2).to_h
    RSXP::GameAPI.inject_input(@state) if defined?(RSXP::GameAPI)
    KEYS.each_key { |key| @held[key]=press?(key) ? @held[key]+1 : 0 }
    RuntimePatches.sync
    nil
  end
  def self.press?(key) = KEYS.fetch(code(key),[]).any? { |scan| __button(scan,1) }
  def self.trigger?(key) = KEYS.fetch(code(key),[]).any? { |scan| __button(scan,2) }
  def self.release?(key) = KEYS.fetch(code(key),[]).any? { |scan| __button(scan,4) }
  def self.time?(key) = @held[code(key)]*1_000_000/Graphics.frame_rate
  def self.mouse_x = RSXP.call('input_mouse').split("\t")[0].to_i
  def self.mouse_y = RSXP.call('input_mouse').split("\t")[1].to_i
  def self.mouse_in_window? = RSXP.call('input_mouse').split("\t")[2]=='1'
  def self.scroll_v = RSXP.call('input_mouse').split("\t")[3].to_i
  def self.text_input = RSXP.call('input_text_mode')=='1'
  def self.text_input=(value)
    RSXP.call('input_text_mode_set',value ? 1 : 0)
  end
  def self.gets = RSXP.call('input_gets')
  def self.repeat?(key)
    count=@held[code(key)]
    count==1 || (count>=24 && count%6==0)
  end
  def self.dir4
    [DOWN,LEFT,RIGHT,UP].find { |key| press?(key) } || 0
  end
  def self.dir8
    x=(press?(RIGHT) ? 1 : 0)-(press?(LEFT) ? 1 : 0)
    y=(press?(DOWN) ? 1 : 0)-(press?(UP) ? 1 : 0)
    x==0 && y==0 ? 0 : 5+x-y*3
  end
  # OS clipboard integration is unavailable; keep an in-process buffer so
  # scripts that copy/paste inside the game still work.
  def self.clipboard = (@clipboard || ''.dup)
  def self.clipboard=(value)
    @clipboard = value.nil? ? nil : value.to_s
  end
  # Hosts publish controller presence alongside virtual action scancodes.
  def self.connected?(index = 0) = (0...16).cover?(index) && __button(620+index,1)
  def self.power_level(_index = 0) = 0
  def self.axes_left(_index = 0) = 0
  def self.axes_right(_index = 0) = 0
  def self.axes_trigger(_index = 0) = 0
  def self.raw_axes(_index = 0) = 0
  def self.raw_button_states(_index = 0) = 0
  def self.count = (0...16).count { |index| connected?(index) }
  module Controller
    def self.connected?(index = 0) = Input.connected?(index)
    def self.name(index = 0) = connected?(index) ? "Game controller" : nil
    def self.power_level(_index = 0) = 0
    def self.axes_left(_index = 0) = 0
    def self.axes_right(_index = 0) = 0
    def self.axes_trigger(_index = 0) = 0
    def self.raw_axes(_index = 0) = 0
    def self.raw_button_states(_index = 0) = 0
    def self.count = Input.count
  end
end

module Kernel
  # Modern Ruby rejects `class Foo < Foo` (superclass mismatch) and would raise
  # a NameError if Foo is undefined. Some Essentials plugins contain this typo
  # while intending to reopen the class, so repair it in memory as legacy RGSS
  # games expect.
  def load_data(path)
    data=Marshal.load(RSXP.call('read',payload:String(path)))
    if path.to_s.tr('\\','/').downcase=='data/pluginscripts.rxdata' && data.is_a?(Array)
      data.each do |entry|
        next unless entry.is_a?(Array) && entry[2].is_a?(Array)
        entry[2].each do |script|
          next unless script.is_a?(Array) && script[1].is_a?(String)
          begin
            source=Zlib.inflate(script[1])
          rescue Zlib::Error
            next
          end
          fixed=source.gsub(/^([ \t]*)class ([A-Z]\w*(?:::\w+)*) < \2[ \t]*\r?$/) { "#{$1}class #{$2}" }
          next if fixed==source
          script[1]=Zlib.deflate(fixed)
          RSXP::ERROR_OUTPUT.call("Repaired redundant class self-inheritance in #{script[0]}, in memory.\n")
        end
      end
    end
    data
  end
  def save_data(object,path)
    File.binwrite(path,Marshal.dump(object))
    nil
  end
  def rgss_main
    begin
      yield
    rescue RGSSReset
      retry
    end
  end
  def rgss_stop
    loop { Graphics.update }
  end
  def msgbox(*messages) = puts(*messages)
  def msgbox_p(*messages) = p(*messages)
end

module Scancodes
	SDL = { :UNKNOWN => 0x00,
		:A => 0x04, :B => 0x05, :C => 0x06, :D => 0x07,
		:E => 0x08, :F => 0x09, :G => 0x0A, :H => 0x0B,
		:I => 0x0C, :J => 0x0D, :K => 0x0E, :L => 0x0F,
		:M => 0x10, :N => 0x11, :O => 0x12, :P => 0x13,
		:Q => 0x14, :R => 0x15, :S => 0x16, :T => 0x17,
		:U => 0x18, :V => 0x19, :W => 0x1A, :X => 0x1B,
		:Y => 0x1C, :Z => 0x1D, :N1 => 0x1E, :N2 => 0x1F,
		:N3 => 0x20, :N4 => 0x21, :N5 => 0x22, :N6 => 0x23,
		:N7 => 0x24, :N8 => 0x25, :N9 => 0x26, :N0 => 0x27,
		:RETURN => 0x28, :ESCAPE => 0x29, :BACKSPACE => 0x2A, :TAB => 0x2B,
		:SPACE => 0x2C, :MINUS => 0x2D, :EQUALS => 0x2E, :LEFTBRACKET => 0x2F,
		:RIGHTBRACKET => 0x30, :BACKSLASH => 0x31, :NONUSHASH => 0x32, :SEMICOLON => 0x33,
		:APOSTROPHE => 0x34, :GRAVE => 0x35, :COMMA => 0x36, :PERIOD => 0x37,
		:SLASH => 0x38, :CAPSLOCK => 0x39, :F1 => 0x3A, :F2 => 0x3B,
		:F3 => 0x3C, :F4 => 0x3D, :F5 => 0x3E, :F6 => 0x3F,
		:F7 => 0x40, :F8 => 0x41, :F9 => 0x42, :F10 => 0x43,
		:F11 => 0x44, :F12 => 0x45, :PRINTSCREEN => 0x46, :SCROLLLOCK => 0x47,
		:PAUSE => 0x48, :INSERT => 0x49, :HOME => 0x4A, :PAGEUP => 0x4B,
		:DELETE => 0x4C, :END => 0x4D, :PAGEDOWN => 0x4E, :RIGHT => 0x4F,
		:LEFT => 0x50, :DOWN => 0x51, :UP => 0x52, :NUMLOCKCLEAR => 0x53,
		:KP_DIVIDE => 0x54, :KP_MULTIPLY => 0x55, :KP_MINUS => 0x56, :KP_PLUS => 0x57,
		:KP_ENTER => 0x58, :KP_1 => 0x59, :KP_2 => 0x5A, :KP_3 => 0x5B,
		:KP_4 => 0x5C, :KP_5 => 0x5D, :KP_6 => 0x5E, :KP_7 => 0x5F,
		:KP_8 => 0x60, :KP_9 => 0x61, :KP_0 => 0x62, :KP_PERIOD => 0x63,
		:NONUSBACKSLASH => 0x64, :APPLICATION => 0x65, :POWER => 0x66, :KP_EQUALS => 0x67,
		:F13 => 0x68, :F14 => 0x69, :F15 => 0x6A, :F16 => 0x6B,
		:F17 => 0x6C, :F18 => 0x6D, :F19 => 0x6E, :F20 => 0x6F,
		:F21 => 0x70, :F22 => 0x71, :F23 => 0x72, :F24 => 0x73,
		:EXECUTE => 0x74, :HELP => 0x75, :MENU => 0x76, :SELECT => 0x77,
		:STOP => 0x78, :AGAIN => 0x79, :UNDO => 0x7A, :CUT => 0x7B,
		:COPY => 0x7C, :PASTE => 0x7D, :FIND => 0x7E, :MUTE => 0x7F,
		:VOLUMEUP => 0x80, :VOLUMEDOWN => 0x81, :LOCKINGCAPSLOCK => 0x82, :LOCKINGNUMLOCK => 0x83,
		:LOCKINGSCROLLLOCK => 0x84, :KP_COMMA => 0x85, :KP_EQUALSAS400 => 0x86, :INTERNATIONAL1 => 0x87,
		:INTERNATIONAL2 => 0x88, :INTERNATIONAL3 => 0x89, :INTERNATIONAL4 => 0x8A, :INTERNATIONAL5 => 0x8B,
		:INTERNATIONAL6 => 0x8C, :INTERNATIONAL7 => 0x8D, :INTERNATIONAL8 => 0x8E, :INTERNATIONAL9 => 0x8F,
		:LANG1 => 0x90, :LANG2 => 0x91, :LANG3 => 0x92, :LANG4 => 0x93,
		:LANG5 => 0x94, :LANG6 => 0x95, :LANG7 => 0x96, :LANG8 => 0x97,
		:LANG9 => 0x98, :ALTERASE => 0x99, :SYSREQ => 0x9A, :CANCEL => 0x9B,
		:CLEAR => 0x9C, :PRIOR => 0x9D, :RETURN2 => 0x9E, :SEPARATOR => 0x9F,
		:OUT => 0xA0, :OPER => 0xA1, :CLEARAGAIN => 0xA2, :CRSEL => 0xA3,
		:EXSEL => 0xA4, :KP_00 => 0xB0, :KP_000 => 0xB1, :THOUSANDSSEPARATOR => 0xB2,
		:DECIMALSEPARATOR => 0xB3, :CURRENCYUNIT => 0xB4, :CURRENCYSUBUNIT => 0xB5, :KP_LEFTPAREN => 0xB6,
		:KP_RIGHTPAREN => 0xB7, :KP_LEFTBRACE => 0xB8, :KP_RIGHTBRACE => 0xB9, :KP_TAB => 0xBA,
		:KP_BACKSPACE => 0xBB, :KP_A => 0xBC, :KP_B => 0xBD, :KP_C => 0xBE,
		:KP_D => 0xBF, :KP_E => 0xC0, :KP_F => 0xC1, :KP_XOR => 0xC2,
		:KP_POWER => 0xC3, :KP_PERCENT => 0xC4, :KP_LESS => 0xC5, :KP_GREATER => 0xC6,
		:KP_AMPERSAND => 0xC7, :KP_DBLAMPERSAND => 0xC8, :KP_VERTICALBAR => 0xC9, :KP_DBLVERTICALBAR => 0xCA,
		:KP_COLON => 0xCB, :KP_HASH => 0xCC, :KP_SPACE => 0xCD, :KP_AT => 0xCE,
		:KP_EXCLAM => 0xCF, :KP_MEMSTORE => 0xD0, :KP_MEMRECALL => 0xD1, :KP_MEMCLEAR => 0xD2,
		:KP_MEMADD => 0xD3, :KP_MEMSUBTRACT => 0xD4, :KP_MEMMULTIPLY => 0xD5, :KP_MEMDIVIDE => 0xD6,
		:KP_PLUSMINUS => 0xD7, :KP_CLEAR => 0xD8, :KP_CLEARENTRY => 0xD9, :KP_BINARY => 0xDA,
		:KP_OCTAL => 0xDB, :KP_DECIMAL => 0xDC, :KP_HEXADECIMAL => 0xDD, :LCTRL => 0xE0,
		:LSHIFT => 0xE1, :LALT => 0xE2, :LGUI => 0xE3, :RCTRL => 0xE4,
		:RSHIFT => 0xE5, :RALT => 0xE6, :RGUI => 0xE7, :MODE => 0x101,
		:AUDIONEXT => 0x102, :AUDIOPREV => 0x103, :AUDIOSTOP => 0x104, :AUDIOPLAY => 0x105,
		:AUDIOMUTE => 0x106, :MEDIASELECT => 0x107, :WWW => 0x108, :MAIL => 0x109,
		:CALCULATOR => 0x10A, :COMPUTER => 0x10B, :AC_SEARCH => 0x10C, :AC_HOME => 0x10D,
		:AC_BACK => 0x10E, :AC_FORWARD => 0x10F, :AC_STOP => 0x110, :AC_REFRESH => 0x111,
		:AC_BOOKMARKS => 0x112, :BRIGHTNESSDOWN => 0x113, :BRIGHTNESSUP => 0x114, :DISPLAYSWITCH => 0x115,
		:KBDILLUMTOGGLE => 0x116, :KBDILLUMDOWN => 0x117, :KBDILLUMUP => 0x118, :EJECT => 0x119,
		:SLEEP => 0x11A, :APP1 => 0x11B, :APP2 => 0x11C
	}

	SDL.default = SDL[:UNKNOWN]

	WIN32 = {
		:LBUTTON => 0x01, :RBUTTON => 0x02, :MBUTTON => 0x04,

		:BACK => 0x08, :TAB => 0x09, :RETURN => 0x0D, :SHIFT => 0x10,
		:CONTROL => 0x11, :MENU => 0x12, :PAUSE => 0x13, :CAPITAL => 0x14,
		:ESCAPE => 0x1B, :SPACE => 0x20, :PRIOR => 0x21, :NEXT => 0x22,
		:END => 0x23, :HOME => 0x24, :LEFT => 0x25, :UP => 0x26,
		:RIGHT => 0x27, :DOWN => 0x28, :PRINT => 0x2A, :INSERT => 0x2D,
		:DELETE => 0x2E,

		:N0 => 0x30, :N1 => 0x31, :N2 => 0x32, :N3 => 0x33,
		:N4 => 0x34, :N5 => 0x35, :N6 => 0x36, :N7 => 0x37, :N8 => 0x38,
		:N9 => 0x39,

		:A => 0x41, :B => 0x42, :C => 0x43, :D => 0x44, :E => 0x45, :F => 0x46,
		:G => 0x47, :H => 0x48, :I => 0x49, :J => 0x4A, :K => 0x4B, :L => 0x4C,
		:M => 0x4D, :N => 0x4E, :O => 0x4F, :P => 0x50, :Q => 0x51, :R => 0x52,
		:S => 0x53, :T => 0x54, :U => 0x55, :V => 0x56, :W => 0x57, :X => 0x58,
		:Y => 0x59, :Z => 0x5A,

		:LWIN => 0x5B, :RWIN => 0x5C,

		:NUMPAD0 => 0x60, :NUMPAD1 => 0x61, :NUMPAD2 => 0x62, :NUMPAD3 => 0x63,
		:NUMPAD4 => 0x64, :NUMPAD5 => 0x65, :NUMPAD6 => 0x66, :NUMPAD7 => 0x67,
		:NUMPAD8 => 0x68, :NUMPAD9 => 0x69,
		:MULTIPLY => 0x6A, :ADD => 0x6B, :SEPARATOR => 0x6C, :SUBSTRACT => 0x6D,
		:DECIMAL => 0x6E, :DIVIDE => 0x6F,

		:F1 => 0x70, :F2 => 0x71, :F3 => 0x72, :F4 => 0x73,
		:F5 => 0x74, :F6 => 0x75, :F7 => 0x76, :F8 => 0x77,
		:F9 => 0x78, :F10 => 0x79, :F11 => 0x7A, :F12 => 0x7B,
		:F13 => 0x7C, :F14 => 0x7D, :F15 => 0x7E, :F16 => 0x7F,
		:F17 => 0x80, :F18 => 0x81, :F19 => 0x82, :F20 => 0x83,
		:F21 => 0x84, :F22 => 0x85, :F23 => 0x86, :F24 => 0x87,

		:NUMLOCK => 0x90, :SCROLL => 0x91,
		:LSHIFT => 0xA0, :RSHIFT => 0xA1, :LCONTROL => 0xA2, :RCONTROL => 0xA3,
		:LMENU => 0xA4, :RMENU => 0xA5,
		
		:OEM_1 => 0xBA,
		:OEM_PLUS => 0xBB, :OEM_COMMA => 0xBC, :OEM_MINUS => 0xBD, :OEM_PERIOD => 0xBE,
		:OEM_2 => 0xBF, :OEM_3 => 0xC0, :OEM_4 => 0xDB, :OEM_5 => 0xDC,
		:OEM_6 => 0xDD, :OEM_7 => 0xDE
	}

	WIN32INV = WIN32.invert

	WIN2SDL = {
		:BACK => :BACKSPACE,
		:CAPITAL => :CAPSLOCK,
		:PRIOR => :PAGEUP, :NEXT => :PAGEDOWN,
		:PRINT => :PRINTSCREEN,

		:LWIN => :LGUI, :RWIN => :RGUI,

		:NUMPAD0 => :KP_0, :NUMPAD1 => :KP_1, :NUMPAD2 => :KP_2, :NUMPAD3 => :KP_3,
		:NUMPAD4 => :KP_4, :NUMPAD5 => :KP_5, :NUMPAD6 => :KP_6, :NUMPAD7 => :KP_7,
		:NUMPAD8 => :KP_8, :NUMPAD9 => :KP_9,
		:MULTIPLY => :KP_MULTIPLY, :ADD => :KP_PLUS, :SUBSTRACT => :KP_MINUS,
		:DECIMAL => :KP_DECIMAL, :DIVIDE => :KP_DIVIDE,

		:NUMLOCK => :NUMLOCKCLEAR, :SCROLL => :SCROLLLOCK,
		:LCONTROL => :LCTRL, :RCONTROL => :RCTRL,
		:LMENU => :LALT, :RMENU => :RALT,
		
		# These are OEM and can vary by country
		# Values taken from Joiplay's src/input.cpp
		:OEM_1 => :SEMICOLON,
		:OEM_PLUS => :EQUALS, :OEM_COMMA => :COMMA, :OEM_MINUS => :MINUS, :OEM_PERIOD => :PERIOD,
		:OEM_2 => :SLASH, :OEM_3 => :GRAVE, :OEM_4 => :LEFTBRACKET, :OEM_5 => :BACKSLASH,
		:OEM_6 => :RIGHTBRACKET, :OEM_7 => :APOSTROPHE
	}

	WIN2SDL.default = :UNKNOWN
end

$win32KeyStates = nil

module Graphics
	class << self
		alias_method(:win32wrap_update, :update)
		def update
			win32wrap_update
			$win32KeyStates = nil
		end
	end
end

def get_raw_keystates
	if $win32KeyStates == nil
		$win32KeyStates = Input.raw_key_states
	end

	return $win32KeyStates
end

def common_keystate(vkey)
	vkey_name = Scancodes::WIN32INV[vkey]

	states = get_raw_keystates
	pressed = false

	if vkey_name == :LBUTTON
		pressed = Input.press?(Input::MOUSELEFT)
	elsif vkey_name == :RBUTTON
		pressed = Input.press?(Input::MOUSERIGHT)
	elsif vkey_name == :MBUTTON
		pressed = Input.press?(Input::MOUSEMIDDLE)
	elsif vkey_name == :SHIFT
		pressed = double_state(states, :LSHIFT, :RSHIFT)
	elsif vkey_name == :MENU
		pressed = double_state(states, :LALT, :RALT)
	elsif vkey_name == :CONTROL
		pressed = double_state(states, :LCTRL, :RCTRL)
	else
		scan = nil
		if Scancodes::SDL.key?(vkey_name)
			scan = vkey_name
		else
			scan = Scancodes::WIN2SDL[vkey_name]
		end

		pressed = state_pressed(states, scan)
	end

	return pressed ? 1 : 0
end

def memcpy_string(dst, src)
	i = 0
	src.each_byte do |b|
		dst.setbyte(i, b)
		i += 1
	end
end

def state_pressed(states, sdl_scan)
	return states[Scancodes::SDL[sdl_scan]]
end

def double_state(states, left, right)
	return state_pressed(states, left) || state_pressed(states, right)
end

# KGL2 is a Windows graphics DLL some Pokémon fangames call through Win32API
# for light blending and cast shadows. This module implements its functions on
# top of the Bitmap API so those games work on every platform, mirroring
# mkxp-z's scripts/preload/kgl2_wrap.rb. Arguments arrive as an array because
# that is how rsxp's Win32API wrapper invokes portable implementations.
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

module Win32API_Impl
	# Route Win32API.new('KGL2.klib', 'KglXxx', ...) to the compatibility module.
	KGL2 = KGL2_Impl
	module User32
		class Keybd_event
			Seq = [
				[0xA4, 0, 0, 0],
				[0xD, 0, 0, 0],
				[0xD, 0, 2, 0],
				[0xA4, 0, 2, 0]]
			Seq2 = [
				[0x12, 0, 0, 0],
				[0xD, 0, 0, 0],
				[0xD, 0, 2, 0],
				[0x12, 0, 2, 0]]
			def initialize
				@index = 0
			end
			def call(args)
				seq = [args[0], args[1], args[2], args[3]]

				if seq == Seq[@index] or seq == Seq2[@index]
					@index += 1
				else
					@index = 0
				end

				if @index == 4
					@index = 0
					Graphics.fullscreen = !Graphics.fullscreen
				end
			end
		end

		class GetKeyState
			def call(vkey)
				return common_keystate(vkey[0])
			end
		end
		class GetAsyncKeyState
			PRESSED_BIT = (1 << 15)
			def call(vkey)
				return common_keystate(vkey[0]) == 1 ? PRESSED_BIT : 0
			end
		end
		class GetKeyboardState
			PRESSED_BIT = 0x80
			def call(args)
				out_states = args[0]

				Scancodes::WIN32.each do |name, val|
					pressed = common_keystate(val) == 1

					out_states.setbyte(val, pressed ? PRESSED_BIT : 0)
				end
				return 1
			end
		end

		class ShowCursor
			def initialize
				@cursor_count = 0
			end
			def call(args)
				if args[0] == 1
					@cursor_count += 1
				else
					@cursor_count -= 1
				end

				Graphics.show_cursor = @cursor_count >= 0
			end
		end

		class GetCursorPos
			def call(args)
				out = [Input.mouse_x, Input.mouse_y].pack('ll')
				memcpy_string(args[0], out)
				return 1
			end
		end

		class GetClientRect
			def call(args)
				return 0 if args[0] != 42
				rect = [0, 0, 640, 480]
				begin
					rect[2] = Graphics.width
					rect[3] = Graphics.height
				rescue
				end
				memcpy_string(args[1], rect.pack('l4'))
				return 1
			end
		end

		class ScreenToClient
			def call(args)
				return 1
			end
		end

		class FindWindowA
			def call(args)
				if args[0] == "RGSS Player"
					return 42
				else
					return 0
				end
			end
		end

    FindWindow = FindWindowA
    GetWindowRect = GetClientRect
    class FindWindowEx
      def call(args)
        args[1]==0 && args[2]=='RGSS Player' ? 42 : 0
      end
    end
    FindWindowExA = FindWindowEx
    class GetWindowThreadProcessId
      def call(args)
        return 0 unless args[0]==42
        args[1].replace([Process.pid].pack('L')) if args[1].is_a?(String)
        Thread.current.object_id
      end
    end
    class GetSystemMetrics
      def call(args)
        case args[0]
        when 0 then Graphics.width
        when 1 then Graphics.height
        else 0
        end
      end
    end
    class SetWindowTextA
      def call(args)
        return 0 unless args[0]==42
        RSXP.call('window_title',payload:args[1].to_s)
        1
      end
    end
    SetWindowText = SetWindowTextA
    # Presentation is managed by the host; RGSS window decoration/position
    # requests need no extra OS window or framebuffer resize.
    class SetWindowPos
      def call(args) = args[0]==42 ? 1 : 0
    end
    ShowWindow = SetWindowPos
    class SetWindowLong
      def call(_args) = 0
    end
    SetWindowLongA = SetWindowLong
	end
  module Kernel32
    class GetCurrentThreadId
      def call(_args) = Thread.current.object_id
    end
    class GetCurrentProcessId
      def call(_args) = Process.pid
    end
    class GetPrivateProfileString
      def call(args)
        section,key,default,buffer,capacity,path=args
        value=default.to_s
        begin
          current=nil
          RSXP.call('read',payload:path.to_s).each_line do |line|
            line=line.strip
            if line.start_with?('[') && line.end_with?(']')
              current=line[1...-1]
            elsif current && current.casecmp?(section.to_s)
              name,entry=line.split('=',2)
              value=entry.strip if entry && name.strip.casecmp?(key.to_s)
            end
          end
        rescue StandardError
          # A missing INI file returns the caller's supplied default.
        end
        count=[Integer(capacity),buffer.bytesize].min
        return 0 if count<=0
        bytes=value.b.byteslice(0,count-1)
        memcpy_string(buffer,bytes+"\0")
        bytes.bytesize
      end
    end
    GetPrivateProfileStringA = GetPrivateProfileString
  end
end

def kappatalize(s)
	s[0] = s[0].upcase
	return s
end

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

	alias_method :rsxp_native_initialize, :initialize
	def initialize(dll, func, *args)
		@dll = dll
		@func = func
		@called = false
		@rsxp_wrap_impl = nil

		dll = dll.sub(/\.(dll|klib)\z/i, "").downcase
		dll = "KGL2" if dll=="kgl2" || dll.end_with?("/kgl2") || dll.end_with?("\\kgl2")
		dll = kappatalize(dll)
		func = kappatalize(func.dup)

		if !System.is_windows? or !NATIVE_ON_WINDOWS
			if /\A[A-Z]\w*\z/.match?(dll) && Win32API_Impl.const_defined?(dll,false)
				dll_impl = Win32API_Impl.const_get(dll)
				if /\A[A-Z]\w*\z/.match?(func) && dll_impl.const_defined?(func,false)
					@rsxp_wrap_impl = dll_impl.const_get(func).new
					return
				end
			end
		end

		@rsxp_native_available = false
		begin
			rsxp_native_initialize(@dll, @func, *args)
			@rsxp_native_available = true
			return
		rescue
		end

	end

	alias_method :rsxp_native_call, :call
	def call(*args)
		if @rsxp_wrap_impl
			return @rsxp_wrap_impl.call(args)
		end

		if @rsxp_native_available
			if LOG_NATIVE
				System.puts("[Win32API] [#{@dll}:#{@func}] #{args.to_s}")
			end
			return rsxp_native_call(*args)
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



module RSXP
  # JoiPlay-style games monkeypatch Zlib to route through RSXP.zinflate/zdeflate.
  # The streaming Zlib API is used directly so the game's
  # Zlib::Inflate.inflate / Zlib::Deflate.deflate overrides cannot recurse back
  # into themselves, and a raw deflate stream is accepted as a fallback.
  def self.zinflate(string)
    source=string.to_s.b
    [Zlib::MAX_WBITS,-Zlib::MAX_WBITS].each do |wbits|
      inflater=Zlib::Inflate.new(wbits)
      begin
        data=inflater.inflate(source)
        data<< inflater.finish unless inflater.finished?
        return data
      rescue Zlib::Error
        nil
      ensure
        inflater.close
      end
    end
    raise Zlib::DataError,'invalid compressed data'
  end
  def self.zdeflate(string, level=Zlib::DEFAULT_COMPRESSION)
    deflater=Zlib::Deflate.new(level)
    begin
      deflater.deflate(string.to_s.b,Zlib::FINISH)
    ensure
      deflater.close
    end
  end
end


eval(__rsxp_retained_source,TOPLEVEL_BINDING,'rsxp/retained.rb',1) if defined?(__rsxp_retained_source)

begin
  if RSXP.call('api_supported')=='1'
    eval(RSXP.call('api_game_bridge').force_encoding(Encoding::UTF_8),TOPLEVEL_BINDING,'api/game_bridge.rb',1)
  end
  eval(RSXP.call('rpg').force_encoding(Encoding::UTF_8),TOPLEVEL_BINDING,"<RPG#{RGSS_VERSION}>",1)
  RSXP.startup_yield
  RSXP.before_scripts
  # rsxp executes a game-provided preload script before the main scripts.
  load File.expand_path('preload.rb') if FileTest.exist?('preload.rb')
  RSXP.startup_yield
  if ENV['RSXP_SCRIPT']
    load ENV.fetch('RSXP_SCRIPT')
    RSXP.publish_top_level_methods
    RSXP.startup_yield
  else
    $RGSS_SCRIPTS=load_data(ENV.fetch('RSXP_SCRIPTS'))
    raise TypeError, 'script bundle must contain an Array' unless $RGSS_SCRIPTS.is_a?(Array)
    # Decompress all sections first, as the original engine does. Keep decoded
    # source in slot 3 for scripts that inspect $RGSS_SCRIPTS.
    $RGSS_SCRIPTS.each_with_index do |section,index|
      raise TypeError, "invalid script section #{index}" unless section.is_a?(Array) && section.size>=3 && section[1].is_a?(String) && section[2].is_a?(String)
      section[3]=Zlib::Inflate.inflate(section[2]).force_encoding(Encoding::UTF_8)
      RSXP.startup_yield
    end
    $RGSS_SCRIPTS.each do |section|
      RSXP::ERROR_OUTPUT.call("Loading #{section[1]}\n") if ENV['RSXP_TRACE']=='1'
      RSXP.eval_script(section[3],section[1].encode(Encoding::UTF_8,invalid: :replace,undef: :replace))
      # Keep Kernel.foo working for helpers defined so far; the final Main
      # section runs the game loop as soon as it is evaluated.
      RSXP.publish_top_level_methods
      RSXP.startup_yield
    end
  end
  0
rescue RSXPStop
  0
rescue SystemExit => error
  error.status || 0
rescue Exception => error
  RSXP::ERROR_OUTPUT.call(RSXP::ERROR_FORMAT.bind_call(error,highlight:false))
  1
end
