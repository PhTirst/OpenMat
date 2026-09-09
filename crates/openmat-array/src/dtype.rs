use std::ops::{Add, AddAssign, Mul};

use crate::DenseArray;

/// Element storage classes implemented by the dense-array core.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DType {
    /// IEEE-754 binary32 real values (MATLAB `single`).
    F32,
    /// A pair of IEEE-754 binary32 real and imaginary components.
    ComplexF32,
    /// IEEE-754 binary64 real values (MATLAB `double`).
    F64,
    /// A pair of IEEE-754 binary64 real and imaginary components.
    ComplexF64,
    /// Logical false/true values.
    Logical,
    /// MATLAB `char` UTF-16 code units.
    Char,
    /// Real signed 8-bit integers.
    I8,
    /// Complex signed 8-bit integer components.
    ComplexI8,
    /// Real unsigned 8-bit integers.
    U8,
    /// Complex unsigned 8-bit integer components.
    ComplexU8,
    /// Real signed 16-bit integers.
    I16,
    /// Complex signed 16-bit integer components.
    ComplexI16,
    /// Real unsigned 16-bit integers.
    U16,
    /// Complex unsigned 16-bit integer components.
    ComplexU16,
    /// Real signed 32-bit integers.
    I32,
    /// Complex signed 32-bit integer components.
    ComplexI32,
    /// Real unsigned 32-bit integers.
    U32,
    /// Complex unsigned 32-bit integer components.
    ComplexU32,
    /// Real signed 64-bit integers.
    I64,
    /// Complex signed 64-bit integer components.
    ComplexI64,
    /// Real unsigned 64-bit integers.
    U64,
    /// Complex unsigned 64-bit integer components.
    ComplexU64,
}

impl DType {
    /// Returns the MATLAB-compatible dynamic class name.
    ///
    /// Real and complex storage of the same component type have the same
    /// language class.
    #[must_use]
    pub const fn class_name(self) -> &'static str {
        match self {
            Self::F32 | Self::ComplexF32 => "single",
            Self::F64 | Self::ComplexF64 => "double",
            Self::Logical => "logical",
            Self::Char => "char",
            Self::I8 | Self::ComplexI8 => "int8",
            Self::U8 | Self::ComplexU8 => "uint8",
            Self::I16 | Self::ComplexI16 => "int16",
            Self::U16 | Self::ComplexU16 => "uint16",
            Self::I32 | Self::ComplexI32 => "int32",
            Self::U32 | Self::ComplexU32 => "uint32",
            Self::I64 | Self::ComplexI64 => "int64",
            Self::U64 | Self::ComplexU64 => "uint64",
        }
    }

    /// Returns whether each stored element has real and imaginary components.
    #[must_use]
    pub const fn is_complex(self) -> bool {
        matches!(
            self,
            Self::ComplexF32
                | Self::ComplexF64
                | Self::ComplexI8
                | Self::ComplexU8
                | Self::ComplexI16
                | Self::ComplexU16
                | Self::ComplexI32
                | Self::ComplexU32
                | Self::ComplexI64
                | Self::ComplexU64
        )
    }

    /// Returns whether this is one of the real or complex integer storages.
    #[must_use]
    pub const fn is_integer(self) -> bool {
        matches!(
            self,
            Self::I8
                | Self::ComplexI8
                | Self::U8
                | Self::ComplexU8
                | Self::I16
                | Self::ComplexI16
                | Self::U16
                | Self::ComplexU16
                | Self::I32
                | Self::ComplexI32
                | Self::U32
                | Self::ComplexU32
                | Self::I64
                | Self::ComplexI64
                | Self::U64
                | Self::ComplexU64
        )
    }

    /// Returns the byte width of one real or imaginary component.
    ///
    /// For non-complex storage this is also the stored element width. Logical
    /// storage is explicitly one byte and `char` storage is one two-byte UTF-16
    /// code unit. These widths are also the OEX element widths.
    #[must_use]
    pub const fn component_width_bytes(self) -> usize {
        match self {
            Self::Logical | Self::I8 | Self::ComplexI8 | Self::U8 | Self::ComplexU8 => 1,
            Self::Char | Self::I16 | Self::ComplexI16 | Self::U16 | Self::ComplexU16 => 2,
            Self::F32
            | Self::ComplexF32
            | Self::I32
            | Self::ComplexI32
            | Self::U32
            | Self::ComplexU32 => 4,
            Self::F64
            | Self::ComplexF64
            | Self::I64
            | Self::ComplexI64
            | Self::U64
            | Self::ComplexU64 => 8,
        }
    }

    /// Returns the payload width of one stored element.
    #[must_use]
    pub const fn element_width_bytes(self) -> usize {
        if self.is_complex() {
            self.component_width_bytes() * 2
        } else {
            self.component_width_bytes()
        }
    }
}

