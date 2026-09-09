The safety boundaries below must be rejected by the Rust compiler.

An exact string view prevents changing its backing element:
```compile_fail,E0502
fn invalid<'a>(call: &oex::Call<'a>, value: &mut oex::Value<'a>) -> oex::Result<()> {
    let view = value.string_element(0)?;
    value.set_string_utf8(call, 0, "changed")?;
    println!("{:?}", view.code_units);
    Ok(())
}
```

A container child cannot escape its callback:
```compile_fail
fn invalid<'a>(call: &oex::Call<'a>, value: &oex::Value<'a>) -> oex::Result<oex::Value<'static>> {
    value.cell_element(call, 0)
}
```

An input view prevents taking its slot:
```compile_fail,E0502
fn invalid(call: &mut oex::Call<'_>) -> oex::Result<()> {
    let view = call.input(0)?.array::<f64>()?;
    let value = call.take_input(0)?;
    println!("{:?}", view.as_slice());
    Ok(())
}
```

A mutable view prevents retaining the same value:
```compile_fail,E0502
fn invalid(value: &mut oex::Value<'_>) -> oex::Result<()> {
    let mut view = value.array_mut::<f64>()?;
    let other = value.try_clone()?;
    view.as_mut_slice()[0] = 1.0;
    Ok(())
}
```

Neither owned values nor builders escape the callback scope:
```compile_fail
fn invalid<'a>(call: &oex::Call<'a>) -> oex::Result<oex::Value<'static>> {
    call.scalar(1.0)
}
```
```compile_fail
fn invalid<'a>(call: &oex::Call<'a>) -> oex::Result<oex::DenseBuilder<'static, f64>> {
    call.dense_builder(&[2, 2])
}
```

Views cannot outlive temporary owners:
```compile_fail,E0515
fn invalid<'a>(call: &oex::Call<'a>) -> oex::Result<oex::ArrayView<'a, f64>> {
    call.scalar(1.0)?.array::<f64>()
}
```

Values cannot be sent to a worker thread:
```compile_fail,E0277
fn invalid(value: oex::Value<'_>) {
    std::thread::scope(|s| { s.spawn(move || drop(value)); });
}
```

Uninitialized storage cannot be committed without an explicit unsafe block:
```compile_fail,E0133
fn invalid(builder: oex::DenseBuilder<'_, f64>) {
    builder.assume_init();
}
```

Raw bytes cannot create invalid logical values in safe Rust:
```compile_fail,E0423
let logical = oex::Logical(2);
```

Sparse types are limited to the C ABI's supported element types:
```compile_fail,E0277
fn invalid(call: &oex::Call<'_>) {
    call.sparse_triplets::<i32>(2, 2, 4);
}
```
