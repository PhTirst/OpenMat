use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use openmat_array::{
    ArrayData, Complex32, Complex64 as ArrayComplex64, DenseArray, Logical, Shape,
};
use openmat_bytecode::{
    ApplyArgument, BinaryOperator, BytecodeModule, Constant, ConstantId, Function, FunctionId,
    Instruction, InstructionKind, LocalSlot, Register, SourceLocation,
};
use openmat_linalg::{
    FactorRequest, GemmRequest, LinalgError, LinalgProvider, LuResult, ProviderIntegerAbi,
    RectangularSolveRequest, ReferenceProvider, SolveRequest,
};
use openmat_runtime::{
    ArrayRuntimeError, BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinRegistry,
    Interpreter, NullOutput, RuntimeErrorKind, RuntimeLinalgError, linalg_matrix_left_divide,
    linalg_matrix_multiply, linalg_matrix_right_divide, linalg_square_solve,
};
use openmat_value::Value;

#[derive(Clone, Copy)]
enum ProbeOutcome {
    Success,
    ProviderFailure,
    Cancelled,
}

struct ProbeProvider {
    outcome: ProbeOutcome,
    real_gemm_calls: AtomicUsize,
    complex_gemm_calls: AtomicUsize,
    real_gemm_f32_calls: AtomicUsize,
    complex_gemm_f32_calls: AtomicUsize,
    real_solve_calls: AtomicUsize,
    complex_solve_calls: AtomicUsize,
    real_solve_f32_calls: AtomicUsize,
    complex_solve_f32_calls: AtomicUsize,
    real_rectangular_calls: AtomicUsize,
    complex_rectangular_calls: AtomicUsize,
    real_rectangular_f32_calls: AtomicUsize,
    complex_rectangular_f32_calls: AtomicUsize,
    real_lu_calls: AtomicUsize,
    saw_zero_imaginary_left: AtomicBool,
    saw_zero_imaginary_f32_left: AtomicBool,
    cancel_during_solve: Option<Arc<AtomicBool>>,
}

impl ProbeProvider {
    fn new(outcome: ProbeOutcome) -> Self {
        Self {
            outcome,
            real_gemm_calls: AtomicUsize::new(0),
            complex_gemm_calls: AtomicUsize::new(0),
            real_gemm_f32_calls: AtomicUsize::new(0),
            complex_gemm_f32_calls: AtomicUsize::new(0),
            real_solve_calls: AtomicUsize::new(0),
            complex_solve_calls: AtomicUsize::new(0),
            real_solve_f32_calls: AtomicUsize::new(0),
            complex_solve_f32_calls: AtomicUsize::new(0),
            real_rectangular_calls: AtomicUsize::new(0),
            complex_rectangular_calls: AtomicUsize::new(0),
            real_rectangular_f32_calls: AtomicUsize::new(0),
            complex_rectangular_f32_calls: AtomicUsize::new(0),
            real_lu_calls: AtomicUsize::new(0),
            saw_zero_imaginary_left: AtomicBool::new(false),
            saw_zero_imaginary_f32_left: AtomicBool::new(false),
            cancel_during_solve: None,
        }
    }

    fn with_solve_cancellation(mut self, flag: Arc<AtomicBool>) -> Self {
        self.cancel_during_solve = Some(flag);
        self
    }

    fn gemm_failure(&self) -> Option<LinalgError> {
        match self.outcome {
            ProbeOutcome::Success => None,
            ProbeOutcome::ProviderFailure => Some(LinalgError::ProviderFailure {
                provider: "probe",
                operation: "matrix multiplication",
                detail: "injected GEMM failure".to_owned(),
            }),
            ProbeOutcome::Cancelled => Some(LinalgError::Cancelled {
                operation: "matrix multiplication",
            }),
        }
    }

    fn solve_failure(&self, operation: &'static str) -> Option<LinalgError> {
        match self.outcome {
            ProbeOutcome::Success => None,
            ProbeOutcome::ProviderFailure => Some(LinalgError::ProviderFailure {
                provider: "probe",
                operation,
                detail: "injected solve failure".to_owned(),
            }),
            ProbeOutcome::Cancelled => Some(LinalgError::Cancelled { operation }),
        }
    }
}

impl LinalgProvider for ProbeProvider {
    fn name(&self) -> &'static str {
        "probe"
    }

    fn integer_abi(&self) -> ProviderIntegerAbi {
        ProviderIntegerAbi::RustU64
    }

    fn gemm_f64(
        &self,
        request: GemmRequest<'_, f64>,
        output: &mut DenseArray<f64>,
    ) -> Result<(), LinalgError> {
        self.real_gemm_calls.fetch_add(1, Ordering::Relaxed);
        if let Some(error) = self.gemm_failure() {
            return Err(error);
        }
        ReferenceProvider.gemm_f64(request, output)
    }

    fn gemm_f32(
        &self,
        request: GemmRequest<'_, f32>,
        output: &mut DenseArray<f32>,
    ) -> Result<(), LinalgError> {
        self.real_gemm_f32_calls.fetch_add(1, Ordering::Relaxed);
        if let Some(error) = self.gemm_failure() {
            return Err(error);
        }
        ReferenceProvider.gemm_f32(request, output)
    }

    fn gemm_complex64(
        &self,
        request: GemmRequest<'_, ArrayComplex64>,
        output: &mut DenseArray<ArrayComplex64>,
    ) -> Result<(), LinalgError> {
        self.complex_gemm_calls.fetch_add(1, Ordering::Relaxed);
        self.saw_zero_imaginary_left.store(
            request
                .left
                .as_slice()
                .iter()
                .all(|value| value.im.to_bits() == 0.0_f64.to_bits()),
            Ordering::Relaxed,
        );
        if let Some(error) = self.gemm_failure() {
            return Err(error);
        }
        ReferenceProvider.gemm_complex64(request, output)
    }

    fn gemm_complex32(
        &self,
        request: GemmRequest<'_, Complex32>,
        output: &mut DenseArray<Complex32>,
    ) -> Result<(), LinalgError> {
        self.complex_gemm_f32_calls.fetch_add(1, Ordering::Relaxed);
        self.saw_zero_imaginary_f32_left.store(
            request
                .left
                .as_slice()
                .iter()
                .all(|value| value.im.to_bits() == 0.0_f32.to_bits()),
            Ordering::Relaxed,
        );
        if let Some(error) = self.gemm_failure() {
            return Err(error);
        }
        ReferenceProvider.gemm_complex32(request, output)
    }

    fn dot_f64(&self, left: &DenseArray<f64>, right: &DenseArray<f64>) -> Result<f64, LinalgError> {
        ReferenceProvider.dot_f64(left, right)
    }

    fn norm2_f64(&self, input: &DenseArray<f64>) -> Result<f64, LinalgError> {
        ReferenceProvider.norm2_f64(input)
    }

    fn solve_f64(&self, request: SolveRequest<'_, f64>) -> Result<DenseArray<f64>, LinalgError> {
        self.real_solve_calls.fetch_add(1, Ordering::Relaxed);
        if let Some(flag) = &self.cancel_during_solve {
            flag.store(true, Ordering::Release);
            request.check_cancellation()?;
        }
        if let Some(error) = self.solve_failure("general linear solve") {
            return Err(error);
        }
        ReferenceProvider.solve_f64(request)
    }

    fn solve_f32(&self, request: SolveRequest<'_, f32>) -> Result<DenseArray<f32>, LinalgError> {
        self.real_solve_f32_calls.fetch_add(1, Ordering::Relaxed);
        if let Some(flag) = &self.cancel_during_solve {
            flag.store(true, Ordering::Release);
            request.check_cancellation()?;
        }
        if let Some(error) = self.solve_failure("general linear solve") {
            return Err(error);
        }
        ReferenceProvider.solve_f32(request)
    }

    fn solve_complex64(
        &self,
        request: SolveRequest<'_, ArrayComplex64>,
    ) -> Result<DenseArray<ArrayComplex64>, LinalgError> {
        self.complex_solve_calls.fetch_add(1, Ordering::Relaxed);
        if let Some(flag) = &self.cancel_during_solve {
            flag.store(true, Ordering::Release);
            request.check_cancellation()?;
        }
        if let Some(error) = self.solve_failure("general linear solve") {
            return Err(error);
        }
        ReferenceProvider.solve_complex64(request)
    }

    fn solve_complex32(
        &self,
        request: SolveRequest<'_, Complex32>,
    ) -> Result<DenseArray<Complex32>, LinalgError> {
        self.complex_solve_f32_calls.fetch_add(1, Ordering::Relaxed);
        if let Some(flag) = &self.cancel_during_solve {
            flag.store(true, Ordering::Release);
            request.check_cancellation()?;
        }
        if let Some(error) = self.solve_failure("general linear solve") {
            return Err(error);
        }
        ReferenceProvider.solve_complex32(request)
    }

    fn solve_rectangular_f64(
        &self,
        request: RectangularSolveRequest<'_, f64>,
    ) -> Result<DenseArray<f64>, LinalgError> {
        self.real_rectangular_calls.fetch_add(1, Ordering::Relaxed);
        if let Some(flag) = &self.cancel_during_solve {
            flag.store(true, Ordering::Release);
            request.check_cancellation()?;
        }
        if let Some(error) = self.solve_failure("rectangular linear solve") {
            return Err(error);
        }
        ReferenceProvider.solve_rectangular_f64(request)
    }

    fn solve_rectangular_f32(
        &self,
        request: RectangularSolveRequest<'_, f32>,
    ) -> Result<DenseArray<f32>, LinalgError> {
        self.real_rectangular_f32_calls
            .fetch_add(1, Ordering::Relaxed);
        if let Some(flag) = &self.cancel_during_solve {
            flag.store(true, Ordering::Release);
            request.check_cancellation()?;
        }
        if let Some(error) = self.solve_failure("rectangular linear solve") {
            return Err(error);
        }
        ReferenceProvider.solve_rectangular_f32(request)
    }

    fn solve_rectangular_complex64(
        &self,
        request: RectangularSolveRequest<'_, ArrayComplex64>,
    ) -> Result<DenseArray<ArrayComplex64>, LinalgError> {
        self.complex_rectangular_calls
            .fetch_add(1, Ordering::Relaxed);
        if let Some(flag) = &self.cancel_during_solve {
            flag.store(true, Ordering::Release);
            request.check_cancellation()?;
        }
        if let Some(error) = self.solve_failure("rectangular linear solve") {
            return Err(error);
        }
        ReferenceProvider.solve_rectangular_complex64(request)
    }

    fn solve_rectangular_complex32(
        &self,
        request: RectangularSolveRequest<'_, Complex32>,
    ) -> Result<DenseArray<Complex32>, LinalgError> {
        self.complex_rectangular_f32_calls
            .fetch_add(1, Ordering::Relaxed);
        if let Some(flag) = &self.cancel_during_solve {
            flag.store(true, Ordering::Release);
            request.check_cancellation()?;
        }
        if let Some(error) = self.solve_failure("rectangular linear solve") {
            return Err(error);
        }
        ReferenceProvider.solve_rectangular_complex32(request)
    }

    fn factor_lu_f64(&self, request: FactorRequest<'_, f64>) -> Result<LuResult<f64>, LinalgError> {
        self.real_lu_calls.fetch_add(1, Ordering::Relaxed);
        ReferenceProvider.factor_lu_f64(request)
    }
}

