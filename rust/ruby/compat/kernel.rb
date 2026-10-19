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

