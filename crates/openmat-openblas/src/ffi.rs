//! Narrow Windows FFI boundary for `OpenBLAS` dynamic loading and CBLAS calls.

use std::ffi::{CStr, c_char, c_int, c_void};
use std::fmt;
use std::path::Path;
use std::sync::Mutex;

use libloading::os::windows::{
    LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32, Library as NativeLibrary,
};
use openmat_linalg::{
    CholeskyTriangle, Lp64CholeskyDimensions, Lp64FactorDimensions, Lp64GemmDimensions,
    Lp64RectangularSolveDimensions, Lp64SolveDimensions, Lp64SpectralDimensions, MatrixTranspose,
    SvdVectors,
};

use crate::OpenBlasError;

// CBLAS and LAPACKE intentionally use the same C enum value for column-major
// layout. Keep the literal at this narrow boundary rather than exposing it as
// a Rust-side ABI promise.
const COLUMN_MAJOR_LAYOUT: c_int = 102;
const CBLAS_NO_TRANS: c_int = 111;
const CBLAS_TRANS: c_int = 112;
const CBLAS_CONJ_TRANS: c_int = 113;
const LAPACK_NO_TRANS: c_char = 78;
const LAPACK_UPPER: c_char = 85;
const LAPACK_LOWER: c_char = 76;
const LAPACK_ALL_VECTORS: c_char = 65;
const LAPACK_SOME_VECTORS: c_char = 83;
const LAPACK_NO_VECTORS: c_char = 78;
const LAPACK_VECTORS: c_char = 86;
const UNIT_STRIDE: i32 = 1;

// OpenBLAS 0.3.x constructs its config in one process-global mutable buffer.
// Serialize identity calls made through this crate so the returned C string is
// stable until it has been copied into Rust-owned storage.
static OPENBLAS_IDENTITY_LOCK: Mutex<()> = Mutex::new(());

type GetStringFn = unsafe extern "C" fn() -> *const c_char;
type SgemmFn = unsafe extern "C" fn(
    c_int,
    c_int,
    c_int,
    i32,
    i32,
    i32,
    f32,
    *const f32,
    i32,
    *const f32,
    i32,
    f32,
    *mut f32,
    i32,
);
type CgemmFn = unsafe extern "C" fn(
    c_int,
    c_int,
    c_int,
    i32,
    i32,
    i32,
    *const c_void,
    *const c_void,
    i32,
    *const c_void,
    i32,
    *const c_void,
    *mut c_void,
    i32,
);
type DgemmFn = unsafe extern "C" fn(
    c_int,
    c_int,
    c_int,
    i32,
    i32,
    i32,
    f64,
    *const f64,
    i32,
    *const f64,
    i32,
    f64,
    *mut f64,
    i32,
);
type ZgemmFn = unsafe extern "C" fn(
    c_int,
    c_int,
    c_int,
    i32,
    i32,
    i32,
    *const c_void,
    *const c_void,
    i32,
    *const c_void,
    i32,
    *const c_void,
    *mut c_void,
    i32,
);
type DdotFn = unsafe extern "C" fn(i32, *const f64, i32, *const f64, i32) -> f64;
type Dnrm2Fn = unsafe extern "C" fn(i32, *const f64, i32) -> f64;
type SgesvFn = unsafe extern "C" fn(c_int, i32, i32, *mut f32, i32, *mut i32, *mut f32, i32) -> i32;
type CgesvFn =
    unsafe extern "C" fn(c_int, i32, i32, *mut c_void, i32, *mut i32, *mut c_void, i32) -> i32;
type DgesvFn = unsafe extern "C" fn(c_int, i32, i32, *mut f64, i32, *mut i32, *mut f64, i32) -> i32;
type ZgesvFn =
    unsafe extern "C" fn(c_int, i32, i32, *mut c_void, i32, *mut i32, *mut c_void, i32) -> i32;
type SgelsFn =
    unsafe extern "C" fn(c_int, c_char, i32, i32, i32, *mut f32, i32, *mut f32, i32) -> i32;
type CgelsFn =
    unsafe extern "C" fn(c_int, c_char, i32, i32, i32, *mut c_void, i32, *mut c_void, i32) -> i32;
type DgelsFn =
    unsafe extern "C" fn(c_int, c_char, i32, i32, i32, *mut f64, i32, *mut f64, i32) -> i32;
type ZgelsFn =
    unsafe extern "C" fn(c_int, c_char, i32, i32, i32, *mut c_void, i32, *mut c_void, i32) -> i32;
type SgetrfFn = unsafe extern "C" fn(c_int, i32, i32, *mut f32, i32, *mut i32) -> i32;
type DgetrfFn = unsafe extern "C" fn(c_int, i32, i32, *mut f64, i32, *mut i32) -> i32;
type CgetrfFn = unsafe extern "C" fn(c_int, i32, i32, *mut c_void, i32, *mut i32) -> i32;
type ZgetrfFn = unsafe extern "C" fn(c_int, i32, i32, *mut c_void, i32, *mut i32) -> i32;
type SgeqrfFn = unsafe extern "C" fn(c_int, i32, i32, *mut f32, i32, *mut f32) -> i32;
type DgeqrfFn = unsafe extern "C" fn(c_int, i32, i32, *mut f64, i32, *mut f64) -> i32;
type CgeqrfFn = unsafe extern "C" fn(c_int, i32, i32, *mut c_void, i32, *mut c_void) -> i32;
type ZgeqrfFn = unsafe extern "C" fn(c_int, i32, i32, *mut c_void, i32, *mut c_void) -> i32;
type SorgqrFn = unsafe extern "C" fn(c_int, i32, i32, i32, *mut f32, i32, *const f32) -> i32;
type DorgqrFn = unsafe extern "C" fn(c_int, i32, i32, i32, *mut f64, i32, *const f64) -> i32;
type CungqrFn = unsafe extern "C" fn(c_int, i32, i32, i32, *mut c_void, i32, *const c_void) -> i32;
type ZungqrFn = unsafe extern "C" fn(c_int, i32, i32, i32, *mut c_void, i32, *const c_void) -> i32;
type Sgeqp3Fn = unsafe extern "C" fn(c_int, i32, i32, *mut f32, i32, *mut i32, *mut f32) -> i32;
type Dgeqp3Fn = unsafe extern "C" fn(c_int, i32, i32, *mut f64, i32, *mut i32, *mut f64) -> i32;
type Cgeqp3Fn =
    unsafe extern "C" fn(c_int, i32, i32, *mut c_void, i32, *mut i32, *mut c_void) -> i32;
type Zgeqp3Fn =
    unsafe extern "C" fn(c_int, i32, i32, *mut c_void, i32, *mut i32, *mut c_void) -> i32;
type SpotrfFn = unsafe extern "C" fn(c_int, c_char, i32, *mut f32, i32) -> i32;
type DpotrfFn = unsafe extern "C" fn(c_int, c_char, i32, *mut f64, i32) -> i32;
type CpotrfFn = unsafe extern "C" fn(c_int, c_char, i32, *mut c_void, i32) -> i32;
type ZpotrfFn = unsafe extern "C" fn(c_int, c_char, i32, *mut c_void, i32) -> i32;
type SgesddFn = unsafe extern "C" fn(
    c_int,
    c_char,
    i32,
    i32,
    *mut f32,
    i32,
    *mut f32,
    *mut f32,
    i32,
    *mut f32,
    i32,
) -> i32;
type DgesddFn = unsafe extern "C" fn(
    c_int,
    c_char,
    i32,
    i32,
    *mut f64,
    i32,
    *mut f64,
    *mut f64,
    i32,
    *mut f64,
    i32,
) -> i32;
type CgesddFn = unsafe extern "C" fn(
    c_int,
    c_char,
    i32,
    i32,
    *mut c_void,
    i32,
    *mut f32,
    *mut c_void,
    i32,
    *mut c_void,
    i32,
) -> i32;
type ZgesddFn = unsafe extern "C" fn(
    c_int,
    c_char,
    i32,
    i32,
    *mut c_void,
    i32,
    *mut f64,
    *mut c_void,
    i32,
    *mut c_void,
    i32,
) -> i32;
type SgeevFn = unsafe extern "C" fn(
    c_int,
    c_char,
    c_char,
    i32,
    *mut f32,
    i32,
    *mut f32,
    *mut f32,
    *mut f32,
    i32,
    *mut f32,
    i32,
) -> i32;
type DgeevFn = unsafe extern "C" fn(
    c_int,
    c_char,
    c_char,
    i32,
    *mut f64,
    i32,
    *mut f64,
    *mut f64,
    *mut f64,
    i32,
    *mut f64,
    i32,
) -> i32;
type CgeevFn = unsafe extern "C" fn(
    c_int,
    c_char,
    c_char,
    i32,
    *mut c_void,
    i32,
    *mut c_void,
    *mut c_void,
    i32,
    *mut c_void,
    i32,
) -> i32;
type ZgeevFn = unsafe extern "C" fn(
    c_int,
    c_char,
    c_char,
    i32,
    *mut c_void,
    i32,
    *mut c_void,
    *mut c_void,
    i32,
    *mut c_void,
    i32,
) -> i32;
type ComplexSelectFn = unsafe extern "C" fn(*const c_void) -> i32;
type CgeesFn = unsafe extern "C" fn(
    c_int,
    c_char,
    c_char,
    Option<ComplexSelectFn>,
    i32,
    *mut c_void,
    i32,
    *mut i32,
    *mut c_void,
    *mut c_void,
    i32,
) -> i32;
type ZgeesFn = CgeesFn;

#[derive(Debug)]
pub(crate) enum BindError {
    Load(String),
    MissingSymbol {
        symbol: &'static str,
        detail: String,
    },
    NullString {
        symbol: &'static str,
    },
    InvalidStringEncoding {
        symbol: &'static str,
    },
}

impl BindError {
    pub(crate) fn with_path(self, path: &Path) -> OpenBlasError {
        match self {
            Self::Load(detail) => OpenBlasError::LibraryLoad {
                path: path.to_path_buf(),
                detail,
            },
            Self::MissingSymbol { symbol, detail } => OpenBlasError::MissingSymbol {
                path: path.to_path_buf(),
                symbol,
                detail,
            },
            Self::NullString { symbol } => OpenBlasError::NullString { symbol },
            Self::InvalidStringEncoding { symbol } => {
                OpenBlasError::InvalidStringEncoding { symbol }
            }
        }
    }
}

/// Loaded module before size-dependent CBLAS exports are bound.
pub(crate) struct Module {
    native: NativeLibrary,
}

impl Module {
    pub(crate) fn load(path: &Path) -> Result<Self, BindError> {
        let flags = LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32;
        // SAFETY: Loading a DLL executes its initialization and termination
        // routines. The safe public constructor documents that `path` must
        // name a trusted OpenBLAS build. A fully qualified path and the chosen
        // flags prevent lookup of the requested DLL or its dependencies via
        // PATH/current-directory search. `NativeLibrary` owns the handle and
        // keeps it alive through every copied function pointer.
        let native = unsafe { NativeLibrary::load_with_flags(path, flags) }
            .map_err(|error| BindError::Load(error.to_string()))?;
        Ok(Self { native })
    }

    pub(crate) fn config(&self) -> Result<String, BindError> {
        self.string_symbol("openblas_get_config", b"openblas_get_config\0")
    }

    pub(crate) fn core_name(&self) -> Result<String, BindError> {
        self.string_symbol("openblas_get_corename", b"openblas_get_corename\0")
    }

    #[allow(clippy::similar_names)]
    pub(crate) fn bind_compute_symbols(self) -> Result<Library, BindError> {
        let sgemm = self.function("cblas_sgemm", b"cblas_sgemm\0")?;
        let cgemm = self.function("cblas_cgemm", b"cblas_cgemm\0")?;
        let dgemm = self.function("cblas_dgemm", b"cblas_dgemm\0")?;
        let zgemm = self.function("cblas_zgemm", b"cblas_zgemm\0")?;
        let ddot = self.function("cblas_ddot", b"cblas_ddot\0")?;
        let dnrm2 = self.function("cblas_dnrm2", b"cblas_dnrm2\0")?;
        let sgesv = self.function("LAPACKE_sgesv", b"LAPACKE_sgesv\0")?;
        let cgesv = self.function("LAPACKE_cgesv", b"LAPACKE_cgesv\0")?;
        let dgesv = self.function("LAPACKE_dgesv", b"LAPACKE_dgesv\0")?;
        let zgesv = self.function("LAPACKE_zgesv", b"LAPACKE_zgesv\0")?;
        let sgels = self.function("LAPACKE_sgels", b"LAPACKE_sgels\0")?;
        let cgels = self.function("LAPACKE_cgels", b"LAPACKE_cgels\0")?;
        let dgels = self.function("LAPACKE_dgels", b"LAPACKE_dgels\0")?;
        let zgels = self.function("LAPACKE_zgels", b"LAPACKE_zgels\0")?;
        let sgetrf = self.function("LAPACKE_sgetrf", b"LAPACKE_sgetrf\0")?;
        let dgetrf = self.function("LAPACKE_dgetrf", b"LAPACKE_dgetrf\0")?;
        let cgetrf = self.function("LAPACKE_cgetrf", b"LAPACKE_cgetrf\0")?;
        let zgetrf = self.function("LAPACKE_zgetrf", b"LAPACKE_zgetrf\0")?;
        let sgeqrf = self.function("LAPACKE_sgeqrf", b"LAPACKE_sgeqrf\0")?;
        let dgeqrf = self.function("LAPACKE_dgeqrf", b"LAPACKE_dgeqrf\0")?;
        let cgeqrf = self.function("LAPACKE_cgeqrf", b"LAPACKE_cgeqrf\0")?;
        let zgeqrf = self.function("LAPACKE_zgeqrf", b"LAPACKE_zgeqrf\0")?;
        let sorgqr = self.function("LAPACKE_sorgqr", b"LAPACKE_sorgqr\0")?;
        let dorgqr = self.function("LAPACKE_dorgqr", b"LAPACKE_dorgqr\0")?;
        let cungqr = self.function("LAPACKE_cungqr", b"LAPACKE_cungqr\0")?;
        let zungqr = self.function("LAPACKE_zungqr", b"LAPACKE_zungqr\0")?;
        let sgeqp3 = self.function("LAPACKE_sgeqp3", b"LAPACKE_sgeqp3\0")?;
        let dgeqp3 = self.function("LAPACKE_dgeqp3", b"LAPACKE_dgeqp3\0")?;
        let cgeqp3 = self.function("LAPACKE_cgeqp3", b"LAPACKE_cgeqp3\0")?;
        let zgeqp3 = self.function("LAPACKE_zgeqp3", b"LAPACKE_zgeqp3\0")?;
        let spotrf = self.function("LAPACKE_spotrf", b"LAPACKE_spotrf\0")?;
        let dpotrf = self.function("LAPACKE_dpotrf", b"LAPACKE_dpotrf\0")?;
        let cpotrf = self.function("LAPACKE_cpotrf", b"LAPACKE_cpotrf\0")?;
        let zpotrf = self.function("LAPACKE_zpotrf", b"LAPACKE_zpotrf\0")?;
        let sgesdd = self.function("LAPACKE_sgesdd", b"LAPACKE_sgesdd\0")?;
        let dgesdd = self.function("LAPACKE_dgesdd", b"LAPACKE_dgesdd\0")?;
        let cgesdd = self.function("LAPACKE_cgesdd", b"LAPACKE_cgesdd\0")?;
        let zgesdd = self.function("LAPACKE_zgesdd", b"LAPACKE_zgesdd\0")?;
        let sgeev = self.function("LAPACKE_sgeev", b"LAPACKE_sgeev\0")?;
        let dgeev = self.function("LAPACKE_dgeev", b"LAPACKE_dgeev\0")?;
        let cgeev = self.function("LAPACKE_cgeev", b"LAPACKE_cgeev\0")?;
        let zgeev = self.function("LAPACKE_zgeev", b"LAPACKE_zgeev\0")?;
        let cgees = self.function("LAPACKE_cgees", b"LAPACKE_cgees\0")?;
        let zgees = self.function("LAPACKE_zgees", b"LAPACKE_zgees\0")?;
        Ok(Library {
            _native: Some(self.native),
            sgemm,
            cgemm,
            dgemm,
            zgemm,
            ddot,
            dnrm2,
            sgesv,
            cgesv,
            dgesv,
            zgesv,
            sgels,
            cgels,
            dgels,
            zgels,
            sgetrf,
            dgetrf,
            cgetrf,
            zgetrf,
            sgeqrf,
            dgeqrf,
            cgeqrf,
            zgeqrf,
            sorgqr,
            dorgqr,
            cungqr,
            zungqr,
            sgeqp3,
            dgeqp3,
            cgeqp3,
            zgeqp3,
            spotrf,
            dpotrf,
            cpotrf,
            zpotrf,
            sgesdd,
            dgesdd,
            cgesdd,
            zgesdd,
            sgeev,
            dgeev,
            cgeev,
            zgeev,
            cgees,
            zgees,
        })
    }

