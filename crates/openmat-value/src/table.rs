use std::{
    borrow::Borrow,
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
    sync::Arc,
};

use openmat_array::{ArrayError, Shape};

use crate::Value;

/// A checked table construction or schema mutation failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TableError {
    /// A table variable name is empty or contains an embedded NUL.
    InvalidVariableName { name: String },
    /// A table schema contains the same variable name more than once.
    DuplicateVariableName { name: String },
    /// The variable name and value counts differ.
    VariableCountMismatch { names: usize, variables: usize },
    /// A variable has no language-visible array shape.
    UnsupportedVariable { variable: usize, class: String },
    /// A variable's first dimension differs from the table height.
    RowCountMismatch {
        variable: usize,
        expected: u64,
        actual: u64,
    },
    /// A zero-based variable position is outside the schema.
    VariableOutOfBounds { variable: usize, variables: usize },
    /// A row name is empty or contains an embedded NUL.
    InvalidRowName { name: String },
    /// A table contains the same row name more than once.
    DuplicateRowName { name: String },
    /// The number of row names differs from the table height.
    RowNameCountMismatch { names: usize, rows: u64 },
    /// The table's public two-dimensional shape is not representable.
    Array(ArrayError),
}

impl fmt::Display for TableError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidVariableName { name } => {
                write!(formatter, "invalid table variable name {name:?}")
            }
            Self::DuplicateVariableName { name } => {
                write!(formatter, "duplicate table variable name {name:?}")
            }
            Self::VariableCountMismatch { names, variables } => write!(
                formatter,
                "table schema has {names} names but {variables} variables were supplied"
            ),
            Self::UnsupportedVariable { variable, class } => write!(
                formatter,
                "table variable {variable} has unsupported class {class}"
            ),
            Self::RowCountMismatch {
                variable,
                expected,
                actual,
            } => write!(
                formatter,
                "table variable {variable} has {actual} rows but {expected} rows are required"
            ),
            Self::VariableOutOfBounds {
                variable,
                variables,
            } => write!(
                formatter,
                "table variable {variable} is outside a schema with {variables} variables"
            ),
            Self::InvalidRowName { name } => write!(formatter, "invalid table row name {name:?}"),
            Self::DuplicateRowName { name } => {
                write!(formatter, "duplicate table row name {name:?}")
            }
            Self::RowNameCountMismatch { names, rows } => {
                write!(
                    formatter,
                    "table has {rows} rows but {names} row names were supplied"
                )
            }
            Self::Array(error) => error.fmt(formatter),
        }
    }
}

impl Error for TableError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Array(error) => Some(error),
            _ => None,
        }
    }
}

impl From<ArrayError> for TableError {
    fn from(value: ArrayError) -> Self {
        Self::Array(value)
    }
}

/// An ordered MATLAB table variable name.
///
/// Table variable names are intentionally not restricted to source-language
/// identifiers. Dynamic-name indexing can address names containing spaces or
/// Unicode even though static `T.name` syntax cannot spell every such name.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TableVariableName(String);

impl TableVariableName {
    /// Validates and owns a table variable name.
    ///
    /// # Errors
    ///
    /// Returns [`TableError::InvalidVariableName`] for an empty name or one
    /// containing an embedded NUL.
    pub fn new(name: impl Into<String>) -> Result<Self, TableError> {
        let name = name.into();
        if name.is_empty() || name.contains('\0') {
            return Err(TableError::InvalidVariableName { name });
        }
        Ok(Self(name))
    }

    /// Borrows the exact public name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TableVariableName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Borrow<str> for TableVariableName {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

/// One validated MATLAB table row name.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TableRowName(String);

impl TableRowName {
    /// Validates and owns a table row name.
    ///
    /// # Errors
    ///
    /// Returns [`TableError::InvalidRowName`] for an empty name or embedded NUL.
    pub fn new(name: impl Into<String>) -> Result<Self, TableError> {
        let name = name.into();
        if name.is_empty() || name.contains('\0') {
            return Err(TableError::InvalidRowName { name });
        }
        Ok(Self(name))
    }

    /// Borrows the exact public row name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TableRowName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Shared ordered table schema.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableSchema {
    names: Vec<TableVariableName>,
    lookup: BTreeMap<TableVariableName, usize>,
}

impl TableSchema {
    fn new(names: Vec<TableVariableName>) -> Result<Self, TableError> {
        let mut lookup = BTreeMap::new();
        for (index, name) in names.iter().cloned().enumerate() {
            if lookup.insert(name.clone(), index).is_some() {
                return Err(TableError::DuplicateVariableName {
                    name: name.to_string(),
                });
            }
        }
        Ok(Self { names, lookup })
    }

