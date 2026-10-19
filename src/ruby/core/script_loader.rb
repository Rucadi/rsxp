module RSXP
  # JoiPlay-style games monkeypatch Zlib to route through RSXP.zinflate/zdeflate.
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


eval(__rsxp_retained_source,TOPLEVEL_BINDING,'rsxp/retained.rb',1) if defined?(__rsxp_retained_source)

begin
  if RSXP.call('api_supported')=='1'
    eval(RSXP.call('api_game_bridge').force_encoding(Encoding::UTF_8),TOPLEVEL_BINDING,'api/game_bridge.rb',1)
  end
  eval(RSXP.call('rpg').force_encoding(Encoding::UTF_8),TOPLEVEL_BINDING,"<RPG#{RGSS_VERSION}>",1)
  RSXP.startup_yield
  RSXP.before_scripts
  # rsxp executes a game-provided preload script before the main scripts.
  load File.expand_path('preload.rb') if FileTest.exist?('preload.rb')
  RSXP.startup_yield
  if ENV['RSXP_SCRIPT']
    load ENV.fetch('RSXP_SCRIPT')
    RSXP.publish_top_level_methods
    RSXP.startup_yield
  else
    $RGSS_SCRIPTS=load_data(ENV.fetch('RSXP_SCRIPTS'))
    raise TypeError, 'script bundle must contain an Array' unless $RGSS_SCRIPTS.is_a?(Array)
    # Decompress all sections first, as the original engine does. Keep decoded
    # source in slot 3 for scripts that inspect $RGSS_SCRIPTS.
    $RGSS_SCRIPTS.each_with_index do |section,index|
      raise TypeError, "invalid script section #{index}" unless section.is_a?(Array) && section.size>=3 && section[1].is_a?(String) && section[2].is_a?(String)
      section[3]=Zlib::Inflate.inflate(section[2]).force_encoding(Encoding::UTF_8)
      RSXP.startup_yield
    end
    $RGSS_SCRIPTS.each do |section|
      RSXP::ERROR_OUTPUT.call("Loading #{section[1]}\n") if ENV['RSXP_TRACE']=='1'
      RSXP.eval_script(section[3],section[1].encode(Encoding::UTF_8,invalid: :replace,undef: :replace))
      # Keep Kernel.foo working for helpers defined so far; the final Main
      # section runs the game loop as soon as it is evaluated.
      RSXP.publish_top_level_methods
      RSXP.startup_yield
    end
  end
  0
rescue RSXPStop
  0
rescue SystemExit => error
  error.status || 0
rescue Exception => error
  RSXP::ERROR_OUTPUT.call(RSXP::ERROR_FORMAT.bind_call(error,highlight:false))
  1
end