/// An explicitly represented complex binary32 element.
///
/// Both components remain `f32`; no construction or access path widens the
/// stored value to `f64`. Its interleaved fields are the OEX complex-f32
/// element representation.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Complex32 {
    /// Real component.
    pub re: f32,
    /// Imaginary component.
    pub im: f32,
}

impl Complex32 {
    /// Additive identity.
    pub const ZERO: Self = Self { re: 0.0, im: 0.0 };

    /// Constructs an exact pair of binary32 components.
    #[must_use]
    pub const fn new(re: f32, im: f32) -> Self {
        Self { re, im }
    }

    /// Returns the complex conjugate without changing component precision.
    #[must_use]
    pub const fn conjugate(self) -> Self {
        Self {
            re: self.re,
            im: -self.im,
        }
    }
}

impl From<f32> for Complex32 {
    fn from(value: f32) -> Self {
        Self::new(value, 0.0)
    }
}

/// One MATLAB `char` element represented as an exact UTF-16 code unit.
///
/// Every `u16`, including an isolated surrogate, is valid. This distinct type
/// prevents `char` storage from being confused with MATLAB `uint16` storage.
/// Its transparent `u16` layout is the OEX char16 element representation.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CharCodeUnit(u16);

impl CharCodeUnit {
    /// Constructs a code unit without Unicode scalar-value validation.
    #[must_use]
    pub const fn new(value: u16) -> Self {
        Self(value)
    }

    /// Returns the exact stored code unit.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

impl From<u16> for CharCodeUnit {
    fn from(value: u16) -> Self {
        Self::new(value)
    }
}

impl From<CharCodeUnit> for u16 {
    fn from(value: CharCodeUnit) -> Self {
        value.get()
    }
}

/// Exact real and imaginary integer components for complex integer storage.
///
/// Only the eight fixed-width primitive integer instantiations implement
/// [`ArrayElement`]. Those supported instantiations use the same interleaved
/// field layout as the corresponding OEX complex-integer representation.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ComplexInteger<T> {
    re: T,
    im: T,
}

impl<T> ComplexInteger<T> {
    /// Constructs a value from exact real and imaginary components.
    #[must_use]
    pub const fn new(re: T, im: T) -> Self {
        Self { re, im }
    }

    /// Borrows the real component.
    #[must_use]
    pub const fn real(&self) -> &T {
        &self.re
    }

    /// Borrows the imaginary component.
    #[must_use]
    pub const fn imaginary(&self) -> &T {
        &self.im
    }

    /// Decomposes the value into exact real and imaginary components.
    #[must_use]
    pub fn into_parts(self) -> (T, T) {
        (self.re, self.im)
    }
}

impl<T: Copy> ComplexInteger<T> {
    /// Copies the real component.
    #[must_use]
    pub const fn re(self) -> T {
        self.re
    }

    /// Copies the imaginary component.
    #[must_use]
    pub const fn im(self) -> T {
        self.im
    }
}

/// An explicitly represented complex binary64 scalar.
///
/// Its interleaved fields are the OEX complex-f64 element representation.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Complex64 {
    /// Real component.
    pub re: f64,
    /// Imaginary component.
    pub im: f64,
}

impl Complex64 {
    /// Additive identity.
    pub const ZERO: Self = Self { re: 0.0, im: 0.0 };

    /// Constructs a complex value.
    #[must_use]
    pub const fn new(re: f64, im: f64) -> Self {
        Self { re, im }
    }

    /// Returns the complex conjugate.
    #[must_use]
    pub const fn conjugate(self) -> Self {
        Self {
            re: self.re,
            im: -self.im,
        }
    }
}

impl Add for Complex64 {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self {
            re: self.re + rhs.re,
            im: self.im + rhs.im,
        }
    }
}

impl AddAssign for Complex64 {
    fn add_assign(&mut self, rhs: Self) {
        self.re += rhs.re;
        self.im += rhs.im;
    }
}

impl Mul for Complex64 {
    type Output = Self;

    fn mul(self, rhs: Self) -> Self::Output {
        Self {
            re: self.re.mul_add(rhs.re, -(self.im * rhs.im)),
            im: self.re.mul_add(rhs.im, self.im * rhs.re),
        }
    }
}

/// One-byte logical storage shared by the Rust core and the stable C ABI.
///
/// Constructors produce the canonical bytes zero and one. Native extensions
/// must likewise write only those values through mutable OEX views.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Logical(u8);

impl Logical {
    /// Logical false.
    pub const FALSE: Self = Self(0);
    /// Logical true.
    pub const TRUE: Self = Self(1);