fn binary_module(operator: BinaryOperator, location: Option<SourceLocation>) -> BytecodeModule {
    let binary = InstructionKind::Binary {
        operator,
        dst: Register::new(2),
        lhs: Register::new(0),
        rhs: Register::new(1),
    };
    let binary = match location {
        Some(location) => Instruction::located(binary, location),
        None => Instruction::new(binary),
    };
    BytecodeModule::new(
        vec![Function {
            name: "binary".to_owned(),
            register_count: 3,
            pack_register_count: 0,
            local_count: 2,
            persistent_slot_count: 0,
            parameter_count: 2,
            argument_layout: None,
            constants: Vec::new(),
            instructions: vec![
                Instruction::new(InstructionKind::LoadLocal {
                    dst: Register::new(0),
                    local: LocalSlot::new(0),
                }),
                Instruction::new(InstructionKind::LoadLocal {
                    dst: Register::new(1),
                    local: LocalSlot::new(1),
                }),
                binary,
                Instruction::new(InstructionKind::Return {
                    values: vec![Register::new(2)],
                }),
            ],
            exception_handlers: Vec::new(),
        }],
        FunctionId::new(0),
    )
}

fn real(dimensions: impl IntoIterator<Item = u64>, values: Vec<f64>) -> Value {
    Value::Array(ArrayData::F64(
        DenseArray::from_vec(Shape::new(dimensions).unwrap(), values).unwrap(),
    ))
}

fn complex(dimensions: impl IntoIterator<Item = u64>, values: Vec<ArrayComplex64>) -> Value {
    Value::Array(ArrayData::ComplexF64(
        DenseArray::from_vec(Shape::new(dimensions).unwrap(), values).unwrap(),
    ))
}

fn single_complex(dimensions: impl IntoIterator<Item = u64>, values: Vec<Complex32>) -> Value {
    Value::Array(ArrayData::ComplexF32(
        DenseArray::from_vec(Shape::new(dimensions).unwrap(), values).unwrap(),
    ))
}

fn single_real(dimensions: impl IntoIterator<Item = u64>, values: Vec<f32>) -> Value {
    Value::Array(ArrayData::F32(
        DenseArray::from_vec(Shape::new(dimensions).unwrap(), values).unwrap(),
    ))
}

fn real_array(value: &Value) -> &DenseArray<f64> {
    let Value::Array(ArrayData::F64(array)) = value else {
        panic!("expected a real double array, received {value:?}");
    };
    array
}

fn complex_array(value: &Value) -> &DenseArray<ArrayComplex64> {
    let Value::Array(ArrayData::ComplexF64(array)) = value else {
        panic!("expected a complex double array, received {value:?}");
    };
    array
}

fn single_complex_array(value: &Value) -> &DenseArray<Complex32> {
    let Value::Array(ArrayData::ComplexF32(array)) = value else {
        panic!("expected a complex single array, received {value:?}");
    };
    array
}

fn single_real_array(value: &Value) -> &DenseArray<f32> {
    let Value::Array(ArrayData::F32(array)) = value else {
        panic!("expected a real single array, received {value:?}");
    };
    array
}

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 1.0e-12,
        "expected {expected}, received {actual}"
    );
}

fn assert_complex_close(actual: ArrayComplex64, expected: ArrayComplex64) {
    assert_close(actual.re, expected.re);
    assert_close(actual.im, expected.im);
}

#[test]
fn real_gemm_preserves_input_cow_and_returns_independent_storage() {
    let provider = ReferenceProvider;
    let left = real([2, 3], vec![1.0, 4.0, 2.0, 5.0, 3.0, 6.0]);
    let right = real([3, 2], vec![7.0, 9.0, 11.0, 8.0, 10.0, 12.0]);
    let left_alias = left.clone();
    let right_alias = right.clone();

    let result = linalg_matrix_multiply(&provider, &left, &right).unwrap();

    assert_eq!(real_array(&result).shape().dimensions(), &[2, 2]);
    assert_eq!(real_array(&result).as_slice(), &[58.0, 139.0, 64.0, 154.0]);
    assert!(left.shares_storage_with(&left_alias));
    assert!(right.shares_storage_with(&right_alias));
    assert!(!result.shares_storage_with(&left));
    assert!(!result.shares_storage_with(&right));
}

#[test]
fn mixed_real_complex_gemm_promotes_exactly_and_accepts_scalar_values() {
    let provider = ReferenceProvider;
    let matrix = complex(
        [2, 1],
        vec![
            ArrayComplex64::new(3.0, 1.0),
            ArrayComplex64::new(4.0, -2.0),
        ],
    );

    let matrix_result =
        linalg_matrix_multiply(&provider, &real([1, 2], vec![1.0, 2.0]), &matrix).unwrap();
    assert_eq!(complex_array(&matrix_result).shape().dimensions(), &[1, 1]);
    assert_complex_close(
        complex_array(&matrix_result).as_slice()[0],
        ArrayComplex64::new(11.0, -3.0),
    );

    let scalar_result =
        linalg_matrix_multiply(&provider, &Value::Double(2.0), &Value::Double(4.0)).unwrap();
    assert_eq!(real_array(&scalar_result).as_slice(), &[8.0]);
}

