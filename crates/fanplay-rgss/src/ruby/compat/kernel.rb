module Kernel
  # mkxp-z's optional raw flag bypasses Marshal for archive-backed file reads.
  def load_data(path, raw=false)
    bytes=begin
      FANPLAY.call('read',payload:String(path))
    rescue => error
      # mkxp reports a missing asset through the filesystem error that games
      # rescue while probing optional files (Essentials' pbGetFileChar uses
      # load_data and expects Errno::ENOENT for absent graphics). Restricted
      # engine's asset store does not see files written to Ruby's virtual
      # filesystem, so fall back to a normal read before reporting the miss.
      raise unless error.message.start_with?('asset not found: ')
      File.binread(path)
    end
    return bytes if raw
    data=Marshal.load(bytes)
    # Modern Ruby rejects `class Foo < Foo` (superclass mismatch) and would raise
    # a NameError if Foo is undefined. Some Essentials plugins contain this typo
    # while intending to reopen the class, so repair it in memory as legacy RGSS
    # games expect.
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
          # This engine's pattern library has no backreferences, so capture the
          # superclass and compare it to the class name in the block.
          fixed=source.gsub(/^([ \t]*)class ([A-Z]\w*(?:::\w+)*) < ([A-Z]\w*(?:::\w+)*)([ \t]*\r?)$/) do
            if $2==$3
              "#{$1}class #{$2}"
            else
              "#{$1}class #{$2} < #{$3}#{$4}"
            end
          end
          next if fixed==source
          script[1]=Zlib.deflate(fixed)
          FANPLAY::ERROR_OUTPUT.call("Repaired redundant class self-inheritance in #{script[0]}, in memory.\n")
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
    rescue Exception => error
      # Some Essentials forks raise their own Reset exception from title
      # screen styles instead of using the engine's RGSSReset class.
      raise error unless error.class.name == 'Reset'
      retry
    end
  end
  def rgss_stop
    loop { Graphics.update }
  end
  def msgbox(*messages) = puts(*messages)
  def msgbox_p(*messages) = p(*messages)
end

# Essentials' plugin loader and event interpreter call Kernel#eval directly,
# bypassing prepare_script. Apply the same compatibility rewrites there; the
# native eval still receives the caller's binding and self.
module Kernel
  alias_method :__fanplay_native_eval, :eval unless method_defined?(:__fanplay_native_eval)
  def eval(source,binding=nil,filename='(eval)',lineno=1)
    source=FANPLAY.prepare_script(source) if source.is_a?(String)
    __fanplay_native_eval(source,binding,filename,lineno)
  end
  module_function :eval

  # Cable Club pulls in the CRuby socket extension, which does not exist here.
  # It is optional online-play code; skip the native libraries so the plugin
  # can load for offline play.
  alias_method :__fanplay_native_require, :require unless method_defined?(:__fanplay_native_require)
  def require(name)
    return false if %w[socket socket.so io/wait io/wait.so].include?(name.to_s)
    __fanplay_native_require(name)
  end
  module_function :require
end

