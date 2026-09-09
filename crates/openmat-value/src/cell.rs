use openmat_array::{DenseArray, Shape};

use crate::{AggregateError, Value};

/// A canonical MATLAB-compatible cell array.
///
/// Values are stored contiguously in column-major order. Cloning a cell array
/// shares its value buffer, and the first successful replacement detaches that
/// buffer. Rust [`PartialEq`] compares representation contents for tests and
/// change detection; it is not MATLAB cell equality.
#[derive(Clone, Debug, PartialEq)]
pub struct CellArray {
    storage: DenseArray<Value>,
}

impl CellArray {
    /// Constructs a cell array from values in column-major order.
    ///
    /// Inputs are assumed to have already crossed the runtime's language-copy
    /// boundary. This value-layer constructor never accesses an object store.
    ///
    /// # Errors
    ///
    /// Returns an aggregate array error if the canonical shape does not fit a
    /// host `Vec` or if `values.len()` differs from `shape.numel()`.
    pub fn from_values(shape: Shape, values: Vec<Value>) -> Result<Self, AggregateError> {
        Ok(Self {
            storage: DenseArray::from_vec(shape, values)?,
        })
    }

    /// Constructs a cell array filled with real `0x0 double` values.
    ///
    /// # Errors
    ///
    /// Returns an aggregate array error if the canonical shape does not fit a
    /// host `Vec`.
    pub fn filled_empty_double(shape: Shape) -> Result<Self, AggregateError> {
        Ok(Self {
            storage: DenseArray::from_elem(shape, Value::empty_double())?,
        })
    }

    /// Returns the canonical shape.
    #[must_use]
    pub const fn shape(&self) -> &Shape {
        self.storage.shape()
    }

    /// Returns the number of cell elements.
    #[must_use]
    pub const fn numel(&self) -> u64 {
        self.storage.numel()
    }

    /// Returns whether this cell array has no elements.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.storage.is_empty()
    }

    /// Returns the contiguous column-major value buffer.
    #[must_use]
    pub fn values(&self) -> &[Value] {
        self.storage.as_slice()
    }

    /// Returns a value at a zero-based storage offset.
    #[must_use]
    pub fn value_at_offset(&self, offset: usize) -> Option<&Value> {
        self.storage.as_slice().get(offset)
    }

    /// Replaces a value at a zero-based storage offset.
    ///
    /// The supplied value is assumed to have already crossed the runtime's
    /// language-copy boundary. An invalid offset is rejected before shared
    /// storage is detached.
    ///
    /// # Errors
    ///
    /// Returns [`AggregateError::OffsetOutOfBounds`] when `offset` is outside
    /// the cell buffer.
    pub fn replace_at_offset(
        &mut self,
        offset: usize,
        value: Value,
    ) -> Result<Value, AggregateError> {
        if self.storage.as_slice().get(offset).is_none() {
            return Err(AggregateError::OffsetOutOfBounds {
                offset,
                numel: self.numel(),
            });
        }
        Ok(std::mem::replace(
            &mut self.storage.as_mut_slice()[offset],
            value,
        ))
    }

    /// Returns whether two cell arrays share their outer value buffer.
    #[must_use]
    pub fn shares_storage_with(&self, other: &Self) -> bool {
        self.storage.shares_storage_with(&other.storage)
    }
}

#[cfg(test)]
mod tests {
    use openmat_array::{ArrayData, ArrayError};

    use super::*;

