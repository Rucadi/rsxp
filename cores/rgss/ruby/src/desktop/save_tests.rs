// SPDX-License-Identifier: GPL-2.0-or-later
use super::RubyCore;

#[test]
#[ignore = "requires a local game test script; set FANPLAY_SAVE_CHECK_SCRIPT"]
fn local_game_save_roundtrip() {
    use fanplay_rgss::engine::{self, Runtime, RuntimeOptions};
    struct Host;
    impl fanplay_rgss::host::Host for Host {
        fn read_file(&self,path:&str)->Option<Vec<u8>> { std::fs::read(path).ok() }
        fn image(&self,_:&[u8])->Result<fanplay_rgss::graphics::Bitmap,String> { Err("image not used in save test".into()) }
        fn text(&self,_:Option<&[u8]>,_:i32,_:bool,_:bool,_:&str,_:fanplay_rgss::graphics::Color,_:bool)->Result<(u32,u32,Option<fanplay_rgss::graphics::Bitmap>),String> { Ok((0,0,None)) }
    }
    let script=std::env::var("FANPLAY_SAVE_CHECK_SCRIPT").expect("set FANPLAY_SAVE_CHECK_SCRIPT");
    engine::set_runtime(Runtime::new(RuntimeOptions {
        width:64,height:48,version:1,headless:true,frame_limit:None,rate:60,
        fast_forward:true,archives:vec![],scripted:vec![],host:Box::new(Host),
    }).unwrap());
    let mut core=RubyCore::prepare().unwrap();
    core.inject_eval(&format!(r#"
      module ENV
        def self.[](key)
          return {:?} if key=='FANPLAY_SCRIPT'
          return '/tmp/fanplay-save-check' if key=='FANPLAY_HOME'
          __sabi_env(key)
        end
      end
    "#,script),"save-test-env.rb").unwrap();
    let result=core.start();
    engine::take_runtime();
    assert_eq!(result.unwrap(),0);
}

#[test]
fn save_slot_file_timestamps_are_times() {
    let path = std::env::temp_dir().join(format!("fanplay-save-mtime-{}", std::process::id()));
    std::fs::write(&path, b"save fixture").unwrap();
    let core = RubyCore::prepare().unwrap();
    let result = core.inject_eval(&format!(r#"
      path={:?}
      stamp=File.mtime(path)
      raise 'timestamp type' unless stamp.is_a?(Time)
      raise 'stat timestamp' unless stamp==File.stat(path).mtime
      raise 'timestamp epoch' unless (Time.now-stamp).abs < 60
      raise 'timestamp format' unless stamp.strftime('%Y').size==4
      raise 'ctime type' unless File.ctime(path).is_a?(Time)
      raise 'size' unless File.stat(path).size==12
    "#, path.to_string_lossy()), "save-timestamp.rb");
    std::fs::remove_file(path).unwrap();
    result.unwrap();
}

#[test]
fn time_marshal_roundtrip_preserves_precision_and_links() {
    RubyCore::prepare().unwrap().inject_eval(r#"
      time=__sabi_time_nsec(Time.utc(2026,10,27,12,34,56),123456789)
      time.instance_variable_set(:@label,'save')
      loaded=Marshal.load(Marshal.dump([time,time,'after']))
      raise 'time type' unless loaded[0].is_a?(Time)
      raise 'time value' unless loaded[0]==time && loaded[0].utc?
      raise 'nanoseconds' unless loaded[0].nsec==123456789
      raise 'time ivars' unless loaded[0].instance_variable_get(:@label)=='save'
      raise 'object link' unless loaded[0].equal?(loaded[1])
      raise 'after time' unless loaded[2]=='after'
      # Independent eight-byte CRuby time.c layout for UTC 2026-10-27.
      fixture=[3223299948,2340545088].pack('V2')
      t=Time._load(fixture)
      raise 'CRuby layout' unless [t.year,t.month,t.day,t.hour,t.min,t.sec,t.usec]==[2026,10,27,12,34,56,123456]
      raise 'CRuby layout dump' unless t._dump==fixture
      legacy=Time._load([123456,654321].pack('V2'))
      raise 'old format' unless legacy.to_i==123456 && legacy.usec==654321
      fixture=[4,8,91,8,73,117,58,9,84,105,109,101,13,108,167,31,128,64,226,129,139,10,58,13,110,97,110,111,95,110,117,109,105,2,21,3,58,13,110,97,110,111,95,100,101,110,105,6,58,13,115,117,98,109,105,99,114,111,34,7,120,144,58,11,111,102,102,115,101,116,105,2,32,28,58,9,122,111,110,101,48,64,7,73,34,10,97,102,116,101,114,6,58,6,69,84].pack('C*')
      loaded=Marshal.load(fixture)
      raise 'CRuby nanoseconds' unless loaded[0].nsec==123456789
      raise 'CRuby links' unless loaded[0].equal?(loaded[1]) && loaded[2]=='after'
      again=Marshal.load(Marshal.dump(loaded))
      raise 'local links' unless again[0].equal?(again[1]) && again[2]=='after'
      text='Pokémon'.force_encoding(Encoding::UTF_8)
      binary="\xff".b
      # CRuby's I/string encoding record must survive saving non-ASCII names.
      dumped=Marshal.dump([text,text,binary])
      strings=Marshal.load(dumped)
      raise 'text encoding' unless strings[0].encoding.name=='UTF-8' && strings[0]==text
      raise 'string links' unless strings[0].equal?(strings[1])
      raise 'binary encoding' unless strings[2].encoding.name=='ASCII-8BIT'
    "#, "save-time.rb").unwrap();
}
