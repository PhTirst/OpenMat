use openmat_array::{ArrayData, DenseArray, Shape};
use openmat_builtins::{BuiltinRegistry, minimal_registry};
use openmat_graphics_model::{
    AxesCoordinateSystem, Axis, CDataMapping, DataDType, GraphicsClass, GraphicsDeltaOperation,
    GraphicsObject, GraphicsProperty, GraphicsPropertyValue, GraphicsRequest, GraphicsResponse,
    GraphicsSession, Interpreter, LimitMode, LineStyle, Marker, NextPlot, PredefinedColormap,
    ProjectionMode, SurfaceColor, ThetaDirection, ThetaZeroLocation, TickMode, parula_r2022b,
};
use openmat_runtime::{
    BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinInvocationError, CancellationToken,
    OutputEvent, VecOutput,
};
use openmat_value::{StringElement, StringValue, Value};

fn invoke(
    registry: &BuiltinRegistry,
    session: &mut GraphicsSession,
    name: &str,
    arguments: &[Value],
    requested_outputs: usize,
) -> (Result<Vec<Value>, BuiltinError>, VecOutput) {
    let handle = registry
        .handle_by_name(name)
        .expect("graphics built-in registered");
    let cancellation = CancellationToken::new();
    let mut output = VecOutput::new();
    let mut context = BuiltinContext::with_graphics_service(
        requested_outputs,
        &cancellation,
        &mut output,
        session,
    );
    let result = registry
        .invoke(handle, arguments, &mut context)
        .map_err(|error| {
            let BuiltinInvocationError::Failed { error, .. } = error else {
                panic!("looked-up graphics handle must resolve")
            };
            error
        });
    (result, output)
}

#[test]
fn zero_output_graphics_creators_discard_handles_while_getters_return_them() {
    let registry = minimal_registry().unwrap();
    let mut session = GraphicsSession::new();

    let created = invoke(&registry, &mut session, "title", &[text("discarded")], 0)
        .0
        .unwrap();
    assert!(created.is_empty());
    assert!(session.current_figure().is_some());

    let plotted = invoke(
        &registry,
        &mut session,
        "plot",
        &[f64_matrix(1, 2, vec![1.0, 2.0])],
        0,
    )
    .0
    .unwrap();
    assert!(plotted.is_empty());

    let current = invoke(&registry, &mut session, "gcf", &[], 0).0.unwrap();
    assert!(
        matches!(current.as_slice(), [Value::Graphics(handle)] if handle.class() == GraphicsClass::Figure)
    );
}

#[test]
#[allow(clippy::float_cmp)]
fn plot3_scatter3_and_aspect_controls_share_the_existing_graphics_classes() {
    let registry = minimal_registry().unwrap();
    let mut session = GraphicsSession::new();
    let x = f64_matrix(1, 3, vec![1.0, 2.0, 3.0]);
    let y = f64_matrix(1, 3, vec![4.0, 5.0, 6.0]);
    let z = f64_matrix(1, 3, vec![7.0, 8.0, 10.0]);

    let (line, _) = invoke(
        &registry,
        &mut session,
        "plot3",
        &[x.clone(), y.clone(), z.clone(), text("--o")],
        1,
    );
    let Value::GraphicsArray(lines) = &line.unwrap()[0] else {
        panic!("plot3 must return a Line handle column")
    };
    let line = lines.as_slice()[0];
    assert!(matches!(
        session.object(line),
        Ok(GraphicsObject::LineSeries(properties))
            if properties.z_data.as_ref().is_some_and(|data| decode_f64(&session, data) == [7.0, 8.0, 10.0])
                && properties.line_style == LineStyle::Dash
                && properties.marker == Marker::Circle
    ));

    invoke(&registry, &mut session, "hold", &[text("on")], 0)
        .0
        .unwrap();
    let (scatter, _) = invoke(
        &registry,
        &mut session,
        "scatter3",
        &[
            x,
            y,
            z,
            Value::Double(25.0),
            f64_matrix(1, 3, vec![1.0, 0.0, 0.0]),
            text("filled"),
        ],
        1,
    );
    let scatter = handle(&scatter.unwrap()[0]);
    assert!(matches!(
        session.object(scatter),
        Ok(GraphicsObject::ScatterSeries(properties))
            if properties.z_data.as_ref().is_some_and(|data| decode_f64(&session, data) == [7.0, 8.0, 10.0])
                && properties.marker_size_points == 5.0
                && properties.face_color_rgba == [1.0, 0.0, 0.0, 1.0]
    ));

    invoke(&registry, &mut session, "axis", &[text("equal")], 0)
        .0
        .unwrap();
    invoke(
        &registry,
        &mut session,
        "daspect",
        &[f64_matrix(1, 3, vec![1.0, 2.0, 3.0])],
        0,
    )
    .0
    .unwrap();
    invoke(
        &registry,
        &mut session,
        "pbaspect",
        &[f64_matrix(1, 3, vec![2.0, 1.0, 1.0])],
        0,
    )
    .0
    .unwrap();
    let axes = session.current_axes().unwrap();
    assert!(matches!(
        session.object(axes),
        Ok(GraphicsObject::Axes2D(properties))
            if properties.view_azimuth_degrees == -37.5
                && properties.view_elevation_degrees == 30.0
                && properties.data_aspect_ratio == [1.0, 2.0, 3.0]
                && properties.data_aspect_ratio_mode == LimitMode::Manual
                && properties.plot_box_aspect_ratio == [2.0, 1.0, 1.0]
                && properties.plot_box_aspect_ratio_mode == LimitMode::Manual
    ));
}

#[test]
#[allow(clippy::float_cmp)]
fn polarplot_limits_ticks_labels_and_properties_form_one_real_service_loop() {
    let registry = minimal_registry().unwrap();
    let mut session = GraphicsSession::new();
    let theta = f64_matrix(1, 4, vec![0.0, 0.5, 1.0, 1.5]);
    let radius = f64_matrix(1, 4, vec![1.0, 1.25, 0.75, 1.0]);
    let (line, _) = invoke(
        &registry,
        &mut session,
        "polarplot",
        &[theta, radius, text("LineWidth"), Value::Double(2.0)],
        1,
    );
    let Value::GraphicsArray(lines) = &line.unwrap()[0] else {
        panic!("polarplot must return a Line handle column")
    };
    let line = lines.as_slice()[0];
    let axes = session.current_axes().unwrap();
    assert_eq!(axes.class(), GraphicsClass::PolarAxes);
    assert!(matches!(
        session.object(axes),
        Ok(GraphicsObject::Axes2D(properties))
            if properties.coordinate_system == AxesCoordinateSystem::Polar
                && properties.x_limits == [0.0, 360.0]
                && properties.y_limits == [0.0, 1.25]
                && properties.grid_x
                && properties.grid_y
                && properties.theta_direction == ThetaDirection::Counterclockwise
                && properties.theta_zero_location == ThetaZeroLocation::Right
    ));
    assert!(matches!(
        session.object(line),
        Ok(GraphicsObject::LineSeries(properties)) if properties.line_width_points == 2.0
    ));

    invoke(&registry, &mut session, "title", &[text("Polar Flower")], 1)
        .0
        .unwrap();
    invoke(
        &registry,
        &mut session,
        "thetalim",
        &[f64_matrix(1, 2, vec![0.0, 180.0])],
        0,
    )
    .0
    .unwrap();
    invoke(
        &registry,
        &mut session,
        "rlim",
        &[f64_matrix(1, 2, vec![0.0, 1.5])],
        0,
    )
    .0
    .unwrap();
    invoke(
        &registry,
        &mut session,
        "thetaticks",
        &[f64_matrix(1, 3, vec![0.0, 90.0, 180.0])],
        0,
    )
    .0
    .unwrap();
    invoke(
        &registry,
        &mut session,
        "thetaticklabels",
        &[text_cell(&["east", "north", "west"])],
        0,
    )
    .0
    .unwrap();

    let GraphicsResponse::PropertyValues(values) = session
        .execute(GraphicsRequest::GetProperties {
            handles: vec![line],
            property: GraphicsProperty::ThetaData,
        })
        .unwrap()
        .response
    else {
        panic!("ThetaData query must return one value")
    };
    assert!(matches!(
        values.as_slice(),
        [GraphicsPropertyValue::NumericMatrix { shape: [1, 4], values }]
            if values == &[0.0, 0.5, 1.0, 1.5]
    ));
    let Ok(GraphicsObject::Axes2D(properties)) = session.object(axes) else {
        panic!("PolarAxes must retain the shared axes payload")
    };
    assert_eq!(properties.x_limits, [0.0, 180.0]);
    assert_eq!(properties.y_limits, [0.0, 1.5]);
    assert_eq!(properties.x_ticks, [0.0, 90.0, 180.0]);
    assert_eq!(
        properties.x_tick_labels_utf16,
        ["east", "north", "west"].map(|value| value.encode_utf16().collect::<Vec<_>>())
    );
    assert!(properties.title.is_some());
}