#[test]
fn single_gemm_uses_native_real_and_complex32_provider_methods() {
    let provider = ProbeProvider::new(ProbeOutcome::Success);
    let left = single_real([1, 2], vec![1.0, 2.0]);
    let right = single_real([2, 1], vec![3.0, 4.0]);
    let left_alias = left.clone();
    let right_alias = right.clone();

    let real_result = linalg_matrix_multiply(&provider, &left, &right).unwrap();
    assert_eq!(single_real_array(&real_result).as_slice(), &[11.0]);
    assert_eq!(provider.real_gemm_f32_calls.load(Ordering::Relaxed), 1);
    assert_eq!(provider.real_gemm_calls.load(Ordering::Relaxed), 0);
    assert!(left.shares_storage_with(&left_alias));
    assert!(right.shares_storage_with(&right_alias));
    assert!(!real_result.shares_storage_with(&left));
    assert!(!real_result.shares_storage_with(&right));

    let complex_result = linalg_matrix_multiply(
        &provider,
        &single_real([1, 2], vec![1.0, 2.0]),
        &single_complex(
            [2, 1],
            vec![Complex32::new(3.0, 1.0), Complex32::new(4.0, -2.0)],
        ),
    )
    .unwrap();
    assert_eq!(
        single_complex_array(&complex_result).as_slice(),
        &[Complex32::new(11.0, -3.0)]
    );
    assert_eq!(provider.complex_gemm_f32_calls.load(Ordering::Relaxed), 1);
    assert_eq!(provider.complex_gemm_calls.load(Ordering::Relaxed), 0);
    assert!(provider.saw_zero_imaginary_f32_left.load(Ordering::Relaxed));
}

#[test]
fn real_square_solve_supports_multiple_right_hand_sides() {
    let provider = ReferenceProvider;
    let coefficients = real([2, 2], vec![3.0, 1.0, 1.0, 2.0]);
    let right_hand_side = real([2, 2], vec![9.0, 8.0, 1.0, 7.0]);
    let coefficient_alias = coefficients.clone();
    let right_hand_side_alias = right_hand_side.clone();

    let solution = linalg_square_solve(&provider, &coefficients, &right_hand_side, None).unwrap();

    for (&actual, expected) in real_array(&solution)
        .as_slice()
        .iter()
        .zip([2.0, 3.0, -1.0, 4.0])
    {
        assert_close(actual, expected);
    }
    assert!(coefficients.shares_storage_with(&coefficient_alias));
    assert!(right_hand_side.shares_storage_with(&right_hand_side_alias));
    assert!(!solution.shares_storage_with(&coefficients));
    assert!(!solution.shares_storage_with(&right_hand_side));
}

#[test]
fn complex_square_solve_promotes_real_multiple_right_hand_sides() {
    let provider = ReferenceProvider;
    let coefficients = complex(
        [2, 2],
        vec![
            ArrayComplex64::new(1.0, 1.0),
            ArrayComplex64::ZERO,
            ArrayComplex64::ZERO,
            ArrayComplex64::new(2.0, -1.0),
        ],
    );
    let right_hand_side = real([2, 2], vec![2.0, 3.0, 4.0, 5.0]);

    let solution = linalg_square_solve(&provider, &coefficients, &right_hand_side, None).unwrap();
    let expected = [
        ArrayComplex64::new(1.0, -1.0),
        ArrayComplex64::new(1.2, 0.6),
        ArrayComplex64::new(2.0, -2.0),
        ArrayComplex64::new(2.0, 1.0),
    ];
    for (&actual, expected) in complex_array(&solution).as_slice().iter().zip(expected) {
        assert_complex_close(actual, expected);
    }
}

#[test]
fn empty_dimensions_flow_through_gemm_and_square_solve() {
    let provider = ReferenceProvider;
    let product = linalg_matrix_multiply(
        &provider,
        &real([0, 2], vec![]),
        &real([2, 3], vec![1.0; 6]),
    )
    .unwrap();
    assert_eq!(real_array(&product).shape().dimensions(), &[0, 3]);
    assert!(real_array(&product).is_empty());

    let single_product = linalg_matrix_multiply(
        &provider,
        &single_real([0, 2], vec![]),
        &single_real([2, 3], vec![1.0; 6]),
    )
    .unwrap();
    assert_eq!(
        single_real_array(&single_product).shape().dimensions(),
        &[0, 3]
    );
    assert!(single_real_array(&single_product).is_empty());

    let coefficients = complex([0, 0], vec![]);
    let right_hand_side = real([0, 4], vec![]);
    let solution = linalg_square_solve(&provider, &coefficients, &right_hand_side, None).unwrap();
    assert_eq!(complex_array(&solution).shape().dimensions(), &[0, 4]);
    assert!(complex_array(&solution).is_empty());
    assert!(!solution.shares_storage_with(&coefficients));
}

#[test]
fn shape_failures_remain_structured_linalg_errors() {
    let provider = ReferenceProvider;
    let mismatch = linalg_matrix_multiply(
        &provider,
        &real([2, 3], vec![0.0; 6]),
        &real([4, 2], vec![0.0; 8]),
    )
    .unwrap_err();
    assert!(matches!(
        mismatch,
        RuntimeLinalgError::Linalg(LinalgError::DimensionMismatch {
            operation: "matrix multiplication inner dimensions",
            left: 3,
            right: 4,
        })
    ));

    let higher_rank = real([1, 1, 2], vec![1.0, 2.0]);
    let rank_error =
        linalg_matrix_multiply(&provider, &higher_rank, &real([2, 1], vec![1.0, 1.0])).unwrap_err();
    assert!(matches!(
        rank_error,
        RuntimeLinalgError::Linalg(LinalgError::MatrixRequired {
            operand: "left matrix-multiply operand",
            dimensions,
        }) if dimensions == vec![1, 1, 2]
    ));

    let non_square = linalg_square_solve(
        &provider,
        &real([2, 3], vec![0.0; 6]),
        &real([2, 1], vec![0.0; 2]),
        None,
    )
    .unwrap_err();
    assert!(matches!(
        non_square,
        RuntimeLinalgError::Linalg(LinalgError::SquareMatrixRequired {
            rows: 2,
            columns: 3,
            ..
        })
    ));
}

#[test]
fn unsupported_values_are_rejected_before_provider_dispatch() {
    let provider = ReferenceProvider;
    let logical = Value::Array(ArrayData::Logical(
        DenseArray::from_vec(Shape::new([1, 1]).unwrap(), vec![Logical::TRUE]).unwrap(),
    ));

    assert!(matches!(
        linalg_matrix_multiply(&provider, &logical, &Value::Double(1.0)),
        Err(RuntimeLinalgError::DoubleMatrixRequired {
            operation: "matrix multiplication",
            operand: "left operand",
            ..
        })
    ));
}

#[test]
fn singularity_and_cancellation_cross_the_bridge_without_string_conversion() {
    let provider = ReferenceProvider;
    let singular = linalg_square_solve(
        &provider,
        &real([2, 2], vec![1.0, 2.0, 2.0, 4.0]),
        &real([2, 1], vec![3.0, 6.0]),
        None,
    )
    .unwrap_err();
    assert_eq!(
        singular,
        RuntimeLinalgError::Linalg(LinalgError::SingularMatrix {
            provider: "reference",
            operation: "general linear solve",
            pivot: 2,
        })
    );

    let cancellation = AtomicBool::new(true);
    let cancelled = linalg_square_solve(
        &provider,
        &Value::Double(2.0),
        &Value::Double(4.0),
        Some(&cancellation),
    )
    .unwrap_err();
    assert_eq!(
        cancelled,
        RuntimeLinalgError::Linalg(LinalgError::Cancelled {
            operation: "general linear solve",
        })
    );
}

