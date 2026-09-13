//! SUNDIALS 7.5 C ABI, double precision, 64-bit indices, serial/non-MPI.
use libloading::Library;
use openmat_sim::RunError;
use std::ffi::{c_char, c_int, c_long, c_void};
use std::path::Path;

pub type Handle = *mut c_void;
pub type RootFn = unsafe extern "C" fn(f64, Handle, *mut f64, Handle) -> c_int;
pub type Rhs = unsafe extern "C" fn(f64, Handle, Handle, Handle) -> c_int;

pub struct Api {
    pub context_create: unsafe extern "C" fn(c_int, *mut Handle) -> c_int,
    pub context_free: unsafe extern "C" fn(*mut Handle) -> c_int,
    pub clear_handlers: unsafe extern "C" fn(Handle) -> c_int,
    pub vector_new: unsafe extern "C" fn(i64, Handle) -> Handle,
    pub vector_destroy: unsafe extern "C" fn(Handle),
    pub vector_data: unsafe extern "C" fn(Handle) -> *mut f64,
    pub matrix_new: unsafe extern "C" fn(i64, i64, Handle) -> Handle,
    pub matrix_destroy: unsafe extern "C" fn(Handle),
    pub linear_new: unsafe extern "C" fn(Handle, Handle, Handle) -> Handle,
    pub linear_free: unsafe extern "C" fn(Handle) -> c_int,
    pub create: unsafe extern "C" fn(c_int, Handle) -> Handle,
    pub free: unsafe extern "C" fn(*mut Handle),
    pub init: unsafe extern "C" fn(Handle, Rhs, f64, Handle) -> c_int,
    pub root_init: unsafe extern "C" fn(Handle, c_int, RootFn) -> c_int,
    pub root_info: unsafe extern "C" fn(Handle, *mut c_int) -> c_int,
    pub reinit: unsafe extern "C" fn(Handle, f64, Handle) -> c_int,
    pub tolerances: unsafe extern "C" fn(Handle, f64, f64) -> c_int,
    pub set_linear: unsafe extern "C" fn(Handle, Handle, Handle) -> c_int,
    pub set_user: unsafe extern "C" fn(Handle, Handle) -> c_int,
    pub max_step: unsafe extern "C" fn(Handle, f64) -> c_int,
    pub stop_time: unsafe extern "C" fn(Handle, f64) -> c_int,
    pub advance: unsafe extern "C" fn(Handle, f64, Handle, *mut f64, c_int) -> c_int,
    pub failures: unsafe extern "C" fn(Handle, *mut c_long) -> c_int,
    pub iterations: unsafe extern "C" fn(Handle, *mut c_long) -> c_int,
    // Libraries outlive all objects, callbacks and function pointer invocations.
    _libraries: Vec<Library>,
}

pub fn check(status: c_int, operation: &str) -> Result<(), RunError> {
    if status < 0 {
        Err(RunError::new(
            "cvode",
            format!("{operation} failed (SUNDIALS status {status})"),
        ))
    } else {
        Ok(())
    }
}