    /// Returns the contained Boolean value.
    #[must_use]
    pub const fn get(self) -> bool {
        self.0 != 0
    }
}

impl From<bool> for Logical {
    fn from(value: bool) -> Self {
        if value { Self::TRUE } else { Self::FALSE }
    }
}

impl From<Logical> for bool {
    fn from(value: Logical) -> Self {
        value.get()
    }
}

mod private {
    pub trait Sealed {}
}

/// A scalar type accepted by the dense-array core.
///
/// The trait is sealed so downstream crates cannot claim unsupported element
/// classes. It is a Rust-internal API and must not be used as a stable external
/// ABI; external boundaries should carry an explicit [`DType`] and serialized
/// data. [`ArrayData::as_typed`] and [`ArrayData::as_typed_mut`] expose exact
/// typed storage without requiring callers to repeat dynamic variant matches.
pub trait ArrayElement: private::Sealed + Clone + Sized + 'static {
    /// Runtime storage tag for this element type.
    const DTYPE: DType;

    #[doc(hidden)]
    fn array_ref(data: &ArrayData) -> Option<&DenseArray<Self>>;

    #[doc(hidden)]
    fn array_mut(data: &mut ArrayData) -> Option<&mut DenseArray<Self>>;

    #[doc(hidden)]
    fn into_array(array: DenseArray<Self>) -> ArrayData;
}

/// A fixed-width primitive or complex integer accepted by integer storage.
///
/// This sealed trait powers the generic typed accessors on
/// [`IntegerArrayData`].
pub trait IntegerElement: ArrayElement {
    #[doc(hidden)]
    fn integer_ref(data: &IntegerArrayData) -> Option<&DenseArray<Self>>;

    #[doc(hidden)]
    fn integer_mut(data: &mut IntegerArrayData) -> Option<&mut DenseArray<Self>>;

    #[doc(hidden)]
    fn into_integer(array: DenseArray<Self>) -> IntegerArrayData;
}

macro_rules! impl_array_element {
    ($element:ty, $dtype:ident, $variant:ident) => {
        impl private::Sealed for $element {}

        impl ArrayElement for $element {
            const DTYPE: DType = DType::$dtype;

            fn array_ref(data: &ArrayData) -> Option<&DenseArray<Self>> {
                match data {
                    ArrayData::$variant(array) => Some(array),
                    _ => None,
                }
            }

            fn array_mut(data: &mut ArrayData) -> Option<&mut DenseArray<Self>> {
                match data {
                    ArrayData::$variant(array) => Some(array),
                    _ => None,
                }
            }

            fn into_array(array: DenseArray<Self>) -> ArrayData {
                ArrayData::$variant(array)
            }
        }
    };
}

impl_array_element!(f32, F32, F32);
impl_array_element!(Complex32, ComplexF32, ComplexF32);
impl_array_element!(f64, F64, F64);
impl_array_element!(Complex64, ComplexF64, ComplexF64);
impl_array_element!(Logical, Logical, Logical);
impl_array_element!(CharCodeUnit, Char, Char);

/// Exact fixed-width integer dense storage.
///
/// Real and complex storage are distinct variants, while [`ArrayData`] exposes
/// the entire integer family through one nested variant. This enum's Rust
/// layout and generic instantiations are not a stable ABI.
#[derive(Clone, Debug, PartialEq)]
pub enum IntegerArrayData {
    /// Real signed 8-bit values.
    I8(DenseArray<i8>),
    /// Complex signed 8-bit values.
    ComplexI8(DenseArray<ComplexInteger<i8>>),
    /// Real unsigned 8-bit values.
    U8(DenseArray<u8>),
    /// Complex unsigned 8-bit values.
    ComplexU8(DenseArray<ComplexInteger<u8>>),
    /// Real signed 16-bit values.
    I16(DenseArray<i16>),
    /// Complex signed 16-bit values.
    ComplexI16(DenseArray<ComplexInteger<i16>>),
    /// Real unsigned 16-bit values.
    U16(DenseArray<u16>),
    /// Complex unsigned 16-bit values.
    ComplexU16(DenseArray<ComplexInteger<u16>>),
    /// Real signed 32-bit values.
    I32(DenseArray<i32>),
    /// Complex signed 32-bit values.
    ComplexI32(DenseArray<ComplexInteger<i32>>),
    /// Real unsigned 32-bit values.
    U32(DenseArray<u32>),
    /// Complex unsigned 32-bit values.
    ComplexU32(DenseArray<ComplexInteger<u32>>),
    /// Real signed 64-bit values.
    I64(DenseArray<i64>),
    /// Complex signed 64-bit values.
    ComplexI64(DenseArray<ComplexInteger<i64>>),
    /// Real unsigned 64-bit values.
    U64(DenseArray<u64>),
    /// Complex unsigned 64-bit values.
    ComplexU64(DenseArray<ComplexInteger<u64>>),
}