fn f64_matrix(rows: u64, columns: u64, values: Vec<f64>) -> Value {
    let shape = Shape::new([rows, columns]).unwrap();
    Value::Array(ArrayData::F64(DenseArray::from_vec(shape, values).unwrap()))
}

fn f32_matrix(rows: u64, columns: u64, values: Vec<f32>) -> Value {
    let shape = Shape::new([rows, columns]).unwrap();
    Value::Array(ArrayData::F32(DenseArray::from_vec(shape, values).unwrap()))
}

fn text(value: &str) -> Value {
    Value::String(StringValue::Scalar(StringElement::from(value)))
}

fn text_cell(values: &[&str]) -> Value {
    let shape = Shape::new([1, values.len() as u64]).unwrap();
    Value::Cell(
        openmat_value::CellArray::from_values(shape, values.iter().copied().map(text).collect())
            .unwrap(),
    )
}

fn handle(value: &Value) -> openmat_graphics_model::GraphicsHandle {
    let Value::Graphics(handle) = value else {
        panic!("expected scalar graphics handle")
    };
    *handle
}

fn decode_f64(session: &GraphicsSession, reference: &openmat_graphics_model::DataRef) -> Vec<f64> {
    let resource = session.data_resource(reference.id).unwrap();
    resource
        .bytes()
        .chunks_exact(8)
        .map(|chunk| {
            f64::from_le_bytes([
                chunk[0], chunk[1], chunk[2], chunk[3], chunk[4], chunk[5], chunk[6], chunk[7],
            ])
        })
        .collect()
}

fn char_text(value: &Value) -> String {
    let Value::Array(ArrayData::Char(array)) = value else {
        panic!("expected char row")
    };
    String::from_utf16(
        &array
            .as_slice()
            .iter()
            .copied()
            .map(openmat_array::CharCodeUnit::get)
            .collect::<Vec<_>>(),
    )
    .unwrap()
}

fn f64_values(value: &Value) -> &[f64] {
    let Value::Array(ArrayData::F64(array)) = value else {
        panic!("expected double array")
    };
    array.as_slice()
}

fn f64_dimensions(value: &Value) -> &[u64] {
    let Value::Array(ArrayData::F64(array)) = value else {
        panic!("expected double array")
    };
    array.shape().dimensions()
}

fn u64_values(value: &Value) -> &[u64] {
    let Value::Array(ArrayData::Integer(array)) = value else {
        panic!("expected integer array")
    };
    array.as_typed::<u64>().unwrap().as_slice()
}

#[test]
fn standalone_context_reports_structured_missing_graphics_service() {
    let registry = minimal_registry().unwrap();
    let handle = registry.handle_by_name("gcf").unwrap();
    let cancellation = CancellationToken::new();
    let mut output = VecOutput::new();
    let mut context = BuiltinContext::new(1, &cancellation, &mut output);
    let BuiltinInvocationError::Failed { error, .. } = registry
        .invoke(handle, &[], &mut context)
        .expect_err("standalone graphics call must fail")
    else {
        panic!()
    };
    assert_eq!(error.category, BuiltinErrorCategory::HostService);
}

#[test]
fn gcf_gca_and_explicit_figure_follow_measured_identity_and_classes() {
    let registry = minimal_registry().unwrap();
    let mut session = GraphicsSession::new();
    let (figure_result, output) = invoke(&registry, &mut session, "gcf", &[], 1);
    let figure = handle(&figure_result.unwrap()[0]);
    assert_eq!(figure.class().class_name(), "matlab.ui.Figure");
    assert!(
        output
            .events()
            .iter()
            .any(|event| matches!(event, OutputEvent::GraphicsNotice(notice) if notice.discovery))
    );
    assert!(session.current_axes().is_none());

    let (axes_result, _) = invoke(&registry, &mut session, "gca", &[], 1);
    let axes = handle(&axes_result.unwrap()[0]);
    assert_eq!(axes.class().class_name(), "matlab.graphics.axis.Axes");

    let (seven, _) = invoke(&registry, &mut session, "figure", &[Value::Double(7.0)], 1);
    let seven = handle(&seven.unwrap()[0]);
    let (selected, _) = invoke(&registry, &mut session, "figure", &[Value::Double(7.0)], 1);
    assert_eq!(handle(&selected.unwrap()[0]), seven);
}

#[test]
fn no_argument_figure_creates_fresh_windows_while_gcf_reuses_current() {
    let registry = minimal_registry().unwrap();
    let mut session = GraphicsSession::new();
    let (first, _) = invoke(&registry, &mut session, "figure", &[], 1);
    let first = handle(&first.unwrap()[0]);
    let (second, _) = invoke(&registry, &mut session, "figure", &[], 1);
    let second = handle(&second.unwrap()[0]);
    assert_ne!(first, second);
    assert!(matches!(
        session.object(first),
        Ok(GraphicsObject::Figure(properties)) if properties.number == 1
    ));
    assert!(matches!(
        session.object(second),
        Ok(GraphicsObject::Figure(properties)) if properties.number == 2
    ));

    let (current, _) = invoke(&registry, &mut session, "gcf", &[], 1);
    assert_eq!(handle(&current.unwrap()[0]), second);
    let (selected, _) = invoke(&registry, &mut session, "figure", &[Value::Double(1.0)], 1);
    assert_eq!(handle(&selected.unwrap()[0]), first);
}

