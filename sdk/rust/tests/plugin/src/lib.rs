use std::{
    fmt::Debug,
    sync::atomic::{AtomicUsize, Ordering},
};

use oex::{
    Call, Char16, Complex, Element, Error, ErrorKind, Logical, NativeClass, NativeMethod,
    NativeProperty, Result, ValueKind,
};

fn roundtrip<T: Element + PartialEq + Debug>(call: &Call<'_>, value: T) -> Result<()> {
    let original = call.array(&[2, 2], &[value; 4])?;
    assert!(original.is_dense::<T>()?);
    assert!(!original.is_scalar::<T>()?);
    assert_eq!(original.kind()?, ValueKind::Dense);
    assert_eq!(original.info()?.element_size as usize, size_of::<T>());
    let mut copy = original.try_clone()?;
    {
        let view = original.array::<T>()?;
        assert_eq!(view.dimensions(), &[2, 2]);
        assert_eq!(view.get(&[1, 1]), Some(&value));
        assert_eq!(view.get(&[2, 0]), None);
        assert_eq!(view.get(&[0]), None);
        assert_eq!(
            view.as_slice().as_ptr(),
            copy.array::<T>()?.as_slice().as_ptr()
        );
        let mut mutable = copy.array_mut::<T>()?;
        mutable.as_mut_slice().fill(T::default());
        assert_eq!(mutable.get_mut(&[9, 0]), None);
        assert_eq!(view.as_slice(), &[value; 4]);
    }
    assert_eq!(copy.array::<T>()?.as_slice(), &[T::default(); 4]);
    assert_eq!(original.array::<T>()?.as_slice(), &[value; 4]);
    let empty = call.zeros::<T>(&[0, 3])?;
    assert!(empty.array::<T>()?.is_empty());
    let scalar = call.array(&[1, 1], &[value])?;
    assert!(scalar.is_scalar::<T>()?);
    Ok(())
}

fn selftest(call: &mut Call<'_>) -> Result<()> {
    containers_selftest(call)?;
    macro_rules! real { ($($ty:ty),*) => { $(roundtrip(call, 7 as $ty)?;)* }; }
    macro_rules! complex { ($($ty:ty),*) => { $(roundtrip(call, Complex { re: 2 as $ty, im: 3 as $ty })?;)* }; }
    real!(i8, u8, i16, u16, i32, u32, i64, u64, f32, f64);
    complex!(i8, u8, i16, u16, i32, u32, i64, u64, f32, f64);
    roundtrip(call, Logical::TRUE)?;
    roundtrip(call, Char16(0xd800))?;
    assert!(call.dense_builder::<f64>(&[4]).is_err());
    assert!(call.array(&[2, 2], &[1.0]).is_err());
    let mut scalar = call.scalar(5.0)?;
    assert!(scalar.array::<i64>().is_err());
    assert!(scalar.array_mut::<i64>().is_err());
    let mut buffer = call.dense_builder::<i32>(&[2, 2])?;
    for (i, slot) in buffer.uninit_slice().iter_mut().enumerate() {
        slot.write(i as i32);
    }
    // SAFETY: all four i32 slots were just initialized.
    let result = unsafe { buffer.assume_init()? };
    assert_eq!(result.array::<i32>()?.as_slice(), &[0, 1, 2, 3]);
    let mut abandoned = call.dense_builder::<f64>(&[100, 1])?;
    abandoned.uninit_slice()[0].write(1.0);
    drop(abandoned);
    let filled = call
        .dense_builder::<f64>(&[2, 2])?
        .fill_with(|i| i as f64)?;
    assert_eq!(filled.array::<f64>()?.as_slice(), &[0., 1., 2., 3.]);

    let mut triplets = call.sparse_triplets::<f64>(3, 2, 4)?;
    assert!(triplets.push(3, 0, 1.).is_err());
    triplets.extend([(2, 1, 4.), (0, 0, 1.), (0, 0, 2.), (1, 0, 0.)])?;
    assert!(triplets.push(0, 0, 1.).is_err());
    let value = triplets.finish()?;
    assert!(value.is_sparse::<f64>()?);
    let view = value.sparse::<f64>()?;
    assert_eq!(view.column_offsets(), &[0, 1, 2]);
    assert_eq!(view.row_indices(), &[0, 2]);
    assert_eq!(view.values(), &[3., 4.]);
    assert_eq!(view.rows(), 3);
    assert_eq!(view.columns(), 2);
    let empty = call.sparse_triplets::<f64>(0, 0, 0)?.finish()?;
    assert_eq!(empty.sparse::<f64>()?.column_offsets(), &[0]);

    assert!(call.sparse_pattern(2, 2, &[0, 1], &[0]).is_err());
    assert!(call.sparse_pattern(2, 1, &[0, 2], &[1, 0]).is_err());
    let pattern = call.sparse_pattern(2, 2, &[0, 1, 2], &[0, 1])?;
    let retained = pattern.try_clone()?;
    let values = call.sparse_values::<f64>(&pattern)?;
    drop(pattern);
    assert_eq!(
        values.copy_from_slice(&[2., 0.])?.sparse::<f64>()?.values(),
        &[2.]
    );
    let logical = call
        .sparse_values::<Logical>(&retained)?
        .fill(Logical::TRUE)?;
    assert_eq!(logical.sparse::<Logical>()?.stored_count(), 2);
    let complex = call
        .sparse_values::<Complex<f64>>(&retained)?
        .fill_with(|i| Complex {
            re: i as f64,
            im: 1.,
        })?;
    assert_eq!(complex.sparse::<Complex<f64>>()?.values()[1].im, 1.);
    let mut initialized = call.sparse_values::<f64>(&retained)?;
    for slot in initialized.uninit_slice() {
        slot.write(9.0);
    }
    assert_eq!(
        // SAFETY: the loop initialized the entire pattern buffer.
        unsafe { initialized.assume_init()? }
            .sparse::<f64>()?
            .values(),
        &[9., 9.]
    );
    drop(call.sparse_values::<f64>(&retained)?);
    drop(retained);

    let cancellation = call.cancellation();
    std::thread::scope(|scope| {
        scope
            .spawn(move || assert!(!cancellation.is_requested()))
            .join()
            .unwrap()
    });
    call.check_cancelled()?;
    call.set_output(0, call.scalar(1.0)?)
}

