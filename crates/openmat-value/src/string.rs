use std::sync::Arc;

use openmat_array::{ArrayError, DenseArray, Shape};

/// One exact MATLAB string element.
///
/// The canonical payload is a sequence of UTF-16 code units, not Rust UTF-8
/// text. Every `u16`, including an isolated surrogate, is preserved. Missing is
/// an independent state and is never inferred from an empty payload. A missing
/// element always normalizes its payload to empty.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct StringElement {
    code_units: Arc<[u16]>,
    missing: bool,
}

impl StringElement {
    /// Constructs a non-missing element from exact UTF-16 code units.
    #[must_use]
    pub fn from_code_units(code_units: impl Into<Arc<[u16]>>) -> Self {
        Self {
            code_units: code_units.into(),
            missing: false,
        }
    }

    /// Constructs a non-missing element from valid UTF-8 text.
    #[must_use]
    pub fn from_utf8(value: &str) -> Self {
        Self::from_code_units(value.encode_utf16().collect::<Vec<_>>())
    }

    /// Constructs the canonical missing string element.
    #[must_use]
    pub fn missing() -> Self {
        Self {
            code_units: Arc::from([]),
            missing: true,
        }
    }

    /// Returns the exact UTF-16 payload.
    #[must_use]
    pub fn code_units(&self) -> &[u16] {
        &self.code_units
    }

    /// Returns whether this is a missing element.
    #[must_use]
    pub const fn is_missing(&self) -> bool {
        self.missing
    }

    /// Returns the number of exact UTF-16 code units.
    #[must_use]
    pub fn code_unit_len(&self) -> usize {
        self.code_units.len()
    }

    /// Returns whether the code-unit payload is empty.
    ///
    /// Both an empty string and a missing string have an empty payload; inspect
    /// [`Self::is_missing`] to distinguish them.
    #[must_use]
    pub fn is_payload_empty(&self) -> bool {
        self.code_units.is_empty()
    }

    /// Converts the payload to UTF-8 for display, replacing invalid sequences.
    ///
    /// This helper is intentionally named as lossy. Its result must not be used
    /// for equality, indexing, protocol encoding, or conformance comparison.
    #[must_use]
    pub fn to_utf8_lossy(&self) -> String {
        String::from_utf16_lossy(&self.code_units)
    }
}

impl From<&str> for StringElement {
    fn from(value: &str) -> Self {
        Self::from_utf8(value)
    }
}

impl From<String> for StringElement {
    fn from(value: String) -> Self {
        Self::from_utf8(&value)
    }
}

impl From<Arc<str>> for StringElement {
    fn from(value: Arc<str>) -> Self {
        Self::from_utf8(&value)
    }
}

impl From<Vec<u16>> for StringElement {
    fn from(value: Vec<u16>) -> Self {
        Self::from_code_units(value)
    }
}

impl From<Arc<[u16]>> for StringElement {
    fn from(value: Arc<[u16]>) -> Self {
        Self::from_code_units(value)
    }
}

/// An owned dense array of exact string elements with copy-on-write storage.
///
/// The shape follows the same canonical column-major conventions as numeric
/// arrays. Cloning shares the dense element buffer; replacing a string in
/// either clone detaches that clone. The Rust representation is not a plugin or
/// process ABI.
#[derive(Clone, Debug, PartialEq)]
pub struct StringArray {
    storage: DenseArray<StringElement>,
}

impl StringArray {
    /// Constructs a string array from canonical shape and valid UTF-8 values.
    ///
    /// This compatibility constructor converts the previous `Arc<str>` input
    /// into canonical UTF-16 storage. Use [`Self::from_elements`] when exact
    /// code units or missing elements are already available.
    ///
    /// # Errors
    ///
    /// Returns the array core's checked length or host-size error.
    pub fn from_vec(shape: Shape, values: Vec<Arc<str>>) -> Result<Self, ArrayError> {
        let values = values.into_iter().map(Into::into).collect();
        DenseArray::from_vec(shape, values).map(|storage| Self { storage })
    }

    /// Constructs a string array from exact column-major string elements.
    ///
    /// # Errors
    ///
    /// Returns the array core's checked length or host-size error.
    pub fn from_elements(shape: Shape, values: Vec<StringElement>) -> Result<Self, ArrayError> {
        DenseArray::from_vec(shape, values).map(|storage| Self { storage })
    }

    /// Returns the canonical language-visible shape.
    #[must_use]
    pub const fn shape(&self) -> &Shape {
        self.storage.shape()
    }

    /// Returns the number of string elements.
    #[must_use]
    pub const fn numel(&self) -> u64 {
        self.storage.numel()
    }