    /// Returns variable names in observable creation order.
    #[must_use]
    pub fn variable_names(&self) -> &[TableVariableName] {
        &self.names
    }

    /// Looks up an exact variable name.
    #[must_use]
    pub fn variable_index(&self, name: &str) -> Option<usize> {
        self.lookup.get(name).copied()
    }

    /// Returns the variable count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.names.len()
    }

    /// Returns whether this schema has no variables.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    fn append(&mut self, name: TableVariableName) {
        let index = self.names.len();
        let previous = self.lookup.insert(name.clone(), index);
        debug_assert!(previous.is_none(), "duplicate names are validated first");
        self.names.push(name);
    }

    fn remove(&mut self, index: usize) -> TableVariableName {
        let name = self.names.remove(index);
        self.lookup.clear();
        for (index, name) in self.names.iter().cloned().enumerate() {
            self.lookup.insert(name, index);
        }
        name
    }
}

/// A heterogeneous column-oriented MATLAB table with copy-on-write variables.
#[derive(Clone, Debug, PartialEq)]
pub struct TableArray {
    shape: Shape,
    schema: Arc<TableSchema>,
    variables: Arc<Vec<Value>>,
    row_names: Option<Arc<Vec<TableRowName>>>,
}

impl TableArray {
    /// Constructs a zero-variable table with an explicit height.
    ///
    /// # Errors
    ///
    /// Returns an error if the public table shape cannot be represented.
    pub fn empty(row_count: u64) -> Result<Self, TableError> {
        Self::from_parts(row_count, Vec::new(), Vec::new())
    }

    /// Constructs a table and infers its height from the first variable.
    ///
    /// Every variable retains its complete value and must have the same first
    /// dimension. A variable may itself have multiple columns or trailing
    /// dimensions; MATLAB table width counts variables, not their storage
    /// columns.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid or duplicate names, unsupported variables,
    /// inconsistent row counts, or an unrepresentable table shape.
    pub fn from_variables(
        names: Vec<TableVariableName>,
        variables: Vec<Value>,
    ) -> Result<Self, TableError> {
        let row_count = variables
            .first()
            .and_then(Value::dimensions)
            .and_then(|dimensions| dimensions.first().copied())
            .unwrap_or(0);
        Self::from_parts(row_count, names, variables)
    }

    /// Constructs a table with an explicit height.
    ///
    /// # Errors
    ///
    /// Returns an error for name/value count mismatch, invalid or duplicate
    /// names, unsupported variables, inconsistent rows, or an invalid shape.
    pub fn from_parts(
        row_count: u64,
        names: Vec<TableVariableName>,
        variables: Vec<Value>,
    ) -> Result<Self, TableError> {
        Self::from_parts_with_row_names(row_count, names, variables, None)
    }

    /// Constructs a table with an explicit height and optional row names.
    ///
    /// # Errors
    ///
    /// Returns the ordinary table construction errors, plus invalid, duplicate,
    /// or incorrectly sized row-name errors.
    pub fn from_parts_with_row_names(
        row_count: u64,
        names: Vec<TableVariableName>,
        variables: Vec<Value>,
        row_names: Option<Vec<TableRowName>>,
    ) -> Result<Self, TableError> {
        if names.len() != variables.len() {
            return Err(TableError::VariableCountMismatch {
                names: names.len(),
                variables: variables.len(),
            });
        }
        let schema = TableSchema::new(names)?;
        for (index, variable) in variables.iter().enumerate() {
            let Some(actual) = variable
                .dimensions()
                .and_then(|dimensions| dimensions.first().copied())
            else {
                return Err(TableError::UnsupportedVariable {
                    variable: index,
                    class: variable.class_name().to_owned(),
                });
            };
            if actual != row_count {
                return Err(TableError::RowCountMismatch {
                    variable: index,
                    expected: row_count,
                    actual,
                });
            }
        }
        let width = u64::try_from(schema.len())
            .map_err(|_| TableError::Array(ArrayError::HostLengthOverflow { numel: u64::MAX }))?;
        let shape = Shape::new([row_count, width])?;
        let row_names = row_names
            .map(|names| validate_row_names(row_count, names).map(Arc::new))
            .transpose()?;
        Ok(Self {
            shape,
            schema: Arc::new(schema),
            variables: Arc::new(variables),
            row_names,
        })
    }

    /// Returns the public `height-by-width` shape.
    #[must_use]
    pub const fn shape(&self) -> &Shape {
        &self.shape
    }