    fn string_symbol(
        &self,
        display_name: &'static str,
        symbol: &'static [u8],
    ) -> Result<String, BindError> {
        let _identity_guard = OPENBLAS_IDENTITY_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let function: GetStringFn = self.function(display_name, symbol)?;
        // SAFETY: The symbol was resolved with OpenBLAS's documented
        // `char *function(void)` signature. The module is still loaded, and
        // OpenBLAS returns a pointer to a static NUL-terminated string.
        let pointer = unsafe { function() };
        if pointer.is_null() {
            return Err(BindError::NullString {
                symbol: display_name,
            });
        }
        // SAFETY: The non-null pointer and NUL-termination are guaranteed by
        // the OpenBLAS string-function contract described above. The text is
        // copied before the module can be unloaded.
        let value = unsafe { CStr::from_ptr(pointer) };
        value
            .to_str()
            .map(str::to_owned)
            .map_err(|_| BindError::InvalidStringEncoding {
                symbol: display_name,
            })
    }

    fn function<T: Copy>(
        &self,
        display_name: &'static str,
        symbol: &'static [u8],
    ) -> Result<T, BindError> {
        // SAFETY: Every call site supplies the exact `extern "C"` function
        // pointer type declared by the installed OpenBLAS CBLAS header. The
        // returned pointer is copied only into a structure that owns `native`,
        // so it cannot outlive the loaded module.
        unsafe { self.native.get::<T>(symbol) }
            .map(|function| *function)
            .map_err(|error| BindError::MissingSymbol {
                symbol: display_name,
                detail: error.to_string(),
            })
    }
}

/// Bound functions plus the owning module handle.
pub(crate) struct Library {
    // Dropping this field unloads the DLL only after this `Library` can no
    // longer be borrowed for a native call.
    // Production binding always stores `Some`; unit tests use `None` only to
    // exercise this narrow ABI wrapper with local `extern "C"` mock functions.
    _native: Option<NativeLibrary>,
    sgemm: SgemmFn,
    cgemm: CgemmFn,
    dgemm: DgemmFn,
    zgemm: ZgemmFn,
    ddot: DdotFn,
    dnrm2: Dnrm2Fn,
    sgesv: SgesvFn,
    cgesv: CgesvFn,
    dgesv: DgesvFn,
    zgesv: ZgesvFn,
    sgels: SgelsFn,
    cgels: CgelsFn,
    dgels: DgelsFn,
    zgels: ZgelsFn,
    sgetrf: SgetrfFn,
    dgetrf: DgetrfFn,
    cgetrf: CgetrfFn,
    zgetrf: ZgetrfFn,
    sgeqrf: SgeqrfFn,
    dgeqrf: DgeqrfFn,
    cgeqrf: CgeqrfFn,
    zgeqrf: ZgeqrfFn,
    sorgqr: SorgqrFn,
    dorgqr: DorgqrFn,
    cungqr: CungqrFn,
    zungqr: ZungqrFn,
    sgeqp3: Sgeqp3Fn,
    dgeqp3: Dgeqp3Fn,
    cgeqp3: Cgeqp3Fn,
    zgeqp3: Zgeqp3Fn,
    spotrf: SpotrfFn,
    dpotrf: DpotrfFn,
    cpotrf: CpotrfFn,
    zpotrf: ZpotrfFn,
    sgesdd: SgesddFn,
    dgesdd: DgesddFn,
    cgesdd: CgesddFn,
    zgesdd: ZgesddFn,
    sgeev: SgeevFn,
    dgeev: DgeevFn,
    cgeev: CgeevFn,
    zgeev: ZgeevFn,
    cgees: CgeesFn,
    zgees: ZgeesFn,
}

macro_rules! real_getrf_method {
    ($name:ident, $field:ident, $scalar:ty) => {
        pub(crate) fn $name(
            &self,
            dimensions: Lp64FactorDimensions,
            matrix: &mut [$scalar],
            pivots: &mut [i32],
        ) -> Result<i32, CallError> {
            validate_factor_buffers(dimensions, matrix.len())?;
            validate_vector(dimensions.k, pivots.len(), "LU pivot buffer")?;
            // SAFETY: Validation proves complete column-major matrix and pivot
            // buffers; LAPACKE is synchronous and the DLL remains loaded.
            Ok(unsafe {
                (self.$field)(
                    COLUMN_MAJOR_LAYOUT,
                    dimensions.m,
                    dimensions.n,
                    matrix.as_mut_ptr(),
                    dimensions.lda,
                    pivots.as_mut_ptr(),
                )
            })
        }
    };
}

macro_rules! complex_getrf_method {
    ($name:ident, $field:ident, $component:ty) => {
        pub(crate) fn $name(
            &self,
            dimensions: Lp64FactorDimensions,
            matrix: &mut [[$component; 2]],
            pivots: &mut [i32],
        ) -> Result<i32, CallError> {
            validate_factor_buffers(dimensions, matrix.len())?;
            validate_vector(dimensions.k, pivots.len(), "LU pivot buffer")?;
            // SAFETY: Validation proves bounds. Complex elements cross only as
            // adjacent component pairs through the LAPACKE `void *` ABI.
            Ok(unsafe {
                (self.$field)(
                    COLUMN_MAJOR_LAYOUT,
                    dimensions.m,
                    dimensions.n,
                    matrix.as_mut_ptr().cast(),
                    dimensions.lda,
                    pivots.as_mut_ptr(),
                )
            })
        }
    };
}

macro_rules! real_geqrf_method {
    ($name:ident, $field:ident, $scalar:ty) => {
        pub(crate) fn $name(
            &self,
            dimensions: Lp64FactorDimensions,
            matrix: &mut [$scalar],
            tau: &mut [$scalar],
        ) -> Result<i32, CallError> {
            validate_qr_buffers(dimensions, matrix.len(), tau.len())?;
            // SAFETY: Validation proves complete matrix and reflector buffers.
            Ok(unsafe {
                (self.$field)(
                    COLUMN_MAJOR_LAYOUT,
                    dimensions.m,
                    dimensions.n,
                    matrix.as_mut_ptr(),
                    dimensions.lda,
                    tau.as_mut_ptr(),
                )
            })
        }
    };
}

macro_rules! complex_geqrf_method {
    ($name:ident, $field:ident, $component:ty) => {
        pub(crate) fn $name(
            &self,
            dimensions: Lp64FactorDimensions,
            matrix: &mut [[$component; 2]],
            tau: &mut [[$component; 2]],
        ) -> Result<i32, CallError> {
            validate_qr_buffers(dimensions, matrix.len(), tau.len())?;
            // SAFETY: Validation proves bounds; explicit adjacent pairs are the
            // only complex representation crossing the C ABI.
            Ok(unsafe {
                (self.$field)(
                    COLUMN_MAJOR_LAYOUT,
                    dimensions.m,
                    dimensions.n,
                    matrix.as_mut_ptr().cast(),
                    dimensions.lda,
                    tau.as_mut_ptr().cast(),
                )
            })
        }
    };
}

macro_rules! real_generate_q_method {
    ($name:ident, $field:ident, $scalar:ty) => {
        pub(crate) fn $name(
            &self,
            dimensions: Lp64FactorDimensions,
            q_columns: i32,
            matrix: &mut [$scalar],
            tau: &[$scalar],
        ) -> Result<i32, CallError> {
            validate_generate_q_buffers(dimensions, q_columns, matrix.len(), tau.len())?;
            // SAFETY: Validation proves LAPACKE's m/n/k relationships and
            // complete matrix/tau buffers.
            Ok(unsafe {
                (self.$field)(
                    COLUMN_MAJOR_LAYOUT,
                    dimensions.m,
                    q_columns,
                    dimensions.k,
                    matrix.as_mut_ptr(),
                    dimensions.lda,
                    tau.as_ptr(),
                )
            })
        }
    };
}

macro_rules! complex_generate_q_method {
    ($name:ident, $field:ident, $component:ty) => {
        pub(crate) fn $name(
            &self,
            dimensions: Lp64FactorDimensions,
            q_columns: i32,
            matrix: &mut [[$component; 2]],
            tau: &[[$component; 2]],
        ) -> Result<i32, CallError> {
            validate_generate_q_buffers(dimensions, q_columns, matrix.len(), tau.len())?;
            // SAFETY: Validation proves bounds and LAPACKE dimension
            // relationships; complex storage is explicit adjacent pairs.
            Ok(unsafe {
                (self.$field)(
                    COLUMN_MAJOR_LAYOUT,
                    dimensions.m,
                    q_columns,
                    dimensions.k,
                    matrix.as_mut_ptr().cast(),
                    dimensions.lda,
                    tau.as_ptr().cast(),
                )
            })
        }
    };
}

macro_rules! real_geqp3_method {
    ($name:ident, $field:ident, $scalar:ty) => {
        pub(crate) fn $name(
            &self,
            dimensions: Lp64FactorDimensions,
            matrix: &mut [$scalar],
            pivots: &mut [i32],
            tau: &mut [$scalar],
        ) -> Result<i32, CallError> {
            validate_qr_buffers(dimensions, matrix.len(), tau.len())?;
            validate_vector(dimensions.n, pivots.len(), "QR column pivot buffer")?;
            // SAFETY: Validation proves complete matrix, pivot, and tau buffers.
            Ok(unsafe {
                (self.$field)(
                    COLUMN_MAJOR_LAYOUT,
                    dimensions.m,
                    dimensions.n,
                    matrix.as_mut_ptr(),
                    dimensions.lda,
                    pivots.as_mut_ptr(),
                    tau.as_mut_ptr(),
                )
            })
        }
    };
}

macro_rules! complex_geqp3_method {
    ($name:ident, $field:ident, $component:ty) => {
        pub(crate) fn $name(
            &self,
            dimensions: Lp64FactorDimensions,
            matrix: &mut [[$component; 2]],
            pivots: &mut [i32],
            tau: &mut [[$component; 2]],
        ) -> Result<i32, CallError> {
            validate_qr_buffers(dimensions, matrix.len(), tau.len())?;
            validate_vector(dimensions.n, pivots.len(), "QR column pivot buffer")?;
            // SAFETY: Validation proves bounds and complex values are explicit
            // adjacent component pairs across the C ABI.
            Ok(unsafe {
                (self.$field)(
                    COLUMN_MAJOR_LAYOUT,
                    dimensions.m,
                    dimensions.n,
                    matrix.as_mut_ptr().cast(),
                    dimensions.lda,
                    pivots.as_mut_ptr(),
                    tau.as_mut_ptr().cast(),
                )
            })
        }
    };
}

macro_rules! real_potrf_method {
    ($name:ident, $field:ident, $scalar:ty) => {
        pub(crate) fn $name(
            &self,
            dimensions: Lp64CholeskyDimensions,
            triangle: CholeskyTriangle,
            matrix: &mut [$scalar],
        ) -> Result<i32, CallError> {
            validate_cholesky_buffer(dimensions, matrix.len())?;
            // SAFETY: Validation proves a complete square column-major buffer.
            Ok(unsafe {
                (self.$field)(
                    COLUMN_MAJOR_LAYOUT,
                    lapack_triangle(triangle),
                    dimensions.n,
                    matrix.as_mut_ptr(),
                    dimensions.lda,
                )
            })
        }
    };
}

macro_rules! complex_potrf_method {
    ($name:ident, $field:ident, $component:ty) => {
        pub(crate) fn $name(
            &self,
            dimensions: Lp64CholeskyDimensions,
            triangle: CholeskyTriangle,
            matrix: &mut [[$component; 2]],
        ) -> Result<i32, CallError> {
            validate_cholesky_buffer(dimensions, matrix.len())?;
            // SAFETY: Validation proves bounds and complex storage crosses as
            // explicit adjacent component pairs only.
            Ok(unsafe {
                (self.$field)(
                    COLUMN_MAJOR_LAYOUT,
                    lapack_triangle(triangle),
                    dimensions.n,
                    matrix.as_mut_ptr().cast(),
                    dimensions.lda,
                )
            })
        }
    };
}

