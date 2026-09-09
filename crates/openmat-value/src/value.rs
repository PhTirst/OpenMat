use std::{error::Error, fmt};

use openmat_array::{ArrayData, ArrayError, Complex32, DType, DenseArray, Shape};
use openmat_sparse::SparseArrayData;

use crate::{
    BytecodeFunctionHandle, CellArray, Complex64, FunctionHandle, GraphicsHandle,
    GraphicsHandleArray, ObjectArray, ObjectHandle, StringArray, StringElement, StringValue,
    StructArray, TableArray,
};

const SCALAR_DIMENSIONS: [u64; 2] = [1, 1];
const CONDITION_CHECK_INTERVAL: usize = 1_024;

/// A checked cell or struct construction or mutation failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AggregateError {
    /// A shape, host-length, or cell-buffer check from the array core failed.
    Array(ArrayError),
    /// A struct field name is outside the first-tranche ASCII identifier set.
    InvalidFieldName {
        /// Rejected name.
        name: String,
    },
    /// A struct schema contains the same field more than once.
    DuplicateFieldName {
        /// Repeated name.
        name: String,
    },
    /// A struct constructor received a different number of fields and columns.
    FieldColumnCountMismatch {
        /// Number of schema fields.
        fields: usize,
        /// Number of supplied columns.
        columns: usize,
    },
    /// A struct field column length differs from the record count.
    FieldColumnLengthMismatch {
        /// Zero-based field position.
        field: usize,
        /// Required record count.
        expected: u64,
        /// Supplied column length.
        actual: usize,
    },
    /// A zero-based field position is outside the schema.
    FieldOutOfBounds {
        /// Supplied field position.
        field: usize,
        /// Number of fields in the schema.
        fields: usize,
    },
    /// A zero-based element or record offset is outside the aggregate.
    OffsetOutOfBounds {
        /// Supplied storage offset.
        offset: usize,
        /// Number of aggregate elements or records.
        numel: u64,
    },
}

impl fmt::Display for AggregateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Array(error) => error.fmt(formatter),
            Self::InvalidFieldName { name } => {
                write!(formatter, "invalid struct field name {name:?}")
            }
            Self::DuplicateFieldName { name } => {
                write!(formatter, "duplicate struct field name {name:?}")
            }
            Self::FieldColumnCountMismatch { fields, columns } => write!(
                formatter,
                "struct schema has {fields} fields but {columns} columns were supplied"
            ),
            Self::FieldColumnLengthMismatch {
                field,
                expected,
                actual,
            } => write!(
                formatter,
                "struct field {field} requires {expected} values but {actual} were supplied"
            ),
            Self::FieldOutOfBounds { field, fields } => {
                write!(
                    formatter,
                    "struct field {field} is outside a schema with {fields} fields"
                )
            }
            Self::OffsetOutOfBounds { offset, numel } => write!(
                formatter,
                "aggregate offset {offset} is outside storage with {numel} elements"
            ),
        }
    }
}

impl Error for AggregateError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Array(error) => Some(error),
            _ => None,
        }
    }
}

impl From<ArrayError> for AggregateError {
    fn from(value: ArrayError) -> Self {
        Self::Array(value)
    }
}

/// A coarse runtime value category used by structured errors and dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ValueKind {
    /// Internal no-value marker.
    Nothing,
    /// Scalar logical.
    Logical,
    /// Scalar real double.
    Double,
    /// Scalar complex double.
    Complex,
    /// Real or complex binary32 array, including language scalars.
    Single,
    /// Scalar or array-valued string.
    String,
    /// Exact UTF-16 `char` array.
    Char,
    /// Exact real or complex fixed-width integer array.
    Integer,
    /// Owned copy-on-write dense array.
    Array,
    /// Owned copy-on-write normalized sparse array.
    Sparse,
    /// Heterogeneous copy-on-write cell array.
    Cell,
    /// Ordered-schema, field-major copy-on-write struct array.
    Struct,
    /// Ordered heterogeneous MATLAB table.
    Table,
    /// Opaque object adapter handle.
    Object,
    /// Scalar or homogeneous row graphics handle.
    Graphics,
    /// Callable handle.
    Function,
}

impl ValueKind {
    /// Returns the basic MATLAB-compatible class name where one is known.
    #[must_use]
    pub const fn class_name(self) -> &'static str {
        match self {
            Self::Nothing => "nothing",
            Self::Logical => "logical",
            Self::Double | Self::Complex => "double",
            Self::Single => "single",
            Self::String => "string",
            Self::Char => "char",
            Self::Integer => "integer",
            Self::Array => "array",
            Self::Sparse => "sparse",
            Self::Cell => "cell",
            Self::Struct => "struct",
            Self::Table => "table",
            Self::Object => "object",
            Self::Graphics => "graphics",
            Self::Function => "function_handle",
        }
    }
}

impl fmt::Display for ValueKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.class_name())
    }
}