/// One exact dynamically typed integer component.
///
/// Signed and unsigned domains remain distinct. Fixed-width stored components
/// are widened losslessly, so `uint64::MAX` and every signed minimum remain
/// representable without floating point.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IntegerComponent {
    /// A sign-extended fixed-width signed component.
    Signed(i128),
    /// A zero-extended fixed-width unsigned component.
    Unsigned(u128),
}

impl IntegerComponent {
    /// Returns the signed value, or `None` for an unsigned component.
    #[must_use]
    pub const fn as_signed(self) -> Option<i128> {
        match self {
            Self::Signed(value) => Some(value),
            Self::Unsigned(_) => None,
        }
    }

    /// Returns the unsigned value, or `None` for a signed component.
    #[must_use]
    pub const fn as_unsigned(self) -> Option<u128> {
        match self {
            Self::Unsigned(value) => Some(value),
            Self::Signed(_) => None,
        }
    }

    /// Returns whether the exact component is zero.
    #[must_use]
    pub const fn is_zero(self) -> bool {
        match self {
            Self::Signed(value) => value == 0,
            Self::Unsigned(value) => value == 0,
        }
    }

    /// Formats the component as canonical base-ten text.
    #[must_use]
    pub fn canonical_decimal(self) -> String {
        match self {
            Self::Signed(value) => value.to_string(),
            Self::Unsigned(value) => value.to_string(),
        }
    }
}

/// One exact dynamically viewed real or complex integer element.
///
/// `imaginary_component()` is `None` for real storage and `Some`, including an
/// exact zero, for complex storage. Each component retains signedness.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct IntegerElementValue {
    real: IntegerComponent,
    imaginary: Option<IntegerComponent>,
}

impl IntegerElementValue {
    fn real(real: IntegerComponent) -> Self {
        Self {
            real,
            imaginary: None,
        }
    }

    fn complex(real: IntegerComponent, imaginary: IntegerComponent) -> Self {
        Self {
            real,
            imaginary: Some(imaginary),
        }
    }

    /// Returns the exact real component.
    #[must_use]
    pub const fn real_component(self) -> IntegerComponent {
        self.real
    }

    /// Returns the exact imaginary component for complex storage.
    #[must_use]
    pub const fn imaginary_component(self) -> Option<IntegerComponent> {
        self.imaginary
    }

    /// Returns whether this element came from complex storage.
    #[must_use]
    pub const fn is_complex(self) -> bool {
        self.imaginary.is_some()
    }

    /// Formats both components as canonical base-ten text.
    ///
    /// Real storage uses `"0"` for the imaginary text while
    /// [`Self::is_complex`] continues to distinguish the storage form.
    #[must_use]
    pub fn canonical_decimal(self) -> IntegerElementDecimal {
        IntegerElementDecimal {
            real: self.real.canonical_decimal(),
            imaginary: self
                .imaginary
                .map_or_else(|| String::from("0"), IntegerComponent::canonical_decimal),
        }
    }
}

/// Canonical decimal components of one exact integer element.
///
/// Real storage reports an imaginary component of `"0"`. Keeping both
/// components as decimal text lets JSON-safe consumers preserve the complete
/// signed and unsigned 64-bit ranges without an intermediate `f64`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct IntegerElementDecimal {
    real: String,
    imaginary: String,
}

impl IntegerElementDecimal {
    /// Returns the canonical decimal real component.
    #[must_use]
    pub fn real_component(&self) -> &str {
        &self.real
    }

    /// Returns the canonical decimal imaginary component.
    #[must_use]
    pub fn imaginary_component(&self) -> &str {
        &self.imaginary
    }

    /// Decomposes the value into owned canonical decimal components.
    #[must_use]
    pub fn into_components(self) -> (String, String) {
        (self.real, self.imaginary)
    }
}

/// Iterator over exact dynamic integer elements in column-major linear order.
#[derive(Clone, Debug)]
pub struct IntegerElements<'a> {
    storage: &'a IntegerArrayData,
    next: usize,
    end: usize,
}

impl Iterator for IntegerElements<'_> {
    type Item = IntegerElementValue;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next == self.end {
            return None;
        }
        let value = self.storage.element(self.next);
        self.next += 1;
        value
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.end - self.next;
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for IntegerElements<'_> {}
impl std::iter::FusedIterator for IntegerElements<'_> {}