macro_rules! real_gesdd_method {
    ($name:ident, $field:ident, $scalar:ty) => {
        #[allow(clippy::too_many_arguments)]
        pub(crate) fn $name(
            &self,
            dimensions: Lp64SpectralDimensions,
            vectors: SvdVectors,
            matrix: &mut [$scalar],
            singular_values: &mut [$scalar],
            u: &mut [$scalar],
            ldu: i32,
            vt: &mut [$scalar],
            ldvt: i32,
        ) -> Result<i32, CallError> {
            validate_svd_buffers(
                dimensions,
                vectors,
                matrix.len(),
                singular_values.len(),
                u.len(),
                ldu,
                vt.len(),
                ldvt,
            )?;
            // SAFETY: Validation proves every matrix/vector extent. LAPACKE is
            // synchronous and the owning DLL remains loaded through `self`.
            Ok(unsafe {
                (self.$field)(
                    COLUMN_MAJOR_LAYOUT,
                    svd_job(vectors),
                    dimensions.m,
                    dimensions.n,
                    matrix.as_mut_ptr(),
                    dimensions.lda,
                    singular_values.as_mut_ptr(),
                    u.as_mut_ptr(),
                    ldu,
                    vt.as_mut_ptr(),
                    ldvt,
                )
            })
        }
    };
}

macro_rules! complex_gesdd_method {
    ($name:ident, $field:ident, $component:ty) => {
        #[allow(clippy::too_many_arguments)]
        pub(crate) fn $name(
            &self,
            dimensions: Lp64SpectralDimensions,
            vectors: SvdVectors,
            matrix: &mut [[$component; 2]],
            singular_values: &mut [$component],
            u: &mut [[$component; 2]],
            ldu: i32,
            vt: &mut [[$component; 2]],
            ldvt: i32,
        ) -> Result<i32, CallError> {
            validate_svd_buffers(
                dimensions,
                vectors,
                matrix.len(),
                singular_values.len(),
                u.len(),
                ldu,
                vt.len(),
                ldvt,
            )?;
            // SAFETY: Bounds and dimensions are validated; complex values are
            // explicit adjacent component pairs across the C ABI.
            Ok(unsafe {
                (self.$field)(
                    COLUMN_MAJOR_LAYOUT,
                    svd_job(vectors),
                    dimensions.m,
                    dimensions.n,
                    matrix.as_mut_ptr().cast(),
                    dimensions.lda,
                    singular_values.as_mut_ptr(),
                    u.as_mut_ptr().cast(),
                    ldu,
                    vt.as_mut_ptr().cast(),
                    ldvt,
                )
            })
        }
    };
}

macro_rules! real_geev_method {
    ($name:ident, $field:ident, $scalar:ty) => {
        #[allow(clippy::too_many_arguments)]
        pub(crate) fn $name(
            &self,
            dimensions: Lp64SpectralDimensions,
            left_vectors: bool,
            right_vectors: bool,
            matrix: &mut [$scalar],
            real_values: &mut [$scalar],
            imaginary_values: &mut [$scalar],
            left: &mut [$scalar],
            right: &mut [$scalar],
        ) -> Result<i32, CallError> {
            validate_eig_buffers(
                dimensions,
                left_vectors,
                right_vectors,
                matrix.len(),
                real_values.len(),
                imaginary_values.len(),
                left.len(),
                right.len(),
            )?;
            let leading = dimensions.n.max(1);
            Ok(unsafe {
                (self.$field)(
                    COLUMN_MAJOR_LAYOUT,
                    eig_job(left_vectors),
                    eig_job(right_vectors),
                    dimensions.n,
                    matrix.as_mut_ptr(),
                    dimensions.lda,
                    real_values.as_mut_ptr(),
                    imaginary_values.as_mut_ptr(),
                    left.as_mut_ptr(),
                    leading,
                    right.as_mut_ptr(),
                    leading,
                )
            })
        }
    };
}

macro_rules! complex_geev_method {
    ($name:ident, $field:ident, $component:ty) => {
        #[allow(clippy::too_many_arguments)]
        pub(crate) fn $name(
            &self,
            dimensions: Lp64SpectralDimensions,
            left_vectors: bool,
            right_vectors: bool,
            matrix: &mut [[$component; 2]],
            values: &mut [[$component; 2]],
            left: &mut [[$component; 2]],
            right: &mut [[$component; 2]],
        ) -> Result<i32, CallError> {
            validate_eig_buffers(
                dimensions,
                left_vectors,
                right_vectors,
                matrix.len(),
                values.len(),
                values.len(),
                left.len(),
                right.len(),
            )?;
            let leading = dimensions.n.max(1);
            Ok(unsafe {
                (self.$field)(
                    COLUMN_MAJOR_LAYOUT,
                    eig_job(left_vectors),
                    eig_job(right_vectors),
                    dimensions.n,
                    matrix.as_mut_ptr().cast(),
                    dimensions.lda,
                    values.as_mut_ptr().cast(),
                    left.as_mut_ptr().cast(),
                    leading,
                    right.as_mut_ptr().cast(),
                    leading,
                )
            })
        }
    };
}

macro_rules! complex_gees_method {
    ($name:ident, $field:ident, $component:ty) => {
        pub(crate) fn $name(
            &self,
            dimensions: Lp64SpectralDimensions,
            matrix: &mut [[$component; 2]],
            values: &mut [[$component; 2]],
            vectors: &mut [[$component; 2]],
        ) -> Result<(i32, i32), CallError> {
            validate_schur_buffers(dimensions, matrix.len(), values.len(), vectors.len())?;
            let leading = dimensions.n.max(1);
            let mut selected = 0_i32;
            // SAFETY: Bounds and dimensions are validated, sorting is disabled
            // so the nullable selector is never called, complex values are
            // explicit adjacent component pairs, and the DLL stays loaded.
            let info = unsafe {
                (self.$field)(
                    COLUMN_MAJOR_LAYOUT,
                    LAPACK_VECTORS,
                    LAPACK_NO_VECTORS,
                    None,
                    dimensions.n,
                    matrix.as_mut_ptr().cast(),
                    dimensions.lda,
                    &mut selected,
                    values.as_mut_ptr().cast(),
                    vectors.as_mut_ptr().cast(),
                    leading,
                )
            };
            Ok((info, selected))
        }
    };
}