#[test]
#[allow(clippy::float_cmp)]
fn figure_constructor_applies_supported_name_value_properties_atomically() {
    let registry = minimal_registry().unwrap();
    let mut session = GraphicsSession::new();
    let (created, output) = invoke(
        &registry,
        &mut session,
        "figure",
        &[
            text("Color"),
            text("w"),
            text("Name"),
            text("Möbius"),
            text("Visible"),
            text("off"),
        ],
        1,
    );
    let figure = handle(&created.unwrap()[0]);
    assert!(matches!(
        session.object(figure),
        Ok(GraphicsObject::Figure(properties))
            if properties.number == 1
                && properties.name_utf16 == "Möbius".encode_utf16().collect::<Vec<_>>()
                && !properties.visible
                && properties.background_rgba == [1.0, 1.0, 1.0, 1.0]
    ));
    assert!(
        output
            .events()
            .iter()
            .any(|event| matches!(event, OutputEvent::GraphicsNotice(notice) if notice.discovery))
    );
}

#[test]
#[allow(clippy::float_cmp)]
fn figure_number_title_position_and_invalid_position_are_atomic() {
    let registry = minimal_registry().unwrap();
    let mut session = GraphicsSession::new();
    let (result, _) = invoke(
        &registry,
        &mut session,
        "figure",
        &[
            text("NumberTitle"),
            text("off"),
            text("Position"),
            f64_matrix(1, 4, vec![10.0, 20.0, 300.0, 200.0]),
        ],
        1,
    );
    let figure = handle(&result.unwrap()[0]);
    assert!(matches!(
        session.object(figure),
        Ok(GraphicsObject::Figure(properties))
            if !properties.number_title
                && properties.position_css_pixels == [10.0, 20.0, 300.0, 200.0]
    ));

    let mut session = GraphicsSession::new();
    let (result, output) = invoke(
        &registry,
        &mut session,
        "figure",
        &[
            text("Position"),
            f64_matrix(1, 4, vec![10.0, 20.0, 0.0, 200.0]),
        ],
        1,
    );
    assert_eq!(result.unwrap_err().category, BuiltinErrorCategory::Type);
    assert_eq!(session.object_count(), 0);
    assert!(output.events().is_empty());

    let mut session = GraphicsSession::new();
    let (result, _) = invoke(
        &registry,
        &mut session,
        "figure",
        &[Value::Double(7.0), text("Color"), text("w")],
        1,
    );
    assert_eq!(
        result.unwrap_err().category,
        BuiltinErrorCategory::ArgumentCount
    );
    assert_eq!(session.object_count(), 0);
}

#[test]
fn close_all_and_supported_modifiers_close_every_representable_figure() {
    let registry = minimal_registry().unwrap();
    let mut session = GraphicsSession::new();
    let first = handle(&invoke(&registry, &mut session, "figure", &[], 1).0.unwrap()[0]);
    let second = handle(&invoke(&registry, &mut session, "figure", &[], 1).0.unwrap()[0]);
    session.take_pending_deltas();

    let (result, output) = invoke(&registry, &mut session, "close", &[text("all")], 0);
    result.unwrap();
    assert!(output.events().is_empty());
    assert!(!session.is_valid(first));
    assert!(!session.is_valid(second));
    assert!(session.current_figure().is_none());
    assert_eq!(session.take_pending_deltas().len(), 2);

    let third = handle(&invoke(&registry, &mut session, "figure", &[], 1).0.unwrap()[0]);
    let fourth = handle(&invoke(&registry, &mut session, "figure", &[], 1).0.unwrap()[0]);
    session.take_pending_deltas();
    invoke(
        &registry,
        &mut session,
        "close",
        &[text("all"), text("hidden"), text("force")],
        0,
    )
    .0
    .unwrap();
    assert!(!session.is_valid(third));
    assert!(!session.is_valid(fourth));
    assert!(session.current_figure().is_none());
    assert_eq!(session.take_pending_deltas().len(), 2);

    invoke(&registry, &mut session, "close", &[text("all")], 0)
        .0
        .unwrap();
}

#[test]
fn graphics_class_returns_a_matlab_char_row() {
    let registry = minimal_registry().unwrap();
    let mut session = GraphicsSession::new();
    let (figure, _) = invoke(&registry, &mut session, "figure", &[], 1);
    let figure = figure.unwrap().remove(0);
    let (class, _) = invoke(&registry, &mut session, "class", &[figure], 1);
    assert_eq!(char_text(&class.unwrap()[0]), "matlab.ui.Figure");
}

#[test]
fn matrix_plot_returns_line_column_and_copies_columns_to_independent_resources() {
    let registry = minimal_registry().unwrap();
    let mut session = GraphicsSession::new();
    let y = f64_matrix(3, 2, vec![1.0, 3.0, 5.0, 2.0, 4.0, 6.0]);
    let (result, output) = invoke(&registry, &mut session, "plot", &[y], 1);
    let Value::GraphicsArray(handles) = &result.unwrap()[0] else {
        panic!("matrix plot returns a homogeneous graphics row")
    };
    assert_eq!(handles.shape().dimensions(), &[2, 1]);
    assert_eq!(handles.class(), GraphicsClass::LineSeries);
    assert_eq!(
        handles.class().class_name(),
        "matlab.graphics.chart.primitive.Line"
    );
    assert_eq!(output.events().len(), 1);

    let first = session.object(handles.as_slice()[0]).unwrap();
    let second = session.object(handles.as_slice()[1]).unwrap();
    let GraphicsObject::LineSeries(first) = first else {
        panic!()
    };
    let GraphicsObject::LineSeries(second) = second else {
        panic!()
    };
    assert_eq!(decode_f64(&session, &first.x_data), vec![1.0, 2.0, 3.0]);
    assert_eq!(decode_f64(&session, &first.y_data), vec![1.0, 3.0, 5.0]);
    assert_eq!(decode_f64(&session, &second.x_data), vec![1.0, 2.0, 3.0]);
    assert_eq!(decode_f64(&session, &second.y_data), vec![2.0, 4.0, 6.0]);
    assert_ne!(first.x_data.id, second.x_data.id);
}

