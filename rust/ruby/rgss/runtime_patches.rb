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

