//! Explicit-path Windows `OpenBLAS` provider with an isolated native boundary.
//!
//! Deployment supplies a trusted LP64 `OpenBLAS` DLL by absolute path. Any
//! non-system dependencies must be deployed beside that DLL; this crate does
//! not search or modify `PATH` and does not use the current directory.

use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

use openmat_array::{Complex32, Complex64, DenseArray, Shape};
use openmat_linalg::{
    CholeskyRequest, CholeskyResult, CholeskyTriangle, EigRequest, EigResult, FactorDimensions,
    FactorRequest, GemmRequest, LinalgError, LinalgProvider, Lp64CholeskyDimensions,
    Lp64FactorDimensions, Lp64GemmDimensions, Lp64RectangularSolveDimensions, Lp64SolveDimensions,
    Lp64SpectralDimensions, LuResult, ProviderIntegerAbi, QrRequest, QrResult, QrVectors,
    RectangularSolveDimensions, RectangularSolveRequest, SchurRequest, SchurResult, SolveRequest,
    SvdRequest, SvdResult, SvdVectors, SwapParity, checked_lp64_dimension, validate_cholesky,
    validate_eig, validate_factor, validate_gemm, validate_qr, validate_rectangular_solve,
    validate_schur, validate_solve, validate_svd,
};

#[cfg(windows)]
mod ffi;

const PROVIDER_NAME: &str = "openblas";

/// Provider crate version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// `OpenBLAS` integer interface detected from its runtime configuration string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenBlasIntegerAbi {
    /// Signed 32-bit BLAS dimensions (`blasint` is `int`).
    Lp64,
    /// Signed 64-bit BLAS dimensions (`USE64BITINT` or `INTERFACE64`).
    Ilp64,
}

impl fmt::Display for OpenBlasIntegerAbi {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lp64 => formatter.write_str("LP64"),
            Self::Ilp64 => formatter.write_str("ILP64"),
        }
    }
}

/// Runtime identity and ABI information copied from a loaded `OpenBLAS` DLL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenBlasInfo {
    path: PathBuf,
    config: String,
    version: String,
    core_name: String,
    integer_abi: OpenBlasIntegerAbi,
}

impl OpenBlasInfo {
    /// Returns the exact absolute path supplied by the caller.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the string from `openblas_get_config`.
    #[must_use]
    pub fn config(&self) -> &str {
        &self.config
    }

    /// Returns the `OpenBLAS` version parsed from the configuration string.
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Returns the string from `openblas_get_corename`.
    #[must_use]
    pub fn core_name(&self) -> &str {
        &self.core_name
    }

    /// Returns the detected integer interface.
    #[must_use]
    pub const fn integer_abi(&self) -> OpenBlasIntegerAbi {
        self.integer_abi
    }
}

/// Failures while locating, identifying, or binding an `OpenBLAS` DLL.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum OpenBlasError {
    /// This provider is only implemented on Windows.
    UnsupportedPlatform,
    /// Loading requires a fully qualified path and never searches `PATH` or
    /// the current directory for the requested DLL.
    PathNotAbsolute {
        /// Rejected caller input.
        path: PathBuf,
    },
    /// Windows could not load the explicitly named DLL or one of its
    /// dependencies.
    LibraryLoad {
        /// Explicit DLL path.
        path: PathBuf,
        /// Native loader diagnostic.
        detail: String,
    },
    /// A required `OpenBLAS` export was absent.
    MissingSymbol {
        /// Explicit DLL path.
        path: PathBuf,
        /// Required undecorated export name.
        symbol: &'static str,
        /// Native loader diagnostic.
        detail: String,
    },
    /// An `OpenBLAS` string-returning export returned a null pointer.
    NullString {
        /// Export that returned null.
        symbol: &'static str,
    },
    /// An `OpenBLAS` string-returning export did not return UTF-8.
    InvalidStringEncoding {
        /// Export that returned invalid text.
        symbol: &'static str,
    },
    /// `openblas_get_config` did not identify a parseable `OpenBLAS` version.
    MalformedConfig {
        /// Configuration text returned by the DLL.
        config: String,
        /// Stable parser diagnostic.
        reason: &'static str,
    },
    /// The DLL uses an integer ABI this provider cannot call.
    IncompatibleIntegerAbi {
        /// Configuration text returned by the DLL.
        config: String,
        /// Detected ABI.
        detected: OpenBlasIntegerAbi,
    },
}

impl fmt::Display for OpenBlasError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedPlatform => {
                formatter.write_str("the OpenBLAS provider is only available on Windows")
            }
            Self::PathNotAbsolute { path } => write!(
                formatter,
                "OpenBLAS DLL path must be absolute: {}",
                path.display()
            ),
            Self::LibraryLoad { path, detail } => write!(
                formatter,
                "failed to load OpenBLAS DLL {}: {detail}",
                path.display()
            ),
            Self::MissingSymbol {
                path,
                symbol,
                detail,
            } => write!(
                formatter,
                "OpenBLAS DLL {} is missing symbol {symbol}: {detail}",
                path.display()
            ),
            Self::NullString { symbol } => {
                write!(formatter, "OpenBLAS symbol {symbol} returned null")
            }
            Self::InvalidStringEncoding { symbol } => {
                write!(formatter, "OpenBLAS symbol {symbol} returned invalid UTF-8")
            }
            Self::MalformedConfig { config, reason } => {
                write!(formatter, "malformed OpenBLAS config {config:?}: {reason}")
            }
            Self::IncompatibleIntegerAbi { config, detected } => write!(
                formatter,
                "OpenBLAS config {config:?} selects {detected}, but this provider requires LP64"
            ),
        }
    }
}

impl Error for OpenBlasError {}

/// Dynamically loaded LP64 `OpenBLAS` implementation.
///
/// Construction requires a caller-supplied absolute DLL path. On Windows, the
/// DLL directory is searched only for that DLL's dependencies, with System32
/// as the sole fallback. The process `PATH` and current directory are not
/// modified or searched. The native module remains loaded for this value's
/// entire lifetime.
pub struct OpenBlasProvider {
    #[cfg(windows)]
    library: ffi::Library,
    info: OpenBlasInfo,
}

impl fmt::Debug for OpenBlasProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OpenBlasProvider")
            .field("info", &self.info)
            .finish_non_exhaustive()
    }
}

impl OpenBlasProvider {
    /// Loads and validates an `OpenBLAS` DLL from an explicit absolute path.
    ///
    /// The DLL must export the `OpenBLAS` identity functions plus every CBLAS
    /// and LAPACKE operation used by [`LinalgProvider`]. ILP64 builds are
    /// rejected before any size-dependent compute symbol is bound or invoked.
    ///
    /// # Errors
    ///
    /// Returns a typed path, loader, symbol, configuration, or ABI error.
    #[cfg(windows)]
    pub fn load(path: impl AsRef<Path>) -> Result<Self, OpenBlasError> {
        let path = path.as_ref();
        if !path.is_absolute() {
            return Err(OpenBlasError::PathNotAbsolute {
                path: path.to_path_buf(),
            });
        }

        let module = ffi::Module::load(path).map_err(|error| error.with_path(path))?;
        let config = module.config().map_err(|error| error.with_path(path))?;
        let parsed = parse_config(&config)?;
        require_lp64(&config, parsed.integer_abi)?;
        let core_name = module.core_name().map_err(|error| error.with_path(path))?;
        let library = module
            .bind_compute_symbols()
            .map_err(|error| error.with_path(path))?;

        Ok(Self {
            library,
            info: OpenBlasInfo {
                path: path.to_path_buf(),
                config,
                version: parsed.version,
                core_name,
                integer_abi: parsed.integer_abi,
            },
        })
    }

    /// Reports that the Windows-only provider is unavailable on other hosts.
    ///
    /// # Errors
    ///
    /// Always returns [`OpenBlasError::UnsupportedPlatform`].
    #[cfg(not(windows))]
    pub fn load(_path: impl AsRef<Path>) -> Result<Self, OpenBlasError> {
        Err(OpenBlasError::UnsupportedPlatform)
    }

    /// Returns copied runtime identity and ABI information.
    #[must_use]
    pub const fn info(&self) -> &OpenBlasInfo {
        &self.info
    }
}