fn containers_selftest(call: &Call<'_>) -> Result<()> {
    let shape = [2, 2];
    let original = call.strings_utf8(&shape, &[Some("中文🙂\0x"), Some(""), None, Some("last")])?;
    assert_eq!(original.class_name(call)?, "string");
    assert_eq!(original.dimensions()?, shape);
    assert_eq!(original.string_utf8(0)?.as_deref(), Some("中文🙂\0x"));
    assert_eq!(original.string_utf8(1)?.as_deref(), Some(""));
    assert_eq!(original.string_utf8(2)?, None);
    let mut strings = original.try_clone()?;
    strings.set_string_utf16(call, 0, &[0xd800, 0])?;
    assert_eq!(strings.string_element(0)?.code_units, &[0xd800, 0]);
    assert_eq!(
        strings.string_utf8(0).unwrap_err().kind(),
        ErrorKind::Encoding
    );
    strings.set_strings_utf8(call, 0, &[Some("a"), None])?;
    strings.set_strings_utf16(call, 2, &[Some(&[0xdfff]), Some(&[])])?;
    assert!(strings.string_elements(0, 4)?[1].is_missing);
    strings.set_missing(call, 3)?;
    let exact = call.strings_utf16(&[1, 1], &[Some(&[0xd800])])?;
    assert_eq!(exact.string_element(0)?.code_units, &[0xd800]);
    assert!(!call.strings(&[1, 1])?.string_element(0)?.is_missing);
    assert_eq!(original.string_utf8(0)?.as_deref(), Some("中文🙂\0x"));
    let scalar = call.scalar(3.)?;
    let mut cells = call.cell(&shape)?;
    cells.set_cell_elements(call, 1, &[strings.as_value_ref(), scalar.as_value_ref()])?;
    assert_eq!(cells.cell_elements(call, 1, 2)?[1].as_f64()?, 3.);
    let mut child = cells.cell_element(call, 1)?;
    child.set_string_utf8(call, 0, "changed")?;
    assert_eq!(
        cells.cell_element(call, 1)?.string_utf8(0)?.as_deref(),
        Some("a")
    );
    cells.set_cell_element(call, 1, child.as_value_ref())?;
    let mut structure = call.structure(&shape, &["a", "b"])?;
    structure.set_struct_field_values(call, 0, cells.as_value_ref())?;
    assert_eq!(structure.struct_field_values(call, 0)?.dimensions()?, shape);
    structure.set_struct_field(call, 3, 1, scalar.as_value_ref())?;
    assert_eq!(structure.struct_field(call, 3, 1)?.as_f64()?, 3.);
    structure.add_struct_field(call, "c")?;
    structure.remove_struct_field(call, 2)?;
    structure.set_struct_field_names(call, &["b", "a"])?;
    assert_eq!(structure.struct_field_count()?, 2);
    assert_eq!(structure.struct_field_name(0)?, "b");
    assert_eq!(structure.find_struct_field("a")?, Some(1));
    assert_eq!(structure.find_struct_field("absent")?, None);
    let mut table = call.table(
        2,
        &[
            ("text data", original.as_value_ref()),
            ("records", structure.as_value_ref()),
        ],
    )?;
    assert_eq!(table.table_row_count()?, 2);
    assert_eq!(table.table_variable_count()?, 2);
    assert_eq!(table.table_variable_name(0)?, "text data");
    assert_eq!(table.find_table_variable("records")?, Some(1));
    assert_eq!(table.find_table_variable("absent")?, None);
    assert_eq!(table.table_variable(call, 0)?.dimensions()?, shape);
    table.set_table_variable(call, 0, strings.as_value_ref())?;
    table.append_table_variable(call, "cells", cells.as_value_ref())?;
    table.remove_table_variable(call, 2)?;
    table.set_table_variable_names(call, &["Text", "Data"])?;
    table.set_table_row_names(call, &["第一行", "second"])?;
    assert_eq!(
        table
            .table_row_names(call)?
            .unwrap()
            .string_utf8(0)?
            .as_deref(),
        Some("第一行")
    );
    assert!(table.set_table_variable_names(call, &["x", "x"]).is_err());
    assert_eq!(table.table_variable_name(0)?, "Text");
    table.clear_table_row_names(call)?;
    assert!(table.table_row_names(call)?.is_none());
    let mut zero = call.table(0, &[])?;
    assert!(zero.table_row_names(call)?.is_none());
    zero.set_table_row_names(call, &[])?;
    assert_eq!(
        zero.table_row_names(call)?.unwrap().info()?.element_count,
        0
    );
    assert_eq!(call.table(7, &[])?.dimensions()?, [7, 0]);
    Ok(())
}