#[test]
fn matrix_left_division_dispatches_square_overdetermined_and_basic_underdetermined_paths() {
    let provider = ProbeProvider::new(ProbeOutcome::Success);

    let square = linalg_matrix_left_divide(
        &provider,
        &real([2, 2], vec![3.0, 1.0, 1.0, 2.0]),
        &real([2, 2], vec![9.0, 8.0, 1.0, 7.0]),
        None,
    )
    .unwrap();
    assert_eq!(real_array(&square).as_slice(), &[2.0, 3.0, -1.0, 4.0]);
    assert_eq!(provider.real_solve_calls.load(Ordering::Relaxed), 1);
    assert_eq!(provider.real_rectangular_calls.load(Ordering::Relaxed), 0);

    let overdetermined = linalg_matrix_left_divide(
        &provider,
        &real([4, 2], vec![1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0]),
        &real([4, 2], vec![2.0, 3.0, 2.0, 3.0, 4.0, 5.0, 4.0, 5.0]),
        None,
    )
    .unwrap();
    assert_eq!(real_array(&overdetermined).shape().dimensions(), &[2, 2]);
    for (&actual, expected) in real_array(&overdetermined)
        .as_slice()
        .iter()
        .zip([2.0, 3.0, 4.0, 5.0])
    {
        assert_close(actual, expected);
    }
    assert_eq!(provider.real_solve_calls.load(Ordering::Relaxed), 1);
    assert_eq!(provider.real_rectangular_calls.load(Ordering::Relaxed), 1);

    let underdetermined = linalg_matrix_left_divide(
        &provider,
        &real([2, 3], vec![1.0, 0.0, 0.0, 1.0, 1.0, 1.0]),
        &real([2, 1], vec![1.0, 2.0]),
        None,
    )
    .unwrap();
    assert_eq!(real_array(&underdetermined).shape().dimensions(), &[3, 1]);
    assert_eq!(real_array(&underdetermined).as_slice(), &[-1.0, 0.0, 2.0]);
    assert_eq!(provider.real_solve_calls.load(Ordering::Relaxed), 2);
    assert_eq!(provider.real_rectangular_calls.load(Ordering::Relaxed), 1);
}

#[test]
fn single_left_division_dispatches_native_square_rectangular_and_basic_paths() {
    let provider = ProbeProvider::new(ProbeOutcome::Success);

    let square = linalg_matrix_left_divide(
        &provider,
        &single_real([2, 2], vec![3.0, 1.0, 1.0, 2.0]),
        &single_real([2, 2], vec![9.0, 8.0, 1.0, 7.0]),
        None,
    )
    .unwrap();
    assert_eq!(
        single_real_array(&square).as_slice(),
        &[2.0, 3.0, -1.0, 4.0]
    );
    assert_eq!(provider.real_solve_f32_calls.load(Ordering::Relaxed), 1);
    assert_eq!(
        provider.real_rectangular_f32_calls.load(Ordering::Relaxed),
        0
    );

    let overdetermined = linalg_matrix_left_divide(
        &provider,
        &single_real([4, 2], vec![1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0]),
        &single_real([4, 2], vec![2.0, 3.0, 2.0, 3.0, 4.0, 5.0, 4.0, 5.0]),
        None,
    )
    .unwrap();
    assert_eq!(
        single_real_array(&overdetermined).shape().dimensions(),
        &[2, 2]
    );
    for (&actual, expected) in single_real_array(&overdetermined)
        .as_slice()
        .iter()
        .zip([2.0_f32, 3.0, 4.0, 5.0])
    {
        assert!((actual - expected).abs() <= 1.0e-5);
    }
    assert_eq!(provider.real_solve_f32_calls.load(Ordering::Relaxed), 1);
    assert_eq!(
        provider.real_rectangular_f32_calls.load(Ordering::Relaxed),
        1
    );

    let underdetermined = linalg_matrix_left_divide(
        &provider,
        &single_real([2, 3], vec![1.0, 0.0, 0.0, 1.0, 1.0, 1.0]),
        &single_real([2, 1], vec![1.0, 2.0]),
        None,
    )
    .unwrap();
    assert_eq!(
        single_real_array(&underdetermined).shape().dimensions(),
        &[3, 1]
    );
    assert_eq!(
        single_real_array(&underdetermined).as_slice(),
        &[-1.0, 0.0, 2.0]
    );
    assert_eq!(provider.real_solve_f32_calls.load(Ordering::Relaxed), 2);
    assert_eq!(
        provider.real_rectangular_f32_calls.load(Ordering::Relaxed),
        1
    );
    assert_eq!(provider.real_solve_calls.load(Ordering::Relaxed), 0);
    assert_eq!(provider.real_rectangular_calls.load(Ordering::Relaxed), 0);
}

#[test]
fn interpreter_routes_matrix_division_to_its_injected_provider_but_not_element_division() {
    let provider = Arc::new(ProbeProvider::new(ProbeOutcome::Success));
    let mut interpreter = Interpreter::with_linalg_provider(
        binary_module(BinaryOperator::LeftDivide, None),
        provider.clone(),
    )
    .unwrap();

    interpreter
        .execute_entry(&[
            real([2, 2], vec![3.0, 1.0, 1.0, 2.0]),
            real([2, 1], vec![9.0, 8.0]),
        ])
        .unwrap();
    interpreter
        .execute_entry(&[
            real([4, 2], vec![1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0]),
            real([4, 1], vec![2.0, 3.0, 2.0, 3.0]),
        ])
        .unwrap();
    interpreter
        .execute_entry(&[
            real([2, 3], vec![1.0, 0.0, 0.0, 1.0, 1.0, 1.0]),
            real([2, 1], vec![1.0, 2.0]),
        ])
        .unwrap();
    assert_eq!(provider.real_solve_calls.load(Ordering::Relaxed), 2);
    assert_eq!(provider.real_rectangular_calls.load(Ordering::Relaxed), 1);

    interpreter
        .replace_module(binary_module(BinaryOperator::Divide, None))
        .unwrap();
    interpreter
        .execute_entry(&[
            complex(
                [1, 2],
                vec![
                    ArrayComplex64::new(2.0, 1.0),
                    ArrayComplex64::new(3.0, -2.0),
                ],
            ),
            complex(
                [2, 2],
                vec![
                    ArrayComplex64::new(1.0, 1.0),
                    ArrayComplex64::ZERO,
                    ArrayComplex64::ZERO,
                    ArrayComplex64::new(1.0, -1.0),
                ],
            ),
        ])
        .unwrap();
    assert_eq!(provider.complex_solve_calls.load(Ordering::Relaxed), 1);

    for operator in [
        BinaryOperator::ElementDivide,
        BinaryOperator::ElementLeftDivide,
    ] {
        interpreter
            .replace_module(binary_module(operator, None))
            .unwrap();
        interpreter
            .execute_entry(&[real([1, 2], vec![2.0, 3.0]), real([1, 2], vec![8.0, 15.0])])
            .unwrap();
    }
    assert_eq!(provider.real_solve_calls.load(Ordering::Relaxed), 2);
    assert_eq!(provider.real_rectangular_calls.load(Ordering::Relaxed), 1);
    assert_eq!(provider.complex_solve_calls.load(Ordering::Relaxed), 1);
    assert_eq!(
        provider.complex_rectangular_calls.load(Ordering::Relaxed),
        0
    );
}

#[test]
fn matrix_division_promotes_complex_inputs_and_preserves_input_cow() {
    let provider = ProbeProvider::new(ProbeOutcome::Success);
    let coefficients = complex(
        [2, 2],
        vec![
            ArrayComplex64::new(1.0, 1.0),
            ArrayComplex64::ZERO,
            ArrayComplex64::ZERO,
            ArrayComplex64::new(2.0, -1.0),
        ],
    );
    let right_hand_side = real([2, 1], vec![2.0, 3.0]);
    let coefficient_alias = coefficients.clone();
    let right_hand_side_alias = right_hand_side.clone();

    let result =
        linalg_matrix_left_divide(&provider, &coefficients, &right_hand_side, None).unwrap();
    assert_eq!(provider.complex_solve_calls.load(Ordering::Relaxed), 1);
    assert_eq!(complex_array(&result).shape().dimensions(), &[2, 1]);
    assert_complex_close(
        complex_array(&result).as_slice()[0],
        ArrayComplex64::new(1.0, -1.0),
    );
    assert_complex_close(
        complex_array(&result).as_slice()[1],
        ArrayComplex64::new(1.2, 0.6),
    );
    assert!(coefficients.shares_storage_with(&coefficient_alias));
    assert!(right_hand_side.shares_storage_with(&right_hand_side_alias));
    assert!(!result.shares_storage_with(&coefficients));
    assert!(!result.shares_storage_with(&right_hand_side));
}