#[test]
fn vector_x_matrix_y_and_matrix_pairs_follow_column_rules() {
    let registry = minimal_registry().unwrap();
    let mut session = GraphicsSession::new();
    let x = f64_matrix(3, 1, vec![10.0, 20.0, 30.0]);
    let y = f64_matrix(3, 2, vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    let (result, _) = invoke(&registry, &mut session, "plot", &[x, y], 1);
    let Value::GraphicsArray(handles) = &result.unwrap()[0] else {
        panic!()
    };
    assert_eq!(handles.numel(), 2);
    for handle in handles.as_slice() {
        let GraphicsObject::LineSeries(series) = session.object(*handle).unwrap() else {
            panic!()
        };
        assert_eq!(decode_f64(&session, &series.x_data), vec![10.0, 20.0, 30.0]);
    }

    let x = f64_matrix(3, 2, vec![1.0, 2.0, 3.0, 11.0, 12.0, 13.0]);
    let y = f64_matrix(3, 2, vec![4.0, 5.0, 6.0, 14.0, 15.0, 16.0]);
    let (result, _) = invoke(&registry, &mut session, "plot", &[x, y], 1);
    let Value::GraphicsArray(handles) = &result.unwrap()[0] else {
        panic!()
    };
    let GraphicsObject::LineSeries(second) = session.object(handles.as_slice()[1]).unwrap() else {
        panic!()
    };
    assert_eq!(decode_f64(&session, &second.x_data), vec![11.0, 12.0, 13.0]);
    assert_eq!(decode_f64(&session, &second.y_data), vec![14.0, 15.0, 16.0]);
}

#[test]
#[allow(clippy::float_cmp)]
fn surf_view_zlim_and_zlabel_form_a_matlab_visible_3d_loop() {
    let registry = minimal_registry().unwrap();
    let mut session = GraphicsSession::new();
    let z = f32_matrix(2, 3, vec![1.0, 2.0, 2.0, 5.0, 3.0, 4.0]);
    let (surface, output) = invoke(&registry, &mut session, "surf", &[z], 1);
    let surface = handle(&surface.unwrap()[0]);
    assert_eq!(surface.class(), GraphicsClass::SurfaceSeries);
    let GraphicsObject::SurfaceSeries(properties) = session.object(surface).unwrap() else {
        panic!()
    };
    assert_eq!(properties.x_data.shape, [1, 3]);
    assert_eq!(properties.y_data.shape, [2, 1]);
    assert_eq!(properties.z_data.shape, [2, 3]);
    assert_eq!(properties.z_data.dtype, DataDType::F32);
    assert_eq!(properties.c_data.dtype, DataDType::F32);
    assert_eq!(properties.face_color, SurfaceColor::Flat);
    assert_eq!(
        properties.edge_color,
        SurfaceColor::Rgba([0.0, 0.0, 0.0, 1.0])
    );
    assert_eq!(properties.c_data_mapping, CDataMapping::Scaled);
    assert_eq!(output.events().len(), 1);

    let axes = session.current_axes().unwrap();
    let GraphicsObject::Axes2D(axes_properties) = session.object(axes).unwrap() else {
        panic!()
    };
    assert_eq!(axes_properties.view_azimuth_degrees, -37.5);
    assert_eq!(axes_properties.view_elevation_degrees, 30.0);
    assert_eq!(axes_properties.projection, ProjectionMode::Orthographic);
    assert_eq!(axes_properties.z_limits, [1.0, 5.0]);

    invoke(
        &registry,
        &mut session,
        "view",
        &[Value::Double(45.0), Value::Double(30.0)],
        0,
    )
    .0
    .unwrap();
    let (angles, _) = invoke(&registry, &mut session, "view", &[], 2);
    assert_eq!(
        angles.unwrap(),
        vec![Value::Double(45.0), Value::Double(30.0)]
    );

    invoke(
        &registry,
        &mut session,
        "zlim",
        &[f64_matrix(1, 2, vec![0.0, 10.0])],
        0,
    )
    .0
    .unwrap();
    let (limits, _) = invoke(&registry, &mut session, "zlim", &[], 1);
    assert_eq!(f64_values(&limits.unwrap()[0]), &[0.0, 10.0]);
    let (label, _) = invoke(&registry, &mut session, "zlabel", &[text("Height")], 1);
    assert_eq!(handle(&label.unwrap()[0]).class(), GraphicsClass::Text);
}

#[test]
fn mesh_preserves_matlab_default_face_and_edge_modes() {
    let registry = minimal_registry().unwrap();
    let mut session = GraphicsSession::new();
    let (surface, _) = invoke(
        &registry,
        &mut session,
        "mesh",
        &[f64_matrix(2, 2, vec![1.0, 2.0, 3.0, 4.0])],
        1,
    );
    let GraphicsObject::SurfaceSeries(properties) =
        session.object(handle(&surface.unwrap()[0])).unwrap()
    else {
        panic!()
    };
    assert_eq!(
        properties.face_color,
        SurfaceColor::Rgba([1.0, 1.0, 1.0, 1.0])
    );
    assert_eq!(properties.edge_color, SurfaceColor::Flat);
}

#[test]
fn matrix_surface_shading_and_colorbar_form_one_r2022b_visible_loop() {
    let registry = minimal_registry().unwrap();
    let mut session = GraphicsSession::new();
    let x = f64_matrix(2, 2, vec![-1.0, -1.0, 1.0, 1.0]);
    let y = f64_matrix(2, 2, vec![-1.0, 1.0, -1.0, 1.0]);
    let z = f64_matrix(2, 2, vec![1.0, 2.0, 3.0, 4.0]);
    let (surface, _) = invoke(&registry, &mut session, "surf", &[x, y, z], 1);
    let surface = handle(&surface.unwrap()[0]);
    let GraphicsObject::SurfaceSeries(properties) = session.object(surface).unwrap() else {
        panic!()
    };
    assert_eq!(properties.x_data.shape, [2, 2]);
    assert_eq!(properties.y_data.shape, [2, 2]);

    invoke(&registry, &mut session, "shading", &[text("interp")], 0)
        .0
        .unwrap();
    let GraphicsObject::SurfaceSeries(properties) = session.object(surface).unwrap() else {
        panic!()
    };
    assert_eq!(properties.face_color, SurfaceColor::Interp);
    assert_eq!(properties.edge_color, SurfaceColor::None);

    let (first_colorbar, _) = invoke(&registry, &mut session, "colorbar", &[], 1);
    let first_colorbar = handle(&first_colorbar.unwrap()[0]);
    assert_eq!(first_colorbar.class(), GraphicsClass::ColorBar);
    assert_eq!(
        first_colorbar.class().class_name(),
        "matlab.graphics.illustration.ColorBar"
    );
    let (second_colorbar, _) = invoke(&registry, &mut session, "colorbar", &[], 1);
    let second_colorbar = handle(&second_colorbar.unwrap()[0]);
    assert!(!session.is_valid(first_colorbar));
    assert!(session.is_valid(second_colorbar));
    let figure = session.current_figure().unwrap();
    let (parent, _) = invoke(
        &registry,
        &mut session,
        "get",
        &[Value::Graphics(second_colorbar), text("Parent")],
        1,
    );
    assert_eq!(handle(&parent.unwrap()[0]), figure);
    let GraphicsObject::Axes2D(axes) = session.object(session.current_axes().unwrap()).unwrap()
    else {
        panic!()
    };
    assert!(axes.colorbar_visible);

    invoke(&registry, &mut session, "colorbar", &[text("off")], 0)
        .0
        .unwrap();
    assert!(!session.is_valid(second_colorbar));
    let GraphicsObject::Axes2D(axes) = session.object(session.current_axes().unwrap()).unwrap()
    else {
        panic!()
    };
    assert!(!axes.colorbar_visible);
}

#[test]
fn empty_scalar_nan_and_single_plot_forms_preserve_measured_basics() {
    let registry = minimal_registry().unwrap();
    let mut session = GraphicsSession::new();
    let empty = f64_matrix(0, 0, Vec::new());
    let (result, _) = invoke(&registry, &mut session, "plot", &[empty], 1);
    let Value::GraphicsArray(handles) = &result.unwrap()[0] else {
        panic!()
    };
    assert_eq!(handles.shape().dimensions(), &[0, 1]);
    assert!(
        session
            .children(session.current_axes().unwrap())
            .unwrap()
            .is_empty()
    );

    let (result, _) = invoke(&registry, &mut session, "plot", &[Value::Double(1.0)], 1);
    let Value::GraphicsArray(handles) = &result.unwrap()[0] else {
        panic!()
    };
    let GraphicsObject::LineSeries(series) = session.object(handles.as_slice()[0]).unwrap() else {
        panic!()
    };
    assert_eq!(decode_f64(&session, &series.x_data), vec![1.0]);
    assert_eq!(decode_f64(&session, &series.y_data), vec![1.0]);

    let x = f32_matrix(1, 3, vec![1.0, f32::NAN, 3.0]);
    let y = f32_matrix(1, 3, vec![4.0, 5.0, 6.0]);
    let (result, _) = invoke(&registry, &mut session, "plot", &[x, y], 1);
    let Value::GraphicsArray(handles) = &result.unwrap()[0] else {
        panic!()
    };
    let GraphicsObject::LineSeries(series) = session.object(handles.as_slice()[0]).unwrap() else {
        panic!()
    };
    assert_eq!(series.x_data.dtype, DataDType::F32);
    assert_eq!(series.y_data.dtype, DataDType::F32);
    let resource = session.data_resource(series.x_data.id).unwrap();
    assert_eq!(&resource.bytes()[4..8], &f32::NAN.to_bits().to_le_bytes());
}

#[test]
#[allow(clippy::float_cmp)]
fn hold_text_legend_limits_and_lifecycle_form_one_real_service_loop() {
    let registry = minimal_registry().unwrap();
    let mut session = GraphicsSession::new();
    let (initial, _) = invoke(&registry, &mut session, "ishold", &[], 1);
    assert_eq!(initial.unwrap(), vec![Value::Double(0.0)]);
    assert!(session.current_axes().is_none());

    invoke(&registry, &mut session, "hold", &[text("on")], 0)
        .0
        .unwrap();
    let axes = session.current_axes().unwrap();
    assert!(
        matches!(session.object(axes), Ok(GraphicsObject::Axes2D(value)) if value.next_plot == NextPlot::Add)
    );
    invoke(
        &registry,
        &mut session,
        "plot",
        &[f64_matrix(1, 2, vec![1.0, 2.0])],
        1,
    )
    .0
    .unwrap();
    invoke(
        &registry,
        &mut session,
        "plot",
        &[f64_matrix(1, 2, vec![3.0, 4.0])],
        1,
    )
    .0
    .unwrap();
    assert_eq!(session.children(axes).unwrap().len(), 2);

    let (title, _) = invoke(&registry, &mut session, "title", &[text("测量")], 1);
    assert_eq!(handle(&title.unwrap()[0]).class(), GraphicsClass::Text);
    let (legend, _) = invoke(
        &registry,
        &mut session,
        "legend",
        &[text("a"), text("b")],
        1,
    );
    assert_eq!(handle(&legend.unwrap()[0]).class(), GraphicsClass::Legend);

    invoke(
        &registry,
        &mut session,
        "xlim",
        &[f64_matrix(1, 2, vec![0.0, 10.0])],
        1,
    )
    .0
    .unwrap();
    let GraphicsObject::Axes2D(properties) = session.object(axes).unwrap() else {
        panic!()
    };
    assert_eq!(properties.x_limit_mode, LimitMode::Manual);
    assert_eq!(properties.x_limits, [0.0, 10.0]);

    invoke(&registry, &mut session, "cla", &[], 0).0.unwrap();
    assert!(session.is_valid(axes));
    assert!(session.children(axes).unwrap().is_empty());
    assert!(
        matches!(session.object(axes), Ok(GraphicsObject::Axes2D(value)) if value.next_plot == NextPlot::Add)
    );
    invoke(&registry, &mut session, "clf", &[], 0).0.unwrap();
    assert!(!session.is_valid(axes));
    let figure = session.current_figure().unwrap();
    invoke(
        &registry,
        &mut session,
        "close",
        &[Value::Graphics(figure)],
        0,
    )
    .0
    .unwrap();
    assert!(!session.is_valid(figure));
}

#[test]
fn invalid_plot_shape_is_rejected_before_graphics_mutation() {
    let registry = minimal_registry().unwrap();
    let mut session = GraphicsSession::new();
    let x = f64_matrix(1, 2, vec![1.0, 2.0]);
    let y = f64_matrix(1, 3, vec![1.0, 2.0, 3.0]);
    let (result, output) = invoke(&registry, &mut session, "plot", &[x, y], 1);
    assert_eq!(result.unwrap_err().category, BuiltinErrorCategory::Domain);
    assert_eq!(session.object_count(), 0);
    assert!(output.events().is_empty());
}

#[test]
fn model_query_remains_available_to_host_adapter_without_wire_token() {
    let mut session = GraphicsSession::new();
    let execution = session.execute(GraphicsRequest::CurrentFigure).unwrap();
    let GraphicsResponse::Handle(figure) = execution.response else {
        panic!()
    };
    assert!(execution.notice.unwrap().discovery);
    assert_eq!(figure.class(), GraphicsClass::Figure);
    let limits = session
        .execute(GraphicsRequest::GetLimits {
            axes: None,
            axis: Axis::X,
        })
        .unwrap();
    assert_eq!(limits.response, GraphicsResponse::Limits([0.0, 1.0]));
}

#[test]
#[allow(clippy::float_cmp)]
fn plot_line_spec_and_name_value_properties_commit_atomically() {
    let registry = minimal_registry().unwrap();
    let mut session = GraphicsSession::new();
    let x = f64_matrix(1, 3, vec![1.0, 2.0, 3.0]);
    let y = f64_matrix(1, 3, vec![4.0, 5.0, 6.0]);
    let color = f64_matrix(1, 3, vec![0.2, 0.4, 0.6]);
    let (result, output) = invoke(
        &registry,
        &mut session,
        "plot",
        &[
            x,
            y,
            text("--o"),
            text("LineWidth"),
            Value::Double(2.0),
            text("Color"),
            color,
            text("MarkerSize"),
            Value::Double(8.0),
        ],
        1,
    );
    let Value::GraphicsArray(handles) = &result.unwrap()[0] else {
        panic!()
    };
    let line = handles.as_slice()[0];
    assert_eq!(line.class(), GraphicsClass::LineSeries);
    assert_eq!(output.events().len(), 1);
    assert!(matches!(
        session.object(line),
        Ok(GraphicsObject::LineSeries(value))
            if value.line_style == LineStyle::Dash
                && value.marker == Marker::Circle
                && value.line_width_points == 2.0
                && value.marker_size_points == 8.0
                && value.color_rgba == [0.2, 0.4, 0.6, 1.0]
    ));

    let (style, _) = invoke(
        &registry,
        &mut session,
        "get",
        &[Value::Graphics(line), text("LineStyle")],
        1,
    );
    assert_eq!(char_text(&style.unwrap()[0]), "--");
    let (rgb, _) = invoke(
        &registry,
        &mut session,
        "get",
        &[Value::Graphics(line), text("Color")],
        1,
    );
    assert_eq!(f64_values(&rgb.unwrap()[0]), &[0.2, 0.4, 0.6]);
}

#[test]
fn line_spec_and_marker_property_cover_protocol_marker_shapes() {
    let registry = minimal_registry().unwrap();
    let cases = [
        (".", Marker::Point),
        ("o", Marker::Circle),
        ("+", Marker::Plus),
        ("*", Marker::Star),
        ("x", Marker::Cross),
        ("s", Marker::Square),
        ("d", Marker::Diamond),
        ("^", Marker::TriangleUp),
        ("v", Marker::TriangleDown),
        (">", Marker::TriangleRight),
        ("<", Marker::TriangleLeft),
    ];
    for (spec, expected) in cases {
        let mut session = GraphicsSession::new();
        let (result, _) = invoke(
            &registry,
            &mut session,
            "plot",
            &[f64_matrix(1, 2, vec![1.0, 2.0]), text(spec)],
            1,
        );
        let Value::GraphicsArray(handles) = &result.unwrap()[0] else {
            panic!()
        };
        assert!(matches!(
            session.object(handles.as_slice()[0]),
            Ok(GraphicsObject::LineSeries(value)) if value.marker == expected
        ));
    }

    let mut session = GraphicsSession::new();
    let (line, _) = invoke(
        &registry,
        &mut session,
        "plot",
        &[f64_matrix(1, 1, vec![1.0])],
        1,
    );
    let Value::GraphicsArray(handles) = &line.unwrap()[0] else {
        panic!()
    };
    let line = handles.as_slice()[0];
    invoke(
        &registry,
        &mut session,
        "set",
        &[Value::Graphics(line), text("Marker"), text("triangleleft")],
        0,
    )
    .0
    .unwrap();
    assert!(matches!(
        session.object(line),
        Ok(GraphicsObject::LineSeries(value)) if value.marker == Marker::TriangleLeft
    ));
}

#[test]
fn set_updates_handle_columns_once_and_get_returns_a_matching_cell_column() {
    let registry = minimal_registry().unwrap();
    let mut session = GraphicsSession::new();
    let (result, _) = invoke(
        &registry,
        &mut session,
        "plot",
        &[f64_matrix(2, 2, vec![1.0, 2.0, 3.0, 4.0])],
        1,
    );
    let Value::GraphicsArray(handles) = &result.unwrap()[0] else {
        panic!()
    };
    let handles = handles.clone();
    assert_eq!(handles.class(), GraphicsClass::LineSeries);
    session.take_pending_deltas();
    let (set_result, output) = invoke(
        &registry,
        &mut session,
        "set",
        &[
            Value::GraphicsArray(handles.clone()),
            text("LineWidth"),
            Value::Double(3.0),
            text("Visible"),
            text("off"),
            text("Marker"),
            text("square"),
        ],
        0,
    );
    assert!(set_result.unwrap().is_empty());
    assert_eq!(output.events().len(), 1);
    assert_eq!(handles.class(), GraphicsClass::LineSeries);
    let deltas = session.take_pending_deltas();
    assert_eq!(deltas.len(), 1);
    assert_eq!(
        deltas[0]
            .operations
            .iter()
            .filter(|operation| matches!(operation, GraphicsDeltaOperation::UpsertObject(_)))
            .count(),
        2
    );

    let (get_result, output) = invoke(
        &registry,
        &mut session,
        "get",
        &[Value::GraphicsArray(handles), text("LineWidth")],
        1,
    );
    assert!(output.events().is_empty());
    let Value::Cell(values) = &get_result.unwrap()[0] else {
        panic!("handle-column get returns a cell column")
    };
    assert_eq!(values.shape().dimensions(), &[2, 1]);
    assert_eq!(values.values(), &[Value::Double(3.0), Value::Double(3.0)]);
}

#[test]
#[allow(clippy::float_cmp)]
fn scatter_scalar_size_color_marker_and_visibility_round_trip() {
    let registry = minimal_registry().unwrap();
    let mut session = GraphicsSession::new();
    let (scatter, _) = invoke(
        &registry,
        &mut session,
        "scatter",
        &[
            f64_matrix(1, 2, vec![1.0, 2.0]),
            f64_matrix(1, 2, vec![3.0, 4.0]),
        ],
        1,
    );
    let scatter = handle(&scatter.unwrap()[0]);
    invoke(
        &registry,
        &mut session,
        "set",
        &[
            Value::Graphics(scatter),
            text("SizeData"),
            Value::Double(49.0),
            text("CData"),
            f64_matrix(1, 3, vec![0.25, 0.5, 0.75]),
            text("Marker"),
            text("diamond"),
            text("Visible"),
            text("OFF"),
        ],
        0,
    )
    .0
    .unwrap();
    assert!(matches!(
        session.object(scatter),
        Ok(GraphicsObject::ScatterSeries(value))
            if value.marker_size_points == 7.0
                && value.face_color_rgba == [0.25, 0.5, 0.75, 1.0]
                && value.edge_color_rgba == [0.25, 0.5, 0.75, 1.0]
                && value.marker == Marker::Diamond
                && !value.visible
    ));
    let (size, _) = invoke(
        &registry,
        &mut session,
        "get",
        &[Value::Graphics(scatter), text("SizeData")],
        1,
    );
    assert_eq!(size.unwrap(), vec![Value::Double(49.0)]);
    let (color, _) = invoke(
        &registry,
        &mut session,
        "get",
        &[Value::Graphics(scatter), text("CData")],
        1,
    );
    assert_eq!(f64_values(&color.unwrap()[0]), &[0.25, 0.5, 0.75]);
}

#[test]
fn property_diagnostics_are_structured_and_fail_before_partial_mutation() {
    let registry = minimal_registry().unwrap();
    let mut session = GraphicsSession::new();
    let (line, _) = invoke(
        &registry,
        &mut session,
        "plot",
        &[f64_matrix(1, 2, vec![1.0, 2.0])],
        1,
    );
    let Value::GraphicsArray(handles) = &line.unwrap()[0] else {
        panic!()
    };
    let line = handles.as_slice()[0];
    let figure = session.current_figure().unwrap();
    let before = session.snapshot(figure).unwrap();
    session.take_pending_deltas();

    let (invalid_color, output) = invoke(
        &registry,
        &mut session,
        "set",
        &[
            Value::Graphics(line),
            text("LineWidth"),
            Value::Double(2.0),
            text("Color"),
            f64_matrix(1, 3, vec![2.0, 0.0, 0.0]),
        ],
        0,
    );
    assert_eq!(
        invalid_color.unwrap_err().category,
        BuiltinErrorCategory::Domain
    );
    assert!(output.events().is_empty());
    assert_eq!(session.snapshot(figure).unwrap(), before);
    assert!(session.take_pending_deltas().is_empty());

    let (wrong_type, _) = invoke(
        &registry,
        &mut session,
        "set",
        &[Value::Graphics(line), text("LineWidth"), text("wide")],
        0,
    );
    assert_eq!(wrong_type.unwrap_err().category, BuiltinErrorCategory::Type);
    let (unknown, _) = invoke(
        &registry,
        &mut session,
        "get",
        &[Value::Graphics(line), text("DefinitelyUnknown")],
        1,
    );
    assert_eq!(
        unknown.unwrap_err().category,
        BuiltinErrorCategory::Graphics
    );
    let (zero_width, _) = invoke(
        &registry,
        &mut session,
        "set",
        &[Value::Graphics(line), text("LineWidth"), Value::Double(0.0)],
        0,
    );
    assert_eq!(zero_width.unwrap_err().category, BuiltinErrorCategory::Type);

    let axes = session.current_axes().unwrap();
    let (wrong_class, _) = invoke(
        &registry,
        &mut session,
        "set",
        &[
            Value::Graphics(axes),
            text("MarkerSize"),
            Value::Double(1.0),
        ],
        0,
    );
    assert_eq!(
        wrong_class.unwrap_err().category,
        BuiltinErrorCategory::Graphics
    );
}

#[test]
fn invalid_line_spec_and_stale_handle_never_mutate_graphics_state() {
    let registry = minimal_registry().unwrap();
    let mut empty_session = GraphicsSession::new();
    let (invalid_spec, output) = invoke(
        &registry,
        &mut empty_session,
        "plot",
        &[f64_matrix(1, 1, vec![1.0]), text("--oo")],
        1,
    );
    assert_eq!(
        invalid_spec.unwrap_err().category,
        BuiltinErrorCategory::Domain
    );
    assert_eq!(empty_session.object_count(), 0);
    assert!(output.events().is_empty());

    let mut session = GraphicsSession::new();
    let (line, _) = invoke(
        &registry,
        &mut session,
        "plot",
        &[f64_matrix(1, 1, vec![1.0])],
        1,
    );
    let Value::GraphicsArray(handles) = &line.unwrap()[0] else {
        panic!()
    };
    let stale = handles.as_slice()[0];
    let figure = session.current_figure().unwrap();
    invoke(
        &registry,
        &mut session,
        "close",
        &[Value::Graphics(figure)],
        0,
    )
    .0
    .unwrap();
    let (result, output) = invoke(
        &registry,
        &mut session,
        "set",
        &[Value::Graphics(stale), text("Visible"), text("off")],
        0,
    );
    assert_eq!(result.unwrap_err().category, BuiltinErrorCategory::Graphics);
    assert!(output.events().is_empty());
    assert_eq!(session.object_count(), 0);
}

#[test]
fn marker_indices_default_uint64_and_explicit_double_round_trip_atomically() {
    let registry = minimal_registry().unwrap();
    let mut session = GraphicsSession::new();
    let (result, _) = invoke(
        &registry,
        &mut session,
        "plot",
        &[
            f64_matrix(1, 6, vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]),
            text("o"),
        ],
        1,
    );
    let Value::GraphicsArray(lines) = &result.unwrap()[0] else {
        panic!()
    };
    let line = lines.as_slice()[0];
    let (default_indices, _) = invoke(
        &registry,
        &mut session,
        "get",
        &[Value::Graphics(line), text("MarkerIndices")],
        1,
    );
    assert_eq!(
        u64_values(&default_indices.unwrap()[0]),
        &[1, 2, 3, 4, 5, 6]
    );
    invoke(
        &registry,
        &mut session,
        "set",
        &[
            Value::Graphics(line),
            text("MarkerIndices"),
            f64_matrix(3, 1, vec![5.0, 2.0, 9.0]),
        ],
        0,
    )
    .0
    .unwrap();
    let (explicit_indices, _) = invoke(
        &registry,
        &mut session,
        "get",
        &[Value::Graphics(line), text("MarkerIndices")],
        1,
    );
    assert_eq!(f64_values(&explicit_indices.unwrap()[0]), &[5.0, 2.0, 9.0]);
    assert!(matches!(
        session.object(line),
        Ok(GraphicsObject::LineSeries(properties))
            if properties.marker_indices == [5, 2, 9]
                && !properties.marker_indices_automatic
    ));

    let figure = session.current_figure().unwrap();
    let before = session.snapshot(figure).unwrap();
    session.take_pending_deltas();
    let (invalid, output) = invoke(
        &registry,
        &mut session,
        "set",
        &[
            Value::Graphics(line),
            text("MarkerIndices"),
            f64_matrix(1, 3, vec![1.0, f64::INFINITY, 2.0]),
        ],
        0,
    );
    assert_eq!(invalid.unwrap_err().category, BuiltinErrorCategory::Type);
    assert_eq!(session.snapshot(figure).unwrap(), before);
    assert!(session.take_pending_deltas().is_empty());
    assert!(output.events().is_empty());
}

