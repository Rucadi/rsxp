// SPDX-License-Identifier: GPL-2.0-or-later
//! Legacy `Win32API` native calls. The shared Ruby shim emulates supported APIs
//! off Windows; actual DLL calls use the desktop Windows ABI only.
//!
//! The `Win32API` class is `#[magnus::wrap]`ped over
//! `RefCell<Option<Arc<Procedure>>>` (the `Arc` lets an in-flight `call` keep the
//! procedure alive independently of the wrapper). `initialize` accepts 3–5
//! arguments — DLL, function, argument type string/array, optional return type,
//! optional calling convention — whose type codes are parsed by `Kind`
//! (`Void`/`Integer`/`Pointer`) and `Abi` (`stdcall`→`System`, `cdecl`); `inputs`
//! rejects mixed void codes and more than 16 arguments. `call` (aliased `Call`)
//! marshals the arguments to machine words before dispatching.
//!
//! On Windows `native::LibraryFunction` loads the DLL (released by RAII) and
//! `call_windows` copies mutable string arguments into Rust-owned buffers,
//! invokes the function, and copies changed bytes back before freeing the
//! buffers. Off Windows `initialize` raises `StandardError`, which the copied
//! Ruby compatibility wrapper rescues to fall back to its portable
//! implementation.
#[cfg(windows)]
use magnus::RString;
use magnus::{method, prelude::*, Error, RArray, Ruby, TryConvert, Value};
use std::{cell::RefCell, sync::Arc};

#[cfg(any(windows, test))]
mod native;

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

#[derive(Default)]
#[magnus::wrap(class = "Win32API", free_immediately)]
struct RubyWin32Api(RefCell<Option<Arc<Procedure>>>);

impl RubyWin32Api {
    fn initialize(ruby: &Ruby, this: &Self, args: &[Value]) -> Result<(), Error> {
        if !(3..=5).contains(&args.len()) {
            return Err(Error::new(ruby.exception_arg_error(), "expected DLL, function, argument types, optional return type and calling convention"));
        }
        let dll = String::try_convert(args[0])?;
        let function = String::try_convert(args[1])?;
        let proto = if let Some(array) = RArray::from_value(args[2]) {
            array.funcall("join", ())?
        } else {
            String::try_convert(args[2])?
        };
        let input =
            inputs(&proto).map_err(|message| Error::new(ruby.exception_arg_error(), message))?;
        let output = if args.len() >= 4 {
            String::try_convert(args[3])?
        } else {
            "0".into()
        };
        let output = match output.chars().collect::<Vec<_>>().as_slice() {
            [] => Ok(Kind::Void),
            [code] => Kind::parse(*code),
            _ => Err("Win32API return type must contain one type code".into()),
        }
        .map_err(|message| Error::new(ruby.exception_arg_error(), message))?;
        let abi = if args.len() == 5 {
            args[4].funcall("to_s", ())?
        } else {
            "stdcall".to_string()
        };
        let abi =
            Abi::parse(&abi).map_err(|message| Error::new(ruby.exception_arg_error(), message))?;
        #[cfg(windows)]
        {
            let target = native::LibraryFunction::open(&dll, &function, abi)
                .map_err(|message| Error::new(ruby.exception_runtime_error(), message))?;
            *this.0.borrow_mut() = Some(Arc::new(Procedure {
                inputs: input,
                output,
                target,
            }));
            Ok(())
        }
        #[cfg(not(windows))]
        {
            let _ = (this, input, output, abi);
            // StandardError is intentional: the copied Ruby compatibility
            // wrapper rescues it and falls back to its portable implementation.
            Err(Error::new(
                ruby.exception_runtime_error(),
                format!("native Win32API is unavailable on this platform: {dll}!{function}"),
            ))
        }
    }

    fn call(ruby: &Ruby, this: &Self, args: &[Value]) -> Result<Value, Error> {
        let procedure = this.0.borrow().as_ref().cloned().ok_or_else(|| {
            Error::new(
                ruby.exception_runtime_error(),
                "uninitialized native Win32API",
            )
        })?;
        if args.len() != procedure.inputs.len() {
            return Err(Error::new(
                ruby.exception_arg_error(),
                "wrong number of Win32API arguments",
            ));
        }
        #[cfg(windows)]
        {
            call_windows(ruby, &procedure, args)
        }
        #[cfg(not(windows))]
        {
            let _ = procedure.output;
            Err(Error::new(
                ruby.exception_runtime_error(),
                "native Win32API is unavailable on this platform",
            ))
        }
    }
}

#[cfg(windows)]
fn call_windows(ruby: &Ruby, procedure: &Procedure, args: &[Value]) -> Result<Value, Error> {
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
            if let Some(string) = RString::from_value(value) {
                let mut alias = None;
                for (buffer_index, buffer) in buffers.iter().enumerate() {
                    if value.funcall::<_, _, bool>("equal?", (args[buffer.index],))? {
                        alias = Some(buffer_index);
                        break;
                    }
                }
                let buffer_index = if let Some(alias) = alias {
                    alias
                } else {
                    // SAFETY: copy the bytes before any further Ruby API call.
                    let mut bytes = unsafe { string.as_slice() }.to_vec();
                    let len = bytes.len();
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
        let number: i64 = ruby.module_kernel().funcall("Integer", (value,))?;
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
        Kind::Void => ruby.integer_from_i64(0).as_value(),
        Kind::Integer => ruby.integer_from_i64(result as i32 as i64).as_value(),
        Kind::Pointer if result == 0 => ruby.integer_from_i64(0).as_value(),
        Kind::Pointer => {
            // SAFETY: the declared P return is a NUL-terminated native string,
            // as in legacy Win32API. Copy it before freeing argument buffers.
            let bytes =
                unsafe { std::ffi::CStr::from_ptr(result as *const std::ffi::c_char) }.to_bytes();
            ruby.str_from_slice(bytes).as_value()
        }
    };
    for buffer in buffers {
        let string = RString::try_convert(args[buffer.index])?;
        // SAFETY: comparison does not invoke Ruby; reacquire the string after
        // conversion/GC rather than keeping its address in the buffers vector.
        if unsafe { string.as_slice() } != &buffer.bytes[..buffer.len] {
            string.replace(ruby.str_from_slice(&buffer.bytes[..buffer.len]))?;
        }
    }
    Ok(result)
}

pub(super) fn bind(ruby: &Ruby) -> Result<(), Error> {
    let class = ruby.define_class("Win32API", ruby.class_object())?;
    class.define_alloc_func::<RubyWin32Api>();
    class.define_method("initialize", method!(RubyWin32Api::initialize, -1))?;
    class.define_method("call", method!(RubyWin32Api::call, -1))?;
    class.define_alias("Call", "call")?;
    Ok(())
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
}