macro_rules! dispatch_integer_array {
    ($value:expr, |$array:ident| $body:expr) => {
        match $value {
            IntegerArrayData::I8($array) => $body,
            IntegerArrayData::ComplexI8($array) => $body,
            IntegerArrayData::U8($array) => $body,
            IntegerArrayData::ComplexU8($array) => $body,
            IntegerArrayData::I16($array) => $body,
            IntegerArrayData::ComplexI16($array) => $body,
            IntegerArrayData::U16($array) => $body,
            IntegerArrayData::ComplexU16($array) => $body,
            IntegerArrayData::I32($array) => $body,
            IntegerArrayData::ComplexI32($array) => $body,
            IntegerArrayData::U32($array) => $body,
            IntegerArrayData::ComplexU32($array) => $body,
            IntegerArrayData::I64($array) => $body,
            IntegerArrayData::ComplexI64($array) => $body,
            IntegerArrayData::U64($array) => $body,
            IntegerArrayData::ComplexU64($array) => $body,
        }
    };
}

impl IntegerArrayData {
    /// Constructs integer storage from an exact supported typed dense array.
    #[must_use]
    pub fn from_typed<T: IntegerElement>(array: DenseArray<T>) -> Self {
        T::into_integer(array)
    }

    /// Returns the exact typed dense array when the requested type matches.
    #[must_use]
    pub fn as_typed<T: IntegerElement>(&self) -> Option<&DenseArray<T>> {
        T::integer_ref(self)
    }

    /// Returns the exact mutable typed dense array when its type matches.
    ///
    /// Mutable element access through the returned dense array preserves its
    /// checked copy-on-write detachment semantics.
    pub fn as_typed_mut<T: IntegerElement>(&mut self) -> Option<&mut DenseArray<T>> {
        T::integer_mut(self)
    }

    /// Returns the exact runtime storage class.
    #[must_use]
    pub const fn dtype(&self) -> DType {
        match self {
            Self::I8(_) => DType::I8,
            Self::ComplexI8(_) => DType::ComplexI8,
            Self::U8(_) => DType::U8,
            Self::ComplexU8(_) => DType::ComplexU8,
            Self::I16(_) => DType::I16,
            Self::ComplexI16(_) => DType::ComplexI16,
            Self::U16(_) => DType::U16,
            Self::ComplexU16(_) => DType::ComplexU16,
            Self::I32(_) => DType::I32,
            Self::ComplexI32(_) => DType::ComplexI32,
            Self::U32(_) => DType::U32,
            Self::ComplexU32(_) => DType::ComplexU32,
            Self::I64(_) => DType::I64,
            Self::ComplexI64(_) => DType::ComplexI64,
            Self::U64(_) => DType::U64,
            Self::ComplexU64(_) => DType::ComplexU64,
        }
    }

