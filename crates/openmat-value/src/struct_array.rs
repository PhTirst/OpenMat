use std::{borrow::Borrow, collections::BTreeMap, fmt, sync::Arc};

use openmat_array::{ArrayError, Shape};

use crate::{AggregateError, Value};

/// A validated field name in the first-tranche ASCII MATLAB identifier subset.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FieldName(Arc<str>);

impl FieldName {
    /// Validates and owns a field name.
    ///
    /// # Errors
    ///
    /// Returns [`AggregateError::InvalidFieldName`] unless the name begins
    /// with an ASCII letter and the remaining characters are ASCII letters,
    /// digits, or underscores.
    pub fn new(name: impl Into<Arc<str>>) -> Result<Self, AggregateError> {
        let name = name.into();
        let mut bytes = name.bytes();
        let valid = bytes.next().is_some_and(|byte| byte.is_ascii_alphabetic())
            && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
        if !valid {
            return Err(AggregateError::InvalidFieldName {
                name: name.to_string(),
            });
        }
        Ok(Self(name))
    }

    /// Returns the exact field-name text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for FieldName {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Borrow<str> for FieldName {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for FieldName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl TryFrom<&str> for FieldName {
    type Error = AggregateError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<String> for FieldName {
    type Error = AggregateError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

/// Ordered field metadata shared by every record in a struct array.
///
/// `field_names` is the only source of observable order. The map exists only
/// for exact-name lookup and never determines presentation or traversal order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StructSchema {
    names: Vec<FieldName>,
    lookup: BTreeMap<FieldName, usize>,
}

impl StructSchema {
    /// Builds an ordered schema.
    ///
    /// # Errors
    ///
    /// Returns [`AggregateError::DuplicateFieldName`] if a field occurs more
    /// than once.
    pub fn new(names: Vec<FieldName>) -> Result<Self, AggregateError> {
        let mut lookup = BTreeMap::new();
        for (index, name) in names.iter().cloned().enumerate() {
            if lookup.insert(name.clone(), index).is_some() {
                return Err(AggregateError::DuplicateFieldName {
                    name: name.to_string(),
                });
            }
        }
        Ok(Self { names, lookup })
    }

    /// Returns field names in observable creation order.
    #[must_use]
    pub fn field_names(&self) -> &[FieldName] {
        &self.names
    }

    /// Looks up an exact field name without defining field order.
    #[must_use]
    pub fn field_index(&self, name: &str) -> Option<usize> {
        self.lookup.get(name).copied()
    }

    /// Returns the field count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.names.len()
    }

    /// Returns whether the schema has no fields.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    fn append(&mut self, name: FieldName) {
        let index = self.names.len();
        let previous = self.lookup.insert(name.clone(), index);
        debug_assert!(previous.is_none(), "duplicate fields are validated first");
        self.names.push(name);
    }
}

/// A canonical MATLAB-compatible struct array with field-major COW columns.
///
/// All records share one ordered schema. Each field column stores one value per
/// record in column-major record order. Rust [`PartialEq`] compares
/// representation contents for tests and change detection; it is not MATLAB
/// struct equality.
#[derive(Clone, Debug, PartialEq)]
pub struct StructArray {
    shape: Shape,
    schema: Arc<StructSchema>,
    columns: Arc<Vec<Arc<Vec<Value>>>>,
}

impl StructArray {
    /// Constructs a struct array whose fields contain real `0x0 double` values.
    ///
    /// A zero-element shape retains the supplied schema and empty field
    /// columns. Inputs are ordered field metadata, not a lookup-sorted set.
    ///
    /// # Errors
    ///
    /// Returns an error if the shape does not fit a host `Vec` or a field name
    /// occurs more than once.
    pub fn empty(shape: Shape, fields: Vec<FieldName>) -> Result<Self, AggregateError> {
        let length = host_length(&shape)?;
        let schema = StructSchema::new(fields)?;
        let empty_double = Value::empty_double();
        let columns = (0..schema.len())
            .map(|_| Arc::new(vec![empty_double.clone(); length]))
            .collect();
        Ok(Self {
            shape,
            schema: Arc::new(schema),
            columns: Arc::new(columns),
        })
    }

