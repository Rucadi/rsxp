// SPDX-License-Identifier: GPL-2.0-or-later
// Browser-specific VM services; shared by the page and the headless runner.
export const PRELUDE = `
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
    alias_method :__rsxp_real_exist?, :exist?
    alias_method :__rsxp_real_file?, :file?
    alias_method :__rsxp_real_foreach, :foreach
    # Ruby expands metadata paths against its game cwd, while engine assets
    # are named relative to that cwd (including assets held only in archives).
    def __rsxp_asset_path(path)
      absolute = expand_path(path)
      prefix = Dir.pwd.sub(%r{/+$}, '') + '/'
      absolute.start_with?(prefix) ? absolute.delete_prefix(prefix) : path.to_s
    end
    def file?(path)
      RSXP.call('exists', payload: RSXP.utf8(__rsxp_asset_path(path))) == '1' || __rsxp_real_file?(path)
    end
    def foreach(path, *args, encoding: nil, **options, &block)
      return enum_for(:foreach, path, *args, encoding: encoding, **options) unless block
      asset = __rsxp_asset_path(path)
      if RSXP.call('exists', payload: RSXP.utf8(asset)) == '1'
        data = RSXP.call('read', payload: RSXP.utf8(asset))
        data.force_encoding(encoding || Encoding.default_external)
        StringIO.new(data).each_line(*args, **options, &block)
        nil
      else
        options[:encoding] = encoding if encoding
        __rsxp_real_foreach(path, *args, **options, &block)
      end
    end
    alias_method :__rsxp_real_read, :read
    alias_method :__rsxp_real_binread, :binread
    alias_method :__rsxp_real_binwrite, :binwrite
    def binwrite(path, data, *args)
      result = __rsxp_real_binwrite(path, data, *args)
      RSXP.persist_save(path)
      result
    end
    def exist?(path)
      return true if RSXP.call('exists', payload: RSXP.utf8(__rsxp_asset_path(path))) == '1'
      __rsxp_real_exist?(path)
    end
    def exists?(path) = exist?(path)
    def binread(path, *args)
      if RSXP.call('exists', payload: RSXP.utf8(__rsxp_asset_path(path))) == '1'
        RSXP.call('read', payload: RSXP.utf8(__rsxp_asset_path(path)))
      else
        __rsxp_real_binread(path, *args)
      end
    end
    def read(path, *args)
      if RSXP.call('exists', payload: RSXP.utf8(__rsxp_asset_path(path))) == '1'
        RSXP.call('read', payload: RSXP.utf8(__rsxp_asset_path(path)))
      else
        __rsxp_real_read(path, *args)
      end
    end
    def readlines(path, *args)
      read(path, *args).split($/, -1)
    end
    alias_method :__rsxp_real_open, :open unless method_defined?(:__rsxp_real_open)
    def open(path, mode = 'r', *rest, &block)
      mode = 'r' if mode.nil?
      if mode.to_s.start_with?('r') && RSXP.call('exists', payload: RSXP.utf8(__rsxp_asset_path(path))) == '1'
        io = StringIO.new(RSXP.call('read', payload: RSXP.utf8(__rsxp_asset_path(path))))
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
        result = __rsxp_real_open(path, mode, *rest, &block)
        RSXP.persist_save(path) if mode.to_s.match?(/[wa+]/)
        result
      else
        __rsxp_real_open(path, mode, *rest)
      end
    end
  end
end
module FileTest
  class << self
    def exist?(path) = File.exist?(path)
    def exists?(path) = File.exist?(path)
    def file?(path) = File.file?(path)
    def size(path) = File.binread(path).bytesize
  end
end
def __rsxp_native(request)
  request.force_encoding(Encoding::UTF_8)
  if request.valid_encoding?
    result = JS.global[:RSXP].native_text(request)
    text = result.nil? ? '' : result.to_s
    return nil if text == '__RSXP_STOP__'
    return ''.b if text.empty?
    payload = text.byteslice(1..-1) || ''
    return payload.b if text.getbyte(0) == 85  # UTF-8 response
    return payload.unpack1('m0') if text.getbyte(0) == 66 # binary response
    raise "invalid native response encoding"
  end
  result = JS.global[:RSXP].native([request].pack('m0'))
  text = result.nil? ? '' : result.to_s
  return nil if text == '__RSXP_STOP__'
  return ''.b if text.empty?
  text.unpack1('m0')
end
def __mkxp_native(request) = __rsxp_native(request)
module RSXP
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
    data = File.__rsxp_real_binread(path)
    JS.global[:RSXP].persistSave(path, [data].pack('m0'))
  rescue StandardError
    nil
  end
  # Restore mirrored saves into the filesystem before the game reads them.
  def self.restore_saves
    require 'json'
    raw = JS.global[:RSXP][:savedSavesJson].to_s
    return if raw.empty?
    JSON.parse(raw).each do |path, encoded|
      FileUtils.mkdir_p(File.dirname(path))
      File.__rsxp_real_binwrite(path, encoded.unpack1('m0'))
    rescue StandardError
      nil
    end
  rescue StandardError
    nil
  end
  def self.startup_yield
    @startup_work = (@startup_work || 0) + 1
    return unless (@startup_work % 8).zero?
    JS.global[:RSXP].startup_progress(@startup_work)
    Fiber.yield
  end
  def self.before_scripts
    # Copy the game files into ruby.wasm's own filesystem so Ruby's FileTest,
    # File and Dir APIs see them (the engine keeps its own copy for rendering).
    require 'fileutils'
    restore_saves
    paths = JS.global[:RSXP][:manifestText].to_s.split("\n")
    paths << 'preload.rb'
    paths.each do |path|
      next if path.empty?
      begin
        FileUtils.mkdir_p(File.dirname(path))
        File.binwrite(path, RSXP.call('read', payload: path))
      rescue StandardError
        nil
      end
      startup_yield
    end
    Graphics.singleton_class.class_eval do
      alias_method :__rsxp_web_update, :update
      def update
        __rsxp_web_update
        JS.global[:RSXP].game_started
        Fiber.yield
      end
    end

  end
end
`;
