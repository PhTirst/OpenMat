use std::collections::BTreeSet;

use openmat_array::Shape;
use openmat_value::{
    BytecodeFunctionHandle, CellArray, ClassHandle, FieldName, FunctionHandle, ObjectArray,
    ObjectHandle, StructArray, TableArray, TableVariableName, Value,
};

#[test]
fn tracing_finds_handles_through_every_recursive_value_aggregate() {
    let object_array = ObjectArray::from_vec(
        ClassHandle::new(7),
        Shape::new([2, 1]).unwrap(),
        vec![ObjectHandle::new(2), ObjectHandle::new(3)],
    )
    .unwrap();
    let cell = CellArray::from_values(
        Shape::new([1, 2]).unwrap(),
        vec![
            Value::Object(ObjectHandle::new(1)),
            Value::ObjectArray(object_array),
        ],
    )
    .unwrap();
    let structure = StructArray::from_columns(
        Shape::new([1, 1]).unwrap(),
        vec![FieldName::new("nested").unwrap()],
        vec![vec![Value::Cell(cell)]],
    )
    .unwrap();
    let table = TableArray::from_variables(
        vec![TableVariableName::new("payload").unwrap()],
        vec![Value::Struct(structure)],
    )
    .unwrap();
    let root = Value::Cell(
        CellArray::from_values(
            Shape::new([1, 2]).unwrap(),
            vec![
                Value::Table(table),
                Value::Function(FunctionHandle::Bytecode(BytecodeFunctionHandle::new(41))),
            ],
        )
        .unwrap(),
    );

    let mut objects = BTreeSet::new();
    let mut functions = BTreeSet::new();
    root.trace_handles(
        &mut |handle| {
            objects.insert(handle.identifier());
        },
        &mut |handle| {
            functions.insert(handle.index());
        },
    );

    assert_eq!(objects, BTreeSet::from([1, 2, 3]));
    assert_eq!(functions, BTreeSet::from([41]));
}