impl Library {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn sgemm(
        &self,
        dimensions: Lp64GemmDimensions,
        left_transpose: MatrixTranspose,
        right_transpose: MatrixTranspose,
        alpha: f32,
        left: &[f32],
        right: &[f32],
        beta: f32,
        output: &mut [f32],
    ) -> Result<(), CallError> {
        validate_gemm_buffers(
            dimensions,
            left_transpose,
            right_transpose,
            left.len(),
            right.len(),
            output.len(),
        )?;
        // SAFETY: `validate_gemm_buffers` proves complete, disjoint buffers
        // and exact LP64 dimensions. The DLL is retained by `self`, and CBLAS
        // is synchronous and retains no pointer.
        unsafe {
            (self.sgemm)(
                COLUMN_MAJOR_LAYOUT,
                real_transpose(left_transpose),
                real_transpose(right_transpose),
                dimensions.m,
                dimensions.n,
                dimensions.k,
                alpha,
                left.as_ptr(),
                dimensions.lda,
                right.as_ptr(),
                dimensions.ldb,
                beta,
                output.as_mut_ptr(),
                dimensions.ldc,
            );
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn cgemm(
        &self,
        dimensions: Lp64GemmDimensions,
        left_transpose: MatrixTranspose,
        right_transpose: MatrixTranspose,
        alpha: [f32; 2],
        left: &[[f32; 2]],
        right: &[[f32; 2]],
        beta: [f32; 2],
        output: &mut [[f32; 2]],
    ) -> Result<(), CallError> {
        validate_gemm_buffers(
            dimensions,
            left_transpose,
            right_transpose,
            left.len(),
            right.len(),
            output.len(),
        )?;
        // SAFETY: The same bounds and lifetime proof as `sgemm` applies.
        // Complex values are explicit adjacent `[f32; 2]` pairs rather than a
        // Rust complex representation.
        unsafe {
            (self.cgemm)(
                COLUMN_MAJOR_LAYOUT,
                complex_transpose(left_transpose),
                complex_transpose(right_transpose),
                dimensions.m,
                dimensions.n,
                dimensions.k,
                alpha.as_ptr().cast(),
                left.as_ptr().cast(),
                dimensions.lda,
                right.as_ptr().cast(),
                dimensions.ldb,
                beta.as_ptr().cast(),
                output.as_mut_ptr().cast(),
                dimensions.ldc,
            );
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn dgemm(
        &self,
        dimensions: Lp64GemmDimensions,
        left_transpose: MatrixTranspose,
        right_transpose: MatrixTranspose,
        alpha: f64,
        left: &[f64],
        right: &[f64],
        beta: f64,
        output: &mut [f64],
    ) -> Result<(), CallError> {
        validate_gemm_buffers(
            dimensions,
            left_transpose,
            right_transpose,
            left.len(),
            right.len(),
            output.len(),
        )?;
        // SAFETY: `validate_gemm_buffers` proves all dimensions are positive,
        // leading dimensions match the physical column-major matrices, and
        // each slice covers every element CBLAS may access. The slices are
        // disjoint according to Rust borrowing, the DLL is held by `self`, and
        // CBLAS GEMM is synchronous and does not retain pointers.
        unsafe {
            (self.dgemm)(
                COLUMN_MAJOR_LAYOUT,
                real_transpose(left_transpose),
                real_transpose(right_transpose),
                dimensions.m,
                dimensions.n,
                dimensions.k,
                alpha,
                left.as_ptr(),
                dimensions.lda,
                right.as_ptr(),
                dimensions.ldb,
                beta,
                output.as_mut_ptr(),
                dimensions.ldc,
            );
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn zgemm(
        &self,
        dimensions: Lp64GemmDimensions,
        left_transpose: MatrixTranspose,
        right_transpose: MatrixTranspose,
        alpha: [f64; 2],
        left: &[[f64; 2]],
        right: &[[f64; 2]],
        beta: [f64; 2],
        output: &mut [[f64; 2]],
    ) -> Result<(), CallError> {
        validate_gemm_buffers(
            dimensions,
            left_transpose,
            right_transpose,
            left.len(),
            right.len(),
            output.len(),
        )?;
        // SAFETY: The same bounds/lifetime argument as `dgemm` applies.
        // Complex values are deliberately packed as arrays of two adjacent
        // `f64`s (real then imaginary), which is the storage consumed through
        // OpenBLAS CBLAS's `void *` complex parameters. No Rust complex type
        // crosses this boundary, and the synchronous call retains no pointer.
        unsafe {
            (self.zgemm)(
                COLUMN_MAJOR_LAYOUT,
                complex_transpose(left_transpose),
                complex_transpose(right_transpose),
                dimensions.m,
                dimensions.n,
                dimensions.k,
                alpha.as_ptr().cast(),
                left.as_ptr().cast(),
                dimensions.lda,
                right.as_ptr().cast(),
                dimensions.ldb,
                beta.as_ptr().cast(),
                output.as_mut_ptr().cast(),
                dimensions.ldc,
            );
        }
        Ok(())
    }

    pub(crate) fn ddot(&self, n: i32, left: &[f64], right: &[f64]) -> Result<f64, CallError> {
        validate_vector(n, left.len(), "left dot operand")?;
        validate_vector(n, right.len(), "right dot operand")?;
        // SAFETY: Both slices contain exactly `n > 0` elements, unit strides
        // keep every access in bounds, the DLL is loaded, and CBLAS does not
        // retain the pointers.
        Ok(unsafe { (self.ddot)(n, left.as_ptr(), UNIT_STRIDE, right.as_ptr(), UNIT_STRIDE) })
    }

    pub(crate) fn dnrm2(&self, n: i32, input: &[f64]) -> Result<f64, CallError> {
        validate_vector(n, input.len(), "norm input")?;
        // SAFETY: The slice contains exactly `n > 0` elements, unit stride
        // keeps every access in bounds, the DLL is loaded, and CBLAS does not
        // retain the pointer.
        Ok(unsafe { (self.dnrm2)(n, input.as_ptr(), UNIT_STRIDE) })
    }

    pub(crate) fn sgesv(
        &self,
        dimensions: Lp64SolveDimensions,
        coefficients: &mut [f32],
        pivots: &mut [i32],
        right_hand_side: &mut [f32],
    ) -> Result<i32, CallError> {
        validate_solve_buffers(
            dimensions,
            coefficients.len(),
            pivots.len(),
            right_hand_side.len(),
        )?;
        // SAFETY: Validation proves exact LP64 dimensions and complete,
        // disjoint column-major buffers. LAPACKE is synchronous and retains no
        // pointer.
        Ok(unsafe {
            (self.sgesv)(
                COLUMN_MAJOR_LAYOUT,
                dimensions.n,
                dimensions.nrhs,
                coefficients.as_mut_ptr(),
                dimensions.lda,
                pivots.as_mut_ptr(),
                right_hand_side.as_mut_ptr(),
                dimensions.ldb,
            )
        })
    }

    pub(crate) fn cgesv(
        &self,
        dimensions: Lp64SolveDimensions,
        coefficients: &mut [[f32; 2]],
        pivots: &mut [i32],
        right_hand_side: &mut [[f32; 2]],
    ) -> Result<i32, CallError> {
        validate_solve_buffers(
            dimensions,
            coefficients.len(),
            pivots.len(),
            right_hand_side.len(),
        )?;
        // SAFETY: The same proof as `sgesv` applies. Complex values cross as
        // explicit adjacent `[f32; 2]` pairs and never expose a Rust complex
        // layout.
        Ok(unsafe {
            (self.cgesv)(
                COLUMN_MAJOR_LAYOUT,
                dimensions.n,
                dimensions.nrhs,
                coefficients.as_mut_ptr().cast(),
                dimensions.lda,
                pivots.as_mut_ptr(),
                right_hand_side.as_mut_ptr().cast(),
                dimensions.ldb,
            )
        })
    }

    pub(crate) fn dgesv(
        &self,
        dimensions: Lp64SolveDimensions,
        coefficients: &mut [f64],
        pivots: &mut [i32],
        right_hand_side: &mut [f64],
    ) -> Result<i32, CallError> {
        validate_solve_buffers(
            dimensions,
            coefficients.len(),
            pivots.len(),
            right_hand_side.len(),
        )?;
        // SAFETY: `validate_solve_buffers` proves `n > 0`, `nrhs >= 0`, exact
        // column-major leading dimensions, and complete, disjoint buffers for
        // A, B, and `ipiv`. All sizes are already checked LP64 `i32` values.
        // LAPACKE is synchronous and retains no pointer. The owning DLL handle
        // remains alive through `self` for the entire call.
        Ok(unsafe {
            (self.dgesv)(
                COLUMN_MAJOR_LAYOUT,
                dimensions.n,
                dimensions.nrhs,
                coefficients.as_mut_ptr(),
                dimensions.lda,
                pivots.as_mut_ptr(),
                right_hand_side.as_mut_ptr(),
                dimensions.ldb,
            )
        })
    }

    pub(crate) fn zgesv(
        &self,
        dimensions: Lp64SolveDimensions,
        coefficients: &mut [[f64; 2]],
        pivots: &mut [i32],
        right_hand_side: &mut [[f64; 2]],
    ) -> Result<i32, CallError> {
        validate_solve_buffers(
            dimensions,
            coefficients.len(),
            pivots.len(),
            right_hand_side.len(),
        )?;
        // SAFETY: The same bounds, lifetime, and LP64 proof as `dgesv`
        // applies. Each complex value is an explicit `[f64; 2]` (real then
        // imaginary), the representation required by OpenBLAS LAPACKE. No
        // Rust complex type crosses the C ABI.
        Ok(unsafe {
            (self.zgesv)(
                COLUMN_MAJOR_LAYOUT,
                dimensions.n,
                dimensions.nrhs,
                coefficients.as_mut_ptr().cast(),
                dimensions.lda,
                pivots.as_mut_ptr(),
                right_hand_side.as_mut_ptr().cast(),
                dimensions.ldb,
            )
        })
    }

    pub(crate) fn sgels(
        &self,
        dimensions: Lp64RectangularSolveDimensions,
        coefficients: &mut [f32],
        right_hand_side: &mut [f32],
    ) -> Result<i32, CallError> {
        validate_rectangular_solve_buffers(dimensions, coefficients.len(), right_hand_side.len())?;
        // SAFETY: Validation proves exact LP64 dimensions and complete,
        // disjoint column-major buffers. `N` selects `A * X = B`; LAPACKE is
        // synchronous and retains no pointer.
        Ok(unsafe {
            (self.sgels)(
                COLUMN_MAJOR_LAYOUT,
                LAPACK_NO_TRANS,
                dimensions.m,
                dimensions.n,
                dimensions.nrhs,
                coefficients.as_mut_ptr(),
                dimensions.lda,
                right_hand_side.as_mut_ptr(),
                dimensions.ldb,
            )
        })
    }

    pub(crate) fn cgels(
        &self,
        dimensions: Lp64RectangularSolveDimensions,
        coefficients: &mut [[f32; 2]],
        right_hand_side: &mut [[f32; 2]],
    ) -> Result<i32, CallError> {
        validate_rectangular_solve_buffers(dimensions, coefficients.len(), right_hand_side.len())?;
        // SAFETY: The same proof as `sgels` applies, with complex values stored
        // only as explicit adjacent `[f32; 2]` pairs.
        Ok(unsafe {
            (self.cgels)(
                COLUMN_MAJOR_LAYOUT,
                LAPACK_NO_TRANS,
                dimensions.m,
                dimensions.n,
                dimensions.nrhs,
                coefficients.as_mut_ptr().cast(),
                dimensions.lda,
                right_hand_side.as_mut_ptr().cast(),
                dimensions.ldb,
            )
        })
    }

    pub(crate) fn dgels(
        &self,
        dimensions: Lp64RectangularSolveDimensions,
        coefficients: &mut [f64],
        right_hand_side: &mut [f64],
    ) -> Result<i32, CallError> {
        validate_rectangular_solve_buffers(dimensions, coefficients.len(), right_hand_side.len())?;
        // SAFETY: Validation proves positive `m`/`n`, nonnegative `nrhs`,
        // exact LP64 leading dimensions, and complete disjoint column-major
        // buffers. `N` requests `A * X = B`; LAPACKE is synchronous and the
        // owning DLL remains loaded through `self`.
        Ok(unsafe {
            (self.dgels)(
                COLUMN_MAJOR_LAYOUT,
                LAPACK_NO_TRANS,
                dimensions.m,
                dimensions.n,
                dimensions.nrhs,
                coefficients.as_mut_ptr(),
                dimensions.lda,
                right_hand_side.as_mut_ptr(),
                dimensions.ldb,
            )
        })
    }

    pub(crate) fn zgels(
        &self,
        dimensions: Lp64RectangularSolveDimensions,
        coefficients: &mut [[f64; 2]],
        right_hand_side: &mut [[f64; 2]],
    ) -> Result<i32, CallError> {
        validate_rectangular_solve_buffers(dimensions, coefficients.len(), right_hand_side.len())?;
        // SAFETY: The same validation and lifetime proof as `dgels` applies.
        // Complex values are explicit adjacent real/imaginary pairs; no Rust
        // representation crosses the LAPACKE C ABI.
        Ok(unsafe {
            (self.zgels)(
                COLUMN_MAJOR_LAYOUT,
                LAPACK_NO_TRANS,
                dimensions.m,
                dimensions.n,
                dimensions.nrhs,
                coefficients.as_mut_ptr().cast(),
                dimensions.lda,
                right_hand_side.as_mut_ptr().cast(),
                dimensions.ldb,
            )
        })
    }

    real_getrf_method!(sgetrf, sgetrf, f32);
    real_getrf_method!(dgetrf, dgetrf, f64);
    complex_getrf_method!(cgetrf, cgetrf, f32);
    complex_getrf_method!(zgetrf, zgetrf, f64);
    real_geqrf_method!(sgeqrf, sgeqrf, f32);
    real_geqrf_method!(dgeqrf, dgeqrf, f64);
    complex_geqrf_method!(cgeqrf, cgeqrf, f32);
    complex_geqrf_method!(zgeqrf, zgeqrf, f64);
    real_generate_q_method!(sorgqr, sorgqr, f32);
    real_generate_q_method!(dorgqr, dorgqr, f64);
    complex_generate_q_method!(cungqr, cungqr, f32);
    complex_generate_q_method!(zungqr, zungqr, f64);
    real_geqp3_method!(sgeqp3, sgeqp3, f32);
    real_geqp3_method!(dgeqp3, dgeqp3, f64);
    complex_geqp3_method!(cgeqp3, cgeqp3, f32);
    complex_geqp3_method!(zgeqp3, zgeqp3, f64);
    real_potrf_method!(spotrf, spotrf, f32);
    real_potrf_method!(dpotrf, dpotrf, f64);
    complex_potrf_method!(cpotrf, cpotrf, f32);
    complex_potrf_method!(zpotrf, zpotrf, f64);
    real_gesdd_method!(sgesdd, sgesdd, f32);
    real_gesdd_method!(dgesdd, dgesdd, f64);
    complex_gesdd_method!(cgesdd, cgesdd, f32);
    complex_gesdd_method!(zgesdd, zgesdd, f64);
    real_geev_method!(sgeev, sgeev, f32);
    real_geev_method!(dgeev, dgeev, f64);
    complex_geev_method!(cgeev, cgeev, f32);
    complex_geev_method!(zgeev, zgeev, f64);
    complex_gees_method!(cgees, cgees, f32);
    complex_gees_method!(zgees, zgees, f64);
}

#[derive(Debug)]
pub(crate) enum CallError {
    NonPositiveDimension {
        parameter: &'static str,
        value: i32,
    },
    LeadingDimension {
        parameter: &'static str,
        expected: i32,
        actual: i32,
    },
    BufferLength {
        operand: &'static str,
        expected: usize,
        actual: usize,
    },
    HostLengthOverflow {
        operand: &'static str,
    },
    DimensionOutOfRange {
        parameter: &'static str,
        value: i32,
        minimum: i32,
        maximum: i32,
    },
}

impl fmt::Display for CallError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonPositiveDimension { parameter, value } => {
                write!(formatter, "internal {parameter}={value} is not positive")
            }
            Self::LeadingDimension {
                parameter,
                expected,
                actual,
            } => write!(
                formatter,
                "internal {parameter}={actual} does not equal required value {expected}"
            ),
            Self::BufferLength {
                operand,
                expected,
                actual,
            } => write!(
                formatter,
                "internal {operand} buffer requires {expected} elements, received {actual}"
            ),
            Self::HostLengthOverflow { operand } => {
                write!(
                    formatter,
                    "internal {operand} buffer length overflows usize"
                )
            }
            Self::DimensionOutOfRange {
                parameter,
                value,
                minimum,
                maximum,
            } => write!(
                formatter,
                "internal {parameter}={value} is outside {minimum}..={maximum}"
            ),
        }
    }
}

fn validate_gemm_buffers(
    dimensions: Lp64GemmDimensions,
    left_transpose: MatrixTranspose,
    right_transpose: MatrixTranspose,
    left_len: usize,
    right_len: usize,
    output_len: usize,
) -> Result<(), CallError> {
    positive("m", dimensions.m)?;
    positive("n", dimensions.n)?;
    positive("k", dimensions.k)?;

    let left_rows = match left_transpose {
        MatrixTranspose::None => dimensions.m,
        MatrixTranspose::Transpose | MatrixTranspose::ConjugateTranspose => dimensions.k,
    };
    let right_rows = match right_transpose {
        MatrixTranspose::None => dimensions.k,
        MatrixTranspose::Transpose | MatrixTranspose::ConjugateTranspose => dimensions.n,
    };
    leading_dimension("lda", left_rows, dimensions.lda)?;
    leading_dimension("ldb", right_rows, dimensions.ldb)?;
    leading_dimension("ldc", dimensions.m, dimensions.ldc)?;

    exact_product_len("left GEMM operand", dimensions.m, dimensions.k, left_len)?;
    exact_product_len("right GEMM operand", dimensions.k, dimensions.n, right_len)?;
    exact_product_len("GEMM output", dimensions.m, dimensions.n, output_len)
}

fn validate_solve_buffers(
    dimensions: Lp64SolveDimensions,
    coefficient_len: usize,
    pivot_len: usize,
    right_hand_side_len: usize,
) -> Result<(), CallError> {
    positive("n", dimensions.n)?;
    nonnegative("nrhs", dimensions.nrhs)?;
    leading_dimension("lda", dimensions.n, dimensions.lda)?;
    leading_dimension("ldb", dimensions.n, dimensions.ldb)?;
    exact_product_len(
        "linear solve coefficient matrix",
        dimensions.n,
        dimensions.n,
        coefficient_len,
    )?;
    validate_vector(dimensions.n, pivot_len, "linear solve pivot buffer")?;
    exact_product_len(
        "linear solve right-hand side",
        dimensions.n,
        dimensions.nrhs,
        right_hand_side_len,
    )
}

fn validate_rectangular_solve_buffers(
    dimensions: Lp64RectangularSolveDimensions,
    coefficient_len: usize,
    right_hand_side_len: usize,
) -> Result<(), CallError> {
    positive("m", dimensions.m)?;
    positive("n", dimensions.n)?;
    nonnegative("nrhs", dimensions.nrhs)?;
    leading_dimension("lda", dimensions.m, dimensions.lda)?;
    let workspace_rows = dimensions.m.max(dimensions.n);
    leading_dimension("ldb", workspace_rows, dimensions.ldb)?;
    exact_product_len(
        "rectangular solve coefficient matrix",
        dimensions.m,
        dimensions.n,
        coefficient_len,
    )?;
    exact_product_len(
        "rectangular solve right-hand-side workspace",
        workspace_rows,
        dimensions.nrhs,
        right_hand_side_len,
    )
}

fn validate_factor_buffers(
    dimensions: Lp64FactorDimensions,
    matrix_len: usize,
) -> Result<(), CallError> {
    positive("m", dimensions.m)?;
    positive("n", dimensions.n)?;
    positive("k", dimensions.k)?;
    leading_dimension("lda", dimensions.m, dimensions.lda)?;
    exact_product_len(
        "factorization matrix",
        dimensions.m,
        dimensions.n,
        matrix_len,
    )
}

fn validate_qr_buffers(
    dimensions: Lp64FactorDimensions,
    matrix_len: usize,
    tau_len: usize,
) -> Result<(), CallError> {
    validate_factor_buffers(dimensions, matrix_len)?;
    validate_vector(dimensions.k, tau_len, "QR reflector buffer")
}

fn validate_generate_q_buffers(
    dimensions: Lp64FactorDimensions,
    q_columns: i32,
    matrix_len: usize,
    tau_len: usize,
) -> Result<(), CallError> {
    positive("m", dimensions.m)?;
    positive("k", dimensions.k)?;
    if q_columns < dimensions.k || q_columns > dimensions.m {
        return Err(CallError::DimensionOutOfRange {
            parameter: "Q columns",
            value: q_columns,
            minimum: dimensions.k,
            maximum: dimensions.m,
        });
    }
    leading_dimension("lda", dimensions.m, dimensions.lda)?;
    exact_product_len("explicit Q workspace", dimensions.m, q_columns, matrix_len)?;
    validate_vector(dimensions.k, tau_len, "QR reflector buffer")
}

fn validate_cholesky_buffer(
    dimensions: Lp64CholeskyDimensions,
    matrix_len: usize,
) -> Result<(), CallError> {
    positive("n", dimensions.n)?;
    leading_dimension("lda", dimensions.n, dimensions.lda)?;
    exact_product_len(
        "Cholesky factorization matrix",
        dimensions.n,
        dimensions.n,
        matrix_len,
    )
}

#[allow(clippy::too_many_arguments)]
fn validate_svd_buffers(
    dimensions: Lp64SpectralDimensions,
    vectors: SvdVectors,
    matrix_len: usize,
    singular_values_len: usize,
    u_len: usize,
    ldu: i32,
    vt_len: usize,
    ldvt: i32,
) -> Result<(), CallError> {
    positive("m", dimensions.m)?;
    positive("n", dimensions.n)?;
    positive("k", dimensions.k)?;
    leading_dimension("lda", dimensions.m, dimensions.lda)?;
    exact_product_len("SVD matrix", dimensions.m, dimensions.n, matrix_len)?;
    validate_vector(dimensions.k, singular_values_len, "SVD singular values")?;
    match vectors {
        SvdVectors::None => {
            leading_dimension("ldu", 0, ldu)?;
            leading_dimension("ldvt", 0, ldvt)?;
            exact_product_len("SVD omitted U", 0, 0, u_len)?;
            exact_product_len("SVD omitted Vt", 0, 0, vt_len)
        }
        SvdVectors::Thin => {
            leading_dimension("ldu", dimensions.m, ldu)?;
            leading_dimension("ldvt", dimensions.k, ldvt)?;
            exact_product_len("SVD thin U", dimensions.m, dimensions.k, u_len)?;
            exact_product_len("SVD thin Vt", dimensions.k, dimensions.n, vt_len)
        }
        SvdVectors::Full => {
            leading_dimension("ldu", dimensions.m, ldu)?;
            leading_dimension("ldvt", dimensions.n, ldvt)?;
            exact_product_len("SVD full U", dimensions.m, dimensions.m, u_len)?;
            exact_product_len("SVD full Vt", dimensions.n, dimensions.n, vt_len)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_eig_buffers(
    dimensions: Lp64SpectralDimensions,
    left_vectors: bool,
    right_vectors: bool,
    matrix_len: usize,
    values_len: usize,
    second_values_len: usize,
    left_len: usize,
    right_len: usize,
) -> Result<(), CallError> {
    positive("n", dimensions.n)?;
    if dimensions.m != dimensions.n || dimensions.k != dimensions.n {
        return Err(CallError::DimensionOutOfRange {
            parameter: "eig square order",
            value: dimensions.m,
            minimum: dimensions.n,
            maximum: dimensions.n,
        });
    }
    leading_dimension("lda", dimensions.n, dimensions.lda)?;
    exact_product_len("eig matrix", dimensions.n, dimensions.n, matrix_len)?;
    validate_vector(dimensions.n, values_len, "eig values")?;
    validate_vector(dimensions.n, second_values_len, "eig secondary values")?;
    exact_product_len(
        "eig left vectors",
        if left_vectors { dimensions.n } else { 0 },
        if left_vectors { dimensions.n } else { 0 },
        left_len,
    )?;
    exact_product_len(
        "eig right vectors",
        if right_vectors { dimensions.n } else { 0 },
        if right_vectors { dimensions.n } else { 0 },
        right_len,
    )
}

fn validate_schur_buffers(
    dimensions: Lp64SpectralDimensions,
    matrix_len: usize,
    values_len: usize,
    vectors_len: usize,
) -> Result<(), CallError> {
    positive("n", dimensions.n)?;
    if dimensions.m != dimensions.n || dimensions.k != dimensions.n {
        return Err(CallError::DimensionOutOfRange {
            parameter: "Schur square order",
            value: dimensions.m,
            minimum: dimensions.n,
            maximum: dimensions.n,
        });
    }
    leading_dimension("lda", dimensions.n, dimensions.lda)?;
    exact_product_len("Schur matrix", dimensions.n, dimensions.n, matrix_len)?;
    validate_vector(dimensions.n, values_len, "Schur eigenvalues")?;
    exact_product_len("Schur vectors", dimensions.n, dimensions.n, vectors_len)
}

fn positive(parameter: &'static str, value: i32) -> Result<(), CallError> {
    if value <= 0 {
        Err(CallError::NonPositiveDimension { parameter, value })
    } else {
        Ok(())
    }
}

fn nonnegative(parameter: &'static str, value: i32) -> Result<(), CallError> {
    if value < 0 {
        Err(CallError::NonPositiveDimension { parameter, value })
    } else {
        Ok(())
    }
}

fn leading_dimension(
    parameter: &'static str,
    physical_rows: i32,
    actual: i32,
) -> Result<(), CallError> {
    let expected = physical_rows.max(1);
    if actual == expected {
        Ok(())
    } else {
        Err(CallError::LeadingDimension {
            parameter,
            expected,
            actual,
        })
    }
}

fn exact_product_len(
    operand: &'static str,
    rows: i32,
    columns: i32,
    actual: usize,
) -> Result<(), CallError> {
    let rows = usize::try_from(rows).map_err(|_| CallError::HostLengthOverflow { operand })?;
    let columns =
        usize::try_from(columns).map_err(|_| CallError::HostLengthOverflow { operand })?;
    let expected = rows
        .checked_mul(columns)
        .ok_or(CallError::HostLengthOverflow { operand })?;
    if actual == expected {
        Ok(())
    } else {
        Err(CallError::BufferLength {
            operand,
            expected,
            actual,
        })
    }
}

fn validate_vector(n: i32, actual: usize, operand: &'static str) -> Result<(), CallError> {
    positive("n", n)?;
    let expected = usize::try_from(n).map_err(|_| CallError::HostLengthOverflow { operand })?;
    if actual == expected {
        Ok(())
    } else {
        Err(CallError::BufferLength {
            operand,
            expected,
            actual,
        })
    }
}

const fn real_transpose(transpose: MatrixTranspose) -> c_int {
    match transpose {
        MatrixTranspose::None => CBLAS_NO_TRANS,
        MatrixTranspose::Transpose | MatrixTranspose::ConjugateTranspose => CBLAS_TRANS,
    }
}

const fn complex_transpose(transpose: MatrixTranspose) -> c_int {
    match transpose {
        MatrixTranspose::None => CBLAS_NO_TRANS,
        MatrixTranspose::Transpose => CBLAS_TRANS,
        MatrixTranspose::ConjugateTranspose => CBLAS_CONJ_TRANS,
    }
}

const fn lapack_triangle(triangle: CholeskyTriangle) -> c_char {
    match triangle {
        CholeskyTriangle::Upper => LAPACK_UPPER,
        CholeskyTriangle::Lower => LAPACK_LOWER,
    }
}

const fn svd_job(vectors: SvdVectors) -> c_char {
    match vectors {
        SvdVectors::None => LAPACK_NO_VECTORS,
        SvdVectors::Thin => LAPACK_SOME_VECTORS,
        SvdVectors::Full => LAPACK_ALL_VECTORS,
    }
}

const fn eig_job(vectors: bool) -> c_char {
    if vectors {
        LAPACK_VECTORS
    } else {
        LAPACK_NO_VECTORS
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]
    #![allow(clippy::similar_names)]
    #![allow(dead_code)]

    use std::ffi::{c_char, c_int, c_void};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU32, Ordering};

    use openmat_linalg::{
        CholeskyTriangle, Lp64CholeskyDimensions, Lp64FactorDimensions,
        Lp64RectangularSolveDimensions, Lp64SolveDimensions, Lp64SpectralDimensions, SvdVectors,
    };

    use super::{
        CallError, Library, Lp64GemmDimensions, MatrixTranspose,
        validate_rectangular_solve_buffers, validate_solve_buffers,
    };

    const SGEMM_CALLED: u32 = 1 << 0;
    const CGEMM_CALLED: u32 = 1 << 1;
    const SGESV_CALLED: u32 = 1 << 2;
    const CGESV_CALLED: u32 = 1 << 3;
    const SGELS_CALLED: u32 = 1 << 4;
    const CGELS_CALLED: u32 = 1 << 5;
    const SGETRF_CALLED: u32 = 1 << 6;
    const DGETRF_CALLED: u32 = 1 << 7;
    const CGETRF_CALLED: u32 = 1 << 8;
    const ZGETRF_CALLED: u32 = 1 << 9;
    const SGEQRF_CALLED: u32 = 1 << 10;
    const DGEQRF_CALLED: u32 = 1 << 11;
    const CGEQRF_CALLED: u32 = 1 << 12;
    const ZGEQRF_CALLED: u32 = 1 << 13;
    const SORGQR_CALLED: u32 = 1 << 14;
    const DORGQR_CALLED: u32 = 1 << 15;
    const CUNGQR_CALLED: u32 = 1 << 16;
    const ZUNGQR_CALLED: u32 = 1 << 17;
    const SGEQP3_CALLED: u32 = 1 << 18;
    const DGEQP3_CALLED: u32 = 1 << 19;
    const CGEQP3_CALLED: u32 = 1 << 20;
    const ZGEQP3_CALLED: u32 = 1 << 21;
    const SPOTRF_CALLED: u32 = 1 << 22;
    const DPOTRF_CALLED: u32 = 1 << 23;
    const CPOTRF_CALLED: u32 = 1 << 24;
    const ZPOTRF_CALLED: u32 = 1 << 25;
    const SGESDD_SPECTRAL: u32 = 1 << 0;
    const DGESDD_SPECTRAL: u32 = 1 << 1;
    const CGESDD_SPECTRAL: u32 = 1 << 2;
    const ZGESDD_SPECTRAL: u32 = 1 << 3;
    const SGEEV_SPECTRAL: u32 = 1 << 4;
    const DGEEV_SPECTRAL: u32 = 1 << 5;
    const CGEEV_SPECTRAL: u32 = 1 << 6;
    const ZGEEV_SPECTRAL: u32 = 1 << 7;
    const CGEES_SPECTRAL: u32 = 1 << 8;
    const ZGEES_SPECTRAL: u32 = 1 << 9;
    const ALL_SPECTRAL_CALLS: u32 = (1 << 10) - 1;
    const MOCK_FAILED: u32 = 1 << 31;
    const ALL_SINGLE_CALLS: u32 =
        SGEMM_CALLED | CGEMM_CALLED | SGESV_CALLED | CGESV_CALLED | SGELS_CALLED | CGELS_CALLED;
    const ALL_DECOMPOSITION_CALLS: u32 = SGETRF_CALLED
        | DGETRF_CALLED
        | CGETRF_CALLED
        | ZGETRF_CALLED
        | SGEQRF_CALLED
        | DGEQRF_CALLED
        | CGEQRF_CALLED
        | ZGEQRF_CALLED
        | SORGQR_CALLED
        | DORGQR_CALLED
        | CUNGQR_CALLED
        | ZUNGQR_CALLED
        | SGEQP3_CALLED
        | DGEQP3_CALLED
        | CGEQP3_CALLED
        | ZGEQP3_CALLED
        | SPOTRF_CALLED
        | DPOTRF_CALLED
        | CPOTRF_CALLED
        | ZPOTRF_CALLED;

    static MOCK_FLAGS: AtomicU32 = AtomicU32::new(0);
    static SPECTRAL_FLAGS: AtomicU32 = AtomicU32::new(0);
    static MOCK_LOCK: Mutex<()> = Mutex::new(());

    fn record_mock(bit: u32, valid: bool) {
        let flags = bit | if valid { 0 } else { MOCK_FAILED };
        MOCK_FLAGS.fetch_or(flags, Ordering::SeqCst);
    }

    fn record_spectral(bit: u32, valid: bool) {
        assert!(valid, "invalid spectral mock ABI call for bit {bit:#x}");
        SPECTRAL_FLAGS.fetch_or(bit, Ordering::SeqCst);
    }

    unsafe fn read_pair(pointer: *const c_void) -> Option<[f32; 2]> {
        if pointer.is_null() {
            None
        } else {
            let components = pointer.cast::<f32>();
            // SAFETY: Mock callers pass pointers to explicit adjacent f32
            // pairs. The null case was rejected above.
            Some(unsafe { [*components, *components.add(1)] })
        }
    }

    unsafe fn read_pair64(pointer: *const c_void) -> Option<[f64; 2]> {
        if pointer.is_null() {
            None
        } else {
            let components = pointer.cast::<f64>();
            // SAFETY: Mock callers pass pointers to explicit adjacent f64
            // pairs. The null case was rejected above.
            Some(unsafe { [*components, *components.add(1)] })
        }
    }

    #[allow(clippy::too_many_arguments)]
    unsafe extern "C" fn mock_sgemm(
        layout: c_int,
        left_transpose: c_int,
        right_transpose: c_int,
        m: i32,
        n: i32,
        k: i32,
        alpha: f32,
        left: *const f32,
        lda: i32,
        right: *const f32,
        ldb: i32,
        beta: f32,
        output: *mut f32,
        ldc: i32,
    ) {
        let valid = layout == 102
            && left_transpose == 112
            && right_transpose == 111
            && [m, n, k, lda, ldb, ldc] == [1; 6]
            && alpha == 2.0
            && beta == 3.0
            && !left.is_null()
            && !right.is_null()
            && !output.is_null()
            // SAFETY: The wrapper validation supplies one-element buffers for
            // this 1x1 mock call.
            && unsafe { *left == 4.0 && *right == 5.0 && *output == 6.0 };
        if !output.is_null() {
            // SAFETY: The wrapper supplies a writable one-element output.
            unsafe { *output = 46.0 };
        }
        record_mock(SGEMM_CALLED, valid);
    }

    #[allow(clippy::too_many_arguments)]
    unsafe extern "C" fn mock_cgemm(
        layout: c_int,
        left_transpose: c_int,
        right_transpose: c_int,
        m: i32,
        n: i32,
        k: i32,
        alpha: *const c_void,
        left: *const c_void,
        lda: i32,
        right: *const c_void,
        ldb: i32,
        beta: *const c_void,
        output: *mut c_void,
        ldc: i32,
    ) {
        // SAFETY: The mock ABI contract supplies adjacent f32 pairs.
        let alpha_pair = unsafe { read_pair(alpha) };
        // SAFETY: The mock ABI contract supplies adjacent f32 pairs.
        let left_pair = unsafe { read_pair(left) };
        // SAFETY: The mock ABI contract supplies adjacent f32 pairs.
        let right_pair = unsafe { read_pair(right) };
        // SAFETY: The mock ABI contract supplies adjacent f32 pairs.
        let beta_pair = unsafe { read_pair(beta) };
        // SAFETY: The output pointer has the same pair representation.
        let output_pair = unsafe { read_pair(output.cast_const()) };
        let valid = layout == 102
            && left_transpose == 113
            && right_transpose == 112
            && [m, n, k, lda, ldb, ldc] == [1; 6]
            && alpha_pair == Some([2.0, -1.0])
            && left_pair == Some([1.0, 2.0])
            && right_pair == Some([3.0, 4.0])
            && beta_pair == Some([-0.5, 0.25])
            && output_pair == Some([5.0, 6.0]);
        if !output.is_null() {
            let output = output.cast::<f32>();
            // SAFETY: The wrapper supplies one writable adjacent pair.
            unsafe {
                *output = 7.0;
                *output.add(1) = 8.0;
            }
        }
        record_mock(CGEMM_CALLED, valid);
    }

    unsafe extern "C" fn mock_sgesv(
        layout: c_int,
        n: i32,
        nrhs: i32,
        coefficients: *mut f32,
        lda: i32,
        pivots: *mut i32,
        right_hand_side: *mut f32,
        ldb: i32,
    ) -> i32 {
        let valid = layout == 102
            && [n, nrhs, lda, ldb] == [1; 4]
            && !coefficients.is_null()
            && !pivots.is_null()
            && !right_hand_side.is_null()
            // SAFETY: The wrapper supplies one-element buffers.
            && unsafe { *coefficients == 2.0 && *right_hand_side == 4.0 };
        if !pivots.is_null() && !right_hand_side.is_null() {
            // SAFETY: The wrapper supplies writable one-element buffers.
            unsafe {
                *pivots = 1;
                *right_hand_side = 2.0;
            }
        }
        record_mock(SGESV_CALLED, valid);
        0
    }

    unsafe extern "C" fn mock_cgesv(
        layout: c_int,
        n: i32,
        nrhs: i32,
        coefficients: *mut c_void,
        lda: i32,
        pivots: *mut i32,
        right_hand_side: *mut c_void,
        ldb: i32,
    ) -> i32 {
        // SAFETY: The wrapper supplies adjacent f32 pairs.
        let coefficients_pair = unsafe { read_pair(coefficients.cast_const()) };
        // SAFETY: The wrapper supplies adjacent f32 pairs.
        let rhs_pair = unsafe { read_pair(right_hand_side.cast_const()) };
        let valid = layout == 102
            && [n, nrhs, lda, ldb] == [1; 4]
            && !pivots.is_null()
            && coefficients_pair == Some([1.0, 2.0])
            && rhs_pair == Some([5.0, 6.0]);
        if !pivots.is_null() && !right_hand_side.is_null() {
            let rhs = right_hand_side.cast::<f32>();
            // SAFETY: The wrapper supplies one writable pair and one pivot.
            unsafe {
                *pivots = 1;
                *rhs = 3.0;
                *rhs.add(1) = -4.0;
            }
        }
        record_mock(CGESV_CALLED, valid);
        0
    }

    unsafe extern "C" fn mock_sgels(
        layout: c_int,
        transpose: c_char,
        m: i32,
        n: i32,
        nrhs: i32,
        coefficients: *mut f32,
        lda: i32,
        right_hand_side: *mut f32,
        ldb: i32,
    ) -> i32 {
        let valid = layout == 102
            && transpose == 78
            && [m, n, nrhs, lda, ldb] == [2, 1, 1, 2, 2]
            && !coefficients.is_null()
            && !right_hand_side.is_null()
            // SAFETY: The wrapper supplies two-element buffers.
            && unsafe { *coefficients == 1.0 && *right_hand_side == 3.0 };
        if !right_hand_side.is_null() {
            // SAFETY: The wrapper supplies a writable two-element workspace.
            unsafe { *right_hand_side = 9.0 };
        }
        record_mock(SGELS_CALLED, valid);
        0
    }

    unsafe extern "C" fn mock_cgels(
        layout: c_int,
        transpose: c_char,
        m: i32,
        n: i32,
        nrhs: i32,
        coefficients: *mut c_void,
        lda: i32,
        right_hand_side: *mut c_void,
        ldb: i32,
    ) -> i32 {
        // SAFETY: The wrapper supplies adjacent f32 pairs.
        let coefficients_pair = unsafe { read_pair(coefficients.cast_const()) };
        // SAFETY: The wrapper supplies adjacent f32 pairs.
        let rhs_pair = unsafe { read_pair(right_hand_side.cast_const()) };
        let valid = layout == 102
            && transpose == 78
            && [m, n, nrhs, lda, ldb] == [2, 1, 1, 2, 2]
            && coefficients_pair == Some([1.0, -1.0])
            && rhs_pair == Some([2.0, 3.0]);
        if !right_hand_side.is_null() {
            let rhs = right_hand_side.cast::<f32>();
            // SAFETY: The wrapper supplies a writable adjacent pair.
            unsafe {
                *rhs = -2.0;
                *rhs.add(1) = 4.0;
            }
        }
        record_mock(CGELS_CALLED, valid);
        0
    }

    #[allow(clippy::too_many_arguments)]
    unsafe extern "C" fn unused_dgemm(
        _: c_int,
        _: c_int,
        _: c_int,
        _: i32,
        _: i32,
        _: i32,
        _: f64,
        _: *const f64,
        _: i32,
        _: *const f64,
        _: i32,
        _: f64,
        _: *mut f64,
        _: i32,
    ) {
    }

    #[allow(clippy::too_many_arguments)]
    unsafe extern "C" fn unused_zgemm(
        _: c_int,
        _: c_int,
        _: c_int,
        _: i32,
        _: i32,
        _: i32,
        _: *const c_void,
        _: *const c_void,
        _: i32,
        _: *const c_void,
        _: i32,
        _: *const c_void,
        _: *mut c_void,
        _: i32,
    ) {
    }

    unsafe extern "C" fn unused_ddot(_: i32, _: *const f64, _: i32, _: *const f64, _: i32) -> f64 {
        0.0
    }

    unsafe extern "C" fn unused_dnrm2(_: i32, _: *const f64, _: i32) -> f64 {
        0.0
    }

    unsafe extern "C" fn unused_dgesv(
        _: c_int,
        _: i32,
        _: i32,
        _: *mut f64,
        _: i32,
        _: *mut i32,
        _: *mut f64,
        _: i32,
    ) -> i32 {
        0
    }

    unsafe extern "C" fn unused_zgesv(
        _: c_int,
        _: i32,
        _: i32,
        _: *mut c_void,
        _: i32,
        _: *mut i32,
        _: *mut c_void,
        _: i32,
    ) -> i32 {
        0
    }

    unsafe extern "C" fn unused_dgels(
        _: c_int,
        _: c_char,
        _: i32,
        _: i32,
        _: i32,
        _: *mut f64,
        _: i32,
        _: *mut f64,
        _: i32,
    ) -> i32 {
        0
    }

    unsafe extern "C" fn unused_zgels(
        _: c_int,
        _: c_char,
        _: i32,
        _: i32,
        _: i32,
        _: *mut c_void,
        _: i32,
        _: *mut c_void,
        _: i32,
    ) -> i32 {
        0
    }

    unsafe extern "C" fn unused_sgesdd(
        _: c_int,
        _: c_char,
        _: i32,
        _: i32,
        _: *mut f32,
        _: i32,
        _: *mut f32,
        _: *mut f32,
        _: i32,
        _: *mut f32,
        _: i32,
    ) -> i32 {
        0
    }

    unsafe extern "C" fn unused_dgesdd(
        _: c_int,
        _: c_char,
        _: i32,
        _: i32,
        _: *mut f64,
        _: i32,
        _: *mut f64,
        _: *mut f64,
        _: i32,
        _: *mut f64,
        _: i32,
    ) -> i32 {
        0
    }

    unsafe extern "C" fn unused_cgesdd(
        _: c_int,
        _: c_char,
        _: i32,
        _: i32,
        _: *mut c_void,
        _: i32,
        _: *mut f32,
        _: *mut c_void,
        _: i32,
        _: *mut c_void,
        _: i32,
    ) -> i32 {
        0
    }

    unsafe extern "C" fn unused_zgesdd(
        _: c_int,
        _: c_char,
        _: i32,
        _: i32,
        _: *mut c_void,
        _: i32,
        _: *mut f64,
        _: *mut c_void,
        _: i32,
        _: *mut c_void,
        _: i32,
    ) -> i32 {
        0
    }

    unsafe extern "C" fn unused_sgeev(
        _: c_int,
        _: c_char,
        _: c_char,
        _: i32,
        _: *mut f32,
        _: i32,
        _: *mut f32,
        _: *mut f32,
        _: *mut f32,
        _: i32,
        _: *mut f32,
        _: i32,
    ) -> i32 {
        0
    }

    unsafe extern "C" fn unused_dgeev(
        _: c_int,
        _: c_char,
        _: c_char,
        _: i32,
        _: *mut f64,
        _: i32,
        _: *mut f64,
        _: *mut f64,
        _: *mut f64,
        _: i32,
        _: *mut f64,
        _: i32,
    ) -> i32 {
        0
    }

    unsafe extern "C" fn unused_cgeev(
        _: c_int,
        _: c_char,
        _: c_char,
        _: i32,
        _: *mut c_void,
        _: i32,
        _: *mut c_void,
        _: *mut c_void,
        _: i32,
        _: *mut c_void,
        _: i32,
    ) -> i32 {
        0
    }

    unsafe extern "C" fn unused_zgeev(
        _: c_int,
        _: c_char,
        _: c_char,
        _: i32,
        _: *mut c_void,
        _: i32,
        _: *mut c_void,
        _: *mut c_void,
        _: i32,
        _: *mut c_void,
        _: i32,
    ) -> i32 {
        0
    }

    macro_rules! mock_real_gesdd {
        ($name:ident, $scalar:ty, $bit:ident) => {
            unsafe extern "C" fn $name(
                layout: c_int,
                job: c_char,
                m: i32,
                n: i32,
                matrix: *mut $scalar,
                lda: i32,
                singular_values: *mut $scalar,
                u: *mut $scalar,
                ldu: i32,
                vt: *mut $scalar,
                ldvt: i32,
            ) -> i32 {
                let valid = layout == 102
                    && matches!(job, 65 | 78 | 83)
                    && [m, n, lda, ldu, ldvt] == [1; 5]
                    && !matrix.is_null()
                    && !singular_values.is_null()
                    && !u.is_null()
                    && !vt.is_null();
                if valid {
                    // SAFETY: The wrapper supplies writable one-element buffers.
                    unsafe {
                        *singular_values = 2.0;
                        if job != 78 {
                            *u = 1.0;
                            *vt = 1.0;
                        }
                    }
                }
                record_spectral($bit, valid);
                0
            }
        };
    }

    macro_rules! mock_complex_gesdd {
        ($name:ident, $component:ty, $bit:ident) => {
            unsafe extern "C" fn $name(
                layout: c_int,
                job: c_char,
                m: i32,
                n: i32,
                matrix: *mut c_void,
                lda: i32,
                singular_values: *mut $component,
                u: *mut c_void,
                ldu: i32,
                vt: *mut c_void,
                ldvt: i32,
            ) -> i32 {
                let valid = layout == 102
                    && matches!(job, 65 | 78 | 83)
                    && [m, n, lda, ldu, ldvt] == [1; 5]
                    && !matrix.is_null()
                    && !singular_values.is_null()
                    && !u.is_null()
                    && !vt.is_null();
                if valid {
                    // SAFETY: The wrapper supplies one real and two adjacent-pair outputs.
                    unsafe {
                        *singular_values = 2.0;
                        if job != 78 {
                            *u.cast::<$component>() = 1.0;
                            *u.cast::<$component>().add(1) = 0.0;
                            *vt.cast::<$component>() = 1.0;
                            *vt.cast::<$component>().add(1) = 0.0;
                        }
                    }
                }
                record_spectral($bit, valid);
                0
            }
        };
    }

    macro_rules! mock_real_geev {
        ($name:ident, $scalar:ty, $bit:ident) => {
            unsafe extern "C" fn $name(
                layout: c_int,
                jobvl: c_char,
                jobvr: c_char,
                n: i32,
                matrix: *mut $scalar,
                lda: i32,
                wr: *mut $scalar,
                wi: *mut $scalar,
                vl: *mut $scalar,
                ldvl: i32,
                vr: *mut $scalar,
                ldvr: i32,
            ) -> i32 {
                let valid = layout == 102
                    && ((jobvl == 86 && jobvr == 86) || (jobvl == 78 && jobvr == 78))
                    && [n, lda, ldvl, ldvr] == [1; 4]
                    && !matrix.is_null()
                    && !wr.is_null()
                    && !wi.is_null()
                    && !vl.is_null()
                    && !vr.is_null();
                if valid {
                    // SAFETY: The wrapper supplies writable one-element buffers.
                    unsafe {
                        *wr = 3.0;
                        *wi = 0.0;
                        if jobvl == 86 {
                            *vl = 1.0;
                            *vr = 1.0;
                        }
                    }
                }
                record_spectral($bit, valid);
                0
            }
        };
    }

    macro_rules! mock_complex_geev {
        ($name:ident, $component:ty, $bit:ident) => {
            unsafe extern "C" fn $name(
                layout: c_int,
                jobvl: c_char,
                jobvr: c_char,
                n: i32,
                matrix: *mut c_void,
                lda: i32,
                values: *mut c_void,
                vl: *mut c_void,
                ldvl: i32,
                vr: *mut c_void,
                ldvr: i32,
            ) -> i32 {
                let valid = layout == 102
                    && ((jobvl == 86 && jobvr == 86) || (jobvl == 78 && jobvr == 78))
                    && [n, lda, ldvl, ldvr] == [1; 4]
                    && !matrix.is_null()
                    && !values.is_null()
                    && !vl.is_null()
                    && !vr.is_null();
                if valid {
                    // SAFETY: The wrapper supplies writable adjacent pairs.
                    unsafe {
                        *values.cast::<$component>() = 3.0;
                        *values.cast::<$component>().add(1) = 4.0;
                        if jobvl == 86 {
                            *vl.cast::<$component>() = 1.0;
                            *vl.cast::<$component>().add(1) = 0.0;
                            *vr.cast::<$component>() = 1.0;
                            *vr.cast::<$component>().add(1) = 0.0;
                        }
                    }
                }
                record_spectral($bit, valid);
                0
            }
        };
    }

    macro_rules! mock_complex_gees {
        ($name:ident, $component:ty, $bit:ident) => {
            unsafe extern "C" fn $name(
                layout: c_int,
                jobvs: c_char,
                sort: c_char,
                select: Option<super::ComplexSelectFn>,
                n: i32,
                matrix: *mut c_void,
                lda: i32,
                selected: *mut i32,
                values: *mut c_void,
                vectors: *mut c_void,
                ldvs: i32,
            ) -> i32 {
                let valid = layout == 102
                    && jobvs == 86
                    && sort == 78
                    && select.is_none()
                    && [n, lda, ldvs] == [1; 3]
                    && !matrix.is_null()
                    && !selected.is_null()
                    && !values.is_null()
                    && !vectors.is_null();
                if valid {
                    // SAFETY: The wrapper supplies one integer and writable
                    // adjacent-pair eigenvalue and Schur-vector buffers.
                    unsafe {
                        *selected = 0;
                        *values.cast::<$component>() = 3.0;
                        *values.cast::<$component>().add(1) = 4.0;
                        *vectors.cast::<$component>() = 1.0;
                        *vectors.cast::<$component>().add(1) = 0.0;
                    }
                }
                record_spectral($bit, valid);
                0
            }
        };
    }

    mock_real_gesdd!(mock_sgesdd, f32, SGESDD_SPECTRAL);
    mock_real_gesdd!(mock_dgesdd, f64, DGESDD_SPECTRAL);
    mock_complex_gesdd!(mock_cgesdd, f32, CGESDD_SPECTRAL);
    mock_complex_gesdd!(mock_zgesdd, f64, ZGESDD_SPECTRAL);
    mock_real_geev!(mock_sgeev, f32, SGEEV_SPECTRAL);
    mock_real_geev!(mock_dgeev, f64, DGEEV_SPECTRAL);
    mock_complex_geev!(mock_cgeev, f32, CGEEV_SPECTRAL);
    mock_complex_geev!(mock_zgeev, f64, ZGEEV_SPECTRAL);
    mock_complex_gees!(mock_cgees, f32, CGEES_SPECTRAL);
    mock_complex_gees!(mock_zgees, f64, ZGEES_SPECTRAL);

    macro_rules! mock_real_getrf {
        ($name:ident, $scalar:ty, $bit:ident) => {
            unsafe extern "C" fn $name(
                layout: c_int,
                m: i32,
                n: i32,
                matrix: *mut $scalar,
                lda: i32,
                pivots: *mut i32,
            ) -> i32 {
                let valid = layout == 102
                    && [m, n, lda] == [1, 1, 1]
                    && !matrix.is_null()
                    && !pivots.is_null()
                    // SAFETY: The wrapper supplies one-element buffers.
                    && unsafe { *matrix == 2.0 };
                if !pivots.is_null() {
                    // SAFETY: The wrapper supplies one writable pivot.
                    unsafe { *pivots = 1 };
                }
                record_mock($bit, valid);
                0
            }
        };
    }

    macro_rules! mock_complex_getrf {
        ($name:ident, $component:ty, $reader:ident, $bit:ident) => {
            unsafe extern "C" fn $name(
                layout: c_int,
                m: i32,
                n: i32,
                matrix: *mut c_void,
                lda: i32,
                pivots: *mut i32,
            ) -> i32 {
                // SAFETY: The wrapper supplies one explicit adjacent pair.
                let pair = unsafe { $reader(matrix.cast_const()) };
                let valid = layout == 102
                    && [m, n, lda] == [1, 1, 1]
                    && !pivots.is_null()
                    && pair == Some([2.0, 3.0]);
                if !pivots.is_null() {
                    // SAFETY: The wrapper supplies one writable pivot.
                    unsafe { *pivots = 1 };
                }
                record_mock($bit, valid);
                0
            }
        };
    }

    macro_rules! mock_real_geqrf {
        ($name:ident, $scalar:ty, $bit:ident) => {
            unsafe extern "C" fn $name(
                layout: c_int,
                m: i32,
                n: i32,
                matrix: *mut $scalar,
                lda: i32,
                tau: *mut $scalar,
            ) -> i32 {
                let valid = layout == 102
                    && [m, n, lda] == [1, 1, 1]
                    && !matrix.is_null()
                    && !tau.is_null()
                    // SAFETY: The wrapper supplies one-element buffers.
                    && unsafe { *matrix == 2.0 };
                if !tau.is_null() {
                    // SAFETY: The wrapper supplies one writable reflector.
                    unsafe { *tau = 4.0 };
                }
                record_mock($bit, valid);
                0
            }
        };
    }

    macro_rules! mock_complex_geqrf {
        ($name:ident, $component:ty, $reader:ident, $bit:ident) => {
            unsafe extern "C" fn $name(
                layout: c_int,
                m: i32,
                n: i32,
                matrix: *mut c_void,
                lda: i32,
                tau: *mut c_void,
            ) -> i32 {
                // SAFETY: The wrapper supplies one explicit adjacent pair.
                let pair = unsafe { $reader(matrix.cast_const()) };
                let valid = layout == 102
                    && [m, n, lda] == [1, 1, 1]
                    && !tau.is_null()
                    && pair == Some([2.0, 3.0]);
                if !tau.is_null() {
                    let tau = tau.cast::<$component>();
                    // SAFETY: The wrapper supplies one writable adjacent pair.
                    unsafe {
                        *tau = 4.0;
                        *tau.add(1) = 5.0;
                    }
                }
                record_mock($bit, valid);
                0
            }
        };
    }

    macro_rules! mock_real_generate_q {
        ($name:ident, $scalar:ty, $bit:ident) => {
            unsafe extern "C" fn $name(
                layout: c_int,
                m: i32,
                n: i32,
                k: i32,
                matrix: *mut $scalar,
                lda: i32,
                tau: *const $scalar,
            ) -> i32 {
                let valid = layout == 102
                    && [m, n, k, lda] == [1, 1, 1, 1]
                    && !matrix.is_null()
                    && !tau.is_null()
                    // SAFETY: The wrapper supplies one-element buffers.
                    && unsafe { *tau == 4.0 };
                if !matrix.is_null() {
                    // SAFETY: The wrapper supplies one writable matrix element.
                    unsafe { *matrix = 7.0 };
                }
                record_mock($bit, valid);
                0
            }
        };
    }

    macro_rules! mock_complex_generate_q {
        ($name:ident, $component:ty, $reader:ident, $bit:ident) => {
            unsafe extern "C" fn $name(
                layout: c_int,
                m: i32,
                n: i32,
                k: i32,
                matrix: *mut c_void,
                lda: i32,
                tau: *const c_void,
            ) -> i32 {
                // SAFETY: The wrapper supplies one explicit adjacent pair.
                let tau_pair = unsafe { $reader(tau) };
                let valid = layout == 102
                    && [m, n, k, lda] == [1, 1, 1, 1]
                    && !matrix.is_null()
                    && tau_pair == Some([4.0, 5.0]);
                if !matrix.is_null() {
                    let matrix = matrix.cast::<$component>();
                    // SAFETY: The wrapper supplies one writable adjacent pair.
                    unsafe {
                        *matrix = 7.0;
                        *matrix.add(1) = 8.0;
                    }
                }
                record_mock($bit, valid);
                0
            }
        };
    }

    macro_rules! mock_real_geqp3 {
        ($name:ident, $scalar:ty, $bit:ident) => {
            unsafe extern "C" fn $name(
                layout: c_int,
                m: i32,
                n: i32,
                matrix: *mut $scalar,
                lda: i32,
                pivots: *mut i32,
                tau: *mut $scalar,
            ) -> i32 {
                let valid = layout == 102
                    && [m, n, lda] == [1, 1, 1]
                    && !matrix.is_null()
                    && !pivots.is_null()
                    && !tau.is_null();
                if !pivots.is_null() && !tau.is_null() {
                    // SAFETY: The wrapper supplies one pivot and reflector.
                    unsafe {
                        *pivots = 1;
                        *tau = 6.0;
                    }
                }
                record_mock($bit, valid);
                0
            }
        };
    }

    macro_rules! mock_complex_geqp3 {
        ($name:ident, $component:ty, $bit:ident) => {
            unsafe extern "C" fn $name(
                layout: c_int,
                m: i32,
                n: i32,
                matrix: *mut c_void,
                lda: i32,
                pivots: *mut i32,
                tau: *mut c_void,
            ) -> i32 {
                let valid = layout == 102
                    && [m, n, lda] == [1, 1, 1]
                    && !matrix.is_null()
                    && !pivots.is_null()
                    && !tau.is_null();
                if !pivots.is_null() && !tau.is_null() {
                    let tau = tau.cast::<$component>();
                    // SAFETY: The wrapper supplies one pivot and adjacent pair.
                    unsafe {
                        *pivots = 1;
                        *tau = 6.0;
                        *tau.add(1) = 7.0;
                    }
                }
                record_mock($bit, valid);
                0
            }
        };
    }

    macro_rules! mock_real_potrf {
        ($name:ident, $scalar:ty, $uplo:expr, $bit:ident) => {
            unsafe extern "C" fn $name(
                layout: c_int,
                uplo: c_char,
                n: i32,
                matrix: *mut $scalar,
                lda: i32,
            ) -> i32 {
                let valid =
                    layout == 102 && uplo == $uplo && [n, lda] == [1, 1] && !matrix.is_null();
                record_mock($bit, valid);
                0
            }
        };
    }

    macro_rules! mock_complex_potrf {
        ($name:ident, $uplo:expr, $bit:ident) => {
            unsafe extern "C" fn $name(
                layout: c_int,
                uplo: c_char,
                n: i32,
                matrix: *mut c_void,
                lda: i32,
            ) -> i32 {
                let valid =
                    layout == 102 && uplo == $uplo && [n, lda] == [1, 1] && !matrix.is_null();
                record_mock($bit, valid);
                0
            }
        };
    }

    mock_real_getrf!(mock_sgetrf, f32, SGETRF_CALLED);
    mock_real_getrf!(mock_dgetrf, f64, DGETRF_CALLED);
    mock_complex_getrf!(mock_cgetrf, f32, read_pair, CGETRF_CALLED);
    mock_complex_getrf!(mock_zgetrf, f64, read_pair64, ZGETRF_CALLED);
    mock_real_geqrf!(mock_sgeqrf, f32, SGEQRF_CALLED);
    mock_real_geqrf!(mock_dgeqrf, f64, DGEQRF_CALLED);
    mock_complex_geqrf!(mock_cgeqrf, f32, read_pair, CGEQRF_CALLED);
    mock_complex_geqrf!(mock_zgeqrf, f64, read_pair64, ZGEQRF_CALLED);
    mock_real_generate_q!(mock_sorgqr, f32, SORGQR_CALLED);
    mock_real_generate_q!(mock_dorgqr, f64, DORGQR_CALLED);
    mock_complex_generate_q!(mock_cungqr, f32, read_pair, CUNGQR_CALLED);
    mock_complex_generate_q!(mock_zungqr, f64, read_pair64, ZUNGQR_CALLED);
    mock_real_geqp3!(mock_sgeqp3, f32, SGEQP3_CALLED);
    mock_real_geqp3!(mock_dgeqp3, f64, DGEQP3_CALLED);
    mock_complex_geqp3!(mock_cgeqp3, f32, CGEQP3_CALLED);
    mock_complex_geqp3!(mock_zgeqp3, f64, ZGEQP3_CALLED);
    mock_real_potrf!(mock_spotrf, f32, 85, SPOTRF_CALLED);
    mock_real_potrf!(mock_dpotrf, f64, 85, DPOTRF_CALLED);
    mock_complex_potrf!(mock_cpotrf, 76, CPOTRF_CALLED);
    mock_complex_potrf!(mock_zpotrf, 76, ZPOTRF_CALLED);

    fn mock_library() -> Library {
        Library {
            _native: None,
            sgemm: mock_sgemm,
            cgemm: mock_cgemm,
            dgemm: unused_dgemm,
            zgemm: unused_zgemm,
            ddot: unused_ddot,
            dnrm2: unused_dnrm2,
            sgesv: mock_sgesv,
            cgesv: mock_cgesv,
            dgesv: unused_dgesv,
            zgesv: unused_zgesv,
            sgels: mock_sgels,
            cgels: mock_cgels,
            dgels: unused_dgels,
            zgels: unused_zgels,
            sgetrf: mock_sgetrf,
            dgetrf: mock_dgetrf,
            cgetrf: mock_cgetrf,
            zgetrf: mock_zgetrf,
            sgeqrf: mock_sgeqrf,
            dgeqrf: mock_dgeqrf,
            cgeqrf: mock_cgeqrf,
            zgeqrf: mock_zgeqrf,
            sorgqr: mock_sorgqr,
            dorgqr: mock_dorgqr,
            cungqr: mock_cungqr,
            zungqr: mock_zungqr,
            sgeqp3: mock_sgeqp3,
            dgeqp3: mock_dgeqp3,
            cgeqp3: mock_cgeqp3,
            zgeqp3: mock_zgeqp3,
            spotrf: mock_spotrf,
            dpotrf: mock_dpotrf,
            cpotrf: mock_cpotrf,
            zpotrf: mock_zpotrf,
            sgesdd: mock_sgesdd,
            dgesdd: mock_dgesdd,
            cgesdd: mock_cgesdd,
            zgesdd: mock_zgesdd,
            sgeev: mock_sgeev,
            dgeev: mock_dgeev,
            cgeev: mock_cgeev,
            zgeev: mock_zgeev,
            cgees: mock_cgees,
            zgees: mock_zgees,
        }
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn spectral_wrappers_call_all_four_precisions_with_explicit_complex_pairs() {
        let _guard = MOCK_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        SPECTRAL_FLAGS.store(0, Ordering::SeqCst);
        let library = mock_library();
        let dimensions = Lp64SpectralDimensions {
            m: 1,
            n: 1,
            k: 1,
            lda: 1,
        };
        let mut s = [0.0_f32];
        let mut d = [0.0_f64];
        let mut c = [[1.0_f32, 2.0_f32]];
        let mut z = [[1.0_f64, 2.0_f64]];
        assert_eq!(
            library
                .sgesdd(
                    dimensions,
                    SvdVectors::Thin,
                    &mut [2.0],
                    &mut s,
                    &mut [0.0],
                    1,
                    &mut [0.0],
                    1
                )
                .unwrap(),
            0
        );
        assert_eq!(
            library
                .dgesdd(
                    dimensions,
                    SvdVectors::Full,
                    &mut [2.0],
                    &mut d,
                    &mut [0.0],
                    1,
                    &mut [0.0],
                    1,
                )
                .unwrap(),
            0
        );
        assert_eq!(
            library
                .dgesdd(
                    dimensions,
                    SvdVectors::Thin,
                    &mut [2.0],
                    &mut d,
                    &mut [0.0],
                    1,
                    &mut [0.0],
                    1
                )
                .unwrap(),
            0
        );
        assert_eq!(
            library
                .cgesdd(
                    dimensions,
                    SvdVectors::Thin,
                    &mut c,
                    &mut s,
                    &mut [[0.0, 0.0]],
                    1,
                    &mut [[0.0, 0.0]],
                    1
                )
                .unwrap(),
            0
        );
        assert_eq!(
            library
                .zgesdd(
                    dimensions,
                    SvdVectors::Thin,
                    &mut z,
                    &mut d,
                    &mut [[0.0, 0.0]],
                    1,
                    &mut [[0.0, 0.0]],
                    1
                )
                .unwrap(),
            0
        );
        assert_eq!(
            library
                .sgesdd(
                    dimensions,
                    SvdVectors::None,
                    &mut [2.0],
                    &mut s,
                    &mut [],
                    1,
                    &mut [],
                    1,
                )
                .unwrap(),
            0
        );

        let mut wr32 = [0.0_f32];
        let mut wi32 = [0.0_f32];
        let mut wr64 = [0.0_f64];
        let mut wi64 = [0.0_f64];
        assert_eq!(
            library
                .sgeev(
                    dimensions,
                    true,
                    true,
                    &mut [2.0],
                    &mut wr32,
                    &mut wi32,
                    &mut [0.0],
                    &mut [0.0]
                )
                .unwrap(),
            0
        );
        assert_eq!(
            library
                .dgeev(
                    dimensions,
                    true,
                    true,
                    &mut [2.0],
                    &mut wr64,
                    &mut wi64,
                    &mut [0.0],
                    &mut [0.0]
                )
                .unwrap(),
            0
        );
        assert_eq!(
            library
                .cgeev(
                    dimensions,
                    true,
                    true,
                    &mut c,
                    &mut [[0.0, 0.0]],
                    &mut [[0.0, 0.0]],
                    &mut [[0.0, 0.0]]
                )
                .unwrap(),
            0
        );
        assert_eq!(
            library
                .zgeev(
                    dimensions,
                    true,
                    true,
                    &mut z,
                    &mut [[0.0, 0.0]],
                    &mut [[0.0, 0.0]],
                    &mut [[0.0, 0.0]]
                )
                .unwrap(),
            0
        );
        assert_eq!(
            library
                .sgeev(
                    dimensions,
                    false,
                    false,
                    &mut [2.0],
                    &mut wr32,
                    &mut wi32,
                    &mut [],
                    &mut [],
                )
                .unwrap(),
            0
        );
        assert_eq!(
            library
                .cgees(dimensions, &mut c, &mut [[0.0, 0.0]], &mut [[0.0, 0.0]])
                .unwrap(),
            (0, 0)
        );
        assert_eq!(
            library
                .zgees(dimensions, &mut z, &mut [[0.0, 0.0]], &mut [[0.0, 0.0]])
                .unwrap(),
            (0, 0)
        );
        assert_eq!(SPECTRAL_FLAGS.load(Ordering::SeqCst), ALL_SPECTRAL_CALLS);
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn native_single_wrappers_call_mock_abi_with_explicit_adjacent_pairs() {
        let _guard = MOCK_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        MOCK_FLAGS.store(0, Ordering::SeqCst);
        let library = mock_library();

        let gemm = Lp64GemmDimensions {
            m: 1,
            n: 1,
            k: 1,
            lda: 1,
            ldb: 1,
            ldc: 1,
        };
        let mut real_output = [6.0_f32];
        library
            .sgemm(
                gemm,
                MatrixTranspose::Transpose,
                MatrixTranspose::None,
                2.0,
                &[4.0],
                &[5.0],
                3.0,
                &mut real_output,
            )
            .unwrap();
        assert_eq!(real_output, [46.0]);

        let mut complex_output = [[5.0_f32, 6.0_f32]];
        library
            .cgemm(
                gemm,
                MatrixTranspose::ConjugateTranspose,
                MatrixTranspose::Transpose,
                [2.0, -1.0],
                &[[1.0, 2.0]],
                &[[3.0, 4.0]],
                [-0.5, 0.25],
                &mut complex_output,
            )
            .unwrap();
        assert_eq!(complex_output, [[7.0, 8.0]]);

        let solve = Lp64SolveDimensions {
            n: 1,
            nrhs: 1,
            lda: 1,
            ldb: 1,
        };
        let mut real_coefficients = [2.0_f32];
        let mut real_pivots = [0];
        let mut real_rhs = [4.0_f32];
        assert_eq!(
            library
                .sgesv(
                    solve,
                    &mut real_coefficients,
                    &mut real_pivots,
                    &mut real_rhs,
                )
                .unwrap(),
            0
        );
        assert_eq!(real_pivots, [1]);
        assert_eq!(real_rhs, [2.0]);

        let mut complex_coefficients = [[1.0_f32, 2.0_f32]];
        let mut complex_pivots = [0];
        let mut complex_rhs = [[5.0_f32, 6.0_f32]];
        assert_eq!(
            library
                .cgesv(
                    solve,
                    &mut complex_coefficients,
                    &mut complex_pivots,
                    &mut complex_rhs,
                )
                .unwrap(),
            0
        );
        assert_eq!(complex_pivots, [1]);
        assert_eq!(complex_rhs, [[3.0, -4.0]]);

        let rectangular = Lp64RectangularSolveDimensions {
            m: 2,
            n: 1,
            nrhs: 1,
            lda: 2,
            ldb: 2,
        };
        let mut real_coefficients = [1.0_f32, 2.0];
        let mut real_rhs = [3.0_f32, 4.0];
        assert_eq!(
            library
                .sgels(rectangular, &mut real_coefficients, &mut real_rhs)
                .unwrap(),
            0
        );
        assert_eq!(real_rhs[0], 9.0);

        let mut complex_coefficients = [[1.0_f32, -1.0], [0.5, 2.0]];
        let mut complex_rhs = [[2.0_f32, 3.0], [4.0, 5.0]];
        assert_eq!(
            library
                .cgels(rectangular, &mut complex_coefficients, &mut complex_rhs,)
                .unwrap(),
            0
        );
        assert_eq!(complex_rhs[0], [-2.0, 4.0]);

        assert_eq!(MOCK_FLAGS.load(Ordering::SeqCst), ALL_SINGLE_CALLS);
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn decomposition_wrappers_call_every_precision_and_explicit_complex_pair_abi() {
        let _guard = MOCK_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        MOCK_FLAGS.store(0, Ordering::SeqCst);
        let library = mock_library();
        let dimensions = Lp64FactorDimensions {
            m: 1,
            n: 1,
            k: 1,
            lda: 1,
        };

        let mut s_matrix = [2.0_f32];
        let mut d_matrix = [2.0_f64];
        let mut c_matrix = [[2.0_f32, 3.0_f32]];
        let mut z_matrix = [[2.0_f64, 3.0_f64]];
        let mut pivot = [0_i32];
        assert_eq!(
            library
                .sgetrf(dimensions, &mut s_matrix, &mut pivot)
                .unwrap(),
            0
        );
        pivot[0] = 0;
        assert_eq!(
            library
                .dgetrf(dimensions, &mut d_matrix, &mut pivot)
                .unwrap(),
            0
        );
        pivot[0] = 0;
        assert_eq!(
            library
                .cgetrf(dimensions, &mut c_matrix, &mut pivot)
                .unwrap(),
            0
        );
        pivot[0] = 0;
        assert_eq!(
            library
                .zgetrf(dimensions, &mut z_matrix, &mut pivot)
                .unwrap(),
            0
        );

        let mut s_tau = [0.0_f32];
        let mut d_tau = [0.0_f64];
        let mut c_tau = [[0.0_f32, 0.0_f32]];
        let mut z_tau = [[0.0_f64, 0.0_f64]];
        assert_eq!(
            library.sgeqrf(dimensions, &mut [2.0], &mut s_tau).unwrap(),
            0
        );
        assert_eq!(
            library.dgeqrf(dimensions, &mut [2.0], &mut d_tau).unwrap(),
            0
        );
        assert_eq!(
            library
                .cgeqrf(dimensions, &mut [[2.0, 3.0]], &mut c_tau)
                .unwrap(),
            0
        );
        assert_eq!(
            library
                .zgeqrf(dimensions, &mut [[2.0, 3.0]], &mut z_tau)
                .unwrap(),
            0
        );

        let mut s_q = [0.0_f32];
        let mut d_q = [0.0_f64];
        let mut c_q = [[0.0_f32, 0.0_f32]];
        let mut z_q = [[0.0_f64, 0.0_f64]];
        assert_eq!(library.sorgqr(dimensions, 1, &mut s_q, &s_tau).unwrap(), 0);
        assert_eq!(library.dorgqr(dimensions, 1, &mut d_q, &d_tau).unwrap(), 0);
        assert_eq!(library.cungqr(dimensions, 1, &mut c_q, &c_tau).unwrap(), 0);
        assert_eq!(library.zungqr(dimensions, 1, &mut z_q, &z_tau).unwrap(), 0);
        assert_eq!(s_q, [7.0]);
        assert_eq!(c_q, [[7.0, 8.0]]);

        let mut s_pivots = [0_i32];
        let mut d_pivots = [0_i32];
        let mut c_pivots = [0_i32];
        let mut z_pivots = [0_i32];
        assert_eq!(
            library
                .sgeqp3(dimensions, &mut [2.0], &mut s_pivots, &mut [0.0])
                .unwrap(),
            0
        );
        assert_eq!(
            library
                .dgeqp3(dimensions, &mut [2.0], &mut d_pivots, &mut [0.0])
                .unwrap(),
            0
        );
        assert_eq!(
            library
                .cgeqp3(
                    dimensions,
                    &mut [[2.0, 3.0]],
                    &mut c_pivots,
                    &mut [[0.0, 0.0]],
                )
                .unwrap(),
            0
        );
        assert_eq!(
            library
                .zgeqp3(
                    dimensions,
                    &mut [[2.0, 3.0]],
                    &mut z_pivots,
                    &mut [[0.0, 0.0]],
                )
                .unwrap(),
            0
        );

        let cholesky = Lp64CholeskyDimensions { n: 1, lda: 1 };
        assert_eq!(
            library
                .spotrf(cholesky, CholeskyTriangle::Upper, &mut [4.0_f32])
                .unwrap(),
            0
        );
        assert_eq!(
            library
                .dpotrf(cholesky, CholeskyTriangle::Upper, &mut [4.0_f64])
                .unwrap(),
            0
        );
        assert_eq!(
            library
                .cpotrf(cholesky, CholeskyTriangle::Lower, &mut [[4.0_f32, 0.0]],)
                .unwrap(),
            0
        );
        assert_eq!(
            library
                .zpotrf(cholesky, CholeskyTriangle::Lower, &mut [[4.0_f64, 0.0]],)
                .unwrap(),
            0
        );

        assert_eq!(MOCK_FLAGS.load(Ordering::SeqCst), ALL_DECOMPOSITION_CALLS);
    }

    #[test]
    fn decomposition_buffer_validation_checks_factor_q_and_cholesky_shapes() {
        let dimensions = Lp64FactorDimensions {
            m: 3,
            n: 2,
            k: 2,
            lda: 3,
        };
        assert!(super::validate_factor_buffers(dimensions, 6).is_ok());
        assert!(super::validate_qr_buffers(dimensions, 6, 2).is_ok());
        assert!(super::validate_generate_q_buffers(dimensions, 2, 6, 2).is_ok());
        assert!(matches!(
            super::validate_generate_q_buffers(dimensions, 1, 3, 2),
            Err(CallError::DimensionOutOfRange {
                parameter: "Q columns",
                value: 1,
                minimum: 2,
                maximum: 3,
            })
        ));
        assert!(matches!(
            super::validate_qr_buffers(dimensions, 6, 1),
            Err(CallError::BufferLength {
                operand: "QR reflector buffer",
                expected: 2,
                actual: 1,
            })
        ));

        let cholesky = Lp64CholeskyDimensions { n: 2, lda: 2 };
        assert!(super::validate_cholesky_buffer(cholesky, 4).is_ok());
        assert!(matches!(
            super::validate_cholesky_buffer(cholesky, 3),
            Err(CallError::BufferLength {
                operand: "Cholesky factorization matrix",
                expected: 4,
                actual: 3,
            })
        ));
    }

    #[test]
    fn solve_buffer_validation_checks_every_slice_and_leading_dimension() {
        let valid = Lp64SolveDimensions {
            n: 2,
            nrhs: 1,
            lda: 2,
            ldb: 2,
        };
        assert!(validate_solve_buffers(valid, 4, 2, 2).is_ok());
        assert!(matches!(
            validate_solve_buffers(valid, 3, 2, 2),
            Err(CallError::BufferLength {
                operand: "linear solve coefficient matrix",
                expected: 4,
                actual: 3,
            })
        ));
        assert!(matches!(
            validate_solve_buffers(valid, 4, 1, 2),
            Err(CallError::BufferLength {
                operand: "linear solve pivot buffer",
                expected: 2,
                actual: 1,
            })
        ));
        assert!(matches!(
            validate_solve_buffers(valid, 4, 2, 1),
            Err(CallError::BufferLength {
                operand: "linear solve right-hand side",
                expected: 2,
                actual: 1,
            })
        ));

        let wrong_lda = Lp64SolveDimensions { lda: 1, ..valid };
        assert!(matches!(
            validate_solve_buffers(wrong_lda, 4, 2, 2),
            Err(CallError::LeadingDimension {
                parameter: "lda",
                expected: 2,
                actual: 1,
            })
        ));
    }

    #[test]
    fn solve_buffer_validation_allows_zero_right_hand_sides() {
        let dimensions = Lp64SolveDimensions {
            n: 2,
            nrhs: 0,
            lda: 2,
            ldb: 2,
        };
        assert!(validate_solve_buffers(dimensions, 4, 2, 0).is_ok());
    }

    #[test]
    fn rectangular_solve_buffer_validation_requires_padded_rhs() {
        let dimensions = Lp64RectangularSolveDimensions {
            m: 2,
            n: 3,
            nrhs: 2,
            lda: 2,
            ldb: 3,
        };
        assert!(validate_rectangular_solve_buffers(dimensions, 6, 6).is_ok());
        assert!(matches!(
            validate_rectangular_solve_buffers(dimensions, 6, 4),
            Err(CallError::BufferLength {
                operand: "rectangular solve right-hand-side workspace",
                expected: 6,
                actual: 4,
            })
        ));
        let wrong_ldb = Lp64RectangularSolveDimensions {
            ldb: 2,
            ..dimensions
        };
        assert!(matches!(
            validate_rectangular_solve_buffers(wrong_ldb, 6, 6),
            Err(CallError::LeadingDimension {
                parameter: "ldb",
                expected: 3,
                actual: 2,
            })
        ));
    }
}
