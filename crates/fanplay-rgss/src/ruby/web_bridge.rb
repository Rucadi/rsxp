# SPDX-License-Identifier: GPL-2.0-or-later
# Small browser-only adapter for CRuby built by ruby.wasm.
#
# Ruby values cross the JS boundary as base64 strings so arbitrary game data
# (Marshal, compressed scripts, images and audio) never goes through a UTF-8
# conversion. The Rust callback owns the actual RGSS engine and archive reads.
require 'fanplay_native'
require 'js'
require 'stringio'

# The browser worker reports the actual binding state after the common RGSS
# shim has loaded. This is deliberately a diagnostic side channel: rendering
# still crosses the normal native command/update bridge below. Desktop and
# headless WASM checks do not install the JS callback, so they remain silent.
module FANPLAY
  def self.native_bindings_report
    return unless ENV['FANPLAY_WEB'] == '1'
    flags = [
      'ruby_shim=1',
      'audio=1',
      'table=1',
      'bitmap=1',
      'viewport=1',
      'plane=1',
      "sprite_sync=#{defined?(Sprite::NATIVE_SYNC) && Sprite::NATIVE_SYNC ? 1 : 0}",
      "retained_state=#{defined?(FANPLAY::Retained::ENABLED) && FANPLAY::Retained::ENABLED ? 1 : 0}",
      "tilemap=#{ENV['FANPLAY_NATIVE_TILEMAP'] == '0' ? 0 : 1}",
      "tilemap_batch=#{ENV['FANPLAY_NATIVE_TILEMAP_BATCH'] == '0' ? 0 : 1}",
      "scroll=#{ENV['FANPLAY_NATIVE_SCROLL'] == '0' ? 0 : 1}",
      "world_camera=#{ENV['FANPLAY_WORLD_CAMERA'] == '0' ? 0 : 1}",
      'win32_compat=1',
      "updates=batched",
    ].join(',')
    return if @__fanplay_native_bindings_status == flags

    @__fanplay_native_bindings_status = flags
    JS.global.call(:__fanplay_native_bindings_loaded, "RGSS native bindings loaded (#{flags})")
  rescue StandardError
    # The callback is only present in the production worker. Do not make the
    # Ruby compatibility layer depend on that optional diagnostic endpoint.
    nil
  end

  # Loaded flags describe what was installed. This second report is emitted
  # only after Graphics.update has run and summarizes native families that
  # have demonstrably been exercised by the current game session.
  def self.native_bindings_usage_report
    return unless FANPLAY::NATIVE_USAGE_ENABLED
    operations = NATIVE_USAGE.keys
    stats = defined?(FANPLAY::Retained) ? FANPLAY::Retained.stats : {}
    used = []
    used << 'audio' if operations.any? { |op| op.start_with?('audio_') }
    used << 'bitmap' if operations.any? { |op| op.start_with?('bitmap_') || %w[text_size bitmap_text].include?(op) }
    used << 'viewport' if operations.include?('viewport_sync') || stats[:viewports_synced].to_i.positive?
    used << 'plane' if operations.include?('plane_sync') || stats[:planes_synced].to_i.positive?
    used << 'sprite' if operations.include?('sprite_sync') || stats[:sprites_synced].to_i.positive?
    used << 'tilemap' if operations.include?('sprite_tile') || stats[:legacy_tilemap_rebuilt].to_i.positive? || stats[:map_changed_cells].to_i.positive?
    used << 'retained_state' if stats[:synced].to_i.positive?
    used << 'world_camera' if stats[:world_camera_frames].to_i.positive?
    used << 'scroll' if stats[:bulk_scrolls].to_i.positive?
    used << 'batched_updates' if operations.include?('sync_batch') || operations.include?('native_updates')
    return if used.empty? || @__fanplay_native_usage_status == used

    @__fanplay_native_usage_status = used
    JS.global.call(:__fanplay_native_bindings_loaded, "RGSS native usage observed (#{used.join(',')})")
  rescue StandardError
    nil
  end
end

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
      mark_native_usage('native_updates')
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

  # Seed the canonical RGSS save path before the game scripts start. The
  # browser imports the same file that desktop RGSS uses; keeping the path
  # stable lets SaveData::FILE_PATH discover it on the first title-screen
  # pass, without exposing Ruby's private filesystem to JavaScript.
  def self.__fanplay_import_save(encoded)
    return unless ENV['FANPLAY_WEB'] == '1'

    bytes = encoded.to_s.unpack1('m0')
    # Essentials' RTP.getSaveFileName normally resolves Game.rxdata relative
    # to the game's current VFS directory. Seed that exact path so the title
    # screen can discover an imported save before the first Ruby frame. Keep a
    # copy in the compatibility data directory too for games that explicitly
    # use System.data_directory instead of RTP.getSaveFileName.
    paths = [
      File.join(Dir.pwd, 'Game.rxdata'),
      File.join(System.data_directory, 'Game.rxdata'),
    ].uniq
    written = false
    paths.each do |path|
      begin
        File.binwrite(path, bytes)
        written = true
      rescue StandardError
        # A restricted WASI build may not allow the compatibility directory;
        # the game-directory copy is still sufficient for Essentials.
      end
    end
    written
  end

  # Read the canonical RGSS save from Ruby's private WASI filesystem only when
  # the browser explicitly asks to export it. The browser must not mirror this
  # file into its battery-save store: the VFS is the game's save location.
  def self.__fanplay_export_save
    return unless ENV['FANPLAY_WEB'] == '1'

    # Prefer the save beside the game. That is where RPG Maker Essentials
    # writes RTP.getSaveFileName('Game.rxdata') in the private WASI VFS. The
    # compatibility directory is only a fallback for games that opt into it.
    path = [
      File.join(Dir.pwd, 'Game.rxdata'),
      File.join(System.data_directory, 'Game.rxdata'),
    ].uniq.find { |candidate| File.__fanplay_wasi_file?(candidate) }
    return unless path

    [File.__fanplay_wasi_binread(path)].pack('m0')
  rescue StandardError
    nil
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
        FANPLAY.native_bindings_usage_report
        FANPLAY.startup_yield
      end
    end
    native_bindings_report
  end
end

# Add the virtual asset view after bootstrap.rb has installed its native-file
# compatibility aliases. Writes continue to use ruby.wasm's in-memory WASI
# filesystem. Export/import read and write that same VFS path explicitly; no
# browser-side save mirror is involved.
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