/// The first-release dynamic value representation.
///
/// Logical, real, and complex scalars use optimized variants. An explicit
/// numeric `1x1` array has the same language-level class, shape, and element
/// count but may retain array storage until a scalar operation consumes it.
/// Single, double, logical, `char`, and integer arrays all own an [`ArrayData`]
/// value, which preserves their exact element type. The string variant
/// carries either an optimized scalar or [`StringArray`]; all dense
/// representations clone by copy-on-write. Cells and structs likewise
/// share outer storage and preserve nested value COW. Scalar objects keep
/// opaque handles; homogeneous object arrays additionally retain their exact
/// dynamic class and copy-on-write column-major handle storage. The runtime
/// applies value-versus-handle semantics to those handles. Derived Rust
/// [`PartialEq`] is representation comparison for tests and change detection,
/// not MATLAB equality dispatch.
/// This enum's Rust layout is explicitly not an ABI; process and plugin
/// boundaries must use a separately versioned protocol.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Value {
    /// Internal no-value marker.
    #[default]
    Nothing,
    /// Scalar logical.
    Logical(bool),
    /// Scalar real double.
    Double(f64),
    /// Scalar complex double.
    Complex(Complex64),
    /// Scalar-optimized or owned copy-on-write string array.
    String(StringValue),
    /// Owned dense array with copy-on-write element storage.
    Array(ArrayData),
    /// Owned normalized CSC array with copy-on-write storage.
    Sparse(SparseArrayData),
    /// Heterogeneous cell array with copy-on-write value storage.
    Cell(CellArray),
    /// Ordered-schema struct array with per-field copy-on-write storage.
    Struct(StructArray),
    /// Ordered heterogeneous table with copy-on-write variables.
    Table(TableArray),
    /// Opaque object adapter handle.
    Object(ObjectHandle),
    /// Homogeneous array of runtime object references.
    ObjectArray(ObjectArray),
    /// Scalar session-owned graphics handle.
    Graphics(GraphicsHandle),
    /// Homogeneous `N x 1` graphics handle column.
    GraphicsArray(GraphicsHandleArray),
    /// Bytecode or built-in callable.
    Function(FunctionHandle),
}

impl Value {
    /// Visits every object and registered bytecode-function handle nested in
    /// this value.
    ///
    /// Cells, structs, and tables are traversed recursively; object arrays
    /// visit each element handle. Numeric, text, graphics, and intrinsic or
    /// built-in callable values contain no object-store edges. The traversal
    /// is iterative so deeply nested language aggregates do not consume the
    /// Rust call stack.
    pub fn trace_handles(
        &self,
        visit_object: &mut impl FnMut(ObjectHandle),
        visit_function: &mut impl FnMut(BytecodeFunctionHandle),
    ) {
        let mut pending = vec![self];
        while let Some(value) = pending.pop() {
            match value {
                Self::Cell(cell) => pending.extend(cell.values()),
                Self::Struct(structure) => {
                    for field in 0..structure.field_count() {
                        if let Some(values) = structure.field_values(field) {
                            pending.extend(values);
                        }
                    }
                }
                Self::Table(table) => {
                    for variable in 0..table.variable_count() {
                        if let Some(value) = table.variable(variable) {
                            pending.push(value);
                        }
                    }
                }
                Self::Object(handle) => visit_object(*handle),
                Self::ObjectArray(array) => {
                    array
                        .as_slice()
                        .iter()
                        .copied()
                        .for_each(&mut *visit_object);
                }
                Self::Function(FunctionHandle::Bytecode(handle)) => visit_function(*handle),
                Self::Nothing
                | Self::Logical(_)
                | Self::Double(_)
                | Self::Complex(_)
                | Self::String(_)
                | Self::Array(_)
                | Self::Sparse(_)
                | Self::Graphics(_)
                | Self::GraphicsArray(_)
                | Self::Function(_) => {}
            }
        }
    }

    /// Visits every object-store handle nested in this value.
    pub fn trace_object_handles(&self, visit: &mut impl FnMut(ObjectHandle)) {
        self.trace_handles(visit, &mut |_| {});
    }

    /// Returns the value's coarse runtime category.
    #[must_use]
    pub const fn kind(&self) -> ValueKind {
        match self {
            Self::Nothing => ValueKind::Nothing,
            Self::Logical(_) => ValueKind::Logical,
            Self::Double(_) => ValueKind::Double,
            Self::Complex(_) => ValueKind::Complex,
            Self::Array(ArrayData::F32(_) | ArrayData::ComplexF32(_)) => ValueKind::Single,
            Self::String(_) => ValueKind::String,
            Self::Array(ArrayData::Char(_)) => ValueKind::Char,
            Self::Array(ArrayData::Integer(_)) => ValueKind::Integer,
            Self::Array(_) => ValueKind::Array,
            Self::Sparse(_) => ValueKind::Sparse,
            Self::Cell(_) => ValueKind::Cell,
            Self::Struct(_) => ValueKind::Struct,
            Self::Table(_) => ValueKind::Table,
            Self::Object(_) | Self::ObjectArray(_) => ValueKind::Object,
            Self::Graphics(_) | Self::GraphicsArray(_) => ValueKind::Graphics,
            Self::Function(_) => ValueKind::Function,
        }
    }