impl LinalgProvider for OpenBlasProvider {
    fn name(&self) -> &'static str {
        PROVIDER_NAME
    }

    fn integer_abi(&self) -> ProviderIntegerAbi {
        ProviderIntegerAbi::Lp64
    }

    fn gemm_f64(
        &self,
        request: GemmRequest<'_, f64>,
        output: &mut DenseArray<f64>,
    ) -> Result<(), LinalgError> {
        let dimensions = validate_gemm(&request, output)?;
        let lp64 = Lp64GemmDimensions::try_from(&dimensions)?;
        if dimensions.m() == 0 || dimensions.n() == 0 {
            return Ok(());
        }
        if dimensions.k() == 0 {
            for value in output.as_mut_slice() {
                *value = request
                    .options
                    .alpha
                    .mul_add(0.0, request.options.beta * *value);
            }
            return Ok(());
        }

        #[cfg(windows)]
        return self
            .library
            .dgemm(
                lp64,
                request.options.left_transpose,
                request.options.right_transpose,
                request.options.alpha,
                request.left.as_slice(),
                request.right.as_slice(),
                request.options.beta,
                output.as_mut_slice(),
            )
            .map_err(|error| provider_failure("f64 GEMM", &error));

        #[cfg(not(windows))]
        {
            let _ = lp64;
            unreachable_provider()
        }
    }

    fn gemm_f32(
        &self,
        request: GemmRequest<'_, f32>,
        output: &mut DenseArray<f32>,
    ) -> Result<(), LinalgError> {
        let dimensions = validate_gemm(&request, output)?;
        let lp64 = Lp64GemmDimensions::try_from(&dimensions)?;
        if dimensions.m() == 0 || dimensions.n() == 0 {
            return Ok(());
        }
        if dimensions.k() == 0 {
            for value in output.as_mut_slice() {
                *value = request
                    .options
                    .alpha
                    .mul_add(0.0, request.options.beta * *value);
            }
            return Ok(());
        }

        #[cfg(windows)]
        return self
            .library
            .sgemm(
                lp64,
                request.options.left_transpose,
                request.options.right_transpose,
                request.options.alpha,
                request.left.as_slice(),
                request.right.as_slice(),
                request.options.beta,
                output.as_mut_slice(),
            )
            .map_err(|error| provider_failure("f32 GEMM", &error));

        #[cfg(not(windows))]
        {
            let _ = lp64;
            unreachable_provider()
        }
    }

    fn gemm_complex64(
        &self,
        request: GemmRequest<'_, Complex64>,
        output: &mut DenseArray<Complex64>,
    ) -> Result<(), LinalgError> {
        let dimensions = validate_gemm(&request, output)?;
        let lp64 = Lp64GemmDimensions::try_from(&dimensions)?;
        if dimensions.m() == 0 || dimensions.n() == 0 {
            return Ok(());
        }
        if dimensions.k() == 0 {
            for value in output.as_mut_slice() {
                *value = request.options.alpha * Complex64::ZERO + request.options.beta * *value;
            }
            return Ok(());
        }

        #[cfg(windows)]
        {
            // `openmat_array::Complex64` has no external ABI. Every value,
            // including alpha/beta and the output, crosses FFI as an explicit
            // pair of adjacent `f64` values instead.
            let left = try_pack_complex("complex64 GEMM left copy", request.left.as_slice())?;
            let right = try_pack_complex("complex64 GEMM right copy", request.right.as_slice())?;
            let mut packed_output =
                try_pack_complex("complex64 GEMM output copy", output.as_slice())?;
            self.library
                .zgemm(
                    lp64,
                    request.options.left_transpose,
                    request.options.right_transpose,
                    pack_scalar(request.options.alpha),
                    &left,
                    &right,
                    pack_scalar(request.options.beta),
                    &mut packed_output,
                )
                .map_err(|error| provider_failure("complex64 GEMM", &error))?;
            unpack_complex(&packed_output, output.as_mut_slice());
            Ok(())
        }

        #[cfg(not(windows))]
        {
            let _ = lp64;
            unreachable_provider()
        }
    }

    fn gemm_complex32(
        &self,
        request: GemmRequest<'_, Complex32>,
        output: &mut DenseArray<Complex32>,
    ) -> Result<(), LinalgError> {
        let dimensions = validate_gemm(&request, output)?;
        let lp64 = Lp64GemmDimensions::try_from(&dimensions)?;
        if dimensions.m() == 0 || dimensions.n() == 0 {
            return Ok(());
        }
        if dimensions.k() == 0 {
            for value in output.as_mut_slice() {
                *value = complex32_add(
                    complex32_multiply(request.options.alpha, Complex32::ZERO),
                    complex32_multiply(request.options.beta, *value),
                );
            }
            return Ok(());
        }

        #[cfg(windows)]
        {
            let left = try_pack_complex32("complex32 GEMM left copy", request.left.as_slice())?;
            let right = try_pack_complex32("complex32 GEMM right copy", request.right.as_slice())?;
            let mut packed_output =
                try_pack_complex32("complex32 GEMM output copy", output.as_slice())?;
            self.library
                .cgemm(
                    lp64,
                    request.options.left_transpose,
                    request.options.right_transpose,
                    pack_scalar32(request.options.alpha),
                    &left,
                    &right,
                    pack_scalar32(request.options.beta),
                    &mut packed_output,
                )
                .map_err(|error| provider_failure("complex32 GEMM", &error))?;
            unpack_complex32(&packed_output, output.as_mut_slice());
            Ok(())
        }

        #[cfg(not(windows))]
        {
            let _ = lp64;
            unreachable_provider()
        }
    }

    fn dot_f64(&self, left: &DenseArray<f64>, right: &DenseArray<f64>) -> Result<f64, LinalgError> {
        if left.numel() != right.numel() {
            return Err(LinalgError::DimensionMismatch {
                operation: "real dot product element counts",
                left: left.numel(),
                right: right.numel(),
            });
        }
        let n = checked_lp64_dimension("n", left.numel())?;
        if n == 0 {
            return Ok(0.0);
        }

        #[cfg(windows)]
        return self
            .library
            .ddot(n, left.as_slice(), right.as_slice())
            .map_err(|error| provider_failure("f64 dot", &error));

        #[cfg(not(windows))]
        unreachable_provider()
    }

    fn norm2_f64(&self, input: &DenseArray<f64>) -> Result<f64, LinalgError> {
        let n = checked_lp64_dimension("n", input.numel())?;
        if n == 0 {
            return Ok(0.0);
        }
        let mut has_infinite = false;
        for &value in input.as_slice() {
            if value.is_nan() {
                return Ok(f64::NAN);
            }
            has_infinite |= value.is_infinite();
        }
        if has_infinite {
            return Ok(f64::INFINITY);
        }

        #[cfg(windows)]
        return self
            .library
            .dnrm2(n, input.as_slice())
            .map_err(|error| provider_failure("f64 norm2", &error));

        #[cfg(not(windows))]
        unreachable_provider()
    }

    fn solve_f64(&self, request: SolveRequest<'_, f64>) -> Result<DenseArray<f64>, LinalgError> {
        let dimensions = validate_solve(&request)?;
        let lp64 = Lp64SolveDimensions::try_from(&dimensions)?;
        request.check_cancellation()?;
        if dimensions.order() == 0 {
            return independent_array_copy("f64 solve empty result", request.right_hand_side);
        }

        #[cfg(windows)]
        {
            let mut coefficients = try_copy_buffer(
                "f64 solve coefficient copy",
                request.coefficients.as_slice(),
            )?;
            let mut solution = try_copy_buffer(
                "f64 solve right-hand-side copy",
                request.right_hand_side.as_slice(),
            )?;
            let mut pivots = try_allocate_pivots(lp64.n)?;
            request.check_cancellation()?;
            let info = self
                .library
                .dgesv(lp64, &mut coefficients, &mut pivots, &mut solution)
                .map_err(|error| provider_failure("f64 general solve", &error))?;
            request.check_cancellation()?;
            check_lapack_info("f64 general solve", info)?;
            DenseArray::from_vec(request.right_hand_side.shape().clone(), solution)
                .map_err(Into::into)
        }

        #[cfg(not(windows))]
        {
            let _ = lp64;
            unreachable_provider()
        }
    }

    fn solve_f32(&self, request: SolveRequest<'_, f32>) -> Result<DenseArray<f32>, LinalgError> {
        let dimensions = validate_solve(&request)?;
        let lp64 = Lp64SolveDimensions::try_from(&dimensions)?;
        request.check_cancellation()?;
        if dimensions.order() == 0 {
            return independent_array_copy("f32 solve empty result", request.right_hand_side);
        }

        #[cfg(windows)]
        {
            let mut coefficients = try_copy_buffer(
                "f32 solve coefficient copy",
                request.coefficients.as_slice(),
            )?;
            let mut solution = try_copy_buffer(
                "f32 solve right-hand-side copy",
                request.right_hand_side.as_slice(),
            )?;
            let mut pivots = try_allocate_pivots(lp64.n)?;
            request.check_cancellation()?;
            let info = self
                .library
                .sgesv(lp64, &mut coefficients, &mut pivots, &mut solution)
                .map_err(|error| provider_failure("f32 general solve", &error))?;
            request.check_cancellation()?;
            check_lapack_info("f32 general solve", info)?;
            DenseArray::from_vec(request.right_hand_side.shape().clone(), solution)
                .map_err(Into::into)
        }

        #[cfg(not(windows))]
        {
            let _ = lp64;
            unreachable_provider()
        }
    }

    fn solve_complex64(
        &self,
        request: SolveRequest<'_, Complex64>,
    ) -> Result<DenseArray<Complex64>, LinalgError> {
        let dimensions = validate_solve(&request)?;
        let lp64 = Lp64SolveDimensions::try_from(&dimensions)?;
        request.check_cancellation()?;
        if dimensions.order() == 0 {
            return independent_array_copy("complex64 solve empty result", request.right_hand_side);
        }

        #[cfg(windows)]
        {
            let mut coefficients = try_pack_complex(
                "complex64 solve coefficient copy",
                request.coefficients.as_slice(),
            )?;
            let mut solution = try_pack_complex(
                "complex64 solve right-hand-side copy",
                request.right_hand_side.as_slice(),
            )?;
            let mut pivots = try_allocate_pivots(lp64.n)?;
            request.check_cancellation()?;
            let info = self
                .library
                .zgesv(lp64, &mut coefficients, &mut pivots, &mut solution)
                .map_err(|error| provider_failure("complex64 general solve", &error))?;
            request.check_cancellation()?;
            check_lapack_info("complex64 general solve", info)?;
            let solution = try_unpack_complex("complex64 solve result", solution.as_slice())?;
            DenseArray::from_vec(request.right_hand_side.shape().clone(), solution)
                .map_err(Into::into)
        }

        #[cfg(not(windows))]
        {
            let _ = lp64;
            unreachable_provider()
        }
    }

    fn solve_complex32(
        &self,
        request: SolveRequest<'_, Complex32>,
    ) -> Result<DenseArray<Complex32>, LinalgError> {
        let dimensions = validate_solve(&request)?;
        let lp64 = Lp64SolveDimensions::try_from(&dimensions)?;
        request.check_cancellation()?;
        if dimensions.order() == 0 {
            return independent_array_copy("complex32 solve empty result", request.right_hand_side);
        }

        #[cfg(windows)]
        {
            let mut coefficients = try_pack_complex32(
                "complex32 solve coefficient copy",
                request.coefficients.as_slice(),
            )?;
            let mut solution = try_pack_complex32(
                "complex32 solve right-hand-side copy",
                request.right_hand_side.as_slice(),
            )?;
            let mut pivots = try_allocate_pivots(lp64.n)?;
            request.check_cancellation()?;
            let info = self
                .library
                .cgesv(lp64, &mut coefficients, &mut pivots, &mut solution)
                .map_err(|error| provider_failure("complex32 general solve", &error))?;
            request.check_cancellation()?;
            check_lapack_info("complex32 general solve", info)?;
            let solution = try_unpack_complex32("complex32 solve result", solution.as_slice())?;
            DenseArray::from_vec(request.right_hand_side.shape().clone(), solution)
                .map_err(Into::into)
        }

        #[cfg(not(windows))]
        {
            let _ = lp64;
            unreachable_provider()
        }
    }

    fn solve_rectangular_f64(
        &self,
        request: RectangularSolveRequest<'_, f64>,
    ) -> Result<DenseArray<f64>, LinalgError> {
        let dimensions = validate_rectangular_solve(&request)?;
        let lp64 = Lp64RectangularSolveDimensions::try_from(&dimensions)?;
        request.check_cancellation()?;
        if dimensions.required_rank() == 0 {
            return rectangular_zero_result(
                "f64 rectangular solve empty-rank result",
                dimensions,
                0.0,
            );
        }

        #[cfg(windows)]
        {
            let mut coefficients = try_copy_buffer(
                "f64 rectangular solve coefficient copy",
                request.coefficients.as_slice(),
            )?;
            let mut workspace = try_padded_right_hand_side(
                "f64 rectangular solve right-hand-side workspace",
                dimensions,
                request.right_hand_side.as_slice(),
                0.0,
            )?;
            request.check_cancellation()?;
            let info = self
                .library
                .dgels(lp64, &mut coefficients, &mut workspace)
                .map_err(|error| provider_failure("f64 rectangular solve", &error))?;
            request.check_cancellation()?;
            check_gels_info("f64 rectangular solve", info, dimensions.required_rank())?;
            let solution = try_extract_rectangular_solution(
                "f64 rectangular solve result",
                dimensions,
                &workspace,
            )?;
            DenseArray::from_vec(rectangular_result_shape(dimensions)?, solution)
                .map_err(Into::into)
        }

        #[cfg(not(windows))]
        {
            let _ = lp64;
            unreachable_provider()
        }
    }

    fn solve_rectangular_f32(
        &self,
        request: RectangularSolveRequest<'_, f32>,
    ) -> Result<DenseArray<f32>, LinalgError> {
        let dimensions = validate_rectangular_solve(&request)?;
        let lp64 = Lp64RectangularSolveDimensions::try_from(&dimensions)?;
        request.check_cancellation()?;
        if dimensions.required_rank() == 0 {
            return rectangular_zero_result(
                "f32 rectangular solve empty-rank result",
                dimensions,
                0.0_f32,
            );
        }

        #[cfg(windows)]
        {
            let mut coefficients = try_copy_buffer(
                "f32 rectangular solve coefficient copy",
                request.coefficients.as_slice(),
            )?;
            let mut workspace = try_padded_right_hand_side(
                "f32 rectangular solve right-hand-side workspace",
                dimensions,
                request.right_hand_side.as_slice(),
                0.0_f32,
            )?;
            request.check_cancellation()?;
            let info = self
                .library
                .sgels(lp64, &mut coefficients, &mut workspace)
                .map_err(|error| provider_failure("f32 rectangular solve", &error))?;
            request.check_cancellation()?;
            check_gels_info("f32 rectangular solve", info, dimensions.required_rank())?;
            let solution = try_extract_rectangular_solution(
                "f32 rectangular solve result",
                dimensions,
                &workspace,
            )?;
            DenseArray::from_vec(rectangular_result_shape(dimensions)?, solution)
                .map_err(Into::into)
        }

        #[cfg(not(windows))]
        {
            let _ = lp64;
            unreachable_provider()
        }
    }

    fn solve_rectangular_complex64(
        &self,
        request: RectangularSolveRequest<'_, Complex64>,
    ) -> Result<DenseArray<Complex64>, LinalgError> {
        let dimensions = validate_rectangular_solve(&request)?;
        let lp64 = Lp64RectangularSolveDimensions::try_from(&dimensions)?;
        request.check_cancellation()?;
        if dimensions.required_rank() == 0 {
            return rectangular_zero_result(
                "complex64 rectangular solve empty-rank result",
                dimensions,
                Complex64::ZERO,
            );
        }

        #[cfg(windows)]
        {
            let mut coefficients = try_pack_complex(
                "complex64 rectangular solve coefficient copy",
                request.coefficients.as_slice(),
            )?;
            let packed_rhs = try_pack_complex(
                "complex64 rectangular solve right-hand-side copy",
                request.right_hand_side.as_slice(),
            )?;
            let mut workspace = try_padded_right_hand_side(
                "complex64 rectangular solve right-hand-side workspace",
                dimensions,
                &packed_rhs,
                [0.0, 0.0],
            )?;
            request.check_cancellation()?;
            let info = self
                .library
                .zgels(lp64, &mut coefficients, &mut workspace)
                .map_err(|error| provider_failure("complex64 rectangular solve", &error))?;
            request.check_cancellation()?;
            check_gels_info(
                "complex64 rectangular solve",
                info,
                dimensions.required_rank(),
            )?;
            let packed_solution = try_extract_rectangular_solution(
                "complex64 rectangular solve packed result",
                dimensions,
                &workspace,
            )?;
            let solution =
                try_unpack_complex("complex64 rectangular solve result", &packed_solution)?;
            DenseArray::from_vec(rectangular_result_shape(dimensions)?, solution)
                .map_err(Into::into)
        }

        #[cfg(not(windows))]
        {
            let _ = lp64;
            unreachable_provider()
        }
    }

    fn solve_rectangular_complex32(
        &self,
        request: RectangularSolveRequest<'_, Complex32>,
    ) -> Result<DenseArray<Complex32>, LinalgError> {
        let dimensions = validate_rectangular_solve(&request)?;
        let lp64 = Lp64RectangularSolveDimensions::try_from(&dimensions)?;
        request.check_cancellation()?;
        if dimensions.required_rank() == 0 {
            return rectangular_zero_result(
                "complex32 rectangular solve empty-rank result",
                dimensions,
                Complex32::ZERO,
            );
        }

        #[cfg(windows)]
        {
            let mut coefficients = try_pack_complex32(
                "complex32 rectangular solve coefficient copy",
                request.coefficients.as_slice(),
            )?;
            let packed_rhs = try_pack_complex32(
                "complex32 rectangular solve right-hand-side copy",
                request.right_hand_side.as_slice(),
            )?;
            let mut workspace = try_padded_right_hand_side(
                "complex32 rectangular solve right-hand-side workspace",
                dimensions,
                &packed_rhs,
                [0.0_f32, 0.0_f32],
            )?;
            request.check_cancellation()?;
            let info = self
                .library
                .cgels(lp64, &mut coefficients, &mut workspace)
                .map_err(|error| provider_failure("complex32 rectangular solve", &error))?;
            request.check_cancellation()?;
            check_gels_info(
                "complex32 rectangular solve",
                info,
                dimensions.required_rank(),
            )?;
            let packed_solution = try_extract_rectangular_solution(
                "complex32 rectangular solve packed result",
                dimensions,
                &workspace,
            )?;
            let solution =
                try_unpack_complex32("complex32 rectangular solve result", &packed_solution)?;
            DenseArray::from_vec(rectangular_result_shape(dimensions)?, solution)
                .map_err(Into::into)
        }

        #[cfg(not(windows))]
        {
            let _ = lp64;
            unreachable_provider()
        }
    }

    fn factor_lu_f32(&self, request: FactorRequest<'_, f32>) -> Result<LuResult<f32>, LinalgError> {
        #[cfg(windows)]
        return factor_lu_native(
            request,
            "f32 LU factorization",
            |dimensions, matrix, pivots| self.library.sgetrf(dimensions, matrix, pivots),
        );
        #[cfg(not(windows))]
        {
            let _ = request;
            unreachable_provider()
        }
    }

    fn factor_lu_f64(&self, request: FactorRequest<'_, f64>) -> Result<LuResult<f64>, LinalgError> {
        #[cfg(windows)]
        return factor_lu_native(
            request,
            "f64 LU factorization",
            |dimensions, matrix, pivots| self.library.dgetrf(dimensions, matrix, pivots),
        );
        #[cfg(not(windows))]
        {
            let _ = request;
            unreachable_provider()
        }
    }

    fn factor_lu_complex32(
        &self,
        request: FactorRequest<'_, Complex32>,
    ) -> Result<LuResult<Complex32>, LinalgError> {
        #[cfg(windows)]
        return factor_lu_complex32_native(request, |dimensions, matrix, pivots| {
            self.library.cgetrf(dimensions, matrix, pivots)
        });
        #[cfg(not(windows))]
        {
            let _ = request;
            unreachable_provider()
        }
    }

    fn factor_lu_complex64(
        &self,
        request: FactorRequest<'_, Complex64>,
    ) -> Result<LuResult<Complex64>, LinalgError> {
        #[cfg(windows)]
        return factor_lu_complex64_native(request, |dimensions, matrix, pivots| {
            self.library.zgetrf(dimensions, matrix, pivots)
        });
        #[cfg(not(windows))]
        {
            let _ = request;
            unreachable_provider()
        }
    }

    fn qr_f32(&self, request: QrRequest<'_, f32>) -> Result<QrResult<f32>, LinalgError> {
        #[cfg(windows)]
        return qr_native(
            request,
            "f32 QR factorization",
            0.0_f32,
            1.0_f32,
            f64::from(f32::EPSILON),
            |value| f64::from(value.abs()),
            |dimensions, matrix, pivots, tau, pivot| {
                if pivot {
                    self.library.sgeqp3(dimensions, matrix, pivots, tau)
                } else {
                    self.library.sgeqrf(dimensions, matrix, tau)
                }
            },
            |dimensions, q_columns, matrix, tau| {
                self.library.sorgqr(dimensions, q_columns, matrix, tau)
            },
        );
        #[cfg(not(windows))]
        {
            let _ = request;
            unreachable_provider()
        }
    }

    fn qr_f64(&self, request: QrRequest<'_, f64>) -> Result<QrResult<f64>, LinalgError> {
        #[cfg(windows)]
        return qr_native(
            request,
            "f64 QR factorization",
            0.0_f64,
            1.0_f64,
            f64::EPSILON,
            f64::abs,
            |dimensions, matrix, pivots, tau, pivot| {
                if pivot {
                    self.library.dgeqp3(dimensions, matrix, pivots, tau)
                } else {
                    self.library.dgeqrf(dimensions, matrix, tau)
                }
            },
            |dimensions, q_columns, matrix, tau| {
                self.library.dorgqr(dimensions, q_columns, matrix, tau)
            },
        );
        #[cfg(not(windows))]
        {
            let _ = request;
            unreachable_provider()
        }
    }

    fn qr_complex32(
        &self,
        request: QrRequest<'_, Complex32>,
    ) -> Result<QrResult<Complex32>, LinalgError> {
        #[cfg(windows)]
        return qr_complex32_native(
            request,
            |dimensions, matrix, pivots, tau, pivot| {
                if pivot {
                    self.library.cgeqp3(dimensions, matrix, pivots, tau)
                } else {
                    self.library.cgeqrf(dimensions, matrix, tau)
                }
            },
            |dimensions, q_columns, matrix, tau| {
                self.library.cungqr(dimensions, q_columns, matrix, tau)
            },
        );
        #[cfg(not(windows))]
        {
            let _ = request;
            unreachable_provider()
        }
    }

    fn qr_complex64(
        &self,
        request: QrRequest<'_, Complex64>,
    ) -> Result<QrResult<Complex64>, LinalgError> {
        #[cfg(windows)]
        return qr_complex64_native(
            request,
            |dimensions, matrix, pivots, tau, pivot| {
                if pivot {
                    self.library.zgeqp3(dimensions, matrix, pivots, tau)
                } else {
                    self.library.zgeqrf(dimensions, matrix, tau)
                }
            },
            |dimensions, q_columns, matrix, tau| {
                self.library.zungqr(dimensions, q_columns, matrix, tau)
            },
        );
        #[cfg(not(windows))]
        {
            let _ = request;
            unreachable_provider()
        }
    }

    fn cholesky_f32(
        &self,
        request: CholeskyRequest<'_, f32>,
    ) -> Result<CholeskyResult<f32>, LinalgError> {
        #[cfg(windows)]
        return cholesky_native(request, "f32 Cholesky factorization", 0.0_f32, |d, t, a| {
            self.library.spotrf(d, t, a)
        });
        #[cfg(not(windows))]
        {
            let _ = request;
            unreachable_provider()
        }
    }

    fn cholesky_f64(
        &self,
        request: CholeskyRequest<'_, f64>,
    ) -> Result<CholeskyResult<f64>, LinalgError> {
        #[cfg(windows)]
        return cholesky_native(request, "f64 Cholesky factorization", 0.0_f64, |d, t, a| {
            self.library.dpotrf(d, t, a)
        });
        #[cfg(not(windows))]
        {
            let _ = request;
            unreachable_provider()
        }
    }

    fn cholesky_complex32(
        &self,
        request: CholeskyRequest<'_, Complex32>,
    ) -> Result<CholeskyResult<Complex32>, LinalgError> {
        #[cfg(windows)]
        return cholesky_complex32_native(request, |d, t, a| self.library.cpotrf(d, t, a));
        #[cfg(not(windows))]
        {
            let _ = request;
            unreachable_provider()
        }
    }

    fn cholesky_complex64(
        &self,
        request: CholeskyRequest<'_, Complex64>,
    ) -> Result<CholeskyResult<Complex64>, LinalgError> {
        #[cfg(windows)]
        return cholesky_complex64_native(request, |d, t, a| self.library.zpotrf(d, t, a));
        #[cfg(not(windows))]
        {
            let _ = request;
            unreachable_provider()
        }
    }

    fn svd_f32(&self, request: SvdRequest<'_, f32>) -> Result<SvdResult<f32, f32>, LinalgError> {
        #[cfg(windows)]
        return svd_real_native(
            request,
            0.0_f32,
            1.0_f32,
            "f32 SVD",
            |d, v, a, s, u, ldu, vt, ldvt| self.library.sgesdd(d, v, a, s, u, ldu, vt, ldvt),
        );
        #[cfg(not(windows))]
        {
            let _ = request;
            unreachable_provider()
        }
    }

    fn svd_f64(&self, request: SvdRequest<'_, f64>) -> Result<SvdResult<f64, f64>, LinalgError> {
        #[cfg(windows)]
        return svd_real_native(
            request,
            0.0_f64,
            1.0_f64,
            "f64 SVD",
            |d, v, a, s, u, ldu, vt, ldvt| self.library.dgesdd(d, v, a, s, u, ldu, vt, ldvt),
        );
        #[cfg(not(windows))]
        {
            let _ = request;
            unreachable_provider()
        }
    }

    fn svd_complex32(
        &self,
        request: SvdRequest<'_, Complex32>,
    ) -> Result<SvdResult<Complex32, f32>, LinalgError> {
        #[cfg(windows)]
        return svd_complex32_native(request, |d, v, a, s, u, ldu, vt, ldvt| {
            self.library.cgesdd(d, v, a, s, u, ldu, vt, ldvt)
        });
        #[cfg(not(windows))]
        {
            let _ = request;
            unreachable_provider()
        }
    }

    fn svd_complex64(
        &self,
        request: SvdRequest<'_, Complex64>,
    ) -> Result<SvdResult<Complex64, f64>, LinalgError> {
        #[cfg(windows)]
        return svd_complex64_native(request, |d, v, a, s, u, ldu, vt, ldvt| {
            self.library.zgesdd(d, v, a, s, u, ldu, vt, ldvt)
        });
        #[cfg(not(windows))]
        {
            let _ = request;
            unreachable_provider()
        }
    }

    fn eig_f32(&self, request: EigRequest<'_, f32>) -> Result<EigResult<Complex32>, LinalgError> {
        #[cfg(windows)]
        return eig_real32_native(request, |d, l, r, a, wr, wi, vl, vr| {
            self.library.sgeev(d, l, r, a, wr, wi, vl, vr)
        });
        #[cfg(not(windows))]
        {
            let _ = request;
            unreachable_provider()
        }
    }

    fn eig_f64(&self, request: EigRequest<'_, f64>) -> Result<EigResult<Complex64>, LinalgError> {
        #[cfg(windows)]
        return eig_real64_native(request, |d, l, r, a, wr, wi, vl, vr| {
            self.library.dgeev(d, l, r, a, wr, wi, vl, vr)
        });
        #[cfg(not(windows))]
        {
            let _ = request;
            unreachable_provider()
        }
    }

    fn eig_complex32(
        &self,
        request: EigRequest<'_, Complex32>,
    ) -> Result<EigResult<Complex32>, LinalgError> {
        #[cfg(windows)]
        return eig_complex32_native(request, |d, l, r, a, w, vl, vr| {
            self.library.cgeev(d, l, r, a, w, vl, vr)
        });
        #[cfg(not(windows))]
        {
            let _ = request;
            unreachable_provider()
        }
    }

    fn eig_complex64(
        &self,
        request: EigRequest<'_, Complex64>,
    ) -> Result<EigResult<Complex64>, LinalgError> {
        #[cfg(windows)]
        return eig_complex64_native(request, |d, l, r, a, w, vl, vr| {
            self.library.zgeev(d, l, r, a, w, vl, vr)
        });
        #[cfg(not(windows))]
        {
            let _ = request;
            unreachable_provider()
        }
    }

    fn schur_complex32(
        &self,
        request: SchurRequest<'_, Complex32>,
    ) -> Result<SchurResult<Complex32>, LinalgError> {
        #[cfg(windows)]
        return schur_complex32_native(request, |dimensions, matrix, values, vectors| {
            self.library.cgees(dimensions, matrix, values, vectors)
        });
        #[cfg(not(windows))]
        {
            let _ = request;
            unreachable_provider()
        }
    }

    fn schur_complex64(
        &self,
        request: SchurRequest<'_, Complex64>,
    ) -> Result<SchurResult<Complex64>, LinalgError> {
        #[cfg(windows)]
        return schur_complex64_native(request, |dimensions, matrix, values, vectors| {
            self.library.zgees(dimensions, matrix, values, vectors)
        });
        #[cfg(not(windows))]
        {
            let _ = request;
            unreachable_provider()
        }
    }
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn svd_real_native<T: Copy>(
    request: SvdRequest<'_, T>,
    zero: T,
    one: T,
    operation: &'static str,
    call: impl FnOnce(
        Lp64SpectralDimensions,
        SvdVectors,
        &mut [T],
        &mut [T],
        &mut [T],
        i32,
        &mut [T],
        i32,
    ) -> Result<i32, ffi::CallError>,
) -> Result<SvdResult<T, T>, LinalgError> {
    let dimensions = validate_svd(&request)?;
    let lp64 = Lp64SpectralDimensions::try_from(&dimensions)?;
    request.check_cancellation()?;
    let (u_columns, vh_rows, ldu, ldvt) = svd_output_dimensions(lp64, request.vectors);
    let mut singular_values = try_filled_buffer(
        "OpenBLAS SVD singular values",
        checked_host_length("OpenBLAS SVD order", lp64.k)?,
        zero,
    )?;
    let mut u = identity_rectangular("OpenBLAS SVD U", dimensions.rows(), u_columns, zero, one)?;
    let mut vh = identity_rectangular("OpenBLAS SVD Vh", vh_rows, dimensions.columns(), zero, one)?;
    if dimensions.order() != 0 {
        let mut matrix = try_copy_buffer("OpenBLAS SVD matrix copy", request.matrix.as_slice())?;
        request.check_cancellation()?;
        let info = call(
            lp64,
            request.vectors,
            &mut matrix,
            &mut singular_values,
            &mut u,
            ldu,
            &mut vh,
            ldvt,
        )
        .map_err(|error| provider_failure(operation, &error))?;
        request.check_cancellation()?;
        check_spectral_info(operation, info)?;
    }
    Ok(SvdResult {
        singular_values: DenseArray::from_vec(
            Shape::new([dimensions.order(), 1])?,
            singular_values,
        )?,
        u: (request.vectors != SvdVectors::None)
            .then(|| DenseArray::from_vec(Shape::new([dimensions.rows(), u_columns])?, u))
            .transpose()?,
        vh: (request.vectors != SvdVectors::None)
            .then(|| DenseArray::from_vec(Shape::new([vh_rows, dimensions.columns()])?, vh))
            .transpose()?,
    })
}