    /// Returns the MATLAB-compatible integer class name.
    #[must_use]
    pub const fn class_name(&self) -> &'static str {
        self.dtype().class_name()
    }

    /// Returns whether the storage has real and imaginary components.
    #[must_use]
    pub const fn is_complex(&self) -> bool {
        self.dtype().is_complex()
    }

    /// Returns the byte width of one integer component.
    #[must_use]
    pub const fn component_width_bytes(&self) -> usize {
        self.dtype().component_width_bytes()
    }

    /// Returns the canonical array shape.
    #[must_use]
    pub fn shape(&self) -> &crate::Shape {
        dispatch_integer_array!(self, |array| array.shape())
    }

    /// Returns the number of elements.
    #[must_use]
    pub fn numel(&self) -> u64 {
        self.shape().numel()
    }

    /// Reads one exact dynamic element by zero-based internal linear offset.
    ///
    /// Returns `None` when `offset` is outside the column-major element buffer.
    /// Signedness and real-versus-complex storage are retained, and no
    /// component is converted through floating point.
    #[must_use]
    pub fn element(&self, offset: usize) -> Option<IntegerElementValue> {
        match self {
            Self::I8(array) => array.as_slice().get(offset).map(|value| {
                IntegerElementValue::real(IntegerComponent::Signed(i128::from(*value)))
            }),
            Self::ComplexI8(array) => array.as_slice().get(offset).map(|value| {
                IntegerElementValue::complex(
                    IntegerComponent::Signed(i128::from(value.re())),
                    IntegerComponent::Signed(i128::from(value.im())),
                )
            }),
            Self::U8(array) => array.as_slice().get(offset).map(|value| {
                IntegerElementValue::real(IntegerComponent::Unsigned(u128::from(*value)))
            }),
            Self::ComplexU8(array) => array.as_slice().get(offset).map(|value| {
                IntegerElementValue::complex(
                    IntegerComponent::Unsigned(u128::from(value.re())),
                    IntegerComponent::Unsigned(u128::from(value.im())),
                )
            }),
            Self::I16(array) => array.as_slice().get(offset).map(|value| {
                IntegerElementValue::real(IntegerComponent::Signed(i128::from(*value)))
            }),
            Self::ComplexI16(array) => array.as_slice().get(offset).map(|value| {
                IntegerElementValue::complex(
                    IntegerComponent::Signed(i128::from(value.re())),
                    IntegerComponent::Signed(i128::from(value.im())),
                )
            }),
            Self::U16(array) => array.as_slice().get(offset).map(|value| {
                IntegerElementValue::real(IntegerComponent::Unsigned(u128::from(*value)))
            }),
            Self::ComplexU16(array) => array.as_slice().get(offset).map(|value| {
                IntegerElementValue::complex(
                    IntegerComponent::Unsigned(u128::from(value.re())),
                    IntegerComponent::Unsigned(u128::from(value.im())),
                )
            }),
            Self::I32(array) => array.as_slice().get(offset).map(|value| {
                IntegerElementValue::real(IntegerComponent::Signed(i128::from(*value)))
            }),
            Self::ComplexI32(array) => array.as_slice().get(offset).map(|value| {
                IntegerElementValue::complex(
                    IntegerComponent::Signed(i128::from(value.re())),
                    IntegerComponent::Signed(i128::from(value.im())),
                )
            }),
            Self::U32(array) => array.as_slice().get(offset).map(|value| {
                IntegerElementValue::real(IntegerComponent::Unsigned(u128::from(*value)))
            }),
            Self::ComplexU32(array) => array.as_slice().get(offset).map(|value| {
                IntegerElementValue::complex(
                    IntegerComponent::Unsigned(u128::from(value.re())),
                    IntegerComponent::Unsigned(u128::from(value.im())),
                )
            }),
            Self::I64(array) => array.as_slice().get(offset).map(|value| {
                IntegerElementValue::real(IntegerComponent::Signed(i128::from(*value)))
            }),
            Self::ComplexI64(array) => array.as_slice().get(offset).map(|value| {
                IntegerElementValue::complex(
                    IntegerComponent::Signed(i128::from(value.re())),
                    IntegerComponent::Signed(i128::from(value.im())),
                )
            }),
            Self::U64(array) => array.as_slice().get(offset).map(|value| {
                IntegerElementValue::real(IntegerComponent::Unsigned(u128::from(*value)))
            }),
            Self::ComplexU64(array) => array.as_slice().get(offset).map(|value| {
                IntegerElementValue::complex(
                    IntegerComponent::Unsigned(u128::from(value.re())),
                    IntegerComponent::Unsigned(u128::from(value.im())),
                )
            }),
        }
    }

    /// Iterates exact dynamic elements in column-major linear order.
    #[must_use]
    pub fn elements(&self) -> IntegerElements<'_> {
        let end = dispatch_integer_array!(self, |array| array.as_slice().len());
        IntegerElements {
            storage: self,
            next: 0,
            end,
        }
    }

    /// Formats one zero-based internal element as exact decimal components.
    ///
    /// Returns `None` when `offset` is outside the column-major element buffer.
    /// No component is converted through floating point.
    #[must_use]
    pub fn element_decimal(&self, offset: usize) -> Option<IntegerElementDecimal> {
        self.element(offset)
            .map(IntegerElementValue::canonical_decimal)
    }

    /// Returns whether two values share the same typed element buffer.
    ///
    /// Different real/complex widths or signedness never share storage.
    #[must_use]
    pub fn shares_storage_with(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::I8(left), Self::I8(right)) => left.shares_storage_with(right),
            (Self::ComplexI8(left), Self::ComplexI8(right)) => left.shares_storage_with(right),
            (Self::U8(left), Self::U8(right)) => left.shares_storage_with(right),
            (Self::ComplexU8(left), Self::ComplexU8(right)) => left.shares_storage_with(right),
            (Self::I16(left), Self::I16(right)) => left.shares_storage_with(right),
            (Self::ComplexI16(left), Self::ComplexI16(right)) => left.shares_storage_with(right),
            (Self::U16(left), Self::U16(right)) => left.shares_storage_with(right),
            (Self::ComplexU16(left), Self::ComplexU16(right)) => left.shares_storage_with(right),
            (Self::I32(left), Self::I32(right)) => left.shares_storage_with(right),
            (Self::ComplexI32(left), Self::ComplexI32(right)) => left.shares_storage_with(right),
            (Self::U32(left), Self::U32(right)) => left.shares_storage_with(right),
            (Self::ComplexU32(left), Self::ComplexU32(right)) => left.shares_storage_with(right),
            (Self::I64(left), Self::I64(right)) => left.shares_storage_with(right),
            (Self::ComplexI64(left), Self::ComplexI64(right)) => left.shares_storage_with(right),
            (Self::U64(left), Self::U64(right)) => left.shares_storage_with(right),
            (Self::ComplexU64(left), Self::ComplexU64(right)) => left.shares_storage_with(right),
            _ => false,
        }
    }
}