    /// Returns the basic MATLAB-compatible class name where one is known.
    #[must_use]
    pub fn class_name(&self) -> &'static str {
        match self {
            Self::Graphics(handle) => handle.class().class_name(),
            Self::GraphicsArray(array) => array.class().class_name(),
            Self::Array(array) => array.class_name(),
            Self::Sparse(array) => array.class_name(),
            _ => self.kind().class_name(),
        }
    }

    /// Returns the exact dense storage type for scalar numeric values and
    /// dynamically stored arrays.
    #[must_use]
    pub const fn dtype(&self) -> Option<DType> {
        match self {
            Self::Logical(_) => Some(DType::Logical),
            Self::Double(_) => Some(DType::F64),
            Self::Complex(_) => Some(DType::ComplexF64),
            Self::Array(array) => Some(array.dtype()),
            Self::Sparse(array) => Some(array.dtype()),
            _ => None,
        }
    }

    /// Returns the canonical language-visible dimensions where shape semantics
    /// are implemented.
    ///
    /// Scalar-optimized logical, numeric, and string values report `1x1`, just
    /// like an explicitly stored `1x1` array. Object arrays expose their stored
    /// canonical shape. Internal markers and callable handles have no shape.
    #[must_use]
    pub fn dimensions(&self) -> Option<&[u64]> {
        match self {
            Self::Logical(_)
            | Self::Double(_)
            | Self::Complex(_)
            | Self::Object(_)
            | Self::Graphics(_) => Some(&SCALAR_DIMENSIONS),
            Self::ObjectArray(array) => Some(array.shape().dimensions()),
            Self::GraphicsArray(array) => Some(array.shape().dimensions()),
            Self::String(value) => Some(value.dimensions()),
            Self::Array(array) => Some(array.shape().dimensions()),
            Self::Sparse(array) => Some(array.shape().dimensions()),
            Self::Cell(array) => Some(array.shape().dimensions()),
            Self::Struct(array) => Some(array.shape().dimensions()),
            Self::Table(array) => Some(array.shape().dimensions()),
            Self::Nothing | Self::Function(_) => None,
        }
    }

    /// Returns the language-visible element count where shape semantics are
    /// implemented.
    #[must_use]
    pub fn numel(&self) -> Option<u64> {
        match self {
            Self::Logical(_)
            | Self::Double(_)
            | Self::Complex(_)
            | Self::Object(_)
            | Self::Graphics(_) => Some(1),
            Self::ObjectArray(array) => Some(array.numel()),
            Self::GraphicsArray(array) => Some(array.numel()),
            Self::String(value) => Some(value.numel()),
            Self::Array(array) => Some(array.numel()),
            Self::Sparse(array) => Some(array.numel()),
            Self::Cell(array) => Some(array.numel()),
            Self::Struct(array) => Some(array.numel()),
            Self::Table(array) => Some(array.numel()),
            Self::Nothing | Self::Function(_) => None,
        }
    }

    /// Returns whether this value has implemented scalar shape semantics.
    #[must_use]
    pub fn is_scalar(&self) -> bool {
        self.numel() == Some(1)
    }

    /// Returns whether a numeric value uses complex storage.
    #[must_use]
    pub const fn is_complex_numeric(&self) -> bool {
        match self {
            Self::Complex(_) => true,
            Self::Array(array) => array.is_complex(),
            Self::Sparse(array) => array.is_complex(),
            _ => false,
        }
    }

    /// Borrows owned dense array storage from the dynamic value.
    #[must_use]
    pub const fn as_array(&self) -> Option<&ArrayData> {
        match self {
            Self::Array(array) => Some(array),
            _ => None,
        }
    }

    /// Mutably borrows owned array storage.
    ///
    /// The first element mutation through the contained dense array detaches a
    /// shared buffer using the array core's copy-on-write implementation.
    pub const fn as_array_mut(&mut self) -> Option<&mut ArrayData> {
        match self {
            Self::Array(array) => Some(array),
            _ => None,
        }
    }

    /// Borrows normalized CSC storage from the dynamic value.
    #[must_use]
    pub const fn as_sparse(&self) -> Option<&SparseArrayData> {
        match self {
            Self::Sparse(array) => Some(array),
            _ => None,
        }
    }

    /// Mutably borrows the sparse value wrapper.
    ///
    /// Individual CSC buffers remain copy-on-write and expose no unchecked
    /// mutation path.
    pub const fn as_sparse_mut(&mut self) -> Option<&mut SparseArrayData> {
        match self {
            Self::Sparse(array) => Some(array),
            _ => None,
        }
    }

    /// Borrows exact real or complex binary32 dense storage.
    #[must_use]
    pub const fn as_single(&self) -> Option<&ArrayData> {
        match self {
            Self::Array(array @ (ArrayData::F32(_) | ArrayData::ComplexF32(_))) => Some(array),
            _ => None,
        }
    }

    /// Mutably borrows exact real or complex binary32 dense storage.
    ///
    /// A later element mutation through the contained dense array performs
    /// copy-on-write detachment.
    pub const fn as_single_mut(&mut self) -> Option<&mut ArrayData> {
        match self {
            Self::Array(array @ (ArrayData::F32(_) | ArrayData::ComplexF32(_))) => Some(array),
            _ => None,
        }
    }

    /// Borrows a cell array.
    #[must_use]
    pub const fn as_cell(&self) -> Option<&CellArray> {
        match self {
            Self::Cell(array) => Some(array),
            _ => None,
        }
    }

    /// Mutably borrows a cell array without detaching its value storage.
    ///
    /// A later successful mutation through [`CellArray`] performs COW detach.
    pub const fn as_cell_mut(&mut self) -> Option<&mut CellArray> {
        match self {
            Self::Cell(array) => Some(array),
            _ => None,
        }
    }

    /// Borrows a struct array.
    #[must_use]
    pub const fn as_struct(&self) -> Option<&StructArray> {
        match self {
            Self::Struct(array) => Some(array),
            _ => None,
        }
    }

    /// Mutably borrows a struct array without detaching its field storage.
    ///
    /// A later successful mutation through [`StructArray`] performs COW
    /// detach.
    pub const fn as_struct_mut(&mut self) -> Option<&mut StructArray> {
        match self {
            Self::Struct(array) => Some(array),
            _ => None,
        }
    }

    /// Borrows a table value.
    #[must_use]
    pub const fn as_table(&self) -> Option<&TableArray> {
        match self {
            Self::Table(table) => Some(table),
            _ => None,
        }
    }

    /// Mutably borrows a table without eagerly detaching its variables.
    pub const fn as_table_mut(&mut self) -> Option<&mut TableArray> {
        match self {
            Self::Table(table) => Some(table),
            _ => None,
        }
    }

    /// Returns whether two values share primary copy-on-write storage.
    ///
    /// Dense values use their element buffer, cells use their outer value
    /// buffer, and structs use their outer field-column table. Struct schema
    /// and individual field sharing remain available through [`StructArray`].
    /// Values with different variants never share storage through this API.
    #[must_use]
    pub fn shares_storage_with(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Array(left), Self::Array(right)) => left.shares_storage_with(right),
            (Self::Sparse(left), Self::Sparse(right)) => left.shares_storage_with(right),
            (Self::String(left), Self::String(right)) => {
                match (left.as_array(), right.as_array()) {
                    (Some(left), Some(right)) => left.shares_storage_with(right),
                    _ => false,
                }
            }
            (Self::ObjectArray(left), Self::ObjectArray(right)) => {
                left.class_handle() == right.class_handle() && left.shares_storage_with(right)
            }
            (Self::GraphicsArray(left), Self::GraphicsArray(right)) => {
                left.class() == right.class() && left.shares_storage_with(right)
            }
            (Self::Cell(left), Self::Cell(right)) => left.shares_storage_with(right),
            (Self::Struct(left), Self::Struct(right)) => left.shares_storage_with(right),
            (Self::Table(left), Self::Table(right)) => left.shares_storage_with(right),
            _ => false,
        }
    }

    /// Returns whether two values share their primary array-like storage.
    ///
    /// This compatibility name includes cell buffers and struct field tables in
    /// addition to numeric, logical, char, integer, string, and object arrays.
    #[must_use]
    pub fn shares_array_storage_with(&self, other: &Self) -> bool {
        self.shares_storage_with(other)
    }

    /// Constructs the language value `double.empty(0, 0)`.
    ///
    /// # Panics
    ///
    /// Panics only if the array core rejects the statically valid `0x0` shape
    /// or its matching empty buffer, which would violate an internal invariant.
    #[must_use]
    pub fn empty_double() -> Self {
        let shape = Shape::new([0, 0]).expect("the fixed 0x0 shape is valid");
        let array = DenseArray::from_vec(shape, Vec::new())
            .expect("an empty buffer matches the fixed 0x0 shape");
        Self::Array(ArrayData::F64(array))
    }

    /// Borrows a scalar string, including an explicit `1x1` string array.
    #[must_use]
    pub fn as_string_scalar(&self) -> Option<&StringElement> {
        match self {
            Self::String(value) => value.as_scalar(),
            _ => None,
        }
    }

    /// Borrows explicit string-array storage.
    #[must_use]
    pub const fn as_string_array(&self) -> Option<&StringArray> {
        match self {
            Self::String(StringValue::Array(array)) => Some(array),
            _ => None,
        }
    }

    /// Mutably borrows explicit string-array storage.
    pub const fn as_string_array_mut(&mut self) -> Option<&mut StringArray> {
        match self {
            Self::String(StringValue::Array(array)) => Some(array),
            _ => None,
        }
    }

    /// Returns a scalar numeric value as a complex number.
    #[must_use]
    pub fn as_complex_number(&self) -> Option<Complex64> {
        match self {
            Self::Logical(value) => Some(Complex64::new(f64::from(*value), 0.0)),
            Self::Double(value) => Some(Complex64::new(*value, 0.0)),
            Self::Complex(value) => Some(*value),
            Self::Array(ArrayData::F64(array)) if array.numel() == 1 => {
                Some(Complex64::new(array.as_slice()[0], 0.0))
            }
            Self::Array(ArrayData::ComplexF64(array)) if array.numel() == 1 => {
                Some(array.as_slice()[0].into())
            }
            Self::Array(ArrayData::Logical(array)) if array.numel() == 1 => {
                Some(Complex64::new(f64::from(array.as_slice()[0].get()), 0.0))
            }
            Self::Sparse(SparseArrayData::Logical(array)) if array.numel() == 1 => {
                Some(Complex64::new(
                    f64::from(array.get_linear_one_based(1).ok().flatten().is_some()),
                    0.0,
                ))
            }
            Self::Sparse(SparseArrayData::F64(array)) if array.numel() == 1 => {
                Some(Complex64::new(
                    array
                        .get_linear_one_based(1)
                        .ok()
                        .flatten()
                        .copied()
                        .unwrap_or(0.0),
                    0.0,
                ))
            }
            Self::Sparse(SparseArrayData::ComplexF64(array)) if array.numel() == 1 => {
                let value = array
                    .get_linear_one_based(1)
                    .ok()
                    .flatten()
                    .copied()
                    .unwrap_or(openmat_array::Complex64::ZERO);
                Some(Complex64::new(value.re, value.im))
            }
            _ => None,
        }
    }

    /// Returns a scalar real numeric value.
    #[must_use]
    pub fn as_real_number(&self) -> Option<f64> {
        match self {
            Self::Logical(value) => Some(f64::from(*value)),
            Self::Double(value) => Some(*value),
            Self::Complex(value) if value.imaginary == 0.0 => Some(value.real),
            Self::Array(ArrayData::F64(array)) if array.numel() == 1 => Some(array.as_slice()[0]),
            Self::Array(ArrayData::ComplexF64(array))
                if array.numel() == 1 && array.as_slice()[0].im == 0.0 =>
            {
                Some(array.as_slice()[0].re)
            }
            Self::Array(ArrayData::Logical(array)) if array.numel() == 1 => {
                Some(f64::from(array.as_slice()[0].get()))
            }
            Self::Sparse(_) if self.numel() == Some(1) => self
                .as_complex_number()
                .filter(|value| value.imaginary == 0.0)
                .map(|value| value.real),
            _ => None,
        }
    }

    /// Returns a scalar `single` value as exact binary32 complex components.
    ///
    /// This accessor does not expose single through the binary64 numeric view,
    /// preventing generic double-only dispatch from silently accepting it.
    #[must_use]
    pub fn as_complex_single(&self) -> Option<Complex32> {
        match self {
            Self::Array(ArrayData::F32(array)) if array.numel() == 1 => {
                Some(Complex32::new(array.as_slice()[0], 0.0))
            }
            Self::Array(ArrayData::ComplexF32(array)) if array.numel() == 1 => {
                Some(array.as_slice()[0])
            }
            _ => None,
        }
    }

    /// Returns a scalar real `single` component without widening to `f64`.
    #[must_use]
    pub fn as_real_single(&self) -> Option<f32> {
        match self {
            Self::Array(ArrayData::F32(array)) if array.numel() == 1 => Some(array.as_slice()[0]),
            Self::Array(ArrayData::ComplexF32(array))
                if array.numel() == 1 && array.as_slice()[0].im == 0.0 =>
            {
                Some(array.as_slice()[0].re)
            }
            _ => None,
        }
    }

    /// Converts a logical or numeric value to a control-flow condition.
    ///
    /// Dense arrays are true when they are nonempty and every element is
    /// nonzero. Empty arrays are false.
    ///
    /// # Errors
    ///
    /// Returns an error for no-value markers, strings, and opaque handles.
    pub fn condition(&self) -> Result<bool, ConditionError> {
        let evaluation = self.condition_with_interrupt(|| false)?;
        let ConditionEvaluation::Complete(value) = evaluation else {
            unreachable!("an interrupt-free condition evaluation cannot be interrupted");
        };
        Ok(value)
    }

    /// Evaluates a condition with periodic interruption checkpoints.
    ///
    /// `interrupted` is queried every 1024 dense-array elements. This keeps the
    /// value layer independent of a particular runtime cancellation token while
    /// allowing interpreters to stop large condition scans cooperatively.
    ///
    /// # Errors
    ///
    /// Returns an error for no-value markers, strings, and opaque handles.
    pub fn condition_with_interrupt(
        &self,
        mut interrupted: impl FnMut() -> bool,
    ) -> Result<ConditionEvaluation, ConditionError> {
        match self {
            Self::Logical(value) => Ok(ConditionEvaluation::Complete(*value)),
            Self::Double(value) => Ok(ConditionEvaluation::Complete(*value != 0.0)),
            Self::Complex(value) => Ok(ConditionEvaluation::Complete(!value.is_zero())),
            Self::Array(ArrayData::F64(array)) => Ok(condition_all(
                array.as_slice(),
                |value| *value != 0.0,
                &mut interrupted,
            )),
            Self::Array(ArrayData::ComplexF64(array)) => Ok(condition_all(
                array.as_slice(),
                |value| value.re != 0.0 || value.im != 0.0,
                &mut interrupted,
            )),
            Self::Array(ArrayData::Logical(array)) => Ok(condition_all(
                array.as_slice(),
                |value| value.get(),
                &mut interrupted,
            )),
            Self::Array(ArrayData::F32(array)) => Ok(condition_all(
                array.as_slice(),
                |value| *value != 0.0,
                &mut interrupted,
            )),
            Self::Array(ArrayData::ComplexF32(array)) => Ok(condition_all(
                array.as_slice(),
                |value| value.re != 0.0 || value.im != 0.0,
                &mut interrupted,
            )),
            Self::Sparse(array) => {
                if array.numel() == 0 || u64::try_from(array.nnz()).ok() != Some(array.numel()) {
                    return Ok(ConditionEvaluation::Complete(false));
                }
                for index in (0..array.nnz()).step_by(CONDITION_CHECK_INTERVAL) {
                    let _ = index;
                    if interrupted() {
                        return Ok(ConditionEvaluation::Interrupted);
                    }
                }
                Ok(ConditionEvaluation::Complete(true))
            }
            _ => Err(ConditionError {
                actual: self.kind(),
            }),
        }
    }
}

