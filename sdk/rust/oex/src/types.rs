use crate::{Error, ErrorKind, Result, sys};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ValueKind {
    Nothing,
    Dense,
    Sparse,
    String,
    Cell,
    Struct,
    Table,
    Object,
    Function,
    Graphics,
    Unknown(u32),
}

impl ValueKind {
    pub const fn from_raw(raw: u32) -> Self {
        match raw {
            0 => Self::Nothing,
            1 => Self::Dense,
            2 => Self::Sparse,
            3 => Self::String,
            4 => Self::Cell,
            5 => Self::Struct,
            6 => Self::Table,
            7 => Self::Object,
            8 => Self::Function,
            9 => Self::Graphics,
            raw => Self::Unknown(raw),
        }
    }
}

macro_rules! data_types {
    ($($name:ident = $raw:ident),* $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        #[non_exhaustive]
        pub enum DataType { $($name,)* Unknown(u32) }
        impl DataType {
            pub const fn from_raw(raw: u32) -> Self {
                match raw { $(sys::$raw => Self::$name,)* raw => Self::Unknown(raw) }
            }
            pub const fn as_raw(self) -> u32 {
                match self { $(Self::$name => sys::$raw,)* Self::Unknown(raw) => raw }
            }
        }
    };
}
data_types! {
    None = OEX_DATA_NONE, Logical = OEX_DATA_LOGICAL, Char16 = OEX_DATA_CHAR16,
    I8 = OEX_DATA_I8, U8 = OEX_DATA_U8, I16 = OEX_DATA_I16, U16 = OEX_DATA_U16,
    I32 = OEX_DATA_I32, U32 = OEX_DATA_U32, I64 = OEX_DATA_I64, U64 = OEX_DATA_U64,
    F32 = OEX_DATA_F32, F64 = OEX_DATA_F64,
    ComplexF32 = OEX_DATA_COMPLEX_F32, ComplexF64 = OEX_DATA_COMPLEX_F64,
    ComplexI8 = OEX_DATA_COMPLEX_I8, ComplexU8 = OEX_DATA_COMPLEX_U8,
    ComplexI16 = OEX_DATA_COMPLEX_I16, ComplexU16 = OEX_DATA_COMPLEX_U16,
    ComplexI32 = OEX_DATA_COMPLEX_I32, ComplexU32 = OEX_DATA_COMPLEX_U32,
    ComplexI64 = OEX_DATA_COMPLEX_I64, ComplexU64 = OEX_DATA_COMPLEX_U64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ValueInfo {
    pub kind: ValueKind,
    pub data_type: DataType,
    pub rank: u32,
    pub element_size: u32,
    pub element_count: u64,
    pub is_complex: bool,
}

/// OEX logical storage. Safe construction restricts the underlying byte to 0/1.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Logical(u8);
impl Logical {
    pub const FALSE: Self = Self(0);
    pub const TRUE: Self = Self(1);
    pub const fn get(self) -> bool {
        self.0 != 0
    }
}
impl From<bool> for Logical {
    fn from(value: bool) -> Self {
        Self(u8::from(value))
    }
}
impl From<Logical> for bool {
    fn from(value: Logical) -> bool {
        value.get()
    }
}

/// An exact UTF-16 code unit; lone surrogates are preserved.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Char16(pub u16);

/// Interleaved OEX complex storage (real component followed by imaginary).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Complex<T> {
    pub re: T,
    pub im: T,
}

mod sealed {
    pub trait Sealed {}
}

/// Types whose layout and valid values exactly match an OEX element type.
/// Sealed: arbitrary user types cannot be reinterpreted as host storage.
pub trait Element: sealed::Sealed + Copy + Default + Send + Sync + 'static {
    const DATA_TYPE: DataType;
}
macro_rules! elements {
    ($($ty:ty => $kind:ident),* $(,)?) => { $(
        impl sealed::Sealed for $ty {}
        impl Element for $ty { const DATA_TYPE: DataType = DataType::$kind; }
    )* };
}
elements! {
    Logical => Logical, Char16 => Char16,
    i8 => I8, u8 => U8, i16 => I16, u16 => U16, i32 => I32, u32 => U32,
    i64 => I64, u64 => U64, f32 => F32, f64 => F64,
    Complex<f32> => ComplexF32, Complex<f64> => ComplexF64,
    Complex<i8> => ComplexI8, Complex<u8> => ComplexU8,
    Complex<i16> => ComplexI16, Complex<u16> => ComplexU16,
    Complex<i32> => ComplexI32, Complex<u32> => ComplexU32,
    Complex<i64> => ComplexI64, Complex<u64> => ComplexU64,
}

pub(crate) fn check_element<T: Element>(data_type: u32, element_size: u32) -> Result<()> {
    if data_type != T::DATA_TYPE.as_raw() {
        return Err(Error::with_kind(
            ErrorKind::Type,
            "array element type mismatch",
        ));
    }
    if element_size as usize != size_of::<T>() {
        return Err(Error::with_kind(
            ErrorKind::Abi,
            "array element layout mismatch",
        ));
    }
    Ok(())
}
