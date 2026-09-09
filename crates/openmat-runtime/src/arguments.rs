//! Argument-block input binding, independent of the platform and VM stack.
use openmat_array::{ArrayData, Shape};
use openmat_bytecode::{ArgumentLayout, Function};
use openmat_value::{CellArray, FieldName, StructArray, Value};

pub(crate) fn bind(function: &Function, inputs: &[Value]) -> Result<Vec<Value>, String> {
    let Some(layout) = &function.argument_layout else {
        return Ok(inputs.to_vec());
    };
    let count = layout.positional_count as usize;
    if inputs.len() < layout.required_count as usize {
        return Err(format!(
            "{} requires at least {} positional inputs",
            function.name, layout.required_count
        ));
    }
    let mut bound = vec![Value::Nothing; function.parameter_count as usize];
    let fixed = fixed_inputs(layout, inputs);
    bound[..fixed].clone_from_slice(&inputs[..fixed]);
    let supplied = positional_inputs(layout, inputs);
    let width = layout.repeating_count as usize;
    if width != 0 {
        let repeated = &inputs[fixed..supplied];
        if !repeated.len().is_multiple_of(width) {
            return Err(format!(
                "repeating inputs must occur in complete groups of {width}"
            ));
        }
        let shape = Shape::new([1, (repeated.len() / width) as u64]).map_err(|e| e.to_string())?;
        for offset in 0..width {
            bound[count + offset] = Value::Cell(
                CellArray::from_values(
                    shape.clone(),
                    repeated
                        .iter()
                        .skip(offset)
                        .step_by(width)
                        .cloned()
                        .collect(),
                )
                .map_err(|e| e.to_string())?,
            );
        }
    }
    let tail = &inputs[supplied..];
    if !tail.len().is_multiple_of(2) {
        return Err("name-value inputs must occur in pairs".into());
    }
    let mut fields: Vec<Vec<(String, Value)>> = vec![Vec::new(); bound.len()];
    for pair in tail.chunks_exact(2) {
        let name = option_text(&pair[0]).ok_or("option names must be text scalars")?;
        let (slot, canonical) = resolve_name(layout, &name)?;
        let fields = &mut fields[slot];
        if let Some((_, value)) = fields.iter_mut().find(|(field, _)| field == canonical) {
            *value = pair[1].clone();
        } else {
            fields.push((canonical.clone(), pair[1].clone()));
        }
    }
    for (slot, value) in bound.iter_mut().enumerate().skip(count + width) {
        // Omitted options without defaults do not create fields.
        let selected = &fields[slot];
        *value = Value::Struct(
            StructArray::from_columns(
                Shape::new(vec![1, 1]).map_err(|e| e.to_string())?,
                selected
                    .iter()
                    .map(|(name, _)| FieldName::new(name.clone()))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| e.to_string())?,
                selected
                    .iter()
                    .map(|(_, value)| vec![value.clone()])
                    .collect(),
            )
            .map_err(|e| e.to_string())?,
        );
    }
    Ok(bound)
}

pub(crate) fn positional_inputs(layout: &ArgumentLayout, inputs: &[Value]) -> usize {
    let fixed = fixed_inputs(layout, inputs);
    if layout.repeating_count == 0 {
        return fixed;
    }
    // Only group boundaries can begin name-value pairs. A name-looking value
    // inside a group remains ordinary data; ambiguous names at a boundary are
    // diagnosed by the name-value binder, not silently accepted as group data.
    (fixed..inputs.len())
        .step_by(layout.repeating_count as usize)
        .find(|index| {
            option_text(&inputs[*index]).is_some_and(|name| is_option_name(layout, &name))
        })
        .unwrap_or(inputs.len())
}

fn is_option_name(layout: &ArgumentLayout, name: &str) -> bool {
    !name.is_empty()
        && layout.named.iter().any(|(_, field)| {
            field
                .to_ascii_lowercase()
                .starts_with(&name.to_ascii_lowercase())
        })
}

fn fixed_inputs(layout: &ArgumentLayout, inputs: &[Value]) -> usize {
    (layout.required_count as usize..inputs.len().min(layout.positional_count as usize))
        .find(|index| {
            option_text(&inputs[*index]).is_some_and(|name| resolve_name(layout, &name).is_ok())
        })
        .unwrap_or(inputs.len().min(layout.positional_count as usize))
}

fn resolve_name<'a>(layout: &'a ArgumentLayout, name: &str) -> Result<(usize, &'a String), String> {
    let exact = layout.named.iter().find(|(_, field)| field == name);
    let selected = if let Some(exact) = exact {
        exact
    } else {
        let lower = name.to_ascii_lowercase();
        let has_full = layout
            .named
            .iter()
            .any(|(_, field)| field.eq_ignore_ascii_case(name));
        let mut candidates = layout.named.iter().filter(|(_, field)| {
            !lower.is_empty()
                && if has_full {
                    field.eq_ignore_ascii_case(name)
                } else {
                    field.to_ascii_lowercase().starts_with(&lower)
                }
        });
        let first = candidates
            .next()
            .ok_or_else(|| format!("unknown option {name}"))?;
        if candidates.next().is_some() {
            return Err(format!("ambiguous option {name}"));
        }
        first
    };
    Ok((selected.0.get() as usize, &selected.1))
}

fn option_text(value: &Value) -> Option<String> {
    match value {
        Value::Array(ArrayData::Char(array)) if array.shape().dimensions().first() == Some(&1) => {
            String::from_utf16(
                &array
                    .as_slice()
                    .iter()
                    .map(|unit| unit.get())
                    .collect::<Vec<_>>(),
            )
            .ok()
        }
        Value::String(value) => value
            .as_scalar()
            .filter(|element| !element.is_missing())
            .and_then(|element| String::from_utf16(element.code_units()).ok()),
        _ => None,
    }
}