impl From<bool> for Value {
    fn from(value: bool) -> Self {
        Self::Logical(value)
    }
}

impl From<f64> for Value {
    fn from(value: f64) -> Self {
        Self::Double(value)
    }
}

impl From<Complex64> for Value {
    fn from(value: Complex64) -> Self {
        Self::Complex(value)
    }
}

impl From<ArrayData> for Value {
    fn from(value: ArrayData) -> Self {
        Self::Array(value)
    }
}

impl From<SparseArrayData> for Value {
    fn from(value: SparseArrayData) -> Self {
        Self::Sparse(value)
    }
}

impl From<CellArray> for Value {
    fn from(value: CellArray) -> Self {
        Self::Cell(value)
    }
}

impl From<StructArray> for Value {
    fn from(value: StructArray) -> Self {
        Self::Struct(value)
    }
}

impl From<TableArray> for Value {
    fn from(value: TableArray) -> Self {
        Self::Table(value)
    }
}

impl From<String> for Value {
    fn from(value: String) -> Self {
        Self::String(StringValue::from(value))
    }
}

impl From<&str> for Value {
    fn from(value: &str) -> Self {
        Self::String(StringValue::from(value))
    }
}

impl From<StringArray> for Value {
    fn from(value: StringArray) -> Self {
        Self::String(StringValue::Array(value))
    }
}

