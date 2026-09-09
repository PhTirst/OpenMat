#![doc = "Bounded MATLAB Level-5 MAT-file encoding and decoding for runtime values."]
#![deny(unsafe_code)]

mod hdf5_provider;

use std::{
    error::Error,
    fmt,
    io::{Read, Write},
    sync::atomic::AtomicBool,
};

use flate2::{Compression, read::ZlibDecoder, write::ZlibEncoder};
use openmat_array::{
    ArrayData, CharCodeUnit, Complex32, Complex64, ComplexInteger, DenseArray, IntegerArrayData,
    Logical, Shape,
};
use openmat_value::{CellArray, FieldName, StructArray, Value};

const HEADER_BYTES: usize = 128;
const MATRIX: u32 = 14;
const COMPRESSED: u32 = 15;
const FLAG_LOGICAL: u32 = 0x0200;
const FLAG_COMPLEX: u32 = 0x0800;
const MAX_RECURSION_DEPTH: usize = 64;

const MX_CELL: u8 = 1;
const MX_STRUCT: u8 = 2;
const MX_CHAR: u8 = 4;
const MX_DOUBLE: u8 = 6;
const MX_SINGLE: u8 = 7;
const MX_INT8: u8 = 8;
const MX_UINT8: u8 = 9;
const MX_INT16: u8 = 10;
const MX_UINT16: u8 = 11;
const MX_INT32: u8 = 12;
const MX_UINT32: u8 = 13;
const MX_INT64: u8 = 14;
const MX_UINT64: u8 = 15;

const MI_INT8: u32 = 1;
const MI_UINT8: u32 = 2;
const MI_INT16: u32 = 3;
const MI_UINT16: u32 = 4;
const MI_INT32: u32 = 5;
const MI_UINT32: u32 = 6;
const MI_SINGLE: u32 = 7;
const MI_DOUBLE: u32 = 9;
const MI_INT64: u32 = 12;
const MI_UINT64: u32 = 13;
const MI_UTF8: u32 = 16;
const MI_UTF16: u32 = 17;
const MI_UTF32: u32 = 18;

/// MAT-file output version supported by this project.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MatVersion {
    /// Uncompressed Level-5 elements, compatible with MATLAB -v6.
    V6,
    /// Individually zlib-compressed Level-5 elements, MATLAB's default -v7.
    #[default]
    V7,
    /// HDF5-backed MATLAB 7.3 schema with zlib compression.
    V73,
}

/// One named top-level MAT-file variable.
#[derive(Clone, Debug, PartialEq)]
pub struct MatVariable {
    /// Exact variable name.
    pub name: String,
    /// Decoded runtime value.
    pub value: Value,
}

/// A zero-based, contiguous MATLAB-dimension hyperslab.
///
/// `start` and `count` are expressed in language-visible dimension order. The
/// HDF5 provider reverses them at its boundary so callers never observe the
/// physical row-major dataset order used by MAT v7.3 files.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Mat73Hyperslab {
    start: Vec<u64>,
    count: Vec<u64>,
}

impl Mat73Hyperslab {
    /// Creates a checked contiguous hyperslab.
    ///
    /// # Errors
    ///
    /// Rejects mismatched vectors or fewer than two MATLAB dimensions.
    pub fn new(start: impl Into<Vec<u64>>, count: impl Into<Vec<u64>>) -> Result<Self, MatError> {
        let start = start.into();
        let count = count.into();
        if start.len() != count.len() {
            return Err(MatError::with_kind(
                MatErrorKind::InvalidValue,
                "MAT v7.3 hyperslab start and count must have the same dimensionality",
            ));
        }
        if start.len() < 2 {
            return Err(MatError::with_kind(
                MatErrorKind::InvalidValue,
                "MAT v7.3 hyperslabs require at least two MATLAB dimensions",
            ));
        }
        Ok(Self { start, count })
    }

    /// Returns zero-based starts in MATLAB dimension order.
    #[must_use]
    pub fn start(&self) -> &[u64] {
        &self.start
    }

    /// Returns selected lengths in MATLAB dimension order.
    #[must_use]
    pub fn count(&self) -> &[u64] {
        &self.count
    }
}

impl MatVariable {
    /// Creates a named variable.
    #[must_use]
    pub fn new(name: impl Into<String>, value: Value) -> Self {
        Self {
            name: name.into(),
            value,
        }
    }
}

/// Defensive limits applied in addition to the host filesystem byte limit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MatLimits {
    /// Maximum total bytes produced by zlib decompression.
    pub maximum_decompressed_bytes: usize,
    /// Maximum nested cell/struct depth.
    pub maximum_recursion_depth: usize,
    /// Maximum bytes accepted for one complete MAT/HDF5 file image.
    pub maximum_file_bytes: usize,
    /// Maximum cumulative decoded allocation requested by MAT v7.3 values.
    pub maximum_allocation_bytes: usize,
    /// Maximum cumulative uncompressed HDF5 dataset bytes.
    pub maximum_hdf5_decompressed_bytes: usize,
    /// Maximum HDF5 objects visited while validating one MAT v7.3 file.
    pub maximum_hdf5_objects: usize,
    /// Maximum object references followed from cell and structure values.
    pub maximum_hdf5_references: usize,
    /// Maximum elements in any one decoded dense array.
    pub maximum_array_elements: usize,
}

impl Default for MatLimits {
    fn default() -> Self {
        Self {
            maximum_decompressed_bytes: 256 * 1024 * 1024,
            maximum_recursion_depth: MAX_RECURSION_DEPTH,
            maximum_file_bytes: 256 * 1024 * 1024,
            maximum_allocation_bytes: 512 * 1024 * 1024,
            maximum_hdf5_decompressed_bytes: 256 * 1024 * 1024,
            maximum_hdf5_objects: 1_000_000,
            maximum_hdf5_references: 1_000_000,
            maximum_array_elements: 64 * 1024 * 1024,
        }
    }
}

/// Stable category for OpenMat-owned MAT diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatErrorKind {
    /// Input bytes or MATLAB schema are malformed.
    InvalidFormat,
    /// The requested MATLAB class or feature is outside this tranche.
    Unsupported,
    /// A configured resource limit was exceeded.
    LimitExceeded,
    /// Cooperative cancellation was observed.
    Cancelled,
    /// The official HDF5 library rejected an operation.
    Hdf5,
    /// A runtime value cannot be represented by the requested MAT version.
    InvalidValue,
}

/// A complete OpenMat-owned MAT-file diagnostic.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatError {
    kind: MatErrorKind,
    message: String,
}

