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
      FANPLAY.call('runtime_patch_states').each_line do |line|
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
              FANPLAY::ERROR_OUTPUT.call("[RuntimePatches] enabled #{id}\n")
            end
          rescue StandardError => error
            @failed[id]=true
            FANPLAY::ERROR_OUTPUT.call("[RuntimePatches] #{id}: #{error.class}: #{error.message}\n")
          end
        elsif active
          patch=@patches[id]
          next if patch.nil? || patch==false
          begin
            patch[:revert].call
            @active[id]=false
            FANPLAY::ERROR_OUTPUT.call("[RuntimePatches] disabled #{id}\n")
          rescue StandardError => error
            @failed[id]=true
            FANPLAY::ERROR_OUTPUT.call("[RuntimePatches] #{id}: #{error.class}: #{error.message}\n")
          end
        end
      end
    rescue StandardError => error
      FANPLAY::ERROR_OUTPUT.call("[RuntimePatches] #{error.class}: #{error.message}\n")
    ensure
      @syncing=false
    end
    nil
  end
  def self.load_patch(id)
    source=FANPLAY.call('runtime_patch_source',id)
    return nil if source.nil? || source.empty?
    patch=eval(source.dup.force_encoding(Encoding::UTF_8),TOPLEVEL_BINDING,"runtime patch #{id}",1)
    return nil unless patch.is_a?(Hash) && patch[:apply].respond_to?(:call) && patch[:revert].respond_to?(:call)
    patch
  end
  def self.active?(id) = @active[id]==true

  # Called by the extension bridge before a dynamically registered patch is
  # removed from the native catalog. This gives the patch the same reversible
  # lifecycle as a checkbox toggle instead of leaving method aliases behind.
  def self.disable(id)
    patch=@patches[id]
    return false unless patch && patch!=false
    unless @active[id]
      @patches.delete(id)
      @values.delete(id)
      return true
    end
    patch[:revert].call
    @active.delete(id)
    @values.delete(id)
    @patches.delete(id)
    true
  rescue StandardError => error
    FANPLAY::ERROR_OUTPUT.call("[RuntimePatches] #{id}: #{error.class}: #{error.message}\n")
    false
  end
end

# Transport is deliberately kept in the core Ruby shim so a Rhai extension can
# inject the game-specific dispatcher itself. This first processes the
# bootstrap mutations (ruby_inject/ruby_unload); subsequent requests are
# delegated to FANPLAY::GameAPI once the extension has installed it.
module FANPLAY
  module ExtensionBridge
    # A webview or external API client may disappear while a game-thread
    # operation is still running. Dropping that reply must not turn into a
    # Ruby exception from Input.update and terminate the game session.
    def self.respond(id, payload)
      FANPLAY.call('api_respond', id, payload: payload)
    rescue StandardError
      nil
    end

    def self.poll
      if defined?(FANPLAY::GameAPI) && FANPLAY::GameAPI.respond_to?(:start_pending_wild_encounter)
        FANPLAY::GameAPI.start_pending_wild_encounter
      end
      FANPLAY.call('api_poll').force_encoding(Encoding::UTF_8).each_line do |line|
        id, method, *args = line.chomp.split("\t")
        begin
          result = dispatch(method, args)
          respond(id.to_i, JSON.generate(result))
        rescue StandardError => error
          payload = JSON.generate({ 'error' => "#{error.class}: #{error.message}" })
          respond(id.to_i, payload)
        end
      end
      nil
    end

    def self.dispatch(method, args)
      case method
      when 'ruby_inject'
        name = args.fetch(0)
        source = decode_hex(args.fetch(1))
        patch_id = FANPLAY.call('runtime_patch_register', name, payload: source)
        # Requests already queued behind the injection in this frame can use
        # the newly installed dispatcher immediately.
        RuntimePatches.sync
        { 'patch_id' => patch_id, 'registered' => true }
      when 'ruby_unload'
        patch_id = args.fetch(0)
        raise ArgumentError, 'unknown Ruby patch handle' unless RuntimePatches.disable(patch_id)
        FANPLAY.call('runtime_patch_unregister', patch_id)
        { 'patch_id' => patch_id, 'unloaded' => true }
      else
        unless defined?(FANPLAY::GameAPI) && FANPLAY::GameAPI.respond_to?(:dispatch)
          raise RuntimeError, 'the game API extension is not active yet'
        end
        FANPLAY::GameAPI.dispatch(method, args)
      end
    end

    def self.decode_hex(value)
      raise ArgumentError, 'Ruby source encoding is invalid' unless value.length.even? && value.match?(/\A[0-9a-fA-F]*\z/)
      [value].pack('H*').force_encoding(Encoding::UTF_8)
    end
  end
end

