// SPDX-License-Identifier: GPL-2.0-or-later
use sabiruby::{Vm, Value};
use sabiruby::convert::Bytes;
use std::io::{Read, Write, Seek};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::table::{Slot, Tables};

pub(super) struct Resources {
    resources: Arc<Mutex<HashMap<u64, Vec<u8>>>>,
    tables: Tables,
}
impl Resources {
    pub(super) fn tables(&self) -> Tables { self.tables.clone() }
}
impl Drop for Resources {
    fn drop(&mut self) {
        // Vm::drop does not invoke Data's on_free hook. Release live resources
        // before the owning core removes its thread-local engine.
        for (_, request) in self.resources.lock().unwrap().drain() {
            let _ = fanplay_rgss::engine::native_call_bytes(&request);
        }
    }
}

pub(super) fn install(vm: &mut Vm) -> Result<Resources, String> {
    let object = vm.core.object;
    vm.define_fn(object,"__sabi_encoding",|vm: &mut Vm,value: Value| value.obj().map(|id|vm.ivar_get(id,"@__sabi_encoding")));
    vm.define_fn(object,"__sabi_set_encoding",|vm: &mut Vm,value: Value,name: Value| { if let Some(id)=value.obj() { vm.ivar_set(id,"@__sabi_encoding",name); } });
    vm.define_fn(object, "__sabi_binary", |vm: &mut Vm, value: Value, binary: bool| { vm.str_set_binary(value, binary); });
    vm.define_fn(object, "__sabi_is_binary", |vm: &mut Vm, value: Value| vm.str_binary(value));
    vm.define_fn(object, "__sabi_valid_utf8", |data: Bytes| std::str::from_utf8(&data.0).is_ok());
    vm.define_fn(object, "__sabi_transcode", |data: Bytes, encoding: String, replace: bool| -> Result<String,String> {
        match encoding.as_str() {
            "UTF-8" | "US-ASCII" | "ASCII-8BIT" => {
                if replace { Ok(String::from_utf8_lossy(&data.0).into_owned()) }
                else { String::from_utf8(data.0).map_err(|e| e.to_string()) }
            }
            "ISO-8859-1" => Ok(data.0.iter().map(|&b| char::from(b)).collect()),
            "Windows-1252" => {
                const C1: [u16; 32] = [0x20ac,0x81,0x201a,0x192,0x201e,0x2026,0x2020,0x2021,0x2c6,0x2030,0x160,0x2039,0x152,0x8d,0x17d,0x8f,0x90,0x2018,0x2019,0x201c,0x201d,0x2022,0x2013,0x2014,0x2dc,0x2122,0x161,0x203a,0x153,0x9d,0x17e,0x178];
                Ok(data.0.iter().map(|&b| if (128..160).contains(&b) { char::from_u32(C1[(b-128) as usize] as u32).unwrap() } else { char::from(b) }).collect())
            }
            _ => Err(format!("unsupported source encoding: {encoding}")),
        }
    });
    vm.define_fn(object, "__sabi_env", |key: String| std::env::var(key).ok());
    vm.define_fn(object, "__sabi_pwd", || std::env::current_dir().map(|p| p.to_string_lossy().into_owned()).map_err(|e| e.to_string()));
    vm.define_fn(object, "__sabi_exists", |path: String| std::path::Path::new(&path).exists());
    vm.define_fn(object, "__sabi_directory", |path: String| std::path::Path::new(&path).is_dir());
    vm.define_fn(object, "__sabi_file", |path: String| std::path::Path::new(&path).is_file());
    vm.define_fn(object, "__sabi_mkdir", |path: String| std::fs::create_dir_all(path).map_err(|e| e.to_string()));
    vm.define_fn(object, "__sabi_chdir", |path: String| std::env::set_current_dir(path).map_err(|e| e.to_string()));
    vm.define_fn(object, "__sabi_rmdir", |path: String| std::fs::remove_dir(path).map_err(|e| e.to_string()));
    vm.define_fn(object, "__sabi_delete", |path: String| std::fs::remove_file(path).map_err(|e| e.to_string()));
    vm.define_fn(object, "__sabi_rename", |a: String, b: String| std::fs::rename(a,b).map_err(|e| e.to_string()));
    vm.define_fn(object, "__sabi_read", |path: String| std::fs::read(path).map(Bytes).map_err(|e| e.to_string()));
    vm.define_fn(object, "__sabi_write", |path: String, data: Bytes| -> Result<i64,String> {
        std::fs::write(path, &data.0).map_err(|e| e.to_string())?; Ok(data.0.len() as i64)
    });
    vm.define_fn(object, "__sabi_children", |path: String| -> Result<Vec<String>, String> {
        std::fs::read_dir(path).map_err(|e| e.to_string())?.map(|e| e.map(|e| e.file_name().to_string_lossy().into_owned()).map_err(|e| e.to_string())).collect()
    });
    vm.define_fn(object, "__sabi_glob", |pattern: String| -> Result<Vec<String>, String> {
        glob::glob(&pattern).map_err(|e| e.to_string())?.map(|p| p.map(|p| p.to_string_lossy().into_owned()).map_err(|e| e.to_string())).collect()
    });
    // mruby-time's public Time.at accepts microseconds. Preserve CRuby's
    // nanoseconds in its hidden time storage when loading saves/file metadata.
    vm.define_method(object, "__sabi_time_nsec", |vm, _self, args, _blk| {
        if args.len() != 2 { return Err(vm.raise_arg("expected Time and nanoseconds")); }
        let nsec = vm.expect_int(args[1], "nanoseconds")?;
        if !(0..1_000_000_000).contains(&nsec) { return Err(vm.raise_arg("invalid nanoseconds")); }
        let obj = args[0].obj().ok_or_else(|| vm.raise_type("expected Time"))?;
        let time_name = vm.intern("Time");
        let time_class = vm.const_get(vm.core.object, time_name).and_then(|v| v.obj()).unwrap();
        if !vm.obj_is_kind_of(args[0], time_class) { return Err(vm.raise_type("expected Time")); }
        let key = vm.intern("__nsec");
        vm.heap.ivar_set(obj, key, Value::Int(nsec));
        Ok(args[0])
    });
    vm.define_fn(object, "__sabi_stat", |path: String| -> Result<Vec<i64>, String> {
        let m = std::fs::metadata(path).map_err(|e| e.to_string())?;
        let stamp = |t: std::io::Result<std::time::SystemTime>| t.ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos() as i64);
        #[cfg(unix)]
        let changed = {
            use std::os::unix::fs::MetadataExt;
            m.ctime() * 1_000_000_000 + m.ctime_nsec()
        };
        #[cfg(not(unix))]
        let changed = stamp(m.created());
        Ok(vec![m.len() as i64, stamp(m.modified()), changed])
    });
    vm.define_fn(object, "__sabi_clock", || {
        static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        START.get_or_init(std::time::Instant::now).elapsed().as_secs_f64()
    });
    vm.define_fn(object, "__sabi_pid", || std::process::id() as i64);
    vm.define_fn(object, "__sabi_platform", || std::env::consts::OS.to_owned());
    vm.define_fn(object, "__sabi_stderr", |data: Bytes| -> Result<i64,String> {
        std::io::stderr().write_all(&data.0).map_err(|e| e.to_string())?; Ok(data.0.len() as i64)
    });
    // File handles live in Rust and are reclaimed when their Data token is collected.
    let files = Arc::new(Mutex::new(HashMap::<u64, std::fs::File>::new()));
    let weak = Arc::new(Mutex::new(HashMap::<u64, sabiruby::value::ObjId>::new()));
    let resources = Arc::new(Mutex::new(HashMap::<u64, Vec<u8>>::new()));
    let tables: Tables = Arc::new(Mutex::new(HashMap::<u64, Slot>::new()));
    let serial = Arc::new(std::sync::atomic::AtomicU64::new(1));
    let token_class = vm.define_class("FanplayHostToken", object);
    let f = files.clone(); let w = weak.clone(); let r = resources.clone(); let t = tables.clone();
    vm.set_on_free(Box::new(move |tag, id| {
        match tag {
            1 => { f.lock().unwrap().remove(&id); }
            2 => { w.lock().unwrap().remove(&id); }
            3 => { if let Some(request) = r.lock().unwrap().remove(&id) { let _ = fanplay_rgss::engine::native_call_bytes(&request); } }
            4 => { t.lock().unwrap().remove(&id); }
            5 => { super::retained_map::forget(id); }
            6 => { super::win32::forget(id); }
            _ => {}
        }
    }));
    let f = files.clone(); let seq = serial.clone();
    vm.define_fn(object, "__sabi_open", move |vm: &mut Vm, path: String, mode: String| -> Result<Value,String> {
        let mut opts = std::fs::OpenOptions::new();
        let plus = mode.contains('+');
        match mode.chars().next() {
            Some('r') => { opts.read(true).write(plus); }
            Some('w') => { opts.write(true).read(plus).create(true).truncate(true); }
            Some('a') => { opts.append(true).read(plus).create(true); }
            _ => return Err(format!("invalid file mode: {mode}")),
        }
        let file = opts.open(path).map_err(|e| e.to_string())?;
        let id = seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        f.lock().unwrap().insert(id, file);
        Ok(vm.data_new(token_class, 1, id))
    });
    let f = files.clone();
    vm.define_fn(object, "__sabi_io", move |vm: &mut Vm, token: Value, op: String, data: Bytes, n: i64| -> Result<Value,String> {
        let (_, id) = vm.data_of(token).ok_or("invalid file handle")?;
        let mut files = f.lock().unwrap();
        if op == "close" { files.remove(&id); return Ok(Value::Nil); }
        let file = files.get_mut(&id).ok_or("closed stream")?;
        match op.as_str() {
            "read" => { let mut out=Vec::new(); if n < 0 { file.read_to_end(&mut out) } else { file.take(n as u64).read_to_end(&mut out) }.map_err(|e|e.to_string())?; if n > 0 && out.is_empty() { Ok(Value::Nil) } else { Ok(vm.str_new(&out)) } }
            "write" => { file.write_all(&data.0).map_err(|e|e.to_string())?; Ok(Value::Int(data.0.len() as i64)) }
            "pos" => file.stream_position().map(|n|Value::Int(n as i64)).map_err(|e|e.to_string()),
            "seek" => file.seek(std::io::SeekFrom::Start(n.try_into().map_err(|_|"negative seek")?)).map(|_|Value::Int(0)).map_err(|e|e.to_string()),
            "size" => file.metadata().map(|m|Value::Int(m.len() as i64)).map_err(|e|e.to_string()),
            _ => Err(format!("unsupported file operation {op}")),
        }
    });
    let w=weak.clone(); let seq=serial.clone();
    vm.define_fn(object, "__sabi_weak", move |vm: &mut Vm, value: Value| -> Result<i64,String> {
        let obj=value.obj().ok_or("weak reference requires an object")?;
        let existing=vm.ivar_get(obj,"@__sabi_weak_token");
        if let Some((2,id))=vm.data_of(existing) { return Ok(id as i64); }
        let id=seq.fetch_add(1,std::sync::atomic::Ordering::Relaxed);
        let token=vm.data_new(token_class,2,id);
        vm.ivar_set(obj,"@__sabi_weak_token",token);
        w.lock().unwrap().insert(id,obj); Ok(id as i64)
    });
    vm.define_fn(object, "__sabi_weak_get", move |id: i64| weak.lock().unwrap().get(&(id as u64)).copied().map(Value::Obj));
    let r=resources.clone(); let seq=serial;
    vm.define_fn(object, "__sabi_own", move |vm: &mut Vm, value: Value, op: String, engine_id: i64| -> Result<(),String> {
        let obj=value.obj().ok_or("resource requires an object")?;
        let id=seq.fetch_add(1,std::sync::atomic::Ordering::Relaxed);
        let token=vm.data_new(token_class,3,id);
        vm.ivar_set(obj,"@__sabi_resource_token",token);
        r.lock().unwrap().insert(id,format!("{op}\t{engine_id}\n").into_bytes()); Ok(())
    });
    let remaining = resources.clone();
    vm.define_fn(object,"__sabi_disown",move |vm: &mut Vm, value: Value| {
        if let Some(obj)=value.obj() { if let Some((3,id))=vm.data_of(vm.ivar_get(obj,"@__sabi_resource_token")) { resources.lock().unwrap().remove(&id); } }
    });
    Ok(Resources { resources: remaining, tables })
}