    /// Returns the table height.
    #[must_use]
    pub fn row_count(&self) -> u64 {
        self.shape.dimensions()[0]
    }

    /// Returns the number of table variables.
    #[must_use]
    pub fn variable_count(&self) -> usize {
        self.schema.len()
    }

    /// Returns the public table element count (`height * width`).
    #[must_use]
    pub const fn numel(&self) -> u64 {
        self.shape.numel()
    }

    /// Returns ordered variable names.
    #[must_use]
    pub fn variable_names(&self) -> &[TableVariableName] {
        self.schema.variable_names()
    }

    /// Looks up an exact variable name.
    #[must_use]
    pub fn variable_index(&self, name: &str) -> Option<usize> {
        self.schema.variable_index(name)
    }

    /// Borrows a complete table variable.
    #[must_use]
    pub fn variable(&self, index: usize) -> Option<&Value> {
        self.variables.get(index)
    }

    /// Borrows a complete table variable by name.
    #[must_use]
    pub fn variable_by_name(&self, name: &str) -> Option<&Value> {
        self.variable(self.variable_index(name)?)
    }

    /// Returns the optional ordered row names.
    #[must_use]
    pub fn row_names(&self) -> Option<&[TableRowName]> {
        self.row_names.as_deref().map(Vec::as_slice)
    }

    /// Looks up an exact row name.
    #[must_use]
    pub fn row_name_index(&self, name: &str) -> Option<usize> {
        self.row_names()?
            .iter()
            .position(|candidate| candidate.as_str() == name)
    }

    /// Atomically replaces all row names.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid, duplicate, or incorrectly sized names.
    pub fn set_row_names(&mut self, names: Vec<TableRowName>) -> Result<(), TableError> {
        self.row_names = Some(Arc::new(validate_row_names(self.row_count(), names)?));
        Ok(())
    }

    /// Removes the optional row-name schema.
    pub fn clear_row_names(&mut self) {
        self.row_names = None;
    }

    /// Replaces a complete variable after validating the row count.
    ///
    /// # Errors
    ///
    /// Returns an error when the offset is invalid or the replacement has an
    /// unsupported shape or a different first dimension.
    pub fn replace_variable(&mut self, index: usize, value: Value) -> Result<Value, TableError> {
        if self.variables.get(index).is_none() {
            return Err(TableError::VariableOutOfBounds {
                variable: index,
                variables: self.variable_count(),
            });
        }
        validate_rows(index, self.row_count(), &value)?;
        let variables = Arc::make_mut(&mut self.variables);
        Ok(std::mem::replace(&mut variables[index], value))
    }

    /// Appends one named variable while preserving existing variable storage.
    ///
    /// # Errors
    ///
    /// Returns an error for a duplicate name, an incompatible variable, or an
    /// unrepresentable widened table shape.
    pub fn append_variable(
        &mut self,
        name: TableVariableName,
        value: Value,
    ) -> Result<(), TableError> {
        if self.variable_index(name.as_str()).is_some() {
            return Err(TableError::DuplicateVariableName {
                name: name.to_string(),
            });
        }
        let value_rows = value
            .dimensions()
            .and_then(|dimensions| dimensions.first().copied())
            .ok_or_else(|| TableError::UnsupportedVariable {
                variable: self.variable_count(),
                class: value.class_name().to_owned(),
            })?;
        let rows = if self.variable_count() == 0 && self.row_count() == 0 {
            value_rows
        } else {
            validate_rows(self.variable_count(), self.row_count(), &value)?;
            self.row_count()
        };
        let width = u64::try_from(self.variable_count() + 1)
            .map_err(|_| TableError::Array(ArrayError::HostLengthOverflow { numel: u64::MAX }))?;
        let shape = Shape::new([rows, width])?;
        Arc::make_mut(&mut self.schema).append(name);
        Arc::make_mut(&mut self.variables).push(value);
        self.shape = shape;
        Ok(())
    }

    /// Removes one variable while retaining the independent table height.
    ///
    /// # Errors
    ///
    /// Returns an error when the offset is outside the table schema.
    pub fn remove_variable(
        &mut self,
        index: usize,
    ) -> Result<(TableVariableName, Value), TableError> {
        if self.variables.get(index).is_none() {
            return Err(TableError::VariableOutOfBounds {
                variable: index,
                variables: self.variable_count(),
            });
        }
        let width = u64::try_from(self.variable_count() - 1)
            .map_err(|_| TableError::Array(ArrayError::HostLengthOverflow { numel: u64::MAX }))?;
        let shape = Shape::new([self.row_count(), width])?;
        let name = Arc::make_mut(&mut self.schema).remove(index);
        let value = Arc::make_mut(&mut self.variables).remove(index);
        self.shape = shape;
        Ok((name, value))
    }

