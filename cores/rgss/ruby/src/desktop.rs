// SPDX-License-Identifier: GPL-2.0-or-later
//! Desktop SabiRuby VM. Each core owns an independent VM on its game thread.
use std::cell::RefCell;
use std::io::{Read, Write};
use sabiruby::{Vm, Value};
use sabiruby::convert::Bytes;

mod host;
mod table;
mod retained_map;
mod sprite_sync;
mod retained;
mod world;
mod tilemap;
mod scroll;
mod win32;
mod audio;

pub struct RubyCore {
    vm: RefCell<Vm>,
    startup_status: Option<Result<i64, String>>,
    _resources: host::Resources,
}

impl RubyCore {
    pub fn new() -> Result<Self, String> {
        let mut core = Self::prepare()?;
        core.start()?;
        Ok(core)
    }

    pub fn prepare() -> Result<Self, String> {
        let mut vm = Vm::new();
        vm.set_host(Box::new(sabiruby_compiler::Compiler::new()));
        vm.load_mrblib().map_err(|e| vm.describe_error(&e))?;
        vm.set_load_path(&["."]);
        vm.wall_clock = Some(|| {
            let d = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
            (d.as_secs() as i64, d.subsec_nanos() as i64)
        });
        vm.sleep_hook = Some(|us| std::thread::sleep(std::time::Duration::from_micros(us)));
        sabiruby_serde::install_json(&mut vm);
        for name in ["__fanplay_native", "__mkxp_native"] {
            vm.define_closure(vm.core.object, name, |vm, _self, args, _blk| {
                if args.len() != 1 { return Err(vm.raise_arg("native bridge expects one request")); }
                let Some(request) = vm.str_bytes(args[0]) else {
                    return Err(vm.raise_type("native bridge request must be a String"));
                };
                match fanplay_rgss::engine::native_call_bytes(request) {
                    Ok(Some(bytes)) => Ok(vm.str_new(&bytes)),
                    Ok(None) => Ok(Value::Nil),
                    Err(message) => Err(vm.raise(vm.core.runtime_error, &message)),
                }
            });
        }
        vm.define_fn(vm.core.object, "__sabi_inflate", |data: Bytes, raw: bool| -> Result<Bytes, String> {
            let mut out = Vec::new();
            let result = if raw {
                flate2::read::DeflateDecoder::new(data.0.as_slice()).read_to_end(&mut out)
            } else {
                flate2::read::ZlibDecoder::new(data.0.as_slice()).read_to_end(&mut out)
            };
            result.map_err(|e| e.to_string())?;
            Ok(Bytes(out))
        });
        vm.define_fn(vm.core.object, "__sabi_deflate", |data: Bytes, level: i64| -> Result<Bytes, String> {
            if !(-1..=9).contains(&level) { return Err("invalid compression level".into()); }
            let compression = if level == -1 { flate2::Compression::default() } else { flate2::Compression::new(level as u32) };
            let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), compression);
            encoder.write_all(&data.0).map_err(|e| e.to_string())?;
            encoder.finish().map(Bytes).map_err(|e| e.to_string())
        });
        let resources=host::install(&mut vm)?;
        table::install(&mut vm, resources.tables())?;
        retained_map::install(&mut vm, resources.tables())?;
        sprite_sync::install(&mut vm);
        retained::install(&mut vm);
        world::install(&mut vm);
        scroll::install(&mut vm);
        tilemap::install(&mut vm);
        win32::install(&mut vm)?;
        audio::install(&mut vm)?;
        let core = Self { vm: RefCell::new(vm), startup_status: None, _resources: resources };
        core.inject_eval(include_str!("desktop/compat.rb"), "fanplay/sabiruby/compat.rb")?;
        core.inject_eval(include_str!("desktop/marshal.rb"), "fanplay/sabiruby/marshal.rb")?;
        Ok(core)
    }

    pub fn start(&mut self) -> Result<i64, String> {
        if let Some(status) = &self.startup_status { return status.clone(); }
        let result = self.eval(fanplay_rgss::ruby::RGSS_SOURCE, "rgss.rb").and_then(|v| match v {
            Value::Int(code) => Ok(code),
            _ => Err("RGSS entry point did not return an integer status".into()),
        });
        self.startup_status = Some(result.clone());
        result
    }

    pub fn startup_status(&self) -> Result<i64, String> {
        self.startup_status.clone().unwrap_or_else(|| Err("Ruby core has not been started".into()))
    }
    pub fn inject_eval(&self, source: &str, name: &str) -> Result<(), String> {
        self.eval(source, name).map(|_| ())
    }
    pub fn extract_i64(&self, source: &str) -> Result<i64, String> {
        match self.eval(source, "<fanplay-extract>")? {
            Value::Int(n) => Ok(n),
            _ => Err("Ruby expression did not return an integer".into()),
        }
    }
    pub fn extract_string(&self, source: &str) -> Result<String, String> {
        let value = self.eval(source, "<fanplay-extract>")?;
        let vm = self.vm.borrow();
        let bytes = vm.str_bytes(value).ok_or("Ruby expression did not return a string")?;
        String::from_utf8(bytes.to_vec()).map_err(|e| e.to_string())
    }
    fn eval(&self, source: &str, name: &str) -> Result<Value, String> {
        let bin = sabiruby_compiler::compile(source.as_bytes(), &sabiruby_compiler::Options {
            filename: name.into(), debug_info: true, ..Default::default()
        }).map_err(|e| e.to_string())?;
        let mut vm = self.vm.try_borrow_mut().map_err(|_| "recursive RubyCore evaluation")?;
        let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| vm.load_and_run(&bin)));
        let result = match run {
            Ok(result) => result.map_err(|e| format!("{name}: {}", vm.describe_error(&e))),
            Err(panic) => { eprintln!("Ruby trace: {:?}",vm.backtrace(None)); std::panic::resume_unwind(panic) }
        };
        // stdout is reserved for framed desktop runner IPC.
        let _ = std::io::stderr().write_all(&vm.take_output());
        result
    }
}