impl MatError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            kind: MatErrorKind::InvalidFormat,
            message: message.into(),
        }
    }

    pub(crate) fn with_kind(kind: MatErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    /// Returns the stable structured category.
    #[must_use]
    pub const fn kind(&self) -> MatErrorKind {
        self.kind
    }

    /// Returns the stable human-readable detail.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for MatError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for MatError {}

/// Narrow provider boundary for the HDF5-backed MATLAB 7.3 schema.
///
/// The public MAT dispatcher does not expose HDF5 handles or a Rust ABI across
/// any process/plugin boundary. Providers receive and return owned file images.
pub trait Hdf5Provider {
    /// Decodes one validated MATLAB 7.3 file image.
    ///
    /// # Errors
    ///
    /// Returns an OpenMat-owned schema, limit, cancellation, or provider error.
    fn decode(
        &self,
        bytes: &[u8],
        limits: MatLimits,
        cancellation: Option<&AtomicBool>,
    ) -> Result<Vec<MatVariable>, MatError>;

    /// Encodes one MATLAB 7.3 file image.
    ///
    /// # Errors
    ///
    /// Returns an OpenMat-owned value, limit, cancellation, or provider error.
    fn encode(
        &self,
        variables: &[MatVariable],
        limits: MatLimits,
        cancellation: Option<&AtomicBool>,
    ) -> Result<Vec<u8>, MatError>;

    /// Reads a contiguous hyperslab from one top-level dense dataset.
    ///
    /// Providers that do not implement partial I/O return `Unsupported` by
    /// default, preserving compatibility with the first provider interface.
    ///
    /// # Errors
    ///
    /// Returns a structured provider, schema, bounds, limit, or cancellation error.
    fn read_dense_hyperslab(
        &self,
        _bytes: &[u8],
        _variable: &str,
        _selection: &Mat73Hyperslab,
        _limits: MatLimits,
        _cancellation: Option<&AtomicBool>,
    ) -> Result<Value, MatError> {
        Err(MatError::with_kind(
            MatErrorKind::Unsupported,
            "this MAT v7.3 provider does not implement dense hyperslab reads",
        ))
    }

    /// Replaces a contiguous hyperslab in one top-level dense dataset.
    ///
    /// The returned owned image leaves provider handles behind the boundary.
    ///
    /// # Errors
    ///
    /// Returns a structured provider, schema, bounds, type, limit, or cancellation error.
    fn write_dense_hyperslab(
        &self,
        _bytes: &[u8],
        _variable: &str,
        _selection: &Mat73Hyperslab,
        _value: &Value,
        _limits: MatLimits,
        _cancellation: Option<&AtomicBool>,
    ) -> Result<Vec<u8>, MatError> {
        Err(MatError::with_kind(
            MatErrorKind::Unsupported,
            "this MAT v7.3 provider does not implement dense hyperslab writes",
        ))
    }
}

/// Version-independent façade around one concrete HDF5 provider.
pub struct Mat73Backend<P> {
    provider: P,
}

impl<P> Mat73Backend<P> {
    /// Constructs a MAT 7.3 backend around the supplied provider.
    #[must_use]
    pub const fn new(provider: P) -> Self {
        Self { provider }
    }
}

impl<P: Hdf5Provider> Mat73Backend<P> {
    /// Decodes a MATLAB 7.3 file image.
    ///
    /// # Errors
    ///
    /// Propagates structured validation, limit, cancellation, and provider errors.
    pub fn decode(
        &self,
        bytes: &[u8],
        limits: MatLimits,
        cancellation: Option<&AtomicBool>,
    ) -> Result<Vec<MatVariable>, MatError> {
        self.provider.decode(bytes, limits, cancellation)
    }

    /// Encodes a MATLAB 7.3 file image.
    ///
    /// # Errors
    ///
    /// Propagates structured value, limit, cancellation, and provider errors.
    pub fn encode(
        &self,
        variables: &[MatVariable],
        limits: MatLimits,
        cancellation: Option<&AtomicBool>,
    ) -> Result<Vec<u8>, MatError> {
        self.provider.encode(variables, limits, cancellation)
    }

    /// Reads a contiguous hyperslab from a top-level dense variable.
    ///
    /// # Errors
    ///
    /// Propagates provider, schema, bounds, limit, and cancellation errors.
    pub fn read_dense_hyperslab(
        &self,
        bytes: &[u8],
        variable: &str,
        selection: &Mat73Hyperslab,
        limits: MatLimits,
        cancellation: Option<&AtomicBool>,
    ) -> Result<Value, MatError> {
        self.provider
            .read_dense_hyperslab(bytes, variable, selection, limits, cancellation)
    }

    /// Replaces a contiguous hyperslab in a top-level dense variable.
    ///
    /// # Errors
    ///
    /// Propagates provider, schema, bounds, type, limit, and cancellation errors.
    pub fn write_dense_hyperslab(
        &self,
        bytes: &[u8],
        variable: &str,
        selection: &Mat73Hyperslab,
        value: &Value,
        limits: MatLimits,
        cancellation: Option<&AtomicBool>,
    ) -> Result<Vec<u8>, MatError> {
        self.provider
            .write_dense_hyperslab(bytes, variable, selection, value, limits, cancellation)
    }
}

/// Reads a contiguous hyperslab from a top-level dense MAT v7.3 variable.
///
/// # Errors
///
/// Returns a structured schema, bounds, limit, cancellation, or HDF5 error.
pub fn read_v73_dense_hyperslab(
    bytes: &[u8],
    variable: &str,
    selection: &Mat73Hyperslab,
    limits: MatLimits,
    cancellation: Option<&AtomicBool>,
) -> Result<Value, MatError> {
    hdf5_provider::default_backend().read_dense_hyperslab(
        bytes,
        variable,
        selection,
        limits,
        cancellation,
    )
}

/// Replaces a contiguous hyperslab and returns the updated MAT v7.3 image.
///
/// # Errors
///
/// Returns a structured schema, bounds, type, limit, cancellation, or HDF5 error.
pub fn write_v73_dense_hyperslab(
    bytes: &[u8],
    variable: &str,
    selection: &Mat73Hyperslab,
    value: &Value,
    limits: MatLimits,
    cancellation: Option<&AtomicBool>,
) -> Result<Vec<u8>, MatError> {
    hdf5_provider::default_backend().write_dense_hyperslab(
        bytes,
        variable,
        selection,
        value,
        limits,
        cancellation,
    )
}

/// Encodes named values as one MATLAB Level-5 MAT-file.
///
/// # Errors
///
/// Rejects invalid names, unsupported runtime values, excessive nesting, and
/// any allocation or compression failure.
pub fn encode(variables: &[MatVariable], version: MatVersion) -> Result<Vec<u8>, MatError> {
    encode_cancellable(variables, version, MatLimits::default(), None)
}

/// Encodes named values while enforcing explicit limits and cooperative cancellation.
///
/// # Errors
///
/// Returns a structured MAT error for unsupported values, HDF5 failures, limits,
/// or cancellation.
pub fn encode_cancellable(
    variables: &[MatVariable],
    version: MatVersion,
    limits: MatLimits,
    cancellation: Option<&AtomicBool>,
) -> Result<Vec<u8>, MatError> {
    if version == MatVersion::V73 {
        return hdf5_provider::default_backend().encode(variables, limits, cancellation);
    }
    let mut output = header();
    for variable in variables {
        validate_name(&variable.name).map_err(|error| variable_error(&variable.name, &error))?;
        let matrix = encode_matrix(&variable.name, &variable.value, 0)
            .map_err(|error| variable_error(&variable.name, &error))?;
        match version {
            MatVersion::V6 => output.extend_from_slice(&matrix),
            MatVersion::V7 => {
                let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
                encoder.write_all(&matrix).map_err(|error| {
                    variable_error(
                        &variable.name,
                        &MatError::new(format!("MAT v7 compression failed: {error}")),
                    )
                })?;
                let compressed = encoder.finish().map_err(|error| {
                    variable_error(
                        &variable.name,
                        &MatError::new(format!("MAT v7 compression finalization failed: {error}")),
                    )
                })?;
                write_tagged(&mut output, COMPRESSED, &compressed, false)
                    .map_err(|error| variable_error(&variable.name, &error))?;
            }
            MatVersion::V73 => unreachable!("v7.3 is dispatched before Level-5 encoding"),
        }
    }
    Ok(output)
}

fn variable_error(name: &str, error: &MatError) -> MatError {
    MatError::new(format!("variable '{name}': {error}"))
}

/// Decodes supported values from a MATLAB Level-5/v6/v7 or 7.3 MAT-file.
///
/// # Errors
///
/// Rejects truncated, malformed, unsupported, over-deep, or over-limit files.
/// Ordinary HDF5 files without MATLAB's 7.3 user-block schema are rejected.
pub fn decode(bytes: &[u8], limits: MatLimits) -> Result<Vec<MatVariable>, MatError> {
    decode_cancellable(bytes, limits, None)
}

/// Returns whether the file image carries the MATLAB 7.3/HDF5 signature.
#[must_use]
pub fn is_v73(bytes: &[u8]) -> bool {
    hdf5_provider::is_mat73(bytes)
}

/// Automatically decodes Level-5 or MATLAB 7.3 bytes with cancellation.
///
/// # Errors
///
/// Rejects malformed, unsupported, cyclic, cancelled, or over-limit files.
pub fn decode_cancellable(
    bytes: &[u8],
    limits: MatLimits,
    cancellation: Option<&AtomicBool>,
) -> Result<Vec<MatVariable>, MatError> {
    if hdf5_provider::is_mat73(bytes) {
        return hdf5_provider::default_backend().decode(bytes, limits, cancellation);
    }
    let endian = parse_header(bytes)?;
    let mut cursor = Cursor::new(&bytes[HEADER_BYTES..], endian);
    let mut variables = Vec::new();
    let mut decompressed_total = 0usize;
    while !cursor.is_empty() {
        if cursor.remaining().iter().all(|byte| *byte == 0) {
            break;
        }
        let element = cursor.element(false)?;
        match element.kind {
            MATRIX => {
                let (name, value) = decode_matrix(element.data, endian, limits, 0)?;
                variables.push(MatVariable::new(name, value));
            }
            COMPRESSED => {
                let remaining = limits
                    .maximum_decompressed_bytes
                    .checked_sub(decompressed_total)
                    .ok_or_else(|| MatError::new("MAT-file decompression limit was exceeded"))?;
                let decompressed = decompress_bounded(element.data, remaining)?;
                decompressed_total = decompressed_total
                    .checked_add(decompressed.len())
                    .ok_or_else(|| MatError::new("MAT-file decompression size overflow"))?;
                let mut nested = Cursor::new(&decompressed, endian);
                while !nested.is_empty() {
                    if nested.remaining().iter().all(|byte| *byte == 0) {
                        break;
                    }
                    let matrix = nested.element(true)?;
                    if matrix.kind != MATRIX {
                        return Err(MatError::new(format!(
                            "compressed MAT element contains unsupported data type {}",
                            matrix.kind
                        )));
                    }
                    let (name, value) = decode_matrix(matrix.data, endian, limits, 0)?;
                    variables.push(MatVariable::new(name, value));
                }
            }
            other => {
                return Err(MatError::new(format!(
                    "unsupported top-level MAT data type {other}"
                )));
            }
        }
    }
    Ok(variables)
}

fn header() -> Vec<u8> {
    let mut header = vec![b' '; HEADER_BYTES];
    let description = b"MATLAB 5.0 MAT-file, Created by OpenMat";
    header[..description.len()].copy_from_slice(description);
    header[116..124].fill(0);
    header[124] = 0;
    header[125] = 1;
    header[126] = b'I';
    header[127] = b'M';
    header
}

fn parse_header(bytes: &[u8]) -> Result<Endian, MatError> {
    if bytes.starts_with(b"\x89HDF\r\n\x1a\n") {
        return Err(MatError::new(
            "HDF5-backed MAT v7.3 files are not supported by this Level-5 reader",
        ));
    }
    if bytes.len() < HEADER_BYTES {
        return Err(MatError::new(format!(
            "MAT-file header is truncated: expected 128 bytes, received {}",
            bytes.len()
        )));
    }
    if !bytes[..116].starts_with(b"MATLAB 5.0 MAT-file") {
        return Err(MatError::new("input is not a MATLAB Level-5 MAT-file"));
    }
    match &bytes[126..128] {
        b"IM" => Ok(Endian::Little),
        b"MI" => Ok(Endian::Big),
        marker => Err(MatError::new(format!(
            "invalid MAT-file endian marker {:02X}{:02X}",
            marker[0], marker[1]
        ))),
    }
}

fn validate_name(name: &str) -> Result<(), MatError> {
    if name.is_empty() || name.as_bytes().contains(&0) {
        Err(MatError::new(
            "MAT variable names must be nonempty and contain no NUL",
        ))
    } else {
        Ok(())
    }
}

fn encode_matrix(name: &str, value: &Value, depth: usize) -> Result<Vec<u8>, MatError> {
    if depth > MAX_RECURSION_DEPTH {
        return Err(MatError::new("MAT value nesting exceeds 64 levels"));
    }
    let mut payload = Vec::new();
    let (class, flags) = value_class_and_flags(value)?;
    let mut flag_data = Vec::with_capacity(8);
    write_u32(&mut flag_data, u32::from(class) | flags);
    write_u32(&mut flag_data, 0);
    write_tagged(&mut payload, MI_UINT32, &flag_data, true)?;

    let dimensions = value
        .dimensions()
        .ok_or_else(|| unsupported_value(value, "has no MATLAB array shape"))?;
    let mut dimension_data = Vec::with_capacity(dimensions.len().saturating_mul(4));
    for dimension in dimensions {
        let dimension = i32::try_from(*dimension).map_err(|_| {
            MatError::new(format!(
                "MAT dimension {dimension} exceeds the Level-5 signed 32-bit limit"
            ))
        })?;
        write_i32(&mut dimension_data, dimension);
    }
    write_tagged(&mut payload, MI_INT32, &dimension_data, true)?;
    write_tagged(&mut payload, MI_INT8, name.as_bytes(), true)?;

    match value {
        Value::Logical(value) => {
            write_tagged(&mut payload, MI_UINT8, &[u8::from(*value)], true)?;
        }
        Value::Double(value) => {
            write_numeric(&mut payload, MI_DOUBLE, &[*value], f64::to_le_bytes)?;
        }
        Value::Complex(value) => {
            write_numeric(&mut payload, MI_DOUBLE, &[value.real], f64::to_le_bytes)?;
            write_numeric(
                &mut payload,
                MI_DOUBLE,
                &[value.imaginary],
                f64::to_le_bytes,
            )?;
        }
        Value::Array(array) => encode_array(&mut payload, array)?,
        Value::Cell(cell) => {
            for value in cell.values() {
                payload.extend_from_slice(&encode_matrix("", value, depth + 1)?);
            }
        }
        Value::Struct(structure) => encode_struct(&mut payload, structure, depth + 1)?,
        Value::Nothing
        | Value::Sparse(_)
        | Value::String(_)
        | Value::Table(_)
        | Value::Object(_)
        | Value::ObjectArray(_)
        | Value::Graphics(_)
        | Value::GraphicsArray(_)
        | Value::Function(_) => return Err(unsupported_value(value, "is not serializable")),
    }

    let mut matrix = Vec::new();
    write_tagged(&mut matrix, MATRIX, &payload, true)?;
    Ok(matrix)
}

fn value_class_and_flags(value: &Value) -> Result<(u8, u32), MatError> {
    let result = match value {
        Value::Logical(_) | Value::Array(ArrayData::Logical(_)) => (MX_UINT8, FLAG_LOGICAL),
        Value::Double(_) | Value::Array(ArrayData::F64(_)) => (MX_DOUBLE, 0),
        Value::Complex(_) | Value::Array(ArrayData::ComplexF64(_)) => (MX_DOUBLE, FLAG_COMPLEX),
        Value::Array(ArrayData::F32(_)) => (MX_SINGLE, 0),
        Value::Array(ArrayData::ComplexF32(_)) => (MX_SINGLE, FLAG_COMPLEX),
        Value::Array(ArrayData::Char(_)) => (MX_CHAR, 0),
        Value::Array(ArrayData::Integer(integer)) => integer_class_and_flags(integer),
        Value::Cell(_) => (MX_CELL, 0),
        Value::Struct(_) => (MX_STRUCT, 0),
        _ => return Err(unsupported_value(value, "has no Level-5 class mapping")),
    };
    Ok(result)
}

fn integer_class_and_flags(value: &IntegerArrayData) -> (u8, u32) {
    let flags = if value.is_complex() { FLAG_COMPLEX } else { 0 };
    let class = match value {
        IntegerArrayData::I8(_) | IntegerArrayData::ComplexI8(_) => MX_INT8,
        IntegerArrayData::U8(_) | IntegerArrayData::ComplexU8(_) => MX_UINT8,
        IntegerArrayData::I16(_) | IntegerArrayData::ComplexI16(_) => MX_INT16,
        IntegerArrayData::U16(_) | IntegerArrayData::ComplexU16(_) => MX_UINT16,
        IntegerArrayData::I32(_) | IntegerArrayData::ComplexI32(_) => MX_INT32,
        IntegerArrayData::U32(_) | IntegerArrayData::ComplexU32(_) => MX_UINT32,
        IntegerArrayData::I64(_) | IntegerArrayData::ComplexI64(_) => MX_INT64,
        IntegerArrayData::U64(_) | IntegerArrayData::ComplexU64(_) => MX_UINT64,
    };
    (class, flags)
}

fn encode_array(payload: &mut Vec<u8>, value: &ArrayData) -> Result<(), MatError> {
    match value {
        ArrayData::F32(array) => {
            write_numeric(payload, MI_SINGLE, array.as_slice(), f32::to_le_bytes)
        }
        ArrayData::ComplexF32(array) => {
            write_numeric_iter(
                payload,
                MI_SINGLE,
                array.as_slice().iter().map(|value| value.re),
                f32::to_le_bytes,
            )?;
            write_numeric_iter(
                payload,
                MI_SINGLE,
                array.as_slice().iter().map(|value| value.im),
                f32::to_le_bytes,
            )
        }

        ArrayData::F64(array) => {
            write_numeric(payload, MI_DOUBLE, array.as_slice(), f64::to_le_bytes)
        }
        ArrayData::ComplexF64(array) => {
            write_numeric_iter(
                payload,
                MI_DOUBLE,
                array.as_slice().iter().map(|value| value.re),
                f64::to_le_bytes,
            )?;
            write_numeric_iter(
                payload,
                MI_DOUBLE,
                array.as_slice().iter().map(|value| value.im),
                f64::to_le_bytes,
            )
        }
        ArrayData::Logical(array) => {
            let bytes = array
                .as_slice()
                .iter()
                .map(|value| u8::from(value.get()))
                .collect::<Vec<_>>();
            write_tagged(payload, MI_UINT8, &bytes, true)
        }
        ArrayData::Char(array) => write_numeric_iter(
            payload,
            MI_UTF16,
            array.as_slice().iter().map(|value| value.get()),
            u16::to_le_bytes,
        ),
        ArrayData::Integer(integer) => encode_integer(payload, integer),
    }
}

macro_rules! encode_real_integer {
    ($payload:expr, $array:expr, $kind:expr, $to_bytes:path) => {
        write_numeric($payload, $kind, $array.as_slice(), $to_bytes)
    };
}

macro_rules! encode_complex_integer {
    ($payload:expr, $array:expr, $kind:expr, $to_bytes:path) => {{
        write_numeric_iter(
            $payload,
            $kind,
            $array.as_slice().iter().map(|value| *value.real()),
            $to_bytes,
        )?;
        write_numeric_iter(
            $payload,
            $kind,
            $array.as_slice().iter().map(|value| *value.imaginary()),
            $to_bytes,
        )
    }};
}

fn encode_integer(payload: &mut Vec<u8>, value: &IntegerArrayData) -> Result<(), MatError> {
    match value {
        IntegerArrayData::I8(array) => {
            let bytes = array
                .as_slice()
                .iter()
                .map(|value| value.to_le_bytes()[0])
                .collect::<Vec<_>>();
            write_tagged(payload, MI_INT8, &bytes, true)
        }
        IntegerArrayData::ComplexI8(array) => {
            let real = array
                .as_slice()
                .iter()
                .map(|value| value.real().to_le_bytes()[0])
                .collect::<Vec<_>>();
            let imaginary = array
                .as_slice()
                .iter()
                .map(|value| value.imaginary().to_le_bytes()[0])
                .collect::<Vec<_>>();
            write_tagged(payload, MI_INT8, &real, true)?;
            write_tagged(payload, MI_INT8, &imaginary, true)
        }
        IntegerArrayData::U8(array) => write_tagged(payload, MI_UINT8, array.as_slice(), true),
        IntegerArrayData::ComplexU8(array) => {
            let real = array
                .as_slice()
                .iter()
                .map(|value| *value.real())
                .collect::<Vec<_>>();
            let imaginary = array
                .as_slice()
                .iter()
                .map(|value| *value.imaginary())
                .collect::<Vec<_>>();
            write_tagged(payload, MI_UINT8, &real, true)?;
            write_tagged(payload, MI_UINT8, &imaginary, true)
        }
        IntegerArrayData::I16(array) => {
            encode_real_integer!(payload, array, MI_INT16, i16::to_le_bytes)
        }
        IntegerArrayData::ComplexI16(array) => {
            encode_complex_integer!(payload, array, MI_INT16, i16::to_le_bytes)
        }
        IntegerArrayData::U16(array) => {
            encode_real_integer!(payload, array, MI_UINT16, u16::to_le_bytes)
        }
        IntegerArrayData::ComplexU16(array) => {
            encode_complex_integer!(payload, array, MI_UINT16, u16::to_le_bytes)
        }
        IntegerArrayData::I32(array) => {
            encode_real_integer!(payload, array, MI_INT32, i32::to_le_bytes)
        }
        IntegerArrayData::ComplexI32(array) => {
            encode_complex_integer!(payload, array, MI_INT32, i32::to_le_bytes)
        }
        IntegerArrayData::U32(array) => {
            encode_real_integer!(payload, array, MI_UINT32, u32::to_le_bytes)
        }
        IntegerArrayData::ComplexU32(array) => {
            encode_complex_integer!(payload, array, MI_UINT32, u32::to_le_bytes)
        }
        IntegerArrayData::I64(array) => {
            encode_real_integer!(payload, array, MI_INT64, i64::to_le_bytes)
        }
        IntegerArrayData::ComplexI64(array) => {
            encode_complex_integer!(payload, array, MI_INT64, i64::to_le_bytes)
        }
        IntegerArrayData::U64(array) => {
            encode_real_integer!(payload, array, MI_UINT64, u64::to_le_bytes)
        }
        IntegerArrayData::ComplexU64(array) => {
            encode_complex_integer!(payload, array, MI_UINT64, u64::to_le_bytes)
        }
    }
}

fn encode_struct(payload: &mut Vec<u8>, value: &StructArray, depth: usize) -> Result<(), MatError> {
    let field_width = value
        .field_names()
        .iter()
        .map(|name| name.as_str().len().saturating_add(1))
        .max()
        .unwrap_or(1);
    let field_width = u32::try_from(field_width)
        .map_err(|_| MatError::new("MAT struct field name is too long"))?;
    let mut width = Vec::with_capacity(4);
    write_u32(&mut width, field_width);
    write_tagged(payload, MI_INT32, &width, true)?;

    let field_width_usize = usize::try_from(field_width)
        .map_err(|_| MatError::new("MAT struct field-name width is not representable"))?;
    let field_bytes_len = field_width_usize
        .checked_mul(value.field_count())
        .ok_or_else(|| MatError::new("MAT struct field-name table is too large"))?;
    let mut field_bytes = vec![0; field_bytes_len];
    for (field, name) in value.field_names().iter().enumerate() {
        let start = field
            .checked_mul(field_width_usize)
            .ok_or_else(|| MatError::new("MAT struct field-name offset overflow"))?;
        let bytes = name.as_str().as_bytes();
        field_bytes[start..start + bytes.len()].copy_from_slice(bytes);
    }
    write_tagged(payload, MI_INT8, &field_bytes, true)?;

    for field in 0..value.field_count() {
        let values = value
            .field_values(field)
            .ok_or_else(|| MatError::new("MAT struct field storage is inconsistent"))?;
        for nested in values {
            payload.extend_from_slice(&encode_matrix("", nested, depth)?);
        }
    }
    Ok(())
}

fn unsupported_value(value: &Value, reason: &str) -> MatError {
    MatError::new(format!(
        "OpenMat value of class '{}' {reason} in MAT Level-5",
        value.class_name()
    ))
}

fn write_numeric<T: Copy, const N: usize>(
    output: &mut Vec<u8>,
    kind: u32,
    values: &[T],
    to_bytes: impl Fn(T) -> [u8; N],
) -> Result<(), MatError> {
    write_numeric_iter(output, kind, values.iter().copied(), to_bytes)
}

fn write_numeric_iter<T, const N: usize>(
    output: &mut Vec<u8>,
    kind: u32,
    values: impl IntoIterator<Item = T>,
    to_bytes: impl Fn(T) -> [u8; N],
) -> Result<(), MatError> {
    let values = values.into_iter();
    let (lower, upper) = values.size_hint();
    let capacity = upper.unwrap_or(lower).checked_mul(N).unwrap_or(0);
    let mut bytes = Vec::with_capacity(capacity);
    for value in values {
        bytes.extend_from_slice(&to_bytes(value));
    }
    write_tagged(output, kind, &bytes, true)
}

fn write_tagged(output: &mut Vec<u8>, kind: u32, data: &[u8], pad: bool) -> Result<(), MatError> {
    let length = u32::try_from(data.len())
        .map_err(|_| MatError::new("MAT data element exceeds the Level-5 32-bit byte limit"))?;
    write_u32(output, kind);
    write_u32(output, length);
    output.extend_from_slice(data);
    if pad {
        output.resize(
            output
                .len()
                .checked_add(padding(data.len()))
                .ok_or_else(|| MatError::new("MAT output size overflow"))?,
            0,
        );
    }
    Ok(())
}

const fn padding(length: usize) -> usize {
    (8 - length % 8) % 8
}

fn write_u32(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn write_i32(output: &mut Vec<u8>, value: i32) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn decompress_bounded(data: &[u8], maximum: usize) -> Result<Vec<u8>, MatError> {
    let take = u64::try_from(maximum).unwrap_or(u64::MAX).saturating_add(1);
    let mut decoder = ZlibDecoder::new(data).take(take);
    let mut output = Vec::new();
    decoder
        .read_to_end(&mut output)
        .map_err(|error| MatError::new(format!("invalid MAT v7 zlib stream: {error}")))?;
    if output.len() > maximum {
        return Err(MatError::new(format!(
            "MAT-file decompression exceeds the {maximum}-byte limit"
        )));
    }
    Ok(output)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Endian {
    Little,
    Big,
}

impl Endian {
    fn u16(self, bytes: [u8; 2]) -> u16 {
        match self {
            Self::Little => u16::from_le_bytes(bytes),
            Self::Big => u16::from_be_bytes(bytes),
        }
    }

    fn i16(self, bytes: [u8; 2]) -> i16 {
        match self {
            Self::Little => i16::from_le_bytes(bytes),
            Self::Big => i16::from_be_bytes(bytes),
        }
    }

    fn u32(self, bytes: [u8; 4]) -> u32 {
        match self {
            Self::Little => u32::from_le_bytes(bytes),
            Self::Big => u32::from_be_bytes(bytes),
        }
    }

    fn i32(self, bytes: [u8; 4]) -> i32 {
        match self {
            Self::Little => i32::from_le_bytes(bytes),
            Self::Big => i32::from_be_bytes(bytes),
        }
    }

    fn u64(self, bytes: [u8; 8]) -> u64 {
        match self {
            Self::Little => u64::from_le_bytes(bytes),
            Self::Big => u64::from_be_bytes(bytes),
        }
    }

    fn i64(self, bytes: [u8; 8]) -> i64 {
        match self {
            Self::Little => i64::from_le_bytes(bytes),
            Self::Big => i64::from_be_bytes(bytes),
        }
    }
}

#[derive(Clone, Copy)]
struct Element<'a> {
    kind: u32,
    data: &'a [u8],
}

struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
    endian: Endian,
}

impl<'a> Cursor<'a> {
    const fn new(bytes: &'a [u8], endian: Endian) -> Self {
        Self {
            bytes,
            offset: 0,
            endian,
        }
    }

    fn is_empty(&self) -> bool {
        self.offset == self.bytes.len()
    }

    fn remaining(&self) -> &'a [u8] {
        &self.bytes[self.offset..]
    }

    fn element(&mut self, pad_standard: bool) -> Result<Element<'a>, MatError> {
        let tag = self.take(4)?;
        let first = self.endian.u32(tag.try_into().expect("four-byte tag"));
        let small_kind = first & 0xffff;
        let small_length = usize::try_from(first >> 16).unwrap_or(usize::MAX);
        if small_length != 0 {
            if small_length > 4 {
                return Err(MatError::new(format!(
                    "invalid small MAT element length {small_length}"
                )));
            }
            let storage = self.take(4)?;
            return Ok(Element {
                kind: small_kind,
                data: &storage[..small_length],
            });
        }

        let length_bytes = self.take(4)?;
        let length = usize::try_from(
            self.endian
                .u32(length_bytes.try_into().expect("four-byte length")),
        )
        .map_err(|_| MatError::new("MAT element length is not representable"))?;
        let data = self.take(length)?;
        if pad_standard || first != COMPRESSED {
            self.take(padding(length))?;
        } else {
            let optional_padding = padding(length);
            if self
                .remaining()
                .get(..optional_padding)
                .is_some_and(|bytes| bytes.iter().all(|byte| *byte == 0))
            {
                self.take(optional_padding)?;
            }
        }
        Ok(Element { kind: first, data })
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], MatError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or_else(|| MatError::new("MAT cursor offset overflow"))?;
        let value = self.bytes.get(self.offset..end).ok_or_else(|| {
            MatError::new(format!(
                "MAT data element is truncated at byte {} while reading {length} bytes",
                self.offset
            ))
        })?;
        self.offset = end;
        Ok(value)
    }
}