#[cfg(windows)]
fn svd_complex32_native(
    request: SvdRequest<'_, Complex32>,
    call: impl FnOnce(
        Lp64SpectralDimensions,
        SvdVectors,
        &mut [PackedComplex32],
        &mut [f32],
        &mut [PackedComplex32],
        i32,
        &mut [PackedComplex32],
        i32,
    ) -> Result<i32, ffi::CallError>,
) -> Result<SvdResult<Complex32, f32>, LinalgError> {
    let dimensions = validate_svd(&request)?;
    let lp64 = Lp64SpectralDimensions::try_from(&dimensions)?;
    request.check_cancellation()?;
    let (u_columns, vh_rows, ldu, ldvt) = svd_output_dimensions(lp64, request.vectors);
    let mut singular_values = try_filled_buffer(
        "complex32 SVD singular values",
        checked_host_length("complex32 SVD order", lp64.k)?,
        0.0_f32,
    )?;
    let mut u = identity_rectangular(
        "complex32 SVD U",
        dimensions.rows(),
        u_columns,
        [0.0, 0.0],
        [1.0, 0.0],
    )?;
    let mut vh = identity_rectangular(
        "complex32 SVD Vh",
        vh_rows,
        dimensions.columns(),
        [0.0, 0.0],
        [1.0, 0.0],
    )?;
    if dimensions.order() != 0 {
        let mut matrix =
            try_pack_complex32("complex32 SVD matrix copy", request.matrix.as_slice())?;
        request.check_cancellation()?;
        let info = call(
            lp64,
            request.vectors,
            &mut matrix,
            &mut singular_values,
            &mut u,
            ldu,
            &mut vh,
            ldvt,
        )
        .map_err(|error| provider_failure("complex32 SVD", &error))?;
        request.check_cancellation()?;
        check_spectral_info("complex32 SVD", info)?;
    }
    Ok(SvdResult {
        singular_values: DenseArray::from_vec(
            Shape::new([dimensions.order(), 1])?,
            singular_values,
        )?,
        u: (request.vectors != SvdVectors::None)
            .then(|| -> Result<_, LinalgError> {
                Ok(DenseArray::from_vec(
                    Shape::new([dimensions.rows(), u_columns])?,
                    try_unpack_complex32("complex32 SVD U", &u)?,
                )?)
            })
            .transpose()?,
        vh: (request.vectors != SvdVectors::None)
            .then(|| -> Result<_, LinalgError> {
                Ok(DenseArray::from_vec(
                    Shape::new([vh_rows, dimensions.columns()])?,
                    try_unpack_complex32("complex32 SVD Vh", &vh)?,
                )?)
            })
            .transpose()?,
    })
}