    #[test]
    fn cell_states_preserve_shape_and_column_major_values() {
        let scalar =
            CellArray::from_values(Shape::new([1, 1]).unwrap(), vec![Value::empty_double()])
                .unwrap();
        assert_eq!(scalar.shape().dimensions(), &[1, 1]);
        assert_eq!(scalar.numel(), 1);
        assert!(
            matches!(scalar.values(), [Value::Array(ArrayData::F64(array))] if array.shape().dimensions() == [0, 0])
        );

        let empty = CellArray::from_values(Shape::new([0, 0]).unwrap(), Vec::new()).unwrap();
        let shaped_empty = CellArray::from_values(Shape::new([0, 3]).unwrap(), Vec::new()).unwrap();
        assert!(empty.is_empty());
        assert_eq!(empty.shape().dimensions(), &[0, 0]);
        assert_eq!(shaped_empty.shape().dimensions(), &[0, 3]);

        let matrix = CellArray::from_values(
            Shape::new([2, 2]).unwrap(),
            vec![
                Value::Double(1.0),
                Value::Double(3.0),
                Value::Double(2.0),
                Value::Double(4.0),
            ],
        )
        .unwrap();
        assert_eq!(
            matrix.values(),
            &[
                Value::Double(1.0),
                Value::Double(3.0),
                Value::Double(2.0),
                Value::Double(4.0)
            ]
        );
    }

    #[test]
    fn fill_uses_real_empty_double_and_constructor_is_checked() {
        let filled = CellArray::filled_empty_double(Shape::new([1, 2]).unwrap()).unwrap();
        for value in filled.values() {
            assert_eq!(value.class_name(), "double");
            assert_eq!(value.dimensions(), Some([0, 0].as_slice()));
            assert_eq!(value.numel(), Some(0));
            assert!(!matches!(value, Value::Nothing));
        }

        assert_eq!(
            CellArray::from_values(Shape::new([1, 2]).unwrap(), vec![Value::Double(1.0)]),
            Err(AggregateError::Array(ArrayError::DataLengthMismatch {
                expected: 2,
                actual: 1,
            }))
        );
    }

    #[test]
    fn cell_clone_detaches_only_after_valid_write() {
        let original = CellArray::from_values(
            Shape::new([1, 2]).unwrap(),
            vec![Value::Double(1.0), Value::Double(2.0)],
        )
        .unwrap();
        let mut assigned = original.clone();
        assert!(original.shares_storage_with(&assigned));

        assert_eq!(
            assigned.replace_at_offset(2, Value::Double(9.0)),
            Err(AggregateError::OffsetOutOfBounds {
                offset: 2,
                numel: 2,
            })
        );
        assert!(original.shares_storage_with(&assigned));

        assert_eq!(
            assigned.replace_at_offset(0, Value::Double(9.0)),
            Ok(Value::Double(1.0))
        );
        assert!(!original.shares_storage_with(&assigned));
        assert_eq!(original.value_at_offset(0), Some(&Value::Double(1.0)));
        assert_eq!(assigned.value_at_offset(0), Some(&Value::Double(9.0)));
    }

    #[test]
    fn nested_cell_write_back_preserves_both_cow_layers() {
        let nested =
            CellArray::from_values(Shape::new([1, 1]).unwrap(), vec![Value::Double(1.0)]).unwrap();
        let original =
            CellArray::from_values(Shape::new([1, 1]).unwrap(), vec![Value::Cell(nested)]).unwrap();
        let mut assigned = original.clone();

        let Value::Cell(mut changed_nested) = assigned.value_at_offset(0).unwrap().clone() else {
            panic!("test value must be a nested cell");
        };
        let Value::Cell(original_nested) = original.value_at_offset(0).unwrap() else {
            panic!("test value must be a nested cell");
        };
        assert!(original_nested.shares_storage_with(&changed_nested));

        changed_nested
            .replace_at_offset(0, Value::Double(9.0))
            .unwrap();
        assigned
            .replace_at_offset(0, Value::Cell(changed_nested))
            .unwrap();

        let Value::Cell(original_nested) = original.value_at_offset(0).unwrap() else {
            unreachable!();
        };
        let Value::Cell(changed_nested) = assigned.value_at_offset(0).unwrap() else {
            unreachable!();
        };
        assert_eq!(
            original_nested.value_at_offset(0),
            Some(&Value::Double(1.0))
        );
        assert_eq!(changed_nested.value_at_offset(0), Some(&Value::Double(9.0)));
        assert!(!original.shares_storage_with(&assigned));
        assert!(!original_nested.shares_storage_with(changed_nested));
    }
}
