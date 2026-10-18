// SPDX-License-Identifier: GPL-2.0-or-later
//! Word-argument Windows ABI calls and RAII DLL ownership. No Ruby values.
use super::Abi;
#[cfg(windows)]
use super::Kind;

#[cfg(windows)]
pub(super) struct LibraryFunction {
    handle: usize,
    address: usize,
    abi: Abi,
}

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn LoadLibraryW(name: *const u16) -> *mut std::ffi::c_void;
    fn GetProcAddress(
        module: *mut std::ffi::c_void,
        name: *const std::ffi::c_char,
    ) -> *mut std::ffi::c_void;
    fn FreeLibrary(module: *mut std::ffi::c_void) -> i32;
}

#[cfg(windows)]
impl LibraryFunction {
    pub(super) fn open(dll: &str, function: &str, abi: Abi) -> Result<Self, String> {
        if dll.contains('\0') {
            return Err("DLL name contains NUL".into());
        }
        let name = std::ffi::CString::new(function).map_err(|_| "function name contains NUL")?;
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

    /// The caller supplies a correct Win32 prototype, valid pointer buffers and
    /// at most 16 arguments. Floats, structs and callbacks are not supported.
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

#[cfg(windows)]
impl Drop for LibraryFunction {
    fn drop(&mut self) {
        // SAFETY: each successful LoadLibraryW is owned by exactly one target.
        unsafe {
            FreeLibrary(self.handle as *mut std::ffi::c_void);
        }
    }
}

/// Invoke the legacy integer/pointer ABI without a variadic C function. On
/// 32-bit Windows stdcall and cdecl have distinct stack-cleanup conventions.
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
            let function: unsafe extern $convention fn($(word!($index)),*) -> R = unsafe { std::mem::transmute(address) };
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
            LibraryFunction::open("kernel32.dll", "GetCurrentProcessId", Abi::System).unwrap();
        assert_eq!(
            unsafe { pid.call(&[], Kind::Integer) } as u32,
            std::process::id()
        );
        let directory =
            LibraryFunction::open("kernel32.dll", "GetWindowsDirectoryA", Abi::System).unwrap();
        let mut buffer = vec![0u8; 1024];
        let length =
            unsafe { directory.call(&[buffer.as_mut_ptr() as usize, buffer.len()], Kind::Integer) };
        assert!(length > 0 && length < buffer.len());
        assert_ne!(buffer[0], 0);
        assert_eq!(buffer[length], 0);
    }
}
