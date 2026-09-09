use openmat_array::{ArrayData, CharCodeUnit, Complex64, DenseArray, Logical, Shape};
use openmat_builtins::minimal_registry;
use openmat_fft::{FftComplex32, FftComplex64, FftDirection, FftError, FftErrorKind, FftProvider};
use openmat_runtime::{
    BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinInvocationError, CancellationToken,
    NullOutput,
};
use openmat_value::Value;

fn invoke(name: &str, arguments: &[Value]) -> Result<Vec<Value>, BuiltinError> {
    let registry = minimal_registry().expect("fresh registry");
    let handle = registry.handle_by_name(name).expect("registered built-in");
    let cancellation = CancellationToken::new();
    let mut output = NullOutput;
    let mut context = BuiltinContext::new(1, &cancellation, &mut output);
    registry
        .invoke(handle, arguments, &mut context)
        .map_err(|error| match error {
            BuiltinInvocationError::Failed { error, .. } => error,
            BuiltinInvocationError::UnknownHandle(_) => panic!("looked-up handle must resolve"),
        })
}

struct RejectingProvider;

impl FftProvider for RejectingProvider {
    fn transform_f64(
        &self,
        _values: &mut [FftComplex64],
        _length: usize,
        _direction: FftDirection,
        _cancellation: &std::sync::atomic::AtomicBool,
    ) -> Result<(), FftError> {
        Err(FftError {
            kind: FftErrorKind::ProviderFailure,
        })
    }

    fn transform_f32(
        &self,
        _values: &mut [FftComplex32],
        _length: usize,
        _direction: FftDirection,
        _cancellation: &std::sync::atomic::AtomicBool,
    ) -> Result<(), FftError> {
        Err(FftError {
            kind: FftErrorKind::ProviderFailure,
        })
    }
}

fn invoke_with_provider(
    name: &str,
    arguments: &[Value],
    provider: &dyn FftProvider,
) -> Result<Vec<Value>, BuiltinError> {
    let registry = minimal_registry().expect("fresh registry");
    let handle = registry.handle_by_name(name).expect("registered built-in");
    let cancellation = CancellationToken::new();
    let mut output = NullOutput;
    let mut context = BuiltinContext::with_fft_provider(1, &cancellation, &mut output, provider);
    registry
        .invoke(handle, arguments, &mut context)
        .map_err(|error| match error {
            BuiltinInvocationError::Failed { error, .. } => error,
            BuiltinInvocationError::UnknownHandle(_) => panic!("looked-up handle must resolve"),
        })
}

fn double_array<const N: usize>(dimensions: [u64; N], values: Vec<f64>) -> Value {
    Value::Array(ArrayData::F64(
        DenseArray::from_vec(Shape::new(dimensions).unwrap(), values).unwrap(),
    ))
}

fn single_array<const N: usize>(dimensions: [u64; N], values: Vec<f32>) -> Value {
    Value::Array(ArrayData::F32(
        DenseArray::from_vec(Shape::new(dimensions).unwrap(), values).unwrap(),
    ))
}

fn char_array(text: &str) -> Value {
    let values = text
        .encode_utf16()
        .map(CharCodeUnit::new)
        .collect::<Vec<_>>();
    Value::Array(ArrayData::Char(
        DenseArray::from_vec(Shape::new([1, values.len() as u64]).unwrap(), values).unwrap(),
    ))
}

fn assert_real(value: &Value, dimensions: &[u64], expected: &[f64], tolerance: f64) {
    let Value::Array(ArrayData::F64(array)) = value else {
        panic!("expected real double array, received {value:?}");
    };
    assert_eq!(array.shape().dimensions(), dimensions);
    assert_eq!(array.as_slice().len(), expected.len());
    for (actual, expected) in array.as_slice().iter().zip(expected) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "{actual} != {expected}"
        );
    }
}

fn assert_complex(value: &Value, dimensions: &[u64], expected: &[(f64, f64)], tolerance: f64) {
    let Value::Array(ArrayData::ComplexF64(array)) = value else {
        panic!("expected complex double array, received {value:?}");
    };
    assert_eq!(array.shape().dimensions(), dimensions);
    assert_eq!(array.as_slice().len(), expected.len());
    for (actual, (real, imaginary)) in array.as_slice().iter().zip(expected) {
        assert!((actual.re - real).abs() <= tolerance);
        assert!((actual.im - imaginary).abs() <= tolerance);
    }
}

