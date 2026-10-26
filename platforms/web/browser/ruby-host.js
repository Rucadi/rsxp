// SPDX-License-Identifier: GPL-2.0-or-later
// Browser-specific VM services; shared by the page and the headless runner.

// ruby.wasm's in-memory WASI filesystem does not expose permission bits, so
// File.writable? is always false. Dir.tmpdir and Dir.mktmpdir then reject even
// directories that accept writes. Treat existing directories as writable
// candidates; writes still fail normally if the filesystem is read-only.
export const WASI_TMPDIR_COMPAT = `
class << File
  alias_method :__fanplay_wasi_writable?, :writable? unless method_defined?(:__fanplay_wasi_writable?)
  def writable?(path) = __fanplay_wasi_writable?(path) || directory?(path)
end
`;

export const PRELUDE = `
${WASI_TMPDIR_COMPAT}
require 'js'
require 'stringio'
# RGSS FileInputMixin binary helpers are normally available on files returned
# by pbRgssOpen. Browser asset opens return StringIO instead, so provide the
# same little-endian readers on that adapter.
class StringIO
  def fgetb
    read(1)&.getbyte(0)
  end unless method_defined?(:fgetb)

  def fgetw
    low = fgetb
    high = fgetb
    return nil unless low && high
    low | (high << 8)
  end unless method_defined?(:fgetw)

  def fgetdw
    low = fgetw
    high = fgetw
    return nil unless low && high
    low | (high << 16)
  end unless method_defined?(:fgetdw)
end
# ruby.wasm omits some C-extension stdlibs. Plugins only need the require to
# succeed at load time, so mark unavailable features as loaded.
%w[socket io/wait openssl].each do |feature|
  begin
    require feature
  rescue LoadError
    $LOADED_FEATURES << "#{feature}.rb" unless $LOADED_FEATURES.include?("#{feature}.rb")
  end
end
# ruby.wasm has no thread support. Plugins use Thread.new for deferred setup;
# run the block synchronously when it can be, otherwise ignore it.
class Thread
  def self.new(*_args, &block)
    begin
      block&.call
    rescue Exception
      nil
    end
    Object.new
  end
end
# Game assets are loaded on demand through the engine; Ruby's own filesystem
# only holds save data and any preloaded files.
class File
  class << self
    alias_method :__fanplay_real_exist?, :exist?
    alias_method :__fanplay_real_file?, :file?
    alias_method :__fanplay_real_foreach, :foreach
    # Ruby expands metadata paths against its game cwd, while engine assets
    # are named relative to that cwd (including assets held only in archives).
    def __fanplay_asset_path(path)
      absolute = expand_path(path)
      prefix = Dir.pwd.sub(%r{/+$}, '') + '/'
      absolute.start_with?(prefix) ? absolute.delete_prefix(prefix) : path.to_s
    end
    def __fanplay_asset_exist?(path)
      FANPLAY.call('exists', payload: FANPLAY.utf8(__fanplay_asset_path(path))) == '1'
    end
    def file?(path)
      __fanplay_asset_exist?(path) || __fanplay_real_file?(path)
    end
    def foreach(path, *args, encoding: nil, **options, &block)
      return enum_for(:foreach, path, *args, encoding: encoding, **options) unless block
      asset = __fanplay_asset_path(path)
      if FANPLAY.call('exists', payload: FANPLAY.utf8(asset)) == '1'
        data = FANPLAY.call('read', payload: FANPLAY.utf8(asset))
        data.force_encoding(encoding || Encoding.default_external)
        StringIO.new(data).each_line(*args, **options, &block)
        nil
      else
        options[:encoding] = encoding if encoding
        __fanplay_real_foreach(path, *args, **options, &block)
      end
    end
    alias_method :__fanplay_real_read, :read
    alias_method :__fanplay_real_binread, :binread
    alias_method :__fanplay_real_binwrite, :binwrite
    def binwrite(path, data, *args)
      result = __fanplay_real_binwrite(path, data, *args)
      FANPLAY.persist_save(path)
      result
    end
    def exist?(path)
      return true if __fanplay_asset_exist?(path)
      __fanplay_real_exist?(path)
    end
    def exists?(path) = exist?(path)
    def binread(path, *args)
      if FANPLAY.call('exists', payload: FANPLAY.utf8(__fanplay_asset_path(path))) == '1'
        FANPLAY.call('read', payload: FANPLAY.utf8(__fanplay_asset_path(path)))
      else
        __fanplay_real_binread(path, *args)
      end
    end
    def read(path, *args)
      if FANPLAY.call('exists', payload: FANPLAY.utf8(__fanplay_asset_path(path))) == '1'
        FANPLAY.call('read', payload: FANPLAY.utf8(__fanplay_asset_path(path)))
      else
        __fanplay_real_read(path, *args)
      end
    end
    def readlines(path, *args)
      read(path, *args).split($/, -1)
    end
    alias_method :__fanplay_real_open, :open unless method_defined?(:__fanplay_real_open)
    def open(path, mode = 'r', *rest, &block)
      mode = 'r' if mode.nil?
      if mode.to_s.start_with?('r') && FANPLAY.call('exists', payload: FANPLAY.utf8(__fanplay_asset_path(path))) == '1'
        io = StringIO.new(FANPLAY.call('read', payload: FANPLAY.utf8(__fanplay_asset_path(path))))
        if block
          begin
            return block.call(io)
          ensure
            io.close
          end
        end
        return io
      end
      if block
        result = __fanplay_real_open(path, mode, *rest, &block)
        FANPLAY.persist_save(path) if mode.to_s.match?(/[wa+]/)
        result
      else
        __fanplay_real_open(path, mode, *rest)
      end
    end
  end
end
module FileTest
  class << self
    # rgss.rb aliases FileTest's current probe to __fanplay_case_sensitive_exist?
    # and builds its case-insensitive exist? on top. Defining it here first
    # keeps that alias independent of File.exist?: RGSS games commonly replace
    # File.exist? with a wrapper that falls back to FileTest.exist?, and an
    # alias that dispatched back through the replaceable File.exist? would
    # recurse until the Ruby VM's stack overflows.
    def __fanplay_case_sensitive_exist?(path)
      File.__fanplay_asset_exist?(path) || File.__fanplay_real_exist?(path)
    end
    def exist?(path) = __fanplay_case_sensitive_exist?(path)
    def exists?(path) = exist?(path)
    def file?(path) = File.file?(path)
    def size(path) = File.binread(path).bytesize
  end
end
def __fanplay_native(request)
  request.force_encoding(Encoding::UTF_8)
  if request.valid_encoding?
    result = JS.global[:FANPLAY].native_text(request)
    text = result.nil? ? '' : result.to_s
    return nil if text == '__FANPLAY_STOP__'
    return ''.b if text.empty?
    payload = text.byteslice(1..-1) || ''
    return payload.b if text.getbyte(0) == 85  # UTF-8 response
    return payload.unpack1('m0') if text.getbyte(0) == 66 # binary response
    raise "invalid native response encoding"
  end
  result = JS.global[:FANPLAY].native([request].pack('m0'))
  text = result.nil? ? '' : result.to_s
  return nil if text == '__FANPLAY_STOP__'
  return ''.b if text.empty?
  text.unpack1('m0')
end
def __mkxp_native(request) = __fanplay_native(request)
module FANPLAY
  # Directory holding the game's saves; only files written here are mirrored to
  # the browser's localStorage (game assets under Data/ are not).
  def self.save_directory
    return @save_directory if defined?(@save_directory)
    @save_directory = begin
      System.data_directory
    rescue StandardError
      nil
    end
  end
  # Mirror a written save file to the host so it survives a page reload.
  def self.persist_save(path)
    path = path.to_s
    return unless path.end_with?('.rxdata')
    dir = save_directory
    if dir && !path.start_with?(dir)
      return unless path == 'Game.rxdata' || path == './Game.rxdata'
    end
    data = File.__fanplay_real_binread(path)
    JS.global[:FANPLAY].persistSave(path, [data].pack('m0'))
  rescue StandardError
    nil
  end
  # Restore mirrored saves into the filesystem before the game reads them.
  def self.restore_saves
    require 'json'
    raw = JS.global[:FANPLAY][:savedSavesJson].to_s
    return if raw.empty?
    JSON.parse(raw).each do |path, encoded|
      FileUtils.mkdir_p(File.dirname(path))
      File.__fanplay_real_binwrite(path, encoded.unpack1('m0'))
    rescue StandardError
      nil
    end
  rescue StandardError
    nil
  end
  def self.startup_yield
    @startup_work = (@startup_work || 0) + 1
    return unless (@startup_work % 8).zero?
    JS.global[:FANPLAY].startup_progress(@startup_work)
    Fiber.yield
  end
  def self.before_scripts
    # Copy the game files into ruby.wasm's own filesystem so Ruby's FileTest,
    # File and Dir APIs see them (the engine keeps its own copy for rendering).
    require 'fileutils'
    restore_saves
    paths = JS.global[:FANPLAY][:manifestText].to_s.split("\n")
    paths << 'preload.rb'
    paths.each do |path|
      next if path.empty?
      begin
        FileUtils.mkdir_p(File.dirname(path))
        File.binwrite(path, FANPLAY.call('read', payload: path))
      rescue StandardError
        nil
      end
      startup_yield
    end
    Graphics.singleton_class.class_eval do
      alias_method :__fanplay_web_update, :update
      def update
        __fanplay_web_update
        JS.global[:FANPLAY].game_started
        Fiber.yield
      end
    end

  end
end
`;
