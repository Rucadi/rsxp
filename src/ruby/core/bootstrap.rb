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
    # RPG Maker script sections are normally UTF-8, but Marshal/Zlib data may
    # arrive as ASCII-8BIT. Ruby's parser then treats UTF-8 continuation bytes
    # as invalid source bytes unless the encoding tag is corrected first.
    source=source.dup unless source.is_a?(String)
    source=source.dup unless source.encoding==Encoding::UTF_8
    source.force_encoding(Encoding::UTF_8)
    source=source.encode(Encoding::UTF_8,invalid: :replace,undef: :replace) unless source.valid_encoding?
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

