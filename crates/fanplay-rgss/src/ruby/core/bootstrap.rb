# SPDX-License-Identifier: GPL-2.0-or-later
# Ruby-facing RGSS API. Bitmaps, rendering, input and Table storage live in Rust.
require 'zlib'
require 'weakref'
require 'fileutils'
require 'json'
# CRuby aliases `$:` to `$LOAD_PATH`; SabiRuby has no `$:` global. Point them
# at the same array so plugins that push to `$:` affect require lookup.
$: = $LOAD_PATH unless $:
# Older Essentials bitmap caches call disposed? on cached WeakRef wrappers and
# expect a dead reference to count as disposed. Ruby's Delegator raises instead
# when forwarding to a collected object, so provide that RGSS-compatible query.
class WeakRef
  def disposed?
    object=__getobj__
    object.respond_to?(:disposed?) ? object.disposed? : false
  rescue WeakRef::RefError
    true
  end
end
$FANPLAY = true
$MKXP = $FANPLAY # Compatibility flag used by older game scripts.
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
  class Terminated < Exception; end
  # SabiRuby has no scheduler. RGSS uses Thread mainly for background I/O;
  # run the block synchronously and hand back a finished thread-like object.
  class Synchronous
    def initialize(value); @value=value; end
    def value; @value; end
    def join(*_args); self; end
    def status; false; end
    def alive?; false; end
    def running?; false; end
    def kill; self; end
    def terminate; self; end
    def wakeup; self; end
    def run; self; end
    def name; 'fanplay-sync'; end
    def [](_key); nil; end
    def []=(_key,value); value; end
  end
  class << self
    def new(*args,&block)
      value=nil
      begin
        value=block.call(*args) if block
      rescue Terminated
      end
      Synchronous.new(value)
    end
    alias start new
    alias fork new
    def exit
      raise Terminated
    end
  end
end
module HTTPLite
  # fanplay exposes HTTPLite::JSON with stringify/parse and ParserError.
  module JSON
    ParserError = ::JSON::ParserError
    def self.parse(string) = ::JSON.parse(string)
    def self.stringify(object) = ::JSON.generate(object)
  end
  # Network access is not ported yet; fail explicitly so callers can rescue.
  def self.get(*_args) = raise(FANPLAYError,'HTTPLite.get is not ported yet')
  def self.post(*_args) = raise(FANPLAYError,'HTTPLite.post is not ported yet')
  def self.post_body(*_args) = raise(FANPLAYError,'HTTPLite.post_body is not ported yet')
end

class RGSSError < StandardError; end
class RGSSReset < Exception; end
class FANPLAYError < StandardError; end
class FANPLAYStop < Exception; end
MKXPError = FANPLAYError unless defined?(MKXPError)
MKXPStop = FANPLAYStop unless defined?(MKXPStop)

# CRuby's immediate exit; plugin loaders use it to stop after a fatal error.
module Kernel
  def exit!(status=true)
    exit(status.is_a?(Integer) ? status : (status ? 0 : 1))
  end
  module_function :exit!
end

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

# mkxp games configure process-wide encodings at startup. SabiRuby has no
# transcoder configuration, so keep the assignments as plain accessors rather
# than failing the whole script section.
unless Encoding.respond_to?(:default_internal=)
  class << Encoding
    attr_accessor :default_internal, :default_external
  end
end

# SabiRuby exposes only Zlib::Inflate/Zlib::Deflate; RGSS scripts and the
# engine's load_data use CRuby's module-level shortcuts.
module Zlib
  Error=StandardError unless defined?(Error)
  def self.inflate(data)
    Inflate.inflate(data)
  end
  def self.deflate(data,level=DEFAULT_COMPRESSION)
    Deflate.deflate(data,level)
  end
end

# SabiRuby defines these only on Integer and Float, but RGSS utility plugins
# alias/decorate them on Numeric itself.
class Numeric
  def round(digits=0); to_f.round(digits); end
  def ceil(digits=0); to_f.ceil(digits); end
  def floor(digits=0); to_f.floor(digits); end
  def truncate(digits=0); to_f.truncate(digits); end
end