#[test]
#[allow(clippy::float_cmp)]
fn axes_tick_label_and_style_properties_share_the_generic_set_get_path() {
    let registry = minimal_registry().unwrap();
    let mut session = GraphicsSession::new();
    let (axes, _) = invoke(&registry, &mut session, "gca", &[], 1);
    let axes = handle(&axes.unwrap()[0]);
    invoke(
        &registry,
        &mut session,
        "set",
        &[
            Value::Graphics(axes),
            text("XTick"),
            f64_matrix(3, 1, vec![0.0, 5.0, 10.0]),
            text("XTickLabel"),
            text_cell(&["A", "B"]),
            text("TickLabelInterpreter"),
            text("latex"),
            text("FontSize"),
            Value::Double(12.0),
            text("LineWidth"),
            Value::Double(1.0),
            text("Box"),
            text("on"),
        ],
        0,
    )
    .0
    .unwrap();
    let GraphicsObject::Axes2D(properties) = session.object(axes).unwrap() else {
        panic!()
    };
    assert_eq!(properties.x_ticks, [0.0, 5.0, 10.0]);
    assert_eq!(properties.x_tick_mode, TickMode::Manual);
    assert_eq!(properties.x_tick_label_mode, TickMode::Manual);
    assert_eq!(properties.x_tick_labels_utf16.len(), 2);
    assert_eq!(properties.tick_label_interpreter, Interpreter::Latex);
    assert_eq!(properties.font_size_points, 12.0);
    assert_eq!(properties.line_width_points, 1.0);
    assert!(properties.box_enabled);

    let (labels, _) = invoke(
        &registry,
        &mut session,
        "get",
        &[Value::Graphics(axes), text("XTickLabel")],
        1,
    );
    let Value::Cell(labels) = &labels.unwrap()[0] else {
        panic!()
    };
    assert_eq!(labels.shape().dimensions(), &[2, 1]);
    let (interpreter, _) = invoke(
        &registry,
        &mut session,
        "get",
        &[Value::Graphics(axes), text("TickLabelInterpreter")],
        1,
    );
    assert_eq!(char_text(&interpreter.unwrap()[0]), "latex");
}