macro_rules! decode_integer {
    (
        $cursor:expr,
        $endian:expr,
        $shape:expr,
        $expected:expr,
        $complex:expr,
        $ty:ty,
        $real:ident,
        $complex_variant:ident
    ) => {{
        let real = numeric_as_integer::<$ty>($cursor.element(true)?, $endian, $expected)?;
        let integer = if $complex {
            let imaginary = numeric_as_integer::<$ty>($cursor.element(true)?, $endian, $expected)?;
            let values = real
                .into_iter()
                .zip(imaginary)
                .map(|(re, im)| ComplexInteger::new(re, im))
                .collect();
            IntegerArrayData::$complex_variant(
                DenseArray::from_vec($shape, values).map_err(array_error)?,
            )
        } else {
            IntegerArrayData::$real(DenseArray::from_vec($shape, real).map_err(array_error)?)
        };
        Value::Array(ArrayData::Integer(integer))
    }};
}

#[allow(clippy::too_many_lines)]
fn decode_matrix(
    bytes: &[u8],
    endian: Endian,
    limits: MatLimits,
    depth: usize,
) -> Result<(String, Value), MatError> {
    if depth > limits.maximum_recursion_depth {
        return Err(MatError::new(format!(
            "MAT value nesting exceeds the configured {}-level limit",
            limits.maximum_recursion_depth
        )));
    }
    let mut cursor = Cursor::new(bytes, endian);
    let flags = cursor.element(true)?;
    expect_kind(flags, MI_UINT32, "array flags")?;
    if flags.data.len() < 8 {
        return Err(MatError::new(
            "MAT array flags element is shorter than 8 bytes",
        ));
    }
    let flag_word = endian.u32(flags.data[..4].try_into().expect("four flag bytes"));
    let class = u8::try_from(flag_word & 0xff).expect("masked class fits u8");
    let logical = flag_word & FLAG_LOGICAL != 0;
    let complex = flag_word & FLAG_COMPLEX != 0;

    let dimensions = cursor.element(true)?;
    expect_kind(dimensions, MI_INT32, "array dimensions")?;
    if !dimensions.data.len().is_multiple_of(4) {
        return Err(MatError::new(
            "MAT dimension data is not a sequence of int32 values",
        ));
    }
    let dimensions = dimensions
        .data
        .chunks_exact(4)
        .map(|bytes| endian.i32(bytes.try_into().expect("four dimension bytes")))
        .map(|dimension| {
            u64::try_from(dimension)
                .map_err(|_| MatError::new(format!("negative MAT dimension {dimension}")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let shape = Shape::new(dimensions)
        .map_err(|error| MatError::new(format!("invalid MAT array shape: {error}")))?;
    let expected = usize::try_from(shape.numel())
        .map_err(|_| MatError::new("MAT array element count is not representable"))?;

    let name = cursor.element(true)?;
    if !matches!(name.kind, MI_INT8 | MI_UINT8 | MI_UTF8) {
        return Err(MatError::new(format!(
            "unsupported MAT variable-name data type {}",
            name.kind
        )));
    }
    let name = String::from_utf8(name.data.to_vec())
        .map_err(|_| MatError::new("MAT variable name is not valid UTF-8"))?;

    let value = match class {
        MX_CELL => decode_cell(&mut cursor, endian, limits, depth + 1, shape, expected)?,
        MX_STRUCT => decode_struct(&mut cursor, endian, limits, depth + 1, shape, expected)?,
        MX_CHAR => decode_char(&mut cursor, endian, shape, expected)?,
        MX_DOUBLE => decode_double(&mut cursor, endian, shape, expected, complex)?,
        MX_SINGLE => decode_single(&mut cursor, endian, shape, expected, complex)?,
        MX_INT8 => {
            decode_integer!(cursor, endian, shape, expected, complex, i8, I8, ComplexI8)
        }
        MX_UINT8 if logical => decode_logical(&mut cursor, endian, shape, expected)?,
        MX_UINT8 => {
            decode_integer!(cursor, endian, shape, expected, complex, u8, U8, ComplexU8)
        }
        MX_INT16 => {
            decode_integer!(
                cursor, endian, shape, expected, complex, i16, I16, ComplexI16
            )
        }
        MX_UINT16 => {
            decode_integer!(
                cursor, endian, shape, expected, complex, u16, U16, ComplexU16
            )
        }
        MX_INT32 => {
            decode_integer!(
                cursor, endian, shape, expected, complex, i32, I32, ComplexI32
            )
        }
        MX_UINT32 => {
            decode_integer!(
                cursor, endian, shape, expected, complex, u32, U32, ComplexU32
            )
        }
        MX_INT64 => {
            decode_integer!(
                cursor, endian, shape, expected, complex, i64, I64, ComplexI64
            )
        }
        MX_UINT64 => {
            decode_integer!(
                cursor, endian, shape, expected, complex, u64, U64, ComplexU64
            )
        }
        other => {
            return Err(MatError::new(format!(
                "unsupported MAT array class code {other} for variable '{name}'"
            )));
        }
    };
    if !cursor.is_empty() && cursor.remaining().iter().any(|byte| *byte != 0) {
        return Err(MatError::new(format!(
            "MAT matrix '{name}' contains unexpected trailing data"
        )));
    }
    Ok((name, value))
}

fn decode_double(
    cursor: &mut Cursor<'_>,
    endian: Endian,
    shape: Shape,
    expected: usize,
    complex: bool,
) -> Result<Value, MatError> {
    let real = numeric_as_f64(cursor.element(true)?, endian, expected)?;
    if complex {
        let imaginary = numeric_as_f64(cursor.element(true)?, endian, expected)?;
        if expected == 1 {
            return Ok(Value::Complex(openmat_value::Complex64::new(
                real[0],
                imaginary[0],
            )));
        }
        let values = real
            .into_iter()
            .zip(imaginary)
            .map(|(re, im)| Complex64::new(re, im))
            .collect();
        DenseArray::from_vec(shape, values)
            .map(ArrayData::ComplexF64)
            .map(Value::Array)
            .map_err(array_error)
    } else {
        if expected == 1 {
            return Ok(Value::Double(real[0]));
        }
        DenseArray::from_vec(shape, real)
            .map(ArrayData::F64)
            .map(Value::Array)
            .map_err(array_error)
    }
}

fn decode_single(
    cursor: &mut Cursor<'_>,
    endian: Endian,
    shape: Shape,
    expected: usize,
    complex: bool,
) -> Result<Value, MatError> {
    let real = numeric_as_f32(cursor.element(true)?, endian, expected)?;
    if complex {
        let imaginary = numeric_as_f32(cursor.element(true)?, endian, expected)?;
        let values = real
            .into_iter()
            .zip(imaginary)
            .map(|(re, im)| Complex32::new(re, im))
            .collect();
        DenseArray::from_vec(shape, values)
            .map(ArrayData::ComplexF32)
            .map(Value::Array)
            .map_err(array_error)
    } else {
        DenseArray::from_vec(shape, real)
            .map(ArrayData::F32)
            .map(Value::Array)
            .map_err(array_error)
    }
}

fn decode_logical(
    cursor: &mut Cursor<'_>,
    endian: Endian,
    shape: Shape,
    expected: usize,
) -> Result<Value, MatError> {
    let values = numeric_as_u64(cursor.element(true)?, endian, expected)?
        .into_iter()
        .map(|value| Logical::from(value != 0))
        .collect::<Vec<_>>();
    if expected == 1 {
        return Ok(Value::Logical(values[0].get()));
    }
    DenseArray::from_vec(shape, values)
        .map(ArrayData::Logical)
        .map(Value::Array)
        .map_err(array_error)
}

fn decode_char(
    cursor: &mut Cursor<'_>,
    endian: Endian,
    shape: Shape,
    expected: usize,
) -> Result<Value, MatError> {
    let element = cursor.element(true)?;
    let values = match element.kind {
        MI_UTF16 | MI_UINT16 => chunks_2(element.data, endian, expected)?
            .into_iter()
            .map(CharCodeUnit::new)
            .collect(),
        MI_UTF8 | MI_UINT8 => {
            let text = String::from_utf8(element.data.to_vec())
                .map_err(|_| MatError::new("MAT UTF-8 char payload is invalid"))?;
            let values = text
                .encode_utf16()
                .map(CharCodeUnit::new)
                .collect::<Vec<_>>();
            if values.len() != expected {
                return Err(length_error(expected, values.len(), "char"));
            }
            values
        }
        MI_UTF32 => {
            let scalars = chunks_4_u32(element.data, endian, element.data.len() / 4)?;
            let mut code_units = Vec::new();
            for value in scalars {
                let character = char::from_u32(value)
                    .ok_or_else(|| MatError::new(format!("invalid MAT UTF-32 scalar {value}")))?;
                let mut buffer = [0; 2];
                code_units.extend(character.encode_utf16(&mut buffer).iter().copied());
            }
            if code_units.len() != expected {
                return Err(length_error(expected, code_units.len(), "char"));
            }
            code_units.into_iter().map(CharCodeUnit::new).collect()
        }
        other => {
            return Err(MatError::new(format!(
                "unsupported MAT char payload type {other}"
            )));
        }
    };
    DenseArray::from_vec(shape, values)
        .map(ArrayData::Char)
        .map(Value::Array)
        .map_err(array_error)
}

fn decode_cell(
    cursor: &mut Cursor<'_>,
    endian: Endian,
    limits: MatLimits,
    depth: usize,
    shape: Shape,
    expected: usize,
) -> Result<Value, MatError> {
    let mut values = Vec::with_capacity(expected);
    for _ in 0..expected {
        let element = cursor.element(true)?;
        expect_kind(element, MATRIX, "cell value")?;
        let (_, value) = decode_matrix(element.data, endian, limits, depth)?;
        values.push(value);
    }
    CellArray::from_values(shape, values)
        .map(Value::Cell)
        .map_err(|error| MatError::new(format!("invalid MAT cell array: {error}")))
}

fn decode_struct(
    cursor: &mut Cursor<'_>,
    endian: Endian,
    limits: MatLimits,
    depth: usize,
    shape: Shape,
    expected: usize,
) -> Result<Value, MatError> {
    let width = cursor.element(true)?;
    if !matches!(width.kind, MI_INT32 | MI_UINT32) || width.data.len() != 4 {
        return Err(MatError::new(
            "MAT struct field-name width is not one int32",
        ));
    }
    let width = usize::try_from(endian.u32(width.data.try_into().expect("four width bytes")))
        .map_err(|_| MatError::new("MAT struct field-name width is not representable"))?;
    if width == 0 {
        return Err(MatError::new("MAT struct field-name width is zero"));
    }
    let names = cursor.element(true)?;
    if !matches!(names.kind, MI_INT8 | MI_UINT8 | MI_UTF8)
        || !names.data.len().is_multiple_of(width)
    {
        return Err(MatError::new("MAT struct field-name table is malformed"));
    }
    let names = names
        .data
        .chunks_exact(width)
        .map(|bytes| {
            let end = bytes
                .iter()
                .position(|byte| *byte == 0)
                .unwrap_or(bytes.len());
            let name = String::from_utf8(bytes[..end].to_vec())
                .map_err(|_| MatError::new("MAT struct field name is not valid UTF-8"))?;
            FieldName::new(name)
                .map_err(|error| MatError::new(format!("invalid MAT struct field name: {error}")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut columns = Vec::with_capacity(names.len());
    for _ in 0..names.len() {
        let mut column = Vec::with_capacity(expected);
        for _ in 0..expected {
            let element = cursor.element(true)?;
            expect_kind(element, MATRIX, "struct field value")?;
            let (_, value) = decode_matrix(element.data, endian, limits, depth)?;
            column.push(value);
        }
        columns.push(column);
    }
    StructArray::from_columns(shape, names, columns)
        .map(Value::Struct)
        .map_err(|error| MatError::new(format!("invalid MAT struct array: {error}")))
}

trait MatInteger: Copy {
    fn from_i128(value: i128) -> Option<Self>;
    fn from_u128(value: u128) -> Option<Self>;
}

macro_rules! impl_mat_integer {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl MatInteger for $ty {
                fn from_i128(value: i128) -> Option<Self> {
                    Self::try_from(value).ok()
                }

                fn from_u128(value: u128) -> Option<Self> {
                    Self::try_from(value).ok()
                }
            }
        )+
    };
}

impl_mat_integer!(i8, u8, i16, u16, i32, u32, i64, u64);

fn numeric_as_integer<T: MatInteger>(
    element: Element<'_>,
    endian: Endian,
    expected: usize,
) -> Result<Vec<T>, MatError> {
    let converted: Vec<Option<T>> = match element.kind {
        MI_INT8 => exact_bytes(element.data, expected)?
            .into_iter()
            .map(|value| T::from_i128(i128::from(i8::from_ne_bytes([value]))))
            .collect(),
        MI_UINT8 => exact_bytes(element.data, expected)?
            .into_iter()
            .map(|value| T::from_u128(u128::from(value)))
            .collect(),
        MI_INT16 => chunks_2_i16(element.data, endian, expected)?
            .into_iter()
            .map(|value| T::from_i128(i128::from(value)))
            .collect(),
        MI_UINT16 => chunks_2(element.data, endian, expected)?
            .into_iter()
            .map(|value| T::from_u128(u128::from(value)))
            .collect(),
        MI_INT32 => chunks_4_i32(element.data, endian, expected)?
            .into_iter()
            .map(|value| T::from_i128(i128::from(value)))
            .collect(),
        MI_UINT32 => chunks_4_u32(element.data, endian, expected)?
            .into_iter()
            .map(|value| T::from_u128(u128::from(value)))
            .collect(),
        MI_INT64 => chunks_8_i64(element.data, endian, expected)?
            .into_iter()
            .map(|value| T::from_i128(i128::from(value)))
            .collect(),
        MI_UINT64 => chunks_8_u64(element.data, endian, expected)?
            .into_iter()
            .map(|value| T::from_u128(u128::from(value)))
            .collect(),
        other => {
            return Err(MatError::new(format!(
                "unsupported numeric storage type {other} for integer array"
            )));
        }
    };
    converted
        .into_iter()
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| {
            MatError::new("MAT integer payload contains a value outside its declared class")
        })
}

#[allow(clippy::cast_precision_loss)]
fn numeric_as_f64(
    element: Element<'_>,
    endian: Endian,
    expected: usize,
) -> Result<Vec<f64>, MatError> {
    match element.kind {
        MI_INT8 => Ok(exact_bytes(element.data, expected)?
            .into_iter()
            .map(|value| f64::from(i8::from_ne_bytes([value])))
            .collect()),
        MI_UINT8 => Ok(exact_bytes(element.data, expected)?
            .into_iter()
            .map(f64::from)
            .collect()),
        MI_INT16 => Ok(chunks_2_i16(element.data, endian, expected)?
            .into_iter()
            .map(f64::from)
            .collect()),
        MI_UINT16 => Ok(chunks_2(element.data, endian, expected)?
            .into_iter()
            .map(f64::from)
            .collect()),
        MI_INT32 => Ok(chunks_4_i32(element.data, endian, expected)?
            .into_iter()
            .map(f64::from)
            .collect()),
        MI_UINT32 => Ok(chunks_4_u32(element.data, endian, expected)?
            .into_iter()
            .map(f64::from)
            .collect()),
        MI_SINGLE => Ok(chunks_4_u32(element.data, endian, expected)?
            .into_iter()
            .map(f32::from_bits)
            .map(f64::from)
            .collect()),
        MI_DOUBLE => Ok(chunks_8_u64(element.data, endian, expected)?
            .into_iter()
            .map(f64::from_bits)
            .collect()),
        MI_INT64 => Ok(chunks_8_i64(element.data, endian, expected)?
            .into_iter()
            .map(|value| value as f64)
            .collect()),
        MI_UINT64 => Ok(chunks_8_u64(element.data, endian, expected)?
            .into_iter()
            .map(|value| value as f64)
            .collect()),
        other => Err(MatError::new(format!(
            "unsupported numeric storage type {other} for double array"
        ))),
    }
}

#[allow(clippy::cast_possible_truncation)]
fn numeric_as_f32(
    element: Element<'_>,
    endian: Endian,
    expected: usize,
) -> Result<Vec<f32>, MatError> {
    if element.kind == MI_SINGLE {
        return Ok(chunks_4_u32(element.data, endian, expected)?
            .into_iter()
            .map(f32::from_bits)
            .collect());
    }
    numeric_as_f64(element, endian, expected)
        .map(|values| values.into_iter().map(|value| value as f32).collect())
}

fn numeric_as_u64(
    element: Element<'_>,
    endian: Endian,
    expected: usize,
) -> Result<Vec<u64>, MatError> {
    numeric_as_integer::<u64>(element, endian, expected)
}

fn exact_bytes(data: &[u8], expected: usize) -> Result<Vec<u8>, MatError> {
    if data.len() != expected {
        return Err(length_error(expected, data.len(), "numeric"));
    }
    Ok(data.to_vec())
}

fn chunks_2(data: &[u8], endian: Endian, expected: usize) -> Result<Vec<u16>, MatError> {
    chunks(data, expected, 2, |bytes| {
        endian.u16(bytes.try_into().expect("two bytes"))
    })
}

fn chunks_2_i16(data: &[u8], endian: Endian, expected: usize) -> Result<Vec<i16>, MatError> {
    chunks(data, expected, 2, |bytes| {
        endian.i16(bytes.try_into().expect("two bytes"))
    })
}

fn chunks_4_u32(data: &[u8], endian: Endian, expected: usize) -> Result<Vec<u32>, MatError> {
    chunks(data, expected, 4, |bytes| {
        endian.u32(bytes.try_into().expect("four bytes"))
    })
}

fn chunks_4_i32(data: &[u8], endian: Endian, expected: usize) -> Result<Vec<i32>, MatError> {
    chunks(data, expected, 4, |bytes| {
        endian.i32(bytes.try_into().expect("four bytes"))
    })
}

fn chunks_8_u64(data: &[u8], endian: Endian, expected: usize) -> Result<Vec<u64>, MatError> {
    chunks(data, expected, 8, |bytes| {
        endian.u64(bytes.try_into().expect("eight bytes"))
    })
}

fn chunks_8_i64(data: &[u8], endian: Endian, expected: usize) -> Result<Vec<i64>, MatError> {
    chunks(data, expected, 8, |bytes| {
        endian.i64(bytes.try_into().expect("eight bytes"))
    })
}

fn chunks<T>(
    data: &[u8],
    expected: usize,
    width: usize,
    convert: impl Fn(&[u8]) -> T,
) -> Result<Vec<T>, MatError> {
    let actual = data.len() / width;
    if !data.len().is_multiple_of(width) || actual != expected {
        return Err(length_error(expected, actual, "numeric"));
    }
    Ok(data.chunks_exact(width).map(convert).collect())
}

fn length_error(expected: usize, actual: usize, kind: &str) -> MatError {
    MatError::new(format!(
        "MAT {kind} payload has {actual} elements but its shape requires {expected}"
    ))
}

fn expect_kind(element: Element<'_>, expected: u32, purpose: &str) -> Result<(), MatError> {
    if element.kind == expected {
        Ok(())
    } else {
        Err(MatError::new(format!(
            "MAT {purpose} uses data type {}, expected {expected}",
            element.kind
        )))
    }
}

#[allow(clippy::needless_pass_by_value)]
fn array_error(error: openmat_array::ArrayError) -> MatError {
    MatError::new(format!("invalid MAT array storage: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    fn shape(dimensions: [u64; 2]) -> Shape {
        Shape::new(dimensions).unwrap()
    }

    fn sorted(mut variables: Vec<MatVariable>) -> Vec<MatVariable> {
        variables.sort_by(|left, right| left.name.cmp(&right.name));
        variables
    }

    fn fixtures() -> Vec<MatVariable> {
        let doubles =
            DenseArray::from_vec(shape([2, 3]), vec![1.0, 2.0, 3.0, f64::INFINITY, -0.0, 6.0])
                .unwrap();
        let singles = DenseArray::from_vec(
            shape([1, 2]),
            vec![Complex32::new(1.0, 2.0), Complex32::new(-3.0, -4.0)],
        )
        .unwrap();
        let cell = CellArray::from_values(
            shape([1, 2]),
            vec![
                Value::Array(ArrayData::F64(doubles.clone())),
                Value::Array(ArrayData::Char(
                    DenseArray::from_vec(
                        shape([1, 2]),
                        vec![
                            CharCodeUnit::new(u16::from(b'o')),
                            CharCodeUnit::new(u16::from(b'k')),
                        ],
                    )
                    .unwrap(),
                )),
            ],
        )
        .unwrap();
        let structure = StructArray::from_columns(
            shape([1, 1]),
            vec![
                FieldName::new("value").unwrap(),
                FieldName::new("cell").unwrap(),
            ],
            vec![vec![Value::Double(7.0)], vec![Value::Cell(cell)]],
        )
        .unwrap();
        vec![
            MatVariable::new("a", Value::Array(ArrayData::F64(doubles))),
            MatVariable::new("b", Value::Array(ArrayData::ComplexF32(singles))),
            MatVariable::new("s", Value::Struct(structure)),
        ]
    }

    fn integer_fixtures() -> Vec<MatVariable> {
        let dimensions = shape([1, 2]);
        macro_rules! real {
            ($name:literal, $variant:ident, $first:expr, $second:expr) => {
                MatVariable::new(
                    $name,
                    Value::Array(ArrayData::Integer(IntegerArrayData::$variant(
                        DenseArray::from_vec(dimensions.clone(), vec![$first, $second]).unwrap(),
                    ))),
                )
            };
        }
        macro_rules! complex {
            ($name:literal, $variant:ident, $type:ty, $first:expr, $second:expr) => {
                MatVariable::new(
                    $name,
                    Value::Array(ArrayData::Integer(IntegerArrayData::$variant(
                        DenseArray::from_vec(
                            dimensions.clone(),
                            vec![
                                ComplexInteger::<$type>::new($first, $second),
                                ComplexInteger::<$type>::new($second, $first),
                            ],
                        )
                        .unwrap(),
                    ))),
                )
            };
        }
        vec![
            real!("i8", I8, -1_i8, 2_i8),
            real!("u8", U8, 1_u8, 2_u8),
            real!("i16", I16, -3_i16, 4_i16),
            real!("u16", U16, 3_u16, 4_u16),
            real!("i32", I32, -5_i32, 6_i32),
            real!("u32", U32, 5_u32, 6_u32),
            real!("i64", I64, -7_i64, 8_i64),
            real!("u64", U64, 7_u64, 8_u64),
            complex!("ci8", ComplexI8, i8, -1_i8, 2_i8),
            complex!("cu8", ComplexU8, u8, 1_u8, 2_u8),
            complex!("ci16", ComplexI16, i16, -3_i16, 4_i16),
            complex!("cu16", ComplexU16, u16, 3_u16, 4_u16),
            complex!("ci32", ComplexI32, i32, -5_i32, 6_i32),
            complex!("cu32", ComplexU32, u32, 5_u32, 6_u32),
            complex!("ci64", ComplexI64, i64, -7_i64, 8_i64),
            complex!("cu64", ComplexU64, u64, 7_u64, 8_u64),
        ]
    }

    #[test]
    fn v6_v7_and_v73_round_trip_nested_supported_values() {
        for version in [MatVersion::V6, MatVersion::V7, MatVersion::V73] {
            let variables = fixtures();
            let encoded = encode(&variables, version).unwrap();
            let decoded = decode(&encoded, MatLimits::default()).unwrap();
            assert_eq!(decoded, variables);
        }
    }

    #[test]
    fn v73_round_trip_all_integer_storage_classes() {
        let variables = integer_fixtures();
        let encoded = encode(&variables, MatVersion::V73).unwrap();
        assert!(is_v73(&encoded));
        assert_eq!(
            decode(&encoded, MatLimits::default()).unwrap(),
            sorted(variables)
        );
    }

    #[test]
    fn v73_round_trip_nd_unicode_complex_and_shaped_empties() {
        let nd_shape = Shape::new([2, 3, 4]).unwrap();
        let empty_shape = Shape::new([2, 0, 4]).unwrap();
        let struct_fields = vec![
            FieldName::new("left").unwrap(),
            FieldName::new("right").unwrap(),
        ];
        let variables = vec![
            MatVariable::new(
                "nd",
                Value::Array(ArrayData::ComplexF64(
                    DenseArray::from_vec(
                        nd_shape,
                        (0..24)
                            .map(|value| Complex64::new(f64::from(value), -f64::from(value)))
                            .collect(),
                    )
                    .unwrap(),
                )),
            ),
            MatVariable::new(
                "text",
                Value::Array(ArrayData::Char(
                    DenseArray::from_vec(
                        shape([1, 4]),
                        vec![
                            CharCodeUnit::new(0x6c49),
                            CharCodeUnit::new(0xd83d),
                            CharCodeUnit::new(0xde00),
                            CharCodeUnit::new(0),
                        ],
                    )
                    .unwrap(),
                )),
            ),
            MatVariable::new(
                "empty_single",
                Value::Array(ArrayData::F32(
                    DenseArray::from_vec(empty_shape.clone(), Vec::new()).unwrap(),
                )),
            ),
            MatVariable::new(
                "empty_logical",
                Value::Array(ArrayData::Logical(
                    DenseArray::from_vec(empty_shape.clone(), Vec::new()).unwrap(),
                )),
            ),
            MatVariable::new(
                "empty_char",
                Value::Array(ArrayData::Char(
                    DenseArray::from_vec(empty_shape.clone(), Vec::new()).unwrap(),
                )),
            ),
            MatVariable::new(
                "empty_cell",
                Value::Cell(CellArray::from_values(empty_shape.clone(), Vec::new()).unwrap()),
            ),
            MatVariable::new(
                "empty_struct",
                Value::Struct(StructArray::empty(empty_shape, struct_fields).unwrap()),
            ),
        ];
        let encoded = encode(&variables, MatVersion::V73).unwrap();
        assert_eq!(
            decode(&encoded, MatLimits::default()).unwrap(),
            sorted(variables)
        );
    }

    #[test]
    fn v73_cancellation_and_resource_limits_are_structured() {
        let cancelled = AtomicBool::new(true);
        let error = encode_cancellable(
            &fixtures(),
            MatVersion::V73,
            MatLimits::default(),
            Some(&cancelled),
        )
        .unwrap_err();
        assert_eq!(error.kind(), MatErrorKind::Cancelled);

        let encoded = encode(&fixtures(), MatVersion::V73).unwrap();
        let error =
            decode_cancellable(&encoded, MatLimits::default(), Some(&cancelled)).unwrap_err();
        assert_eq!(error.kind(), MatErrorKind::Cancelled);

        let assert_decode_limit = |limits| {
            let error = decode(&encoded, limits).unwrap_err();
            assert_eq!(error.kind(), MatErrorKind::LimitExceeded);
        };
        assert_decode_limit(MatLimits {
            maximum_file_bytes: encoded.len() - 1,
            ..MatLimits::default()
        });
        assert_decode_limit(MatLimits {
            maximum_allocation_bytes: encoded.len(),
            ..MatLimits::default()
        });
        assert_decode_limit(MatLimits {
            maximum_hdf5_decompressed_bytes: 8,
            ..MatLimits::default()
        });
        assert_decode_limit(MatLimits {
            maximum_hdf5_objects: 1,
            ..MatLimits::default()
        });
        assert_decode_limit(MatLimits {
            maximum_hdf5_references: 0,
            ..MatLimits::default()
        });
        assert_decode_limit(MatLimits {
            maximum_array_elements: 1,
            ..MatLimits::default()
        });
        assert_decode_limit(MatLimits {
            maximum_recursion_depth: 0,
            ..MatLimits::default()
        });

        let error = encode_cancellable(
            &fixtures(),
            MatVersion::V73,
            MatLimits {
                maximum_allocation_bytes: 512,
                ..MatLimits::default()
            },
            None,
        )
        .unwrap_err();
        assert_eq!(error.kind(), MatErrorKind::LimitExceeded);

        for limits in [
            MatLimits {
                maximum_hdf5_objects: 1,
                ..MatLimits::default()
            },
            MatLimits {
                maximum_hdf5_references: 0,
                ..MatLimits::default()
            },
            MatLimits {
                maximum_recursion_depth: 0,
                ..MatLimits::default()
            },
        ] {
            let error = encode_cancellable(&fixtures(), MatVersion::V73, limits, None).unwrap_err();
            assert_eq!(error.kind(), MatErrorKind::LimitExceeded);
        }
    }

    #[test]
    fn rejects_ordinary_hdf5_and_bounds_level5_decompression() {
        assert!(
            decode(b"\x89HDF\r\n\x1a\n", MatLimits::default())
                .unwrap_err()
                .message()
                .contains("v7.3")
        );
        let encoded = encode(&fixtures(), MatVersion::V7).unwrap();
        assert!(
            decode(
                &encoded,
                MatLimits {
                    maximum_decompressed_bytes: 8,
                    ..MatLimits::default()
                }
            )
            .unwrap_err()
            .message()
            .contains("limit")
        );
    }

    #[test]
    fn rejects_runtime_only_values() {
        let error = encode(
            &[MatVariable::new(
                "f",
                Value::Function(openmat_value::FunctionHandle::Builtin(
                    openmat_value::BuiltinHandle::new(1),
                )),
            )],
            MatVersion::V7,
        )
        .unwrap_err();
        assert!(error.message().contains("variable 'f'"));
        assert!(error.message().contains("Level-5 class mapping"));
    }

    #[test]
    #[ignore = "requires OPENMAT_MATLAB_FIXTURE_DIR from an installed MATLAB probe"]
    fn matlab_r2022b_interoperability_fixture() {
        let directory = std::env::var_os("OPENMAT_MATLAB_FIXTURE_DIR")
            .map(std::path::PathBuf::from)
            .expect("OPENMAT_MATLAB_FIXTURE_DIR must name the probe directory");
        for filename in ["v6.mat", "v7.mat"] {
            let bytes = std::fs::read(directory.join(filename)).unwrap();
            let variables = decode(&bytes, MatLimits::default()).unwrap();
            let names = variables
                .iter()
                .map(|variable| variable.name.as_str())
                .collect::<Vec<_>>();
            assert_eq!(names, ["a", "b", "c", "ce", "st", "t"]);
            assert_eq!(variables[0].value.class_name(), "double");
            assert_eq!(variables[0].value.dimensions(), Some([2, 3].as_slice()));
            assert_eq!(variables[1].value.class_name(), "single");
            assert_eq!(variables[2].value.class_name(), "logical");
            assert_eq!(variables[3].value.class_name(), "cell");
            assert_eq!(variables[4].value.class_name(), "struct");
            assert_eq!(variables[5].value.class_name(), "char");
            std::fs::write(
                directory.join(format!("openmat-{filename}")),
                encode(
                    &variables,
                    if filename == "v6.mat" {
                        MatVersion::V6
                    } else {
                        MatVersion::V7
                    },
                )
                .unwrap(),
            )
            .unwrap();
        }
        let unicode = Value::Array(ArrayData::Char(
            DenseArray::from_vec(
                shape([1, 3]),
                vec![
                    CharCodeUnit::new(0x6c49),
                    CharCodeUnit::new(0xd83d),
                    CharCodeUnit::new(0xde00),
                ],
            )
            .unwrap(),
        ));
        std::fs::write(
            directory.join("openmat-unicode.mat"),
            encode(&[MatVariable::new("unicode", unicode)], MatVersion::V7).unwrap(),
        )
        .unwrap();
    }

    #[test]
    #[ignore = "requires OPENMAT_MATLAB_FIXTURE_DIR from an installed MATLAB probe"]
    fn matlab_r2022b_v73_interoperability_fixture() {
        let directory = std::env::var_os("OPENMAT_MATLAB_FIXTURE_DIR")
            .map(std::path::PathBuf::from)
            .expect("OPENMAT_MATLAB_FIXTURE_DIR must name the probe directory");
        let bytes = std::fs::read(directory.join("matlab-v73.mat")).unwrap();
        let variables = decode(&bytes, MatLimits::default()).unwrap();
        let names = variables
            .iter()
            .map(|variable| variable.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "ce", "ch", "d", "ec", "emp", "es", "i8", "lg", "ndempty", "s", "st", "u64"
            ]
        );
        assert_eq!(variables[0].value.class_name(), "cell");
        assert_eq!(variables[1].value.class_name(), "char");
        assert_eq!(variables[2].value.dimensions(), Some([2, 3, 4].as_slice()));
        assert_eq!(variables[3].value.dimensions(), Some([2, 0, 4].as_slice()));
        assert_eq!(variables[4].value.dimensions(), Some([0, 3].as_slice()));
        assert_eq!(variables[5].value.class_name(), "struct");
        assert_eq!(variables[5].value.dimensions(), Some([2, 0, 4].as_slice()));
        assert_eq!(variables[8].value.dimensions(), Some([2, 0, 4].as_slice()));
        assert_eq!(variables[9].value.class_name(), "single");
        assert_eq!(variables[10].value.class_name(), "struct");
        std::fs::write(
            directory.join("openmat-v73.mat"),
            encode(&variables, MatVersion::V73).unwrap(),
        )
        .unwrap();
    }
}
