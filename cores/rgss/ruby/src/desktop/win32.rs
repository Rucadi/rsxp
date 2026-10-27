// SPDX-License-Identifier: GPL-2.0-or-later
//! Legacy `Win32API` native calls for SabiRuby.
//!
//! This is the SabiRuby port of the Magnus `bindings/win32` pair. The Ruby
//! compatibility shim (`fanplay_rgss::ruby::compat::win32_api.rb`) still owns
//! the portable `Win32API_Impl` fallback; the class installed here only exposes
//! the native three-to-five-argument constructor, its `call`/`Call` alias and
//! the real DLL call on Windows.
//!
//! `initialize` accepts DLL, function, argument type string (or Array joined by
//! `Array#join`), optional return type and optional calling convention. Type
//! codes are parsed by [`Kind`] (`Void`/`Integer`/`Pointer`) and [`Abi`]
//! (`stdcall`→`System`, `cdecl`); [`inputs`] rejects mixed void codes and more
//! than 16 arguments. `call` marshals arguments to machine words before
//! dispatching. On Windows mutable String arguments are copied into Rust-owned
//! buffers, the native function runs, and changed bytes are copied back. Off
//! Windows `initialize` raises `RuntimeError`, which the shim rescues to fall
//! back to its portable implementation.
//!
//! Instances are `Vm::data_new` objects with host tag [`TAG`]; the procedure
//! lives in [`PROCEDURES`], so no Rust pointer is ever stored in a Ruby value.
//! [`forget`] drops a slot when the owning VM reports the data object freed.
use sabiruby::{object::Aspec, Value, Vm, VmError};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

/// Host-data tag for a `Win32API` handle in `Vm::data_of`.
pub(super) const TAG: u32 = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Void,
    Integer,
    Pointer,
}