impl From<StringElement> for Value {
    fn from(value: StringElement) -> Self {
        Self::String(StringValue::Scalar(value))
    }
}

fn condition_all<T>(
    values: &[T],
    mut nonzero: impl FnMut(&T) -> bool,
    interrupted: &mut impl FnMut() -> bool,
) -> ConditionEvaluation {
    if values.is_empty() {
        return ConditionEvaluation::Complete(false);
    }
    for (index, value) in values.iter().enumerate() {
        if index.is_multiple_of(CONDITION_CHECK_INTERVAL) && interrupted() {
            return ConditionEvaluation::Interrupted;
        }
        if !nonzero(value) {
            return ConditionEvaluation::Complete(false);
        }
    }
    ConditionEvaluation::Complete(true)
}

/// Result of condition evaluation with cooperative interruption checkpoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConditionEvaluation {
    /// Evaluation completed with the language truth value.
    Complete(bool),
    /// The supplied interruption callback requested an early stop.
    Interrupted,
}

/// A failed conversion of a dynamic value to a scalar condition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConditionError {
    /// Actual value category.
    pub actual: ValueKind,
}

impl fmt::Display for ConditionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "a logical or numeric condition was required, found {}",
            self.actual
        )
    }
}

impl Error for ConditionError {}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use openmat_array::{Complex64 as ArrayComplex64, DenseArray, Logical, Shape};
    use openmat_sparse::CooEntry;

    use super::*;
    use crate::{
        BuiltinHandle, BytecodeFunctionHandle, CellArray, ClassHandle, FieldName, StructArray,
    };

    #[test]
    fn complex_values_have_double_class() {
        let value = Value::Complex(Complex64::new(2.0, -1.0));
        assert_eq!(value.kind(), ValueKind::Complex);
        assert_eq!(value.class_name(), "double");
    }

    #[test]
    fn scalar_conditions_are_checked() {
        assert_eq!(Value::Double(-1.0).condition(), Ok(true));
        assert_eq!(
            Value::Complex(Complex64::new(0.0, 0.0)).condition(),
            Ok(false)
        );
        assert_eq!(
            Value::from("text").condition(),
            Err(ConditionError {
                actual: ValueKind::String
            })
        );
    }

    #[test]
    fn callable_handles_keep_domains_separate() {
        let bytecode = FunctionHandle::Bytecode(BytecodeFunctionHandle::new(3));
        let builtin = FunctionHandle::Builtin(BuiltinHandle::new(3));
        assert_ne!(bytecode, builtin);
    }

    #[test]
    fn scalar_and_one_by_one_array_metadata_agree() {
        let scalar = Value::Double(3.0);
        let array = Value::Array(ArrayData::F64(
            DenseArray::from_vec(Shape::new([1, 1]).unwrap(), vec![3.0]).unwrap(),
        ));

        assert_eq!(scalar.class_name(), array.class_name());
        assert_eq!(scalar.dimensions(), array.dimensions());
        assert_eq!(scalar.numel(), array.numel());
        assert_eq!(array.as_real_number(), Some(3.0));
        assert_eq!(array.condition(), Ok(true));
    }

    #[test]
    fn arrays_report_element_class_shape_and_empty_numel() {
        let complex = Value::Array(ArrayData::ComplexF64(
            DenseArray::from_vec(
                Shape::new([1, 2]).unwrap(),
                vec![ArrayComplex64::new(1.0, -2.0), ArrayComplex64::ZERO],
            )
            .unwrap(),
        ));
        let logical = Value::Array(ArrayData::Logical(
            DenseArray::from_vec(Shape::new([0, 3]).unwrap(), Vec::<Logical>::new()).unwrap(),
        ));

        assert_eq!(complex.class_name(), "double");
        assert_eq!(complex.dimensions(), Some([1, 2].as_slice()));
        assert_eq!(complex.numel(), Some(2));
        assert_eq!(logical.class_name(), "logical");
        assert_eq!(logical.dimensions(), Some([0, 3].as_slice()));
        assert_eq!(logical.numel(), Some(0));
        assert_eq!(logical.condition(), Ok(false));
    }

    #[test]
    fn sparse_values_report_underlying_class_shape_cow_and_condition() {
        let sparse = SparseArrayData::try_from_f64_coo(
            2,
            2,
            vec![
                CooEntry::new(0, 0, 1.0),
                CooEntry::new(1, 0, f64::NAN),
                CooEntry::new(0, 1, f64::INFINITY),
            ],
            3,
            None,
        )
        .unwrap();
        let value = Value::Sparse(sparse);
        let alias = value.clone();
        assert_eq!(value.kind(), ValueKind::Sparse);
        assert_eq!(value.class_name(), "double");
        assert_eq!(value.dtype(), Some(DType::F64));
        assert_eq!(value.dimensions(), Some([2, 2].as_slice()));
        assert_eq!(value.numel(), Some(4));
        assert!(!value.is_complex_numeric());
        assert!(value.shares_storage_with(&alias));
        assert_eq!(value.condition(), Ok(false));
    }

    #[test]
    fn aggregate_values_report_metadata_accessors_and_cow_storage() {
        let cell =
            Value::from(CellArray::from_values(Shape::new([0, 3]).unwrap(), Vec::new()).unwrap());
        let cell_alias = cell.clone();
        assert_eq!(cell.kind(), ValueKind::Cell);
        assert_eq!(cell.class_name(), "cell");
        assert_eq!(cell.dtype(), None);
        assert_eq!(cell.dimensions(), Some([0, 3].as_slice()));
        assert_eq!(cell.numel(), Some(0));
        assert!(!cell.is_scalar());
        assert!(cell.as_cell().is_some());
        assert!(cell.shares_storage_with(&cell_alias));
        assert!(cell.shares_array_storage_with(&cell_alias));

        let structure = Value::from(
            StructArray::from_columns(
                Shape::new([2, 1]).unwrap(),
                vec![FieldName::new("payload").unwrap()],
                vec![vec![Value::Double(1.0), Value::Double(2.0)]],
            )
            .unwrap(),
        );
        let mut structure_alias = structure.clone();
        assert_eq!(structure.kind(), ValueKind::Struct);
        assert_eq!(structure.class_name(), "struct");
        assert_eq!(structure.dtype(), None);
        assert_eq!(structure.dimensions(), Some([2, 1].as_slice()));
        assert_eq!(structure.numel(), Some(2));
        assert!(structure.as_struct().is_some());
        assert!(structure.shares_storage_with(&structure_alias));

        structure_alias
            .as_struct_mut()
            .unwrap()
            .replace_at(0, 1, Value::Double(9.0))
            .unwrap();
        assert!(!structure.shares_storage_with(&structure_alias));
        assert_eq!(
            structure.as_struct().unwrap().value_at(0, 1),
            Some(&Value::Double(2.0))
        );
        assert_eq!(
            structure_alias.as_struct().unwrap().value_at(0, 1),
            Some(&Value::Double(9.0))
        );
        assert_eq!(
            structure.condition(),
            Err(ConditionError {
                actual: ValueKind::Struct,
            })
        );
    }

    #[test]
    fn array_values_clone_storage_and_detach_on_first_write() {
        let original = Value::Array(ArrayData::F64(
            DenseArray::from_vec(Shape::new([2, 1]).unwrap(), vec![1.0, 2.0]).unwrap(),
        ));
        let mut assigned = original.clone();
        assert!(original.shares_array_storage_with(&assigned));

        {
            let Some(ArrayData::F64(array)) = assigned.as_array_mut() else {
                panic!("test value must be a real array");
            };
            *array.get_mut_linear(1).unwrap() = 10.0;
        }

        assert!(!original.shares_array_storage_with(&assigned));
        let Some(ArrayData::F64(original_array)) = original.as_array() else {
            panic!("test value must be a real array");
        };
        assert_eq!(original_array.as_slice(), &[1.0, 2.0]);
        let Some(ArrayData::F64(assigned_array)) = assigned.as_array() else {
            panic!("test value must be a real array");
        };
        assert_eq!(assigned_array.as_slice(), &[10.0, 2.0]);
    }

    #[test]
    fn scalar_and_array_complex_conversion_copies_components() {
        let array = ArrayComplex64::new(2.5, -7.0);
        let scalar = Complex64::from(array);
        assert_eq!(scalar, Complex64::new(2.5, -7.0));
        assert_eq!(ArrayComplex64::from(scalar), array);

        let one_by_one = Value::Array(ArrayData::ComplexF64(
            DenseArray::from_vec(Shape::new([1, 1]).unwrap(), vec![array]).unwrap(),
        ));
        assert_eq!(one_by_one.as_complex_number(), Some(scalar));
    }

    #[test]
    fn dense_array_conditions_require_all_nonzero_and_treat_empty_as_false() {
        let real_true = Value::Array(ArrayData::F64(
            DenseArray::from_vec(Shape::new([1, 2]).unwrap(), vec![1.0, 2.0]).unwrap(),
        ));
        let real_false = Value::Array(ArrayData::F64(
            DenseArray::from_vec(Shape::new([1, 2]).unwrap(), vec![1.0, 0.0]).unwrap(),
        ));
        let complex_true = Value::Array(ArrayData::ComplexF64(
            DenseArray::from_vec(
                Shape::new([2, 1]).unwrap(),
                vec![ArrayComplex64::new(0.0, 1.0), ArrayComplex64::new(2.0, 0.0)],
            )
            .unwrap(),
        ));
        let logical_false = Value::Array(ArrayData::Logical(
            DenseArray::from_vec(
                Shape::new([1, 2]).unwrap(),
                vec![Logical::TRUE, Logical::FALSE],
            )
            .unwrap(),
        ));
        let empty = Value::Array(ArrayData::F64(
            DenseArray::from_vec(Shape::new([0, 0]).unwrap(), Vec::new()).unwrap(),
        ));

        assert_eq!(real_true.condition(), Ok(true));
        assert_eq!(real_false.condition(), Ok(false));
        assert_eq!(complex_true.condition(), Ok(true));
        assert_eq!(logical_false.condition(), Ok(false));
        assert_eq!(empty.condition(), Ok(false));
    }

    #[test]
    fn long_condition_scans_expose_periodic_interruption() {
        let value = Value::Array(ArrayData::F64(
            DenseArray::from_vec(Shape::new([1, 2_049]).unwrap(), vec![1.0; 2_049]).unwrap(),
        ));
        let mut checkpoints = 0;
        let evaluation = value
            .condition_with_interrupt(|| {
                checkpoints += 1;
                checkpoints == 2
            })
            .unwrap();

        assert_eq!(evaluation, ConditionEvaluation::Interrupted);
        assert_eq!(checkpoints, 2);
    }

    #[test]
    fn string_arrays_preserve_class_shape_numel_and_copy_on_write() {
        let empty_scalar = Value::from("");
        assert_eq!(empty_scalar.dimensions(), Some([1, 1].as_slice()));
        assert_eq!(empty_scalar.numel(), Some(1));
        let Value::String(empty_scalar) = &empty_scalar else {
            panic!("empty string scalar must retain string storage");
        };
        assert_eq!(empty_scalar.utf16_code_unit_len(), 0);

        let scalar = Value::from("alpha");
        assert_eq!(scalar.class_name(), "string");
        assert_eq!(scalar.dimensions(), Some([1, 1].as_slice()));
        assert_eq!(
            scalar.as_string_scalar().map(StringElement::to_utf8_lossy),
            Some(String::from("alpha"))
        );

        let array = StringArray::from_vec(
            Shape::new([1, 2]).unwrap(),
            vec![Arc::from("alpha"), Arc::from("beta")],
        )
        .unwrap();
        let original = Value::from(array);
        let mut assigned = original.clone();
        assert_eq!(original.kind(), ValueKind::String);
        assert_eq!(original.class_name(), "string");
        assert_eq!(original.dimensions(), Some([1, 2].as_slice()));
        assert_eq!(original.numel(), Some(2));
        assert_eq!(original.as_string_scalar(), None);
        assert!(original.shares_array_storage_with(&assigned));

        assigned
            .as_string_array_mut()
            .unwrap()
            .replace_linear(1, "changed")
            .unwrap();
        assert!(!original.shares_array_storage_with(&assigned));
        assert_eq!(
            original.as_string_array().unwrap().as_slice()[0].code_units(),
            "alpha".encode_utf16().collect::<Vec<_>>()
        );
        assert_eq!(
            assigned.as_string_array().unwrap().as_slice()[0].code_units(),
            "changed".encode_utf16().collect::<Vec<_>>()
        );
        assert_eq!(
            original.condition(),
            Err(ConditionError {
                actual: ValueKind::String
            })
        );
    }

    #[test]
    fn homogeneous_object_arrays_preserve_dynamic_class_shape_and_cow_handles() {
        let array = ObjectArray::from_vec(
            ClassHandle::new(7),
            Shape::new([1, 2]).unwrap(),
            vec![ObjectHandle::new(11), ObjectHandle::new(12)],
        )
        .unwrap();
        let original = Value::ObjectArray(array);
        let mut assigned = original.clone();

        assert_eq!(original.kind(), ValueKind::Object);
        assert_eq!(original.class_name(), "object");
        assert_eq!(original.dimensions(), Some([1, 2].as_slice()));
        assert_eq!(original.numel(), Some(2));
        assert!(original.shares_array_storage_with(&assigned));

        let Value::ObjectArray(array) = &mut assigned else {
            panic!("test value must remain an object array");
        };
        array.as_mut_slice()[0] = ObjectHandle::new(13);
        assert!(!original.shares_array_storage_with(&assigned));
        let Value::ObjectArray(original) = original else {
            unreachable!();
        };
        assert_eq!(original.class_handle(), ClassHandle::new(7));
        assert_eq!(original.as_slice()[0], ObjectHandle::new(11));
    }
}