    /// Constructs a struct array from field-major, column-major record values.
    ///
    /// Values are assumed to have already crossed the runtime's language-copy
    /// boundary. This value-layer constructor never accesses an object store.
    ///
    /// # Errors
    ///
    /// Returns an error for duplicate fields, a shape that does not fit a host
    /// `Vec`, a column count different from the field count, or a field column
    /// whose length differs from `shape.numel()`.
    pub fn from_columns(
        shape: Shape,
        fields: Vec<FieldName>,
        columns: Vec<Vec<Value>>,
    ) -> Result<Self, AggregateError> {
        let expected = host_length(&shape)?;
        let schema = StructSchema::new(fields)?;
        if columns.len() != schema.len() {
            return Err(AggregateError::FieldColumnCountMismatch {
                fields: schema.len(),
                columns: columns.len(),
            });
        }
        for (field, column) in columns.iter().enumerate() {
            if column.len() != expected {
                return Err(AggregateError::FieldColumnLengthMismatch {
                    field,
                    expected: shape.numel(),
                    actual: column.len(),
                });
            }
        }
        Ok(Self {
            shape,
            schema: Arc::new(schema),
            columns: Arc::new(columns.into_iter().map(Arc::new).collect()),
        })
    }

    /// Returns the canonical shape.
    #[must_use]
    pub const fn shape(&self) -> &Shape {
        &self.shape
    }

    /// Returns the number of records.
    #[must_use]
    pub const fn numel(&self) -> u64 {
        self.shape.numel()
    }

    /// Returns whether this struct array has no records.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.shape.is_empty()
    }

    /// Returns shared ordered schema metadata.
    #[must_use]
    pub fn schema(&self) -> &StructSchema {
        &self.schema
    }

    /// Returns field names in observable creation order.
    #[must_use]
    pub fn field_names(&self) -> &[FieldName] {
        self.schema.field_names()
    }

    /// Returns the field count.
    #[must_use]
    pub fn field_count(&self) -> usize {
        self.schema.len()
    }

    /// Removes a field from every record, preserving all other field storage.
    ///
    /// # Errors
    ///
    /// Returns an error without modifying the array for an invalid index.
    pub fn remove_field(&mut self, field: usize) -> Result<(), AggregateError> {
        if field >= self.field_count() {
            return Err(AggregateError::FieldOutOfBounds {
                field,
                fields: self.field_count(),
            });
        }
        let mut names = self.field_names().to_vec();
        names.remove(field);
        let schema = StructSchema::new(names)?;
        Arc::make_mut(&mut self.columns).remove(field);
        self.schema = Arc::new(schema);
        Ok(())
    }

    /// Replaces the ordered schema atomically without copying field values.
    ///
    /// # Errors
    ///
    /// Returns an error for a wrong count or duplicate field name.
    pub fn rename_fields(&mut self, names: Vec<FieldName>) -> Result<(), AggregateError> {
        if names.len() != self.field_count() {
            return Err(AggregateError::FieldColumnCountMismatch {
                fields: names.len(),
                columns: self.field_count(),
            });
        }
        self.schema = Arc::new(StructSchema::new(names)?);
        Ok(())
    }

    /// Looks up an exact field name without defining field order.
    #[must_use]
    pub fn field_index(&self, name: &str) -> Option<usize> {
        self.schema.field_index(name)
    }

    /// Returns a field column in column-major record order.
    #[must_use]
    pub fn field_values(&self, field: usize) -> Option<&[Value]> {
        self.columns.get(field).map(|column| column.as_slice())
    }

    /// Returns a value by zero-based field and record offsets.
    #[must_use]
    pub fn value_at(&self, field: usize, offset: usize) -> Option<&Value> {
        self.columns.get(field)?.get(offset)
    }

    /// Returns a value by exact field name and zero-based record offset.
    #[must_use]
    pub fn value_at_name(&self, name: &str, offset: usize) -> Option<&Value> {
        self.value_at(self.field_index(name)?, offset)
    }

    /// Replaces a value by zero-based field and record offsets.
    ///
    /// The supplied value is assumed to have already crossed the runtime's
    /// language-copy boundary. Both offsets are validated before any shared
    /// metadata or field storage is detached.
    ///
    /// # Errors
    ///
    /// Returns a field or record bounds error for an invalid offset.
    pub fn replace_at(
        &mut self,
        field: usize,
        offset: usize,
        value: Value,
    ) -> Result<Value, AggregateError> {
        let Some(column) = self.columns.get(field) else {
            return Err(AggregateError::FieldOutOfBounds {
                field,
                fields: self.field_count(),
            });
        };
        if column.get(offset).is_none() {
            return Err(AggregateError::OffsetOutOfBounds {
                offset,
                numel: self.numel(),
            });
        }

        let columns = Arc::make_mut(&mut self.columns);
        let column = Arc::make_mut(&mut columns[field]);
        Ok(std::mem::replace(&mut column[offset], value))
    }

    /// Appends a field column without changing existing field order.
    ///
    /// Values are assumed to have already crossed the runtime's language-copy
    /// boundary. The name and complete column are validated before schema or
    /// storage is detached.
    ///
    /// # Errors
    ///
    /// Returns an error if the field already exists or the column length does
    /// not equal the record count.
    pub fn append_field(
        &mut self,
        name: FieldName,
        default_values: Vec<Value>,
    ) -> Result<(), AggregateError> {
        if self.field_index(name.as_str()).is_some() {
            return Err(AggregateError::DuplicateFieldName {
                name: name.to_string(),
            });
        }
        let expected = host_length(&self.shape)?;
        if default_values.len() != expected {
            return Err(AggregateError::FieldColumnLengthMismatch {
                field: self.field_count(),
                expected: self.numel(),
                actual: default_values.len(),
            });
        }

        Arc::make_mut(&mut self.schema).append(name);
        Arc::make_mut(&mut self.columns).push(Arc::new(default_values));
        Ok(())
    }

    /// Appends a field filled with real `0x0 double` values.
    ///
    /// # Errors
    ///
    /// Returns an error if the field already exists or the shape does not fit
    /// a host `Vec`.
    pub fn append_empty_field(&mut self, name: FieldName) -> Result<(), AggregateError> {
        if self.field_index(name.as_str()).is_some() {
            return Err(AggregateError::DuplicateFieldName {
                name: name.to_string(),
            });
        }
        let length = host_length(&self.shape)?;
        self.append_field(name, vec![Value::empty_double(); length])
    }

    /// Returns whether two structs share their ordered schema allocation.
    #[must_use]
    pub fn shares_schema_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.schema, &other.schema)
    }

    /// Returns whether two structs share their outer field-column table.
    #[must_use]
    pub fn shares_storage_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.columns, &other.columns)
    }

    /// Returns whether two structs share a particular field column allocation.
    #[must_use]
    pub fn shares_field_storage_with(&self, other: &Self, field: usize) -> bool {
        self.columns
            .get(field)
            .zip(other.columns.get(field))
            .is_some_and(|(left, right)| Arc::ptr_eq(left, right))
    }
}