impl Kind {
    fn parse(code: char) -> Result<Self, String> {
        match code {
            '0' | 'v' | 'V' => Ok(Self::Void),
            'i' | 'I' | 'l' | 'L' | 'n' | 'N' => Ok(Self::Integer),
            'p' | 'P' | 's' | 'S' => Ok(Self::Pointer),
            _ => Err(format!("unsupported Win32API type: {code}")),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Abi {
    System,
    Cdecl,
}

impl Abi {
    fn parse(name: &str) -> Result<Self, String> {
        match name {
            "stdcall" => Ok(Self::System),
            "cdecl" => Ok(Self::Cdecl),
            _ => Err(format!("unsupported Win32API calling convention: {name}")),
        }
    }
}

struct Procedure {
    inputs: Vec<Kind>,
    output: Kind,
    #[cfg(windows)]
    target: native::LibraryFunction,
}

static PROCEDURES: LazyLock<Mutex<HashMap<u64, Arc<Procedure>>>> =
    LazyLock::new(Mutex::default);
static SERIAL: AtomicU64 = AtomicU64::new(1);

/// Releases the procedure named by a collected `Win32API` data object.
///
/// The desktop `on_free` hook is installed once for all host data; the
/// integration calls this for [`TAG`] handles, in the same way it already
/// dispatches the other tags.
pub(super) fn forget(id: u64) {
    PROCEDURES.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
}

fn inputs(proto: &str) -> Result<Vec<Kind>, String> {
    let kinds = proto
        .chars()
        .map(Kind::parse)
        .collect::<Result<Vec<_>, _>>()?;
    if kinds.iter().all(|kind| *kind == Kind::Void) {
        return Ok(Vec::new());
    }
    if kinds.contains(&Kind::Void) {
        return Err("void is only valid for an empty Win32API argument list".into());
    }
    if kinds.len() > 16 {
        return Err("Win32API supports at most 16 arguments".into());
    }
    Ok(kinds)
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// `String.try_convert`: a String as it is, otherwise the result of `to_str`.
fn string_value(vm: &mut Vm, value: Value) -> Result<Vec<u8>, VmError> {
    if let Some(bytes) = vm.str_bytes(value) {
        return Ok(bytes.to_vec());
    }
    let to_str = vm.intern("to_str");
    let converted = vm.funcall(value, to_str, &[], Value::Nil)?;
    vm.expect_str(converted, "string")
}

fn procedure(vm: &mut Vm, this: Value) -> Result<Arc<Procedure>, VmError> {
    let uninitialized = "uninitialized native Win32API";
    let (tag, id) = vm
        .data_of(this)
        .ok_or_else(|| vm.raise(vm.core.runtime_error, uninitialized))?;
    if tag != TAG {
        return Err(vm.raise(vm.core.runtime_error, uninitialized));
    }
    PROCEDURES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&id)
        .cloned()
        .ok_or_else(|| vm.raise(vm.core.runtime_error, uninitialized))
}

fn initialize(vm: &mut Vm, this: Value, args: &[Value]) -> Result<Value, VmError> {
    if !(3..=5).contains(&args.len()) {
        return Err(vm.raise_arg(
            "expected DLL, function, argument types, optional return type and calling convention",
        ));
    }
    let dll = text(&string_value(vm, args[0])?);
    let function = text(&string_value(vm, args[1])?);
    let proto = if vm.ary_vals(args[2]).is_some() {
        let join = vm.intern("join");
        let joined = vm.funcall(args[2], join, &[], Value::Nil)?;
        text(&string_value(vm, joined)?)
    } else {
        text(&string_value(vm, args[2])?)
    };
    let input = inputs(&proto).map_err(|message| vm.raise_arg(&message))?;
    let output = if args.len() >= 4 {
        text(&string_value(vm, args[3])?)
    } else {
        "0".into()
    };
    let output = match output.chars().collect::<Vec<_>>().as_slice() {
        [] => Ok(Kind::Void),
        [code] => Kind::parse(*code),
        _ => Err("Win32API return type must contain one type code".into()),
    }
    .map_err(|message| vm.raise_arg(&message))?;
    let abi = if args.len() == 5 {
        let to_s = vm.intern("to_s");
        let value = vm.funcall(args[4], to_s, &[], Value::Nil)?;
        text(&string_value(vm, value)?)
    } else {
        "stdcall".to_string()
    };
    let abi = Abi::parse(&abi).map_err(|message| vm.raise_arg(&message))?;
    #[cfg(windows)]
    {
        let target = native::LibraryFunction::open(&dll, &function, abi)
            .map_err(|message| vm.raise(vm.core.runtime_error, &message))?;
        let (tag, id) = vm
            .data_of(this)
            .ok_or_else(|| vm.raise(vm.core.runtime_error, "uninitialized native Win32API"))?;
        if tag != TAG {
            return Err(vm.raise(vm.core.runtime_error, "uninitialized native Win32API"));
        }
        PROCEDURES
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                id,
                Arc::new(Procedure {
                    inputs: input,
                    output,
                    target,
                }),
            );
        Ok(Value::Nil)
    }
    #[cfg(not(windows))]
    {
        let _ = (this, input, output, abi);
        // RuntimeError is intentional: the copied Ruby compatibility wrapper
        // rescues it and falls back to its portable implementation.
        Err(vm.raise(
            vm.core.runtime_error,
            &format!("native Win32API is unavailable on this platform: {dll}!{function}"),
        ))
    }
}

fn call(vm: &mut Vm, this: Value, args: &[Value]) -> Result<Value, VmError> {
    let procedure = procedure(vm, this)?;
    if args.len() != procedure.inputs.len() {
        return Err(vm.raise_arg("wrong number of Win32API arguments"));
    }
    #[cfg(windows)]
    {
        call_windows(vm, &procedure, args)
    }
    #[cfg(not(windows))]
    {
        let _ = (procedure.output, args);
        Err(vm.raise(
            vm.core.runtime_error,
            "native Win32API is unavailable on this platform",
        ))
    }
}

#[cfg(windows)]
fn call_windows(
    vm: &mut Vm,
    procedure: &Procedure,
    args: &[Value],
) -> Result<Value, VmError> {
    struct Buffer {
        index: usize,
        len: usize,
        bytes: Vec<u8>,
    }
    // Store indices, never Ruby values, in heap collections. Original arguments
    // stay rooted by the Ruby argument stack throughout conversion and copyback.
    let mut buffers: Vec<Buffer> = Vec::new();
    let mut words = Vec::with_capacity(args.len());
    for (index, (&value, kind)) in args.iter().zip(&procedure.inputs).enumerate() {
        if *kind == Kind::Pointer {
            if let Some(bytes) = vm.str_bytes(value) {
                let bytes = bytes.to_vec();
                let mut alias = None;
                for (buffer_index, buffer) in buffers.iter().enumerate() {
                    if value == args[buffer.index] {
                        alias = Some(buffer_index);
                        break;
                    }
                }
                let buffer_index = if let Some(alias) = alias {
                    alias
                } else {
                    let len = bytes.len();
                    let mut bytes = bytes;
                    bytes.push(0);
                    buffers.push(Buffer { index, len, bytes });
                    buffers.len() - 1
                };
                words.push(buffers[buffer_index].bytes.as_mut_ptr() as usize);
                continue;
            }
            if value.is_nil() {
                words.push(0);
                continue;
            }
        }
        let number = vm.expect_int(value, "Win32API argument")?;
        words.push(if *kind == Kind::Integer {
            number as i32 as usize
        } else {
            number as usize
        });
    }
    // No Ruby values are borrowed, no runtime registry is borrowed, and all
    // pointed-to storage is owned by Rust until the native call returns.
    let result = unsafe { procedure.target.call(&words, procedure.output) };
    let result = match procedure.output {
        Kind::Void => Value::Int(0),
        Kind::Integer => Value::Int(result as i32 as i64),
        Kind::Pointer if result == 0 => Value::Int(0),
        Kind::Pointer => {
            // SAFETY: the declared P return is a NUL-terminated native string,
            // as in legacy Win32API. Copy it before freeing argument buffers.
            let bytes =
                unsafe { std::ffi::CStr::from_ptr(result as *const std::ffi::c_char) }.to_bytes();
            vm.str_new(bytes)
        }
    };
    for buffer in buffers {
        let current = vm
            .str_bytes(args[buffer.index])
            .map(|bytes| bytes.to_vec())
            .unwrap_or_default();
        // Reacquire the string after conversion/GC rather than keeping its
        // address in the buffers vector.
        if current != buffer.bytes[..buffer.len] {
            let replacement = vm.str_new(&buffer.bytes[..buffer.len]);
            let replace = vm.intern("replace");
            vm.funcall(args[buffer.index], replace, &[replacement], Value::Nil)?;
        }
    }
    Ok(result)
}

pub(super) fn install(vm: &mut Vm) -> Result<(), String> {
    let class = vm.define_class("Win32API", vm.core.object);

    // `Class#new` dispatches `allocate`, so the constructor returns a host-data
    // object and `initialize` only has to fill its map slot.
    let sc = vm
        .singleton_class(Value::Obj(class))
        .map_err(|e| vm.describe_error(&e))?;
    vm.define_closure(sc, "allocate", move |vm, _class, _args, _blk| {
        let id = SERIAL.fetch_add(1, Ordering::Relaxed);
        Ok(vm.data_new(class, TAG, id))
    });

    vm.define_closure_args(class, "initialize", Aspec::any(), |vm, this, args, _blk| {
        initialize(vm, this, args)
    });
    vm.define_closure_args(class, "call", Aspec::any(), |vm, this, args, _blk| {
        call(vm, this, args)
    });
    let call = vm.intern("call");
    let call_alias = vm.intern("Call");
    vm.alias_method(class, call_alias, call)
        .map_err(|e| vm.describe_error(&e))?;
    Ok(())
}

#[cfg(windows)]
mod native {
    use super::{invoke, Abi, Kind};

    pub(super) struct LibraryFunction {
        handle: usize,
        address: usize,
        abi: Abi,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LoadLibraryW(name: *const u16) -> *mut std::ffi::c_void;
        fn GetProcAddress(
            module: *mut std::ffi::c_void,
            name: *const std::ffi::c_char,
        ) -> *mut std::ffi::c_void;
        fn FreeLibrary(module: *mut std::ffi::c_void) -> i32;
    }

    impl LibraryFunction {
        pub(super) fn open(dll: &str, function: &str, abi: Abi) -> Result<Self, String> {
            if dll.contains('\0') {
                return Err("DLL name contains NUL".into());
            }
            let name =
                std::ffi::CString::new(function).map_err(|_| "function name contains NUL")?;
            let wide: Vec<u16> = dll.encode_utf16().chain(Some(0)).collect();
            // SAFETY: wide is NUL-terminated and lives until LoadLibraryW returns.
            let handle = unsafe { LoadLibraryW(wide.as_ptr()) };
            if handle.is_null() {
                return Err(format!(
                    "cannot load {dll}: {}",
                    std::io::Error::last_os_error()
                ));
            }
            let mut target = Self {
                handle: handle as usize,
                address: 0,
                abi,
            };
            // SAFETY: the live library owns the symbol; name is NUL-terminated.
            let mut address = unsafe { GetProcAddress(handle, name.as_ptr()) };
            if address.is_null() && !function.ends_with(['A', 'W']) {
                let ansi = std::ffi::CString::new(format!("{function}A"))
                    .map_err(|_| "function name contains NUL")?;
                address = unsafe { GetProcAddress(handle, ansi.as_ptr()) };
            }
            if address.is_null() {
                return Err(format!(
                    "cannot find {dll}!{function}: {}",
                    std::io::Error::last_os_error()
                ));
            }
            target.address = address as usize;
            Ok(target)
        }

        /// The caller supplies a correct Win32 prototype, valid pointer buffers
        /// and at most 16 arguments. Floats, structs and callbacks are not
        /// supported.
        pub(super) unsafe fn call(&self, arguments: &[usize], output: Kind) -> usize {
            match output {
                Kind::Void => {
                    unsafe { invoke::<()>(self.address, self.abi, arguments) };
                    0
                }
                Kind::Integer => {
                    (unsafe { invoke::<i32>(self.address, self.abi, arguments) }) as isize as usize
                }
                Kind::Pointer => unsafe { invoke::<usize>(self.address, self.abi, arguments) },
            }
        }
    }

    impl Drop for LibraryFunction {
        fn drop(&mut self) {
            // SAFETY: each successful LoadLibraryW is owned by exactly one target.
            unsafe {
                FreeLibrary(self.handle as *mut std::ffi::c_void);
            }
        }
    }
}

/// Invoke the legacy integer/pointer ABI without a variadic C function. On
/// 32-bit Windows stdcall and cdecl have distinct stack-cleanup conventions.
#[cfg(any(windows, test))]
unsafe fn invoke<R>(address: usize, abi: Abi, args: &[usize]) -> R {
    macro_rules! word {
        ($index:tt) => {
            usize
        };
    }
    macro_rules! invoke_arity {
        ($convention:literal; $($index:tt),*) => {{
            // SAFETY: argument count/types and return R are checked by the
            // binding before selecting this ABI-specific function pointer.
            let function: unsafe extern $convention fn($(word!($index)),*) -> R =
                unsafe { std::mem::transmute(address) };
            unsafe { function($(args[$index]),*) }
        }};
    }
    macro_rules! invoke_abi {
        ($convention:literal) => {
            match args.len() {
                0 => invoke_arity!($convention;),
                1 => invoke_arity!($convention; 0),
                2 => invoke_arity!($convention; 0,1),
                3 => invoke_arity!($convention; 0,1,2),
                4 => invoke_arity!($convention; 0,1,2,3),
                5 => invoke_arity!($convention; 0,1,2,3,4),
                6 => invoke_arity!($convention; 0,1,2,3,4,5),
                7 => invoke_arity!($convention; 0,1,2,3,4,5,6),
                8 => invoke_arity!($convention; 0,1,2,3,4,5,6,7),
                9 => invoke_arity!($convention; 0,1,2,3,4,5,6,7,8),
                10 => invoke_arity!($convention; 0,1,2,3,4,5,6,7,8,9),
                11 => invoke_arity!($convention; 0,1,2,3,4,5,6,7,8,9,10),
                12 => invoke_arity!($convention; 0,1,2,3,4,5,6,7,8,9,10,11),
                13 => invoke_arity!($convention; 0,1,2,3,4,5,6,7,8,9,10,11,12),
                14 => invoke_arity!($convention; 0,1,2,3,4,5,6,7,8,9,10,11,12,13),
                15 => invoke_arity!($convention; 0,1,2,3,4,5,6,7,8,9,10,11,12,13,14),
                16 => invoke_arity!($convention; 0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15),
                _ => unreachable!("validated Win32API argument count"),
            }
        };
    }
    match abi {
        Abi::System => invoke_abi!("system"),
        Abi::Cdecl => invoke_abi!("C"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_legacy_signatures() {
        assert_eq!(
            inputs("pILnS").unwrap(),
            vec![
                Kind::Pointer,
                Kind::Integer,
                Kind::Integer,
                Kind::Integer,
                Kind::Pointer
            ]
        );
        assert!(inputs("V").unwrap().is_empty());
        assert!(inputs("").unwrap().is_empty());
        assert!(inputs("PV").is_err());
        assert!(inputs("F").is_err());
        assert!(inputs(&"I".repeat(17)).is_err());
        assert_eq!(Abi::parse("cdecl").unwrap(), Abi::Cdecl);
        assert!(Abi::parse("unknown").is_err());
    }

    unsafe extern "C" fn sum(a: usize, b: usize) -> i32 {
        (a + b) as i32
    }
    unsafe extern "system" fn write_byte(pointer: usize, value: usize) {
        unsafe {
            *(pointer as *mut u8) = value as u8;
        }
    }

    #[test]
    fn calls_word_arguments_and_mutable_buffers() {
        let sum = unsafe { invoke::<i32>(sum as *const () as usize, Abi::Cdecl, &[19, 23]) };
        assert_eq!(sum, 42);
        let mut byte = 0u8;
        unsafe {
            invoke::<()>(
                write_byte as *const () as usize,
                Abi::System,
                &[(&mut byte as *mut u8) as usize, 231],
            )
        };
        assert_eq!(byte, 231);
    }

    #[cfg(windows)]
    #[test]
    fn loads_windows_exports_and_writes_output_buffers() {
        let pid =
            native::LibraryFunction::open("kernel32.dll", "GetCurrentProcessId", Abi::System)
                .unwrap();
        assert_eq!(
            unsafe { pid.call(&[], Kind::Integer) } as u32,
            std::process::id()
        );
        let directory =
            native::LibraryFunction::open("kernel32.dll", "GetWindowsDirectoryA", Abi::System)
                .unwrap();
        let mut buffer = vec![0u8; 1024];
        let length = unsafe {
            directory.call(&[buffer.as_mut_ptr() as usize, buffer.len()], Kind::Integer)
        };
        assert!(length > 0 && length < buffer.len());
        assert_ne!(buffer[0], 0);
        assert_eq!(buffer[length], 0);
    }
}