    /// Returns exact string elements in column-major linear order.
    #[must_use]
    pub fn as_slice(&self) -> &[StringElement] {
        self.storage.as_slice()
    }

    /// Returns whether two clones currently share the same element buffer.
    #[must_use]
    pub fn shares_storage_with(&self, other: &Self) -> bool {
        self.storage.shares_storage_with(&other.storage)
    }

    /// Replaces a string at a one-based linear language index.
    ///
    /// Shared storage is detached only after the index has been checked.
    ///
    /// # Errors
    ///
    /// Returns an indexing error when `one_based_index` is outside the array.
    pub fn replace_linear(
        &mut self,
        one_based_index: u64,
        value: impl Into<StringElement>,
    ) -> Result<StringElement, ArrayError> {
        let target = self.storage.get_mut_linear(one_based_index)?;
        Ok(std::mem::replace(target, value.into()))
    }
}

/// Scalar-optimized or array-valued storage carried by [`crate::Value::String`].
#[derive(Clone, Debug, PartialEq)]
pub enum StringValue {
    /// One exact scalar string without a dense shape allocation.
    Scalar(StringElement),
    /// An explicit shaped array of exact string elements.
    Array(StringArray),
}

impl StringValue {
    /// Constructs an optimized non-missing scalar from exact or UTF-8 input.
    #[must_use]
    pub fn scalar(value: impl Into<StringElement>) -> Self {
        Self::Scalar(value.into())
    }

    /// Constructs an optimized missing scalar.
    #[must_use]
    pub fn missing() -> Self {
        Self::Scalar(StringElement::missing())
    }

    /// Constructs an explicit shaped string array.
    #[must_use]
    pub const fn array(value: StringArray) -> Self {
        Self::Array(value)
    }

    /// Returns the canonical language-visible dimensions.
    #[must_use]
    pub fn dimensions(&self) -> &[u64] {
        match self {
            Self::Scalar(_) => &[1, 1],
            Self::Array(array) => array.shape().dimensions(),
        }
    }

    /// Returns the language-visible string element count.
    #[must_use]
    pub const fn numel(&self) -> u64 {
        match self {
            Self::Scalar(_) => 1,
            Self::Array(array) => array.numel(),
        }
    }

    /// Borrows the value as one exact string scalar.
    ///
    /// Explicit `1x1` arrays are scalar for language dispatch; other shapes
    /// return `None`.
    #[must_use]
    pub fn as_scalar(&self) -> Option<&StringElement> {
        match self {
            Self::Scalar(value) => Some(value),
            Self::Array(array) if array.numel() == 1 => array.as_slice().first(),
            Self::Array(_) => None,
        }
    }

    /// Borrows explicit array storage.
    #[must_use]
    pub const fn as_array(&self) -> Option<&StringArray> {
        match self {
            Self::Array(array) => Some(array),
            Self::Scalar(_) => None,
        }
    }

    /// Mutably borrows explicit array storage.
    pub const fn as_array_mut(&mut self) -> Option<&mut StringArray> {
        match self {
            Self::Array(array) => Some(array),
            Self::Scalar(_) => None,
        }
    }

    /// Returns one exact element by zero-based internal linear offset.
    #[must_use]
    pub fn element(&self, offset: usize) -> Option<&StringElement> {
        match self {
            Self::Scalar(value) if offset == 0 => Some(value),
            Self::Array(array) => array.as_slice().get(offset),
            Self::Scalar(_) => None,
        }
    }

    /// Returns total UTF-16 code units across all string elements.
    ///
    /// This is storage metadata, not MATLAB `isempty` semantics. In particular,
    /// a scalar empty string has zero payload code units but still has one
    /// element.
    #[must_use]
    pub fn utf16_code_unit_len(&self) -> usize {
        match self {
            Self::Scalar(value) => value.code_unit_len(),
            Self::Array(array) => array.as_slice().iter().fold(0_usize, |total, value| {
                total.saturating_add(value.code_unit_len())
            }),
        }
    }
}

impl From<StringElement> for StringValue {
    fn from(value: StringElement) -> Self {
        Self::Scalar(value)
    }
}

impl From<String> for StringValue {
    fn from(value: String) -> Self {
        Self::scalar(value)
    }
}

impl From<&str> for StringValue {
    fn from(value: &str) -> Self {
        Self::scalar(value)
    }
}

impl From<Arc<str>> for StringValue {
    fn from(value: Arc<str>) -> Self {
        Self::scalar(value)
    }
}

impl From<StringArray> for StringValue {
    fn from(value: StringArray) -> Self {
        Self::Array(value)
    }
}
