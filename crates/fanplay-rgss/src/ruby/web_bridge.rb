# SPDX-License-Identifier: GPL-2.0-or-later
# Small browser-only adapter for CRuby built by ruby.wasm.
#
# Ruby values cross the JS boundary as base64 strings so arbitrary game data
# (Marshal, compressed scripts, images and audio) never goes through a UTF-8
# conversion. The Rust callback owns the actual RGSS engine and archive reads.
require 'fanplay_native'
require 'js'
require 'stringio'

# ruby.wasm is a WASI build: the filesystem is private to the VM and several
# native-only libraries are unavailable. Keep optional game/plugin requires
# harmless while leaving the actual RGSS file access on FANPLAY's Rust host.
%w[socket io/wait openssl].each do |feature|
  begin
    require feature
  rescue LoadError
    $LOADED_FEATURES << "#{feature}.rb" unless $LOADED_FEATURES.include?("#{feature}.rb")
  end
end

# Ruby's WASI build has no native threads. Essentials uses Thread.new for
# deferred setup, so execute those small blocks synchronously instead of
# letting the first unsupported thread creation abort the game.
class Thread
  class << self
    alias_method :__fanplay_real_new, :new unless method_defined?(:__fanplay_real_new)

    def new(*_args, &block)
      block&.call
      Object.new
    rescue Exception
      Object.new
    end
  end
end

module FANPLAY
  # bootstrap.rb invokes this as an unqualified call from a FANPLAY singleton
  # method, so the browser hook must be a FANPLAY singleton method rather than
  # a private Object method.
  def self.__fanplay_native(request)
    # Drain converted native updates before any query, render or disposal.
    # The queue holds only Rust values; Ruby objects stay in this interpreter.
    if (updates = __fanplay_take_render_commands)
      __fanplay_web_request("native_updates\n".b + updates)
    end
    __fanplay_web_request(request)
  end

  def self.__fanplay_web_request(request)
    encoded = [request].pack('m0')
    # JS::Object#call takes a method name, unlike Ruby Proc#call.
    result = JS.global.call(:__fanplay_native_b64, encoded).to_s
    raise FANPLAYStop if result == '__FANPLAY_STOP__'
    result.unpack1('m0')
  end

  class << self
    alias __mkxp_native __fanplay_native
  end

  # Ruby code in Essentials reads both loose assets and files stored in an
  # encrypted RGSS archive. Route reads and existence probes through the same
  # synchronous Rust asset dispatcher that Graphics and Bitmap already use.
  def self.__fanplay_asset_path(path)
    path = path.to_path if path.respond_to?(:to_path)
    return path.to_s unless path.is_a?(String)
    absolute = File.expand_path(path.tr('\\', '/'))
    prefix = Dir.pwd.sub(%r{/+$}, '') + '/'
    absolute.start_with?(prefix) ? absolute.delete_prefix(prefix) : path
  end

  def self.__fanplay_asset_exists?(path)
    call('exists', payload: __fanplay_asset_path(path)) == '1'
  rescue FANPLAYStop
    false
  end

  def self.__fanplay_asset_read(path)
    call('read', payload: __fanplay_asset_path(path))
  end

  def self.before_scripts
    # script_loader.rb uses Kernel#load for this optional file. Copy it into
    # ruby.wasm's private filesystem after the virtual asset probe succeeds.
    if __fanplay_asset_exists?('preload.rb')
      File.binwrite('preload.rb', __fanplay_asset_read('preload.rb'))
    end

    # Yield after each native frame. The JavaScript pump resumes this Fiber on
    # the next animation frame, allowing the Rust host to present and poll.
    class << Graphics
      alias_method :__fanplay_web_update, :update unless method_defined?(:__fanplay_web_update)

      def update
        __fanplay_web_update
        FANPLAY.startup_yield
      end
    end
  end
end

# Add the virtual asset view after bootstrap.rb has installed its native-file
# compatibility aliases. Writes continue to use ruby.wasm's in-memory WASI
# filesystem, which is where save_data and temporary plugin files belong.
class File
  class << self
    alias_method :__fanplay_wasi_exist?, :exist? unless method_defined?(:__fanplay_wasi_exist?)
    alias_method :__fanplay_wasi_file?, :file? unless method_defined?(:__fanplay_wasi_file?)
    alias_method :__fanplay_wasi_foreach, :foreach unless method_defined?(:__fanplay_wasi_foreach)
    alias_method :__fanplay_wasi_read, :read unless method_defined?(:__fanplay_wasi_read)
    alias_method :__fanplay_wasi_binread, :binread unless method_defined?(:__fanplay_wasi_binread)
    alias_method :__fanplay_wasi_open, :open unless method_defined?(:__fanplay_wasi_open)

    def exist?(path)
      FANPLAY.__fanplay_asset_exists?(path) || __fanplay_wasi_exist?(path)
    end

    alias_method :exists?, :exist?

    def file?(path)
      FANPLAY.__fanplay_asset_exists?(path) || __fanplay_wasi_file?(path)
    end

    def binread(path, *args)
      return FANPLAY.__fanplay_asset_read(path) if FANPLAY.__fanplay_asset_exists?(path)

      __fanplay_wasi_binread(path, *args)
    end

    def read(path, *args)
      return FANPLAY.__fanplay_asset_read(path) if FANPLAY.__fanplay_asset_exists?(path)

      __fanplay_wasi_read(path, *args)
    end

    def readlines(path, *args)
      read(path, *args).split($/, -1)
    end

    def foreach(path, *args, encoding: nil, **options, &block)
      return enum_for(:foreach, path, *args, encoding: encoding, **options) unless block

      if FANPLAY.__fanplay_asset_exists?(path)
        data = FANPLAY.__fanplay_asset_read(path)
        data.force_encoding(encoding || Encoding.default_external)
        StringIO.new(data).each_line(*args, **options, &block)
        nil
      else
        options[:encoding] = encoding if encoding
        __fanplay_wasi_foreach(path, *args, **options, &block)
      end
    end

    def open(path, mode = 'r', *rest, &block)
      mode = 'r' if mode.nil?
      if mode.to_s.start_with?('r') && FANPLAY.__fanplay_asset_exists?(path)
        io = StringIO.new(FANPLAY.__fanplay_asset_read(path))
        return block.call(io) if block

        return io
      end
      __fanplay_wasi_open(path, mode, *rest, &block)
    end
  end
end

module FileTest
  class << self
    alias_method :__fanplay_wasi_exist?, :exist? unless method_defined?(:__fanplay_wasi_exist?)

    def exist?(path)
      FANPLAY.__fanplay_asset_exists?(path) || __fanplay_wasi_exist?(path)
    end

    alias_method :exists?, :exist?

    def file?(path)
      File.file?(path)
    end

    def size(path)
      File.binread(path).bytesize
    end
  end
end

# The browser host must return to eframe after each native Graphics.update.
# Ruby's Fiber gives the existing RGSS script loop cooperative scheduling with
# no per-frame JavaScript game-loop implementation.
module FANPLAY
  def self.startup_yield
    Fiber.yield
  end
end