#[cfg(windows)]
fn svd_complex64_native(
    request: SvdRequest<'_, Complex64>,
    call: impl FnOnce(
        Lp64SpectralDimensions,
        SvdVectors,
        &mut [PackedComplex64],
        &mut [f64],
        &mut [PackedComplex64],
        i32,
        &mut [PackedComplex64],
        i32,
    ) -> Result<i32, ffi::CallError>,
) -> Result<SvdResult<Complex64, f64>, LinalgError> {
    let dimensions = validate_svd(&request)?;
    let lp64 = Lp64SpectralDimensions::try_from(&dimensions)?;
    request.check_cancellation()?;
    let (u_columns, vh_rows, ldu, ldvt) = svd_output_dimensions(lp64, request.vectors);
    let mut singular_values = try_filled_buffer(
        "complex64 SVD singular values",
        checked_host_length("complex64 SVD order", lp64.k)?,
        0.0_f64,
    )?;
    let mut u = identity_rectangular(
        "complex64 SVD U",
        dimensions.rows(),
        u_columns,
        [0.0, 0.0],
        [1.0, 0.0],
    )?;
    let mut vh = identity_rectangular(
        "complex64 SVD Vh",
        vh_rows,
        dimensions.columns(),
        [0.0, 0.0],
        [1.0, 0.0],
    )?;
    if dimensions.order() != 0 {
        let mut matrix = try_pack_complex("complex64 SVD matrix copy", request.matrix.as_slice())?;
        request.check_cancellation()?;
        let info = call(
            lp64,
            request.vectors,
            &mut matrix,
            &mut singular_values,
            &mut u,
            ldu,
            &mut vh,
            ldvt,
        )
        .map_err(|error| provider_failure("complex64 SVD", &error))?;
        request.check_cancellation()?;
        check_spectral_info("complex64 SVD", info)?;
    }
    Ok(SvdResult {
        singular_values: DenseArray::from_vec(
            Shape::new([dimensions.order(), 1])?,
            singular_values,
        )?,
        u: (request.vectors != SvdVectors::None)
            .then(|| -> Result<_, LinalgError> {
                Ok(DenseArray::from_vec(
                    Shape::new([dimensions.rows(), u_columns])?,
                    try_unpack_complex("complex64 SVD U", &u)?,
                )?)
            })
            .transpose()?,
        vh: (request.vectors != SvdVectors::None)
            .then(|| -> Result<_, LinalgError> {
                Ok(DenseArray::from_vec(
                    Shape::new([vh_rows, dimensions.columns()])?,
                    try_unpack_complex("complex64 SVD Vh", &vh)?,
                )?)
            })
            .transpose()?,
    })
}

#[cfg(windows)]
fn eig_real32_native(
    request: EigRequest<'_, f32>,
    call: impl FnOnce(
        Lp64SpectralDimensions,
        bool,
        bool,
        &mut [f32],
        &mut [f32],
        &mut [f32],
        &mut [f32],
        &mut [f32],
    ) -> Result<i32, ffi::CallError>,
) -> Result<EigResult<Complex32>, LinalgError> {
    eig_real_native(request, Complex32::new, "f32 general eig", call)
}