    /// Atomically replaces the complete ordered variable-name schema.
    ///
    /// # Errors
    ///
    /// Returns an error for a wrong name count, invalid name, or duplicate.
    pub fn rename_variables(&mut self, names: Vec<TableVariableName>) -> Result<(), TableError> {
        if names.len() != self.variable_count() {
            return Err(TableError::VariableCountMismatch {
                names: names.len(),
                variables: self.variable_count(),
            });
        }
        let schema = TableSchema::new(names)?;
        self.schema = Arc::new(schema);
        Ok(())
    }

    /// Selects complete variables by zero-based offsets.
    ///
    /// # Errors
    ///
    /// Returns an error when an offset is outside the table schema.
    pub fn select_variables(&self, offsets: &[usize]) -> Result<Self, TableError> {
        let mut names = Vec::with_capacity(offsets.len());
        let mut variables = Vec::with_capacity(offsets.len());
        for &offset in offsets {
            let Some(name) = self.variable_names().get(offset) else {
                return Err(TableError::VariableOutOfBounds {
                    variable: offset,
                    variables: self.variable_count(),
                });
            };
            names.push(name.clone());
            variables.push(self.variables[offset].clone());
        }
        Self::from_parts_with_row_names(
            self.row_count(),
            names,
            variables,
            self.row_names().map(<[TableRowName]>::to_vec),
        )
    }

    /// Returns whether two tables share their ordered schema allocation.
    #[must_use]
    pub fn shares_schema_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.schema, &other.schema)
    }

    /// Returns whether two tables share their outer variable vector.
    #[must_use]
    pub fn shares_storage_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.variables, &other.variables)
    }
}

fn validate_rows(variable: usize, expected: u64, value: &Value) -> Result<(), TableError> {
    let Some(actual) = value
        .dimensions()
        .and_then(|dimensions| dimensions.first().copied())
    else {
        return Err(TableError::UnsupportedVariable {
            variable,
            class: value.class_name().to_owned(),
        });
    };
    if actual != expected {
        return Err(TableError::RowCountMismatch {
            variable,
            expected,
            actual,
        });
    }
    Ok(())
}

fn validate_row_names(
    row_count: u64,
    names: Vec<TableRowName>,
) -> Result<Vec<TableRowName>, TableError> {
    if u64::try_from(names.len()).unwrap_or(u64::MAX) != row_count {
        return Err(TableError::RowNameCountMismatch {
            names: names.len(),
            rows: row_count,
        });
    }
    let mut unique = BTreeSet::new();
    for name in &names {
        if !unique.insert(name.as_str()) {
            return Err(TableError::DuplicateRowName {
                name: name.to_string(),
            });
        }
    }
    Ok(names)
}

#[cfg(test)]
mod tests {
    use openmat_array::{ArrayData, DenseArray};

    use super::*;

    fn column(values: &[f64]) -> Value {
        Value::Array(ArrayData::F64(
            DenseArray::from_vec(
                Shape::new([values.len() as u64, 1]).unwrap(),
                values.to_vec(),
            )
            .unwrap(),
        ))
    }

    fn names(values: &[&str]) -> Vec<TableVariableName> {
        values
            .iter()
            .map(|value| TableVariableName::new(*value).unwrap())
            .collect()
    }

    #[test]
    fn table_shape_counts_variables_not_variable_storage_columns() {
        let matrix = Value::Array(ArrayData::F64(
            DenseArray::from_vec(Shape::new([2, 2]).unwrap(), vec![1.0, 2.0, 3.0, 4.0]).unwrap(),
        ));
        let table = TableArray::from_variables(
            names(&["Matrix", "Column"]),
            vec![matrix, column(&[5.0, 6.0])],
        )
        .unwrap();
        assert_eq!(table.shape().dimensions(), &[2, 2]);
        assert_eq!(table.row_count(), 2);
        assert_eq!(table.variable_count(), 2);
        assert_eq!(table.variable_names(), names(&["Matrix", "Column"]));
        assert_eq!(table.variable_by_name("Column"), Some(&column(&[5.0, 6.0])));
    }

