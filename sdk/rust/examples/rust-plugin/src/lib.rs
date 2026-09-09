use oex::{Call, NativeClass, NativeMethod, NativeProperty, Result};

fn add(call: &mut Call<'_>) -> Result<()> {
    let a = call.input(0)?.as_f64()?;
    let b = call.input(1)?.as_f64()?;
    call.set_output(0, call.scalar(a + b)?)
}

fn scale(call: &mut Call<'_>) -> Result<()> {
    let factor = call.input(1)?.as_f64()?;
    let mut value = call.take_input(0)?;
    for x in value.array_mut::<f64>()?.as_mut_slice() {
        *x *= factor;
    }
    call.set_output(0, value)
}

fn sparse(call: &mut Call<'_>) -> Result<()> {
    let mut builder = call.sparse_triplets::<f64>(2, 2, 3)?;
    builder.extend([(1, 1, 3.0), (0, 0, 1.0), (0, 0, 1.0)])?;
    call.set_output(0, builder.finish()?)
}

fn invoke(call: &mut Call<'_>) -> Result<()> {
    let mut outputs = call.invoke(call.input(0)?, &[call.input(1)?], 1)?;
    call.set_output(0, outputs.remove(0))
}

struct Counter {
    value: f64,
}
static COUNTER: NativeClass<Counter> = NativeClass::new();
fn construct(call: &mut Call<'_>) -> Result<Counter> {
    Ok(Counter {
        value: call.input(0)?.as_f64()?,
    })
}
fn increment(call: &mut Call<'_>, counter: &mut Counter) -> Result<()> {
    counter.value += call.input(0)?.as_f64()?;
    Ok(())
}
fn get_value(call: &mut Call<'_>, counter: &mut Counter) -> Result<()> {
    call.set_output(0, call.scalar(counter.value)?)
}
fn set_value(call: &mut Call<'_>, counter: &mut Counter) -> Result<()> {
    counter.value = call.input(0)?.as_f64()?;
    Ok(())
}
static METHODS: &[NativeMethod<Counter>] =
    &[oex::method!(COUNTER, "increment", 1..=1, 0..=0, increment)];
static PROPERTIES: &[NativeProperty<Counter>] = &[NativeProperty::new(
    "Value",
    Some(oex::method!(COUNTER, "get", 0..=0, 1..=1, get_value)),
    Some(oex::method!(COUNTER, "set", 1..=1, 0..=0, set_value)),
)];

oex::export_plugin! {
    name: "Rust OEX example", version: "1.0.0",
    functions: [
        oex::function!("rust_add", 2..=2, 1..=1, add),
        oex::function!("rust_scale", 2..=2, 1..=1, scale),
        oex::function!("rust_sparse", 0..=0, 1..=1, sparse),
        oex::function!("rust_invoke", 2..=2, 1..=1, invoke),
    ],
    classes: [oex::class!(COUNTER, "RustCounter", construct, methods: METHODS, properties: PROPERTIES)],
}