# SabiRuby ships Set, but not the Enumerable convenience or VERSION constant
# that the bundled stdlib copy adds.
class Set
  VERSION='1.1.0' unless defined?(VERSION)
end
module Enumerable
  def to_set(klass=Set,*args,&block)
    klass.new(self,*args,&block)
  end unless method_defined?(:to_set)
end

# SabiRuby has no Unicode normalization. RGSS scripts use NFD mainly to strip
# accents for sorting, so map the common Latin letters to their ASCII bases.
class String
  FANPLAY_ACCENT_MAP={
    'á'=>'a','à'=>'a','ä'=>'a','â'=>'a','ã'=>'a','å'=>'a','ā'=>'a','ă'=>'a','ą'=>'a',
    'Á'=>'A','À'=>'A','Ä'=>'A','Â'=>'A','Ã'=>'A','Å'=>'A','Ā'=>'A','Ă'=>'A','Ą'=>'A',
    'é'=>'e','è'=>'e','ë'=>'e','ê'=>'e','ē'=>'e','ė'=>'e','ę'=>'e',
    'É'=>'E','È'=>'E','Ë'=>'E','Ê'=>'E','Ē'=>'E','Ė'=>'E','Ę'=>'E',
    'í'=>'i','ì'=>'i','ï'=>'i','î'=>'i','ī'=>'i','į'=>'i',
    'Í'=>'I','Ì'=>'I','Ï'=>'I','Î'=>'I','Ī'=>'I','Į'=>'I',
    'ó'=>'o','ò'=>'o','ö'=>'o','ô'=>'o','õ'=>'o','ø'=>'o','ō'=>'o','ő'=>'o',
    'Ó'=>'O','Ò'=>'O','Ö'=>'O','Ô'=>'O','Õ'=>'O','Ø'=>'O','Ō'=>'O','Ő'=>'O',
    'ú'=>'u','ù'=>'u','ü'=>'u','û'=>'u','ū'=>'u','ű'=>'u',
    'Ú'=>'U','Ù'=>'U','Ü'=>'U','Û'=>'U','Ū'=>'U','Ű'=>'U',
    'ñ'=>'n','ń'=>'n','Ñ'=>'N','Ń'=>'N',
    'ç'=>'c','ć'=>'c','č'=>'c','Ç'=>'C','Ć'=>'C','Č'=>'C',
    'ý'=>'y','ÿ'=>'y','Ý'=>'Y','Ÿ'=>'Y',
    'š'=>'s','ś'=>'s','Š'=>'S','Ś'=>'S',
    'ž'=>'z','ź'=>'z','ż'=>'z','Ž'=>'Z','Ź'=>'Z','Ż'=>'Z',
    'ł'=>'l','Ł'=>'L','đ'=>'d','Đ'=>'D','ß'=>'ss','æ'=>'ae','Æ'=>'AE','œ'=>'oe','Œ'=>'OE'
  }
  def unicode_normalize(form=:nfc)
    return self if form==:nfc
    out=''.dup
    each_char { |char| out << (FANPLAY_ACCENT_MAP[char] || char) }
    out
  end
  def unicode_normalized?(form=:nfc)
    unicode_normalize(form)==self
  end
end

# SabiRuby's File is not an IO subclass and it exposes no IO module, but RGSS
# file wrappers reference `IO::SEEK_*` and call `IO.read`. Provide the small
# surface the games and their plugins use.
unless defined?(IO)
  module IO
    SEEK_SET=0
    SEEK_CUR=1
    SEEK_END=2
    def self.read(path,length=nil,offset=0)
      length ? File.read(path,length,offset) : File.read(path)
    end
    def self.binread(path,length=nil,offset=0)
      length ? File.binread(path,length,offset) : File.binread(path)
    end
    def self.write(path,data,offset=nil)
      offset ? File.write(path,data,offset) : File.write(path,data)
    end
    def self.binwrite(path,data,offset=nil)
      offset ? File.binwrite(path,data,offset) : File.binwrite(path,data)
    end
    def self.copy_stream(source,destination,length=nil,offset=0)
      data=source.respond_to?(:read) ? source.read(length) : File.binread(source.to_s)
      if destination.respond_to?(:write)
        destination.write(data)
      else
        File.binwrite(destination.to_s,data)
      end
      data.bytesize
    end
    def self.select(*_args)
      raise NotImplementedError,'IO.select is not available'
    end
    def self.popen(*_args)
      raise NotImplementedError,'IO.popen is not available'
    end
  end