    #[test]
    fn table_validates_names_counts_and_rows_before_detaching() {
        assert!(matches!(
            TableArray::from_variables(names(&["A"]), Vec::new()),
            Err(TableError::VariableCountMismatch { .. })
        ));
        assert!(matches!(
            TableArray::from_variables(names(&["A", "A"]), vec![column(&[1.0]), column(&[2.0])]),
            Err(TableError::DuplicateVariableName { .. })
        ));
        assert!(matches!(
            TableArray::from_variables(
                names(&["A", "B"]),
                vec![column(&[1.0]), column(&[2.0, 3.0])]
            ),
            Err(TableError::RowCountMismatch { .. })
        ));

        let original =
            TableArray::from_variables(names(&["A"]), vec![column(&[1.0, 2.0])]).unwrap();
        let mut assigned = original.clone();
        assert!(matches!(
            assigned.replace_variable(0, column(&[9.0])),
            Err(TableError::RowCountMismatch { .. })
        ));
        assert!(original.shares_schema_with(&assigned));
        assert!(original.shares_storage_with(&assigned));
    }

    #[test]
    fn table_clone_detaches_only_after_valid_schema_or_value_mutation() {
        let original =
            TableArray::from_variables(names(&["A"]), vec![column(&[1.0, 2.0])]).unwrap();
        let mut replaced = original.clone();
        replaced.replace_variable(0, column(&[3.0, 4.0])).unwrap();
        assert!(original.shares_schema_with(&replaced));
        assert!(!original.shares_storage_with(&replaced));

        let mut appended = original.clone();
        appended
            .append_variable(TableVariableName::new("B").unwrap(), column(&[5.0, 6.0]))
            .unwrap();
        assert!(!original.shares_schema_with(&appended));
        assert!(!original.shares_storage_with(&appended));
        assert_eq!(appended.shape().dimensions(), &[2, 2]);
    }

    #[test]
    fn empty_table_preserves_explicit_height() {
        let table = TableArray::empty(3).unwrap();
        assert_eq!(table.shape().dimensions(), &[3, 0]);
        assert_eq!(table.numel(), 0);
    }

    #[test]
    fn first_append_establishes_height_and_last_delete_retains_it() {
        let mut table = TableArray::empty(0).unwrap();
        table
            .append_variable(
                TableVariableName::new("A").unwrap(),
                column(&[1.0, 2.0, 3.0]),
            )
            .unwrap();
        assert_eq!(table.shape().dimensions(), &[3, 1]);
        let removed = table.remove_variable(0).unwrap();
        assert_eq!(removed.0.as_str(), "A");
        assert_eq!(table.shape().dimensions(), &[3, 0]);
        assert!(
            table
                .append_variable(TableVariableName::new("B").unwrap(), column(&[9.0]))
                .is_err()
        );
        assert_eq!(table.shape().dimensions(), &[3, 0]);
    }

    #[test]
    fn rename_validates_complete_schema_before_detaching() {
        let original =
            TableArray::from_variables(names(&["A", "B"]), vec![column(&[1.0]), column(&[2.0])])
                .unwrap();
        let mut renamed = original.clone();
        assert!(renamed.rename_variables(names(&["X", "X"])).is_err());
        assert!(original.shares_schema_with(&renamed));
        renamed.rename_variables(names(&["X", "Y"])).unwrap();
        assert_eq!(renamed.variable_names(), names(&["X", "Y"]));
        assert!(!original.shares_schema_with(&renamed));
        assert!(original.shares_storage_with(&renamed));
    }

    #[test]
    fn row_names_validate_lookup_and_survive_variable_selection() {
        let row_names = ["Alice", "Bob", "Carol"]
            .into_iter()
            .map(TableRowName::new)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let table = TableArray::from_parts_with_row_names(
            3,
            names(&["A", "B"]),
            vec![column(&[1.0, 2.0, 3.0]), column(&[4.0, 5.0, 6.0])],
            Some(row_names),
        )
        .unwrap();
        assert_eq!(table.row_name_index("Bob"), Some(1));
        assert_eq!(table.row_name_index("Missing"), None);

        let selected = table.select_variables(&[1]).unwrap();
        assert_eq!(
            selected
                .row_names()
                .unwrap()
                .iter()
                .map(TableRowName::as_str)
                .collect::<Vec<_>>(),
            ["Alice", "Bob", "Carol"]
        );
        assert!(matches!(
            TableArray::from_parts_with_row_names(
                3,
                names(&["A"]),
                vec![column(&[1.0, 2.0, 3.0])],
                Some(vec![TableRowName::new("only").unwrap()]),
            ),
            Err(TableError::RowNameCountMismatch { .. })
        ));
        assert!(matches!(
            TableArray::from_parts_with_row_names(
                2,
                names(&["A"]),
                vec![column(&[1.0, 2.0])],
                Some(vec![
                    TableRowName::new("same").unwrap(),
                    TableRowName::new("same").unwrap(),
                ]),
            ),
            Err(TableError::DuplicateRowName { .. })
        ));
    }
}