#[test]
#[allow(clippy::float_cmp)]
fn axes_color_round_trips_rgb_and_none_through_set_get() {
    let registry = minimal_registry().unwrap();
    let mut session = GraphicsSession::new();
    let (axes, _) = invoke(&registry, &mut session, "gca", &[], 1);
    let axes = handle(&axes.unwrap()[0]);

    invoke(
        &registry,
        &mut session,
        "set",
        &[
            Value::Graphics(axes),
            text("Color"),
            f64_matrix(1, 3, vec![0.25, 0.5, 0.75]),
        ],
        0,
    )
    .0
    .unwrap();
    let (color, _) = invoke(
        &registry,
        &mut session,
        "get",
        &[Value::Graphics(axes), text("Color")],
        1,
    );
    assert_eq!(color.unwrap()[0], f64_matrix(1, 3, vec![0.25, 0.5, 0.75]));

    invoke(
        &registry,
        &mut session,
        "set",
        &[Value::Graphics(axes), text("Color"), text("none")],
        0,
    )
    .0
    .unwrap();
    let (transparent, _) = invoke(
        &registry,
        &mut session,
        "get",
        &[Value::Graphics(axes), text("Color")],
        1,
    );
    assert_eq!(char_text(&transparent.unwrap()[0]), "none");
}