end

# SabiRuby reports file/network failures as RuntimeError and defines no Errno
# constants. Provide the ones RGSS scripts name so their rescue/case clauses
# compile; the game's message-based formatter still handles the actual errors.
unless defined?(Errno)
  module Errno
    EACCES=Class.new(SystemCallError)
    ECONNABORTED=Class.new(SystemCallError)
    ECONNREFUSED=Class.new(SystemCallError)
    EINVAL=Class.new(SystemCallError)
    EISDIR=Class.new(SystemCallError)
    ENOENT=Class.new(SystemCallError)
    ETIMEDOUT=Class.new(SystemCallError)
  end
end

# Game scripts authored for Windows often normalize asset names to uppercase
# before checking them (e.g. the title plugin asks whether TITULO.ogg exists
# when the file is named Titulo.ogg). The native asset loader already resolves
# paths case-insensitively; make Ruby's FileTest preflight match that behavior.
module FileTest
  class << self
    alias_method :__fanplay_case_sensitive_exist?, :exist? unless method_defined?(:__fanplay_case_sensitive_exist?)

    def exist?(path)
      return true if __fanplay_case_sensitive_exist?(path)
      !FANPLAY.case_insensitive_path(path).nil?
    end

    alias_method :exists?, :exist?
  end
end

module FANPLAY
  # Resolve a path component by component with Windows-style case folding.
  # Keeping this at the Ruby boundary makes filesystem preflight checks and
  # subsequent File.open calls agree with the host asset loader.
  def self.case_insensitive_path(path)
    path = path.to_path if path.respond_to?(:to_path)
    return nil unless path.is_a?(String) && !path.empty?
    return path if FileTest.__fanplay_case_sensitive_exist?(path)

    absolute = File.expand_path(path.tr('\\', '/'))
    current = File::SEPARATOR
    absolute.split(File::SEPARATOR).reject(&:empty?).each do |part|
      candidate=File.join(current,part)
      if FileTest.__fanplay_case_sensitive_exist?(candidate)
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
    FileTest.__fanplay_case_sensitive_exist?(current) ? current : nil
  end

  # Resolve mixed-case game asset paths for direct Ruby reads as well as the
  # native engine's asset requests. Preserve normal File.open mode/kwargs and
  # block semantics by forwarding them to the original implementation.
  class << File
    alias_method :__fanplay_case_sensitive_open, :open unless method_defined?(:__fanplay_case_sensitive_open)

    def open(path, *args, **kwargs, &block)
      resolved = FANPLAY.case_insensitive_path(path)
      __fanplay_case_sensitive_open(resolved || path, *args, **kwargs, &block)
    end
  end

  # Some games reopen WeakRef with a Ruby 1.8 implementation. Keep the engine's
  # resource registry on the interpreter's GC-aware implementation.
  ResourceRef = defined?(FanplayWeakRef) ? FanplayWeakRef : WeakRef.dup
  ERROR_OUTPUT = STDERR.method(:write)
  ERROR_FORMAT = Exception.instance_method(:full_message)
  SYNC_QUEUE = []
  def self.call(op, *args, payload: ''.b)
    flush_sync unless op == 'sync_batch'
    result = __fanplay_native(([op, *args].join("\t") + "\n").b + payload.b)
    raise FANPLAYStop if result.nil?
    result
  end
  # Sprite/plane/viewport snapshots are only consumed by Graphics.update. Batch
  # adjacent snapshots to avoid crossing the Ruby bridge once per object.
  def self.flush_sync
    return if SYNC_QUEUE.empty?
    payload = SYNC_QUEUE.join("\x1e").b
    SYNC_QUEUE.clear
    result = __fanplay_native("sync_batch\n".b + payload)
    raise FANPLAYStop if result.nil?
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

  # RPG Maker projects may store script source as UTF-8, Windows-1252, or
  # ISO-8859-1. Prefer valid UTF-8 so multibyte text is never mojibaked, then
  # decode legacy Western encodings into UTF-8 for Ruby's parser.
  def self.decode_script_source(source)
    bytes=source.to_s.dup.force_encoding(Encoding::BINARY)
    utf8=bytes.dup.force_encoding(Encoding::UTF_8)
    return utf8 if utf8.valid_encoding?
    begin
      bytes.force_encoding(Encoding::Windows_1252).encode(Encoding::UTF_8)
    rescue EncodingError
      bytes.force_encoding(Encoding::ISO_8859_1).encode(Encoding::UTF_8)
    end
  end

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
  # Extension point: hosts may hook here after the RGSS
  # API is defined but before the game's scripts run. Defined only if a host
  # has not already provided one.
  def self.before_scripts; end unless singleton_class.method_defined?(:before_scripts)
  def self.prepare_script(source)
    # SabiRuby already provides Set; the bundled CRuby stdlib copy uses
    # syntax its compiler does not understand and would just replace it.
    if defined?(Set) && source.include?('set.rb - defines the Set class')
      return "# Set is provided by SabiRuby; bundled stdlib copy skipped.\n"
    end
    # Pokemon Royal bundles two implementations of Make It Rain in one
    # section. The later standalone implementation replaces the earlier
    # StatDownMove implementation; reopening it would both reject the new
    # superclass and retain the old damage hook (awarding coins twice).
    # Drop only this known superseded definition, preserving backtrace lines.
    move='Battle::Move::AddMoneyGainedFromBattleLowerUserSpAtk1'
    old_definition=/^class #{Regexp.escape(move)} < Battle::Move::LowerUserSpAtk1[ \t]*\r?\n.*?^end[ \t]*\r?$/m
    replacement=/^class #{Regexp.escape(move)} < Battle::Move[ \t]*\r?$/
    old=source.match(old_definition)
    if old && source.match(replacement,old.end(0))
      source=source.sub(old_definition) { |definition| definition.gsub(/[^\r\n]/,'') }
    end
    # Realidea's PSystem_Controls replaces Input.update and reads the host
    # keyboard state via Win32API. Refresh FANPLAY's raw state before that poll.
    if source.include?('GetAsyncKeyState') && source.include?('def self.update')
      source=source.sub(/def self\.update\s*\n/) { |definition| definition+"    Input.refresh_raw_key_states\n" }
    end
    # Essentials' Main loader evaluates each archived script section itself.
    # Route only those calls through the decoder; leave event/plugin evals on
    # Ruby's native Kernel.eval so their caller binding and self are preserved.
    if source.include?('load_scripts_from_folder')
      # This engine's pattern library has no lookbehind/lookahead. Match the
      # character in front of `eval` and write it back, and repeat until no
      # adjacent call is left unconverted.
      loop do
        fixed=source.gsub(/(\A|[^.\w:])eval([ \t]*\()/) { "#{$1}FANPLAY.eval_game_script#{$2}" }
        break if fixed==source
        source=fixed
      end
    end
    # SabiRuby's File stands alone, so RGSS wrappers that reopen it as
    # `class File < IO` would hit a superclass mismatch. Reopen File itself;
    # its file methods are already on the class.
    if source.include?('File < IO')
      source=source.gsub(/^([ \t]*)class([ \t]+)File[ \t]*<[ \t]*IO[ \t]*\r?$/) { "#{$1}class#{$2}File" }
    end
    # SabiRuby's compiler rejects op-assignment on a scoped constant, which
    # Essentials v21.1 Hotfixes uses to append its label to ERROR_TEXT.
    if source.include?('Essentials::ERROR_TEXT +=')
      source=source.gsub(/^([ \t]*)(Essentials::ERROR_TEXT)[ \t]*\+=[ \t]*(.+)$/) { "#{$1}#{$2} = #{$2} + (#{$3})" }
    end
    if RGSS_VERSION==1
      # Binary readers written for Ruby 1.8 expect String#[0] to return a byte.
      if source.include?('.read(1)[0]')
        source=source.gsub(/\.read\(1\)\[0\]/,'.read(1).getbyte(0)')
      end
      if source.include?('0x') || source.include?('>>')
        source=source.gsub(/\b([a-z_]\w*)\[([^\[\]\n]+)\](\s*(?:[!=]=\s*0x[\da-fA-F]+|>>\s*\d+))/,'FANPLAY.byte_index(\1,\2)\3')
      end
      if source.include?('Events.')
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
        # The engine has no lookahead, so consume the following whitespace and
        # the next character instead of testing that it is not `|`; put them
        # back unchanged. A `|` here means a parameter list this pass handles
        # elsewhere (possibly split over several lines), so leave it alone.
        source=source.gsub(/(\bEvents\.\w+[ \t]*\+=[ \t]*)proc([ \t]*(?:\{|do\b))(\s*)(\||[^|])/) do
          if $4=='|'
            "#{$1}proc#{$2}#{$3}#{$4}"
          else
            "#{$1}lambda#{$2}|*|#{$3}#{$4}"
          end
        end
      end
    end
    # Ruby 1.8 byte-API polyfills must not replace Ruby's native byte methods
    # with modern character-indexing methods.
    if source.include?('alias')
      source=source.gsub(/^([ \t]*alias[ \t]+(getbyte|setbyte|bytesize)[ \t]+(?:\[\]=?|size))[ \t]*\r?$/,'\1 unless method_defined?(:\2)')
    end
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
    # eval strings from Marshal/Zlib are UTF-8; keep an owned copy so the
    # encoding adapter below can safely prepare it.
    source=decode_script_source(source)
    source.replace(prepare_script(source))
    begin
      if name.to_s=='Main'
        # Some Essentials forks replace the usual rgss_main wrapper with a
        # custom top-level loop. Keep their Reset exception inside the engine
        # restart boundary by wrapping evaluation of Main itself.
        main=TOPLEVEL_BINDING.eval('self')
        main.__send__(:rgss_main) { eval(source,TOPLEVEL_BINDING,name,1) }
      else
        eval(source,TOPLEVEL_BINDING,name,1)
      end
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
        elsif message.include?('Invalid next') && line.match?(/\A[ \t]*next\b/)
          # Older RGSS scripts use next as an early exit from a method. Modern
          # Ruby rejects it there; return preserves the intended method exit.
          fixed=line.sub(/\bnext\b/,'return')
        elsif message.include?("unexpected ':'")
          # No lookahead: match the following character and put it back so a
          # `::` constant separator is still left untouched.
          fixed=line.sub(/\A([ \t]*when\s+-?\d+(?:\s*,\s*-?\d+)*)[ \t]*:([^:]|$)/,'\1 then\2')
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
  def self.eval_game_script(source,binding=TOPLEVEL_BINDING,filename='(eval)',lineno=1)
    eval(decode_script_source(source),binding || TOPLEVEL_BINDING,filename || '(eval)',lineno || 1)
  end
  # Hosts may use this hook to cooperatively yield during large startup
  # workloads. Desktop execution leaves it as a no-op.
  def self.startup_yield; end unless singleton_class.method_defined?(:startup_yield)
  # fanplay re-tags ASCII-8BIT strings from Marshal data as UTF-8.
  def self.force_utf8(object)
    if object.is_a?(String) && object.encoding == Encoding::ASCII_8BIT
      object.force_encoding(Encoding::UTF_8)
    end
    object
  end
  # RGSS/fanplay lets scripts call top-level helper methods as `Kernel.foo`
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
      FANPLAY.call('sprite_tile',__id)
    end
  end

  def self.install_native_tilemap
    return if ENV['FANPLAY_NATIVE_TILEMAP']=='0'
    return unless defined?(TilemapRenderer) && TilemapRenderer.const_defined?(:TileSprite,false)
    klass=TilemapRenderer.const_get(:TileSprite)
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
    return if $__fanplay_plugin_manager_patched
    return unless defined?(PluginManager) && PluginManager.respond_to?(:runPlugins)
    $__fanplay_plugin_manager_patched = true
    PluginManager.singleton_class.class_eval do
      alias_method :__fanplay_runPlugins, :runPlugins unless method_defined?(:__fanplay_runPlugins)
      define_method(:runPlugins) do
        __fanplay_runPlugins
        FANPLAY.republicize_sprite_setters
      end
    end
  end
end