impl Api {
    #[allow(clippy::too_many_lines)] // Keep the validated ABI and complete symbol table together.
    pub fn load(directory: &Path) -> Result<Self, RunError> {
        if !cfg!(all(windows, target_arch = "x86_64")) {
            return Err(RunError::new(
                "sundials_platform",
                "this adapter currently supports the pinned Windows x64 SUNDIALS 7.5.0 serial runtime",
            ));
        }
        let directory = directory.canonicalize().map_err(|e| {
            RunError::new(
                "sundials_library",
                format!("SUNDIALS runtime directory: {e}"),
            )
        })?;
        // The trusted host chooses this directory; models cannot supply it.
        // Require the matching configuration, not a possibly incompatible MPI,
        // single precision or 32-bit-index build with similar symbol names.
        let config =
            std::fs::read_to_string(directory.join("../include/sundials/sundials_config.h"))
                .map_err(|e| {
                    RunError::new(
                        "sundials_abi",
                        format!("matching SUNDIALS configuration header is required: {e}"),
                    )
                })?;
        for required in [
            "#define SUNDIALS_DOUBLE_PRECISION 1",
            "#define SUNDIALS_INDEX_TYPE int64_t",
            "#define SUNDIALS_MPI_ENABLED 0",
        ] {
            if !config.lines().any(|line| line.trim() == required) {
                return Err(RunError::new(
                    "sundials_abi",
                    "SUNDIALS requires double precision, 64-bit indices and non-MPI configuration",
                ));
            }
        }
        let mut libraries = Vec::new();
        for name in [
            "libsundials_core-7.dll",
            "libsundials_nvecserial-7.dll",
            "libsundials_sunmatrixdense-5.dll",
            "libsundials_sunlinsoldense-5.dll",
            "libsundials_cvode-7.dll",
        ] {
            libraries.push(load_library(&directory.join(name))?);
        }
        // SAFETY: Signatures below are the public C ABI for the explicitly
        // required configuration. Missing symbols/version fail before use.
        unsafe {
            macro_rules! symbol {
                ($lib:expr, $name:literal) => {
                    *libraries[$lib]
                        .get(concat!($name, "\0").as_bytes())
                        .map_err(|e| RunError::new("sundials_symbol", format!("{}: {e}", $name)))?
                };
            }
            let version: unsafe extern "C" fn(
                *mut c_int,
                *mut c_int,
                *mut c_int,
                *mut c_char,
                c_int,
            ) -> c_int = symbol!(0, "SUNDIALSGetVersionNumber");
            let (mut major, mut minor, mut patch) = (0, 0, 0);
            let mut label = [0; 128];
            check(
                version(
                    &raw mut major,
                    &raw mut minor,
                    &raw mut patch,
                    label.as_mut_ptr(),
                    128,
                ),
                "get version",
            )?;
            if (major, minor, patch) != (7, 5, 0) {
                return Err(RunError::new(
                    "sundials_version",
                    format!("expected SUNDIALS 7.5.0, found {major}.{minor}.{patch}"),
                ));
            }
            Ok(Self {
                context_create: symbol!(0, "SUNContext_Create"),
                context_free: symbol!(0, "SUNContext_Free"),
                clear_handlers: symbol!(0, "SUNContext_ClearErrHandlers"),
                vector_new: symbol!(1, "N_VNew_Serial"),
                vector_destroy: symbol!(1, "N_VDestroy_Serial"),
                vector_data: symbol!(1, "N_VGetArrayPointer_Serial"),
                matrix_new: symbol!(2, "SUNDenseMatrix"),
                matrix_destroy: symbol!(2, "SUNMatDestroy_Dense"),
                linear_new: symbol!(3, "SUNLinSol_Dense"),
                linear_free: symbol!(3, "SUNLinSolFree_Dense"),
                create: symbol!(4, "CVodeCreate"),
                free: symbol!(4, "CVodeFree"),
                init: symbol!(4, "CVodeInit"),
                reinit: symbol!(4, "CVodeReInit"),
                root_init: symbol!(4, "CVodeRootInit"),
                root_info: symbol!(4, "CVodeGetRootInfo"),
                tolerances: symbol!(4, "CVodeSStolerances"),
                set_linear: symbol!(4, "CVodeSetLinearSolver"),
                set_user: symbol!(4, "CVodeSetUserData"),
                max_step: symbol!(4, "CVodeSetMaxStep"),
                stop_time: symbol!(4, "CVodeSetStopTime"),
                advance: symbol!(4, "CVode"),
                failures: symbol!(4, "CVodeGetNumErrTestFails"),
                iterations: symbol!(4, "CVodeGetNumNonlinSolvIters"),
                _libraries: libraries,
            })
        }
    }
}

fn load_library(path: &Path) -> Result<Library, RunError> {
    // SAFETY: Explicit trusted native dependencies; restrict Windows dependency
    // search to the selected directory and System32, never the working folder.
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
            .map_err(|e| RunError::new("sundials_library", format!("load {}: {e}", path.display())))
        }
        #[cfg(not(windows))]
        {
            Library::new(path).map_err(|e| RunError::new("sundials_library", e.to_string()))
        }
    }
}
