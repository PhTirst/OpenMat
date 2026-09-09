//! Bounded UI presentation data with session-local keys, never native handles.
use super::{
    AccessContext, ArrayData, Interpreter, ObjectAccess, OutputEvent, RuntimeResult,
    SourceLocation, Value,
};
use serde_json::{Map, Value as Json, json};

impl Interpreter {
    pub(super) fn ui_intrinsic(
        &mut self,
        arguments: &[Value],
        outputs: usize,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Vec<Value>> {
        self.expect_intrinsic_arity("openmat_ui", arguments, 2, location)?;
        let kind = self.function_name_text("openmat_ui", &arguments[0], location)?;
        if kind == "identity" {
            let Value::Object(handle) = &arguments[1] else {
                return Err(self.invalid_state("UI identity requires a scalar object"));
            };
            if outputs != 1 {
                return Err(self.invalid_state("UI identity returns one value"));
            }
            return Ok(vec![self.char_row_value(
                &format!("ui-{}", handle.identifier()),
                "UI identity",
                location,
            )?]);
        }
        if outputs != 0 {
            return Err(
                self.invalid_state("openmat_ui publishes a display and does not return values")
            );
        }
        if kind == "destroy" {
            let class_id = self
                .object_value_class_id(&arguments[1])
                .map_err(|()| self.invalid_state("UI destruction requires a component"))?;
            let base = self
                .classes
                .class_id("matlab.ui.componentcontainer.ComponentContainer")
                .map_err(|_| self.invalid_state("UI base class is not loaded"))?;
            if !self.classes.is_subclass_of(class_id, base).unwrap_or(false) {
                return Err(self.invalid_state("UI destruction requires a component"));
            }
            self.process_lifecycle_deletion(&arguments[1], location)?;
            return Ok(Vec::new());
        }
        if kind != "snapshot" && kind != "describe" {
            return Err(self.invalid_state("openmat_ui expects snapshot or describe"));
        }
        let mut budget = 16_384usize;
        let component =
            self.ui_component(&arguments[1], kind == "describe", 0, &mut budget, location)?;
        let payload = json!({"version": 1, "kind": kind, "component": component}).to_string();
        if payload.len() > 750_000 {
            return Err(self.invalid_state("UI display exceeds 750000 bytes"));
        }
        self.output
            .emit(OutputEvent::UiDisplay(payload))
            .map_err(|error| self.invalid_state(&error.message))?;
        Ok(Vec::new())
    }

    fn ui_component(
        &mut self,
        value: &Value,
        describe: bool,
        depth: usize,
        budget: &mut usize,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<Json> {
        if depth > 32 || *budget == 0 {
            return Err(self.invalid_state("UI component tree exceeds its depth or size limit"));
        }
        *budget -= 1;
        let class_id = self
            .object_value_class_id(value)
            .map_err(|()| self.invalid_state("UI components must be scalar classdef objects"))?;
        let base = self
            .classes
            .class_id("matlab.ui.componentcontainer.ComponentContainer")
            .map_err(|_| self.invalid_state("UI base class is not loaded"))?;
        if !self.classes.is_subclass_of(class_id, base).unwrap_or(false) {
            return Err(self.invalid_state(
                "UI components must inherit matlab.ui.componentcontainer.ComponentContainer",
            ));
        }
        let class_name = self.value_class_name(value);
        let fields = self
            .classes
            .stored_properties(class_id)
            .map_err(|error| self.object_error("UI metadata", &error, location))?
            .into_iter()
            .filter(|(_, p)| p.get_access() == Some(ObjectAccess::Public))
            .map(|(_, p)| {
                (
                    p.name().to_owned(),
                    p.set_access() == Some(ObjectAccess::Public),
                )
            })
            .collect::<Vec<_>>();
        let mut properties = Map::new();
        let mut schema = Vec::new();
        let mut children = Vec::new();
        let mut layout = json!({});
        let (events, methods) = if describe {
            self.ui_class_members(class_id, location)?
        } else {
            (Vec::new(), Vec::new())
        };
        for (name, writable) in fields {
            if name == "Parent" {
                continue;
            }
            let field = self.get_field(value, &name, AccessContext::external(), location)?;
            if name == "Layout" {
                layout = self.ui_layout(&field)?;
                continue;
            }
            if name == "Children" {
                if let Value::Cell(cell) = field {
                    for child in cell.values() {
                        children.push(self.ui_component(
                            child,
                            false,
                            depth + 1,
                            budget,
                            location,
                        )?);
                    }
                } else if field.numel() != Some(0) {
                    return Err(self.invalid_state(
                        "UI Children must be a cell array of scalar component objects",
                    ));
                }
                continue;
            }
            let encoded = ui_value(&field, budget, 0);
            if describe {
                schema.push(json!({"name": name, "writable": writable && encoded.is_some(), "value": encoded, "className": self.value_class_name(&field)}));
            }
            if let Some(encoded) = encoded {
                properties.insert(name, encoded);
            }
        }
        Ok(
            json!({"className": class_name, "properties": properties, "layout": layout, "schema": schema, "children": children, "events": events, "methods": methods}),
        )
    }

    fn ui_layout(&self, value: &Value) -> RuntimeResult<Json> {
        let Value::Struct(layout) = value else {
            return Err(self.invalid_state("UI Layout must be a scalar struct"));
        };
        if value.numel() != Some(1) {
            return Err(self.invalid_state("UI Layout must be a scalar struct"));
        }
        let names = layout
            .field_names()
            .iter()
            .map(|name| name.as_str().to_owned())
            .collect::<Vec<_>>();
        let mut result = Map::new();
        for name in names {
            if ![
                "Mode",
                "X",
                "Y",
                "Width",
                "Height",
                "Row",
                "Column",
                "RowSpan",
                "ColumnSpan",
                "Columns",
                "Gap",
                "Padding",
                "Grow",
                "WidthMode",
                "HeightMode",
                "MinWidth",
                "MinHeight",
                "MaxWidth",
                "MaxHeight",
                "GrowX",
                "GrowY",
                "RowTracks",
                "ColumnTracks",
            ]
            .contains(&name.as_str())
            {
                return Err(self.invalid_state("UI Layout contains an unsupported field"));
            }
            let field = layout
                .value_at(
                    layout
                        .field_index(&name)
                        .ok_or_else(|| self.invalid_state("UI Layout field is missing"))?,
                    0,
                )
                .ok_or_else(|| self.invalid_state("UI Layout field has no value"))?;
            let mut budget = 16;
            let encoded = ui_value(field, &mut budget, 0)
                .ok_or_else(|| self.invalid_state("UI Layout field is not serializable"))?;
            let valid = if name == "Mode" {
                encoded
                    .as_str()
                    .is_some_and(|mode| ["absolute", "grid", "row", "column"].contains(&mode))
            } else if name == "WidthMode" || name == "HeightMode" {
                encoded
                    .as_str()
                    .is_some_and(|mode| ["auto", "fixed", "content", "fill"].contains(&mode))
            } else if name == "RowTracks" || name == "ColumnTracks" {
                encoded.as_str().is_some_and(valid_ui_tracks)
            } else {
                encoded.as_f64().is_some_and(|number| {
                    (0.0..=10_000.0).contains(&number)
                        && (![
                            "Width",
                            "Height",
                            "Row",
                            "Column",
                            "RowSpan",
                            "ColumnSpan",
                            "Columns",
                            "MaxWidth",
                            "MaxHeight",
                        ]
                        .contains(&name.as_str())
                            || number >= 1.0)
                        && (!["Row", "Column", "RowSpan", "ColumnSpan", "Columns"]
                            .contains(&name.as_str())
                            || number.fract() == 0.0)
                })
            };
            if !valid {
                return Err(self.invalid_state("UI Layout field has an invalid value"));
            }
            let key = format!("{}{}", name[..1].to_ascii_lowercase(), &name[1..]);
            result.insert(key, encoded);
        }
        for (min, max) in [("minWidth", "maxWidth"), ("minHeight", "maxHeight")] {
            if result.get(min).and_then(Json::as_f64).unwrap_or(0.0)
                > result.get(max).and_then(Json::as_f64).unwrap_or(10_000.0)
            {
                return Err(self.invalid_state("UI minimum size exceeds its maximum size"));
            }
        }
        Ok(Json::Object(result))
    }

    fn ui_class_members(
        &self,
        class_id: openmat_object::ClassId,
        location: Option<SourceLocation>,
    ) -> RuntimeResult<(Vec<Json>, Vec<Json>)> {
        let mut events = Vec::new();
        let mut methods = Vec::new();
        for (owner, event) in self
            .classes
            .effective_events(class_id)
            .map_err(|error| self.object_error("UI events", &error, location))?
        {
            if event.listen_access() == ObjectAccess::Public && !event.is_hidden() {
                let owner = self
                    .classes
                    .class(owner)
                    .map_err(|error| self.object_error("UI event owner", &error, location))?;
                events.push(json!({"name": event.name(), "declaringClass": owner.name()}));
            }
        }
        for (owner, method) in self
            .classes
            .effective_methods(class_id)
            .map_err(|error| self.object_error("UI methods", &error, location))?
        {
            if method.kind() == openmat_object::MethodKind::Instance
                && !method.is_abstract()
                && !method.name().contains('.')
            {
                let owner = self
                    .classes
                    .class(owner)
                    .map_err(|error| self.object_error("UI method owner", &error, location))?;
                let access = match method.access() {
                    ObjectAccess::Public => "public",
                    ObjectAccess::Protected => "protected",
                    ObjectAccess::Private => "private",
                };
                methods.push(json!({"name": method.name(), "access": access, "declaringClass": owner.name()}));
            }
        }
        Ok((events, methods))
    }
}

fn valid_ui_tracks(text: &str) -> bool {
    let mut count = 0;
    text.split_whitespace().all(|entry| {
        count += 1;
        if count > 64 {
            return false;
        }
        if entry == "auto" {
            return true;
        }
        let number = entry
            .strip_suffix("px")
            .or_else(|| entry.strip_suffix("fr"))
            .unwrap_or(entry);
        let mut parts = number.split('.');
        let digits = |part: &str| !part.is_empty() && part.bytes().all(|c| c.is_ascii_digit());
        if !parts.next().is_some_and(digits)
            || !parts.next().is_none_or(digits)
            || parts.next().is_some()
        {
            return false;
        }
        number
            .parse::<f64>()
            .is_ok_and(|value| value > 0.0 && value <= 10_000.0)
    })
}

fn ui_value(value: &Value, budget: &mut usize, depth: usize) -> Option<Json> {
    if *budget == 0 || depth > 4 {
        return None;
    }
    *budget -= 1;
    match value {
        Value::Double(v) if v.is_finite() => Some(json!(v)),
        Value::Logical(v) => Some(json!(v)),
        Value::Array(ArrayData::Char(v)) if v.numel() < 65_536 => Some(json!(
            String::from_utf16_lossy(&v.as_slice().iter().map(|c| c.get()).collect::<Vec<_>>())
        )),
        Value::String(v) => v
            .as_scalar()
            .filter(|s| !s.is_missing() && s.code_units().len() < 65_536)
            .map(|s| json!(String::from_utf16_lossy(s.code_units()))),
        Value::Cell(cell) if cell.numel() <= 4096 => {
            let values = cell
                .values()
                .iter()
                .map(|v| ui_value(v, budget, depth + 1))
                .collect::<Option<Vec<_>>>()?;
            let dimensions = cell.shape().dimensions();
            if dimensions.len() != 2 {
                return None;
            }
            if values.iter().all(Json::is_string) && (dimensions[0] <= 1 || dimensions[1] <= 1) {
                return Some(Json::Array(values));
            }
            let rows = usize::try_from(dimensions[0]).ok()?;
            let columns = usize::try_from(dimensions[1]).ok()?;
            Some(Json::Array(
                (0..rows)
                    .map(|r| {
                        Json::Array((0..columns).map(|c| values[r + c * rows].clone()).collect())
                    })
                    .collect(),
            ))
        }
        Value::Array(ArrayData::F64(array))
            if array.numel() <= 4096 && array.as_slice().iter().all(|v| v.is_finite()) =>
        {
            let dims = array.shape().dimensions();
            if dims.len() != 2 {
                return None;
            }
            let rows = usize::try_from(dims[0]).ok()?;
            let columns = usize::try_from(dims[1]).ok()?;
            Some(Json::Array(
                (0..rows)
                    .map(|r| {
                        Json::Array(
                            (0..columns)
                                .map(|c| json!(array.as_slice()[r + c * rows]))
                                .collect(),
                        )
                    })
                    .collect(),
            ))
        }
        _ => None,
    }
}