fn host_length(shape: &Shape) -> Result<usize, AggregateError> {
    usize::try_from(shape.numel()).map_err(|_| {
        AggregateError::Array(ArrayError::HostLengthOverflow {
            numel: shape.numel(),
        })
    })
}

#[cfg(test)]
mod tests {
    use openmat_array::ArrayData;

    use super::*;

    fn fields(names: &[&str]) -> Vec<FieldName> {
        names
            .iter()
            .map(|name| FieldName::new(*name).unwrap())
            .collect()
    }

    #[test]
    fn field_names_validate_ascii_identifiers_and_duplicates() {
        for valid in ["a", "Alpha_2", "x0"] {
            assert_eq!(FieldName::new(valid).unwrap().as_str(), valid);
        }
        for invalid in ["", "0x", "has space", "naïve", "_x"] {
            assert!(matches!(
                FieldName::new(invalid),
                Err(AggregateError::InvalidFieldName { .. })
            ));
        }
        assert_eq!(
            StructSchema::new(fields(&["b", "a", "b"])),
            Err(AggregateError::DuplicateFieldName {
                name: String::from("b")
            })
        );
    }

    #[test]
    fn struct_states_preserve_shape_schema_order_and_field_major_columns() {
        let scalar = StructArray::empty(Shape::new([1, 1]).unwrap(), Vec::new()).unwrap();
        assert_eq!(scalar.shape().dimensions(), &[1, 1]);
        assert_eq!(scalar.numel(), 1);
        assert!(scalar.schema().is_empty());

        let empty = StructArray::empty(Shape::new([0, 0]).unwrap(), Vec::new()).unwrap();
        assert_eq!(empty.shape().dimensions(), &[0, 0]);
        assert!(empty.is_empty());

        let shaped_empty =
            StructArray::empty(Shape::new([0, 3]).unwrap(), fields(&["z", "a"])).unwrap();
        assert_eq!(shaped_empty.shape().dimensions(), &[0, 3]);
        assert_eq!(shaped_empty.field_names(), fields(&["z", "a"]));
        assert_eq!(shaped_empty.field_values(0), Some([].as_slice()));
        assert_eq!(shaped_empty.field_values(1), Some([].as_slice()));

        let matrix = StructArray::from_columns(
            Shape::new([2, 2]).unwrap(),
            fields(&["z", "a"]),
            vec![
                vec![
                    Value::Double(1.0),
                    Value::Double(3.0),
                    Value::Double(2.0),
                    Value::Double(4.0),
                ],
                vec![
                    Value::Double(10.0),
                    Value::Double(30.0),
                    Value::Double(20.0),
                    Value::Double(40.0),
                ],
            ],
        )
        .unwrap();
        assert_eq!(matrix.field_names(), fields(&["z", "a"]));
        assert_eq!(matrix.field_index("z"), Some(0));
        assert_eq!(matrix.field_index("a"), Some(1));
        assert_eq!(matrix.value_at_name("z", 1), Some(&Value::Double(3.0)));
        assert_eq!(matrix.value_at_name("a", 2), Some(&Value::Double(20.0)));
    }

