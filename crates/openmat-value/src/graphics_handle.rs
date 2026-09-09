use openmat_array::{DenseArray, Shape};
use openmat_graphics_model::{GraphicsClass, GraphicsHandle};

/// A homogeneous `N x 1` column of identity-preserving graphics handles.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphicsHandleArray {
    class: GraphicsClass,
    storage: DenseArray<GraphicsHandle>,
}

impl GraphicsHandleArray {
    /// Creates a homogeneous graphics handle column.
    ///
    /// Empty rows retain the supplied dynamic class. Nonempty rows reject a
    /// handle carrying any other graphics class.
    ///
    /// # Errors
    ///
    /// Returns a checked shape/storage error or a mixed-class error.
    pub fn column(
        class: GraphicsClass,
        handles: Vec<GraphicsHandle>,
    ) -> Result<Self, GraphicsHandleArrayError> {
        if let Some(handle) = handles.iter().find(|handle| handle.class() != class) {
            return Err(GraphicsHandleArrayError::MixedClass {
                expected: class,
                actual: handle.class(),
            });
        }
        let length =
            u64::try_from(handles.len()).map_err(|_| GraphicsHandleArrayError::LengthOverflow)?;
        let shape = Shape::new([length, 1]).map_err(GraphicsHandleArrayError::Array)?;
        let storage =
            DenseArray::from_vec(shape, handles).map_err(GraphicsHandleArrayError::Array)?;
        Ok(Self { class, storage })
    }

    /// Returns the dynamic graphics class shared by every element.
    #[must_use]
    pub const fn class(&self) -> GraphicsClass {
        self.class
    }

    /// Returns the canonical `N x 1` shape.
    #[must_use]
    pub const fn shape(&self) -> &Shape {
        self.storage.shape()
    }

    /// Returns the element count.
    #[must_use]
    pub const fn numel(&self) -> u64 {
        self.storage.numel()
    }

    /// Borrows handles in row/column-major storage order.
    #[must_use]
    pub fn as_slice(&self) -> &[GraphicsHandle] {
        self.storage.as_slice()
    }

    /// Returns whether two arrays share their outer COW handle storage.
    #[must_use]
    pub fn shares_storage_with(&self, other: &Self) -> bool {
        self.storage.shares_storage_with(&other.storage)
    }
}

/// Graphics handle column construction failures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GraphicsHandleArrayError {
    /// Checked array construction failed.
    Array(openmat_array::ArrayError),
    /// Host length cannot be represented in the language shape.
    LengthOverflow,
    /// At least one element has another graphics class.
    MixedClass {
        /// Required class.
        expected: GraphicsClass,
        /// Actual class.
        actual: GraphicsClass,
    },
}

impl std::fmt::Display for GraphicsHandleArrayError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Array(error) => error.fmt(formatter),
            Self::LengthOverflow => formatter.write_str("graphics handle column length overflowed"),
            Self::MixedClass { expected, actual } => write!(
                formatter,
                "graphics handle column requires {}, found {}",
                expected.class_name(),
                actual.class_name()
            ),
        }
    }
}

impl std::error::Error for GraphicsHandleArrayError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Array(error) => Some(error),
            Self::LengthOverflow | Self::MixedClass { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handle(slot: u32, class: GraphicsClass) -> GraphicsHandle {
        GraphicsHandle::new(slot, 1, class).unwrap()
    }

    #[test]
    fn column_is_canonical_and_identity_preserving() {
        let original = GraphicsHandleArray::column(
            GraphicsClass::LineSeries,
            vec![
                handle(1, GraphicsClass::LineSeries),
                handle(2, GraphicsClass::LineSeries),
            ],
        )
        .unwrap();
        let copied = original.clone();
        assert_eq!(original.shape().dimensions(), &[2, 1]);
        assert_eq!(original.as_slice(), copied.as_slice());
        assert!(original.shares_storage_with(&copied));
    }

    #[test]
    fn empty_column_keeps_dynamic_class_and_mixed_columns_fail() {
        let empty = GraphicsHandleArray::column(GraphicsClass::LineSeries, Vec::new()).unwrap();
        assert_eq!(empty.shape().dimensions(), &[0, 1]);
        assert_eq!(empty.class(), GraphicsClass::LineSeries);
        assert!(matches!(
            GraphicsHandleArray::column(
                GraphicsClass::LineSeries,
                vec![handle(1, GraphicsClass::ScatterSeries)]
            ),
            Err(GraphicsHandleArrayError::MixedClass { .. })
        ));
    }
}
