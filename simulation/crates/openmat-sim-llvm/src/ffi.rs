//! Minimal declarations of the public LLVM C API; no LLVM or C++ layout crosses here.
use std::ffi::{CStr, c_char, c_int, c_uint, c_void};
use std::path::Path;
use std::sync::Mutex;

use libloading::Library;
use openmat_sim::numeric::EvalError;

pub type Handle = *mut c_void;
pub type Entry = unsafe extern "C" fn(u32, *const f64, u64, *mut f64, u64) -> u32;

#[repr(C)]
pub struct SymbolFlags {
    pub generic: u8,
    pub target: u8,
}
#[repr(C)]
pub struct EvaluatedSymbol {
    pub address: u64,
    pub flags: SymbolFlags,
}
#[repr(C)]
pub struct SymbolPair {
    pub name: Handle,
    pub symbol: EvaluatedSymbol,
}

pub struct Api {
    pub get_version: unsafe extern "C" fn(*mut c_uint, *mut c_uint, *mut c_uint),
    pub new_context: unsafe extern "C" fn() -> Handle,
    pub wrap_context: unsafe extern "C" fn(Handle) -> Handle,
    pub dispose_context: unsafe extern "C" fn(Handle),
    pub new_buffer: unsafe extern "C" fn(*const c_char, usize, *const c_char) -> Handle,
    pub parse_ir: unsafe extern "C" fn(Handle, Handle, *mut Handle, *mut *mut c_char) -> c_int,
    pub verify_module: unsafe extern "C" fn(Handle, c_int, *mut *mut c_char) -> c_int,
    pub dispose_module: unsafe extern "C" fn(Handle),
    pub dispose_message: unsafe extern "C" fn(*mut c_char),
    pub new_module: unsafe extern "C" fn(Handle, Handle) -> Handle,
    pub create_jit: unsafe extern "C" fn(*mut Handle, Handle) -> Handle,
    pub dispose_jit: unsafe extern "C" fn(Handle) -> Handle,
    pub get_dylib: unsafe extern "C" fn(Handle) -> Handle,
    pub add_module: unsafe extern "C" fn(Handle, Handle, Handle) -> Handle,
    pub lookup: unsafe extern "C" fn(Handle, *mut u64, *const c_char) -> Handle,
    pub intern: unsafe extern "C" fn(Handle, *const c_char) -> Handle,
    pub absolute_symbols: unsafe extern "C" fn(*mut SymbolPair, usize) -> Handle,
    pub define: unsafe extern "C" fn(Handle, Handle) -> Handle,
    pub dispose_unit: unsafe extern "C" fn(Handle),
    pub pass_options: unsafe extern "C" fn() -> Handle,
    pub dispose_pass_options: unsafe extern "C" fn(Handle),
    pub run_passes: unsafe extern "C" fn(Handle, *const c_char, Handle, Handle) -> Handle,
    pub error_message: unsafe extern "C" fn(Handle) -> *mut c_char,
    pub dispose_error_message: unsafe extern "C" fn(*mut c_char),
    pub get_triple: unsafe extern "C" fn(Handle) -> *const c_char,
    pub get_layout: unsafe extern "C" fn(Handle) -> *const c_char,
    pub set_triple: unsafe extern "C" fn(Handle, *const c_char),
    pub set_layout: unsafe extern "C" fn(Handle, *const c_char),
    // Function pointers above stay valid until this owning library is dropped.
    library: Library,
}

static INITIALIZE: Mutex<()> = Mutex::new(());