    #[test]
    fn schema_edits_preserve_values_and_fail_atomically() {
        let original = StructArray::from_columns(
            Shape::new([2, 1]).unwrap(),
            fields(&["a", "b"]),
            vec![
                vec![Value::Double(1.), Value::Double(2.)],
                vec![Value::Double(3.), Value::Double(4.)],
            ],
        )
        .unwrap();
        let mut changed = original.clone();
        assert!(changed.rename_fields(fields(&["x", "x"])).is_err());
        assert!(original.shares_schema_with(&changed));
        assert!(original.shares_storage_with(&changed));
        changed.rename_fields(fields(&["b", "a"])).unwrap();
        assert!(original.shares_field_storage_with(&changed, 0));
        assert_eq!(changed.value_at_name("b", 1), Some(&Value::Double(2.)));
        let before = changed.clone();
        assert!(changed.remove_field(9).is_err());
        assert!(before.shares_schema_with(&changed));
        changed.remove_field(0).unwrap();
        assert_eq!(changed.field_count(), 1);
        assert_eq!(changed.value_at_name("a", 1), Some(&Value::Double(4.)));
        assert_eq!(original.field_count(), 2);
        assert_eq!(original.value_at_name("a", 1), Some(&Value::Double(2.)));
    }

    #[test]
    fn struct_construction_checks_column_counts_and_lengths() {
        assert_eq!(
            StructArray::from_columns(Shape::new([1, 1]).unwrap(), fields(&["a"]), Vec::new()),
            Err(AggregateError::FieldColumnCountMismatch {
                fields: 1,
                columns: 0,
            })
        );
        assert_eq!(
            StructArray::from_columns(
                Shape::new([1, 2]).unwrap(),
                fields(&["a"]),
                vec![vec![Value::Double(1.0)]]
            ),
            Err(AggregateError::FieldColumnLengthMismatch {
                field: 0,
                expected: 2,
                actual: 1,
            })
        );
    }

    #[test]
    fn empty_and_appended_fields_use_real_empty_double_values() {
        let mut value =
            StructArray::empty(Shape::new([1, 2]).unwrap(), fields(&["first"])).unwrap();
        value
            .append_empty_field(FieldName::new("second").unwrap())
            .unwrap();
        assert_eq!(value.field_names(), fields(&["first", "second"]));
        for field in 0..2 {
            for item in value.field_values(field).unwrap() {
                assert_eq!(item.class_name(), "double");
                assert_eq!(item.dimensions(), Some([0, 0].as_slice()));
                assert_eq!(item.numel(), Some(0));
                assert!(matches!(item, Value::Array(ArrayData::F64(_))));
            }
        }
    }