#[cfg(windows)]
fn eig_real64_native(
    request: EigRequest<'_, f64>,
    call: impl FnOnce(
        Lp64SpectralDimensions,
        bool,
        bool,
        &mut [f64],
        &mut [f64],
        &mut [f64],
        &mut [f64],
        &mut [f64],
    ) -> Result<i32, ffi::CallError>,
) -> Result<EigResult<Complex64>, LinalgError> {
    eig_real_native(request, Complex64::new, "f64 general eig", call)
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn eig_real_native<R, C: Copy>(
    request: EigRequest<'_, R>,
    complex: impl Fn(R, R) -> C + Copy,
    operation: &'static str,
    call: impl FnOnce(
        Lp64SpectralDimensions,
        bool,
        bool,
        &mut [R],
        &mut [R],
        &mut [R],
        &mut [R],
        &mut [R],
    ) -> Result<i32, ffi::CallError>,
) -> Result<EigResult<C>, LinalgError>
where
    R: Copy + Default + PartialOrd + std::ops::Neg<Output = R>,
{
    let dimensions = validate_eig(&request)?;
    let lp64 = Lp64SpectralDimensions::try_from(&dimensions)?;
    request.check_cancellation()?;
    let order = checked_host_length("OpenBLAS eig order", lp64.n)?;
    let square = order
        .checked_mul(order)
        .ok_or_else(|| allocation_failure("OpenBLAS eig vector dimensions", usize::MAX))?;
    let mut matrix = try_copy_buffer("OpenBLAS eig matrix copy", request.matrix.as_slice())?;
    let mut real_values = try_filled_buffer("OpenBLAS eig real values", order, R::default())?;
    let mut imaginary_values =
        try_filled_buffer("OpenBLAS eig imaginary values", order, R::default())?;
    let mut left = try_filled_buffer(
        "OpenBLAS eig left workspace",
        if request.left_vectors { square } else { 0 },
        R::default(),
    )?;
    let mut right = try_filled_buffer(
        "OpenBLAS eig right workspace",
        if request.right_vectors { square } else { 0 },
        R::default(),
    )?;
    if order != 0 {
        request.check_cancellation()?;
        let info = call(
            lp64,
            request.left_vectors,
            request.right_vectors,
            &mut matrix,
            &mut real_values,
            &mut imaginary_values,
            &mut left,
            &mut right,
        )
        .map_err(|error| provider_failure(operation, &error))?;
        request.check_cancellation()?;
        check_spectral_info(operation, info)?;
    }
    let mut eigenvalues = Vec::new();
    eigenvalues
        .try_reserve_exact(order)
        .map_err(|_| allocation_failure("OpenBLAS real eig values", order))?;
    eigenvalues.extend(
        real_values
            .iter()
            .copied()
            .zip(imaginary_values.iter().copied())
            .map(|(re, im)| complex(re, im)),
    );
    Ok(EigResult {
        eigenvalues: DenseArray::from_vec(Shape::new([dimensions.rows(), 1])?, eigenvalues)?,
        left_vectors: request
            .left_vectors
            .then(|| {
                normalize_real_eigenvectors(operation, order, &imaginary_values, &left, complex)
            })
            .transpose()?
            .map(|values| {
                DenseArray::from_vec(
                    Shape::new([dimensions.rows(), dimensions.rows()])
                        .expect("validated eig shape"),
                    values,
                )
            })
            .transpose()?,
        right_vectors: request
            .right_vectors
            .then(|| {
                normalize_real_eigenvectors(operation, order, &imaginary_values, &right, complex)
            })
            .transpose()?
            .map(|values| {
                DenseArray::from_vec(
                    Shape::new([dimensions.rows(), dimensions.rows()])
                        .expect("validated eig shape"),
                    values,
                )
            })
            .transpose()?,
    })
}

#[cfg(windows)]
fn eig_complex32_native(
    request: EigRequest<'_, Complex32>,
    call: impl FnOnce(
        Lp64SpectralDimensions,
        bool,
        bool,
        &mut [PackedComplex32],
        &mut [PackedComplex32],
        &mut [PackedComplex32],
        &mut [PackedComplex32],
    ) -> Result<i32, ffi::CallError>,
) -> Result<EigResult<Complex32>, LinalgError> {
    eig_complex_native(
        request,
        [0.0_f32, 0.0_f32],
        "complex32 general eig",
        try_pack_complex32,
        try_unpack_complex32,
        call,
    )
}

#[cfg(windows)]
fn eig_complex64_native(
    request: EigRequest<'_, Complex64>,
    call: impl FnOnce(
        Lp64SpectralDimensions,
        bool,
        bool,
        &mut [PackedComplex64],
        &mut [PackedComplex64],
        &mut [PackedComplex64],
        &mut [PackedComplex64],
    ) -> Result<i32, ffi::CallError>,
) -> Result<EigResult<Complex64>, LinalgError> {
    eig_complex_native(
        request,
        [0.0_f64, 0.0_f64],
        "complex64 general eig",
        try_pack_complex,
        try_unpack_complex,
        call,
    )
}

#[cfg(windows)]
#[allow(clippy::type_complexity)]
fn eig_complex_native<T: Copy, P: Copy>(
    request: EigRequest<'_, T>,
    zero: P,
    operation: &'static str,
    pack: impl Fn(&'static str, &[T]) -> Result<Vec<P>, LinalgError>,
    unpack: impl Fn(&'static str, &[P]) -> Result<Vec<T>, LinalgError>,
    call: impl FnOnce(
        Lp64SpectralDimensions,
        bool,
        bool,
        &mut [P],
        &mut [P],
        &mut [P],
        &mut [P],
    ) -> Result<i32, ffi::CallError>,
) -> Result<EigResult<T>, LinalgError> {
    let dimensions = validate_eig(&request)?;
    let lp64 = Lp64SpectralDimensions::try_from(&dimensions)?;
    request.check_cancellation()?;
    let order = checked_host_length("OpenBLAS complex eig order", lp64.n)?;
    let square = order
        .checked_mul(order)
        .ok_or_else(|| allocation_failure("OpenBLAS complex eig vector dimensions", usize::MAX))?;
    let mut matrix = pack(
        "OpenBLAS complex eig matrix copy",
        request.matrix.as_slice(),
    )?;
    let mut values = try_filled_buffer("OpenBLAS complex eig values", order, zero)?;
    let mut left = try_filled_buffer(
        "OpenBLAS complex eig left workspace",
        if request.left_vectors { square } else { 0 },
        zero,
    )?;
    let mut right = try_filled_buffer(
        "OpenBLAS complex eig right workspace",
        if request.right_vectors { square } else { 0 },
        zero,
    )?;
    if order != 0 {
        request.check_cancellation()?;
        let info = call(
            lp64,
            request.left_vectors,
            request.right_vectors,
            &mut matrix,
            &mut values,
            &mut left,
            &mut right,
        )
        .map_err(|error| provider_failure(operation, &error))?;
        request.check_cancellation()?;
        check_spectral_info(operation, info)?;
    }
    Ok(EigResult {
        eigenvalues: DenseArray::from_vec(
            Shape::new([dimensions.rows(), 1])?,
            unpack(operation, &values)?,
        )?,
        left_vectors: request
            .left_vectors
            .then(|| -> Result<_, LinalgError> {
                Ok(DenseArray::from_vec(
                    Shape::new([dimensions.rows(), dimensions.rows()])?,
                    unpack(operation, &left)?,
                )?)
            })
            .transpose()?,
        right_vectors: request
            .right_vectors
            .then(|| -> Result<_, LinalgError> {
                Ok(DenseArray::from_vec(
                    Shape::new([dimensions.rows(), dimensions.rows()])?,
                    unpack(operation, &right)?,
                )?)
            })
            .transpose()?,
    })
}

#[cfg(windows)]
fn schur_complex32_native(
    request: SchurRequest<'_, Complex32>,
    call: impl FnOnce(
        Lp64SpectralDimensions,
        &mut [PackedComplex32],
        &mut [PackedComplex32],
        &mut [PackedComplex32],
    ) -> Result<(i32, i32), ffi::CallError>,
) -> Result<SchurResult<Complex32>, LinalgError> {
    schur_complex_native(
        request,
        [0.0_f32, 0.0_f32],
        "complex32 Schur decomposition",
        try_pack_complex32,
        try_unpack_complex32,
        call,
    )
}

#[cfg(windows)]
fn schur_complex64_native(
    request: SchurRequest<'_, Complex64>,
    call: impl FnOnce(
        Lp64SpectralDimensions,
        &mut [PackedComplex64],
        &mut [PackedComplex64],
        &mut [PackedComplex64],
    ) -> Result<(i32, i32), ffi::CallError>,
) -> Result<SchurResult<Complex64>, LinalgError> {
    schur_complex_native(
        request,
        [0.0_f64, 0.0_f64],
        "complex64 Schur decomposition",
        try_pack_complex,
        try_unpack_complex,
        call,
    )
}

#[cfg(windows)]
#[allow(clippy::type_complexity)]
fn schur_complex_native<T: Copy, P: Copy>(
    request: SchurRequest<'_, T>,
    zero: P,
    operation: &'static str,
    pack: impl Fn(&'static str, &[T]) -> Result<Vec<P>, LinalgError>,
    unpack: impl Fn(&'static str, &[P]) -> Result<Vec<T>, LinalgError>,
    call: impl FnOnce(
        Lp64SpectralDimensions,
        &mut [P],
        &mut [P],
        &mut [P],
    ) -> Result<(i32, i32), ffi::CallError>,
) -> Result<SchurResult<T>, LinalgError> {
    let dimensions = validate_schur(&request)?;
    let lp64 = Lp64SpectralDimensions::try_from(&dimensions)?;
    request.check_cancellation()?;
    let order = checked_host_length("OpenBLAS Schur order", lp64.n)?;
    let square = order
        .checked_mul(order)
        .ok_or_else(|| allocation_failure("OpenBLAS Schur dimensions", usize::MAX))?;
    let mut form = pack("OpenBLAS Schur matrix copy", request.matrix.as_slice())?;
    let mut values = try_filled_buffer("OpenBLAS Schur eigenvalues", order, zero)?;
    let mut vectors = try_filled_buffer("OpenBLAS Schur vectors", square, zero)?;
    if order != 0 {
        request.check_cancellation()?;
        let (info, selected) = call(lp64, &mut form, &mut values, &mut vectors)
            .map_err(|error| provider_failure(operation, &error))?;
        request.check_cancellation()?;
        check_spectral_info(operation, info)?;
        if selected != 0 {
            return Err(LinalgError::ProviderFailure {
                provider: PROVIDER_NAME,
                operation,
                detail: format!(
                    "LAPACKE GEES reported {selected} selected eigenvalues with sorting disabled"
                ),
            });
        }
    }
    let shape = Shape::new([dimensions.rows(), dimensions.rows()])?;
    Ok(SchurResult {
        form: DenseArray::from_vec(shape.clone(), unpack(operation, &form)?)?,
        vectors: DenseArray::from_vec(shape, unpack(operation, &vectors)?)?,
    })
}

#[cfg(windows)]
fn normalize_real_eigenvectors<R, C>(
    operation: &'static str,
    order: usize,
    imaginary_values: &[R],
    vectors: &[R],
    complex: impl Fn(R, R) -> C,
) -> Result<Vec<C>, LinalgError>
where
    R: Copy + Default + PartialOrd + std::ops::Neg<Output = R>,
    C: Copy,
{
    let zero = R::default();
    let mut result = Vec::new();
    result
        .try_reserve_exact(vectors.len())
        .map_err(|_| allocation_failure(operation, vectors.len()))?;
    result.resize_with(vectors.len(), || complex(zero, zero));
    let mut column = 0;
    while column < order {
        let imaginary = imaginary_values[column];
        if imaginary > zero {
            if column + 1 >= order || imaginary_values[column + 1] != -imaginary {
                return Err(LinalgError::ProviderFailure {
                    provider: PROVIDER_NAME,
                    operation,
                    detail: format!("invalid real GEEV conjugate-pair encoding at column {column}"),
                });
            }
            for row in 0..order {
                let real = vectors[column * order + row];
                let imag = vectors[(column + 1) * order + row];
                result[column * order + row] = complex(real, imag);
                result[(column + 1) * order + row] = complex(real, -imag);
            }
            column += 2;
        } else if imaginary < zero {
            return Err(LinalgError::ProviderFailure {
                provider: PROVIDER_NAME,
                operation,
                detail: format!("unpaired negative imaginary eigenvalue at column {column}"),
            });
        } else {
            for row in 0..order {
                result[column * order + row] = complex(vectors[column * order + row], zero);
            }
            column += 1;
        }
    }
    Ok(result)
}

#[cfg(windows)]
fn svd_output_dimensions(
    dimensions: Lp64SpectralDimensions,
    vectors: SvdVectors,
) -> (u64, u64, i32, i32) {
    match vectors {
        SvdVectors::None => (0, 0, 1, 1),
        SvdVectors::Thin => (
            u64::from(dimensions.k.unsigned_abs()),
            u64::from(dimensions.k.unsigned_abs()),
            dimensions.m.max(1),
            dimensions.k.max(1),
        ),
        SvdVectors::Full => (
            u64::from(dimensions.m.unsigned_abs()),
            u64::from(dimensions.n.unsigned_abs()),
            dimensions.m.max(1),
            dimensions.n.max(1),
        ),
    }
}

#[cfg(windows)]
fn identity_rectangular<T: Copy>(
    operation: &'static str,
    rows: u64,
    columns: u64,
    zero: T,
    one: T,
) -> Result<Vec<T>, LinalgError> {
    let rows_host = checked_host_u64(operation, rows)?;
    let columns_host = checked_host_u64(operation, columns)?;
    let length = rows_host
        .checked_mul(columns_host)
        .ok_or_else(|| allocation_failure(operation, usize::MAX))?;
    let mut result = try_filled_buffer(operation, length, zero)?;
    for diagonal in 0..rows_host.min(columns_host) {
        result[diagonal * rows_host + diagonal] = one;
    }
    Ok(result)
}

#[cfg(windows)]
fn factor_lu_native<T: Copy>(
    request: FactorRequest<'_, T>,
    operation: &'static str,
    call: impl FnOnce(Lp64FactorDimensions, &mut [T], &mut [i32]) -> Result<i32, ffi::CallError>,
) -> Result<LuResult<T>, LinalgError> {
    let dimensions = validate_factor(&request)?;
    let lp64 = Lp64FactorDimensions::try_from(&dimensions)?;
    request.check_cancellation()?;
    let mut packed_lu = try_copy_buffer("OpenBLAS LU matrix copy", request.matrix.as_slice())?;
    let mut pivots = try_allocate_i32("OpenBLAS LU pivot buffer", lp64.k)?;
    let info = if dimensions.order() == 0 {
        0
    } else {
        request.check_cancellation()?;
        let info = call(lp64, &mut packed_lu, &mut pivots)
            .map_err(|error| provider_failure(operation, &error))?;
        request.check_cancellation()?;
        info
    };
    let first_zero_pivot = normalize_factor_info(operation, info, dimensions.order())?;
    let (row_permutation_zero_based, swap_parity) =
        normalize_row_pivots(operation, dimensions.rows(), &pivots)?;
    Ok(LuResult {
        packed_lu: DenseArray::from_vec(request.matrix.shape().clone(), packed_lu)?,
        row_permutation_zero_based,
        swap_parity,
        first_zero_pivot,
    })
}

#[cfg(windows)]
fn factor_lu_complex32_native(
    request: FactorRequest<'_, Complex32>,
    call: impl FnOnce(
        Lp64FactorDimensions,
        &mut [PackedComplex32],
        &mut [i32],
    ) -> Result<i32, ffi::CallError>,
) -> Result<LuResult<Complex32>, LinalgError> {
    let dimensions = validate_factor(&request)?;
    let lp64 = Lp64FactorDimensions::try_from(&dimensions)?;
    request.check_cancellation()?;
    let mut packed = try_pack_complex32("complex32 LU matrix copy", request.matrix.as_slice())?;
    let mut pivots = try_allocate_i32("complex32 LU pivot buffer", lp64.k)?;
    let info = if dimensions.order() == 0 {
        0
    } else {
        request.check_cancellation()?;
        let info = call(lp64, &mut packed, &mut pivots)
            .map_err(|error| provider_failure("complex32 LU factorization", &error))?;
        request.check_cancellation()?;
        info
    };
    let first_zero_pivot =
        normalize_factor_info("complex32 LU factorization", info, dimensions.order())?;
    let (row_permutation_zero_based, swap_parity) =
        normalize_row_pivots("complex32 LU factorization", dimensions.rows(), &pivots)?;
    Ok(LuResult {
        packed_lu: DenseArray::from_vec(
            request.matrix.shape().clone(),
            try_unpack_complex32("complex32 LU result", &packed)?,
        )?,
        row_permutation_zero_based,
        swap_parity,
        first_zero_pivot,
    })
}

#[cfg(windows)]
fn factor_lu_complex64_native(
    request: FactorRequest<'_, Complex64>,
    call: impl FnOnce(
        Lp64FactorDimensions,
        &mut [PackedComplex64],
        &mut [i32],
    ) -> Result<i32, ffi::CallError>,
) -> Result<LuResult<Complex64>, LinalgError> {
    let dimensions = validate_factor(&request)?;
    let lp64 = Lp64FactorDimensions::try_from(&dimensions)?;
    request.check_cancellation()?;
    let mut packed = try_pack_complex("complex64 LU matrix copy", request.matrix.as_slice())?;
    let mut pivots = try_allocate_i32("complex64 LU pivot buffer", lp64.k)?;
    let info = if dimensions.order() == 0 {
        0
    } else {
        request.check_cancellation()?;
        let info = call(lp64, &mut packed, &mut pivots)
            .map_err(|error| provider_failure("complex64 LU factorization", &error))?;
        request.check_cancellation()?;
        info
    };
    let first_zero_pivot =
        normalize_factor_info("complex64 LU factorization", info, dimensions.order())?;
    let (row_permutation_zero_based, swap_parity) =
        normalize_row_pivots("complex64 LU factorization", dimensions.rows(), &pivots)?;
    Ok(LuResult {
        packed_lu: DenseArray::from_vec(
            request.matrix.shape().clone(),
            try_unpack_complex("complex64 LU result", &packed)?,
        )?,
        row_permutation_zero_based,
        swap_parity,
        first_zero_pivot,
    })
}

#[cfg(windows)]
struct NativeQr<T> {
    q: Vec<T>,
    r: Vec<T>,
    q_columns: u64,
    column_permutation_zero_based: Vec<u64>,
    numerical_rank: u64,
    first_rank_deficient_diagonal: Option<u64>,
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn qr_buffer<T: Copy>(
    dimensions: FactorDimensions,
    lp64: Lp64FactorDimensions,
    vectors: QrVectors,
    pivot_columns: bool,
    mut matrix: Vec<T>,
    zero: T,
    one: T,
    epsilon: f64,
    magnitude: impl Fn(T) -> f64,
    mut check_cancellation: impl FnMut() -> Result<(), LinalgError>,
    operation: &'static str,
    factor: impl FnOnce(
        Lp64FactorDimensions,
        &mut [T],
        &mut [i32],
        &mut [T],
        bool,
    ) -> Result<i32, ffi::CallError>,
    generate: impl FnOnce(Lp64FactorDimensions, i32, &mut [T], &[T]) -> Result<i32, ffi::CallError>,
) -> Result<NativeQr<T>, LinalgError> {
    let mut pivots = try_allocate_i32("OpenBLAS QR column pivot buffer", lp64.n)?;
    let mut tau = try_filled_buffer(
        "OpenBLAS QR reflector buffer",
        checked_host_length("OpenBLAS QR reflector length", lp64.k)?,
        zero,
    )?;
    let scale = matrix
        .iter()
        .copied()
        .map(&magnitude)
        .fold(0.0_f64, f64::max);
    check_cancellation()?;
    if dimensions.order() != 0 {
        let info = factor(lp64, &mut matrix, &mut pivots, &mut tau, pivot_columns)
            .map_err(|error| provider_failure(operation, &error))?;
        check_cancellation()?;
        check_qr_info(operation, info)?;
    }

    let column_permutation_zero_based = if pivot_columns && dimensions.order() != 0 {
        normalize_column_pivots(operation, dimensions.columns(), &pivots)?
    } else {
        identity_permutation("OpenBLAS QR identity permutation", dimensions.columns())?
    };
    let rows = checked_host_u64("OpenBLAS QR row dimension", dimensions.rows())?;
    let columns = checked_host_u64("OpenBLAS QR column dimension", dimensions.columns())?;
    let order = rows.min(columns);
    let q_columns = match vectors {
        QrVectors::Full => rows,
        QrVectors::Thin => order,
    };
    let r_length = q_columns
        .checked_mul(columns)
        .ok_or_else(|| allocation_failure("OpenBLAS QR R dimensions", usize::MAX))?;
    let mut r = try_filled_buffer("OpenBLAS QR R", r_length, zero)?;
    for column in 0..columns {
        for row in 0..q_columns.min(column.saturating_add(1)) {
            r[column * q_columns + row] = matrix[column * rows + row];
        }
    }

    let q_length = rows
        .checked_mul(q_columns)
        .ok_or_else(|| allocation_failure("OpenBLAS QR Q dimensions", usize::MAX))?;
    let mut q = try_filled_buffer("OpenBLAS QR Q workspace", q_length, zero)?;
    if order == 0 {
        for diagonal in 0..q_columns.min(rows) {
            q[diagonal * rows + diagonal] = one;
        }
    }
    for column in 0..order {
        let source = column * rows;
        q[source..source + rows].copy_from_slice(&matrix[source..source + rows]);
    }
    if order != 0 {
        let q_columns_i32 =
            checked_lp64_dimension("Q columns", u64::try_from(q_columns).unwrap_or(u64::MAX))?;
        let info = generate(lp64, q_columns_i32, &mut q, &tau)
            .map_err(|error| provider_failure(operation, &error))?;
        check_cancellation()?;
        check_qr_info(operation, info)?;
    }

    let tolerance = scale * epsilon * f64::from(lp64.m.max(lp64.n));
    let mut numerical_rank = 0_usize;
    let mut first_rank_deficient_diagonal = None;
    for diagonal in 0..order {
        if magnitude(matrix[diagonal * rows + diagonal]) > tolerance {
            numerical_rank += 1;
        } else if first_rank_deficient_diagonal.is_none() {
            first_rank_deficient_diagonal = Some(u64::try_from(diagonal).unwrap_or(u64::MAX));
        }
    }
    Ok(NativeQr {
        q,
        r,
        q_columns: u64::try_from(q_columns).unwrap_or(u64::MAX),
        column_permutation_zero_based,
        numerical_rank: u64::try_from(numerical_rank).unwrap_or(u64::MAX),
        first_rank_deficient_diagonal,
    })
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn qr_native<T: Copy>(
    request: QrRequest<'_, T>,
    operation: &'static str,
    zero: T,
    one: T,
    epsilon: f64,
    magnitude: impl Fn(T) -> f64,
    factor: impl FnOnce(
        Lp64FactorDimensions,
        &mut [T],
        &mut [i32],
        &mut [T],
        bool,
    ) -> Result<i32, ffi::CallError>,
    generate: impl FnOnce(Lp64FactorDimensions, i32, &mut [T], &[T]) -> Result<i32, ffi::CallError>,
) -> Result<QrResult<T>, LinalgError> {
    let dimensions = validate_qr(&request)?;
    let lp64 = Lp64FactorDimensions::try_from(&dimensions)?;
    request.check_cancellation()?;
    let native = qr_buffer(
        dimensions,
        lp64,
        request.vectors,
        request.pivot_columns,
        try_copy_buffer("OpenBLAS QR matrix copy", request.matrix.as_slice())?,
        zero,
        one,
        epsilon,
        magnitude,
        || request.check_cancellation(),
        operation,
        factor,
        generate,
    )?;
    Ok(QrResult {
        q: DenseArray::from_vec(Shape::new([dimensions.rows(), native.q_columns])?, native.q)?,
        r: DenseArray::from_vec(
            Shape::new([native.q_columns, dimensions.columns()])?,
            native.r,
        )?,
        column_permutation_zero_based: native.column_permutation_zero_based,
        numerical_rank: native.numerical_rank,
        first_rank_deficient_diagonal: native.first_rank_deficient_diagonal,
    })
}

#[cfg(windows)]
fn qr_complex32_native(
    request: QrRequest<'_, Complex32>,
    factor: impl FnOnce(
        Lp64FactorDimensions,
        &mut [PackedComplex32],
        &mut [i32],
        &mut [PackedComplex32],
        bool,
    ) -> Result<i32, ffi::CallError>,
    generate: impl FnOnce(
        Lp64FactorDimensions,
        i32,
        &mut [PackedComplex32],
        &[PackedComplex32],
    ) -> Result<i32, ffi::CallError>,
) -> Result<QrResult<Complex32>, LinalgError> {
    let dimensions = validate_qr(&request)?;
    let lp64 = Lp64FactorDimensions::try_from(&dimensions)?;
    request.check_cancellation()?;
    let native = qr_buffer(
        dimensions,
        lp64,
        request.vectors,
        request.pivot_columns,
        try_pack_complex32("complex32 QR matrix copy", request.matrix.as_slice())?,
        [0.0_f32, 0.0_f32],
        [1.0_f32, 0.0_f32],
        f64::from(f32::EPSILON),
        |value| f64::from(value[0].hypot(value[1])),
        || request.check_cancellation(),
        "complex32 QR factorization",
        factor,
        generate,
    )?;
    Ok(QrResult {
        q: DenseArray::from_vec(
            Shape::new([dimensions.rows(), native.q_columns])?,
            try_unpack_complex32("complex32 QR Q", &native.q)?,
        )?,
        r: DenseArray::from_vec(
            Shape::new([native.q_columns, dimensions.columns()])?,
            try_unpack_complex32("complex32 QR R", &native.r)?,
        )?,
        column_permutation_zero_based: native.column_permutation_zero_based,
        numerical_rank: native.numerical_rank,
        first_rank_deficient_diagonal: native.first_rank_deficient_diagonal,
    })
}

#[cfg(windows)]
fn qr_complex64_native(
    request: QrRequest<'_, Complex64>,
    factor: impl FnOnce(
        Lp64FactorDimensions,
        &mut [PackedComplex64],
        &mut [i32],
        &mut [PackedComplex64],
        bool,
    ) -> Result<i32, ffi::CallError>,
    generate: impl FnOnce(
        Lp64FactorDimensions,
        i32,
        &mut [PackedComplex64],
        &[PackedComplex64],
    ) -> Result<i32, ffi::CallError>,
) -> Result<QrResult<Complex64>, LinalgError> {
    let dimensions = validate_qr(&request)?;
    let lp64 = Lp64FactorDimensions::try_from(&dimensions)?;
    request.check_cancellation()?;
    let native = qr_buffer(
        dimensions,
        lp64,
        request.vectors,
        request.pivot_columns,
        try_pack_complex("complex64 QR matrix copy", request.matrix.as_slice())?,
        [0.0_f64, 0.0_f64],
        [1.0_f64, 0.0_f64],
        f64::EPSILON,
        |value| value[0].hypot(value[1]),
        || request.check_cancellation(),
        "complex64 QR factorization",
        factor,
        generate,
    )?;
    Ok(QrResult {
        q: DenseArray::from_vec(
            Shape::new([dimensions.rows(), native.q_columns])?,
            try_unpack_complex("complex64 QR Q", &native.q)?,
        )?,
        r: DenseArray::from_vec(
            Shape::new([native.q_columns, dimensions.columns()])?,
            try_unpack_complex("complex64 QR R", &native.r)?,
        )?,
        column_permutation_zero_based: native.column_permutation_zero_based,
        numerical_rank: native.numerical_rank,
        first_rank_deficient_diagonal: native.first_rank_deficient_diagonal,
    })
}

#[cfg(windows)]
fn cholesky_native<T: Copy>(
    request: CholeskyRequest<'_, T>,
    operation: &'static str,
    zero: T,
    call: impl FnOnce(Lp64CholeskyDimensions, CholeskyTriangle, &mut [T]) -> Result<i32, ffi::CallError>,
) -> Result<CholeskyResult<T>, LinalgError> {
    let dimensions = validate_cholesky(&request)?;
    let lp64 = Lp64CholeskyDimensions::try_from(&dimensions)?;
    request.check_cancellation()?;
    let mut matrix = try_copy_buffer("OpenBLAS Cholesky matrix copy", request.matrix.as_slice())?;
    let info = if dimensions.order() == 0 {
        0
    } else {
        request.check_cancellation()?;
        let info = call(lp64, request.triangle, &mut matrix)
            .map_err(|error| provider_failure(operation, &error))?;
        request.check_cancellation()?;
        info
    };
    let first_non_positive_minor = normalize_factor_info(operation, info, dimensions.order())?;
    let factor = normalize_cholesky_factor(
        operation,
        &matrix,
        dimensions.order(),
        request.triangle,
        first_non_positive_minor,
        zero,
    )?;
    Ok(CholeskyResult {
        factor: DenseArray::from_vec(request.matrix.shape().clone(), factor)?,
        first_non_positive_minor,
    })
}

#[cfg(windows)]
fn cholesky_complex32_native(
    request: CholeskyRequest<'_, Complex32>,
    call: impl FnOnce(
        Lp64CholeskyDimensions,
        CholeskyTriangle,
        &mut [PackedComplex32],
    ) -> Result<i32, ffi::CallError>,
) -> Result<CholeskyResult<Complex32>, LinalgError> {
    let dimensions = validate_cholesky(&request)?;
    let lp64 = Lp64CholeskyDimensions::try_from(&dimensions)?;
    request.check_cancellation()?;
    let mut matrix =
        try_pack_complex32("complex32 Cholesky matrix copy", request.matrix.as_slice())?;
    let info = if dimensions.order() == 0 {
        0
    } else {
        request.check_cancellation()?;
        let info = call(lp64, request.triangle, &mut matrix)
            .map_err(|error| provider_failure("complex32 Cholesky factorization", &error))?;
        request.check_cancellation()?;
        info
    };
    let status =
        normalize_factor_info("complex32 Cholesky factorization", info, dimensions.order())?;
    let factor = normalize_cholesky_factor(
        "complex32 Cholesky factorization",
        &matrix,
        dimensions.order(),
        request.triangle,
        status,
        [0.0_f32, 0.0_f32],
    )?;
    Ok(CholeskyResult {
        factor: DenseArray::from_vec(
            request.matrix.shape().clone(),
            try_unpack_complex32("complex32 Cholesky result", &factor)?,
        )?,
        first_non_positive_minor: status,
    })
}

#[cfg(windows)]
fn cholesky_complex64_native(
    request: CholeskyRequest<'_, Complex64>,
    call: impl FnOnce(
        Lp64CholeskyDimensions,
        CholeskyTriangle,
        &mut [PackedComplex64],
    ) -> Result<i32, ffi::CallError>,
) -> Result<CholeskyResult<Complex64>, LinalgError> {
    let dimensions = validate_cholesky(&request)?;
    let lp64 = Lp64CholeskyDimensions::try_from(&dimensions)?;
    request.check_cancellation()?;
    let mut matrix = try_pack_complex("complex64 Cholesky matrix copy", request.matrix.as_slice())?;
    let info = if dimensions.order() == 0 {
        0
    } else {
        request.check_cancellation()?;
        let info = call(lp64, request.triangle, &mut matrix)
            .map_err(|error| provider_failure("complex64 Cholesky factorization", &error))?;
        request.check_cancellation()?;
        info
    };
    let status =
        normalize_factor_info("complex64 Cholesky factorization", info, dimensions.order())?;
    let factor = normalize_cholesky_factor(
        "complex64 Cholesky factorization",
        &matrix,
        dimensions.order(),
        request.triangle,
        status,
        [0.0_f64, 0.0_f64],
    )?;
    Ok(CholeskyResult {
        factor: DenseArray::from_vec(
            request.matrix.shape().clone(),
            try_unpack_complex("complex64 Cholesky result", &factor)?,
        )?,
        first_non_positive_minor: status,
    })
}

#[cfg(windows)]
fn normalize_factor_info(
    operation: &'static str,
    info: i32,
    limit: u64,
) -> Result<Option<u64>, LinalgError> {
    if info < 0 {
        return Err(LinalgError::InvalidProviderArgument {
            provider: PROVIDER_NAME,
            operation,
            argument: info.unsigned_abs(),
        });
    }
    if info == 0 {
        return Ok(None);
    }
    let one_based = u64::from(info.unsigned_abs());
    if one_based > limit {
        return Err(LinalgError::ProviderFailure {
            provider: PROVIDER_NAME,
            operation,
            detail: format!("LAPACK info {info} exceeds factor order {limit}"),
        });
    }
    Ok(Some(one_based - 1))
}

#[cfg(windows)]
fn check_qr_info(operation: &'static str, info: i32) -> Result<(), LinalgError> {
    match info.cmp(&0) {
        std::cmp::Ordering::Less => Err(LinalgError::InvalidProviderArgument {
            provider: PROVIDER_NAME,
            operation,
            argument: info.unsigned_abs(),
        }),
        std::cmp::Ordering::Greater => Err(LinalgError::ProviderFailure {
            provider: PROVIDER_NAME,
            operation,
            detail: format!("LAPACK returned unexpected positive info {info}"),
        }),
        std::cmp::Ordering::Equal => Ok(()),
    }
}

#[cfg(windows)]
fn check_spectral_info(operation: &'static str, info: i32) -> Result<(), LinalgError> {
    match info.cmp(&0) {
        std::cmp::Ordering::Equal => Ok(()),
        std::cmp::Ordering::Greater => Err(LinalgError::NoConvergence {
            provider: PROVIDER_NAME,
            operation,
            iterations: None,
            unconverged: Some(u64::from(info.unsigned_abs())),
        }),
        std::cmp::Ordering::Less if matches!(info, -1010 | -1011) => {
            Err(LinalgError::AllocationFailure {
                operation,
                elements: 0,
            })
        }
        std::cmp::Ordering::Less => Err(LinalgError::InvalidProviderArgument {
            provider: PROVIDER_NAME,
            operation,
            argument: info.unsigned_abs(),
        }),
    }
}

#[cfg(windows)]
fn normalize_row_pivots(
    operation: &'static str,
    rows: u64,
    pivots: &[i32],
) -> Result<(Vec<u64>, SwapParity), LinalgError> {
    let mut permutation = identity_permutation("OpenBLAS LU row permutation", rows)?;
    let mut parity = SwapParity::Even;
    for (index, &pivot) in pivots.iter().enumerate() {
        let one_based_index = i64::try_from(index)
            .ok()
            .and_then(|index| index.checked_add(1))
            .unwrap_or(i64::MAX);
        let pivot_i64 = i64::from(pivot);
        if pivot_i64 < one_based_index || u64::try_from(pivot_i64).map_or(true, |p| p > rows) {
            return Err(LinalgError::ProviderFailure {
                provider: PROVIDER_NAME,
                operation,
                detail: format!(
                    "LAPACK row pivot {} at position {} is outside {}..={rows}",
                    pivot,
                    index + 1,
                    index + 1
                ),
            });
        }
        let pivot_index = usize::try_from(pivot - 1).map_err(|_| LinalgError::ProviderFailure {
            provider: PROVIDER_NAME,
            operation,
            detail: format!("LAPACK row pivot {pivot} cannot index host memory"),
        })?;
        if pivot_index != index {
            permutation.swap(index, pivot_index);
            parity = match parity {
                SwapParity::Even => SwapParity::Odd,
                SwapParity::Odd => SwapParity::Even,
            };
        }
    }
    Ok((permutation, parity))
}

#[cfg(windows)]
fn normalize_column_pivots(
    operation: &'static str,
    columns: u64,
    pivots: &[i32],
) -> Result<Vec<u64>, LinalgError> {
    let length = checked_host_u64("OpenBLAS QR column permutation", columns)?;
    if pivots.len() != length {
        return Err(LinalgError::ProviderFailure {
            provider: PROVIDER_NAME,
            operation,
            detail: format!(
                "LAPACK returned {} column pivots for {columns} columns",
                pivots.len()
            ),
        });
    }
    let mut seen = try_filled_buffer("OpenBLAS QR pivot validation", length, false)?;
    let mut permutation = Vec::new();
    permutation
        .try_reserve_exact(length)
        .map_err(|_| allocation_failure("OpenBLAS QR column permutation", length))?;
    for &pivot in pivots {
        if pivot <= 0 || u64::from(pivot.unsigned_abs()) > columns {
            return Err(LinalgError::ProviderFailure {
                provider: PROVIDER_NAME,
                operation,
                detail: format!("LAPACK column pivot {pivot} is outside 1..={columns}"),
            });
        }
        let index = usize::try_from(pivot - 1).map_err(|_| LinalgError::ProviderFailure {
            provider: PROVIDER_NAME,
            operation,
            detail: format!("LAPACK column pivot {pivot} cannot index host memory"),
        })?;
        if seen[index] {
            return Err(LinalgError::ProviderFailure {
                provider: PROVIDER_NAME,
                operation,
                detail: format!("LAPACK column pivot {pivot} is duplicated"),
            });
        }
        seen[index] = true;
        permutation.push(u64::try_from(index).unwrap_or(u64::MAX));
    }
    Ok(permutation)
}

#[cfg(windows)]
fn identity_permutation(operation: &'static str, length: u64) -> Result<Vec<u64>, LinalgError> {
    let length = checked_host_u64(operation, length)?;
    let mut permutation = Vec::new();
    permutation
        .try_reserve_exact(length)
        .map_err(|_| allocation_failure(operation, length))?;
    for index in 0..length {
        permutation.push(u64::try_from(index).unwrap_or(u64::MAX));
    }
    Ok(permutation)
}

#[cfg(windows)]
fn normalize_cholesky_factor<T: Copy>(
    operation: &'static str,
    source: &[T],
    order: u64,
    triangle: CholeskyTriangle,
    first_non_positive_minor: Option<u64>,
    zero: T,
) -> Result<Vec<T>, LinalgError> {
    let order = checked_host_u64(operation, order)?;
    let length = order
        .checked_mul(order)
        .ok_or_else(|| allocation_failure(operation, usize::MAX))?;
    if source.len() != length {
        return Err(LinalgError::ProviderFailure {
            provider: PROVIDER_NAME,
            operation,
            detail: format!(
                "LAPACK Cholesky buffer has {} elements, expected {length}",
                source.len()
            ),
        });
    }
    let successful_order = first_non_positive_minor.map_or(order, |minor| {
        usize::try_from(minor).unwrap_or(usize::MAX).min(order)
    });
    let mut factor = try_filled_buffer(operation, length, zero)?;
    for column in 0..successful_order {
        for row in 0..successful_order {
            let selected = match triangle {
                CholeskyTriangle::Upper => row <= column,
                CholeskyTriangle::Lower => row >= column,
            };
            if selected {
                factor[column * order + row] = source[column * order + row];
            }
        }
    }
    Ok(factor)
}

#[cfg(windows)]
fn checked_host_length(operation: &'static str, value: i32) -> Result<usize, LinalgError> {
    usize::try_from(value).map_err(|_| LinalgError::ProviderFailure {
        provider: PROVIDER_NAME,
        operation,
        detail: format!("validated LP64 length {value} is negative"),
    })
}

#[cfg(windows)]
fn checked_host_u64(operation: &'static str, value: u64) -> Result<usize, LinalgError> {
    usize::try_from(value).map_err(|_| LinalgError::AllocationFailure {
        operation,
        elements: value,
    })
}

#[cfg(windows)]
fn provider_failure(operation: &'static str, error: &ffi::CallError) -> LinalgError {
    LinalgError::ProviderFailure {
        provider: PROVIDER_NAME,
        operation,
        detail: error.to_string(),
    }
}

#[cfg(windows)]
fn check_lapack_info(operation: &'static str, info: i32) -> Result<(), LinalgError> {
    match info.cmp(&0) {
        std::cmp::Ordering::Less => Err(LinalgError::InvalidProviderArgument {
            provider: PROVIDER_NAME,
            operation,
            argument: info.unsigned_abs(),
        }),
        std::cmp::Ordering::Greater => Err(LinalgError::SingularMatrix {
            provider: PROVIDER_NAME,
            operation,
            pivot: u64::from(info.unsigned_abs()),
        }),
        std::cmp::Ordering::Equal => Ok(()),
    }
}

#[cfg(windows)]
fn check_gels_info(
    operation: &'static str,
    info: i32,
    required_rank: u64,
) -> Result<(), LinalgError> {
    match info.cmp(&0) {
        std::cmp::Ordering::Less => Err(LinalgError::InvalidProviderArgument {
            provider: PROVIDER_NAME,
            operation,
            argument: info.unsigned_abs(),
        }),
        std::cmp::Ordering::Greater => Err(LinalgError::RankDeficient {
            provider: PROVIDER_NAME,
            operation,
            deficient_diagonal: u64::from(info.unsigned_abs()),
            required_rank,
        }),
        std::cmp::Ordering::Equal => Ok(()),
    }
}

#[cfg(not(windows))]
fn unreachable_provider<T>() -> Result<T, LinalgError> {
    Err(LinalgError::ProviderFailure {
        provider: PROVIDER_NAME,
        operation: "native call",
        detail: "a Windows-only provider cannot be constructed on this platform".to_owned(),
    })
}

fn complex32_add(left: Complex32, right: Complex32) -> Complex32 {
    Complex32::new(left.re + right.re, left.im + right.im)
}

fn complex32_multiply(left: Complex32, right: Complex32) -> Complex32 {
    Complex32::new(
        left.re * right.re - left.im * right.im,
        left.re * right.im + left.im * right.re,
    )
}

#[cfg(windows)]
type PackedComplex32 = [f32; 2];

#[cfg(windows)]
fn pack_scalar32(value: Complex32) -> PackedComplex32 {
    [value.re, value.im]
}

#[cfg(windows)]
fn try_pack_complex32(
    operation: &'static str,
    values: &[Complex32],
) -> Result<Vec<PackedComplex32>, LinalgError> {
    let mut packed = Vec::new();
    packed
        .try_reserve_exact(values.len())
        .map_err(|_| allocation_failure(operation, values.len()))?;
    packed.extend(values.iter().copied().map(pack_scalar32));
    Ok(packed)
}

#[cfg(windows)]
fn unpack_complex32(source: &[PackedComplex32], destination: &mut [Complex32]) {
    debug_assert_eq!(source.len(), destination.len());
    for (source, destination) in source.iter().zip(destination) {
        *destination = Complex32::new(source[0], source[1]);
    }
}

#[cfg(windows)]
fn try_unpack_complex32(
    operation: &'static str,
    source: &[PackedComplex32],
) -> Result<Vec<Complex32>, LinalgError> {
    let mut unpacked = Vec::new();
    unpacked
        .try_reserve_exact(source.len())
        .map_err(|_| allocation_failure(operation, source.len()))?;
    unpacked.extend(
        source
            .iter()
            .map(|value| Complex32::new(value[0], value[1])),
    );
    Ok(unpacked)
}

#[cfg(windows)]
type PackedComplex64 = [f64; 2];

#[cfg(windows)]
fn pack_scalar(value: Complex64) -> PackedComplex64 {
    [value.re, value.im]
}

#[cfg(windows)]
fn try_pack_complex(
    operation: &'static str,
    values: &[Complex64],
) -> Result<Vec<PackedComplex64>, LinalgError> {
    let mut packed = Vec::new();
    packed
        .try_reserve_exact(values.len())
        .map_err(|_| allocation_failure(operation, values.len()))?;
    packed.extend(values.iter().copied().map(pack_scalar));
    Ok(packed)
}

#[cfg(windows)]
fn unpack_complex(source: &[PackedComplex64], destination: &mut [Complex64]) {
    debug_assert_eq!(source.len(), destination.len());
    for (source, destination) in source.iter().zip(destination) {
        *destination = Complex64::new(source[0], source[1]);
    }
}

#[cfg(windows)]
fn try_unpack_complex(
    operation: &'static str,
    source: &[PackedComplex64],
) -> Result<Vec<Complex64>, LinalgError> {
    let mut unpacked = Vec::new();
    unpacked
        .try_reserve_exact(source.len())
        .map_err(|_| allocation_failure(operation, source.len()))?;
    unpacked.extend(
        source
            .iter()
            .map(|value| Complex64::new(value[0], value[1])),
    );
    Ok(unpacked)
}

fn try_copy_buffer<T: Copy>(operation: &'static str, source: &[T]) -> Result<Vec<T>, LinalgError> {
    let mut copy = Vec::new();
    copy.try_reserve_exact(source.len())
        .map_err(|_| allocation_failure(operation, source.len()))?;
    copy.extend_from_slice(source);
    Ok(copy)
}

fn try_filled_buffer<T: Copy>(
    operation: &'static str,
    length: usize,
    value: T,
) -> Result<Vec<T>, LinalgError> {
    let mut buffer = Vec::new();
    buffer
        .try_reserve_exact(length)
        .map_err(|_| allocation_failure(operation, length))?;
    buffer.resize(length, value);
    Ok(buffer)
}

fn rectangular_result_shape(dimensions: RectangularSolveDimensions) -> Result<Shape, LinalgError> {
    Shape::new([
        dimensions.coefficient_columns(),
        dimensions.right_hand_sides(),
    ])
    .map_err(Into::into)
}

fn rectangular_zero_result<T: Copy>(
    operation: &'static str,
    dimensions: RectangularSolveDimensions,
    zero: T,
) -> Result<DenseArray<T>, LinalgError> {
    let columns = usize::try_from(dimensions.coefficient_columns())
        .map_err(|_| allocation_failure(operation, usize::MAX))?;
    let right_hand_sides = usize::try_from(dimensions.right_hand_sides())
        .map_err(|_| allocation_failure(operation, usize::MAX))?;
    let length = columns
        .checked_mul(right_hand_sides)
        .ok_or_else(|| allocation_failure(operation, usize::MAX))?;
    let result = try_filled_buffer(operation, length, zero)?;
    DenseArray::from_vec(rectangular_result_shape(dimensions)?, result).map_err(Into::into)
}

#[cfg(windows)]
fn try_padded_right_hand_side<T: Copy>(
    operation: &'static str,
    dimensions: RectangularSolveDimensions,
    source: &[T],
    zero: T,
) -> Result<Vec<T>, LinalgError> {
    let rows = usize::try_from(dimensions.coefficient_rows())
        .map_err(|_| allocation_failure(operation, usize::MAX))?;
    let workspace_rows = usize::try_from(
        dimensions
            .coefficient_rows()
            .max(dimensions.coefficient_columns()),
    )
    .map_err(|_| allocation_failure(operation, usize::MAX))?;
    let right_hand_sides = usize::try_from(dimensions.right_hand_sides())
        .map_err(|_| allocation_failure(operation, usize::MAX))?;
    let workspace_len = workspace_rows
        .checked_mul(right_hand_sides)
        .ok_or_else(|| allocation_failure(operation, usize::MAX))?;
    let mut workspace = try_filled_buffer(operation, workspace_len, zero)?;
    for column in 0..right_hand_sides {
        let source_start = column * rows;
        let destination_start = column * workspace_rows;
        workspace[destination_start..destination_start + rows]
            .copy_from_slice(&source[source_start..source_start + rows]);
    }
    Ok(workspace)
}

#[cfg(windows)]
fn try_extract_rectangular_solution<T: Copy>(
    operation: &'static str,
    dimensions: RectangularSolveDimensions,
    workspace: &[T],
) -> Result<Vec<T>, LinalgError> {
    let columns = usize::try_from(dimensions.coefficient_columns())
        .map_err(|_| allocation_failure(operation, usize::MAX))?;
    let workspace_rows = usize::try_from(
        dimensions
            .coefficient_rows()
            .max(dimensions.coefficient_columns()),
    )
    .map_err(|_| allocation_failure(operation, usize::MAX))?;
    let right_hand_sides = usize::try_from(dimensions.right_hand_sides())
        .map_err(|_| allocation_failure(operation, usize::MAX))?;
    let result_len = columns
        .checked_mul(right_hand_sides)
        .ok_or_else(|| allocation_failure(operation, usize::MAX))?;
    let mut result = Vec::new();
    result
        .try_reserve_exact(result_len)
        .map_err(|_| allocation_failure(operation, result_len))?;
    for column in 0..right_hand_sides {
        let start = column * workspace_rows;
        result.extend_from_slice(&workspace[start..start + columns]);
    }
    Ok(result)
}

fn independent_array_copy<T: Copy>(
    operation: &'static str,
    source: &DenseArray<T>,
) -> Result<DenseArray<T>, LinalgError> {
    let data = try_copy_buffer(operation, source.as_slice())?;
    DenseArray::from_vec(source.shape().clone(), data).map_err(Into::into)
}

fn allocation_failure(operation: &'static str, elements: usize) -> LinalgError {
    LinalgError::AllocationFailure {
        operation,
        elements: u64::try_from(elements).unwrap_or(u64::MAX),
    }
}

#[cfg(windows)]
fn try_allocate_pivots(n: i32) -> Result<Vec<i32>, LinalgError> {
    try_allocate_i32("general linear solve pivot buffer", n)
}

#[cfg(windows)]
fn try_allocate_i32(operation: &'static str, length: i32) -> Result<Vec<i32>, LinalgError> {
    let length = usize::try_from(length).map_err(|_| LinalgError::ProviderFailure {
        provider: PROVIDER_NAME,
        operation,
        detail: format!("validated LP64 length {length} is negative"),
    })?;
    try_filled_buffer(operation, length, 0)
}

#[cfg(any(windows, test))]
struct ParsedConfig {
    version: String,
    integer_abi: OpenBlasIntegerAbi,
}

#[cfg(any(windows, test))]
fn parse_config(config: &str) -> Result<ParsedConfig, OpenBlasError> {
    let mut tokens = config.split_ascii_whitespace();
    let product = tokens
        .next()
        .ok_or_else(|| OpenBlasError::MalformedConfig {
            config: config.to_owned(),
            reason: "configuration is empty",
        })?;
    if !product.eq_ignore_ascii_case("OpenBLAS") {
        return Err(OpenBlasError::MalformedConfig {
            config: config.to_owned(),
            reason: "configuration does not begin with OpenBLAS",
        });
    }
    let version = tokens
        .next()
        .filter(|version| !version.is_empty())
        .ok_or_else(|| OpenBlasError::MalformedConfig {
            config: config.to_owned(),
            reason: "configuration does not contain a version",
        })?
        .to_owned();
    let integer_abi = if config.split_ascii_whitespace().any(token_selects_ilp64) {
        OpenBlasIntegerAbi::Ilp64
    } else {
        OpenBlasIntegerAbi::Lp64
    };
    Ok(ParsedConfig {
        version,
        integer_abi,
    })
}

#[cfg(any(windows, test))]
fn token_selects_ilp64(token: &str) -> bool {
    let (name, value) = token
        .split_once('=')
        .map_or((token, None), |(name, value)| (name, Some(value)));
    let known_name = name.eq_ignore_ascii_case("USE64BITINT")
        || name.eq_ignore_ascii_case("INTERFACE64")
        || name.eq_ignore_ascii_case("OPENBLAS_USE64BITINT");
    known_name
        && value.is_none_or(|value| {
            !value.eq_ignore_ascii_case("0")
                && !value.eq_ignore_ascii_case("false")
                && !value.eq_ignore_ascii_case("no")
                && !value.eq_ignore_ascii_case("off")
        })
}

#[cfg(any(windows, test))]
fn require_lp64(config: &str, integer_abi: OpenBlasIntegerAbi) -> Result<(), OpenBlasError> {
    if integer_abi == OpenBlasIntegerAbi::Lp64 {
        Ok(())
    } else {
        Err(OpenBlasError::IncompatibleIntegerAbi {
            config: config.to_owned(),
            detected: integer_abi,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{OpenBlasError, OpenBlasIntegerAbi, parse_config, require_lp64};

    #[cfg(windows)]
    use super::{
        check_gels_info, check_lapack_info, check_spectral_info, eig_real64_native,
        normalize_column_pivots, normalize_factor_info, normalize_row_pivots, qr_native,
        schur_complex64_native, svd_real_native,
    };
    #[cfg(windows)]
    use openmat_array::{DenseArray, Shape};
    #[cfg(windows)]
    use openmat_linalg::{
        CholeskyTriangle, EigRequest, LinalgError, QrRequest, QrVectors, SchurRequest, SvdRequest,
        SvdVectors, SwapParity,
    };

    #[test]
    fn parses_lp64_config_and_version() {
        let parsed =
            parse_config("OpenBLAS 0.3.31 DYNAMIC_ARCH NO_AFFINITY SkylakeX MAX_THREADS=64")
                .unwrap();
        assert_eq!(parsed.version, "0.3.31");
        assert_eq!(parsed.integer_abi, OpenBlasIntegerAbi::Lp64);
    }

    #[test]
    fn detects_all_supported_ilp64_markers() {
        for marker in ["USE64BITINT", "INTERFACE64", "OPENBLAS_USE64BITINT=1"] {
            let parsed = parse_config(&format!("OpenBLAS 0.3.31 {marker} DYNAMIC_ARCH")).unwrap();
            assert_eq!(parsed.integer_abi, OpenBlasIntegerAbi::Ilp64);
        }
    }

    #[test]
    fn explicit_disabled_markers_remain_lp64() {
        for marker in ["USE64BITINT=0", "INTERFACE64=false"] {
            let parsed = parse_config(&format!("OpenBLAS 0.3.31 {marker}")).unwrap();
            assert_eq!(parsed.integer_abi, OpenBlasIntegerAbi::Lp64);
        }
    }

    #[test]
    fn ilp64_is_structurally_rejected() {
        let config = "OpenBLAS 0.3.31 USE64BITINT DYNAMIC_ARCH";
        let parsed = parse_config(config).unwrap();
        assert_eq!(
            require_lp64(config, parsed.integer_abi),
            Err(OpenBlasError::IncompatibleIntegerAbi {
                config: config.to_owned(),
                detected: OpenBlasIntegerAbi::Ilp64,
            })
        );
    }

    #[test]
    fn malformed_config_is_typed() {
        assert!(matches!(
            parse_config("not-openblas 1.0"),
            Err(OpenBlasError::MalformedConfig {
                reason: "configuration does not begin with OpenBLAS",
                ..
            })
        ));
        assert!(matches!(
            parse_config("OpenBLAS"),
            Err(OpenBlasError::MalformedConfig {
                reason: "configuration does not contain a version",
                ..
            })
        ));
    }

    #[cfg(windows)]
    #[test]
    fn lapack_info_is_mapped_without_losing_argument_or_pivot() {
        assert_eq!(check_lapack_info("test solve", 0), Ok(()));
        assert_eq!(
            check_lapack_info("test solve", -4),
            Err(LinalgError::InvalidProviderArgument {
                provider: "openblas",
                operation: "test solve",
                argument: 4,
            })
        );
        assert_eq!(
            check_lapack_info("test solve", 3),
            Err(LinalgError::SingularMatrix {
                provider: "openblas",
                operation: "test solve",
                pivot: 3,
            })
        );
        assert_eq!(
            check_gels_info("test rectangular solve", 2, 4),
            Err(LinalgError::RankDeficient {
                provider: "openblas",
                operation: "test rectangular solve",
                deficient_diagonal: 2,
                required_rank: 4,
            })
        );
    }

    #[cfg(windows)]
    #[test]
    fn decomposition_pivots_and_status_are_normalized_to_zero_based_data() {
        let (permutation, parity) = normalize_row_pivots("test LU", 4, &[3, 2, 4]).unwrap();
        assert_eq!(permutation, vec![2, 1, 3, 0]);
        assert_eq!(parity, SwapParity::Even);
        assert_eq!(
            normalize_column_pivots("test QR", 4, &[3, 1, 4, 2]).unwrap(),
            vec![2, 0, 3, 1]
        );
        assert_eq!(normalize_factor_info("test factor", 0, 4), Ok(None));
        assert_eq!(normalize_factor_info("test factor", 3, 4), Ok(Some(2)));
    }

    #[cfg(windows)]
    #[test]
    fn malformed_decomposition_pivots_remain_structured_provider_failures() {
        assert!(matches!(
            normalize_row_pivots("test LU", 3, &[0, 2]),
            Err(LinalgError::ProviderFailure {
                provider: "openblas",
                operation: "test LU",
                ..
            })
        ));
        assert!(matches!(
            normalize_column_pivots("test QR", 3, &[1, 1, 3]),
            Err(LinalgError::ProviderFailure {
                provider: "openblas",
                operation: "test QR",
                ..
            })
        ));
        assert!(matches!(
            normalize_factor_info("test factor", 4, 3),
            Err(LinalgError::ProviderFailure {
                provider: "openblas",
                operation: "test factor",
                ..
            })
        ));
    }

    #[cfg(windows)]
    #[test]
    fn native_qr_empty_columns_build_full_identity_without_calling_lapack() {
        let matrix = DenseArray::from_vec(Shape::new([3, 0]).unwrap(), Vec::<f64>::new()).unwrap();
        let result = qr_native(
            QrRequest::new(&matrix, QrVectors::Full),
            "test QR",
            0.0_f64,
            1.0_f64,
            f64::EPSILON,
            f64::abs,
            |_, _, _, _, _| panic!("empty QR must not call factorization"),
            |_, _, _, _| panic!("empty QR must not generate reflectors"),
        )
        .unwrap();
        assert_eq!(result.q.shape().dimensions(), &[3, 3]);
        assert_eq!(
            result.q.as_slice(),
            &[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]
        );
        assert_eq!(result.r.shape().dimensions(), &[3, 0]);
        assert_eq!(result.first_rank_deficient_diagonal, None);
    }

    #[cfg(windows)]
    #[test]
    fn cholesky_normalization_discards_native_data_outside_successful_block() {
        let normalized = super::normalize_cholesky_factor(
            "test Cholesky",
            &[1_i32, 2, 3, 4, 5, 6, 7, 8, 9],
            3,
            CholeskyTriangle::Upper,
            Some(1),
            0,
        )
        .unwrap();
        assert_eq!(normalized, vec![1, 0, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[cfg(windows)]
    #[test]
    fn native_svd_helper_preserves_cow_and_empty_full_shapes() {
        let matrix = DenseArray::from_vec(Shape::new([2, 1]).unwrap(), vec![3.0_f64, 4.0]).unwrap();
        let shared = matrix.clone();
        let result = svd_real_native(
            SvdRequest::new(&matrix, SvdVectors::Thin),
            0.0,
            1.0,
            "test SVD",
            |_, _, a, s, u, _, vt, _| {
                assert_eq!(a, &[3.0, 4.0]);
                s[0] = 5.0;
                u.copy_from_slice(&[0.6, 0.8]);
                vt[0] = 1.0;
                Ok(0)
            },
        )
        .unwrap();
        assert_eq!(result.singular_values.as_slice(), &[5.0]);
        assert!(matrix.shares_storage_with(&shared));

        let empty = DenseArray::from_vec(Shape::new([0, 2]).unwrap(), Vec::<f64>::new()).unwrap();
        let full = svd_real_native(
            SvdRequest::new(&empty, SvdVectors::Full),
            0.0,
            1.0,
            "test empty SVD",
            |_, _, _, _, _, _, _, _| panic!("empty SVD must not call LAPACK"),
        )
        .unwrap();
        assert_eq!(full.u.unwrap().shape().dimensions(), &[0, 0]);
        assert_eq!(full.vh.unwrap().as_slice(), &[1.0, 0.0, 0.0, 1.0]);

        let cancellation = std::sync::atomic::AtomicBool::new(false);
        let cancelled = svd_real_native(
            SvdRequest::new(&matrix, SvdVectors::Thin).with_cancellation_flag(&cancellation),
            0.0,
            1.0,
            "test cancelled SVD",
            |_, _, _, _, _, _, _, _| {
                cancellation.store(true, std::sync::atomic::Ordering::Release);
                Ok(0)
            },
        );
        assert!(matches!(cancelled, Err(LinalgError::Cancelled { .. })));
    }

    #[cfg(windows)]
    #[test]
    fn native_schur_helper_preserves_factors_and_structured_status() {
        let matrix = DenseArray::from_vec(
            Shape::new([2, 2]).unwrap(),
            vec![
                openmat_array::Complex64::new(1.0, 0.0),
                openmat_array::Complex64::new(3.0, 0.0),
                openmat_array::Complex64::new(2.0, 0.0),
                openmat_array::Complex64::new(4.0, 0.0),
            ],
        )
        .unwrap();
        let result = schur_complex64_native(
            SchurRequest::new(&matrix),
            |dimensions, form, values, vectors| {
                assert_eq!(
                    [dimensions.m, dimensions.n, dimensions.k, dimensions.lda],
                    [2; 4]
                );
                form.copy_from_slice(&[[1.0, 0.0], [0.0, 0.0], [2.0, 0.0], [4.0, 0.0]]);
                values.copy_from_slice(&[[1.0, 0.0], [4.0, 0.0]]);
                vectors.copy_from_slice(&[[1.0, 0.0], [0.0, 0.0], [0.0, 0.0], [1.0, 0.0]]);
                Ok((0, 0))
            },
        )
        .unwrap();
        assert_eq!(result.form.as_slice()[1], openmat_array::Complex64::ZERO);
        assert_eq!(
            result.vectors.as_slice()[0],
            openmat_array::Complex64::new(1.0, 0.0)
        );

        assert!(matches!(
            schur_complex64_native(SchurRequest::new(&matrix), |_, _, _, _| Ok((2, 0))),
            Err(LinalgError::NoConvergence {
                unconverged: Some(2),
                ..
            })
        ));
        assert!(matches!(
            schur_complex64_native(SchurRequest::new(&matrix), |_, _, _, _| Ok((0, 1))),
            Err(LinalgError::ProviderFailure {
                provider: "openblas",
                ..
            })
        ));
    }

    #[cfg(windows)]
    #[test]
    #[allow(clippy::float_cmp)]
    fn real_geev_conjugate_pairs_are_normalized_to_explicit_complex_columns() {
        let matrix =
            DenseArray::from_vec(Shape::new([2, 2]).unwrap(), vec![0.0_f64, 1.0, -1.0, 0.0])
                .unwrap();
        let result = eig_real64_native(
            EigRequest::new(&matrix)
                .with_left_vectors(true)
                .with_right_vectors(true),
            |_, _, _, _, wr, wi, vl, vr| {
                wr.copy_from_slice(&[0.0, 0.0]);
                wi.copy_from_slice(&[1.0, -1.0]);
                vl.copy_from_slice(&[1.0, 0.0, 0.0, 1.0]);
                vr.copy_from_slice(&[1.0, 0.0, 0.0, 1.0]);
                Ok(0)
            },
        )
        .unwrap();
        assert_eq!(result.eigenvalues.as_slice()[0].im, 1.0);
        assert_eq!(result.eigenvalues.as_slice()[1].im, -1.0);
        let right = result.right_vectors.unwrap();
        assert_eq!(right.as_slice()[0], openmat_array::Complex64::new(1.0, 0.0));
        assert_eq!(right.as_slice()[1], openmat_array::Complex64::new(0.0, 1.0));
        assert_eq!(
            right.as_slice()[2],
            openmat_array::Complex64::new(1.0, -0.0)
        );
        assert_eq!(
            right.as_slice()[3],
            openmat_array::Complex64::new(0.0, -1.0)
        );

        let cancellation = std::sync::atomic::AtomicBool::new(false);
        let cancelled = eig_real64_native(
            EigRequest::new(&matrix).with_cancellation_flag(&cancellation),
            |_, _, _, _, _, _, _, _| {
                cancellation.store(true, std::sync::atomic::Ordering::Release);
                Ok(0)
            },
        );
        assert!(matches!(cancelled, Err(LinalgError::Cancelled { .. })));
    }

    #[cfg(windows)]
    #[test]
    fn spectral_info_normalizes_nonconvergence_and_lapack_allocation_failure() {
        assert!(matches!(
            check_spectral_info("test eig", 3),
            Err(LinalgError::NoConvergence {
                unconverged: Some(3),
                ..
            })
        ));
        assert!(matches!(
            check_spectral_info("test SVD", -1010),
            Err(LinalgError::AllocationFailure { .. })
        ));
        assert!(matches!(
            check_spectral_info("test eig", -4),
            Err(LinalgError::InvalidProviderArgument { argument: 4, .. })
        ));
    }
}