fn taken(call: &mut Call<'_>) -> Result<()> {
    let retained = call.input(0)?.try_to_owned()?;
    let mut value = call.take_input(0)?;
    assert!(matches!(call.input(0), Err(e) if e.kind() == ErrorKind::State));
    assert!(matches!(call.take_input(0), Err(e) if e.kind() == ErrorKind::State));
    value.array_mut::<f64>()?.as_mut_slice().fill(99.0);
    call.set_output(0, value)?;
    call.set_output(1, retained)
}

fn failure(call: &mut Call<'_>) -> Result<()> {
    let _uncommitted = call.dense_builder::<f64>(&[20, 20])?;
    let _uncommitted_sparse = call.sparse_triplets::<f64>(4, 4, 8)?;
    let _value = call.scalar(7.0)?;
    Err(Error::new("RustTest:Failure", "intentional failure"))
}

fn panic_test(call: &mut Call<'_>) -> Result<()> {
    call.dense_builder::<f64>(&[2, 2])?.fill_with(|i| {
        if i == 2 {
            panic!("intentional fill panic");
        }
        1.0
    })?;
    Ok(())
}

fn invoke(call: &mut Call<'_>) -> Result<()> {
    let outputs = call.invoke(call.input(0)?, &[call.input(1)?], call.output_count())?;
    for (index, value) in outputs.into_iter().enumerate() {
        call.set_output(index as u32, value)?;
    }
    Ok(())
}
fn swallow(call: &mut Call<'_>) -> Result<()> {
    assert!(call.invoke(call.input(0)?, &[], 0).is_err());
    Err(Error::new(
        "RustTest:Replacement",
        "must not replace language error",
    ))
}