#[test]
fn matrix_right_division_uses_the_complex_conjugate_transpose_relation() {
    let provider = ProbeProvider::new(ProbeOutcome::Success);
    let left = complex(
        [1, 2],
        vec![
            ArrayComplex64::new(2.0, 1.0),
            ArrayComplex64::new(3.0, -2.0),
        ],
    );
    let right = complex(
        [3, 2],
        vec![
            ArrayComplex64::new(1.0, 1.0),
            ArrayComplex64::ZERO,
            ArrayComplex64::new(1.0, 0.0),
            ArrayComplex64::ZERO,
            ArrayComplex64::new(1.0, -1.0),
            ArrayComplex64::new(1.0, 0.0),
        ],
    );
    let left_alias = left.clone();
    let right_alias = right.clone();

    let result = linalg_matrix_right_divide(&provider, &left, &right, None).unwrap();
    assert_eq!(complex_array(&result).shape().dimensions(), &[1, 3]);
    for (&actual, expected) in complex_array(&result).as_slice().iter().zip([
        ArrayComplex64::new(1.5, -0.5),
        ArrayComplex64::new(2.5, 0.5),
        ArrayComplex64::ZERO,
    ]) {
        assert_complex_close(actual, expected);
    }
    assert_eq!(provider.complex_solve_calls.load(Ordering::Relaxed), 1);
    assert_eq!(
        provider.complex_rectangular_calls.load(Ordering::Relaxed),
        0
    );
    assert!(left.shares_storage_with(&left_alias));
    assert!(right.shares_storage_with(&right_alias));
    assert!(!result.shares_storage_with(&left));
    assert!(!result.shares_storage_with(&right));
}

#[test]
fn single_right_division_uses_native_complex32_rectangular_and_basic_paths() {
    let provider = ProbeProvider::new(ProbeOutcome::Success);

    let overdetermined = linalg_matrix_right_divide(
        &provider,
        &single_complex(
            [1, 3],
            vec![
                Complex32::new(2.0, 1.0),
                Complex32::new(3.0, -1.0),
                Complex32::new(5.0, 0.0),
            ],
        ),
        &single_complex(
            [2, 3],
            vec![
                Complex32::new(1.0, 0.0),
                Complex32::ZERO,
                Complex32::ZERO,
                Complex32::new(1.0, 0.0),
                Complex32::new(1.0, 0.0),
                Complex32::new(1.0, 0.0),
            ],
        ),
        None,
    )
    .unwrap();
    assert_eq!(
        single_complex_array(&overdetermined).shape().dimensions(),
        &[1, 2]
    );
    for (&actual, expected) in single_complex_array(&overdetermined)
        .as_slice()
        .iter()
        .zip([Complex32::new(2.0, 1.0), Complex32::new(3.0, -1.0)])
    {
        assert!((actual.re - expected.re).abs() <= 1.0e-5);
        assert!((actual.im - expected.im).abs() <= 1.0e-5);
    }
    assert_eq!(
        provider
            .complex_rectangular_f32_calls
            .load(Ordering::Relaxed),
        1
    );

    let underdetermined = linalg_matrix_right_divide(
        &provider,
        &single_complex(
            [1, 2],
            vec![Complex32::new(2.0, 1.0), Complex32::new(3.0, -2.0)],
        ),
        &single_complex(
            [3, 2],
            vec![
                Complex32::new(1.0, 1.0),
                Complex32::ZERO,
                Complex32::new(1.0, 0.0),
                Complex32::ZERO,
                Complex32::new(1.0, -1.0),
                Complex32::new(1.0, 0.0),
            ],
        ),
        None,
    )
    .unwrap();
    assert_eq!(
        single_complex_array(&underdetermined).shape().dimensions(),
        &[1, 3]
    );
    for (&actual, expected) in single_complex_array(&underdetermined)
        .as_slice()
        .iter()
        .zip([
            Complex32::new(1.5, -0.5),
            Complex32::new(2.5, 0.5),
            Complex32::ZERO,
        ])
    {
        assert!((actual.re - expected.re).abs() <= 1.0e-5);
        assert!((actual.im - expected.im).abs() <= 1.0e-5);
    }
    assert_eq!(provider.complex_solve_f32_calls.load(Ordering::Relaxed), 1);
    assert_eq!(provider.complex_solve_calls.load(Ordering::Relaxed), 0);
    assert_eq!(
        provider.complex_rectangular_calls.load(Ordering::Relaxed),
        0
    );
}

#[test]
fn matrix_division_empty_shapes_short_circuit_without_provider_calls() {
    let provider = ProbeProvider::new(ProbeOutcome::ProviderFailure);
    let left_results = [
        linalg_matrix_left_divide(
            &provider,
            &real([2, 2], vec![1.0, 0.0, 0.0, 1.0]),
            &real([2, 0], vec![]),
            None,
        )
        .unwrap(),
        linalg_matrix_left_divide(
            &provider,
            &real([3, 0], vec![]),
            &real([3, 2], vec![0.0; 6]),
            None,
        )
        .unwrap(),
        linalg_matrix_left_divide(
            &provider,
            &real([0, 3], vec![]),
            &real([0, 2], vec![]),
            None,
        )
        .unwrap(),
        linalg_matrix_left_divide(
            &provider,
            &real([0, 0], vec![]),
            &real([0, 0], vec![]),
            None,
        )
        .unwrap(),
    ];
    for (result, shape) in left_results.iter().zip([[2, 0], [0, 2], [3, 2], [0, 0]]) {
        assert_eq!(real_array(result).shape().dimensions(), shape);
        assert!(
            real_array(result)
                .as_slice()
                .iter()
                .all(|value| *value == 0.0)
        );
    }

    let right_empty = linalg_matrix_right_divide(
        &provider,
        &real([0, 2], vec![]),
        &real([2, 2], vec![1.0, 0.0, 0.0, 1.0]),
        None,
    )
    .unwrap();
    assert_eq!(real_array(&right_empty).shape().dimensions(), &[0, 2]);
    let right_zero = linalg_matrix_right_divide(
        &provider,
        &real([2, 0], vec![]),
        &real([3, 0], vec![]),
        None,
    )
    .unwrap();
    assert_eq!(real_array(&right_zero).shape().dimensions(), &[2, 3]);
    assert_eq!(real_array(&right_zero).as_slice(), &[0.0; 6]);

    assert_eq!(provider.real_solve_calls.load(Ordering::Relaxed), 0);
    assert_eq!(provider.real_rectangular_calls.load(Ordering::Relaxed), 0);

    let single_left = linalg_matrix_left_divide(
        &provider,
        &single_real([0, 3], vec![]),
        &single_real([0, 2], vec![]),
        None,
    )
    .unwrap();
    assert_eq!(
        single_real_array(&single_left).shape().dimensions(),
        &[3, 2]
    );
    assert_eq!(single_real_array(&single_left).as_slice(), &[0.0; 6]);
    let single_right = linalg_matrix_right_divide(
        &provider,
        &single_complex([2, 0], vec![]),
        &single_complex([3, 0], vec![]),
        None,
    )
    .unwrap();
    assert_eq!(
        single_complex_array(&single_right).shape().dimensions(),
        &[2, 3]
    );
    assert_eq!(
        single_complex_array(&single_right).as_slice(),
        &[Complex32::ZERO; 6]
    );
    assert_eq!(provider.real_solve_f32_calls.load(Ordering::Relaxed), 0);
    assert_eq!(provider.complex_solve_f32_calls.load(Ordering::Relaxed), 0);
    assert_eq!(
        provider.real_rectangular_f32_calls.load(Ordering::Relaxed),
        0
    );
    assert_eq!(
        provider
            .complex_rectangular_f32_calls
            .load(Ordering::Relaxed),
        0
    );
}

