#![doc = "Numerical-provider boundary and linear algebra operations."]
#![forbid(unsafe_code)]

mod decomposition;
mod error;
mod lp64;
mod matrix_function;
mod provider;
mod rectangular_solve;
mod reference;
mod reference_spectral;
mod schur;
mod solve;
mod sparse_provider;
mod spectral;
mod underdetermined_basic_solve;

pub use decomposition::{
    CholeskyDimensions, CholeskyRequest, CholeskyResult, CholeskyTriangle, FactorDimensions,
    FactorRequest, LuResult, QrRequest, QrResult, QrVectors, SwapParity, cholesky_complex32,
    cholesky_complex64, cholesky_f32, cholesky_f64, factor_lu_complex32, factor_lu_complex64,
    factor_lu_f32, factor_lu_f64, qr_complex32, qr_complex64, qr_f32, qr_f64, validate_cholesky,
    validate_factor, validate_qr,
};
pub use error::LinalgError;
pub use lp64::{
    LP64_MAX_DIMENSION, Lp64CholeskyDimensions, Lp64FactorDimensions, Lp64GemmDimensions,
    Lp64RectangularSolveDimensions, Lp64SolveDimensions, Lp64SpectralDimensions,
    checked_lp64_dimension,
};
pub use matrix_function::{
    MatrixFunctionRequest, MatrixSquareRootResult, matrix_power_complex32, matrix_power_complex64,
    matrix_power_f32, matrix_power_f64, matrix_sqrt_complex32, matrix_sqrt_complex64,
    matrix_sqrt_f32, matrix_sqrt_f64,
};
pub use provider::{
    GemmDimensions, GemmOptions, GemmRequest, LinalgProvider, MatrixTranspose, ProviderIntegerAbi,
    matrix_dimensions, matrix_multiply_complex32, matrix_multiply_complex64, matrix_multiply_f32,
    matrix_multiply_f64, validate_gemm,
};
pub use rectangular_solve::{
    RectangularSolveDimensions, RectangularSolveKind, RectangularSolveRequest,
    solve_rectangular_complex32, solve_rectangular_complex64, solve_rectangular_f32,
    solve_rectangular_f64, validate_rectangular_solve,
};
pub use reference::ReferenceProvider;
pub use schur::{SchurRequest, SchurResult, schur_complex32, schur_complex64, validate_schur};
pub use solve::{
    SolveDimensions, SolveRequest, solve_complex32, solve_complex64, solve_f32, solve_f64,
    validate_solve,
};
pub use sparse_provider::{
    CscMatrixRef, CscPatternRef, OwnedCscMatrix, SparseCancellation, SparseCapabilities,
    SparseCholeskyFactor, SparseError, SparseFactorKind, SparseIndexWidth, SparseLuFactor,
    SparseMetrics, SparseNumericFactor, SparsePhase, SparseProvider, SparseQrFactor,
    SparseSolveResult, SparseSymbolicFactor,
};
pub use spectral::{
    EigRequest, EigResult, SpectralDimensions, SvdRequest, SvdResult, SvdVectors, eig_complex32,
    eig_complex64, eig_f32, eig_f64, svd_complex32, svd_complex64, svd_f32, svd_f64, validate_eig,
    validate_svd,
};
pub use underdetermined_basic_solve::{
    solve_underdetermined_basic_complex32, solve_underdetermined_basic_complex64,
    solve_underdetermined_basic_f32, solve_underdetermined_basic_f64,
};