#[test]
fn fft_and_ifft_match_r2022b_vector_orientation_values_and_real_recovery() {
    let source = double_array([1, 4], vec![1.0, 2.0, 3.0, 4.0]);
    let transformed = invoke("fft", std::slice::from_ref(&source))
        .unwrap()
        .remove(0);
    assert_complex(
        &transformed,
        &[1, 4],
        &[(10.0, 0.0), (-2.0, 2.0), (-2.0, 0.0), (-2.0, -2.0)],
        1.0e-12,
    );
    let inverse = invoke("ifft", &[transformed]).unwrap().remove(0);
    assert_real(&inverse, &[1, 4], &[1.0, 2.0, 3.0, 4.0], 1.0e-12);

    let scalar_length = invoke("fft", &[Value::Double(7.0), Value::Double(3.0)])
        .unwrap()
        .remove(0);
    assert_real(&scalar_length, &[1, 3], &[7.0, 7.0, 7.0], 1.0e-12);
}

#[test]
fn fft_default_and_explicit_dimensions_use_column_major_padding() {
    let source = double_array([2, 3], vec![1.0, 4.0, 2.0, 5.0, 3.0, 6.0]);
    let default = invoke("fft", std::slice::from_ref(&source))
        .unwrap()
        .remove(0);
    assert_real(
        &default,
        &[2, 3],
        &[5.0, -3.0, 7.0, -3.0, 9.0, -3.0],
        1.0e-12,
    );

    let along_rows = invoke(
        "fft",
        &[source, double_array([0, 0], Vec::new()), Value::Double(2.0)],
    )
    .unwrap()
    .remove(0);
    assert_complex(
        &along_rows,
        &[2, 3],
        &[
            (6.0, 0.0),
            (15.0, 0.0),
            (-1.5, 0.866_025_403_784_438_6),
            (-1.5, 0.866_025_403_784_438_6),
            (-1.5, -0.866_025_403_784_438_6),
            (-1.5, -0.866_025_403_784_438_6),
        ],
        1.0e-12,
    );
}

#[test]
fn fft_preserves_single_and_promotes_logical() {
    let single = invoke("fft", &[single_array([1, 4], vec![1.0, 2.0, 3.0, 4.0])])
        .unwrap()
        .remove(0);
    let Value::Array(ArrayData::ComplexF32(array)) = single else {
        panic!("expected complex single FFT");
    };
    assert_eq!(array.shape().dimensions(), [1, 4]);
    assert!((array.as_slice()[1].re + 2.0).abs() <= f32::EPSILON);
    assert!((array.as_slice()[1].im - 2.0).abs() <= f32::EPSILON);

    let logical = Value::Array(ArrayData::Logical(
        DenseArray::from_vec(
            Shape::new([1, 3]).unwrap(),
            vec![
                Logical::from(true),
                Logical::from(false),
                Logical::from(true),
            ],
        )
        .unwrap(),
    ));
    let promoted = invoke("fft", &[logical]).unwrap().remove(0);
    assert_complex(
        &promoted,
        &[1, 3],
        &[
            (2.0, 0.0),
            (0.5, 0.866_025_403_784_438_6),
            (0.5, -0.866_025_403_784_438_6),
        ],
        1.0e-12,
    );
}

#[test]
fn multidimensional_transforms_match_r2022b_shapes_and_roundtrip() {
    let matrix = double_array([2, 2], vec![1.0, 3.0, 2.0, 4.0]);
    let transformed = invoke("fft2", std::slice::from_ref(&matrix))
        .unwrap()
        .remove(0);
    assert_real(&transformed, &[2, 2], &[10.0, -4.0, -2.0, 0.0], 1.0e-12);
    let inverse = invoke("ifft2", &[transformed]).unwrap().remove(0);
    assert_real(&inverse, &[2, 2], &[1.0, 3.0, 2.0, 4.0], 1.0e-12);

    let cube = double_array([2, 2, 2], (1..=8).map(f64::from).collect());
    let transformed = invoke("fftn", std::slice::from_ref(&cube))
        .unwrap()
        .remove(0);
    assert_real(
        &transformed,
        &[2, 2, 2],
        &[36.0, -4.0, -8.0, 0.0, -16.0, 0.0, 0.0, 0.0],
        1.0e-12,
    );
    let inverse = invoke("ifftn", &[transformed]).unwrap().remove(0);
    assert_real(
        &inverse,
        &[2, 2, 2],
        &(1..=8).map(f64::from).collect::<Vec<_>>(),
        1.0e-12,
    );
}