impl Api {
    pub fn load(library_path: &Path) -> Result<Self, EvalError> {
        let resolved_library = library_path
            .canonicalize()
            .map_err(|e| EvalError(format!("LLVM library path: {e}")))?;
        let library = load_library(&resolved_library)?;
        // SAFETY: Only a caller-selected trusted LLVM library is accepted. Each
        // symbol type is the public LLVM C API signature; missing symbols fail.
        unsafe {
            macro_rules! symbol {
                ($name:literal) => {
                    *library
                        .get(concat!($name, "\0").as_bytes())
                        .map_err(|e| EvalError(format!("missing LLVM C API {}: {e}", $name)))?
                };
            }
            let get_version: unsafe extern "C" fn(*mut c_uint, *mut c_uint, *mut c_uint) =
                symbol!("LLVMGetVersion");
            let (mut major, mut minor, mut patch) = (0, 0, 0);
            get_version(&raw mut major, &raw mut minor, &raw mut patch);
            if major != 22 {
                return Err(EvalError(format!(
                    "LLVM {major}.{minor}.{patch} is unsupported; simulation v0 requires LLVM 22"
                )));
            }
            let api = Self {
                get_version,
                new_context: symbol!("LLVMContextCreate"),
                wrap_context: symbol!("LLVMOrcCreateNewThreadSafeContextFromLLVMContext"),
                dispose_context: symbol!("LLVMOrcDisposeThreadSafeContext"),
                new_buffer: symbol!("LLVMCreateMemoryBufferWithMemoryRangeCopy"),
                parse_ir: symbol!("LLVMParseIRInContext"),
                verify_module: symbol!("LLVMVerifyModule"),
                dispose_module: symbol!("LLVMDisposeModule"),
                dispose_message: symbol!("LLVMDisposeMessage"),
                new_module: symbol!("LLVMOrcCreateNewThreadSafeModule"),
                create_jit: symbol!("LLVMOrcCreateLLJIT"),
                dispose_jit: symbol!("LLVMOrcDisposeLLJIT"),
                get_dylib: symbol!("LLVMOrcLLJITGetMainJITDylib"),
                add_module: symbol!("LLVMOrcLLJITAddLLVMIRModule"),
                lookup: symbol!("LLVMOrcLLJITLookup"),
                intern: symbol!("LLVMOrcLLJITMangleAndIntern"),
                absolute_symbols: symbol!("LLVMOrcAbsoluteSymbols"),
                define: symbol!("LLVMOrcJITDylibDefine"),
                dispose_unit: symbol!("LLVMOrcDisposeMaterializationUnit"),
                pass_options: symbol!("LLVMCreatePassBuilderOptions"),
                dispose_pass_options: symbol!("LLVMDisposePassBuilderOptions"),
                run_passes: symbol!("LLVMRunPasses"),
                error_message: symbol!("LLVMGetErrorMessage"),
                dispose_error_message: symbol!("LLVMDisposeErrorMessage"),
                get_triple: symbol!("LLVMOrcLLJITGetTripleString"),
                get_layout: symbol!("LLVMOrcLLJITGetDataLayoutStr"),
                set_triple: symbol!("LLVMSetTarget"),
                set_layout: symbol!("LLVMSetDataLayout"),
                library,
            };
            let _guard = INITIALIZE
                .lock()
                .map_err(|_| EvalError("LLVM initialization lock was poisoned".into()))?;
            for name in [
                c"LLVMInitializeX86TargetInfo",
                c"LLVMInitializeX86Target",
                c"LLVMInitializeX86TargetMC",
                c"LLVMInitializeX86AsmPrinter",
            ] {
                let initialize: libloading::Symbol<unsafe extern "C" fn()> = api
                    .library
                    .get(name.to_bytes_with_nul())
                    .map_err(|e| EvalError(format!("LLVM x86 target initialization: {e}")))?;
                initialize();
            }
            Ok(api)
        }
    }

    pub fn check(&self, error: Handle) -> Result<(), EvalError> {
        if error.is_null() {
            return Ok(());
        }
        // SAFETY: LLVM errors are uniquely consumed by LLVMGetErrorMessage.
        unsafe {
            let message = (self.error_message)(error);
            let text = read_message(message);
            (self.dispose_error_message)(message);
            Err(EvalError(text))
        }
    }

    pub fn message(&self, message: *mut c_char) -> String {
        if message.is_null() {
            return "LLVM did not supply a diagnostic".into();
        }
        // SAFETY: The caller passes the owned diagnostic returned by LLVM.
        unsafe {
            let text = read_message(message);
            (self.dispose_message)(message);
            text
        }
    }
}

unsafe fn read_message(message: *const c_char) -> String {
    if message.is_null() {
        return "LLVM did not supply a diagnostic".into();
    }
    // SAFETY: LLVM diagnostics are live, NUL-terminated strings.
    unsafe { CStr::from_ptr(message).to_string_lossy().into_owned() }
}

fn load_library(path: &Path) -> Result<Library, EvalError> {
    // SAFETY: Native libraries are explicit trusted host configuration, not model
    // input. On Windows dependencies resolve beside the DLL or in System32.
    unsafe {
        #[cfg(windows)]
        {
            use libloading::os::windows::{
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32,
                Library as WinLibrary,
            };
            WinLibrary::load_with_flags(
                path,
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
            )
            .map(Into::into)
            .map_err(|e| EvalError(format!("load LLVM and its adjacent runtime DLLs: {e}")))
        }
        #[cfg(not(windows))]
        {
            Library::new(path).map_err(|e| EvalError(format!("load LLVM: {e}")))
        }
    }
}