    #[test]
    fn successful_struct_write_detaches_only_target_field_column() {
        let original = StructArray::from_columns(
            Shape::new([1, 2]).unwrap(),
            fields(&["a", "b"]),
            vec![
                vec![Value::Double(1.0), Value::Double(2.0)],
                vec![Value::Double(3.0), Value::Double(4.0)],
            ],
        )
        .unwrap();
        let mut assigned = original.clone();
        assert!(original.shares_schema_with(&assigned));
        assert!(original.shares_storage_with(&assigned));
        assert!(original.shares_field_storage_with(&assigned, 0));
        assert!(original.shares_field_storage_with(&assigned, 1));

        assert_eq!(
            assigned.replace_at(0, 1, Value::Double(9.0)),
            Ok(Value::Double(2.0))
        );
        assert!(original.shares_schema_with(&assigned));
        assert!(!original.shares_storage_with(&assigned));
        assert!(!original.shares_field_storage_with(&assigned, 0));
        assert!(original.shares_field_storage_with(&assigned, 1));
        assert_eq!(original.value_at(0, 1), Some(&Value::Double(2.0)));
        assert_eq!(assigned.value_at(0, 1), Some(&Value::Double(9.0)));
    }

    #[test]
    fn failed_struct_mutations_do_not_detach_metadata_or_columns() {
        let original = StructArray::from_columns(
            Shape::new([1, 1]).unwrap(),
            fields(&["a"]),
            vec![vec![Value::Double(1.0)]],
        )
        .unwrap();
        let mut assigned = original.clone();

        assert!(matches!(
            assigned.replace_at(1, 0, Value::Double(2.0)),
            Err(AggregateError::FieldOutOfBounds { .. })
        ));
        assert!(matches!(
            assigned.replace_at(0, 1, Value::Double(2.0)),
            Err(AggregateError::OffsetOutOfBounds { .. })
        ));
        assert!(matches!(
            assigned.append_field(FieldName::new("a").unwrap(), vec![Value::Double(2.0)]),
            Err(AggregateError::DuplicateFieldName { .. })
        ));
        assert!(matches!(
            assigned.append_field(FieldName::new("b").unwrap(), Vec::new()),
            Err(AggregateError::FieldColumnLengthMismatch { .. })
        ));
        assert!(original.shares_schema_with(&assigned));
        assert!(original.shares_storage_with(&assigned));
        assert!(original.shares_field_storage_with(&assigned, 0));
    }

    #[test]
    fn append_detaches_schema_but_keeps_existing_field_columns_shared() {
        let original = StructArray::from_columns(
            Shape::new([1, 1]).unwrap(),
            fields(&["a"]),
            vec![vec![Value::Double(1.0)]],
        )
        .unwrap();
        let mut assigned = original.clone();
        assigned
            .append_field(FieldName::new("b").unwrap(), vec![Value::Double(2.0)])
            .unwrap();

        assert!(!original.shares_schema_with(&assigned));
        assert!(!original.shares_storage_with(&assigned));
        assert!(original.shares_field_storage_with(&assigned, 0));
        assert_eq!(assigned.field_names(), fields(&["a", "b"]));
    }

    #[test]
    fn nested_aggregate_write_back_preserves_inner_and_outer_cow() {
        let nested =
            crate::CellArray::from_values(Shape::new([1, 1]).unwrap(), vec![Value::Double(1.0)])
                .unwrap();
        let original = StructArray::from_columns(
            Shape::new([1, 1]).unwrap(),
            fields(&["payload"]),
            vec![vec![Value::Cell(nested)]],
        )
        .unwrap();
        let mut assigned = original.clone();

        let Value::Cell(mut changed_nested) = assigned.value_at(0, 0).unwrap().clone() else {
            panic!("test value must be a nested cell");
        };
        changed_nested
            .replace_at_offset(0, Value::Double(9.0))
            .unwrap();
        assigned
            .replace_at(0, 0, Value::Cell(changed_nested))
            .unwrap();

        let Value::Cell(original_nested) = original.value_at(0, 0).unwrap() else {
            unreachable!();
        };
        let Value::Cell(changed_nested) = assigned.value_at(0, 0).unwrap() else {
            unreachable!();
        };
        assert_eq!(
            original_nested.value_at_offset(0),
            Some(&Value::Double(1.0))
        );
        assert_eq!(changed_nested.value_at_offset(0), Some(&Value::Double(9.0)));
        assert!(!original.shares_field_storage_with(&assigned, 0));
        assert!(!original_nested.shares_storage_with(changed_nested));
    }
}