#[test]
fn text_and_legend_interpreter_name_value_parsing_preserves_labels() {
    let registry = minimal_registry().unwrap();
    let mut session = GraphicsSession::new();
    let (title, _) = invoke(
        &registry,
        &mut session,
        "title",
        &[text("$x$"), text("Interpreter"), text("latex")],
        1,
    );
    let title = handle(&title.unwrap()[0]);
    assert!(matches!(
        session.object(title),
        Ok(GraphicsObject::Text(properties)) if properties.interpreter == Interpreter::Latex
    ));

    invoke(
        &registry,
        &mut session,
        "plot",
        &[f64_matrix(1, 2, vec![1.0, 2.0])],
        1,
    )
    .0
    .unwrap();
    invoke(&registry, &mut session, "hold", &[text("on")], 0)
        .0
        .unwrap();
    invoke(
        &registry,
        &mut session,
        "plot",
        &[f64_matrix(1, 2, vec![2.0, 1.0])],
        1,
    )
    .0
    .unwrap();
    let (legend, _) = invoke(
        &registry,
        &mut session,
        "legend",
        &[
            text("first"),
            text("second"),
            text("Interpreter"),
            text("none"),
            text("Location"),
            text("best"),
        ],
        1,
    );
    let legend = handle(&legend.unwrap()[0]);
    assert!(matches!(
        session.object(legend),
        Ok(GraphicsObject::Legend(properties))
            if properties.labels_utf16
                == [
                    "first".encode_utf16().collect::<Vec<_>>(),
                    "second".encode_utf16().collect::<Vec<_>>()
                ]
                && properties.interpreter == Interpreter::None
                && properties.location == openmat_graphics_model::LegendLocation::Best
    ));
}