#[test]
fn complex_single_matrix_solve_uses_native_complex32_and_keeps_storage_class() {
    let provider = ProbeProvider::new(ProbeOutcome::Success);
    let coefficients = single_complex(
        [2, 2],
        vec![
            Complex32::new(1.0, 1.0),
            Complex32::ZERO,
            Complex32::ZERO,
            Complex32::new(2.0, -1.0),
        ],
    );
    let right_hand_side = single_complex(
        [2, 1],
        vec![Complex32::new(2.0, 0.0), Complex32::new(3.0, 0.0)],
    );
    let coefficient_alias = coefficients.clone();
    let right_hand_side_alias = right_hand_side.clone();
    let result =
        linalg_matrix_left_divide(&provider, &coefficients, &right_hand_side, None).unwrap();
    assert_eq!(provider.complex_solve_f32_calls.load(Ordering::Relaxed), 1);
    assert_eq!(provider.complex_solve_calls.load(Ordering::Relaxed), 0);
    assert_eq!(single_complex_array(&result).shape().dimensions(), &[2, 1]);
    for (&actual, expected) in single_complex_array(&result)
        .as_slice()
        .iter()
        .zip([Complex32::new(1.0, -1.0), Complex32::new(1.2, 0.6)])
    {
        assert!((actual.re - expected.re).abs() <= 5.0e-6);
        assert!((actual.im - expected.im).abs() <= 5.0e-6);
    }
    assert!(coefficients.shares_storage_with(&coefficient_alias));
    assert!(right_hand_side.shares_storage_with(&right_hand_side_alias));
    assert!(!result.shares_storage_with(&coefficients));
    assert!(!result.shares_storage_with(&right_hand_side));
}

#[test]
fn matrix_division_rejects_class_rank_and_shape_boundaries_structurally() {
    let provider = ProbeProvider::new(ProbeOutcome::Success);
    let mismatch = linalg_matrix_left_divide(
        &provider,
        &real([2, 2], vec![1.0, 0.0, 0.0, 1.0]),
        &real([3, 1], vec![1.0; 3]),
        None,
    )
    .unwrap_err();
    assert!(matches!(
        mismatch,
        RuntimeLinalgError::Linalg(LinalgError::DimensionMismatch {
            operation: "matrix left division coefficient and right-hand-side rows",
            left: 2,
            right: 3,
        })
    ));

    let right_mismatch = linalg_matrix_right_divide(
        &provider,
        &real([1, 2], vec![1.0, 2.0]),
        &real([3, 3], vec![0.0; 9]),
        None,
    )
    .unwrap_err();
    assert!(matches!(
        right_mismatch,
        RuntimeLinalgError::Linalg(LinalgError::DimensionMismatch {
            operation: "matrix right division operand columns",
            left: 2,
            right: 3,
        })
    ));
    assert_eq!(provider.real_solve_calls.load(Ordering::Relaxed), 0);
    assert_eq!(provider.real_rectangular_calls.load(Ordering::Relaxed), 0);

    let logical = Value::Array(ArrayData::Logical(
        DenseArray::from_vec(Shape::new([2, 2]).unwrap(), vec![Logical::TRUE; 4]).unwrap(),
    ));
    assert!(matches!(
        linalg_matrix_left_divide(&provider, &logical, &real([2, 1], vec![1.0, 2.0]), None),
        Err(RuntimeLinalgError::NumericMatrixRequired {
            operation: "matrix left division",
            operand: "coefficient matrix",
            ..
        })
    ));

    assert!(matches!(
        linalg_matrix_left_divide(
            &provider,
            &single_complex([2, 2], vec![Complex32::ZERO; 4]),
            &real([2, 1], vec![1.0, 2.0]),
            None,
        ),
        Err(RuntimeLinalgError::MixedPrecisionMatrixDivision { .. })
    ));

    let higher_rank = real([1, 1, 2], vec![1.0, 2.0]);
    assert!(matches!(
        linalg_matrix_left_divide(&provider, &higher_rank, &real([1, 1], vec![1.0]), None),
        Err(RuntimeLinalgError::Linalg(LinalgError::MatrixRequired {
            operand: "matrix left division coefficient matrix",
            dimensions,
        })) if dimensions == vec![1, 1, 2]
    ));

    let higher_rank_single = single_real([1, 1, 2], vec![1.0, 2.0]);
    assert!(matches!(
        linalg_matrix_left_divide(
            &provider,
            &higher_rank_single,
            &single_real([1, 1], vec![1.0]),
            None,
        ),
        Err(RuntimeLinalgError::Linalg(LinalgError::MatrixRequired {
            operand: "matrix left division coefficient matrix",
            dimensions,
        })) if dimensions == vec![1, 1, 2]
    ));
}

#[test]
fn matrix_division_preserves_rank_singularity_provider_failure_and_cancellation_errors() {
    let singular = linalg_matrix_left_divide(
        &ReferenceProvider,
        &real([2, 2], vec![1.0, 2.0, 2.0, 4.0]),
        &real([2, 1], vec![3.0, 6.0]),
        None,
    )
    .unwrap_err();
    assert!(matches!(
        singular,
        RuntimeLinalgError::Linalg(LinalgError::SingularMatrix { pivot: 2, .. })
    ));

    let rank_deficient = linalg_matrix_left_divide(
        &ReferenceProvider,
        &real([3, 2], vec![1.0, 2.0, 3.0, 2.0, 4.0, 6.0]),
        &real([3, 1], vec![1.0, 2.0, 3.0]),
        None,
    )
    .unwrap_err();
    assert!(matches!(
        rank_deficient,
        RuntimeLinalgError::Linalg(LinalgError::RankDeficient { .. })
    ));

    let failure_provider = ProbeProvider::new(ProbeOutcome::ProviderFailure);
    let failure = linalg_matrix_left_divide(
        &failure_provider,
        &real([2, 2], vec![1.0, 0.0, 0.0, 1.0]),
        &real([2, 1], vec![1.0, 2.0]),
        None,
    )
    .unwrap_err();
    assert!(matches!(
        failure,
        RuntimeLinalgError::Linalg(LinalgError::ProviderFailure {
            provider: "probe",
            detail,
            ..
        }) if detail == "injected solve failure"
    ));
    let cancellation = Arc::new(AtomicBool::new(false));
    let cancelling_provider =
        ProbeProvider::new(ProbeOutcome::Success).with_solve_cancellation(cancellation.clone());
    let cancelled = linalg_matrix_left_divide(
        &cancelling_provider,
        &real([2, 2], vec![1.0, 0.0, 0.0, 1.0]),
        &real([2, 1], vec![1.0, 2.0]),
        Some(cancellation.as_ref()),
    )
    .unwrap_err();
    assert!(matches!(
        cancelled,
        RuntimeLinalgError::Linalg(LinalgError::Cancelled {
            operation: "general linear solve",
        })
    ));
    assert_eq!(
        cancelling_provider.real_solve_calls.load(Ordering::Relaxed),
        1
    );
}

#[test]
fn single_matrix_division_preserves_provider_failure_and_cancellation_errors() {
    let failure_provider = ProbeProvider::new(ProbeOutcome::ProviderFailure);
    let single_failure = linalg_matrix_left_divide(
        &failure_provider,
        &single_real([2, 2], vec![1.0, 0.0, 0.0, 1.0]),
        &single_real([2, 1], vec![1.0, 2.0]),
        None,
    )
    .unwrap_err();
    assert!(matches!(
        single_failure,
        RuntimeLinalgError::Linalg(LinalgError::ProviderFailure {
            provider: "probe",
            detail,
            ..
        }) if detail == "injected solve failure"
    ));
    assert_eq!(
        failure_provider
            .real_solve_f32_calls
            .load(Ordering::Relaxed),
        1
    );
    assert_eq!(failure_provider.real_solve_calls.load(Ordering::Relaxed), 0);

    let cancellation = Arc::new(AtomicBool::new(false));
    let cancelling_provider =
        ProbeProvider::new(ProbeOutcome::Success).with_solve_cancellation(cancellation.clone());
    let single_cancelled = linalg_matrix_left_divide(
        &cancelling_provider,
        &single_real([2, 2], vec![1.0, 0.0, 0.0, 1.0]),
        &single_real([2, 1], vec![1.0, 2.0]),
        Some(cancellation.as_ref()),
    )
    .unwrap_err();
    assert!(matches!(
        single_cancelled,
        RuntimeLinalgError::Linalg(LinalgError::Cancelled {
            operation: "general linear solve",
        })
    ));
    assert_eq!(
        cancelling_provider
            .real_solve_f32_calls
            .load(Ordering::Relaxed),
        1
    );
    assert_eq!(
        cancelling_provider.real_solve_calls.load(Ordering::Relaxed),
        0
    );
}