#[cfg(test)]
mod save_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vm_eval_errors_and_independent_instances() {
        let a = RubyCore::prepare().unwrap();
        let b = RubyCore::prepare().unwrap();
        a.inject_eval("$answer=41", "state.rb").unwrap();
        assert_eq!(a.extract_i64("$answer+1").unwrap(),42);
        assert_eq!(b.extract_i64("$answer.nil? ? 1 : 0").unwrap(),1);
        assert!(a.inject_eval("def broken(","broken.rb").unwrap_err().contains("broken.rb"));
        assert!(a.inject_eval("raise 'expected'","error.rb").unwrap_err().contains("expected"));
        assert_eq!(a.extract_i64("6*7").unwrap(),42);
        assert!(a.extract_string("42").is_err());
    }

    #[test]
    fn marshal_compression_and_weak_references() {
        let core=RubyCore::prepare().unwrap();
        core.inject_eval(r#"
          a=[-12345, :name, "\x00\xff".b, 1.25, 2**70]; a << a
          b=Marshal.load(Zlib::Inflate.inflate(Zlib::Deflate.deflate(Marshal.dump(a))))
          raise 'values' unless b[0,5]==a[0,5]
          raise 'cycle' unless b[5].equal?(b)
          # CRuby Marshal.dump([1, "abc", :x]) fixture.
          fixture=[4,8,91,8,105,6,73,34,8,97,98,99,6,58,6,69,84,58,6,120].pack('C*')
          raise 'fixture' unless Marshal.load(fixture)==[1,'abc',:x]
          def make_weak; object=Object.new; WeakRef.new(object); end
          $weak=make_weak
        "#,"runtime-test.rb").unwrap();
        core.inject_eval("GC.start; raise 'weak retained object' if $weak.weakref_alive?","weak-gc.rb").unwrap();
    }

    struct TestHost;
    impl fanplay_rgss::host::Host for TestHost {
        fn read_file(&self,path:&str)->Option<Vec<u8>> { std::fs::read(path).ok() }
        fn image(&self,_:&[u8])->Result<fanplay_rgss::graphics::Bitmap,String> { Err("unused".into()) }
        fn text(&self,_:Option<&[u8]>,_:i32,_:bool,_:bool,_:&str,_:fanplay_rgss::graphics::Color,_:bool)->Result<(u32,u32,Option<fanplay_rgss::graphics::Bitmap>),String> { Ok((0,0,None)) }
    }
    #[test]
    fn desktop_rgss_bootstrap_and_frames() {
        use fanplay_rgss::engine::{self,Runtime,RuntimeOptions};
        engine::set_runtime(Runtime::new(RuntimeOptions {
            width:64,height:48,version:1,headless:true,frame_limit:None,rate:60,
            fast_forward:true,archives:vec![],scripted:vec![],host:Box::new(TestHost),
        }).unwrap());
        let mut core=RubyCore::prepare().unwrap();
        // Exercise the real entry point with a real loaded script, without
        // process-global environment mutation or a system Ruby executable.
        let path=std::env::temp_dir().join(format!("fanplay-sabi-test-{}.rb",std::process::id()));
        std::fs::write(&path,r#"
          raise 'RPG missing' unless RPG::Actor.new.is_a?(RPG::Actor)
          table=Table.new(2,2); table[1,1]=-1234
          raise 'table marshal' unless Marshal.load(Marshal.dump(table))[1,1]==-1234
          bitmap=Bitmap.new(16,16); bitmap.fill_rect(0,0,16,16,Color.new(255,0,0))
          sprite=Sprite.new; sprite.bitmap=bitmap
          3.times { Input.update; Graphics.update }
          sprite.dispose; bitmap.dispose
          $sabi_frames=Graphics.frame_count
        "#).unwrap();
        core.inject_eval(&format!("module ENV; def self.[](key); key=='FANPLAY_SCRIPT' ? {:?} : __sabi_env(key); end; end",path.to_string_lossy()),"test-env.rb").unwrap();
        let status=core.start();
        let _=std::fs::remove_file(path);
        assert_eq!(status.unwrap(),0);
        assert_eq!(core.extract_i64("$sabi_frames").unwrap(),3);
        assert_eq!(core.start().unwrap(),0);
        engine::take_runtime();
    }
}