static DESTROYED: AtomicUsize = AtomicUsize::new(0);
struct Counter {
    value: f64,
}
impl Drop for Counter {
    fn drop(&mut self) {
        DESTROYED.fetch_add(1, Ordering::SeqCst);
    }
}
static COUNTER: NativeClass<Counter> = NativeClass::new();
static UNREGISTERED: NativeClass<Counter> = NativeClass::new();
fn construct(call: &mut Call<'_>) -> Result<Counter> {
    Ok(Counter {
        value: call.input(0)?.as_f64()?,
    })
}
fn getter(call: &mut Call<'_>, counter: &mut Counter) -> Result<()> {
    call.set_output(0, call.scalar(counter.value)?)
}
fn setter(call: &mut Call<'_>, counter: &mut Counter) -> Result<()> {
    counter.value = call.input(0)?.as_f64()?;
    Ok(())
}
fn increment(call: &mut Call<'_>, counter: &mut Counter) -> Result<()> {
    counter.value += call.input(0)?.as_f64()?;
    Ok(())
}
fn reenter(call: &mut Call<'_>, _: &mut Counter) -> Result<()> {
    call.invoke(call.input(0)?, &[], 0)?;
    Ok(())
}
fn read_object(call: &mut Call<'_>) -> Result<()> {
    assert_eq!(call.input(0)?.class_name(call)?, "RustTestCounter");
    let mut cell = call.cell(&[1, 1])?;
    cell.set_cell_element(call, 0, call.input(0)?)?;
    let child = cell.cell_element(call, 0)?;
    assert_eq!(child.class_name(call)?, "RustTestCounter");
    let nested_object = COUNTER.borrow(call, child.as_value_ref())?;
    let nested_value = nested_object.try_lock()?.value;
    let object = COUNTER.borrow(call, call.input(0)?)?;
    let other = object.clone();
    let state = object.try_lock()?;
    assert_eq!(state.value, nested_value);
    assert!(other.try_lock().is_err());
    call.set_output(0, call.scalar(state.value)?)
}
fn factory(call: &mut Call<'_>) -> Result<()> {
    call.set_output(0, COUNTER.create(call, Counter { value: 12. })?)
}
fn failed_factory(call: &mut Call<'_>) -> Result<()> {
    let before = DESTROYED.load(Ordering::SeqCst);
    assert!(UNREGISTERED.create(call, Counter { value: 0. }).is_err());
    assert_eq!(DESTROYED.load(Ordering::SeqCst), before + 1);
    let value = COUNTER.create(call, Counter { value: 1. })?;
    // Invalid output must release the owned handle; callback error rolls back
    // the already-transferred native allocation exactly once.
    call.set_output(99, value)
}
fn destroyed(call: &mut Call<'_>) -> Result<()> {
    call.set_output(0, call.scalar(DESTROYED.load(Ordering::SeqCst) as f64)?)
}
fn duplicate_output(call: &mut Call<'_>) -> Result<()> {
    call.set_output(0, call.scalar(1.0)?)?;
    assert!(
        matches!(call.set_output(0, call.scalar(2.0)?), Err(e) if e.kind() == ErrorKind::State)
    );
    Ok(())
}

static METHODS: &[NativeMethod<Counter>] = &[
    oex::method!(COUNTER, "increment", 1..=1, 0..=0, increment),
    oex::method!(COUNTER, "reenter", 1..=1, 0..=0, reenter),
];
static PROPERTIES: &[NativeProperty<Counter>] = &[NativeProperty::new(
    "Value",
    Some(oex::method!(COUNTER, "get", 0..=0, 1..=1, getter)),
    Some(oex::method!(COUNTER, "set", 1..=1, 0..=0, setter)),
)];

oex::export_plugin! {
    name: "Rust SDK tests", version: "1.0.0",
    functions: [
        oex::function!("rust_selftest", 0..=0, 1..=1, selftest),
        oex::function!("rust_taken", 1..=1, 2..=2, taken),
        oex::function!("rust_failure", 0..=0, 0..=0, failure),
        oex::function!("rust_panic", 0..=0, 0..=0, panic_test),
        oex::function!("rust_callback", 2..=2, 0..=2, invoke),
        oex::function!("rust_swallow", 1..=1, 0..=0, swallow),
        oex::function!("rust_read_object", 1..=1, 1..=1, read_object),
        oex::function!("rust_factory", 0..=0, 1..=1, factory),
        oex::function!("rust_failed_factory", 0..=0, 0..=0, failed_factory),
        oex::function!("rust_destroyed", 0..=0, 1..=1, destroyed),
        oex::function!("rust_duplicate_output", 0..=0, 1..=1, duplicate_output),
    ],
    classes: [oex::class!(COUNTER, "RustTestCounter", construct, methods: METHODS, properties: PROPERTIES)],
}