macro_rules! impl_integer_element {
    ($element:ty, $dtype:ident, $variant:ident) => {
        impl private::Sealed for $element {}

        impl ArrayElement for $element {
            const DTYPE: DType = DType::$dtype;

            fn array_ref(data: &ArrayData) -> Option<&DenseArray<Self>> {
                match data {
                    ArrayData::Integer(IntegerArrayData::$variant(array)) => Some(array),
                    _ => None,
                }
            }

            fn array_mut(data: &mut ArrayData) -> Option<&mut DenseArray<Self>> {
                match data {
                    ArrayData::Integer(IntegerArrayData::$variant(array)) => Some(array),
                    _ => None,
                }
            }

            fn into_array(array: DenseArray<Self>) -> ArrayData {
                ArrayData::Integer(IntegerArrayData::$variant(array))
            }
        }

        impl IntegerElement for $element {
            fn integer_ref(data: &IntegerArrayData) -> Option<&DenseArray<Self>> {
                match data {
                    IntegerArrayData::$variant(array) => Some(array),
                    _ => None,
                }
            }

            fn integer_mut(data: &mut IntegerArrayData) -> Option<&mut DenseArray<Self>> {
                match data {
                    IntegerArrayData::$variant(array) => Some(array),
                    _ => None,
                }
            }

            fn into_integer(array: DenseArray<Self>) -> IntegerArrayData {
                IntegerArrayData::$variant(array)
            }
        }
    };
}

impl_integer_element!(i8, I8, I8);
impl_integer_element!(ComplexInteger<i8>, ComplexI8, ComplexI8);
impl_integer_element!(u8, U8, U8);
impl_integer_element!(ComplexInteger<u8>, ComplexU8, ComplexU8);
impl_integer_element!(i16, I16, I16);
impl_integer_element!(ComplexInteger<i16>, ComplexI16, ComplexI16);
impl_integer_element!(u16, U16, U16);
impl_integer_element!(ComplexInteger<u16>, ComplexU16, ComplexU16);
impl_integer_element!(i32, I32, I32);
impl_integer_element!(ComplexInteger<i32>, ComplexI32, ComplexI32);
impl_integer_element!(u32, U32, U32);
impl_integer_element!(ComplexInteger<u32>, ComplexU32, ComplexU32);
impl_integer_element!(i64, I64, I64);
impl_integer_element!(ComplexInteger<i64>, ComplexI64, ComplexI64);
impl_integer_element!(u64, U64, U64);
impl_integer_element!(ComplexInteger<u64>, ComplexU64, ComplexU64);

/// Dynamically tagged array storage for all implemented element classes.
///
/// This enum gives the value layer a type-erased Rust representation without
/// treating generic instantiations or their layout as a stable process ABI.
#[derive(Clone, Debug, PartialEq)]
pub enum ArrayData {
    /// Real binary32 data.
    F32(DenseArray<f32>),
    /// Complex binary32 data.
    ComplexF32(DenseArray<Complex32>),
    /// Real binary64 data.
    F64(DenseArray<f64>),
    /// Complex binary64 data.
    ComplexF64(DenseArray<Complex64>),
    /// Logical data.
    Logical(DenseArray<Logical>),
    /// Exact MATLAB `char` UTF-16 code units.
    Char(DenseArray<CharCodeUnit>),
    /// Exact real or complex fixed-width integer data.
    Integer(IntegerArrayData),
}

impl ArrayData {
    /// Borrows exact real binary32 storage.
    #[must_use]
    pub const fn as_f32(&self) -> Option<&DenseArray<f32>> {
        match self {
            Self::F32(array) => Some(array),
            _ => None,
        }
    }

    /// Mutably borrows real binary32 storage, preserving COW on element writes.
    pub const fn as_f32_mut(&mut self) -> Option<&mut DenseArray<f32>> {
        match self {
            Self::F32(array) => Some(array),
            _ => None,
        }
    }

    /// Borrows exact complex binary32 storage.
    #[must_use]
    pub const fn as_complex_f32(&self) -> Option<&DenseArray<Complex32>> {
        match self {
            Self::ComplexF32(array) => Some(array),
            _ => None,
        }
    }

    /// Mutably borrows complex binary32 storage, preserving COW on element writes.
    pub const fn as_complex_f32_mut(&mut self) -> Option<&mut DenseArray<Complex32>> {
        match self {
            Self::ComplexF32(array) => Some(array),
            _ => None,
        }
    }

    /// Constructs dynamically tagged storage from a supported dense array.
    #[must_use]
    pub fn from_typed<T: ArrayElement>(array: DenseArray<T>) -> Self {
        T::into_array(array)
    }

