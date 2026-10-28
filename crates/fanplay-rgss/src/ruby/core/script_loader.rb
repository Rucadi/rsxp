module FANPLAY
  # JoiPlay-style games monkeypatch Zlib to route through FANPLAY.zinflate/zdeflate.
  # The streaming Zlib API is used directly so the game's
  # Zlib::Inflate.inflate / Zlib::Deflate.deflate overrides cannot recurse back
  # into themselves, and a raw deflate stream is accepted as a fallback.
  def self.zinflate(string)
    source=string.to_s.b
    [Zlib::MAX_WBITS,-Zlib::MAX_WBITS].each do |wbits|
      inflater=Zlib::Inflate.new(wbits)
      begin
        data=inflater.inflate(source)
        data<< inflater.finish unless inflater.finished?
        return data
      rescue Zlib::Error
        nil
      ensure
        inflater.close
      end
    end
    raise Zlib::DataError,'invalid compressed data'
  end
  def self.zdeflate(string, level=Zlib::DEFAULT_COMPRESSION)
    deflater=Zlib::Deflate.new(level)
    begin
      deflater.deflate(string.to_s.b,Zlib::FINISH)
    ensure
      deflater.close
    end
  end
end


eval(__fanplay_retained_source,TOPLEVEL_BINDING,'fanplay/retained.rb',1) if defined?(__fanplay_retained_source)

begin
  eval(FANPLAY.call('rpg').force_encoding(Encoding::UTF_8),TOPLEVEL_BINDING,"<RPG#{RGSS_VERSION}>",1)
  FANPLAY.startup_yield
  FANPLAY.before_scripts
  # fanplay executes a game-provided preload script before the main scripts.
  load File.expand_path('preload.rb') if FileTest.exist?('preload.rb')
  FANPLAY.startup_yield
  if ENV['FANPLAY_SCRIPT']
    load ENV.fetch('FANPLAY_SCRIPT')
    FANPLAY.publish_top_level_methods
    FANPLAY.startup_yield
  else
    $RGSS_SCRIPTS=load_data(ENV.fetch('FANPLAY_SCRIPTS'))
    raise TypeError, 'script bundle must contain an Array' unless $RGSS_SCRIPTS.is_a?(Array)
    # Decompress all sections first, as the original engine does. Keep decoded
    # source in slot 3 for scripts that inspect $RGSS_SCRIPTS.
    $RGSS_SCRIPTS.each_with_index do |section,index|
      raise TypeError, "invalid script section #{index}" unless section.is_a?(Array) && section.size>=3 && section[1].is_a?(String) && section[2].is_a?(String)
      section[3]=Zlib::Inflate.inflate(section[2]).force_encoding(Encoding::UTF_8)
      FANPLAY.startup_yield
    end
    $RGSS_SCRIPTS.each do |section|
      FANPLAY::ERROR_OUTPUT.call("Loading #{section[1]}\n") if ENV['FANPLAY_TRACE']=='1'
      FANPLAY.eval_script(section[3],section[1].encode(Encoding::UTF_8,invalid: :replace,undef: :replace))
      # Keep Kernel.foo working for helpers defined so far; the final Main
      # section runs the game loop as soon as it is evaluated.
      FANPLAY.publish_top_level_methods
      FANPLAY.startup_yield
    end
  end
  0
rescue FANPLAYStop
  0
rescue SystemExit => error
  error.status || 0
rescue Exception => error
  FANPLAY::ERROR_OUTPUT.call(FANPLAY::ERROR_FORMAT.bind_call(error,highlight:false))
  1
end