#[test]
fn fftshift_and_ifftshift_preserve_class_shape_and_odd_length_conventions() {
    let source = double_array([1, 5], vec![1.0, 2.0, 3.0, 4.0, 5.0]);
    let shifted = invoke("fftshift", std::slice::from_ref(&source))
        .unwrap()
        .remove(0);
    assert_real(&shifted, &[1, 5], &[4.0, 5.0, 1.0, 2.0, 3.0], 0.0);
    let inverse = invoke("ifftshift", &[source]).unwrap().remove(0);
    assert_real(&inverse, &[1, 5], &[3.0, 4.0, 5.0, 1.0, 2.0], 0.0);

    let shifted = invoke("fftshift", &[char_array("ABC")]).unwrap().remove(0);
    let Value::Array(ArrayData::Char(array)) = shifted else {
        panic!("fftshift must preserve char storage");
    };
    assert_eq!(
        array
            .as_slice()
            .iter()
            .map(|value| value.get())
            .collect::<Vec<_>>(),
        [u16::from(b'C'), u16::from(b'A'), u16::from(b'B')]
    );
}

#[test]
fn fft_family_preserves_empty_shapes_and_rejects_invalid_inputs() {
    let empty = double_array([0, 3], Vec::new());
    let output = invoke("fft", std::slice::from_ref(&empty))
        .unwrap()
        .remove(0);
    assert_real(&output, &[0, 3], &[], 0.0);
    let padded = invoke("fft", &[empty, Value::Double(2.0)])
        .unwrap()
        .remove(0);
    assert_real(&padded, &[2, 3], &[0.0; 6], 0.0);

    let invalid_type = invoke("fft", &[char_array("AB")]).unwrap_err();
    assert_eq!(invalid_type.category, BuiltinErrorCategory::Type);
    let invalid_length = invoke(
        "fft",
        &[double_array([1, 2], vec![1.0, 2.0]), Value::Double(2.5)],
    )
    .unwrap_err();
    assert_eq!(invalid_length.category, BuiltinErrorCategory::Domain);
    let short_size = invoke(
        "fftn",
        &[
            double_array([2, 2, 2], (1..=8).map(f64::from).collect()),
            double_array([1, 2], vec![3.0, 1.0]),
        ],
    )
    .unwrap_err();
    assert_eq!(short_size.category, BuiltinErrorCategory::Domain);
}

#[test]
fn ifft_symmetry_options_control_real_output_without_changing_values() {
    let spectrum = Value::Array(ArrayData::ComplexF64(
        DenseArray::from_vec(
            Shape::new([1, 3]).unwrap(),
            vec![
                Complex64::new(1.0, 0.0),
                Complex64::new(2.0, 0.0),
                Complex64::new(3.0, 0.0),
            ],
        )
        .unwrap(),
    ));
    let automatic = invoke("ifft", std::slice::from_ref(&spectrum))
        .unwrap()
        .remove(0);
    assert!(matches!(automatic, Value::Array(ArrayData::ComplexF64(_))));
    let symmetric = invoke("ifft", &[spectrum, char_array("symmetric")])
        .unwrap()
        .remove(0);
    assert_real(
        &symmetric,
        &[1, 3],
        &[5.0 / 3.0, -1.0 / 3.0, -1.0 / 3.0],
        1.0e-12,
    );
}

#[test]
fn fft_uses_the_injected_provider_and_maps_backend_failures() {
    let failure = invoke_with_provider(
        "fft",
        &[double_array([1, 2], vec![1.0, 2.0])],
        &RejectingProvider,
    )
    .unwrap_err();
    assert_eq!(failure.category, BuiltinErrorCategory::Other);
    assert!(failure.message.contains("provider failed"));
}