#[test]
fn parula_and_colormap_preserve_r2022b_rows_and_axes_state() {
    let registry = minimal_registry().unwrap();
    let mut session = GraphicsSession::new();
    let (two_colors, _) = invoke(&registry, &mut session, "parula", &[Value::Double(2.0)], 1);
    let two_colors = two_colors.unwrap();
    let Value::Array(ArrayData::F64(two_colors)) = &two_colors[0] else {
        panic!("parula must return a double matrix")
    };
    assert_eq!(two_colors.shape().dimensions(), &[2, 3]);
    assert_eq!(
        two_colors.as_slice(),
        &[0.2422, 0.9769, 0.1504, 0.9839, 0.6603, 0.0805]
    );

    let custom = f64_matrix(3, 3, vec![0.0, 0.3, 0.6, 0.1, 0.4, 0.7, 0.2, 0.5, 0.8]);
    let (set_result, _) = invoke(&registry, &mut session, "colormap", &[custom], 1);
    assert_eq!(
        f64_values(&set_result.unwrap()[0]),
        &[0.0, 0.3, 0.6, 0.1, 0.4, 0.7, 0.2, 0.5, 0.8]
    );
    let axes = session.current_axes().unwrap();
    let GraphicsObject::Axes2D(properties) = session.object(axes).unwrap() else {
        panic!()
    };
    assert_eq!(
        properties.colormap,
        [[0.0, 0.1, 0.2], [0.3, 0.4, 0.5], [0.6, 0.7, 0.8]]
    );

    let (named, _) = invoke(&registry, &mut session, "colormap", &[text("parula")], 1);
    assert_eq!(
        f64_values(&named.unwrap()[0]),
        flatten_colormap(&parula_r2022b(3))
    );
}

#[test]
fn predefined_colormaps_follow_current_length_without_query_side_effects() {
    let registry = minimal_registry().unwrap();
    let mut session = GraphicsSession::new();

    let (default_turbo, _) = invoke(&registry, &mut session, "turbo", &[], 1);
    let default_turbo = default_turbo.unwrap();
    assert_eq!(f64_dimensions(&default_turbo[0]), &[256, 3]);
    assert!(session.current_figure().is_none());
    assert!(session.current_axes().is_none());

    let custom = f64_matrix(3, 3, vec![0.0, 0.3, 0.6, 0.1, 0.4, 0.7, 0.2, 0.5, 0.8]);
    invoke(&registry, &mut session, "colormap", &[custom], 0)
        .0
        .unwrap();

    for colormap in PredefinedColormap::ALL {
        let (result, _) = invoke(&registry, &mut session, colormap.name(), &[], 1);
        let result = result.unwrap();
        let expected_rows = if colormap == PredefinedColormap::Vga {
            16
        } else {
            3
        };
        assert_eq!(f64_dimensions(&result[0]), &[expected_rows, 3]);
    }

    let (named_vga, _) = invoke(&registry, &mut session, "colormap", &[text("vga")], 1);
    assert_eq!(f64_dimensions(&named_vga.unwrap()[0]), &[16, 3]);
    let (vga_error, _) = invoke(&registry, &mut session, "vga", &[Value::Double(8.0)], 1);
    assert_eq!(
        vga_error.unwrap_err().category,
        BuiltinErrorCategory::ArgumentCount
    );
}

#[test]
fn colorbar_tick_properties_round_trip_through_set_and_get() {
    let registry = minimal_registry().unwrap();
    let mut session = GraphicsSession::new();
    let (colorbar, _) = invoke(&registry, &mut session, "colorbar", &[], 1);
    let colorbar = handle(&colorbar.unwrap()[0]);

    let (automatic, _) = invoke(
        &registry,
        &mut session,
        "get",
        &[Value::Graphics(colorbar), text("Ticks")],
        1,
    );
    let automatic = automatic.unwrap();
    assert_eq!(f64_values(&automatic[0]).len(), 11);

    invoke(
        &registry,
        &mut session,
        "set",
        &[
            Value::Graphics(colorbar),
            text("Ticks"),
            f64_matrix(1, 3, vec![0.2, 0.5, 0.8]),
            text("TickLabels"),
            text_cell(&["low", "middle", "high"]),
        ],
        0,
    )
    .0
    .unwrap();
    let (ticks, _) = invoke(
        &registry,
        &mut session,
        "get",
        &[Value::Graphics(colorbar), text("Ticks")],
        1,
    );
    assert_eq!(f64_values(&ticks.unwrap()[0]), &[0.2, 0.5, 0.8]);
    let (mode, _) = invoke(
        &registry,
        &mut session,
        "get",
        &[Value::Graphics(colorbar), text("TicksMode")],
        1,
    );
    assert_eq!(char_text(&mode.unwrap()[0]), "manual");
    assert!(matches!(
        session.object(colorbar),
        Ok(GraphicsObject::ColorBar(properties))
            if properties.tick_labels_utf16
                == [
                    "low".encode_utf16().collect::<Vec<_>>(),
                    "middle".encode_utf16().collect::<Vec<_>>(),
                    "high".encode_utf16().collect::<Vec<_>>()
                ]
                && properties.tick_label_mode == TickMode::Manual
    ));

    invoke(
        &registry,
        &mut session,
        "set",
        &[
            Value::Graphics(colorbar),
            text("TicksMode"),
            text("auto"),
            text("TickLabelsMode"),
            text("auto"),
        ],
        0,
    )
    .0
    .unwrap();
    assert!(matches!(
        session.object(colorbar),
        Ok(GraphicsObject::ColorBar(properties))
            if properties.ticks.len() == 11
                && properties.tick_mode == TickMode::Auto
                && properties.tick_label_mode == TickMode::Auto
    ));
}

fn flatten_colormap(colors: &[[f64; 3]]) -> Vec<f64> {
    let mut values = Vec::with_capacity(colors.len() * 3);
    for column in 0..3 {
        values.extend(colors.iter().map(|color| color[column]));
    }
    values
}
