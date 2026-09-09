/// Release-one arithmetic and comparison operator dispatch names.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operator {
    UnaryPlus,
    UnaryMinus,
    Add,
    Subtract,
    ElementWiseMultiply,
    MatrixMultiply,
    ElementWiseRightDivide,
    MatrixRightDivide,
    ElementWiseLeftDivide,
    MatrixLeftDivide,
    ElementWisePower,
    MatrixPower,
    Equal,
    NotEqual,
    LessThan,
    LessThanOrEqual,
    GreaterThan,
    GreaterThanOrEqual,
}

impl Operator {
    /// Returns the conventional `classdef` method selected for this operator.
    #[must_use]
    pub const fn method_name(self) -> &'static str {
        match self {
            Self::UnaryPlus => "uplus",
            Self::UnaryMinus => "uminus",
            Self::Add => "plus",
            Self::Subtract => "minus",
            Self::ElementWiseMultiply => "times",
            Self::MatrixMultiply => "mtimes",
            Self::ElementWiseRightDivide => "rdivide",
            Self::MatrixRightDivide => "mrdivide",
            Self::ElementWiseLeftDivide => "ldivide",
            Self::MatrixLeftDivide => "mldivide",
            Self::ElementWisePower => "power",
            Self::MatrixPower => "mpower",
            Self::Equal => "eq",
            Self::NotEqual => "ne",
            Self::LessThan => "lt",
            Self::LessThanOrEqual => "le",
            Self::GreaterThan => "gt",
            Self::GreaterThanOrEqual => "ge",
        }
    }
}