#[test]
fn matrix_division_conversion_transpose_and_empty_paths_observe_pre_cancellation() {
    let provider = ProbeProvider::new(ProbeOutcome::Success);
    let cancellation = AtomicBool::new(true);

    let single_cancelled = linalg_matrix_left_divide(
        &provider,
        &single_complex([2, 2], vec![Complex32::ZERO; 4]),
        &single_complex([2, 1], vec![Complex32::ZERO; 2]),
        Some(&cancellation),
    )
    .unwrap_err();
    assert!(matches!(
        single_cancelled,
        RuntimeLinalgError::Linalg(LinalgError::Cancelled {
            operation: "matrix left division",
        })
    ));

    let transpose_cancelled = linalg_matrix_right_divide(
        &provider,
        &complex([1, 2], vec![ArrayComplex64::ZERO; 2]),
        &complex([2, 2], vec![ArrayComplex64::ZERO; 4]),
        Some(&cancellation),
    )
    .unwrap_err();
    assert!(matches!(
        transpose_cancelled,
        RuntimeLinalgError::Linalg(LinalgError::Cancelled {
            operation: "matrix right division conjugate transpose",
        })
    ));

    let empty_cancelled = linalg_matrix_left_divide(
        &provider,
        &real([0, 3], vec![]),
        &real([0, 2], vec![]),
        Some(&cancellation),
    )
    .unwrap_err();
    assert!(matches!(
        empty_cancelled,
        RuntimeLinalgError::Linalg(LinalgError::Cancelled {
            operation: "matrix left division",
        })
    ));
    assert_eq!(provider.real_solve_calls.load(Ordering::Relaxed), 0);
    assert_eq!(provider.complex_solve_calls.load(Ordering::Relaxed), 0);
    assert_eq!(provider.real_solve_f32_calls.load(Ordering::Relaxed), 0);
    assert_eq!(provider.complex_solve_f32_calls.load(Ordering::Relaxed), 0);
    assert_eq!(provider.real_rectangular_calls.load(Ordering::Relaxed), 0);
    assert_eq!(
        provider.complex_rectangular_calls.load(Ordering::Relaxed),
        0
    );
}

#[test]
fn interpreter_builtin_context_dispatches_to_the_injected_provider() {
    let provider = Arc::new(ProbeProvider::new(ProbeOutcome::Success));
    let mut registry = BuiltinRegistry::new();
    registry
        .register(
            "probe_lu",
            |arguments: &[Value], context: &mut BuiltinContext<'_>| {
                let Some(Value::Array(ArrayData::F64(matrix))) = arguments.first() else {
                    return Err(BuiltinError::new(
                        BuiltinErrorCategory::Type,
                        "probe_lu requires a double matrix",
                    ));
                };
                let result = context
                    .linalg_provider()
                    .factor_lu_f64(
                        FactorRequest::new(matrix)
                            .with_cancellation_flag(context.cancellation_flag()),
                    )
                    .map_err(|error| {
                        BuiltinError::new(BuiltinErrorCategory::Other, error.to_string())
                    })?;
                Ok(vec![Value::Array(ArrayData::F64(result.packed_lu))])
            },
        )
        .expect("probe built-in registration");
    let function = Function {
        name: "builtin_provider_probe".to_owned(),
        register_count: 3,
        pack_register_count: 0,
        local_count: 1,
        persistent_slot_count: 0,
        parameter_count: 1,
        argument_layout: None,
        constants: vec![Constant::String("probe_lu".to_owned())],
        instructions: vec![
            Instruction::new(InstructionKind::LoadFunctionHandle {
                dst: Register::new(0),
                name: ConstantId::new(0),
            }),
            Instruction::new(InstructionKind::LoadLocal {
                dst: Register::new(1),
                local: LocalSlot::new(0),
            }),
            Instruction::new(InstructionKind::Apply {
                outputs: vec![Register::new(2)],
                target: Register::new(0),
                arguments: vec![ApplyArgument::Value(Register::new(1))],
            }),
            Instruction::new(InstructionKind::Return {
                values: vec![Register::new(2)],
            }),
        ],
        exception_handlers: Vec::new(),
    };
    let module = BytecodeModule::new(vec![function], FunctionId::new(0));
    let mut interpreter = Interpreter::with_components_and_linalg_provider(
        module,
        registry,
        Box::new(NullOutput),
        provider.clone(),
    )
    .expect("probe module should verify");

    let input = real([2, 2], vec![0.0, 2.0, 1.0, 3.0]);
    let result = interpreter
        .execute_entry(std::slice::from_ref(&input))
        .expect("probe built-in should execute");

    assert_eq!(provider.real_lu_calls.load(Ordering::Relaxed), 1);
    assert_eq!(real_array(&result[0]).shape().dimensions(), &[2, 2]);
    assert!(!result[0].shares_storage_with(&input));
}

#[test]
fn interpreter_dispatches_double_matrix_products_to_injected_provider() {
    let provider = Arc::new(ProbeProvider::new(ProbeOutcome::Success));
    let mut interpreter = Interpreter::with_linalg_provider(
        binary_module(BinaryOperator::Multiply, None),
        provider.clone(),
    )
    .expect("matrix-multiply bytecode should verify");
    assert_eq!(interpreter.linalg_provider().name(), "probe");

    let left = real([2, 3], vec![1.0, 4.0, 2.0, 5.0, 3.0, 6.0]);
    let right = real([3, 2], vec![7.0, 9.0, 11.0, 8.0, 10.0, 12.0]);
    let left_alias = left.clone();
    let right_alias = right.clone();
    let product = interpreter
        .execute_entry(&[left.clone(), right.clone()])
        .expect("real GEMM should execute")
        .remove(0);
    assert_eq!(provider.real_gemm_calls.load(Ordering::Relaxed), 1);
    assert_eq!(real_array(&product).shape().dimensions(), &[2, 2]);
    assert_eq!(real_array(&product).as_slice(), &[58.0, 139.0, 64.0, 154.0]);
    assert!(left.shares_storage_with(&left_alias));
    assert!(right.shares_storage_with(&right_alias));
    assert!(!product.shares_storage_with(&left));
    assert!(!product.shares_storage_with(&right));

    let mixed = interpreter
        .execute_entry(&[
            real([1, 2], vec![1.0, 2.0]),
            complex(
                [2, 1],
                vec![
                    ArrayComplex64::new(3.0, 1.0),
                    ArrayComplex64::new(4.0, -2.0),
                ],
            ),
        ])
        .expect("mixed real/complex GEMM should execute")
        .remove(0);
    assert_eq!(provider.complex_gemm_calls.load(Ordering::Relaxed), 1);
    assert!(provider.saw_zero_imaginary_left.load(Ordering::Relaxed));
    assert_eq!(complex_array(&mixed).shape().dimensions(), &[1, 1]);
    assert_complex_close(
        complex_array(&mixed).as_slice()[0],
        ArrayComplex64::new(11.0, -3.0),
    );

    let empty = interpreter
        .execute_entry(&[real([0, 2], Vec::new()), real([2, 3], vec![1.0; 6])])
        .expect("empty GEMM should execute")
        .remove(0);
    assert_eq!(provider.real_gemm_calls.load(Ordering::Relaxed), 2);
    assert_eq!(real_array(&empty).shape().dimensions(), &[0, 3]);
    assert!(real_array(&empty).is_empty());

    let scalar = interpreter
        .execute_entry(&[Value::Double(2.0), Value::Double(4.0)])
        .expect("scalar multiply should retain scalar semantics");
    assert_eq!(scalar, [Value::Double(8.0)]);
    assert_eq!(provider.real_gemm_calls.load(Ordering::Relaxed), 2);

    let scaled = interpreter
        .execute_entry(&[real([1, 2], vec![2.0, 3.0]), Value::Double(4.0)])
        .expect("matrix/scalar multiply should retain element-wise scaling")
        .remove(0);
    assert_eq!(real_array(&scaled).as_slice(), &[8.0, 12.0]);
    assert_eq!(provider.real_gemm_calls.load(Ordering::Relaxed), 2);

    interpreter
        .replace_module(binary_module(BinaryOperator::ElementMultiply, None))
        .expect("element-multiply bytecode should verify");
    let elementwise = interpreter
        .execute_entry(&[real([1, 2], vec![2.0, 3.0]), real([1, 2], vec![4.0, 5.0])])
        .expect("element multiply should remain outside the provider")
        .remove(0);
    assert_eq!(real_array(&elementwise).as_slice(), &[8.0, 15.0]);
    assert_eq!(provider.real_gemm_calls.load(Ordering::Relaxed), 2);
}