    /// Returns the exact typed dense array when the requested type matches.
    #[must_use]
    pub fn as_typed<T: ArrayElement>(&self) -> Option<&DenseArray<T>> {
        T::array_ref(self)
    }

    /// Returns the exact mutable typed dense array when its type matches.
    ///
    /// Mutable element access through the returned dense array preserves its
    /// checked copy-on-write detachment semantics.
    pub fn as_typed_mut<T: ArrayElement>(&mut self) -> Option<&mut DenseArray<T>> {
        T::array_mut(self)
    }

    /// Borrows the unified dynamic integer storage, if present.
    #[must_use]
    pub const fn as_integer(&self) -> Option<&IntegerArrayData> {
        match self {
            Self::Integer(integer) => Some(integer),
            _ => None,
        }
    }

    /// Mutably borrows the unified dynamic integer storage, if present.
    pub const fn as_integer_mut(&mut self) -> Option<&mut IntegerArrayData> {
        match self {
            Self::Integer(integer) => Some(integer),
            _ => None,
        }
    }

    /// Returns the explicit runtime element storage class.
    #[must_use]
    pub const fn dtype(&self) -> DType {
        match self {
            Self::F32(_) => DType::F32,
            Self::ComplexF32(_) => DType::ComplexF32,
            Self::F64(_) => DType::F64,
            Self::ComplexF64(_) => DType::ComplexF64,
            Self::Logical(_) => DType::Logical,
            Self::Char(_) => DType::Char,
            Self::Integer(integer) => integer.dtype(),
        }
    }

    /// Returns the MATLAB-compatible dynamic class name.
    #[must_use]
    pub const fn class_name(&self) -> &'static str {
        self.dtype().class_name()
    }

    /// Returns whether each stored element has real and imaginary components.
    #[must_use]
    pub const fn is_complex(&self) -> bool {
        self.dtype().is_complex()
    }

    /// Returns the array shape.
    #[must_use]
    pub fn shape(&self) -> &crate::Shape {
        match self {
            Self::F32(array) => array.shape(),
            Self::ComplexF32(array) => array.shape(),
            Self::F64(array) => array.shape(),
            Self::ComplexF64(array) => array.shape(),
            Self::Logical(array) => array.shape(),
            Self::Char(array) => array.shape(),
            Self::Integer(integer) => integer.shape(),
        }
    }

    /// Returns the number of elements.
    #[must_use]
    pub fn numel(&self) -> u64 {
        self.shape().numel()
    }

    /// Returns whether two values share the same typed element buffer.
    ///
    /// Different dynamic storage classes never share storage.
    #[must_use]
    pub fn shares_storage_with(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::F32(left), Self::F32(right)) => left.shares_storage_with(right),
            (Self::ComplexF32(left), Self::ComplexF32(right)) => left.shares_storage_with(right),
            (Self::F64(left), Self::F64(right)) => left.shares_storage_with(right),
            (Self::ComplexF64(left), Self::ComplexF64(right)) => left.shares_storage_with(right),
            (Self::Logical(left), Self::Logical(right)) => left.shares_storage_with(right),
            (Self::Char(left), Self::Char(right)) => left.shares_storage_with(right),
            (Self::Integer(left), Self::Integer(right)) => left.shares_storage_with(right),
            _ => false,
        }
    }
}

impl<T: ArrayElement> From<DenseArray<T>> for ArrayData {
    fn from(array: DenseArray<T>) -> Self {
        Self::from_typed(array)
    }
}

impl<T: IntegerElement> From<DenseArray<T>> for IntegerArrayData {
    fn from(array: DenseArray<T>) -> Self {
        Self::from_typed(array)
    }
}

#[cfg(test)]
mod abi_layout_tests {
    use std::mem::{align_of, size_of};

    use super::{CharCodeUnit, Complex32, Complex64, ComplexInteger, Logical};

    #[test]
    fn oex_element_layouts_are_fixed() {
        assert_eq!(size_of::<Logical>(), 1);
        assert_eq!(align_of::<Logical>(), 1);
        assert_eq!(size_of::<CharCodeUnit>(), 2);
        assert_eq!(align_of::<CharCodeUnit>(), 2);
        assert_eq!(size_of::<Complex32>(), 8);
        assert_eq!(align_of::<Complex32>(), 4);
        assert_eq!(size_of::<Complex64>(), 16);
        assert_eq!(align_of::<Complex64>(), 8);
        assert_eq!(size_of::<ComplexInteger<i8>>(), 2);
        assert_eq!(size_of::<ComplexInteger<u16>>(), 4);
        assert_eq!(size_of::<ComplexInteger<i32>>(), 8);
        assert_eq!(size_of::<ComplexInteger<u64>>(), 16);
    }
}