#[test]
fn interpreter_dispatches_non_scalar_single_products_without_binary64_widening() {
    let provider = Arc::new(ProbeProvider::new(ProbeOutcome::Success));
    let mut interpreter = Interpreter::with_linalg_provider(
        binary_module(BinaryOperator::Multiply, None),
        provider.clone(),
    )
    .expect("matrix-multiply bytecode should verify");

    let left = single_real([1, 3], vec![1.0e10, 1.0, -1.0e10]);
    let right = single_real([3, 1], vec![1.0, 1.0, 1.0]);
    let product = interpreter
        .execute_entry(&[left.clone(), right.clone()])
        .expect("single GEMM should execute")
        .remove(0);
    assert_eq!(
        single_real_array(&product).as_slice()[0].to_bits(),
        0.0_f32.to_bits()
    );
    let widened_sum = [1.0e10_f32, 1.0, -1.0e10]
        .iter()
        .copied()
        .map(f64::from)
        .sum::<f64>();
    assert_eq!(widened_sum.to_bits(), 1.0_f64.to_bits());
    assert_eq!(provider.real_gemm_f32_calls.load(Ordering::Relaxed), 1);
    assert_eq!(provider.real_gemm_calls.load(Ordering::Relaxed), 0);
    assert!(!product.shares_storage_with(&left));
    assert!(!product.shares_storage_with(&right));

    let complex_product = interpreter
        .execute_entry(&[
            single_real([1, 2], vec![1.0, 2.0]),
            single_complex(
                [2, 1],
                vec![Complex32::new(3.0, 1.0), Complex32::new(4.0, -2.0)],
            ),
        ])
        .expect("complex32 GEMM should execute")
        .remove(0);
    assert_eq!(
        single_complex_array(&complex_product).as_slice(),
        &[Complex32::new(11.0, -3.0)]
    );
    assert_eq!(provider.complex_gemm_f32_calls.load(Ordering::Relaxed), 1);
    assert_eq!(provider.complex_gemm_calls.load(Ordering::Relaxed), 0);

    let scalar_error = interpreter
        .execute_entry(&[
            single_real([1, 1], vec![2.0]),
            single_real([1, 1], vec![3.0]),
        ])
        .expect_err("the existing single 1x1 matrix-multiply boundary must remain");
    assert_eq!(
        scalar_error.kind,
        RuntimeErrorKind::UnsupportedArrayOperator {
            operator: BinaryOperator::Multiply,
        }
    );
    assert_eq!(provider.real_gemm_f32_calls.load(Ordering::Relaxed), 1);

    let mixed_error = interpreter
        .execute_entry(&[
            single_real([1, 2], vec![1.0, 2.0]),
            real([2, 1], vec![3.0, 4.0]),
        ])
        .expect_err("mixed single/double matrix multiply must retain its existing boundary");
    assert_eq!(
        mixed_error.kind,
        RuntimeErrorKind::UnsupportedArrayOperator {
            operator: BinaryOperator::Multiply,
        }
    );
    assert_eq!(provider.real_gemm_f32_calls.load(Ordering::Relaxed), 1);
    assert_eq!(provider.real_gemm_calls.load(Ordering::Relaxed), 0);
}

#[test]
fn interpreter_preserves_structured_linalg_errors_location_and_stack() {
    let location = SourceLocation::new(17, 23, 31);
    let shape_provider = Arc::new(ProbeProvider::new(ProbeOutcome::Success));
    let mut interpreter = Interpreter::with_linalg_provider(
        binary_module(BinaryOperator::Multiply, Some(location)),
        shape_provider.clone(),
    )
    .expect("matrix-multiply bytecode should verify");
    let shape_error = interpreter
        .execute_entry(&[real([2, 3], vec![0.0; 6]), real([4, 2], vec![0.0; 8])])
        .expect_err("incompatible matrix dimensions should fail");
    assert_eq!(shape_provider.real_gemm_calls.load(Ordering::Relaxed), 0);
    assert!(matches!(
        shape_error.linalg_error(),
        Some(RuntimeLinalgError::Linalg(LinalgError::DimensionMismatch {
            operation: "matrix multiplication inner dimensions",
            left: 3,
            right: 4,
        }))
    ));
    assert!(matches!(
        shape_error.kind,
        RuntimeErrorKind::InvalidExecutionState {
            array: Some(detail),
            ..
        } if matches!(
            detail.as_ref(),
            ArrayRuntimeError::LinearAlgebra {
                error: RuntimeLinalgError::Linalg(LinalgError::DimensionMismatch {
                    operation: "matrix multiplication inner dimensions",
                    left: 3,
                    right: 4,
                })
            }
        )
    ));
    assert_eq!(shape_error.location, Some(location));
    assert_eq!(shape_error.stack.len(), 1);
    assert_eq!(shape_error.stack[0].location, Some(location));

    let failure_provider = Arc::new(ProbeProvider::new(ProbeOutcome::ProviderFailure));
    let mut interpreter = Interpreter::with_linalg_provider(
        binary_module(BinaryOperator::Multiply, Some(location)),
        failure_provider.clone(),
    )
    .expect("matrix-multiply bytecode should verify");
    let provider_error = interpreter
        .execute_entry(&[real([1, 2], vec![1.0, 2.0]), real([2, 1], vec![3.0, 4.0])])
        .expect_err("injected provider failure should cross the interpreter");
    assert_eq!(failure_provider.real_gemm_calls.load(Ordering::Relaxed), 1);
    assert!(matches!(
        provider_error.linalg_error(),
        Some(RuntimeLinalgError::Linalg(
            LinalgError::ProviderFailure {
                provider: "probe",
                operation: "matrix multiplication",
                detail,
            }
        )) if detail == "injected GEMM failure"
    ));
    assert!(matches!(
        provider_error.kind,
        RuntimeErrorKind::InvalidExecutionState {
            array: Some(detail),
            ..
        } if matches!(
            detail.as_ref(),
            ArrayRuntimeError::LinearAlgebra {
                error: RuntimeLinalgError::Linalg(LinalgError::ProviderFailure {
                    provider: "probe",
                    operation: "matrix multiplication",
                    detail,
                })
            } if detail == "injected GEMM failure"
        )
    ));
    assert_eq!(provider_error.location, Some(location));
    assert_eq!(provider_error.stack.len(), 1);
    assert_eq!(provider_error.stack[0].location, Some(location));
}

#[test]
fn interpreter_cancellation_boundaries_do_not_stringify_or_overrun_provider() {
    let location = SourceLocation::new(19, 5, 12);
    let provider_cancelled = Arc::new(ProbeProvider::new(ProbeOutcome::Cancelled));
    let mut interpreter = Interpreter::with_linalg_provider(
        binary_module(BinaryOperator::Multiply, Some(location)),
        provider_cancelled.clone(),
    )
    .expect("matrix-multiply bytecode should verify");
    let error = interpreter
        .execute_entry(&[real([1, 2], vec![1.0, 2.0]), real([2, 1], vec![3.0, 4.0])])
        .expect_err("provider cancellation should stop execution");
    assert_eq!(error.kind, RuntimeErrorKind::Cancelled);
    assert_eq!(error.location, Some(location));
    assert_eq!(error.stack.len(), 1);
    assert_eq!(
        provider_cancelled.real_gemm_calls.load(Ordering::Relaxed),
        1
    );

    let boundary_provider = Arc::new(ProbeProvider::new(ProbeOutcome::Success));
    let mut interpreter = Interpreter::with_linalg_provider(
        binary_module(BinaryOperator::Multiply, Some(location)),
        boundary_provider.clone(),
    )
    .expect("matrix-multiply bytecode should verify");
    interpreter.cancellation_token().cancel();
    let error = interpreter
        .execute_entry(&[real([1, 2], vec![1.0, 2.0]), real([2, 1], vec![3.0, 4.0])])
        .expect_err("pre-existing cancellation should stop before dispatch");
    assert_eq!(error.kind, RuntimeErrorKind::Cancelled);
    assert_eq!(boundary_provider.real_gemm_calls.load(Ordering::Relaxed), 0);
}
