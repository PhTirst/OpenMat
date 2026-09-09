use std::sync::Arc;

use openmat_array::{
    ArrayData, CharCodeUnit, DenseArray, IntegerArrayData, IntegerComponent, Logical, Shape,
};
use openmat_graphics_model::{
    Axis, AxisDirection, AxisScale, CDataMapping, ChartColor, ChartGroupInput, ChartGroupKind,
    ChartPrimitiveInput, DEFAULT_COLORMAP_LENGTH, FigureCreationProperties, GraphicsClass,
    GraphicsHandle, GraphicsProperty, GraphicsPropertyUpdate, GraphicsPropertyValue,
    GraphicsRequest, GraphicsResponse, GridMode, Interpreter, IsoNormalsInput, LegendLocation,
    LegendOrientation, LightingMode, LimitMethod, LimitMode, LineInput, LineStyle, Marker,
    MarkerIndicesClass, NextTileSelection, NumericData, PatchInput, PredefinedColormap,
    ProjectionMode, ScatterInput, ShadingMode, StyledLineInput, SurfaceColor, SurfaceInput,
    SurfaceStyle, TextRole, ThetaAxisUnits, ThetaDirection, ThetaZeroLocation, TickDirection,
    TickMode, TilePadding, TileSpacing, parula_r2022b,
};
use openmat_runtime::{BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinResult};
use openmat_value::{CellArray, GraphicsHandleArray, Value};

use crate::{
    U64_EXCLUSIVE_UPPER_BOUND, exact_real_integer_scalar, expect_argument_count,
    expect_argument_count_range, expect_max_outputs, type_error,
};

pub(crate) fn figure_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_max_outputs("figure", context, 1)?;
    let request = match arguments {
        [] => GraphicsRequest::Figure { number: None },
        [number] if number.as_real_number().is_some() => GraphicsRequest::Figure {
            number: Some(positive_figure_number(number)?),
        },
        _ => GraphicsRequest::CreateFigure {
            properties: figure_creation_properties(arguments)?,
        },
    };
    created_handle(context, request, "figure")
}

pub(crate) fn axes_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_max_outputs("axes", context, 1)?;
    let request = match arguments {
        [] => GraphicsRequest::Axes { select: None },
        [value]
            if graphics_handle(value)
                .is_some_and(|handle| handle.class() == GraphicsClass::Axes2D) =>
        {
            GraphicsRequest::Axes {
                select: Some(graphics_scalar("axes", 1, value, GraphicsClass::Axes2D)?),
            }
        }
        [value]
            if graphics_handle(value)
                .is_some_and(|handle| handle.class() == GraphicsClass::Figure) =>
        {
            GraphicsRequest::CreateAxes {
                figure: Some(graphics_scalar("axes", 1, value, GraphicsClass::Figure)?),
                properties: Vec::new(),
            }
        }
        _ => {
            let (figure, properties) = axes_creation_properties(arguments)?;
            GraphicsRequest::CreateAxes { figure, properties }
        }
    };
    created_handle(context, request, "axes")
}

pub(crate) fn polaraxes_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_max_outputs("polaraxes", context, 1)?;
    let (figure, properties) =
        axes_creation_properties_for("polaraxes", GraphicsClass::PolarAxes, arguments)?;
    created_handle(
        context,
        GraphicsRequest::CreatePolarAxes { figure, properties },
        "polaraxes",
    )
}

pub(crate) fn subplot_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count("subplot", arguments, 3)?;
    expect_max_outputs("subplot", context, 1)?;
    let rows = positive_u32("subplot", 1, &arguments[0])?;
    let columns = positive_u32("subplot", 2, &arguments[1])?;
    let position = subplot_position(rows, columns, &arguments[2])?;
    created_handle(
        context,
        GraphicsRequest::Subplot {
            figure: None,
            position,
        },
        "subplot",
    )
}

pub(crate) fn tiledlayout_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_max_outputs("tiledlayout", context, 1)?;
    let (figure, arguments, position_offset) = if arguments.first().is_some_and(|value| {
        graphics_handle(value).is_some_and(|handle| handle.class() == GraphicsClass::Figure)
    }) {
        (
            Some(graphics_scalar(
                "tiledlayout",
                1,
                &arguments[0],
                GraphicsClass::Figure,
            )?),
            &arguments[1..],
            1,
        )
    } else {
        (None, arguments, 0)
    };
    if arguments.len() < 2 || !(arguments.len() - 2).is_multiple_of(2) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            "`tiledlayout` requires rows, columns, and optional name/value pairs",
        ));
    }
    let rows = positive_u32("tiledlayout", position_offset + 1, &arguments[0])?;
    let columns = positive_u32("tiledlayout", position_offset + 2, &arguments[1])?;
    let mut tile_spacing = TileSpacing::Loose;
    let mut padding = TilePadding::Loose;
    for (pair, values) in arguments[2..].chunks_exact(2).enumerate() {
        let name_position = position_offset + 3 + pair * 2;
        let value_position = name_position + 1;
        let name = lowercase_ascii_text("tiledlayout", name_position, &values[0])?;
        if "tilespacing".starts_with(&name) {
            tile_spacing = tiled_spacing_value(value_position, &values[1])?;
        } else if "padding".starts_with(&name) {
            padding = tiled_padding_value(value_position, &values[1])?;
        } else {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Graphics,
                format!("`tiledlayout` does not recognize constructor property `{name}`"),
            ));
        }
    }
    created_handle(
        context,
        GraphicsRequest::CreateTiledLayout {
            figure,
            rows,
            columns,
            tile_spacing,
            padding,
        },
        "tiledlayout",
    )
}

pub(crate) fn nexttile_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count_range("nexttile", arguments, 0, 2)?;
    expect_max_outputs("nexttile", context, 1)?;
    let (layout, selection) = match arguments {
        [] => (None, NextTileSelection::Automatic),
        [value]
            if graphics_handle(value)
                .is_some_and(|handle| handle.class() == GraphicsClass::TiledChartLayout) =>
        {
            (
                Some(graphics_scalar(
                    "nexttile",
                    1,
                    value,
                    GraphicsClass::TiledChartLayout,
                )?),
                NextTileSelection::Automatic,
            )
        }
        [value] => (None, nexttile_selection(1, value)?),
        [layout, value] => (
            Some(graphics_scalar(
                "nexttile",
                1,
                layout,
                GraphicsClass::TiledChartLayout,
            )?),
            nexttile_selection(2, value)?,
        ),
        _ => unreachable!("argument count checked above"),
    };
    created_handle(
        context,
        GraphicsRequest::NextTile { layout, selection },
        "nexttile",
    )
}

pub(crate) fn gcf_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count("gcf", arguments, 0)?;
    expect_max_outputs("gcf", context, 1)?;
    one_handle(&context.graphics(GraphicsRequest::CurrentFigure)?, "gcf")
}

pub(crate) fn groot_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count("groot", arguments, 0)?;
    expect_max_outputs("groot", context, 1)?;
    Ok(vec![Value::Double(0.0)])
}

pub(crate) fn gca_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count("gca", arguments, 0)?;
    expect_max_outputs("gca", context, 1)?;
    one_handle(&context.graphics(GraphicsRequest::CurrentAxes)?, "gca")
}

pub(crate) fn plot_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    if arguments.is_empty() {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            "`plot` requires plotting data",
        ));
    }
    expect_max_outputs("plot", context, 1)?;
    let (axes, arguments) = optional_axes("plot", arguments)?;
    let (series, properties) = prepare_plot_series(arguments)?;
    let response = context.graphics(GraphicsRequest::PlotSeries {
        axes,
        series,
        properties,
    })?;
    let GraphicsResponse::Handles(handles) = response else {
        return Err(graphics_state_error("plot", "handle column"));
    };
    if context.requested_outputs() == 0 {
        return Ok(Vec::new());
    }
    let array =
        GraphicsHandleArray::column(GraphicsClass::LineSeries, handles).map_err(|error| {
            BuiltinError::new(
                BuiltinErrorCategory::Graphics,
                format!("`plot` could not construct its Line handle column: {error}"),
            )
        })?;
    Ok(vec![Value::GraphicsArray(array)])
}

pub(crate) fn polarplot_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    if arguments.is_empty() {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            "`polarplot` requires theta and radial data",
        ));
    }
    expect_max_outputs("polarplot", context, 1)?;
    let (axes, arguments) = optional_polar_axes("polarplot", arguments)?;
    let (series, properties) = prepare_plot_series(arguments)?;
    let response = context.graphics(GraphicsRequest::PolarPlotSeries {
        axes,
        series,
        properties,
    })?;
    let GraphicsResponse::Handles(handles) = response else {
        return Err(graphics_state_error("polarplot", "handle column"));
    };
    if context.requested_outputs() == 0 {
        return Ok(Vec::new());
    }
    let array =
        GraphicsHandleArray::column(GraphicsClass::LineSeries, handles).map_err(|error| {
            BuiltinError::new(
                BuiltinErrorCategory::Graphics,
                format!("`polarplot` could not construct its Line handle column: {error}"),
            )
        })?;
    Ok(vec![Value::GraphicsArray(array)])
}

pub(crate) fn stairs_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    high_level_line_chart(
        "stairs",
        ChartGroupKind::Stair,
        arguments,
        context,
        |x, y| {
            let mut sx = Vec::with_capacity(x.len().saturating_mul(2).saturating_sub(1));
            let mut sy = Vec::with_capacity(y.len().saturating_mul(2).saturating_sub(1));
            if let (Some(&first_x), Some(&first_y)) = (x.first(), y.first()) {
                sx.push(first_x);
                sy.push(first_y);
                for index in 1..x.len() {
                    sx.push(x[index]);
                    sy.push(y[index - 1]);
                    sx.push(x[index]);
                    sy.push(y[index]);
                }
            }
            (sx, sy, Vec::new())
        },
    )
}

pub(crate) fn stem_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    high_level_line_chart("stem", ChartGroupKind::Stem, arguments, context, |x, y| {
        let mut sx = Vec::with_capacity(x.len() * 3);
        let mut sy = Vec::with_capacity(y.len() * 3);
        let mut marker_indices = Vec::with_capacity(x.len());
        for (&x, &y) in x.iter().zip(y) {
            sx.extend([x, x, f64::NAN]);
            sy.extend([0.0, y, f64::NAN]);
            marker_indices.push(sx.len() as u64 - 1);
        }
        (
            sx,
            sy,
            vec![
                GraphicsPropertyUpdate::Marker(Marker::Circle),
                GraphicsPropertyUpdate::MarkerIndices {
                    values: marker_indices,
                    value_class: MarkerIndicesClass::UInt64,
                },
            ],
        )
    })
}

pub(crate) fn errorbar_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_max_outputs("errorbar", context, 1)?;
    let (axes, arguments) = optional_axes("errorbar", arguments)?;
    let numeric_count = arguments
        .iter()
        .take_while(|value| is_plot_numeric(value))
        .count();
    if !(2..=4).contains(&numeric_count) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            "`errorbar` requires Y/E, X/Y/E, or X/Y/negative/positive vectors",
        ));
    }
    let (x, y, negative, positive) = match numeric_count {
        2 => {
            let y = prepared_vector("errorbar", 1, &arguments[0])?;
            let error = prepared_vector("errorbar", 2, &arguments[1])?;
            let x = default_f64_x(y.len());
            (x, y, error.clone(), error)
        }
        3 => {
            let x = prepared_vector("errorbar", 1, &arguments[0])?;
            let y = prepared_vector("errorbar", 2, &arguments[1])?;
            let error = prepared_vector("errorbar", 3, &arguments[2])?;
            (x, y, error.clone(), error)
        }
        4 => (
            prepared_vector("errorbar", 1, &arguments[0])?,
            prepared_vector("errorbar", 2, &arguments[1])?,
            prepared_vector("errorbar", 3, &arguments[2])?,
            prepared_vector("errorbar", 4, &arguments[3])?,
        ),
        _ => unreachable!(),
    };
    require_equal_lengths("errorbar", &[&x, &y, &negative, &positive])?;
    if negative
        .iter()
        .chain(&positive)
        .any(|value| !value.is_finite() || *value < 0.0)
    {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "`errorbar` error magnitudes must be nonnegative finite values",
        ));
    }
    let mut cap_size_points = 6.0;
    let properties = parse_chart_line_properties(
        "errorbar",
        numeric_count,
        &arguments[numeric_count..],
        Some(&mut cap_size_points),
    )?;
    let span = finite_span(&x).unwrap_or(1.0);
    let cap_half_width = span * cap_size_points / 500.0;
    let mut gx = x.clone();
    let mut gy = y.clone();
    gx.push(f64::NAN);
    gy.push(f64::NAN);
    for index in 0..x.len() {
        let lower = y[index] - negative[index];
        let upper = y[index] + positive[index];
        gx.extend([
            x[index],
            x[index],
            f64::NAN,
            x[index] - cap_half_width,
            x[index] + cap_half_width,
            f64::NAN,
            x[index] - cap_half_width,
            x[index] + cap_half_width,
            f64::NAN,
        ]);
        gy.extend([
            lower,
            upper,
            f64::NAN,
            lower,
            lower,
            f64::NAN,
            upper,
            upper,
            f64::NAN,
        ]);
    }
    create_chart(
        "errorbar",
        axes,
        ChartGroupKind::ErrorBar,
        vec![line_primitive(gx, gy, properties, true)],
        vec![GraphicsPropertyUpdate::CapSize(cap_size_points)],
        Vec::new(),
        context,
    )
}

pub(crate) fn area_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_max_outputs("area", context, 1)?;
    let (axes, arguments) = optional_axes("area", arguments)?;
    let (x, y, data_count) = chart_xy("area", arguments)?;
    let properties = parse_chart_patch_properties("area", data_count, &arguments[data_count..])?;
    let mut polygon = x.iter().copied().zip(y.iter().copied()).collect::<Vec<_>>();
    if let (Some(&last), Some(&first)) = (x.last(), x.first()) {
        polygon.push((last, 0.0));
        polygon.push((first, 0.0));
    }
    create_chart(
        "area",
        axes,
        ChartGroupKind::Area,
        vec![patch_primitive(
            polygons_patch(
                &[polygon],
                &[],
                SurfaceColor::Rgba([0.0, 0.0, 0.0, 1.0]),
                SurfaceColor::Rgba([0.0, 0.0, 0.0, 1.0]),
            )?,
            properties,
            true,
        )],
        Vec::new(),
        Vec::new(),
        context,
    )
}

#[allow(clippy::cast_precision_loss, clippy::too_many_lines)]
pub(crate) fn bar_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_max_outputs("bar", context, 1)?;
    let (axes, arguments) = optional_axes("bar", arguments)?;
    let Some(first_value) = arguments.first() else {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            "`bar` requires plotting data",
        ));
    };
    let first = PreparedNumeric::from_value("bar", 1, first_value)?;
    let mut cursor = 1;
    let (x, y) = if first.is_vector()
        && let Some(second_value) = arguments.get(1)
        && is_plot_numeric(second_value)
    {
        let second = PreparedNumeric::from_value("bar", 2, second_value)?;
        let second_is_y = (!second.is_vector() && first.numel() == second.rows)
            || (second.is_vector() && second.numel() == first.numel() && second.numel() != 1);
        if second_is_y {
            cursor = 2;
            (first.vector_f64(), second)
        } else {
            (default_f64_x(first.numel()), first)
        }
    } else {
        let rows = if first.is_vector() {
            first.numel()
        } else {
            first.rows
        };
        (default_f64_x(rows), first)
    };
    let rows = if y.is_vector() { y.numel() } else { y.rows };
    if x.len() != rows {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "`bar` XData length must match the number of YData rows",
        ));
    }
    let mut width = 0.8;
    if let Some(value) = arguments.get(cursor).and_then(Value::as_real_number) {
        if !value.is_finite() || value <= 0.0 || value > 1.0 {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "`bar` width must be a finite scalar in (0,1]",
            ));
        }
        width = value;
        cursor += 1;
    }
    let mut stacked = false;
    if let Some(value) = arguments.get(cursor)
        && let Ok(style) = lowercase_ascii_text("bar", cursor + 1, value)
    {
        match style.as_str() {
            "grouped" => cursor += 1,
            "stacked" => {
                stacked = true;
                cursor += 1;
            }
            _ => {}
        }
    }
    let properties = parse_chart_patch_properties("bar", cursor, &arguments[cursor..])?;
    let series = if y.is_vector() {
        vec![y.vector_f64()]
    } else {
        (0..y.columns)
            .map(|column| y.column_f64(column))
            .collect::<Vec<_>>()
    };
    let spacing = minimum_positive_spacing(&x).unwrap_or(1.0);
    let group_width = spacing * width;
    let series_count = series.len();
    let mut positive_base = vec![0.0; rows];
    let mut negative_base = vec![0.0; rows];
    let primitives = series
        .iter()
        .enumerate()
        .map(|(series_index, values)| {
            let bar_width = if stacked {
                group_width
            } else {
                group_width / series_count as f64
            };
            let offset = if stacked {
                0.0
            } else {
                (series_index as f64 + 0.5) * bar_width - group_width / 2.0
            };
            let polygons = x
                .iter()
                .copied()
                .zip(values.iter().copied())
                .enumerate()
                .map(|(row, (x, value))| {
                    let baseline = if stacked {
                        if value >= 0.0 {
                            let baseline = positive_base[row];
                            positive_base[row] += value;
                            baseline
                        } else {
                            let baseline = negative_base[row];
                            negative_base[row] += value;
                            baseline
                        }
                    } else {
                        0.0
                    };
                    let top = baseline + value;
                    let center = x + offset;
                    vec![
                        (center - bar_width / 2.0, baseline),
                        (center + bar_width / 2.0, baseline),
                        (center + bar_width / 2.0, top),
                        (center - bar_width / 2.0, top),
                    ]
                })
                .collect::<Vec<_>>();
            Ok(patch_primitive(
                polygons_patch(
                    &polygons,
                    &[],
                    SurfaceColor::Rgba([0.0, 0.0, 0.0, 1.0]),
                    SurfaceColor::Rgba([0.0, 0.0, 0.0, 1.0]),
                )?,
                properties.clone(),
                true,
            ))
        })
        .collect::<Result<Vec<_>, BuiltinError>>()?;
    create_chart(
        "bar",
        axes,
        ChartGroupKind::Bar,
        primitives,
        vec![GraphicsPropertyUpdate::BarWidth(width)],
        Vec::new(),
        context,
    )
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::too_many_lines
)]
pub(crate) fn histogram_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_max_outputs("histogram", context, 1)?;
    let (axes, arguments) = optional_axes("histogram", arguments)?;
    let Some(data_value) = arguments.first() else {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            "`histogram` requires a real data vector",
        ));
    };
    let data = prepared_vector("histogram", 1, data_value)?;
    let mut cursor = 1;
    let mut edges = None;
    let mut bin_count = None;
    if let Some(value) = arguments.get(cursor)
        && is_plot_numeric(value)
    {
        let numeric = PreparedNumeric::from_value("histogram", cursor + 1, value)?;
        if numeric.numel() == 1 {
            let count = numeric.vector_f64()[0];
            if !count.is_finite() || count < 1.0 || count.fract() != 0.0 {
                return Err(BuiltinError::new(
                    BuiltinErrorCategory::Domain,
                    "`histogram` bin count must be a positive integer scalar",
                ));
            }
            bin_count = Some(count as usize);
        } else {
            let candidate = numeric.vector_f64();
            if !numeric.is_vector()
                || candidate.len() < 2
                || candidate
                    .windows(2)
                    .any(|pair| !pair[0].is_finite() || pair[0] >= pair[1])
                || !candidate.last().is_some_and(|value| value.is_finite())
            {
                return Err(BuiltinError::new(
                    BuiltinErrorCategory::Domain,
                    "`histogram` BinEdges must be a strictly increasing finite vector",
                ));
            }
            edges = Some(candidate);
        }
        cursor += 1;
    }
    let mut normalization = "count";
    let mut patch_properties = Vec::new();
    let trailing = &arguments[cursor..];
    if !trailing.len().is_multiple_of(2) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            "`histogram` options must be name/value pairs",
        ));
    }
    for (pair, values) in trailing.chunks_exact(2).enumerate() {
        let name_position = cursor + pair * 2 + 1;
        let value_position = name_position + 1;
        let name = lowercase_ascii_text("histogram", name_position, &values[0])?;
        match name.as_str() {
            "normalization" => {
                normalization =
                    match lowercase_ascii_text("histogram", value_position, &values[1])?.as_str() {
                        "count" => "count",
                        "probability" => "probability",
                        "pdf" => "pdf",
                        "cdf" => "cdf",
                        "countdensity" => "countdensity",
                        "cumcount" => "cumcount",
                        _ => {
                            return Err(BuiltinError::new(
                                BuiltinErrorCategory::Domain,
                                "unsupported `histogram` Normalization",
                            ));
                        }
                    };
            }
            "facecolor" | "edgecolor" | "linewidth" | "linestyle" | "facealpha" | "edgealpha"
            | "visible" => patch_properties.push(parse_property_update(
                "histogram",
                name_position,
                value_position,
                GraphicsClass::PatchSeries,
                &values[0],
                &values[1],
            )?),
            _ => {
                return Err(BuiltinError::new(
                    BuiltinErrorCategory::Graphics,
                    format!("`histogram` does not recognize property `{name}`"),
                ));
            }
        }
    }
    let finite = data
        .into_iter()
        .filter(|value| value.is_finite())
        .collect::<Vec<_>>();
    let edges = edges.unwrap_or_else(|| automatic_histogram_edges(&finite, bin_count));
    let mut counts = histogram_counts(&finite, &edges);
    normalize_histogram(&mut counts, &edges, finite.len(), normalization);
    let polygons = counts
        .iter()
        .enumerate()
        .map(|(index, count)| {
            vec![
                (edges[index], 0.0),
                (edges[index + 1], 0.0),
                (edges[index + 1], *count),
                (edges[index], *count),
            ]
        })
        .collect::<Vec<_>>();
    create_chart(
        "histogram",
        axes,
        ChartGroupKind::Histogram,
        vec![patch_primitive(
            polygons_patch(
                &polygons,
                &[],
                SurfaceColor::Rgba([0.0, 0.0, 0.0, 1.0]),
                SurfaceColor::Rgba([0.0, 0.0, 0.0, 1.0]),
            )?,
            patch_properties,
            true,
        )],
        Vec::new(),
        Vec::new(),
        context,
    )
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::float_cmp,
    clippy::too_many_lines
)]
pub(crate) fn contour_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_max_outputs("contour", context, 2)?;
    let (axes, arguments) = optional_axes("contour", arguments)?;
    let (x, y, z, rows, columns, mut cursor) = prepare_contour_data(arguments)?;
    let levels = if let Some(value) = arguments.get(cursor)
        && is_plot_numeric(value)
    {
        cursor += 1;
        let requested = prepared_vector("contour", cursor, value)?;
        if requested.len() == 1 {
            let count = requested[0];
            if !count.is_finite() || count < 1.0 || count.fract() != 0.0 {
                return Err(BuiltinError::new(
                    BuiltinErrorCategory::Domain,
                    "`contour` level count must be a positive integer scalar",
                ));
            }
            requested_contour_levels(&z, count as usize)
        } else {
            let mut levels = requested;
            levels.sort_by(f64::total_cmp);
            levels.dedup_by(|left, right| left.total_cmp(right).is_eq());
            levels
        }
    } else {
        automatic_contour_levels(&z)
    };
    if !levels.iter().all(|value| value.is_finite()) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "`contour` levels must be finite",
        ));
    }
    let mut base_properties = Vec::new();
    let trailing = &arguments[cursor..];
    if !trailing.len().is_multiple_of(2) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            "`contour` options must be name/value pairs",
        ));
    }
    for (pair, values) in trailing.chunks_exact(2).enumerate() {
        let name_position = cursor + pair * 2 + 1;
        let value_position = name_position + 1;
        let name = lowercase_ascii_text("contour", name_position, &values[0])?;
        if name == "linecolor" {
            base_properties.push(GraphicsPropertyUpdate::Color(rgb_color(
                "contour",
                value_position,
                "LineColor",
                &values[1],
            )?));
        } else {
            base_properties.push(parse_property_update(
                "contour",
                name_position,
                value_position,
                GraphicsClass::LineSeries,
                &values[0],
                &values[1],
            )?);
        }
    }
    if levels.is_empty() {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "`contour` requires at least one contour level",
        ));
    }
    let color_limits = contour_color_limits(&levels);
    let colors = parula_r2022b(DEFAULT_COLORMAP_LENGTH);
    let mut primitives = Vec::new();
    let mut contour_matrix = Vec::<(f64, Vec<(f64, f64)>)>::new();
    for &level in &levels {
        let segments = marching_squares(&x, &y, &z, rows, columns, level);
        if segments.is_empty() {
            continue;
        }
        let mut lx = Vec::with_capacity(segments.len() * 3);
        let mut ly = Vec::with_capacity(segments.len() * 3);
        for &(start, end) in &segments {
            lx.extend([start.0, end.0, f64::NAN]);
            ly.extend([start.1, end.1, f64::NAN]);
            contour_matrix.push((level, vec![start, end]));
        }
        let mut properties = base_properties.clone();
        if !properties
            .iter()
            .any(|update| matches!(update, GraphicsPropertyUpdate::Color(_)))
        {
            let color_index = if color_limits[0] == color_limits[1] {
                0
            } else {
                (((level - color_limits[0]) / (color_limits[1] - color_limits[0])).clamp(0.0, 1.0)
                    * (colors.len() - 1) as f64)
                    .round() as usize
            };
            let color = colors[color_index.min(colors.len() - 1)];
            properties.push(GraphicsPropertyUpdate::Color([
                color[0] as f32,
                color[1] as f32,
                color[2] as f32,
                1.0,
            ]));
        }
        primitives.push(line_primitive(lx, ly, properties, false));
    }
    if primitives.is_empty() {
        primitives.push(line_primitive(
            Vec::new(),
            Vec::new(),
            base_properties,
            false,
        ));
    }
    let response = context.graphics(GraphicsRequest::ChartGroup {
        axes,
        chart: ChartGroupInput {
            kind: ChartGroupKind::Contour,
            primitives,
            properties: Vec::new(),
            axes_properties: vec![GraphicsPropertyUpdate::CLim(color_limits)],
        },
    })?;
    contour_outputs(&response, contour_matrix, context.requested_outputs())
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::too_many_lines
)]
pub(crate) fn contourf_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_max_outputs("contourf", context, 2)?;
    let (axes, arguments) = optional_axes("contourf", arguments)?;
    let (x, y, z, rows, columns, mut cursor) = prepare_contour_data(arguments)?;
    let levels = if let Some(value) = arguments.get(cursor)
        && is_plot_numeric(value)
    {
        cursor += 1;
        let requested = prepared_vector("contourf", cursor, value)?;
        if requested.len() == 1 {
            let count = requested[0];
            if !count.is_finite() || count < 1.0 || count.fract() != 0.0 {
                return Err(BuiltinError::new(
                    BuiltinErrorCategory::Domain,
                    "`contourf` level count must be a positive integer scalar",
                ));
            }
            requested_contour_levels(&z, count as usize)
        } else {
            let mut levels = requested;
            levels.sort_by(f64::total_cmp);
            levels.dedup_by(|left, right| left.total_cmp(right).is_eq());
            levels
        }
    } else {
        automatic_contour_levels(&z)
    };
    if levels.is_empty() || !levels.iter().all(|value| value.is_finite()) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "`contourf` requires at least one finite contour level",
        ));
    }
    let trailing = &arguments[cursor..];
    if !trailing.len().is_multiple_of(2) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            "`contourf` options must be name/value pairs",
        ));
    }
    let mut properties = Vec::new();
    for (pair, values) in trailing.chunks_exact(2).enumerate() {
        let name_position = cursor + pair * 2 + 1;
        let value_position = name_position + 1;
        let property = lowercase_ascii_text("contourf", name_position, &values[0])?;
        if property == "linecolor" {
            properties.push(GraphicsPropertyUpdate::EdgeColor(surface_color_value(
                "contourf",
                value_position,
                &values[1],
            )?));
        } else {
            properties.push(parse_property_update(
                "contourf",
                name_position,
                value_position,
                GraphicsClass::PatchSeries,
                &values[0],
                &values[1],
            )?);
        }
    }
    let compact = use_compact_filled_contours(rows, columns, levels.len(), &properties);
    let color_limits = contour_range(&z).map_or_else(
        || contour_color_limits(&levels),
        |(minimum, maximum)| {
            if minimum < maximum {
                [minimum, maximum]
            } else {
                [minimum - 1.0, maximum + 1.0]
            }
        },
    );
    let patch = if compact {
        compact_filled_contour_patch(&x, &y, &z, rows, columns, &levels)?
    } else {
        clipped_filled_contour_patch(&x, &y, &z, rows, columns, &levels)?
    };
    let response = context.graphics(GraphicsRequest::ChartGroup {
        axes,
        chart: ChartGroupInput {
            kind: ChartGroupKind::Contour,
            primitives: vec![patch_primitive(patch, properties, false)],
            properties: Vec::new(),
            axes_properties: vec![GraphicsPropertyUpdate::CLim(color_limits)],
        },
    })?;
    let mut contour_matrix = Vec::new();
    if context.requested_outputs() > 0 {
        for &level in &levels {
            contour_matrix.extend(
                marching_squares(&x, &y, &z, rows, columns, level)
                    .into_iter()
                    .map(|(start, end)| (level, vec![start, end])),
            );
        }
    }
    contour_outputs(&response, contour_matrix, context.requested_outputs())
}

pub(crate) fn image_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    image_chart("image", CDataMapping::Direct, arguments, context)
}

pub(crate) fn imagesc_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    image_chart("imagesc", CDataMapping::Scaled, arguments, context)
}

pub(crate) fn semilogx_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    scaled_plot_builtin(
        "semilogx",
        AxisScale::Log,
        AxisScale::Linear,
        arguments,
        context,
    )
}

pub(crate) fn semilogy_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    scaled_plot_builtin(
        "semilogy",
        AxisScale::Linear,
        AxisScale::Log,
        arguments,
        context,
    )
}

pub(crate) fn loglog_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    scaled_plot_builtin("loglog", AxisScale::Log, AxisScale::Log, arguments, context)
}

fn scaled_plot_builtin(
    name: &str,
    x_scale: AxisScale,
    y_scale: AxisScale,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    if arguments.is_empty() {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            format!("`{name}` requires plotting data"),
        ));
    }
    expect_max_outputs(name, context, 1)?;
    let (axes, arguments) = optional_axes(name, arguments)?;
    let (lines, properties) = prepare_styled_plot(arguments)?;
    let GraphicsResponse::Handles(handles) =
        context.graphics(GraphicsRequest::PlotWithAxesProperties {
            axes,
            lines,
            properties,
            axes_properties: vec![
                GraphicsPropertyUpdate::XScale(x_scale),
                GraphicsPropertyUpdate::YScale(y_scale),
            ],
        })?
    else {
        return Err(graphics_state_error(name, "handle column"));
    };
    if context.requested_outputs() == 0 {
        return Ok(Vec::new());
    }
    let array =
        GraphicsHandleArray::column(GraphicsClass::LineSeries, handles).map_err(|error| {
            BuiltinError::new(
                BuiltinErrorCategory::Graphics,
                format!("`{name}` could not construct its Line handle column: {error}"),
            )
        })?;
    Ok(vec![Value::GraphicsArray(array)])
}

pub(crate) fn plot3_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    let (axes, arguments) = optional_axes("plot3", arguments)?;
    if arguments.len() < 3 {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            "`plot3` requires X, Y, and Z plotting data",
        ));
    }
    expect_max_outputs("plot3", context, 1)?;
    let x = PreparedNumeric::from_value("plot3", 1, &arguments[0])?;
    let y = PreparedNumeric::from_value("plot3", 2, &arguments[1])?;
    let z = PreparedNumeric::from_value("plot3", 3, &arguments[2])?;
    let lines = prepare_plot3_xyz(&x, &y, &z)?;
    let trailing = &arguments[3..];
    let mut properties = Vec::new();
    let mut offset = 0;
    if !trailing.len().is_multiple_of(2) {
        properties.extend(parse_line_spec(4, &trailing[0])?);
        offset = 1;
    }
    if !(trailing.len() - offset).is_multiple_of(2) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            "`plot3` property names must be followed by values",
        ));
    }
    for pair in 0..((trailing.len() - offset) / 2) {
        let name_position = 4 + offset + pair * 2;
        properties.push(parse_property_update(
            "plot3",
            name_position,
            name_position + 1,
            GraphicsClass::LineSeries,
            &trailing[offset + pair * 2],
            &trailing[offset + pair * 2 + 1],
        )?);
    }
    let GraphicsResponse::Handles(handles) = context.graphics(GraphicsRequest::PlotStyled {
        axes,
        lines,
        properties,
    })?
    else {
        return Err(graphics_state_error("plot3", "handle column"));
    };
    if context.requested_outputs() == 0 {
        return Ok(Vec::new());
    }
    let array =
        GraphicsHandleArray::column(GraphicsClass::LineSeries, handles).map_err(|error| {
            BuiltinError::new(
                BuiltinErrorCategory::Graphics,
                format!("`plot3` could not construct its Line handle column: {error}"),
            )
        })?;
    Ok(vec![Value::GraphicsArray(array)])
}

pub(crate) fn scatter_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    let (axes, arguments) = optional_axes("scatter", arguments)?;
    if arguments.len() < 2 {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            "`scatter` requires X and Y data",
        ));
    }
    expect_max_outputs("scatter", context, 1)?;
    let x = PreparedNumeric::from_value("scatter", 1, &arguments[0])?;
    let y = PreparedNumeric::from_value("scatter", 2, &arguments[1])?;
    if !x.is_vector() || !y.is_vector() || x.numel() != y.numel() {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "`scatter` v1 requires equal-length real vectors",
        ));
    }
    let mut properties = Vec::new();
    let mut size_data = None;
    let mut cursor = 2;
    if let Some(value) = arguments.get(cursor).filter(|value| is_plot_numeric(value)) {
        let prepared = PreparedNumeric::from_value("scatter", 3, value)?;
        if prepared.numel() == 1 {
            properties.push(GraphicsPropertyUpdate::SizeData(nonnegative_f32_scalar(
                "scatter", 3, "SizeData", value,
            )?));
        } else if prepared.is_vector() && prepared.numel() == x.numel() {
            validate_scatter_sizes("scatter", 3, &prepared)?;
            size_data = Some(prepared.vector());
        } else if prepared.numel() != 0 {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "`scatter` SizeData must be scalar, empty, or match the point count",
            ));
        }
        cursor += 1;
    }
    let mut color_data = None;
    if let Some(value) = arguments.get(cursor).filter(|value| is_plot_numeric(value)) {
        let prepared = PreparedNumeric::from_value("scatter", 4, value)?;
        if prepared.rows == 1 && prepared.columns == 3 {
            properties.push(GraphicsPropertyUpdate::CData(rgb_color(
                "scatter", 4, "CData", value,
            )?));
        } else if prepared.is_vector() && prepared.numel() == x.numel() {
            color_data = Some(prepared.vector());
        } else {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "`scatter` CData must be an RGB row or one scalar per point",
            ));
        }
        cursor += 1;
    }
    let mut filled = false;
    while let Some(value) = arguments.get(cursor).filter(|value| is_text_value(value)) {
        if lowercase_ascii_text("scatter", cursor + 1, value).is_ok_and(|text| text == "filled") {
            filled = true;
            cursor += 1;
        } else if let Ok(marker) = marker_value("scatter", cursor + 1, value) {
            properties.push(GraphicsPropertyUpdate::Marker(marker));
            cursor += 1;
        } else {
            break;
        }
    }
    let trailing = &arguments[cursor..];
    if !trailing.len().is_multiple_of(2) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            "`scatter` options must be complete name/value pairs",
        ));
    }
    for (pair, values) in trailing.chunks_exact(2).enumerate() {
        let name_position = cursor + pair * 2 + 1;
        properties.push(parse_property_update(
            "scatter",
            name_position,
            name_position + 1,
            GraphicsClass::ScatterSeries,
            &values[0],
            &values[1],
        )?);
    }
    created_handle(
        context,
        GraphicsRequest::Scatter {
            axes,
            series: ScatterInput {
                x: x.vector(),
                y: y.vector(),
                z: None,
                size_data,
                color_data,
                filled,
            },
            properties,
        },
        "scatter",
    )
}

pub(crate) fn scatter3_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    let (axes, arguments) = optional_axes("scatter3", arguments)?;
    expect_argument_count_range("scatter3", arguments, 3, 7)?;
    expect_max_outputs("scatter3", context, 1)?;
    let x = PreparedNumeric::from_value("scatter3", 1, &arguments[0])?;
    let y = PreparedNumeric::from_value("scatter3", 2, &arguments[1])?;
    let z = PreparedNumeric::from_value("scatter3", 3, &arguments[2])?;
    if !x.is_vector()
        || !y.is_vector()
        || !z.is_vector()
        || x.numel() != y.numel()
        || x.numel() != z.numel()
    {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "`scatter3` requires equal-length real vectors",
        ));
    }
    let mut properties = Vec::new();
    let mut size_data = None;
    let mut color = None;
    let mut filled = false;
    if let Some(value) = arguments.get(3) {
        let prepared = PreparedNumeric::from_value("scatter3", 4, value)?;
        if prepared.numel() == 1 {
            properties.push(GraphicsPropertyUpdate::SizeData(nonnegative_f32_scalar(
                "scatter3", 4, "SizeData", value,
            )?));
        } else if prepared.is_vector() && prepared.numel() == x.numel() {
            validate_scatter_sizes("scatter3", 4, &prepared)?;
            size_data = Some(prepared.vector());
        } else if prepared.numel() != 0 {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "`scatter3` SizeData must be scalar, empty, or match the point count",
            ));
        }
    }
    let mut color_data = None;
    if let Some(value) = arguments.get(4) {
        let prepared = PreparedNumeric::from_value("scatter3", 5, value)?;
        if prepared.rows == 1 && prepared.columns == 3 {
            let value = rgb_color("scatter3", 5, "CData", value)?;
            color = Some(value);
            properties.push(GraphicsPropertyUpdate::CData(value));
        } else if prepared.is_vector() && prepared.numel() == x.numel() {
            color_data = Some(prepared.vector());
        } else {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "`scatter3` CData must be an RGB row or one scalar per point",
            ));
        }
    }
    for (index, value) in arguments.iter().enumerate().skip(5) {
        let text = lowercase_ascii_text("scatter3", index + 1, value)?;
        if text == "filled" {
            filled = true;
        } else {
            properties.push(GraphicsPropertyUpdate::Marker(marker_value(
                "scatter3",
                index + 1,
                value,
            )?));
        }
    }
    if filled {
        properties.push(GraphicsPropertyUpdate::MarkerFaceColor(
            color.unwrap_or([0.0, 0.447, 0.741, 1.0]),
        ));
    }
    created_handle(
        context,
        GraphicsRequest::Scatter {
            axes,
            series: ScatterInput {
                x: x.vector(),
                y: y.vector(),
                z: Some(z.vector()),
                size_data,
                color_data,
                filled,
            },
            properties,
        },
        "scatter3",
    )
}

fn validate_scatter_sizes(
    name: &str,
    position: usize,
    values: &PreparedNumeric,
) -> Result<(), BuiltinError> {
    if values
        .vector_f64()
        .into_iter()
        .any(|value| !value.is_finite() || value < 0.0)
    {
        Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("input {position} to `{name}` SizeData must contain nonnegative finite values"),
        ))
    } else {
        Ok(())
    }
}

pub(crate) fn surf_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    surface_builtin("surf", SurfaceStyle::Surf, arguments, context)
}

pub(crate) fn mesh_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    surface_builtin("mesh", SurfaceStyle::Mesh, arguments, context)
}

pub(crate) fn patch_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_max_outputs("patch", context, 1)?;
    let (axes, arguments) = optional_axes("patch", arguments)?;
    let (patch, properties) = prepare_patch("patch", arguments, PatchConstructor::Patch)?;
    created_handle(
        context,
        GraphicsRequest::Patch {
            axes,
            patch,
            properties,
        },
        "patch",
    )
}

pub(crate) fn fill_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_max_outputs("fill", context, 1)?;
    let (axes, arguments) = optional_axes("fill", arguments)?;
    let (patch, properties) = prepare_patch("fill", arguments, PatchConstructor::Fill)?;
    created_handle(
        context,
        GraphicsRequest::Patch {
            axes,
            patch,
            properties,
        },
        "fill",
    )
}

pub(crate) fn fill3_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_max_outputs("fill3", context, 1)?;
    let (axes, arguments) = optional_axes("fill3", arguments)?;
    let (patch, properties) = prepare_patch("fill3", arguments, PatchConstructor::Fill3)?;
    created_handle(
        context,
        GraphicsRequest::Patch {
            axes,
            patch,
            properties,
        },
        "fill3",
    )
}

pub(crate) fn trisurf_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    tri_patch_builtin("trisurf", false, arguments, context)
}

pub(crate) fn trimesh_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    tri_patch_builtin("trimesh", true, arguments, context)
}

fn tri_patch_builtin(
    name: &'static str,
    wireframe: bool,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_max_outputs(name, context, 1)?;
    let (axes, arguments) = optional_axes(name, arguments)?;
    let (patch, properties) = prepare_tri_patch(name, arguments, wireframe)?;
    created_handle(
        context,
        GraphicsRequest::Patch {
            axes,
            patch,
            properties,
        },
        name,
    )
}

pub(crate) fn shading_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    let (axes, arguments) = optional_axes("shading", arguments)?;
    expect_argument_count("shading", arguments, 1)?;
    expect_max_outputs("shading", context, 0)?;
    let mode = match lowercase_ascii_text("shading", 1, &arguments[0])?.as_str() {
        "flat" => ShadingMode::Flat,
        "interp" => ShadingMode::Interp,
        "faceted" => ShadingMode::Faceted,
        _ => {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "`shading` mode must be flat, interp, or faceted",
            ));
        }
    };
    no_value(
        &context.graphics(GraphicsRequest::Shading { axes, mode })?,
        "shading",
    )
}

pub(crate) fn camlight_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    let (axes, arguments) = optional_axes("camlight", arguments)?;
    expect_argument_count_range("camlight", arguments, 0, 1)?;
    expect_max_outputs("camlight", context, 0)?;
    if let Some(value) = arguments.first()
        && lowercase_ascii_text("camlight", 1, value)? != "headlight"
    {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "`camlight` currently supports only the camera-following headlight mode",
        ));
    }
    no_value(
        &context.graphics(GraphicsRequest::SetHeadlight {
            axes,
            enabled: true,
        })?,
        "camlight",
    )
}

pub(crate) fn lighting_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    let (axes, arguments) = optional_axes("lighting", arguments)?;
    expect_argument_count("lighting", arguments, 1)?;
    expect_max_outputs("lighting", context, 0)?;
    let mode = match lowercase_ascii_text("lighting", 1, &arguments[0])?.as_str() {
        "none" => LightingMode::None,
        "flat" => LightingMode::Flat,
        "gouraud" => LightingMode::Gouraud,
        _ => {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "`lighting` mode must be none, flat, or gouraud",
            ));
        }
    };
    no_value(
        &context.graphics(GraphicsRequest::SetLighting { axes, mode })?,
        "lighting",
    )
}

pub(crate) fn isonormals_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count_range("isonormals", arguments, 2, 5)?;
    expect_max_outputs("isonormals", context, 0)?;
    if !matches!(arguments.len(), 2 | 5) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            "`isonormals` currently accepts V/Patch or X/Y/Z/V/Patch",
        ));
    }
    let (x, y, z, values, shape) = if arguments.len() == 2 {
        let (shape, values) = real_volume_data("isonormals", 1, &arguments[0])?;
        (None, None, None, values, shape)
    } else {
        let (shape, x) = real_volume_data("isonormals", 1, &arguments[0])?;
        let (y_shape, y) = real_volume_data("isonormals", 2, &arguments[1])?;
        let (z_shape, z) = real_volume_data("isonormals", 3, &arguments[2])?;
        let (value_shape, values) = real_volume_data("isonormals", 4, &arguments[3])?;
        if [y_shape, z_shape, value_shape]
            .into_iter()
            .any(|candidate| candidate != shape)
        {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "`isonormals` coordinate and scalar volumes must have identical shapes",
            ));
        }
        (Some(x), Some(y), Some(z), values, shape)
    };
    let patch = graphics_scalar(
        "isonormals",
        arguments.len(),
        arguments.last().expect("validated input count"),
        GraphicsClass::PatchSeries,
    )?;
    no_value(
        &context.graphics(GraphicsRequest::SetPatchIsoNormals {
            patch,
            volume: IsoNormalsInput {
                x,
                y,
                z,
                values,
                shape,
            },
        })?,
        "isonormals",
    )
}

fn real_volume_data(
    name: &str,
    position: usize,
    value: &Value,
) -> Result<([u64; 3], NumericData), BuiltinError> {
    let (shape, data) = match value {
        Value::Array(ArrayData::F64(array)) => (
            array.shape().dimensions(),
            NumericData::from_f64(Arc::<[f64]>::from(array.as_slice())),
        ),
        Value::Array(ArrayData::F32(array)) => (
            array.shape().dimensions(),
            NumericData::from_f32(Arc::<[f32]>::from(array.as_slice())),
        ),
        _ => {
            return Err(type_error(
                name,
                position,
                "real double or single volume",
                value,
            ));
        }
    };
    if shape.len() != 3 || shape.iter().any(|extent| *extent < 2) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("input {position} to `{name}` must be at least 2-by-2-by-2"),
        ));
    }
    let shape: [u64; 3] = shape.try_into().map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("input {position} to `{name}` must be three-dimensional"),
        )
    })?;
    Ok((shape, data))
}

pub(crate) fn colorbar_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    let (axes, arguments) = optional_axes("colorbar", arguments)?;
    expect_argument_count_range("colorbar", arguments, 0, 1)?;
    expect_max_outputs("colorbar", context, 1)?;
    let visible = arguments
        .first()
        .map_or(Ok(true), |value| on_off_mode("colorbar", 1, value))?;
    let response = context.graphics(GraphicsRequest::Colorbar { axes, visible })?;
    if visible {
        one_handle_if_requested(&response, "colorbar", context.requested_outputs())
    } else if context.requested_outputs() == 0 {
        no_value(&response, "colorbar")
    } else {
        Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            "`colorbar off` does not return a handle",
        ))
    }
}

fn predefined_colormap_builtin(
    colormap: PredefinedColormap,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    let name = colormap.name();
    if colormap.supports_length() {
        expect_argument_count_range(name, arguments, 0, 1)?;
    } else {
        expect_argument_count(name, arguments, 0)?;
    }
    expect_max_outputs(name, context, 1)?;
    let length = if colormap.supports_length() {
        arguments.first().map_or_else(
            || {
                Ok(context
                    .current_colormap_length()
                    .unwrap_or(DEFAULT_COLORMAP_LENGTH))
            },
            |value| colormap_length(name, value),
        )?
    } else {
        16
    };
    context.check_cancelled()?;
    let value = colormap_value(&colormap.generate(length))?;
    context.check_cancelled()?;
    Ok((context.requested_outputs() != 0)
        .then_some(value)
        .into_iter()
        .collect())
}

macro_rules! define_predefined_colormap_builtins {
    ($(($function:ident, $variant:ident)),+ $(,)?) => {
        $(
            pub(crate) fn $function(
                arguments: &[Value],
                context: &mut BuiltinContext<'_>,
            ) -> BuiltinResult {
                predefined_colormap_builtin(PredefinedColormap::$variant, arguments, context)
            }
        )+
    };
}

define_predefined_colormap_builtins!(
    (parula_builtin, Parula),
    (turbo_builtin, Turbo),
    (hsv_builtin, Hsv),
    (hot_builtin, Hot),
    (cool_builtin, Cool),
    (spring_builtin, Spring),
    (summer_builtin, Summer),
    (autumn_builtin, Autumn),
    (winter_builtin, Winter),
    (gray_builtin, Gray),
    (bone_builtin, Bone),
    (copper_builtin, Copper),
    (pink_builtin, Pink),
    (jet_builtin, Jet),
    (lines_builtin, Lines),
    (colorcube_builtin, Colorcube),
    (prism_builtin, Prism),
    (flag_builtin, Flag),
    (white_builtin, White),
    (vga_builtin, Vga),
);

pub(crate) fn colormap_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    let (axes, arguments) = optional_axes("colormap", arguments)?;
    expect_argument_count_range("colormap", arguments, 0, 1)?;
    expect_max_outputs("colormap", context, 1)?;
    let response = if let Some(argument) = arguments.first() {
        let colors = if matches!(
            argument,
            Value::Array(ArrayData::Char(_)) | Value::String(_)
        ) {
            let name = lowercase_ascii_text("colormap", 1, argument)?;
            if name == "default" {
                parula_r2022b(DEFAULT_COLORMAP_LENGTH)
            } else if let Some(colormap) = PredefinedColormap::from_name(&name) {
                if colormap.supports_length() {
                    let GraphicsResponse::Colormap(current) =
                        context.graphics(GraphicsRequest::GetColormap { axes })?
                    else {
                        return Err(graphics_state_error("colormap", "current colormap"));
                    };
                    colormap.generate(current.len())
                } else {
                    colormap.generate(16)
                }
            } else {
                return Err(BuiltinError::new(
                    BuiltinErrorCategory::Domain,
                    "input 1 to `colormap` must name an R2022b predefined colormap or be an m-by-3 RGB matrix",
                ));
            }
        } else {
            colormap_matrix(argument)?
        };
        context.graphics(GraphicsRequest::SetColormap { axes, colors })?
    } else {
        context.graphics(GraphicsRequest::GetColormap { axes })?
    };
    let GraphicsResponse::Colormap(colors) = response else {
        return Err(graphics_state_error("colormap", "RGB matrix"));
    };
    let value = colormap_value(&colors)?;
    Ok((context.requested_outputs() != 0)
        .then_some(value)
        .into_iter()
        .collect())
}

fn colormap_length(name: &str, value: &Value) -> Result<usize, BuiltinError> {
    let length = if let Some(component) = exact_real_integer_scalar(value) {
        match component {
            IntegerComponent::Signed(value) => usize::try_from(value).ok(),
            IntegerComponent::Unsigned(value) => usize::try_from(value).ok(),
        }
    } else {
        let value = real_scalar(name, 1, value)?;
        if value.is_finite()
            && value >= 0.0
            && value.fract() == 0.0
            && value < U64_EXCLUSIVE_UPPER_BOUND
        {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            usize::try_from(value as u64).ok()
        } else {
            None
        }
    }
    .filter(|length| *length <= 1_000_000);
    length.ok_or_else(|| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("input 1 to `{name}` must be a nonnegative integer no greater than 1000000"),
        )
    })
}

fn colormap_matrix(value: &Value) -> Result<Vec<[f64; 3]>, BuiltinError> {
    let matrix = PreparedNumeric::from_value("colormap", 1, value)?;
    if matrix.rows == 0 || matrix.columns != 3 {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "input 1 to `colormap` must be a nonempty m-by-3 RGB matrix",
        ));
    }
    let values = matrix.vector_f64();
    let colors = (0..matrix.rows)
        .map(|row| {
            [
                values[row],
                values[matrix.rows + row],
                values[2 * matrix.rows + row],
            ]
        })
        .collect::<Vec<_>>();
    if colors
        .iter()
        .flatten()
        .any(|component| !component.is_finite() || !(0.0..=1.0).contains(component))
    {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "`colormap` RGB components must be finite values in [0, 1]",
        ));
    }
    Ok(colors)
}

fn colormap_value(colors: &[[f64; 3]]) -> Result<Value, BuiltinError> {
    let rows = u64::try_from(colors.len()).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Other,
            "colormap row count exceeds language limits",
        )
    })?;
    let shape = Shape::new([rows, 3])
        .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Other, error.to_string()))?;
    let mut values = Vec::with_capacity(colors.len() * 3);
    for column in 0..3 {
        values.extend(colors.iter().map(|color| color[column]));
    }
    DenseArray::from_vec(shape, values)
        .map(ArrayData::F64)
        .map(Value::Array)
        .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Other, error.to_string()))
}

fn surface_builtin(
    name: &str,
    style: SurfaceStyle,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    let (axes, arguments) = optional_axes(name, arguments)?;
    expect_max_outputs(name, context, 1)?;
    let property_start = arguments
        .iter()
        .position(|value| matches!(value, Value::String(_) | Value::Array(ArrayData::Char(_))))
        .unwrap_or(arguments.len());
    let (data, property_arguments) = arguments.split_at(property_start);
    if data.is_empty()
        || !matches!(data.len(), 1..=4)
        || !property_arguments.len().is_multiple_of(2)
    {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            format!(
                "`{name}` requires Z, Z/C, X/Y/Z, or X/Y/Z/CData followed by complete property name/value pairs"
            ),
        ));
    }
    let surface = prepare_surface(name, data)?;
    let mut properties = Vec::with_capacity(property_arguments.len() / 2);
    for (pair, values) in property_arguments.chunks_exact(2).enumerate() {
        let name_position = property_start + pair * 2 + 1;
        properties.push(parse_property_update(
            name,
            name_position,
            name_position + 1,
            GraphicsClass::SurfaceSeries,
            &values[0],
            &values[1],
        )?);
    }
    created_handle(
        context,
        GraphicsRequest::Surface {
            axes,
            surface,
            style,
            properties,
        },
        name,
    )
}

pub(crate) fn view_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    let (axes, arguments) = optional_axes("view", arguments)?;
    expect_argument_count_range("view", arguments, 0, 2)?;
    expect_max_outputs("view", context, 2)?;
    if arguments.is_empty() {
        let GraphicsResponse::PropertyValues(values) =
            context.graphics(GraphicsRequest::GetAxesProperty {
                axes,
                property: GraphicsProperty::View,
            })?
        else {
            return Err(graphics_state_error("view", "one Axes View value"));
        };
        let Some(GraphicsPropertyValue::NumericMatrix { values, .. }) = values.first() else {
            return Err(graphics_state_error("view", "one Axes View value"));
        };
        if values.len() != 2 {
            return Err(graphics_state_error("view", "azimuth/elevation pair"));
        }
        return match context.requested_outputs() {
            0 => Ok(Vec::new()),
            1 => Err(BuiltinError::new(
                BuiltinErrorCategory::Other,
                "one-output `view` transform matrices are not implemented yet; use `[az,el] = view` or `get(gca,'View')`",
            )),
            2 => Ok(vec![Value::Double(values[0]), Value::Double(values[1])]),
            _ => unreachable!("validated output count"),
        };
    }
    if context.requested_outputs() != 0 {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            "`view` does not return values while setting a view",
        ));
    }
    let (azimuth_degrees, elevation_degrees) = if arguments.len() == 2 {
        (
            real_scalar("view", 1, &arguments[0])?,
            real_scalar("view", 2, &arguments[1])?,
        )
    } else {
        let value = PreparedNumeric::from_value("view", 1, &arguments[0])?;
        if value.numel() == 1 {
            match value.vector_f64()[0] {
                2.0 => (0.0, 90.0),
                3.0 => (-37.5, 30.0),
                _ => {
                    return Err(BuiltinError::new(
                        BuiltinErrorCategory::Domain,
                        "scalar input to `view` must be 2 or 3",
                    ));
                }
            }
        } else if value.is_vector() && value.numel() == 2 {
            let values = value.vector_f64();
            (values[0], values[1])
        } else {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "`view` requires [azimuth elevation], two scalars, 2, or 3",
            ));
        }
    };
    no_value(
        &context.graphics(GraphicsRequest::SetView {
            axes,
            azimuth_degrees,
            elevation_degrees,
        })?,
        "view",
    )
}

pub(crate) fn daspect_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    aspect_ratio_builtin(
        "daspect",
        GraphicsProperty::DataAspectRatio,
        GraphicsPropertyUpdate::DataAspectRatio,
        GraphicsPropertyUpdate::DataAspectRatioMode,
        arguments,
        context,
    )
}

pub(crate) fn pbaspect_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    aspect_ratio_builtin(
        "pbaspect",
        GraphicsProperty::PlotBoxAspectRatio,
        GraphicsPropertyUpdate::PlotBoxAspectRatio,
        GraphicsPropertyUpdate::PlotBoxAspectRatioMode,
        arguments,
        context,
    )
}

fn aspect_ratio_builtin(
    name: &str,
    property: GraphicsProperty,
    ratio_update: impl FnOnce([f64; 3]) -> GraphicsPropertyUpdate,
    mode_update: impl FnOnce(LimitMode) -> GraphicsPropertyUpdate,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    let (axes, arguments) = optional_axes(name, arguments)?;
    expect_argument_count_range(name, arguments, 0, 1)?;
    expect_max_outputs(name, context, 1)?;
    let Some(value) = arguments.first() else {
        let GraphicsResponse::PropertyValues(values) =
            context.graphics(GraphicsRequest::GetAxesProperty { axes, property })?
        else {
            return Err(graphics_state_error(name, "one Axes aspect ratio"));
        };
        return property_values_to_output(values, true);
    };
    if context.requested_outputs() != 0 {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            format!("`{name}` does not return a value while setting"),
        ));
    }
    let update = if let Ok(mode) = lowercase_ascii_text(name, 1, value) {
        match mode.as_str() {
            "auto" => mode_update(LimitMode::Auto),
            "manual" => mode_update(LimitMode::Manual),
            _ => {
                return Err(BuiltinError::new(
                    BuiltinErrorCategory::Domain,
                    format!("`{name}` mode must be auto or manual"),
                ));
            }
        }
    } else {
        let values = PreparedNumeric::from_value(name, 1, value)?;
        if !values.is_vector() || values.numel() != 3 {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                format!("`{name}` requires a positive three-element vector"),
            ));
        }
        let values = values.vector_f64();
        let ratio = [values[0], values[1], values[2]];
        if ratio
            .iter()
            .any(|value| !value.is_finite() || *value <= 0.0)
        {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                format!("`{name}` requires a positive finite ratio"),
            ));
        }
        ratio_update(ratio)
    };
    no_value(
        &context.graphics(GraphicsRequest::SetAxesProperties {
            axes,
            properties: vec![update],
        })?,
        name,
    )
}

pub(crate) fn axis_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    let (axes, arguments) = optional_axes("axis", arguments)?;
    let Some(argument) = arguments.first() else {
        expect_max_outputs("axis", context, 1)?;
        let mut limits = axes_numeric_property(context, axes, GraphicsProperty::XLim, "axis")?;
        limits.extend(axes_numeric_property(
            context,
            axes,
            GraphicsProperty::YLim,
            "axis",
        )?);
        let view = axes_numeric_property(context, axes, GraphicsProperty::View, "axis")?;
        if view
            .get(1)
            .is_some_and(|elevation| (*elevation - 90.0).abs() > 1.0e-12)
        {
            limits.extend(axes_numeric_property(
                context,
                axes,
                GraphicsProperty::ZLim,
                "axis",
            )?);
        }
        return numeric_row_value(limits);
    };
    expect_max_outputs("axis", context, 0)?;
    let properties = if arguments.iter().all(is_text_value) {
        arguments
            .iter()
            .enumerate()
            .try_fold(Vec::new(), |mut properties, (index, argument)| {
                let mode = lowercase_ascii_text("axis", index + 1, argument)?;
                properties.extend(axis_mode_properties(&mode)?);
                Ok::<_, BuiltinError>(properties)
            })?
    } else {
        expect_argument_count("axis", arguments, 1)?;
        axis_limit_properties(argument)?
    };
    no_value(
        &context.graphics(GraphicsRequest::SetAxesProperties { axes, properties })?,
        "axis",
    )
}

fn axis_mode_properties(mode: &str) -> Result<Vec<GraphicsPropertyUpdate>, BuiltinError> {
    Ok(match mode {
        "equal" => vec![
            GraphicsPropertyUpdate::DataAspectRatio([1.0, 1.0, 1.0]),
            GraphicsPropertyUpdate::PlotBoxAspectRatioMode(LimitMode::Auto),
        ],
        "normal" | "fill" => vec![
            GraphicsPropertyUpdate::DataAspectRatioMode(LimitMode::Auto),
            GraphicsPropertyUpdate::PlotBoxAspectRatioMode(LimitMode::Auto),
        ],
        "vis3d" => vec![
            GraphicsPropertyUpdate::DataAspectRatioMode(LimitMode::Manual),
            GraphicsPropertyUpdate::PlotBoxAspectRatioMode(LimitMode::Manual),
        ],
        "auto" => vec![
            GraphicsPropertyUpdate::XLimMode(LimitMode::Auto),
            GraphicsPropertyUpdate::YLimMode(LimitMode::Auto),
            GraphicsPropertyUpdate::ZLimMode(LimitMode::Auto),
            GraphicsPropertyUpdate::XLimitMethod(LimitMethod::TickAligned),
            GraphicsPropertyUpdate::YLimitMethod(LimitMethod::TickAligned),
            GraphicsPropertyUpdate::ZLimitMethod(LimitMethod::TickAligned),
        ],
        "manual" => vec![
            GraphicsPropertyUpdate::XLimMode(LimitMode::Manual),
            GraphicsPropertyUpdate::YLimMode(LimitMode::Manual),
            GraphicsPropertyUpdate::ZLimMode(LimitMode::Manual),
        ],
        "tight" | "padded" => {
            let method = if mode == "tight" {
                LimitMethod::Tight
            } else {
                LimitMethod::Padded
            };
            vec![
                GraphicsPropertyUpdate::XLimitMethod(method),
                GraphicsPropertyUpdate::YLimitMethod(method),
                GraphicsPropertyUpdate::ZLimitMethod(method),
                GraphicsPropertyUpdate::XLimMode(LimitMode::Auto),
                GraphicsPropertyUpdate::YLimMode(LimitMode::Auto),
                GraphicsPropertyUpdate::ZLimMode(LimitMode::Auto),
            ]
        }
        "square" => vec![
            GraphicsPropertyUpdate::DataAspectRatioMode(LimitMode::Auto),
            GraphicsPropertyUpdate::PlotBoxAspectRatio([1.0, 1.0, 1.0]),
        ],
        "image" => vec![
            GraphicsPropertyUpdate::XLimitMethod(LimitMethod::Tight),
            GraphicsPropertyUpdate::YLimitMethod(LimitMethod::Tight),
            GraphicsPropertyUpdate::ZLimitMethod(LimitMethod::Tight),
            GraphicsPropertyUpdate::XLimMode(LimitMode::Auto),
            GraphicsPropertyUpdate::YLimMode(LimitMode::Auto),
            GraphicsPropertyUpdate::ZLimMode(LimitMode::Auto),
            GraphicsPropertyUpdate::DataAspectRatio([1.0, 1.0, 1.0]),
        ],
        "on" => vec![GraphicsPropertyUpdate::Visible(true)],
        "off" => vec![GraphicsPropertyUpdate::Visible(false)],
        "ij" => vec![GraphicsPropertyUpdate::YDir(AxisDirection::Reverse)],
        "xy" => vec![GraphicsPropertyUpdate::YDir(AxisDirection::Normal)],
        _ => {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "unsupported `axis` mode",
            ));
        }
    })
}

fn axis_limit_properties(value: &Value) -> Result<Vec<GraphicsPropertyUpdate>, BuiltinError> {
    let values = PreparedNumeric::from_value("axis", 1, value)?;
    if !values.is_vector() || !matches!(values.numel(), 2 | 4 | 6 | 8) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "`axis` requires a real limit vector with 2, 4, 6, or 8 elements",
        ));
    }
    values
        .vector_f64()
        .chunks_exact(2)
        .enumerate()
        .map(|(index, pair)| {
            let pair = validated_limit_pair_values("axis", pair[0], pair[1])?;
            Ok(match index {
                0 => GraphicsPropertyUpdate::XLim(pair),
                1 => GraphicsPropertyUpdate::YLim(pair),
                2 => GraphicsPropertyUpdate::ZLim(pair),
                3 => GraphicsPropertyUpdate::CLim(pair),
                _ => unreachable!(),
            })
        })
        .collect()
}

pub(crate) fn hold_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count_range("hold", arguments, 1, 2)?;
    expect_max_outputs("hold", context, 0)?;
    let (axes, mode_position) = if arguments.len() == 2 {
        (
            Some(graphics_scalar(
                "hold",
                1,
                &arguments[0],
                GraphicsClass::Axes2D,
            )?),
            2,
        )
    } else {
        (None, 1)
    };
    let enabled = on_off_mode("hold", mode_position, &arguments[mode_position - 1])?;
    no_value(
        &context.graphics(GraphicsRequest::SetHold { axes, enabled })?,
        "hold",
    )
}

pub(crate) fn ishold_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count_range("ishold", arguments, 0, 1)?;
    expect_max_outputs("ishold", context, 1)?;
    let axes = arguments
        .first()
        .map(|value| graphics_scalar("ishold", 1, value, GraphicsClass::Axes2D))
        .transpose()?;
    let GraphicsResponse::Logical(value) = context.graphics(GraphicsRequest::IsHold { axes })?
    else {
        return Err(graphics_state_error("ishold", "logical result"));
    };
    if arguments.is_empty() {
        Ok(vec![Value::Double(if value { 1.0 } else { 0.0 })])
    } else {
        Ok(vec![Value::Logical(value)])
    }
}

pub(crate) fn isgraphics_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count_range("isgraphics", arguments, 1, 2)?;
    expect_max_outputs("isgraphics", context, 1)?;
    let requested_type = arguments
        .get(1)
        .map(|value| lowercase_ascii_text("isgraphics", 2, value))
        .transpose()?;
    let (handles, scalar, class) = match &arguments[0] {
        Value::Graphics(handle) => (vec![*handle], true, handle.class()),
        Value::GraphicsArray(array) => {
            (array.as_slice().to_vec(), array.numel() == 1, array.class())
        }
        _ => return Ok(vec![Value::Logical(false)]),
    };
    let GraphicsResponse::Logicals(mut values) =
        context.graphics(GraphicsRequest::IsGraphics { handles })?
    else {
        return Err(graphics_state_error("isgraphics", "logical values"));
    };
    if requested_type
        .as_deref()
        .is_some_and(|requested| !graphics_type_matches(class, requested))
    {
        values.fill(false);
    }
    if scalar {
        return Ok(vec![Value::Logical(
            values.first().copied().unwrap_or(false),
        )]);
    }
    let shape = Shape::new([values.len() as u64, 1])
        .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Other, error.to_string()))?;
    let array = DenseArray::from_vec(shape, values.into_iter().map(Logical::from).collect())
        .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Other, error.to_string()))?;
    Ok(vec![Value::Array(ArrayData::Logical(array))])
}

fn graphics_type_matches(class: GraphicsClass, requested: &str) -> bool {
    let short_name = match class {
        GraphicsClass::Figure => "figure",
        GraphicsClass::Axes2D => "axes",
        GraphicsClass::PolarAxes => "polaraxes",
        GraphicsClass::TiledChartLayout => "tiledlayout",
        GraphicsClass::LineSeries => "line",
        GraphicsClass::ScatterSeries => "scatter",
        GraphicsClass::SurfaceSeries => "surface",
        GraphicsClass::PatchSeries => "patch",
        GraphicsClass::Stair => "stair",
        GraphicsClass::Stem => "stem",
        GraphicsClass::ErrorBar => "errorbar",
        GraphicsClass::Area => "area",
        GraphicsClass::Bar => "bar",
        GraphicsClass::Histogram => "histogram",
        GraphicsClass::Contour => "contour",
        GraphicsClass::Image => "image",
        GraphicsClass::Text => "text",
        GraphicsClass::Legend => "legend",
        GraphicsClass::ColorBar => "colorbar",
    };
    requested == short_name || requested.eq_ignore_ascii_case(class.class_name())
}

pub(crate) fn get_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count("get", arguments, 2)?;
    expect_max_outputs("get", context, 1)?;
    let property_name = lowercase_ascii_text("get", 2, &arguments[1])?;
    if arguments[0].as_real_number() == Some(0.0) && "currentfigure".starts_with(&property_name) {
        let GraphicsResponse::Handles(handles) =
            context.graphics(GraphicsRequest::CurrentFigureOrEmpty)?
        else {
            return Err(graphics_state_error("get", "root CurrentFigure handle"));
        };
        return Ok(vec![
            handles
                .first()
                .copied()
                .map_or_else(Value::empty_double, Value::Graphics),
        ]);
    }
    let targets = graphics_property_targets("get", 1, &arguments[0])?;
    let property = graphics_property("get", targets.class, &property_name)?;
    let GraphicsResponse::PropertyValues(values) =
        context.graphics(GraphicsRequest::GetProperties {
            handles: targets.handles,
            property,
        })?
    else {
        return Err(graphics_state_error("get", "property values"));
    };
    property_values_to_output(values, targets.scalar)
}

pub(crate) fn set_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    if arguments.len() < 3 || arguments.len().is_multiple_of(2) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            "`set` requires a graphics handle followed by one or more name/value pairs",
        ));
    }
    expect_max_outputs("set", context, 0)?;
    let targets = graphics_property_targets("set", 1, &arguments[0])?;
    let mut properties = Vec::with_capacity((arguments.len() - 1) / 2);
    for pair in 0..((arguments.len() - 1) / 2) {
        let name_position = 2 + pair * 2;
        let value_position = name_position + 1;
        properties.push(parse_property_update(
            "set",
            name_position,
            value_position,
            targets.class,
            &arguments[name_position - 1],
            &arguments[value_position - 1],
        )?);
    }
    no_value(
        &context.graphics(GraphicsRequest::SetProperties {
            handles: targets.handles,
            properties,
        })?,
        "set",
    )
}

pub(crate) fn title_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    text_builtin("title", TextRole::Title, arguments, context)
}

pub(crate) fn xlabel_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    text_builtin("xlabel", TextRole::XLabel, arguments, context)
}

pub(crate) fn ylabel_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    text_builtin("ylabel", TextRole::YLabel, arguments, context)
}

pub(crate) fn zlabel_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    text_builtin("zlabel", TextRole::ZLabel, arguments, context)
}

pub(crate) fn legend_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_max_outputs("legend", context, 1)?;
    let (axes, arguments) = optional_axes("legend", arguments)?;
    if arguments.is_empty()
        || matches!(arguments, [value] if lowercase_ascii_text("legend", 1, value).is_ok_and(|text| text == "show"))
    {
        return created_handle(
            context,
            GraphicsRequest::Legend {
                axes,
                labels_utf16: Vec::new(),
                properties: vec![GraphicsPropertyUpdate::Visible(true)],
            },
            "legend",
        );
    }
    let property_start = if matches!(arguments.first(), Some(Value::Cell(_))) {
        1
    } else {
        arguments
            .iter()
            .enumerate()
            .find_map(|(index, value)| {
                lowercase_ascii_text("legend", index + 1, value)
                    .ok()
                    .filter(|name| is_legend_property_name(name))
                    .map(|_| index)
            })
            .unwrap_or(arguments.len())
    };
    let label_arguments = &arguments[..property_start];
    let property_arguments = &arguments[property_start..];
    if !property_arguments.len().is_multiple_of(2) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            "`legend` property names must be followed by values",
        ));
    }
    let labels_utf16 = if let [Value::Cell(labels)] = label_arguments {
        labels
            .values()
            .iter()
            .enumerate()
            .map(|(index, value)| text_scalar("legend", index + 1, value))
            .collect::<Result<Vec<_>, _>>()?
    } else {
        label_arguments
            .iter()
            .enumerate()
            .map(|(index, value)| text_scalar("legend", index + 1, value))
            .collect::<Result<Vec<_>, _>>()?
    };
    let mut properties = Vec::with_capacity(property_arguments.len() / 2);
    for pair in 0..(property_arguments.len() / 2) {
        let name_position = property_start + pair * 2 + 1;
        let value_position = name_position + 1;
        properties.push(parse_property_update(
            "legend",
            name_position,
            value_position,
            GraphicsClass::Legend,
            &property_arguments[pair * 2],
            &property_arguments[pair * 2 + 1],
        )?);
    }
    created_handle(
        context,
        GraphicsRequest::Legend {
            axes,
            labels_utf16,
            properties,
        },
        "legend",
    )
}

pub(crate) fn xlim_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    limits_builtin("xlim", Axis::X, arguments, context)
}

pub(crate) fn ylim_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    limits_builtin("ylim", Axis::Y, arguments, context)
}

pub(crate) fn zlim_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    limits_builtin("zlim", Axis::Z, arguments, context)
}

pub(crate) fn thetalim_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    polar_limits_builtin(
        "thetalim",
        GraphicsProperty::ThetaLim,
        GraphicsProperty::ThetaLimMode,
        GraphicsPropertyUpdate::XLim,
        GraphicsPropertyUpdate::XLimMode,
        arguments,
        context,
    )
}

pub(crate) fn rlim_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    polar_limits_builtin(
        "rlim",
        GraphicsProperty::RLim,
        GraphicsProperty::RLimMode,
        GraphicsPropertyUpdate::YLim,
        GraphicsPropertyUpdate::YLimMode,
        arguments,
        context,
    )
}

pub(crate) fn clim_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    color_limits_builtin("clim", arguments, context)
}

pub(crate) fn caxis_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    color_limits_builtin("caxis", arguments, context)
}

pub(crate) fn grid_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    let (axes, arguments) = optional_axes("grid", arguments)?;
    expect_argument_count("grid", arguments, 1)?;
    expect_max_outputs("grid", context, 0)?;
    let mode = if on_off_mode("grid", 1, &arguments[0])? {
        GridMode::On
    } else {
        GridMode::Off
    };
    no_value(
        &context.graphics(GraphicsRequest::SetGrid { axes, mode })?,
        "grid",
    )
}

// Registration is intentionally owned by the integration task.
#[allow(dead_code)]
pub(crate) fn xticks_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    ticks_builtin(
        "xticks",
        Axis::X,
        GraphicsProperty::XTick,
        GraphicsProperty::XTickMode,
        arguments,
        context,
    )
}

#[allow(dead_code)]
pub(crate) fn xticklabels_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    tick_labels_builtin(
        "xticklabels",
        Axis::X,
        GraphicsProperty::XTickLabel,
        GraphicsProperty::XTickLabelMode,
        arguments,
        context,
    )
}

pub(crate) fn yticks_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    ticks_builtin(
        "yticks",
        Axis::Y,
        GraphicsProperty::YTick,
        GraphicsProperty::YTickMode,
        arguments,
        context,
    )
}

pub(crate) fn yticklabels_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    tick_labels_builtin(
        "yticklabels",
        Axis::Y,
        GraphicsProperty::YTickLabel,
        GraphicsProperty::YTickLabelMode,
        arguments,
        context,
    )
}

pub(crate) fn zticks_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    ticks_builtin(
        "zticks",
        Axis::Z,
        GraphicsProperty::ZTick,
        GraphicsProperty::ZTickMode,
        arguments,
        context,
    )
}

pub(crate) fn zticklabels_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    tick_labels_builtin(
        "zticklabels",
        Axis::Z,
        GraphicsProperty::ZTickLabel,
        GraphicsProperty::ZTickLabelMode,
        arguments,
        context,
    )
}

pub(crate) fn thetaticks_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    polar_ticks_builtin(
        "thetaticks",
        GraphicsProperty::ThetaTick,
        GraphicsProperty::ThetaTickMode,
        GraphicsPropertyUpdate::XTick,
        GraphicsPropertyUpdate::XTickMode,
        arguments,
        context,
    )
}

pub(crate) fn rticks_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    polar_ticks_builtin(
        "rticks",
        GraphicsProperty::RTick,
        GraphicsProperty::RTickMode,
        GraphicsPropertyUpdate::YTick,
        GraphicsPropertyUpdate::YTickMode,
        arguments,
        context,
    )
}

pub(crate) fn thetaticklabels_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    polar_tick_labels_builtin(
        "thetaticklabels",
        GraphicsProperty::ThetaTickLabel,
        GraphicsProperty::ThetaTickLabelMode,
        GraphicsPropertyUpdate::XTickLabel,
        GraphicsPropertyUpdate::XTickLabelMode,
        arguments,
        context,
    )
}

pub(crate) fn rticklabels_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    polar_tick_labels_builtin(
        "rticklabels",
        GraphicsProperty::RTickLabel,
        GraphicsProperty::RTickLabelMode,
        GraphicsPropertyUpdate::YTickLabel,
        GraphicsPropertyUpdate::YTickLabelMode,
        arguments,
        context,
    )
}

fn polar_limits_builtin(
    name: &str,
    limits_property: GraphicsProperty,
    mode_property: GraphicsProperty,
    limits_update: fn([f64; 2]) -> GraphicsPropertyUpdate,
    mode_update: fn(LimitMode) -> GraphicsPropertyUpdate,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    let (axes, arguments) = optional_polar_axes(name, arguments)?;
    expect_argument_count_range(name, arguments, 0, 1)?;
    let axes = required_polar_axes(name, axes, context)?;
    let Some(value) = arguments.first() else {
        expect_max_outputs(name, context, 1)?;
        return one_object_property(context, axes, limits_property, name);
    };
    if let Ok(command) = lowercase_ascii_text(name, 1, value) {
        if command == "mode" {
            expect_max_outputs(name, context, 1)?;
            return one_object_property(context, axes, mode_property, name);
        }
        if matches!(command.as_str(), "auto" | "manual") {
            expect_max_outputs(name, context, 0)?;
            return set_object_properties(
                context,
                axes,
                vec![mode_update(if command == "auto" {
                    LimitMode::Auto
                } else {
                    LimitMode::Manual
                })],
                name,
            );
        }
    }
    expect_max_outputs(name, context, 0)?;
    let limits = limit_pair(name, 1, property_display_name(limits_property), value)?;
    set_object_properties(context, axes, vec![limits_update(limits)], name)
}

fn polar_ticks_builtin(
    name: &str,
    tick_property: GraphicsProperty,
    mode_property: GraphicsProperty,
    tick_update: fn(Vec<f64>) -> GraphicsPropertyUpdate,
    mode_update: fn(TickMode) -> GraphicsPropertyUpdate,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    let (axes, arguments) = optional_polar_axes(name, arguments)?;
    expect_argument_count_range(name, arguments, 0, 1)?;
    let axes = required_polar_axes(name, axes, context)?;
    let Some(value) = arguments.first() else {
        expect_max_outputs(name, context, 1)?;
        return one_object_property(context, axes, tick_property, name);
    };
    if let Ok(command) = lowercase_ascii_text(name, 1, value) {
        if command == "mode" {
            expect_max_outputs(name, context, 1)?;
            return one_object_property(context, axes, mode_property, name);
        }
        if matches!(command.as_str(), "auto" | "manual") {
            expect_max_outputs(name, context, 0)?;
            return set_object_properties(
                context,
                axes,
                vec![mode_update(tick_mode(name, 1, value)?)],
                name,
            );
        }
    }
    expect_max_outputs(name, context, 0)?;
    set_object_properties(
        context,
        axes,
        vec![tick_update(tick_values(name, 1, value)?)],
        name,
    )
}

fn polar_tick_labels_builtin(
    name: &str,
    label_property: GraphicsProperty,
    mode_property: GraphicsProperty,
    label_update: fn(Vec<Vec<u16>>) -> GraphicsPropertyUpdate,
    mode_update: fn(TickMode) -> GraphicsPropertyUpdate,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    let (axes, arguments) = optional_polar_axes(name, arguments)?;
    expect_argument_count_range(name, arguments, 0, 1)?;
    let axes = required_polar_axes(name, axes, context)?;
    let Some(value) = arguments.first() else {
        expect_max_outputs(name, context, 1)?;
        return one_object_property(context, axes, label_property, name);
    };
    if let Ok(command) = lowercase_ascii_text(name, 1, value) {
        if command == "mode" {
            expect_max_outputs(name, context, 1)?;
            return one_object_property(context, axes, mode_property, name);
        }
        if matches!(command.as_str(), "auto" | "manual") {
            expect_max_outputs(name, context, 0)?;
            return set_object_properties(
                context,
                axes,
                vec![mode_update(tick_mode(name, 1, value)?)],
                name,
            );
        }
    }
    expect_max_outputs(name, context, 0)?;
    set_object_properties(
        context,
        axes,
        vec![label_update(tick_labels(name, 1, value)?)],
        name,
    )
}

fn one_object_property(
    context: &mut BuiltinContext<'_>,
    handle: GraphicsHandle,
    property: GraphicsProperty,
    name: &str,
) -> BuiltinResult {
    let GraphicsResponse::PropertyValues(values) =
        context.graphics(GraphicsRequest::GetProperties {
            handles: vec![handle],
            property,
        })?
    else {
        return Err(graphics_state_error(name, "one graphics property"));
    };
    property_values_to_output(values, true)
}

fn set_object_properties(
    context: &mut BuiltinContext<'_>,
    handle: GraphicsHandle,
    properties: Vec<GraphicsPropertyUpdate>,
    name: &str,
) -> BuiltinResult {
    no_value(
        &context.graphics(GraphicsRequest::SetProperties {
            handles: vec![handle],
            properties,
        })?,
        name,
    )
}

#[allow(dead_code)]
pub(crate) fn box_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    let (axes, arguments) = optional_axes("box", arguments)?;
    expect_argument_count_range("box", arguments, 0, 1)?;
    expect_max_outputs("box", context, 0)?;
    let request = if let Some(value) = arguments.first() {
        GraphicsRequest::SetAxesProperties {
            axes,
            properties: vec![GraphicsPropertyUpdate::Box(on_off_mode("box", 1, value)?)],
        }
    } else {
        GraphicsRequest::ToggleAxesBox { axes }
    };
    no_value(&context.graphics(request)?, "box")
}

#[allow(dead_code)]
fn ticks_builtin(
    name: &str,
    target_axis: Axis,
    value_property: GraphicsProperty,
    mode_property: GraphicsProperty,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    let (axes, arguments) = optional_axes(name, arguments)?;
    expect_argument_count_range(name, arguments, 0, 1)?;
    let Some(value) = arguments.first() else {
        expect_max_outputs(name, context, 1)?;
        return one_axes_property(context, axes, value_property, name);
    };
    if let Ok(command) = lowercase_ascii_text(name, 1, value) {
        match command.as_str() {
            "mode" => {
                expect_max_outputs(name, context, 1)?;
                return one_axes_property(context, axes, mode_property, name);
            }
            "auto" | "manual" => {
                expect_max_outputs(name, context, 0)?;
                return no_value(
                    &context.graphics(GraphicsRequest::SetAxesProperties {
                        axes,
                        properties: vec![tick_mode(name, 1, value).map(
                            |mode| match target_axis {
                                Axis::X => GraphicsPropertyUpdate::XTickMode(mode),
                                Axis::Y => GraphicsPropertyUpdate::YTickMode(mode),
                                Axis::Z => GraphicsPropertyUpdate::ZTickMode(mode),
                            },
                        )?],
                    })?,
                    name,
                );
            }
            _ => {}
        }
    }
    expect_max_outputs(name, context, 0)?;
    let values = tick_values(name, 1, value)?;
    let update = match target_axis {
        Axis::X => GraphicsPropertyUpdate::XTick(values),
        Axis::Y => GraphicsPropertyUpdate::YTick(values),
        Axis::Z => GraphicsPropertyUpdate::ZTick(values),
    };
    no_value(
        &context.graphics(GraphicsRequest::SetAxesProperties {
            axes,
            properties: vec![update],
        })?,
        name,
    )
}

#[allow(dead_code)]
fn tick_labels_builtin(
    name: &str,
    target_axis: Axis,
    value_property: GraphicsProperty,
    mode_property: GraphicsProperty,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    let (axes, arguments) = optional_axes(name, arguments)?;
    expect_argument_count_range(name, arguments, 0, 1)?;
    let Some(value) = arguments.first() else {
        expect_max_outputs(name, context, 1)?;
        return one_axes_property(context, axes, value_property, name);
    };
    if let Ok(command) = lowercase_ascii_text(name, 1, value) {
        match command.as_str() {
            "mode" => {
                expect_max_outputs(name, context, 1)?;
                return one_axes_property(context, axes, mode_property, name);
            }
            "auto" | "manual" => {
                expect_max_outputs(name, context, 0)?;
                let mode = tick_mode(name, 1, value)?;
                return no_value(
                    &context.graphics(GraphicsRequest::SetAxesProperties {
                        axes,
                        properties: vec![match target_axis {
                            Axis::X => GraphicsPropertyUpdate::XTickLabelMode(mode),
                            Axis::Y => GraphicsPropertyUpdate::YTickLabelMode(mode),
                            Axis::Z => GraphicsPropertyUpdate::ZTickLabelMode(mode),
                        }],
                    })?,
                    name,
                );
            }
            _ => {}
        }
    }
    expect_max_outputs(name, context, 0)?;
    let mut labels = tick_labels(name, 1, value)?;
    let tick_property = match target_axis {
        Axis::X => GraphicsProperty::XTick,
        Axis::Y => GraphicsProperty::YTick,
        Axis::Z => GraphicsProperty::ZTick,
    };
    let tick_count = axes_numeric_property(context, axes, tick_property, name)?.len();
    labels.resize(tick_count, Vec::new());
    labels.truncate(tick_count);
    let update = match target_axis {
        Axis::X => GraphicsPropertyUpdate::XTickLabel(labels),
        Axis::Y => GraphicsPropertyUpdate::YTickLabel(labels),
        Axis::Z => GraphicsPropertyUpdate::ZTickLabel(labels),
    };
    no_value(
        &context.graphics(GraphicsRequest::SetAxesProperties {
            axes,
            properties: vec![update],
        })?,
        name,
    )
}

#[allow(dead_code)]
fn one_axes_property(
    context: &mut BuiltinContext<'_>,
    axes: Option<GraphicsHandle>,
    property: GraphicsProperty,
    name: &str,
) -> BuiltinResult {
    let GraphicsResponse::PropertyValues(values) =
        context.graphics(GraphicsRequest::GetAxesProperty { axes, property })?
    else {
        return Err(graphics_state_error(name, "one Axes property value"));
    };
    property_values_to_output(values, true)
}

fn color_limits_builtin(
    name: &str,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    let (axes, arguments) = optional_axes(name, arguments)?;
    expect_argument_count_range(name, arguments, 0, 1)?;
    let Some(value) = arguments.first() else {
        expect_max_outputs(name, context, 1)?;
        return one_axes_property(context, axes, GraphicsProperty::CLim, name);
    };
    if let Ok(command) = lowercase_ascii_text(name, 1, value) {
        if command == "mode" {
            expect_max_outputs(name, context, 1)?;
            return one_axes_property(context, axes, GraphicsProperty::CLimMode, name);
        }
        if matches!(command.as_str(), "auto" | "manual") {
            expect_max_outputs(name, context, 0)?;
            let mode = if command == "auto" {
                LimitMode::Auto
            } else {
                LimitMode::Manual
            };
            return no_value(
                &context.graphics(GraphicsRequest::SetAxesProperties {
                    axes,
                    properties: vec![GraphicsPropertyUpdate::CLimMode(mode)],
                })?,
                name,
            );
        }
    }
    expect_max_outputs(name, context, 0)?;
    let pair = limit_pair(name, 1, "CLim", value)?;
    no_value(
        &context.graphics(GraphicsRequest::SetAxesProperties {
            axes,
            properties: vec![GraphicsPropertyUpdate::CLim(pair)],
        })?,
        name,
    )
}

fn axes_numeric_property(
    context: &mut BuiltinContext<'_>,
    axes: Option<GraphicsHandle>,
    property: GraphicsProperty,
    name: &str,
) -> Result<Vec<f64>, BuiltinError> {
    let GraphicsResponse::PropertyValues(values) =
        context.graphics(GraphicsRequest::GetAxesProperty { axes, property })?
    else {
        return Err(graphics_state_error(name, "one numeric Axes property"));
    };
    let [GraphicsPropertyValue::NumericMatrix { values, .. }] = values.as_slice() else {
        return Err(graphics_state_error(name, "one numeric Axes property"));
    };
    Ok(values.clone())
}

fn numeric_row_value(values: Vec<f64>) -> BuiltinResult {
    let columns = u64::try_from(values.len()).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Other,
            "numeric row exceeds host limits",
        )
    })?;
    let shape = Shape::new([1, columns])
        .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Other, error.to_string()))?;
    let array = DenseArray::from_vec(shape, values)
        .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Other, error.to_string()))?;
    Ok(vec![Value::Array(ArrayData::F64(array))])
}

pub(crate) fn cla_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count_range("cla", arguments, 0, 1)?;
    expect_max_outputs("cla", context, 0)?;
    let axes = arguments
        .first()
        .map(|value| graphics_scalar("cla", 1, value, GraphicsClass::Axes2D))
        .transpose()?;
    no_value(
        &context.graphics(GraphicsRequest::ClearAxes { axes })?,
        "cla",
    )
}

pub(crate) fn clf_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count_range("clf", arguments, 0, 1)?;
    expect_max_outputs("clf", context, 0)?;
    let figure = arguments
        .first()
        .map(|value| graphics_scalar("clf", 1, value, GraphicsClass::Figure))
        .transpose()?;
    no_value(
        &context.graphics(GraphicsRequest::ClearFigure { figure })?,
        "clf",
    )
}

pub(crate) fn close_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_max_outputs("close", context, 0)?;
    let request = match arguments {
        [] => GraphicsRequest::CloseFigure { figure: None },
        [value] if !is_text_value(value) => GraphicsRequest::CloseFigure {
            figure: Some(graphics_scalar("close", 1, value, GraphicsClass::Figure)?),
        },
        _ => close_command_request(arguments)?,
    };
    no_value(&context.graphics(request)?, "close")
}

fn text_builtin(
    name: &str,
    role: TextRole,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    let (target_axes, arguments) = optional_axes(name, arguments)?;
    if arguments.is_empty() || arguments.len().is_multiple_of(2) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            format!("`{name}` requires text followed by name/value pairs"),
        ));
    }
    expect_max_outputs(name, context, 1)?;
    let code_units = text_scalar(name, 1, &arguments[0])?;
    let mut properties = Vec::new();
    for pair in 0..((arguments.len() - 1) / 2) {
        let name_position = pair * 2 + 2;
        let value_position = name_position + 1;
        properties.push(parse_property_update(
            name,
            name_position,
            value_position,
            GraphicsClass::Text,
            &arguments[name_position - 1],
            &arguments[value_position - 1],
        )?);
    }
    created_handle(
        context,
        GraphicsRequest::SetText {
            axes: target_axes,
            role,
            code_units,
            properties,
        },
        name,
    )
}

fn limits_builtin(
    name: &str,
    axis: Axis,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    let (target_axes, arguments) = optional_axes(name, arguments)?;
    expect_argument_count_range(name, arguments, 0, 1)?;
    expect_max_outputs(name, context, 1)?;
    if let Some(argument) = arguments.first()
        && let Ok(mode) = lowercase_ascii_text(name, 1, argument)
    {
        let mode = match mode.as_str() {
            "auto" => LimitMode::Auto,
            "manual" => LimitMode::Manual,
            _ => {
                return Err(type_error(
                    name,
                    1,
                    "auto, manual, or a two-element limit vector",
                    argument,
                ));
            }
        };
        let response = context.graphics(GraphicsRequest::SetLimitMode {
            axes: target_axes,
            axis,
            mode,
        })?;
        return no_value(&response, name);
    }
    let request = if let Some(argument) = arguments.first() {
        let values = PreparedNumeric::from_value(name, 1, argument)?;
        if !values.is_vector() || values.numel() != 2 {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                format!("`{name}` v1 requires a two-element real limit vector"),
            ));
        }
        let pair = values.vector_f64();
        GraphicsRequest::SetLimits {
            axes: target_axes,
            axis,
            limits: [pair[0], pair[1]],
        }
    } else {
        GraphicsRequest::GetLimits {
            axes: target_axes,
            axis,
        }
    };
    let GraphicsResponse::Limits(limits) = context.graphics(request)? else {
        return Err(graphics_state_error(name, "limit pair"));
    };
    let shape = Shape::new([1, 2])
        .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Other, error.to_string()))?;
    let array = DenseArray::from_vec(shape, limits.to_vec())
        .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Other, error.to_string()))?;
    Ok(vec![Value::Array(ArrayData::F64(array))])
}

struct GraphicsPropertyTargets {
    class: GraphicsClass,
    handles: Vec<GraphicsHandle>,
    scalar: bool,
}

fn graphics_property_targets(
    name: &str,
    position: usize,
    value: &Value,
) -> Result<GraphicsPropertyTargets, BuiltinError> {
    let (class, handles, scalar) = match value {
        Value::Graphics(handle) => (handle.class(), vec![*handle], true),
        Value::GraphicsArray(array) => {
            (array.class(), array.as_slice().to_vec(), array.numel() == 1)
        }
        _ => {
            return Err(type_error(name, position, "graphics handle", value));
        }
    };
    Ok(GraphicsPropertyTargets {
        class,
        handles,
        scalar,
    })
}

fn graphics_property(
    name: &str,
    class: GraphicsClass,
    prefix: &str,
) -> Result<GraphicsProperty, BuiltinError> {
    if class == GraphicsClass::Figure && prefix != "color" && "color".starts_with(prefix) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Other,
            format!("`{name}` graphics property prefix `{prefix}` is ambiguous"),
        ));
    }
    if let Some(property) = ALL_PROPERTIES.iter().copied().find(|property| {
        property_display_name(*property).eq_ignore_ascii_case(prefix)
            && ensure_property_supported(name, class, *property).is_ok()
    }) {
        return Ok(property);
    }
    let matches = ALL_PROPERTIES
        .iter()
        .copied()
        .filter(|property| {
            property_display_name(*property)
                .to_ascii_lowercase()
                .starts_with(prefix)
                && ensure_property_supported(name, class, *property).is_ok()
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [property] => Ok(*property),
        [] => Err(BuiltinError::new(
            BuiltinErrorCategory::Graphics,
            format!("`{name}` does not recognize graphics property `{prefix}`"),
        )),
        _ => Err(BuiltinError::new(
            BuiltinErrorCategory::Other,
            format!("`{name}` graphics property prefix `{prefix}` is ambiguous"),
        )),
    }
}

const ALL_PROPERTIES: &[GraphicsProperty] = &[
    GraphicsProperty::Parent,
    GraphicsProperty::Children,
    GraphicsProperty::Position,
    GraphicsProperty::Name,
    GraphicsProperty::NumberTitle,
    GraphicsProperty::GridSize,
    GraphicsProperty::TileSpacing,
    GraphicsProperty::Padding,
    GraphicsProperty::LineWidth,
    GraphicsProperty::Color,
    GraphicsProperty::LineStyle,
    GraphicsProperty::Marker,
    GraphicsProperty::MarkerSize,
    GraphicsProperty::MarkerIndices,
    GraphicsProperty::DisplayName,
    GraphicsProperty::Clipping,
    GraphicsProperty::Visible,
    GraphicsProperty::SizeData,
    GraphicsProperty::CData,
    GraphicsProperty::MarkerFaceColor,
    GraphicsProperty::MarkerEdgeColor,
    GraphicsProperty::BaseValue,
    GraphicsProperty::BarWidth,
    GraphicsProperty::CapSize,
    GraphicsProperty::ColorOrder,
    GraphicsProperty::LineStyleOrder,
    GraphicsProperty::ColorOrderIndex,
    GraphicsProperty::XLim,
    GraphicsProperty::YLim,
    GraphicsProperty::ZLim,
    GraphicsProperty::XLimMode,
    GraphicsProperty::YLimMode,
    GraphicsProperty::ZLimMode,
    GraphicsProperty::ThetaLim,
    GraphicsProperty::RLim,
    GraphicsProperty::ThetaLimMode,
    GraphicsProperty::RLimMode,
    GraphicsProperty::XScale,
    GraphicsProperty::YScale,
    GraphicsProperty::ZScale,
    GraphicsProperty::XLimitMethod,
    GraphicsProperty::YLimitMethod,
    GraphicsProperty::ZLimitMethod,
    GraphicsProperty::XDir,
    GraphicsProperty::YDir,
    GraphicsProperty::ZDir,
    GraphicsProperty::XTick,
    GraphicsProperty::YTick,
    GraphicsProperty::ZTick,
    GraphicsProperty::XTickMode,
    GraphicsProperty::YTickMode,
    GraphicsProperty::ZTickMode,
    GraphicsProperty::XTickLabel,
    GraphicsProperty::YTickLabel,
    GraphicsProperty::ZTickLabel,
    GraphicsProperty::XTickLabelMode,
    GraphicsProperty::YTickLabelMode,
    GraphicsProperty::ZTickLabelMode,
    GraphicsProperty::ThetaTick,
    GraphicsProperty::RTick,
    GraphicsProperty::ThetaTickMode,
    GraphicsProperty::RTickMode,
    GraphicsProperty::ThetaTickLabel,
    GraphicsProperty::RTickLabel,
    GraphicsProperty::ThetaTickLabelMode,
    GraphicsProperty::RTickLabelMode,
    GraphicsProperty::ThetaAxisUnits,
    GraphicsProperty::ThetaDir,
    GraphicsProperty::ThetaZeroLocation,
    GraphicsProperty::RAxisLocation,
    GraphicsProperty::ThetaGrid,
    GraphicsProperty::RGrid,
    GraphicsProperty::ThetaMinorGrid,
    GraphicsProperty::RMinorGrid,
    GraphicsProperty::Ticks,
    GraphicsProperty::TicksMode,
    GraphicsProperty::TickLabels,
    GraphicsProperty::TickLabelsMode,
    GraphicsProperty::TickLabelInterpreter,
    GraphicsProperty::Box,
    GraphicsProperty::XGrid,
    GraphicsProperty::YGrid,
    GraphicsProperty::ZGrid,
    GraphicsProperty::XMinorGrid,
    GraphicsProperty::YMinorGrid,
    GraphicsProperty::ZMinorGrid,
    GraphicsProperty::TickDir,
    GraphicsProperty::XData,
    GraphicsProperty::YData,
    GraphicsProperty::ZData,
    GraphicsProperty::ThetaData,
    GraphicsProperty::RData,
    GraphicsProperty::View,
    GraphicsProperty::Projection,
    GraphicsProperty::DataAspectRatio,
    GraphicsProperty::DataAspectRatioMode,
    GraphicsProperty::PlotBoxAspectRatio,
    GraphicsProperty::PlotBoxAspectRatioMode,
    GraphicsProperty::CLim,
    GraphicsProperty::CLimMode,
    GraphicsProperty::FaceColor,
    GraphicsProperty::EdgeColor,
    GraphicsProperty::CDataMapping,
    GraphicsProperty::FaceAlpha,
    GraphicsProperty::EdgeAlpha,
    GraphicsProperty::Faces,
    GraphicsProperty::Vertices,
    GraphicsProperty::FaceVertexCData,
    GraphicsProperty::String,
    GraphicsProperty::Location,
    GraphicsProperty::FontSize,
    GraphicsProperty::FontName,
    GraphicsProperty::FontWeight,
    GraphicsProperty::Rotation,
    GraphicsProperty::Orientation,
    GraphicsProperty::NumColumns,
    GraphicsProperty::Interpreter,
];

#[allow(clippy::too_many_lines)]
fn ensure_property_supported(
    name: &str,
    class: GraphicsClass,
    property: GraphicsProperty,
) -> Result<(), BuiltinError> {
    let supported = match class {
        GraphicsClass::Figure => matches!(
            property,
            GraphicsProperty::Parent
                | GraphicsProperty::Children
                | GraphicsProperty::Name
                | GraphicsProperty::NumberTitle
                | GraphicsProperty::Position
                | GraphicsProperty::Color
                | GraphicsProperty::Visible
        ),
        GraphicsClass::Axes2D => matches!(
            property,
            GraphicsProperty::Parent
                | GraphicsProperty::Children
                | GraphicsProperty::Position
                | GraphicsProperty::Color
                | GraphicsProperty::Visible
                | GraphicsProperty::ColorOrder
                | GraphicsProperty::LineStyleOrder
                | GraphicsProperty::ColorOrderIndex
                | GraphicsProperty::XLim
                | GraphicsProperty::YLim
                | GraphicsProperty::ZLim
                | GraphicsProperty::XLimMode
                | GraphicsProperty::YLimMode
                | GraphicsProperty::ZLimMode
                | GraphicsProperty::XScale
                | GraphicsProperty::YScale
                | GraphicsProperty::ZScale
                | GraphicsProperty::XLimitMethod
                | GraphicsProperty::YLimitMethod
                | GraphicsProperty::ZLimitMethod
                | GraphicsProperty::XDir
                | GraphicsProperty::YDir
                | GraphicsProperty::ZDir
                | GraphicsProperty::XGrid
                | GraphicsProperty::YGrid
                | GraphicsProperty::ZGrid
                | GraphicsProperty::XMinorGrid
                | GraphicsProperty::YMinorGrid
                | GraphicsProperty::ZMinorGrid
                | GraphicsProperty::TickDir
                | GraphicsProperty::XTick
                | GraphicsProperty::YTick
                | GraphicsProperty::ZTick
                | GraphicsProperty::XTickMode
                | GraphicsProperty::YTickMode
                | GraphicsProperty::ZTickMode
                | GraphicsProperty::XTickLabel
                | GraphicsProperty::YTickLabel
                | GraphicsProperty::ZTickLabel
                | GraphicsProperty::XTickLabelMode
                | GraphicsProperty::YTickLabelMode
                | GraphicsProperty::ZTickLabelMode
                | GraphicsProperty::TickLabelInterpreter
                | GraphicsProperty::FontSize
                | GraphicsProperty::FontName
                | GraphicsProperty::LineWidth
                | GraphicsProperty::Box
                | GraphicsProperty::View
                | GraphicsProperty::Projection
                | GraphicsProperty::DataAspectRatio
                | GraphicsProperty::DataAspectRatioMode
                | GraphicsProperty::PlotBoxAspectRatio
                | GraphicsProperty::PlotBoxAspectRatioMode
                | GraphicsProperty::CLim
                | GraphicsProperty::CLimMode
        ),
        GraphicsClass::PolarAxes => matches!(
            property,
            GraphicsProperty::Parent
                | GraphicsProperty::Children
                | GraphicsProperty::Position
                | GraphicsProperty::Color
                | GraphicsProperty::Visible
                | GraphicsProperty::ColorOrder
                | GraphicsProperty::LineStyleOrder
                | GraphicsProperty::ColorOrderIndex
                | GraphicsProperty::ThetaLim
                | GraphicsProperty::RLim
                | GraphicsProperty::ThetaLimMode
                | GraphicsProperty::RLimMode
                | GraphicsProperty::ThetaTick
                | GraphicsProperty::RTick
                | GraphicsProperty::ThetaTickMode
                | GraphicsProperty::RTickMode
                | GraphicsProperty::ThetaTickLabel
                | GraphicsProperty::RTickLabel
                | GraphicsProperty::ThetaTickLabelMode
                | GraphicsProperty::RTickLabelMode
                | GraphicsProperty::ThetaAxisUnits
                | GraphicsProperty::ThetaDir
                | GraphicsProperty::ThetaZeroLocation
                | GraphicsProperty::RAxisLocation
                | GraphicsProperty::ThetaGrid
                | GraphicsProperty::RGrid
                | GraphicsProperty::ThetaMinorGrid
                | GraphicsProperty::RMinorGrid
                | GraphicsProperty::TickLabelInterpreter
                | GraphicsProperty::FontSize
                | GraphicsProperty::FontName
                | GraphicsProperty::LineWidth
                | GraphicsProperty::Box
                | GraphicsProperty::CLim
                | GraphicsProperty::CLimMode
        ),
        GraphicsClass::TiledChartLayout => matches!(
            property,
            GraphicsProperty::Parent
                | GraphicsProperty::Children
                | GraphicsProperty::GridSize
                | GraphicsProperty::TileSpacing
                | GraphicsProperty::Padding
        ),
        GraphicsClass::LineSeries => matches!(
            property,
            GraphicsProperty::Parent
                | GraphicsProperty::Children
                | GraphicsProperty::LineWidth
                | GraphicsProperty::Color
                | GraphicsProperty::LineStyle
                | GraphicsProperty::Marker
                | GraphicsProperty::MarkerSize
                | GraphicsProperty::MarkerIndices
                | GraphicsProperty::MarkerFaceColor
                | GraphicsProperty::MarkerEdgeColor
                | GraphicsProperty::DisplayName
                | GraphicsProperty::Clipping
                | GraphicsProperty::Visible
                | GraphicsProperty::XData
                | GraphicsProperty::YData
                | GraphicsProperty::ZData
                | GraphicsProperty::ThetaData
                | GraphicsProperty::RData
        ),
        GraphicsClass::ScatterSeries => matches!(
            property,
            GraphicsProperty::Parent
                | GraphicsProperty::Children
                | GraphicsProperty::LineWidth
                | GraphicsProperty::Marker
                | GraphicsProperty::Visible
                | GraphicsProperty::SizeData
                | GraphicsProperty::CData
                | GraphicsProperty::MarkerFaceColor
                | GraphicsProperty::MarkerEdgeColor
                | GraphicsProperty::DisplayName
                | GraphicsProperty::Clipping
                | GraphicsProperty::XData
                | GraphicsProperty::YData
                | GraphicsProperty::ZData
        ),
        GraphicsClass::SurfaceSeries => matches!(
            property,
            GraphicsProperty::Parent
                | GraphicsProperty::Children
                | GraphicsProperty::LineWidth
                | GraphicsProperty::LineStyle
                | GraphicsProperty::Visible
                | GraphicsProperty::XData
                | GraphicsProperty::YData
                | GraphicsProperty::ZData
                | GraphicsProperty::CData
                | GraphicsProperty::FaceColor
                | GraphicsProperty::EdgeColor
                | GraphicsProperty::CDataMapping
                | GraphicsProperty::FaceAlpha
        ),
        GraphicsClass::PatchSeries => matches!(
            property,
            GraphicsProperty::Parent
                | GraphicsProperty::Children
                | GraphicsProperty::LineWidth
                | GraphicsProperty::LineStyle
                | GraphicsProperty::Visible
                | GraphicsProperty::XData
                | GraphicsProperty::YData
                | GraphicsProperty::ZData
                | GraphicsProperty::CData
                | GraphicsProperty::Faces
                | GraphicsProperty::Vertices
                | GraphicsProperty::FaceVertexCData
                | GraphicsProperty::FaceColor
                | GraphicsProperty::EdgeColor
                | GraphicsProperty::CDataMapping
                | GraphicsProperty::FaceAlpha
                | GraphicsProperty::EdgeAlpha
        ),
        GraphicsClass::Stair => matches!(
            property,
            GraphicsProperty::Parent
                | GraphicsProperty::Children
                | GraphicsProperty::Color
                | GraphicsProperty::LineWidth
                | GraphicsProperty::LineStyle
                | GraphicsProperty::Marker
                | GraphicsProperty::MarkerSize
                | GraphicsProperty::MarkerFaceColor
                | GraphicsProperty::MarkerEdgeColor
                | GraphicsProperty::Visible
        ),
        GraphicsClass::Stem => matches!(
            property,
            GraphicsProperty::Parent
                | GraphicsProperty::Children
                | GraphicsProperty::Color
                | GraphicsProperty::LineWidth
                | GraphicsProperty::LineStyle
                | GraphicsProperty::Marker
                | GraphicsProperty::MarkerSize
                | GraphicsProperty::MarkerFaceColor
                | GraphicsProperty::MarkerEdgeColor
                | GraphicsProperty::BaseValue
                | GraphicsProperty::Visible
        ),
        GraphicsClass::ErrorBar => matches!(
            property,
            GraphicsProperty::Parent
                | GraphicsProperty::Children
                | GraphicsProperty::Color
                | GraphicsProperty::LineWidth
                | GraphicsProperty::LineStyle
                | GraphicsProperty::Marker
                | GraphicsProperty::MarkerSize
                | GraphicsProperty::MarkerFaceColor
                | GraphicsProperty::MarkerEdgeColor
                | GraphicsProperty::CapSize
                | GraphicsProperty::Visible
        ),
        GraphicsClass::Area => matches!(
            property,
            GraphicsProperty::Parent
                | GraphicsProperty::Children
                | GraphicsProperty::FaceColor
                | GraphicsProperty::EdgeColor
                | GraphicsProperty::LineWidth
                | GraphicsProperty::LineStyle
                | GraphicsProperty::FaceAlpha
                | GraphicsProperty::EdgeAlpha
                | GraphicsProperty::BaseValue
                | GraphicsProperty::Visible
        ),
        GraphicsClass::Bar => matches!(
            property,
            GraphicsProperty::Parent
                | GraphicsProperty::Children
                | GraphicsProperty::FaceColor
                | GraphicsProperty::EdgeColor
                | GraphicsProperty::LineWidth
                | GraphicsProperty::LineStyle
                | GraphicsProperty::FaceAlpha
                | GraphicsProperty::EdgeAlpha
                | GraphicsProperty::BaseValue
                | GraphicsProperty::BarWidth
                | GraphicsProperty::Visible
        ),
        GraphicsClass::Histogram => matches!(
            property,
            GraphicsProperty::Parent
                | GraphicsProperty::Children
                | GraphicsProperty::FaceColor
                | GraphicsProperty::EdgeColor
                | GraphicsProperty::LineWidth
                | GraphicsProperty::LineStyle
                | GraphicsProperty::FaceAlpha
                | GraphicsProperty::EdgeAlpha
                | GraphicsProperty::Visible
        ),
        GraphicsClass::Contour | GraphicsClass::Image => matches!(
            property,
            GraphicsProperty::Parent | GraphicsProperty::Children | GraphicsProperty::Visible
        ),
        GraphicsClass::Text => matches!(
            property,
            GraphicsProperty::Parent
                | GraphicsProperty::Children
                | GraphicsProperty::String
                | GraphicsProperty::Color
                | GraphicsProperty::FontSize
                | GraphicsProperty::FontName
                | GraphicsProperty::FontWeight
                | GraphicsProperty::Rotation
                | GraphicsProperty::Interpreter
                | GraphicsProperty::Visible
        ),
        GraphicsClass::Legend => matches!(
            property,
            GraphicsProperty::Parent
                | GraphicsProperty::Children
                | GraphicsProperty::String
                | GraphicsProperty::Location
                | GraphicsProperty::FontSize
                | GraphicsProperty::FontName
                | GraphicsProperty::FontWeight
                | GraphicsProperty::Orientation
                | GraphicsProperty::NumColumns
                | GraphicsProperty::Box
                | GraphicsProperty::Interpreter
                | GraphicsProperty::Visible
        ),
        GraphicsClass::ColorBar => matches!(
            property,
            GraphicsProperty::Parent
                | GraphicsProperty::Children
                | GraphicsProperty::Visible
                | GraphicsProperty::Ticks
                | GraphicsProperty::TicksMode
                | GraphicsProperty::TickLabels
                | GraphicsProperty::TickLabelsMode
        ),
    };
    if supported {
        Ok(())
    } else {
        Err(BuiltinError::new(
            BuiltinErrorCategory::Graphics,
            format!(
                "`{name}` property {} is not defined for {}",
                property_display_name(property),
                class.class_name()
            ),
        ))
    }
}

#[allow(clippy::too_many_lines)]
fn property_display_name(property: GraphicsProperty) -> &'static str {
    match property {
        GraphicsProperty::Parent => "Parent",
        GraphicsProperty::Children => "Children",
        GraphicsProperty::Position => "Position",
        GraphicsProperty::Name => "Name",
        GraphicsProperty::NumberTitle => "NumberTitle",
        GraphicsProperty::GridSize => "GridSize",
        GraphicsProperty::TileSpacing => "TileSpacing",
        GraphicsProperty::Padding => "Padding",
        GraphicsProperty::LineWidth => "LineWidth",
        GraphicsProperty::Color => "Color",
        GraphicsProperty::LineStyle => "LineStyle",
        GraphicsProperty::Marker => "Marker",
        GraphicsProperty::MarkerSize => "MarkerSize",
        GraphicsProperty::MarkerIndices => "MarkerIndices",
        GraphicsProperty::DisplayName => "DisplayName",
        GraphicsProperty::Clipping => "Clipping",
        GraphicsProperty::Visible => "Visible",
        GraphicsProperty::SizeData => "SizeData",
        GraphicsProperty::CData => "CData",
        GraphicsProperty::MarkerFaceColor => "MarkerFaceColor",
        GraphicsProperty::MarkerEdgeColor => "MarkerEdgeColor",
        GraphicsProperty::BaseValue => "BaseValue",
        GraphicsProperty::BarWidth => "BarWidth",
        GraphicsProperty::CapSize => "CapSize",
        GraphicsProperty::ColorOrder => "ColorOrder",
        GraphicsProperty::LineStyleOrder => "LineStyleOrder",
        GraphicsProperty::ColorOrderIndex => "ColorOrderIndex",
        GraphicsProperty::XLim => "XLim",
        GraphicsProperty::YLim => "YLim",
        GraphicsProperty::ZLim => "ZLim",
        GraphicsProperty::XLimMode => "XLimMode",
        GraphicsProperty::YLimMode => "YLimMode",
        GraphicsProperty::ZLimMode => "ZLimMode",
        GraphicsProperty::ThetaLim => "ThetaLim",
        GraphicsProperty::RLim => "RLim",
        GraphicsProperty::ThetaLimMode => "ThetaLimMode",
        GraphicsProperty::RLimMode => "RLimMode",
        GraphicsProperty::XScale => "XScale",
        GraphicsProperty::YScale => "YScale",
        GraphicsProperty::ZScale => "ZScale",
        GraphicsProperty::XLimitMethod => "XLimitMethod",
        GraphicsProperty::YLimitMethod => "YLimitMethod",
        GraphicsProperty::ZLimitMethod => "ZLimitMethod",
        GraphicsProperty::XDir => "XDir",
        GraphicsProperty::YDir => "YDir",
        GraphicsProperty::ZDir => "ZDir",
        GraphicsProperty::XTick => "XTick",
        GraphicsProperty::YTick => "YTick",
        GraphicsProperty::ZTick => "ZTick",
        GraphicsProperty::XTickMode => "XTickMode",
        GraphicsProperty::YTickMode => "YTickMode",
        GraphicsProperty::ZTickMode => "ZTickMode",
        GraphicsProperty::XTickLabel => "XTickLabel",
        GraphicsProperty::YTickLabel => "YTickLabel",
        GraphicsProperty::ZTickLabel => "ZTickLabel",
        GraphicsProperty::XTickLabelMode => "XTickLabelMode",
        GraphicsProperty::YTickLabelMode => "YTickLabelMode",
        GraphicsProperty::ZTickLabelMode => "ZTickLabelMode",
        GraphicsProperty::ThetaTick => "ThetaTick",
        GraphicsProperty::RTick => "RTick",
        GraphicsProperty::ThetaTickMode => "ThetaTickMode",
        GraphicsProperty::RTickMode => "RTickMode",
        GraphicsProperty::ThetaTickLabel => "ThetaTickLabel",
        GraphicsProperty::RTickLabel => "RTickLabel",
        GraphicsProperty::ThetaTickLabelMode => "ThetaTickLabelMode",
        GraphicsProperty::RTickLabelMode => "RTickLabelMode",
        GraphicsProperty::ThetaAxisUnits => "ThetaAxisUnits",
        GraphicsProperty::ThetaDir => "ThetaDir",
        GraphicsProperty::ThetaZeroLocation => "ThetaZeroLocation",
        GraphicsProperty::RAxisLocation => "RAxisLocation",
        GraphicsProperty::ThetaGrid => "ThetaGrid",
        GraphicsProperty::RGrid => "RGrid",
        GraphicsProperty::ThetaMinorGrid => "ThetaMinorGrid",
        GraphicsProperty::RMinorGrid => "RMinorGrid",
        GraphicsProperty::Ticks => "Ticks",
        GraphicsProperty::TicksMode => "TicksMode",
        GraphicsProperty::TickLabels => "TickLabels",
        GraphicsProperty::TickLabelsMode => "TickLabelsMode",
        GraphicsProperty::TickLabelInterpreter => "TickLabelInterpreter",
        GraphicsProperty::Box => "Box",
        GraphicsProperty::XGrid => "XGrid",
        GraphicsProperty::YGrid => "YGrid",
        GraphicsProperty::ZGrid => "ZGrid",
        GraphicsProperty::XMinorGrid => "XMinorGrid",
        GraphicsProperty::YMinorGrid => "YMinorGrid",
        GraphicsProperty::ZMinorGrid => "ZMinorGrid",
        GraphicsProperty::TickDir => "TickDir",
        GraphicsProperty::XData => "XData",
        GraphicsProperty::YData => "YData",
        GraphicsProperty::ZData => "ZData",
        GraphicsProperty::ThetaData => "ThetaData",
        GraphicsProperty::RData => "RData",
        GraphicsProperty::View => "View",
        GraphicsProperty::Projection => "Projection",
        GraphicsProperty::DataAspectRatio => "DataAspectRatio",
        GraphicsProperty::DataAspectRatioMode => "DataAspectRatioMode",
        GraphicsProperty::PlotBoxAspectRatio => "PlotBoxAspectRatio",
        GraphicsProperty::PlotBoxAspectRatioMode => "PlotBoxAspectRatioMode",
        GraphicsProperty::CLim => "CLim",
        GraphicsProperty::CLimMode => "CLimMode",
        GraphicsProperty::FaceColor => "FaceColor",
        GraphicsProperty::EdgeColor => "EdgeColor",
        GraphicsProperty::CDataMapping => "CDataMapping",
        GraphicsProperty::FaceAlpha => "FaceAlpha",
        GraphicsProperty::EdgeAlpha => "EdgeAlpha",
        GraphicsProperty::Faces => "Faces",
        GraphicsProperty::Vertices => "Vertices",
        GraphicsProperty::FaceVertexCData => "FaceVertexCData",
        GraphicsProperty::String => "String",
        GraphicsProperty::Location => "Location",
        GraphicsProperty::FontSize => "FontSize",
        GraphicsProperty::FontName => "FontName",
        GraphicsProperty::FontWeight => "FontWeight",
        GraphicsProperty::Rotation => "Rotation",
        GraphicsProperty::Orientation => "Orientation",
        GraphicsProperty::NumColumns => "NumColumns",
        GraphicsProperty::Interpreter => "Interpreter",
    }
}

#[allow(clippy::too_many_lines)]
fn parse_property_update(
    name: &str,
    name_position: usize,
    value_position: usize,
    class: GraphicsClass,
    property_value: &Value,
    value: &Value,
) -> Result<GraphicsPropertyUpdate, BuiltinError> {
    let property_name = lowercase_ascii_text(name, name_position, property_value)?;
    let property = graphics_property(name, class, &property_name)?;
    match property {
        GraphicsProperty::Position if class == GraphicsClass::Figure => {
            figure_position(name, value_position, value).map(GraphicsPropertyUpdate::Position)
        }
        GraphicsProperty::Position => {
            axes_position(name, value_position, value).map(GraphicsPropertyUpdate::Position)
        }
        GraphicsProperty::Name => Ok(GraphicsPropertyUpdate::Name(text_scalar(
            name,
            value_position,
            value,
        )?)),
        GraphicsProperty::NumberTitle => {
            visible_value(name, value_position, value).map(GraphicsPropertyUpdate::NumberTitle)
        }
        GraphicsProperty::LineWidth => Ok(GraphicsPropertyUpdate::LineWidth(positive_f32_scalar(
            name,
            value_position,
            "LineWidth",
            value,
        )?)),
        GraphicsProperty::Color
            if matches!(class, GraphicsClass::Axes2D | GraphicsClass::PolarAxes) =>
        {
            Ok(GraphicsPropertyUpdate::AxesColor(axes_background_color(
                name,
                value_position,
                value,
            )?))
        }
        GraphicsProperty::Color => Ok(GraphicsPropertyUpdate::Color(rgb_color(
            name,
            value_position,
            "Color",
            value,
        )?)),
        GraphicsProperty::LineStyle => Ok(GraphicsPropertyUpdate::LineStyle(line_style_value(
            name,
            value_position,
            value,
        )?)),
        GraphicsProperty::Marker => Ok(GraphicsPropertyUpdate::Marker(marker_value(
            name,
            value_position,
            value,
        )?)),
        GraphicsProperty::MarkerSize => Ok(GraphicsPropertyUpdate::MarkerSize(
            positive_f32_scalar(name, value_position, "MarkerSize", value)?,
        )),
        GraphicsProperty::MarkerIndices => {
            let (values, value_class) = marker_indices(name, value_position, value)?;
            Ok(GraphicsPropertyUpdate::MarkerIndices {
                values,
                value_class,
            })
        }
        GraphicsProperty::DisplayName => Ok(GraphicsPropertyUpdate::DisplayName(text_scalar(
            name,
            value_position,
            value,
        )?)),
        GraphicsProperty::Clipping => Ok(GraphicsPropertyUpdate::Clipping(visible_value(
            name,
            value_position,
            value,
        )?)),
        GraphicsProperty::Visible => Ok(GraphicsPropertyUpdate::Visible(visible_value(
            name,
            value_position,
            value,
        )?)),
        GraphicsProperty::SizeData => Ok(GraphicsPropertyUpdate::SizeData(nonnegative_f32_scalar(
            name,
            value_position,
            "SizeData",
            value,
        )?)),
        GraphicsProperty::CData if class != GraphicsClass::SurfaceSeries => Ok(
            GraphicsPropertyUpdate::CData(rgb_color(name, value_position, "CData", value)?),
        ),
        GraphicsProperty::MarkerFaceColor if is_line_chart_class(class) => {
            chart_color_value(name, value_position, "MarkerFaceColor", value, true)
                .map(GraphicsPropertyUpdate::ChartMarkerFaceColor)
        }
        GraphicsProperty::MarkerFaceColor => Ok(GraphicsPropertyUpdate::MarkerFaceColor(
            rgb_color(name, value_position, "MarkerFaceColor", value)?,
        )),
        GraphicsProperty::MarkerEdgeColor if is_line_chart_class(class) => {
            chart_color_value(name, value_position, "MarkerEdgeColor", value, true)
                .map(GraphicsPropertyUpdate::ChartMarkerEdgeColor)
        }
        GraphicsProperty::MarkerEdgeColor => Ok(GraphicsPropertyUpdate::MarkerEdgeColor(
            rgb_color(name, value_position, "MarkerEdgeColor", value)?,
        )),
        GraphicsProperty::BaseValue => {
            let scalar = real_scalar(name, value_position, value)?;
            if !scalar.is_finite() {
                return Err(type_error(
                    name,
                    value_position,
                    "finite BaseValue scalar",
                    value,
                ));
            }
            Ok(GraphicsPropertyUpdate::BaseValue(scalar))
        }
        GraphicsProperty::BarWidth => {
            let scalar = real_scalar(name, value_position, value)?;
            if !scalar.is_finite() || scalar <= 0.0 || scalar > 1.0 {
                return Err(type_error(
                    name,
                    value_position,
                    "BarWidth scalar in (0,1]",
                    value,
                ));
            }
            Ok(GraphicsPropertyUpdate::BarWidth(scalar))
        }
        GraphicsProperty::CapSize => {
            let scalar = real_scalar(name, value_position, value)?;
            if !scalar.is_finite() || scalar < 0.0 {
                return Err(type_error(
                    name,
                    value_position,
                    "nonnegative finite CapSize scalar",
                    value,
                ));
            }
            Ok(GraphicsPropertyUpdate::CapSize(scalar))
        }
        GraphicsProperty::ColorOrder => {
            color_order(name, value_position, value).map(GraphicsPropertyUpdate::ColorOrder)
        }
        GraphicsProperty::LineStyleOrder => line_style_order(name, value_position, value)
            .map(GraphicsPropertyUpdate::LineStyleOrder),
        GraphicsProperty::ColorOrderIndex => {
            positive_u32(name, value_position, value).map(GraphicsPropertyUpdate::ColorOrderIndex)
        }
        GraphicsProperty::XLim => {
            limit_pair(name, value_position, "XLim", value).map(GraphicsPropertyUpdate::XLim)
        }
        GraphicsProperty::YLim => {
            limit_pair(name, value_position, "YLim", value).map(GraphicsPropertyUpdate::YLim)
        }
        GraphicsProperty::ZLim => {
            limit_pair(name, value_position, "ZLim", value).map(GraphicsPropertyUpdate::ZLim)
        }
        GraphicsProperty::XLimMode | GraphicsProperty::ThetaLimMode => {
            limit_mode(name, value_position, value).map(GraphicsPropertyUpdate::XLimMode)
        }
        GraphicsProperty::YLimMode | GraphicsProperty::RLimMode => {
            limit_mode(name, value_position, value).map(GraphicsPropertyUpdate::YLimMode)
        }
        GraphicsProperty::ZLimMode => {
            limit_mode(name, value_position, value).map(GraphicsPropertyUpdate::ZLimMode)
        }
        GraphicsProperty::ThetaLim => {
            limit_pair(name, value_position, "ThetaLim", value).map(GraphicsPropertyUpdate::XLim)
        }
        GraphicsProperty::RLim => {
            limit_pair(name, value_position, "RLim", value).map(GraphicsPropertyUpdate::YLim)
        }
        GraphicsProperty::XScale => {
            axis_scale(name, value_position, value).map(GraphicsPropertyUpdate::XScale)
        }
        GraphicsProperty::YScale => {
            axis_scale(name, value_position, value).map(GraphicsPropertyUpdate::YScale)
        }
        GraphicsProperty::ZScale => {
            axis_scale(name, value_position, value).map(GraphicsPropertyUpdate::ZScale)
        }
        GraphicsProperty::XLimitMethod => {
            limit_method(name, value_position, value).map(GraphicsPropertyUpdate::XLimitMethod)
        }
        GraphicsProperty::YLimitMethod => {
            limit_method(name, value_position, value).map(GraphicsPropertyUpdate::YLimitMethod)
        }
        GraphicsProperty::ZLimitMethod => {
            limit_method(name, value_position, value).map(GraphicsPropertyUpdate::ZLimitMethod)
        }
        GraphicsProperty::XDir => {
            axis_direction(name, value_position, value).map(GraphicsPropertyUpdate::XDir)
        }
        GraphicsProperty::YDir => {
            axis_direction(name, value_position, value).map(GraphicsPropertyUpdate::YDir)
        }
        GraphicsProperty::ZDir => {
            axis_direction(name, value_position, value).map(GraphicsPropertyUpdate::ZDir)
        }
        GraphicsProperty::XTick | GraphicsProperty::ThetaTick => {
            tick_values(name, value_position, value).map(GraphicsPropertyUpdate::XTick)
        }
        GraphicsProperty::YTick | GraphicsProperty::RTick => {
            tick_values(name, value_position, value).map(GraphicsPropertyUpdate::YTick)
        }
        GraphicsProperty::ZTick => {
            tick_values(name, value_position, value).map(GraphicsPropertyUpdate::ZTick)
        }
        GraphicsProperty::XTickMode | GraphicsProperty::ThetaTickMode => {
            tick_mode(name, value_position, value).map(GraphicsPropertyUpdate::XTickMode)
        }
        GraphicsProperty::YTickMode | GraphicsProperty::RTickMode => {
            tick_mode(name, value_position, value).map(GraphicsPropertyUpdate::YTickMode)
        }
        GraphicsProperty::ZTickMode => {
            tick_mode(name, value_position, value).map(GraphicsPropertyUpdate::ZTickMode)
        }
        GraphicsProperty::XTickLabel | GraphicsProperty::ThetaTickLabel => {
            tick_labels(name, value_position, value).map(GraphicsPropertyUpdate::XTickLabel)
        }
        GraphicsProperty::YTickLabel | GraphicsProperty::RTickLabel => {
            tick_labels(name, value_position, value).map(GraphicsPropertyUpdate::YTickLabel)
        }
        GraphicsProperty::ZTickLabel => {
            tick_labels(name, value_position, value).map(GraphicsPropertyUpdate::ZTickLabel)
        }
        GraphicsProperty::XTickLabelMode | GraphicsProperty::ThetaTickLabelMode => {
            tick_mode(name, value_position, value).map(GraphicsPropertyUpdate::XTickLabelMode)
        }
        GraphicsProperty::YTickLabelMode | GraphicsProperty::RTickLabelMode => {
            tick_mode(name, value_position, value).map(GraphicsPropertyUpdate::YTickLabelMode)
        }
        GraphicsProperty::ZTickLabelMode => {
            tick_mode(name, value_position, value).map(GraphicsPropertyUpdate::ZTickLabelMode)
        }
        GraphicsProperty::ThetaAxisUnits => theta_axis_units(name, value_position, value)
            .map(GraphicsPropertyUpdate::ThetaAxisUnits),
        GraphicsProperty::ThetaDir => {
            theta_direction(name, value_position, value).map(GraphicsPropertyUpdate::ThetaDir)
        }
        GraphicsProperty::ThetaZeroLocation => theta_zero_location(name, value_position, value)
            .map(GraphicsPropertyUpdate::ThetaZeroLocation),
        GraphicsProperty::RAxisLocation => {
            let scalar = real_scalar(name, value_position, value)?;
            if !scalar.is_finite() {
                return Err(type_error(
                    name,
                    value_position,
                    "finite RAxisLocation scalar",
                    value,
                ));
            }
            Ok(GraphicsPropertyUpdate::RAxisLocation(scalar))
        }
        GraphicsProperty::Ticks => {
            tick_values(name, value_position, value).map(GraphicsPropertyUpdate::Ticks)
        }
        GraphicsProperty::TicksMode => {
            tick_mode(name, value_position, value).map(GraphicsPropertyUpdate::TicksMode)
        }
        GraphicsProperty::TickLabels => {
            tick_labels(name, value_position, value).map(GraphicsPropertyUpdate::TickLabels)
        }
        GraphicsProperty::TickLabelsMode => {
            tick_mode(name, value_position, value).map(GraphicsPropertyUpdate::TickLabelsMode)
        }
        GraphicsProperty::TickLabelInterpreter => interpreter(name, value_position, value)
            .map(GraphicsPropertyUpdate::TickLabelInterpreter),
        GraphicsProperty::Box => {
            visible_value(name, value_position, value).map(GraphicsPropertyUpdate::Box)
        }
        GraphicsProperty::XGrid | GraphicsProperty::ThetaGrid => {
            visible_value(name, value_position, value).map(GraphicsPropertyUpdate::XGrid)
        }
        GraphicsProperty::YGrid | GraphicsProperty::RGrid => {
            visible_value(name, value_position, value).map(GraphicsPropertyUpdate::YGrid)
        }
        GraphicsProperty::ZGrid => {
            visible_value(name, value_position, value).map(GraphicsPropertyUpdate::ZGrid)
        }
        GraphicsProperty::XMinorGrid | GraphicsProperty::ThetaMinorGrid => {
            visible_value(name, value_position, value).map(GraphicsPropertyUpdate::XMinorGrid)
        }
        GraphicsProperty::YMinorGrid | GraphicsProperty::RMinorGrid => {
            visible_value(name, value_position, value).map(GraphicsPropertyUpdate::YMinorGrid)
        }
        GraphicsProperty::ZMinorGrid => {
            visible_value(name, value_position, value).map(GraphicsPropertyUpdate::ZMinorGrid)
        }
        GraphicsProperty::TickDir => {
            tick_direction(name, value_position, value).map(GraphicsPropertyUpdate::TickDir)
        }
        GraphicsProperty::FaceColor if is_patch_chart_class(class) => chart_color_value(
            name,
            value_position,
            "FaceColor",
            value,
            class == GraphicsClass::Histogram,
        )
        .map(GraphicsPropertyUpdate::ChartFaceColor),
        GraphicsProperty::FaceColor => {
            surface_color_value(name, value_position, value).map(GraphicsPropertyUpdate::FaceColor)
        }
        GraphicsProperty::EdgeColor if is_patch_chart_class(class) => chart_color_value(
            name,
            value_position,
            "EdgeColor",
            value,
            class == GraphicsClass::Histogram,
        )
        .map(GraphicsPropertyUpdate::ChartEdgeColor),
        GraphicsProperty::EdgeColor => {
            surface_color_value(name, value_position, value).map(GraphicsPropertyUpdate::EdgeColor)
        }
        GraphicsProperty::CDataMapping => {
            match lowercase_ascii_text(name, value_position, value)?.as_str() {
                "scaled" => Ok(GraphicsPropertyUpdate::CDataMapping(CDataMapping::Scaled)),
                "direct" => Ok(GraphicsPropertyUpdate::CDataMapping(CDataMapping::Direct)),
                _ => Err(type_error(name, value_position, "scaled or direct", value)),
            }
        }
        GraphicsProperty::XData => PreparedNumeric::from_value(name, value_position, value)
            .and_then(|data| {
                data.is_vector()
                    .then(|| data.vector())
                    .ok_or_else(|| type_error(name, value_position, "real vector XData", value))
            })
            .map(GraphicsPropertyUpdate::XData),
        GraphicsProperty::YData => PreparedNumeric::from_value(name, value_position, value)
            .and_then(|data| {
                data.is_vector()
                    .then(|| data.vector())
                    .ok_or_else(|| type_error(name, value_position, "real vector YData", value))
            })
            .map(GraphicsPropertyUpdate::YData),
        GraphicsProperty::ThetaData => PreparedNumeric::from_value(name, value_position, value)
            .and_then(|data| {
                data.is_vector()
                    .then(|| data.vector())
                    .ok_or_else(|| type_error(name, value_position, "real vector ThetaData", value))
            })
            .map(GraphicsPropertyUpdate::XData),
        GraphicsProperty::RData => PreparedNumeric::from_value(name, value_position, value)
            .and_then(|data| {
                data.is_vector()
                    .then(|| data.vector())
                    .ok_or_else(|| type_error(name, value_position, "real vector RData", value))
            })
            .map(GraphicsPropertyUpdate::YData),
        GraphicsProperty::ZData if class != GraphicsClass::SurfaceSeries => {
            PreparedNumeric::from_value(name, value_position, value)
                .and_then(|data| {
                    data.is_vector()
                        .then(|| data.vector())
                        .ok_or_else(|| type_error(name, value_position, "real vector ZData", value))
                })
                .map(GraphicsPropertyUpdate::ZData)
        }
        GraphicsProperty::Projection => {
            projection_mode(name, value_position, value).map(GraphicsPropertyUpdate::Projection)
        }
        GraphicsProperty::DataAspectRatio => {
            aspect_ratio(name, value_position, value).map(GraphicsPropertyUpdate::DataAspectRatio)
        }
        GraphicsProperty::DataAspectRatioMode => {
            limit_mode(name, value_position, value).map(GraphicsPropertyUpdate::DataAspectRatioMode)
        }
        GraphicsProperty::PlotBoxAspectRatio => aspect_ratio(name, value_position, value)
            .map(GraphicsPropertyUpdate::PlotBoxAspectRatio),
        GraphicsProperty::PlotBoxAspectRatioMode => limit_mode(name, value_position, value)
            .map(GraphicsPropertyUpdate::PlotBoxAspectRatioMode),
        GraphicsProperty::CLim => {
            limit_pair(name, value_position, "CLim", value).map(GraphicsPropertyUpdate::CLim)
        }
        GraphicsProperty::CLimMode => {
            limit_mode(name, value_position, value).map(GraphicsPropertyUpdate::CLimMode)
        }
        GraphicsProperty::FontSize => Ok(GraphicsPropertyUpdate::FontSize(positive_f32_scalar(
            name,
            value_position,
            "FontSize",
            value,
        )?)),
        GraphicsProperty::FontName => Ok(GraphicsPropertyUpdate::FontName(text_scalar(
            name,
            value_position,
            value,
        )?)),
        GraphicsProperty::FontWeight => {
            font_weight(name, value_position, value).map(GraphicsPropertyUpdate::FontWeight)
        }
        GraphicsProperty::Rotation => finite_f32_scalar(name, value_position, "Rotation", value)
            .map(GraphicsPropertyUpdate::Rotation),
        GraphicsProperty::Location => {
            legend_location(name, value_position, value).map(GraphicsPropertyUpdate::Location)
        }
        GraphicsProperty::Orientation => {
            legend_orientation(name, value_position, value).map(GraphicsPropertyUpdate::Orientation)
        }
        GraphicsProperty::NumColumns => {
            positive_u32(name, value_position, value).map(GraphicsPropertyUpdate::NumColumns)
        }
        GraphicsProperty::Interpreter => {
            interpreter(name, value_position, value).map(GraphicsPropertyUpdate::Interpreter)
        }
        GraphicsProperty::FaceAlpha => Ok(GraphicsPropertyUpdate::FaceAlpha(unit_f32_scalar(
            name,
            value_position,
            "FaceAlpha",
            value,
        )?)),
        GraphicsProperty::EdgeAlpha => Ok(GraphicsPropertyUpdate::EdgeAlpha(unit_f32_scalar(
            name,
            value_position,
            "EdgeAlpha",
            value,
        )?)),
        GraphicsProperty::Parent
        | GraphicsProperty::Children
        | GraphicsProperty::GridSize
        | GraphicsProperty::TileSpacing
        | GraphicsProperty::Padding
        | GraphicsProperty::String
        | GraphicsProperty::Faces
        | GraphicsProperty::Vertices
        | GraphicsProperty::FaceVertexCData => Err(BuiltinError::new(
            BuiltinErrorCategory::Graphics,
            format!(
                "`{name}` does not support assigning {}",
                property_display_name(property)
            ),
        )),
        GraphicsProperty::CData | GraphicsProperty::ZData | GraphicsProperty::View => {
            Err(BuiltinError::new(
                BuiltinErrorCategory::Graphics,
                format!(
                    "`{name}` does not yet support assigning {}",
                    property_display_name(property)
                ),
            ))
        }
    }
}

fn high_level_line_chart(
    name: &'static str,
    kind: ChartGroupKind,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
    geometry: impl FnOnce(&[f64], &[f64]) -> (Vec<f64>, Vec<f64>, Vec<GraphicsPropertyUpdate>),
) -> BuiltinResult {
    expect_max_outputs(name, context, 1)?;
    let (axes, arguments) = optional_axes(name, arguments)?;
    let (x, y, data_count) = chart_xy(name, arguments)?;
    let mut properties =
        parse_chart_line_properties(name, data_count, &arguments[data_count..], None)?;
    let (x, y, defaults) = geometry(&x, &y);
    properties.splice(0..0, defaults);
    create_chart(
        name,
        axes,
        kind,
        vec![line_primitive(x, y, properties, true)],
        Vec::new(),
        Vec::new(),
        context,
    )
}

fn chart_xy(name: &str, arguments: &[Value]) -> Result<(Vec<f64>, Vec<f64>, usize), BuiltinError> {
    let Some(first) = arguments.first() else {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            format!("`{name}` requires plotting data"),
        ));
    };
    if arguments.get(1).is_some_and(is_plot_numeric) {
        let x = prepared_vector(name, 1, first)?;
        let y = prepared_vector(name, 2, &arguments[1])?;
        require_equal_lengths(name, &[&x, &y])?;
        Ok((x, y, 2))
    } else {
        let y = prepared_vector(name, 1, first)?;
        let x = default_f64_x(y.len());
        Ok((x, y, 1))
    }
}

fn prepared_vector(name: &str, position: usize, value: &Value) -> Result<Vec<f64>, BuiltinError> {
    let numeric = PreparedNumeric::from_value(name, position, value)?;
    if !numeric.is_vector() {
        return Err(type_error(name, position, "real vector", value));
    }
    Ok(numeric.vector_f64())
}

fn require_equal_lengths(name: &str, vectors: &[&[f64]]) -> Result<(), BuiltinError> {
    if vectors
        .first()
        .is_none_or(|first| vectors.iter().all(|vector| vector.len() == first.len()))
    {
        Ok(())
    } else {
        Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` data vectors must have equal lengths"),
        ))
    }
}

#[allow(clippy::cast_precision_loss)]
fn default_f64_x(length: usize) -> Vec<f64> {
    (1..=length).map(|value| value as f64).collect()
}

fn parse_chart_line_properties(
    name: &str,
    data_count: usize,
    trailing: &[Value],
    mut cap_size_points: Option<&mut f64>,
) -> Result<Vec<GraphicsPropertyUpdate>, BuiltinError> {
    let mut properties = Vec::new();
    let mut offset = 0;
    if trailing.len() % 2 == 1 {
        properties.extend(parse_line_spec(data_count + 1, &trailing[0])?);
        offset = 1;
    }
    let pairs = &trailing[offset..];
    for (pair, values) in pairs.chunks_exact(2).enumerate() {
        let name_position = data_count + offset + pair * 2 + 1;
        let value_position = name_position + 1;
        let property = lowercase_ascii_text(name, name_position, &values[0])?;
        if property == "capsize" {
            let Some(target) = cap_size_points.as_deref_mut() else {
                return Err(BuiltinError::new(
                    BuiltinErrorCategory::Graphics,
                    format!("`{name}` does not define property CapSize"),
                ));
            };
            let value = values[1].as_real_number().ok_or_else(|| {
                type_error(
                    name,
                    value_position,
                    "nonnegative finite CapSize",
                    &values[1],
                )
            })?;
            if !value.is_finite() || value < 0.0 {
                return Err(BuiltinError::new(
                    BuiltinErrorCategory::Domain,
                    "CapSize must be a nonnegative finite scalar",
                ));
            }
            *target = value;
        } else {
            properties.push(parse_property_update(
                name,
                name_position,
                value_position,
                GraphicsClass::LineSeries,
                &values[0],
                &values[1],
            )?);
        }
    }
    Ok(properties)
}

fn parse_chart_patch_properties(
    name: &str,
    data_count: usize,
    trailing: &[Value],
) -> Result<Vec<GraphicsPropertyUpdate>, BuiltinError> {
    if !trailing.len().is_multiple_of(2) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            format!("`{name}` options must be name/value pairs"),
        ));
    }
    trailing
        .chunks_exact(2)
        .enumerate()
        .map(|(pair, values)| {
            let name_position = data_count + pair * 2 + 1;
            parse_property_update(
                name,
                name_position,
                name_position + 1,
                GraphicsClass::PatchSeries,
                &values[0],
                &values[1],
            )
        })
        .collect()
}

fn line_primitive(
    x: Vec<f64>,
    y: Vec<f64>,
    properties: Vec<GraphicsPropertyUpdate>,
    automatic_color: bool,
) -> ChartPrimitiveInput {
    ChartPrimitiveInput::Line {
        line: LineInput {
            x: NumericData::from_f64(Arc::from(x)),
            y: NumericData::from_f64(Arc::from(y)),
            z: None,
        },
        properties,
        automatic_color,
    }
}

fn patch_primitive(
    patch: PatchInput,
    properties: Vec<GraphicsPropertyUpdate>,
    automatic_color: bool,
) -> ChartPrimitiveInput {
    ChartPrimitiveInput::Patch {
        patch,
        properties,
        automatic_color,
    }
}

fn create_chart(
    name: &str,
    axes: Option<GraphicsHandle>,
    kind: ChartGroupKind,
    primitives: Vec<ChartPrimitiveInput>,
    properties: Vec<GraphicsPropertyUpdate>,
    axes_properties: Vec<GraphicsPropertyUpdate>,
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    created_handle(
        context,
        GraphicsRequest::ChartGroup {
            axes,
            chart: ChartGroupInput {
                kind,
                primitives,
                properties,
                axes_properties,
            },
        },
        name,
    )
}

#[allow(clippy::cast_precision_loss)]
fn polygons_patch(
    polygons: &[Vec<(f64, f64)>],
    face_cdata: &[f64],
    face_color: SurfaceColor,
    edge_color: SurfaceColor,
) -> Result<PatchInput, BuiltinError> {
    if polygons.is_empty() || polygons.iter().any(|polygon| polygon.len() < 3) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "Patch-backed chart polygons must contain at least three vertices",
        ));
    }
    if !face_cdata.is_empty() && face_cdata.len() != polygons.len() {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "Patch-backed chart CData must contain one value per face",
        ));
    }
    let face_rows = polygons.len();
    let face_columns = polygons.iter().map(Vec::len).max().unwrap_or(0);
    let vertex_rows = polygons.iter().map(Vec::len).sum::<usize>();
    let mut faces = Vec::with_capacity(face_rows * face_columns);
    let mut offsets = Vec::with_capacity(face_rows);
    let mut next_vertex = 1usize;
    for polygon in polygons {
        offsets.push(next_vertex);
        next_vertex += polygon.len();
    }
    for column in 0..face_columns {
        for (row, polygon) in polygons.iter().enumerate() {
            faces.push(if column < polygon.len() {
                (offsets[row] + column) as f64
            } else {
                f64::NAN
            });
        }
    }
    let mut vertices = Vec::with_capacity(vertex_rows * 2);
    vertices.extend(polygons.iter().flatten().map(|point| point.0));
    vertices.extend(polygons.iter().flatten().map(|point| point.1));
    Ok(PatchInput {
        faces: NumericData::from_f64(Arc::from(faces)),
        vertices: NumericData::from_f64(Arc::from(vertices)),
        face_vertex_cdata: NumericData::from_f64(Arc::from(face_cdata.to_vec())),
        face_rows: face_rows as u64,
        face_columns: face_columns as u64,
        vertex_rows: vertex_rows as u64,
        vertex_columns: 2,
        cdata_rows: face_cdata.len() as u64,
        cdata_columns: usize::from(!face_cdata.is_empty()) as u64,
        face_color,
        edge_color,
    })
}

fn finite_span(values: &[f64]) -> Option<f64> {
    let mut minimum = f64::INFINITY;
    let mut maximum = f64::NEG_INFINITY;
    for value in values.iter().copied().filter(|value| value.is_finite()) {
        minimum = minimum.min(value);
        maximum = maximum.max(value);
    }
    (minimum <= maximum).then_some((maximum - minimum).max(1.0))
}

fn minimum_positive_spacing(values: &[f64]) -> Option<f64> {
    let mut sorted = values
        .iter()
        .copied()
        .filter(|value| value.is_finite())
        .collect::<Vec<_>>();
    sorted.sort_by(f64::total_cmp);
    sorted
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .filter(|spacing| *spacing > 0.0)
        .min_by(f64::total_cmp)
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::float_cmp
)]
fn automatic_histogram_edges(values: &[f64], requested_count: Option<usize>) -> Vec<f64> {
    if values.is_empty() {
        return vec![0.0, 1.0];
    }
    let minimum = values.iter().copied().fold(f64::INFINITY, f64::min);
    let maximum = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if minimum == maximum {
        return vec![minimum - 0.5, maximum + 0.5];
    }
    if requested_count.is_none()
        && values.iter().all(|value| value.fract() == 0.0)
        && maximum - minimum <= 65_535.0
    {
        let count = (maximum - minimum) as usize + 1;
        return (0..=count)
            .map(|index| minimum - 0.5 + index as f64)
            .collect();
    }
    let count = requested_count
        .unwrap_or_else(|| ((values.len() as f64).log2() + 1.0).ceil() as usize)
        .clamp(1, 65_536);
    let width = (maximum - minimum) / count as f64;
    (0..=count)
        .map(|index| {
            if index == count {
                maximum
            } else {
                minimum + index as f64 * width
            }
        })
        .collect()
}

#[allow(clippy::float_cmp)]
fn histogram_counts(values: &[f64], edges: &[f64]) -> Vec<f64> {
    let mut counts = vec![0.0; edges.len().saturating_sub(1)];
    for &value in values {
        if value < edges[0] || value > edges[edges.len() - 1] {
            continue;
        }
        let index = if value == edges[edges.len() - 1] {
            counts.len().saturating_sub(1)
        } else {
            edges
                .partition_point(|edge| *edge <= value)
                .saturating_sub(1)
        };
        if let Some(count) = counts.get_mut(index) {
            *count += 1.0;
        }
    }
    counts
}

#[allow(clippy::cast_precision_loss)]
fn normalize_histogram(
    counts: &mut [f64],
    edges: &[f64],
    sample_count: usize,
    normalization: &str,
) {
    let denominator = sample_count.max(1) as f64;
    match normalization {
        "probability" => counts.iter_mut().for_each(|count| *count /= denominator),
        "pdf" => counts.iter_mut().enumerate().for_each(|(index, count)| {
            *count /= denominator * (edges[index + 1] - edges[index]);
        }),
        "countdensity" => counts.iter_mut().enumerate().for_each(|(index, count)| {
            *count /= edges[index + 1] - edges[index];
        }),
        "cdf" | "cumcount" => {
            let mut accumulated = 0.0;
            for count in counts {
                accumulated += *count;
                *count = if normalization == "cdf" {
                    accumulated / denominator
                } else {
                    accumulated
                };
            }
        }
        _ => {}
    }
}

type ContourData = (Vec<f64>, Vec<f64>, Vec<f64>, usize, usize, usize);

fn prepare_contour_data(arguments: &[Value]) -> Result<ContourData, BuiltinError> {
    if arguments.len() >= 3 && arguments[..3].iter().all(is_plot_numeric) {
        let x = PreparedNumeric::from_value("contour", 1, &arguments[0])?;
        let y = PreparedNumeric::from_value("contour", 2, &arguments[1])?;
        let z = PreparedNumeric::from_value("contour", 3, &arguments[2])?;
        require_contour_matrix(&z)?;
        let vectors =
            x.is_vector() && y.is_vector() && x.numel() == z.columns && y.numel() == z.rows;
        let matrices = x.rows == z.rows
            && x.columns == z.columns
            && y.rows == z.rows
            && y.columns == z.columns;
        if !vectors && !matrices {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "`contour` X/Y coordinates must match the Z matrix",
            ));
        }
        Ok((
            x.vector_f64(),
            y.vector_f64(),
            z.vector_f64(),
            z.rows,
            z.columns,
            3,
        ))
    } else {
        let Some(value) = arguments.first() else {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::ArgumentCount,
                "`contour` requires Z or X/Y/Z data",
            ));
        };
        let z = PreparedNumeric::from_value("contour", 1, value)?;
        require_contour_matrix(&z)?;
        Ok((
            default_f64_x(z.columns),
            default_f64_x(z.rows),
            z.vector_f64(),
            z.rows,
            z.columns,
            1,
        ))
    }
}

fn require_contour_matrix(data: &PreparedNumeric) -> Result<(), BuiltinError> {
    if data.rows >= 2 && data.columns >= 2 {
        Ok(())
    } else {
        Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "`contour` ZData must contain at least two rows and columns",
        ))
    }
}

fn contour_range(z: &[f64]) -> Option<(f64, f64)> {
    let minimum = z
        .iter()
        .copied()
        .filter(|value| value.is_finite())
        .fold(f64::INFINITY, f64::min);
    let maximum = z
        .iter()
        .copied()
        .filter(|value| value.is_finite())
        .fold(f64::NEG_INFINITY, f64::max);
    (minimum.is_finite() && maximum.is_finite()).then_some((minimum, maximum))
}

#[allow(clippy::cast_precision_loss, clippy::float_cmp)]
fn requested_contour_levels(z: &[f64], requested: usize) -> Vec<f64> {
    let Some((minimum, maximum)) = contour_range(z) else {
        return Vec::new();
    };
    if minimum == maximum {
        return vec![minimum];
    }
    let count = requested.clamp(1, 256);
    let step = (maximum - minimum) / (count + 1) as f64;
    (1..=count)
        .map(|index| minimum + index as f64 * step)
        .collect()
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::float_cmp
)]
fn automatic_contour_levels(z: &[f64]) -> Vec<f64> {
    let Some((minimum, maximum)) = contour_range(z) else {
        return Vec::new();
    };
    if minimum == maximum {
        return vec![minimum];
    }
    let raw_step = (maximum - minimum) / 10.0;
    let exponent = raw_step.abs().log10().floor();
    let scale = 10.0_f64.powf(exponent);
    let fraction = raw_step / scale;
    let nice_fraction = if fraction <= 1.0 {
        1.0
    } else if fraction <= 2.0 {
        2.0
    } else if fraction <= 2.5 {
        2.5
    } else if fraction <= 5.0 {
        5.0
    } else {
        10.0
    };
    let step = nice_fraction * scale;
    let tolerance = step.abs() * 16.0 * f64::EPSILON;
    let mut first = (minimum / step).ceil() * step;
    if minimum >= 0.0 && (first - minimum).abs() <= tolerance {
        first += step;
    }
    let last = (maximum / step).floor() * step;
    if first > last + tolerance {
        return requested_contour_levels(z, 10);
    }
    let count = (((last - first) / step).floor() as usize + 1).min(256);
    (0..count)
        .map(|index| {
            let level = first + index as f64 * step;
            if level.abs() <= tolerance { 0.0 } else { level }
        })
        .collect()
}

fn contour_color_limits(levels: &[f64]) -> [f64; 2] {
    let minimum = levels[0];
    let maximum = levels[levels.len() - 1];
    if minimum < maximum {
        [minimum, maximum]
    } else {
        [minimum - 1.0, maximum + 1.0]
    }
}

type Segment2D = ((f64, f64), (f64, f64));

fn marching_squares(
    x: &[f64],
    y: &[f64],
    z: &[f64],
    rows: usize,
    columns: usize,
    level: f64,
) -> Vec<Segment2D> {
    let mut segments = Vec::new();
    for column in 0..columns - 1 {
        for row in 0..rows - 1 {
            let corners = [
                contour_point(x, y, z, rows, columns, row, column),
                contour_point(x, y, z, rows, columns, row, column + 1),
                contour_point(x, y, z, rows, columns, row + 1, column + 1),
                contour_point(x, y, z, rows, columns, row + 1, column),
            ];
            if corners.iter().any(|point| !point.2.is_finite()) {
                continue;
            }
            let edges = [(0, 1), (1, 2), (2, 3), (3, 0)];
            let crossings = edges
                .into_iter()
                .filter_map(|(start, end)| contour_crossing(corners[start], corners[end], level))
                .collect::<Vec<_>>();
            match crossings.as_slice() {
                [start, end] => segments.push((*start, *end)),
                [first, second, third, fourth] => {
                    segments.push((*first, *second));
                    segments.push((*third, *fourth));
                }
                _ => {}
            }
        }
    }
    segments
}

fn contour_point(
    x: &[f64],
    y: &[f64],
    z: &[f64],
    rows: usize,
    columns: usize,
    row: usize,
    column: usize,
) -> (f64, f64, f64) {
    let index = row + column * rows;
    let x = if x.len() == columns {
        x[column]
    } else {
        x[index]
    };
    let y = if y.len() == rows { y[row] } else { y[index] };
    (x, y, z[index])
}

#[allow(clippy::float_cmp)]
fn contour_crossing(
    start: (f64, f64, f64),
    end: (f64, f64, f64),
    level: f64,
) -> Option<(f64, f64)> {
    let crosses = (start.2 < level && end.2 >= level) || (end.2 < level && start.2 >= level);
    if !crosses || start.2 == end.2 {
        return None;
    }
    let fraction = (level - start.2) / (end.2 - start.2);
    Some((
        start.0 + fraction * (end.0 - start.0),
        start.1 + fraction * (end.1 - start.1),
    ))
}

type ScalarPoint2D = (f64, f64, f64);

const CONTOURF_COMPACT_WORK_THRESHOLD: usize = 8_000_000;

fn use_compact_filled_contours(
    rows: usize,
    columns: usize,
    level_count: usize,
    properties: &[GraphicsPropertyUpdate],
) -> bool {
    let edges_disabled = properties.iter().rev().find_map(|property| match property {
        GraphicsPropertyUpdate::EdgeColor(color) => Some(*color == SurfaceColor::None),
        _ => None,
    }) == Some(true);
    let work = rows
        .saturating_sub(1)
        .saturating_mul(columns.saturating_sub(1))
        .saturating_mul(2)
        .saturating_mul(level_count.saturating_add(1));
    edges_disabled && work > CONTOURF_COMPACT_WORK_THRESHOLD
}

fn clipped_filled_contour_patch(
    x: &[f64],
    y: &[f64],
    z: &[f64],
    rows: usize,
    columns: usize,
    levels: &[f64],
) -> Result<PatchInput, BuiltinError> {
    let mut polygons = Vec::new();
    let mut face_cdata = Vec::new();
    for column in 0..columns - 1 {
        for row in 0..rows - 1 {
            let corners = [
                contour_point(x, y, z, rows, columns, row, column),
                contour_point(x, y, z, rows, columns, row, column + 1),
                contour_point(x, y, z, rows, columns, row + 1, column + 1),
                contour_point(x, y, z, rows, columns, row + 1, column),
            ];
            if corners
                .iter()
                .any(|point| !point.0.is_finite() || !point.1.is_finite() || !point.2.is_finite())
            {
                continue;
            }
            for triangle in [
                [corners[0], corners[1], corners[2]],
                [corners[0], corners[2], corners[3]],
            ] {
                append_filled_contour_triangle(triangle, levels, &mut polygons, &mut face_cdata);
            }
        }
    }
    if polygons.is_empty() {
        return Err(no_drawable_contour_cell());
    }
    polygons_patch(
        &polygons,
        &face_cdata,
        SurfaceColor::Flat,
        SurfaceColor::Rgba([0.0, 0.0, 0.0, 1.0]),
    )
}

#[allow(clippy::cast_precision_loss)]
fn compact_filled_contour_patch(
    x: &[f64],
    y: &[f64],
    z: &[f64],
    rows: usize,
    columns: usize,
    levels: &[f64],
) -> Result<PatchInput, BuiltinError> {
    let source_count = rows.checked_mul(columns).ok_or_else(|| {
        BuiltinError::new(
            BuiltinErrorCategory::Other,
            "`contourf` grid size exceeds host limits",
        )
    })?;
    let mut source_to_vertex = vec![None; source_count];
    let mut vertices = Vec::<(f64, f64)>::new();
    let mut faces = Vec::<[f64; 3]>::new();
    let mut face_cdata = Vec::new();

    for column in 0..columns - 1 {
        for row in 0..rows - 1 {
            let source_indices = [
                row + column * rows,
                row + (column + 1) * rows,
                row + 1 + (column + 1) * rows,
                row + 1 + column * rows,
            ];
            let corners = [
                contour_point(x, y, z, rows, columns, row, column),
                contour_point(x, y, z, rows, columns, row, column + 1),
                contour_point(x, y, z, rows, columns, row + 1, column + 1),
                contour_point(x, y, z, rows, columns, row + 1, column),
            ];
            if corners
                .iter()
                .any(|point| !point.0.is_finite() || !point.1.is_finite() || !point.2.is_finite())
            {
                continue;
            }
            for local in [[0, 1, 2], [0, 2, 3]] {
                let mut face = [0.0; 3];
                let mut scalar = 0.0;
                for (destination, corner) in local.into_iter().enumerate() {
                    let source = source_indices[corner];
                    let vertex = if let Some(vertex) = source_to_vertex[source] {
                        vertex
                    } else {
                        let vertex = u64::try_from(vertices.len())
                            .ok()
                            .and_then(|value| value.checked_add(1))
                            .ok_or_else(|| {
                                BuiltinError::new(
                                    BuiltinErrorCategory::Other,
                                    "`contourf` vertex count exceeds host limits",
                                )
                            })?;
                        vertices.push((corners[corner].0, corners[corner].1));
                        source_to_vertex[source] = Some(vertex);
                        vertex
                    };
                    face[destination] = vertex as f64;
                    scalar += corners[corner].2;
                }
                faces.push(face);
                face_cdata.push(contour_band_value(scalar / 3.0, levels));
            }
        }
    }
    if faces.is_empty() {
        return Err(no_drawable_contour_cell());
    }

    let mut face_values = Vec::with_capacity(faces.len() * 3);
    for column in 0..3 {
        face_values.extend(faces.iter().map(|face| face[column]));
    }
    let mut vertex_values = Vec::with_capacity(vertices.len() * 2);
    vertex_values.extend(vertices.iter().map(|point| point.0));
    vertex_values.extend(vertices.iter().map(|point| point.1));
    Ok(PatchInput {
        faces: NumericData::from_f64(Arc::from(face_values)),
        vertices: NumericData::from_f64(Arc::from(vertex_values)),
        face_vertex_cdata: NumericData::from_f64(Arc::from(face_cdata)),
        face_rows: faces.len() as u64,
        face_columns: 3,
        vertex_rows: vertices.len() as u64,
        vertex_columns: 2,
        cdata_rows: faces.len() as u64,
        cdata_columns: 1,
        face_color: SurfaceColor::Flat,
        edge_color: SurfaceColor::None,
    })
}

fn contour_band_value(value: f64, levels: &[f64]) -> f64 {
    let band = levels.partition_point(|level| value >= *level);
    if band == 0 {
        levels[0]
    } else if band == levels.len() {
        levels[levels.len() - 1]
    } else {
        f64::midpoint(levels[band - 1], levels[band])
    }
}

fn no_drawable_contour_cell() -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        "`contourf` data does not contain a finite drawable cell",
    )
}

fn append_filled_contour_triangle(
    triangle: [ScalarPoint2D; 3],
    levels: &[f64],
    polygons: &mut Vec<Vec<(f64, f64)>>,
    face_cdata: &mut Vec<f64>,
) {
    for band in 0..=levels.len() {
        let mut polygon = triangle.to_vec();
        if band > 0 {
            polygon = clip_scalar_polygon(&polygon, levels[band - 1], true);
        }
        if band < levels.len() {
            polygon = clip_scalar_polygon(&polygon, levels[band], false);
        }
        let points = polygon
            .into_iter()
            .map(|point| (point.0, point.1))
            .collect::<Vec<_>>();
        if points.len() >= 3 && polygon_area_twice(&points).abs() > f64::EPSILON {
            polygons.push(points);
            face_cdata.push(if band == 0 {
                levels[0]
            } else if band == levels.len() {
                levels[levels.len() - 1]
            } else {
                f64::midpoint(levels[band - 1], levels[band])
            });
        }
    }
}

fn clip_scalar_polygon(
    polygon: &[ScalarPoint2D],
    level: f64,
    keep_above: bool,
) -> Vec<ScalarPoint2D> {
    let Some(&mut_previous) = polygon.last() else {
        return Vec::new();
    };
    let mut output = Vec::with_capacity(polygon.len() + 1);
    let mut previous = mut_previous;
    let mut previous_inside = scalar_inside(previous.2, level, keep_above);
    for &current in polygon {
        let current_inside = scalar_inside(current.2, level, keep_above);
        if current_inside != previous_inside {
            output.push(interpolate_scalar_point(previous, current, level));
        }
        if current_inside {
            output.push(current);
        }
        previous = current;
        previous_inside = current_inside;
    }
    output
}

fn scalar_inside(value: f64, level: f64, keep_above: bool) -> bool {
    if keep_above {
        value >= level
    } else {
        value <= level
    }
}

fn interpolate_scalar_point(start: ScalarPoint2D, end: ScalarPoint2D, level: f64) -> ScalarPoint2D {
    let denominator = end.2 - start.2;
    let fraction = if denominator == 0.0 {
        0.5
    } else {
        ((level - start.2) / denominator).clamp(0.0, 1.0)
    };
    (
        start.0 + fraction * (end.0 - start.0),
        start.1 + fraction * (end.1 - start.1),
        level,
    )
}

fn polygon_area_twice(points: &[(f64, f64)]) -> f64 {
    points
        .iter()
        .copied()
        .zip(points.iter().copied().cycle().skip(1))
        .take(points.len())
        .map(|(left, right)| left.0 * right.1 - right.0 * left.1)
        .sum()
}

#[allow(clippy::cast_precision_loss)]
fn contour_outputs(
    response: &GraphicsResponse,
    segments: Vec<(f64, Vec<(f64, f64)>)>,
    requested_outputs: usize,
) -> BuiltinResult {
    let GraphicsResponse::Handle(handle) = response else {
        return Err(graphics_state_error("contour", "scalar graphics handle"));
    };
    let columns = segments
        .iter()
        .map(|(_, points)| points.len() + 1)
        .sum::<usize>();
    let mut values = Vec::with_capacity(columns * 2);
    for (level, points) in segments {
        values.extend([level, points.len() as f64]);
        for (x, y) in points {
            values.extend([x, y]);
        }
    }
    let shape = Shape::new([2, columns as u64])
        .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Other, error.to_string()))?;
    let matrix = DenseArray::from_vec(shape, values)
        .map(ArrayData::F64)
        .map(Value::Array)
        .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Other, error.to_string()))?;
    Ok(match requested_outputs {
        0 => Vec::new(),
        1 => vec![matrix],
        _ => vec![matrix, Value::Graphics(*handle)],
    })
}

fn image_chart(
    name: &'static str,
    mapping: CDataMapping,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_max_outputs(name, context, 1)?;
    let (axes, arguments) = optional_axes(name, arguments)?;
    let (x, y, c, data_count) =
        if arguments.len() >= 3 && arguments[..3].iter().all(is_plot_numeric) {
            (
                Some(PreparedNumeric::from_value(name, 1, &arguments[0])?),
                Some(PreparedNumeric::from_value(name, 2, &arguments[1])?),
                PreparedNumeric::from_value(name, 3, &arguments[2])?,
                3,
            )
        } else {
            let Some(value) = arguments.first() else {
                return Err(BuiltinError::new(
                    BuiltinErrorCategory::ArgumentCount,
                    format!("`{name}` requires CData"),
                ));
            };
            (None, None, PreparedNumeric::from_value(name, 1, value)?, 1)
        };
    if c.rows == 0 || c.columns == 0 {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` empty CData is not yet renderable"),
        ));
    }
    let x_centers = image_centers(name, x.as_ref(), c.columns, "XData")?;
    let y_centers = image_centers(name, y.as_ref(), c.rows, "YData")?;
    let x_edges = centers_to_edges(&x_centers);
    let y_edges = centers_to_edges(&y_centers);
    let mut polygons = Vec::with_capacity(c.rows * c.columns);
    let mut colors = Vec::with_capacity(c.rows * c.columns);
    let c_values = c.vector_f64();
    for column in 0..c.columns {
        for row in 0..c.rows {
            polygons.push(vec![
                (x_edges[column], y_edges[row]),
                (x_edges[column + 1], y_edges[row]),
                (x_edges[column + 1], y_edges[row + 1]),
                (x_edges[column], y_edges[row + 1]),
            ]);
            colors.push(c_values[row + column * c.rows]);
        }
    }
    let mut properties = parse_chart_patch_properties(name, data_count, &arguments[data_count..])?;
    properties.push(GraphicsPropertyUpdate::CDataMapping(mapping));
    create_chart(
        name,
        axes,
        ChartGroupKind::Image,
        vec![patch_primitive(
            polygons_patch(&polygons, &colors, SurfaceColor::Flat, SurfaceColor::None)?,
            properties,
            false,
        )],
        Vec::new(),
        vec![GraphicsPropertyUpdate::YDir(AxisDirection::Reverse)],
        context,
    )
}

#[allow(clippy::cast_precision_loss)]
fn image_centers(
    name: &str,
    coordinates: Option<&PreparedNumeric>,
    count: usize,
    property: &str,
) -> Result<Vec<f64>, BuiltinError> {
    let Some(coordinates) = coordinates else {
        return Ok(default_f64_x(count));
    };
    if !coordinates.is_vector() {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` {property} must be a vector"),
        ));
    }
    let values = coordinates.vector_f64();
    if values.len() == count {
        return Ok(values);
    }
    if values.len() == 2 {
        if count == 1 {
            return Ok(vec![values[0]]);
        }
        let step = (values[1] - values[0]) / (count - 1) as f64;
        return Ok((0..count)
            .map(|index| values[0] + index as f64 * step)
            .collect());
    }
    Err(BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("`{name}` {property} must have two elements or match CData"),
    ))
}

fn centers_to_edges(centers: &[f64]) -> Vec<f64> {
    if centers.len() == 1 {
        return vec![centers[0] - 0.5, centers[0] + 0.5];
    }
    let mut edges = Vec::with_capacity(centers.len() + 1);
    edges.push(centers[0] - (centers[1] - centers[0]) / 2.0);
    edges.extend(
        centers
            .windows(2)
            .map(|pair| f64::midpoint(pair[0], pair[1])),
    );
    edges.push(
        centers[centers.len() - 1]
            + (centers[centers.len() - 1] - centers[centers.len() - 2]) / 2.0,
    );
    edges
}

fn prepare_styled_plot(
    arguments: &[Value],
) -> Result<(Vec<LineInput>, Vec<GraphicsPropertyUpdate>), BuiltinError> {
    let data_count = if arguments.len() >= 2 && is_plot_numeric(&arguments[1]) {
        2
    } else {
        1
    };
    let lines = prepare_plot(&arguments[..data_count])?;
    let trailing = &arguments[data_count..];
    let mut properties = Vec::new();
    let mut offset = 0;
    if trailing.len() % 2 == 1 {
        properties.extend(parse_line_spec(data_count + 1, &trailing[0])?);
        offset = 1;
    }
    let pairs = &trailing[offset..];
    for pair in 0..(pairs.len() / 2) {
        let name_position = data_count + offset + pair * 2 + 1;
        let value_position = name_position + 1;
        properties.push(parse_property_update(
            "plot",
            name_position,
            value_position,
            GraphicsClass::LineSeries,
            &pairs[pair * 2],
            &pairs[pair * 2 + 1],
        )?);
    }
    Ok((lines, properties))
}

fn prepare_plot_series(
    arguments: &[Value],
) -> Result<(Vec<StyledLineInput>, Vec<GraphicsPropertyUpdate>), BuiltinError> {
    let mut cursor = 0;
    let mut series = Vec::new();
    while cursor < arguments.len() && is_plot_numeric(&arguments[cursor]) {
        let data_start = cursor;
        let data_count = if arguments.get(cursor + 1).is_some_and(is_plot_numeric) {
            2
        } else {
            1
        };
        cursor += data_count;
        let mut local_properties = Vec::new();
        if let Some(value) = arguments.get(cursor)
            && is_text_value(value)
        {
            let remaining = arguments.len() - cursor;
            let followed_by_data = arguments.get(cursor + 1).is_some_and(is_plot_numeric);
            let is_property_name =
                lowercase_ascii_text("plot", cursor + 1, value).is_ok_and(|name| {
                    graphics_property("plot", GraphicsClass::LineSeries, &name).is_ok()
                });
            let parsed_line_spec = parse_line_spec(cursor + 1, value);
            if remaining == 1 {
                match parsed_line_spec {
                    Ok(properties) => local_properties = properties,
                    Err(_) if is_property_name => {}
                    Err(error) => return Err(error),
                }
                if !local_properties.is_empty() {
                    cursor += 1;
                }
            } else if !is_property_name && (followed_by_data || remaining % 2 == 1) {
                local_properties = parsed_line_spec?;
                cursor += 1;
            }
        }
        let lines = prepare_plot(&arguments[data_start..data_start + data_count])?;
        series.extend(lines.into_iter().map(|line| StyledLineInput {
            line,
            properties: local_properties.clone(),
        }));
    }
    if cursor == 0 {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            "`plot` requires one or more numerical data groups",
        ));
    }
    let trailing = &arguments[cursor..];
    if !trailing.len().is_multiple_of(2) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            "`plot` properties must be supplied as complete name/value pairs",
        ));
    }
    let properties = trailing
        .chunks_exact(2)
        .enumerate()
        .map(|(pair, values)| {
            let name_position = cursor + pair * 2 + 1;
            parse_property_update(
                "plot",
                name_position,
                name_position + 1,
                GraphicsClass::LineSeries,
                &values[0],
                &values[1],
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((series, properties))
}

fn is_plot_numeric(value: &Value) -> bool {
    matches!(
        value,
        Value::Double(_) | Value::Array(ArrayData::F64(_) | ArrayData::F32(_))
    )
}

fn parse_line_spec(
    position: usize,
    value: &Value,
) -> Result<Vec<GraphicsPropertyUpdate>, BuiltinError> {
    let spec = ascii_text("plot", position, value)?;
    if spec.is_empty() {
        return Err(line_spec_error(position, &spec));
    }
    let bytes = spec.as_bytes();
    let mut cursor = 0;
    let mut style = None;
    let mut marker = None;
    let mut color = None;
    while cursor < bytes.len() {
        let remaining = &spec[cursor..];
        let (consumed, next_style) = if remaining.starts_with("--") {
            (2, Some(LineStyle::Dash))
        } else if remaining.starts_with("-.") {
            (2, Some(LineStyle::DashDot))
        } else if remaining.starts_with('-') {
            (1, Some(LineStyle::Solid))
        } else if remaining.starts_with(':') {
            (1, Some(LineStyle::Dot))
        } else {
            (0, None)
        };
        if let Some(next_style) = next_style {
            if style.replace(next_style).is_some() {
                return Err(line_spec_error(position, &spec));
            }
            cursor += consumed;
            continue;
        }
        let byte = bytes[cursor];
        if let Some(next_marker) = marker_symbol(byte) {
            if marker.replace(next_marker).is_some() {
                return Err(line_spec_error(position, &spec));
            }
            cursor += 1;
            continue;
        }
        if let Some(next_color) = short_color(byte) {
            if color.replace(next_color).is_some() {
                return Err(line_spec_error(position, &spec));
            }
            cursor += 1;
            continue;
        }
        return Err(line_spec_error(position, &spec));
    }
    let mut updates = Vec::with_capacity(3);
    if let Some(style) = style {
        updates.push(GraphicsPropertyUpdate::LineStyle(style));
    }
    if let Some(marker) = marker {
        updates.push(GraphicsPropertyUpdate::Marker(marker));
    }
    if let Some(color) = color {
        updates.push(GraphicsPropertyUpdate::Color(color));
    }
    Ok(updates)
}

fn line_spec_error(position: usize, spec: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!(
            "input {position} to `plot` is not a supported LineSpec: `{spec}`; use one line style, optional supported marker, and optional MATLAB short color"
        ),
    )
}

fn prepare_plot(arguments: &[Value]) -> Result<Vec<LineInput>, BuiltinError> {
    if arguments.len() == 1 {
        let y = PreparedNumeric::from_value("plot", 1, &arguments[0])?;
        return Ok(prepare_plot_y(&y));
    }
    let x = PreparedNumeric::from_value("plot", 1, &arguments[0])?;
    let y = PreparedNumeric::from_value("plot", 2, &arguments[1])?;
    prepare_plot_xy(&x, &y)
}

fn prepare_plot3_xyz(
    x: &PreparedNumeric,
    y: &PreparedNumeric,
    z: &PreparedNumeric,
) -> Result<Vec<LineInput>, BuiltinError> {
    if x.numel() == 0 && y.numel() == 0 && z.numel() == 0 {
        return Ok(Vec::new());
    }
    if x.is_vector() && y.is_vector() && z.is_vector() {
        if x.numel() != y.numel() || x.numel() != z.numel() {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "`plot3` X, Y, and Z vectors must have equal lengths",
            ));
        }
        return Ok(vec![LineInput {
            x: x.vector(),
            y: y.vector(),
            z: Some(z.vector()),
        }]);
    }
    if x.rows == y.rows && x.columns == y.columns && x.rows == z.rows && x.columns == z.columns {
        let mut lines = Vec::with_capacity(z.columns);
        for column in 0..z.columns {
            lines.push(LineInput {
                x: x.column(column),
                y: y.column(column),
                z: Some(z.column(column)),
            });
        }
        return Ok(lines);
    }
    Err(BuiltinError::new(
        BuiltinErrorCategory::Domain,
        "`plot3` requires equal-length vectors or equal-sized real matrices",
    ))
}

fn prepare_surface(name: &str, arguments: &[Value]) -> Result<SurfaceInput, BuiltinError> {
    let (x, y, z, c) = match arguments.len() {
        1 => {
            let z = PreparedNumeric::from_value(name, 1, &arguments[0])?;
            require_surface_matrix(name, &z)?;
            let x = default_surface_coordinate(z.columns);
            let y = default_surface_coordinate(z.rows);
            (x, y, z.values.clone(), z.values)
        }
        2 => {
            let z = PreparedNumeric::from_value(name, 1, &arguments[0])?;
            require_surface_matrix(name, &z)?;
            let c = PreparedNumeric::from_value(name, 2, &arguments[1])?;
            if c.rows != z.rows || c.columns != z.columns {
                return Err(BuiltinError::new(
                    BuiltinErrorCategory::Domain,
                    format!("`{name}` requires CData with the same shape as Z"),
                ));
            }
            let x = default_surface_coordinate(z.columns);
            let y = default_surface_coordinate(z.rows);
            (x, y, z.values, c.values)
        }
        3 | 4 => {
            let x = PreparedNumeric::from_value(name, 1, &arguments[0])?;
            let y = PreparedNumeric::from_value(name, 2, &arguments[1])?;
            let z = PreparedNumeric::from_value(name, 3, &arguments[2])?;
            require_surface_matrix(name, &z)?;
            let vector_coordinates =
                x.is_vector() && y.is_vector() && x.numel() == z.columns && y.numel() == z.rows;
            let matrix_coordinates = x.rows == z.rows
                && x.columns == z.columns
                && y.rows == z.rows
                && y.columns == z.columns;
            if !vector_coordinates && !matrix_coordinates {
                return Err(BuiltinError::new(
                    BuiltinErrorCategory::Domain,
                    format!(
                        "`{name}` requires X/Y vectors matching Z columns/rows or X/Y matrices matching Z"
                    ),
                ));
            }
            let c = if let Some(value) = arguments.get(3) {
                let c = PreparedNumeric::from_value(name, 4, value)?;
                if c.rows != z.rows || c.columns != z.columns {
                    return Err(BuiltinError::new(
                        BuiltinErrorCategory::Domain,
                        format!("`{name}` requires CData with the same shape as Z"),
                    ));
                }
                c.values
            } else {
                z.values.clone()
            };
            (x.values, y.values, z.values, c)
        }
        _ => {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::ArgumentCount,
                format!("`{name}` requires Z, Z/C, X/Y/Z, or X/Y/Z/CData"),
            ));
        }
    };
    let rows = u64::try_from(match arguments.len() {
        1 | 2 => y.len(),
        3 | 4 => PreparedNumeric::from_value(name, 3, &arguments[2])?.rows,
        _ => unreachable!("validated surface arity"),
    })
    .map_err(|_| plot_size_error(name))?;
    let columns = u64::try_from(match arguments.len() {
        1 | 2 => x.len(),
        3 | 4 => PreparedNumeric::from_value(name, 3, &arguments[2])?.columns,
        _ => unreachable!("validated surface arity"),
    })
    .map_err(|_| plot_size_error(name))?;
    Ok(SurfaceInput {
        x,
        y,
        z,
        c,
        rows,
        columns,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PatchConstructor {
    Patch,
    Fill,
    Fill3,
}

#[allow(clippy::too_many_lines)]
fn prepare_patch(
    name: &str,
    arguments: &[Value],
    constructor: PatchConstructor,
) -> Result<(PatchInput, Vec<GraphicsPropertyUpdate>), BuiltinError> {
    if constructor == PatchConstructor::Patch
        && let Some(Value::Struct(structure)) = arguments.first()
    {
        return prepare_patch_struct(name, structure, &arguments[1..]);
    }
    if constructor == PatchConstructor::Patch
        && arguments.first().is_some_and(|value| {
            matches!(value, Value::String(_) | Value::Array(ArrayData::Char(_)))
        })
    {
        return prepare_patch_name_value(name, arguments);
    }
    let data_count = patch_data_count(arguments, constructor)?;
    if arguments.len() < data_count || !(arguments.len() - data_count).is_multiple_of(2) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            format!("`{name}` requires complete property name/value pairs after its data"),
        ));
    }
    let x = PreparedNumeric::from_value(name, 1, &arguments[0])?;
    let y = PreparedNumeric::from_value(name, 2, &arguments[1])?;
    let (z, color_position) = if data_count == 4 {
        (
            Some(PreparedNumeric::from_value(name, 3, &arguments[2])?),
            4,
        )
    } else {
        (None, 3)
    };
    let color = &arguments[color_position - 1];
    let (faces_per_column, vertices_per_face) = patch_coordinate_shape(name, &x, &y, z.as_ref())?;
    let vertex_count = faces_per_column
        .checked_mul(vertices_per_face)
        .ok_or_else(|| plot_size_error(name))?;
    let face_values = patch_face_values(name, faces_per_column, vertices_per_face)?;
    let mut vertex_values = Vec::with_capacity(vertex_count * if z.is_some() { 3 } else { 2 });
    vertex_values.extend(x.vector_f64());
    vertex_values.extend(y.vector_f64());
    if let Some(z) = &z {
        vertex_values.extend(z.vector_f64());
    }
    let (cdata, face_color, numeric_cdata) =
        if matches!(color, Value::String(_) | Value::Array(ArrayData::Char(_))) {
            (
                Vec::new(),
                SurfaceColor::Rgba(rgb_color(name, color_position, "Patch face color", color)?),
                false,
            )
        } else {
            let cdata = PreparedNumeric::from_value(name, color_position, color)?;
            if cdata.rows == 1 && cdata.columns == 3 {
                (
                    Vec::new(),
                    SurfaceColor::Rgba(rgb_color(name, color_position, "Patch face color", color)?),
                    false,
                )
            } else if cdata.rows != x.rows || cdata.columns != x.columns {
                return Err(BuiltinError::new(
                    BuiltinErrorCategory::Domain,
                    format!("input {color_position} to `{name}` must match X and Y dimensions"),
                ));
            } else {
                (
                    cdata.vector_f64(),
                    if constructor == PatchConstructor::Patch {
                        SurfaceColor::Interp
                    } else {
                        SurfaceColor::Flat
                    },
                    true,
                )
            }
        };
    let mut properties = Vec::new();
    for (pair, values) in arguments[data_count..].chunks_exact(2).enumerate() {
        let name_position = data_count + pair * 2 + 1;
        properties.push(parse_property_update(
            name,
            name_position,
            name_position + 1,
            GraphicsClass::PatchSeries,
            &values[0],
            &values[1],
        )?);
    }
    Ok((
        PatchInput {
            faces: NumericData::from_f64(Arc::from(face_values)),
            vertices: NumericData::from_f64(Arc::from(vertex_values)),
            face_vertex_cdata: NumericData::from_f64(Arc::from(cdata)),
            face_rows: u64::try_from(faces_per_column).map_err(|_| plot_size_error(name))?,
            face_columns: u64::try_from(vertices_per_face).map_err(|_| plot_size_error(name))?,
            vertex_rows: u64::try_from(vertex_count).map_err(|_| plot_size_error(name))?,
            vertex_columns: if z.is_some() { 3 } else { 2 },
            cdata_rows: if numeric_cdata {
                u64::try_from(vertex_count).map_err(|_| plot_size_error(name))?
            } else {
                0
            },
            cdata_columns: u64::from(numeric_cdata),
            face_color,
            edge_color: SurfaceColor::Rgba([0.0, 0.0, 0.0, 1.0]),
        },
        properties,
    ))
}

fn patch_data_count(
    arguments: &[Value],
    constructor: PatchConstructor,
) -> Result<usize, BuiltinError> {
    let data_count = match constructor {
        PatchConstructor::Fill => 3,
        PatchConstructor::Fill3 => 4,
        PatchConstructor::Patch if arguments.len() >= 3 && arguments.len() % 2 == 1 => 3,
        PatchConstructor::Patch if arguments.len() >= 4 => 4,
        PatchConstructor::Patch => {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::ArgumentCount,
                "`patch` requires X/Y/C, X/Y/Z/C, or Faces/Vertices name-value pairs",
            ));
        }
    };
    Ok(data_count)
}

fn prepare_patch_name_value(
    name: &str,
    arguments: &[Value],
) -> Result<(PatchInput, Vec<GraphicsPropertyUpdate>), BuiltinError> {
    if arguments.len() < 4 || !arguments.len().is_multiple_of(2) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            "`patch` Faces/Vertices construction requires complete name/value pairs",
        ));
    }
    let mut faces = None;
    let mut vertices = None;
    let mut cdata = None;
    let mut properties = Vec::new();
    for (pair, values) in arguments.chunks_exact(2).enumerate() {
        let name_position = pair * 2 + 1;
        let property_name = lowercase_ascii_text(name, name_position, &values[0])?;
        match property_name.as_str() {
            "faces" => {
                faces = Some(PreparedNumeric::from_value(
                    name,
                    name_position + 1,
                    &values[1],
                )?);
            }
            "vertices" => {
                vertices = Some(PreparedNumeric::from_value(
                    name,
                    name_position + 1,
                    &values[1],
                )?);
            }
            "facevertexcdata" => {
                cdata = Some(PreparedNumeric::from_value(
                    name,
                    name_position + 1,
                    &values[1],
                )?);
            }
            _ => properties.push(parse_property_update(
                name,
                name_position,
                name_position + 1,
                GraphicsClass::PatchSeries,
                &values[0],
                &values[1],
            )?),
        }
    }
    finish_prepared_patch(name, faces, vertices, cdata, properties)
}

fn prepare_patch_struct(
    name: &str,
    structure: &openmat_value::StructArray,
    property_arguments: &[Value],
) -> Result<(PatchInput, Vec<GraphicsPropertyUpdate>), BuiltinError> {
    if structure.numel() != 1 {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "input 1 to `patch` must be a scalar structure",
        ));
    }
    if !property_arguments.len().is_multiple_of(2) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            "`patch` requires complete property name/value pairs after its structure input",
        ));
    }
    let field = |target: &str| {
        structure
            .field_names()
            .iter()
            .position(|field| field.as_str().eq_ignore_ascii_case(target))
            .and_then(|field| structure.value_at(field, 0))
    };
    let faces = field("faces")
        .map(|value| PreparedNumeric::from_value(name, 1, value))
        .transpose()?;
    let vertices = field("vertices")
        .map(|value| PreparedNumeric::from_value(name, 1, value))
        .transpose()?;
    let cdata = field("facevertexcdata")
        .map(|value| PreparedNumeric::from_value(name, 1, value))
        .transpose()?;
    let mut properties = Vec::with_capacity(property_arguments.len() / 2);
    for (pair, values) in property_arguments.chunks_exact(2).enumerate() {
        let name_position = pair * 2 + 2;
        properties.push(parse_property_update(
            name,
            name_position,
            name_position + 1,
            GraphicsClass::PatchSeries,
            &values[0],
            &values[1],
        )?);
    }
    finish_prepared_patch(name, faces, vertices, cdata, properties)
}

fn finish_prepared_patch(
    name: &str,
    faces: Option<PreparedNumeric>,
    vertices: Option<PreparedNumeric>,
    cdata: Option<PreparedNumeric>,
    properties: Vec<GraphicsPropertyUpdate>,
) -> Result<(PatchInput, Vec<GraphicsPropertyUpdate>), BuiltinError> {
    let faces = faces.ok_or_else(|| {
        BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            "`patch` requires Faces",
        )
    })?;
    let vertices = vertices.ok_or_else(|| {
        BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            "`patch` requires Vertices",
        )
    })?;
    if !matches!(vertices.columns, 2 | 3) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "`patch` Vertices must be an N-by-2 or N-by-3 matrix",
        ));
    }
    let cdata = cdata.unwrap_or_else(empty_prepared_numeric);
    if cdata.numel() != 0
        && (!cdata.is_vector() || (cdata.numel() != faces.rows && cdata.numel() != vertices.rows))
    {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "`patch` FaceVertexCData must contain one scalar per face or vertex",
        ));
    }
    let face_rows = u64::try_from(faces.rows).map_err(|_| plot_size_error(name))?;
    let face_columns = u64::try_from(faces.columns).map_err(|_| plot_size_error(name))?;
    let vertex_rows = u64::try_from(vertices.rows).map_err(|_| plot_size_error(name))?;
    let vertex_columns = u64::try_from(vertices.columns).map_err(|_| plot_size_error(name))?;
    let cdata_rows = u64::try_from(cdata.numel()).map_err(|_| plot_size_error(name))?;
    let has_cdata = !cdata.values.is_empty();
    Ok((
        PatchInput {
            faces: faces.values,
            vertices: vertices.values,
            face_vertex_cdata: cdata.values,
            face_rows,
            face_columns,
            vertex_rows,
            vertex_columns,
            cdata_rows,
            cdata_columns: u64::from(has_cdata),
            face_color: if has_cdata {
                SurfaceColor::Flat
            } else {
                SurfaceColor::Rgba([0.0, 0.0, 0.0, 1.0])
            },
            edge_color: SurfaceColor::Rgba([0.0, 0.0, 0.0, 1.0]),
        },
        properties,
    ))
}

fn empty_prepared_numeric() -> PreparedNumeric {
    PreparedNumeric {
        rows: 0,
        columns: 0,
        values: NumericData::from_f64(Arc::from([])),
    }
}

fn patch_face_values(
    name: &str,
    faces_per_column: usize,
    vertices_per_face: usize,
) -> Result<Vec<f64>, BuiltinError> {
    let count = faces_per_column
        .checked_mul(vertices_per_face)
        .ok_or_else(|| plot_size_error(name))?;
    let mut values = Vec::with_capacity(count);
    for vertex in 0..vertices_per_face {
        for face in 0..faces_per_column {
            let index = face
                .checked_mul(vertices_per_face)
                .and_then(|value| value.checked_add(vertex + 1))
                .and_then(|value| u64::try_from(value).ok())
                .filter(|value| *value <= 9_007_199_254_740_992)
                .ok_or_else(|| plot_size_error(name))?;
            #[allow(clippy::cast_precision_loss)]
            values.push(index as f64);
        }
    }
    Ok(values)
}

fn prepare_tri_patch(
    name: &str,
    arguments: &[Value],
    wireframe: bool,
) -> Result<(PatchInput, Vec<GraphicsPropertyUpdate>), BuiltinError> {
    if arguments.len() < 4 {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            format!("`{name}` requires TRI, X, Y, and Z"),
        ));
    }
    let has_explicit_color = arguments.get(4).is_some_and(color_is_numeric);
    let data_count = if has_explicit_color { 5 } else { 4 };
    if !(arguments.len() - data_count).is_multiple_of(2) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            format!("`{name}` requires complete property name/value pairs after its data"),
        ));
    }
    let faces = PreparedNumeric::from_value(name, 1, &arguments[0])?;
    let x = PreparedNumeric::from_value(name, 2, &arguments[1])?;
    let y = PreparedNumeric::from_value(name, 3, &arguments[2])?;
    let z = PreparedNumeric::from_value(name, 4, &arguments[3])?;
    if faces.rows == 0 || faces.columns != 3 {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` TRI must be a non-empty M-by-3 matrix"),
        ));
    }
    if !x.is_vector()
        || !y.is_vector()
        || !z.is_vector()
        || x.numel() < 3
        || y.numel() != x.numel()
        || z.numel() != x.numel()
    {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` X, Y, and Z must be equal-length real vectors"),
        ));
    }
    let cdata = if has_explicit_color {
        let color = PreparedNumeric::from_value(name, 5, &arguments[4])?;
        if !color.is_vector() || color.numel() != x.numel() {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                format!("`{name}` C must contain one scalar per vertex"),
            ));
        }
        color.vector()
    } else {
        z.vector()
    };
    let mut vertices = Vec::with_capacity(x.numel() * 3);
    vertices.extend(x.vector_f64());
    vertices.extend(y.vector_f64());
    vertices.extend(z.vector_f64());
    let mut properties = Vec::new();
    for (pair, values) in arguments[data_count..].chunks_exact(2).enumerate() {
        let name_position = data_count + pair * 2 + 1;
        properties.push(parse_property_update(
            name,
            name_position,
            name_position + 1,
            GraphicsClass::PatchSeries,
            &values[0],
            &values[1],
        )?);
    }
    Ok((
        PatchInput {
            faces: faces.values,
            vertices: NumericData::from_f64(Arc::from(vertices)),
            face_vertex_cdata: cdata,
            face_rows: u64::try_from(faces.rows).map_err(|_| plot_size_error(name))?,
            face_columns: 3,
            vertex_rows: u64::try_from(x.numel()).map_err(|_| plot_size_error(name))?,
            vertex_columns: 3,
            cdata_rows: u64::try_from(x.numel()).map_err(|_| plot_size_error(name))?,
            cdata_columns: 1,
            face_color: if wireframe {
                SurfaceColor::Rgba([1.0, 1.0, 1.0, 1.0])
            } else {
                SurfaceColor::Flat
            },
            edge_color: if wireframe {
                SurfaceColor::Flat
            } else {
                SurfaceColor::Rgba([0.0, 0.0, 0.0, 1.0])
            },
        },
        properties,
    ))
}

fn patch_coordinate_shape(
    name: &str,
    x: &PreparedNumeric,
    y: &PreparedNumeric,
    z: Option<&PreparedNumeric>,
) -> Result<(usize, usize), BuiltinError> {
    let vectors = x.is_vector()
        && y.is_vector()
        && z.is_none_or(PreparedNumeric::is_vector)
        && x.numel() == y.numel()
        && z.is_none_or(|z| z.numel() == x.numel());
    if vectors {
        if x.numel() < 3 {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                format!("`{name}` requires at least three vertices per face"),
            ));
        }
        return Ok((1, x.numel()));
    }
    let matching = x.rows == y.rows
        && x.columns == y.columns
        && z.is_none_or(|z| z.rows == x.rows && z.columns == x.columns);
    if !matching || x.rows < 3 || x.columns == 0 {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` requires equal-sized coordinate vectors or matrices"),
        ));
    }
    Ok((x.columns, x.rows))
}

fn color_is_numeric(value: &Value) -> bool {
    !matches!(value, Value::String(_) | Value::Array(ArrayData::Char(_)))
}

fn require_surface_matrix(name: &str, value: &PreparedNumeric) -> Result<(), BuiltinError> {
    if value.rows >= 2 && value.columns >= 2 {
        Ok(())
    } else {
        Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` requires a real Z matrix with at least two rows and columns"),
        ))
    }
}

#[allow(clippy::cast_precision_loss)]
fn default_surface_coordinate(length: usize) -> NumericData {
    NumericData::from_f64(Arc::from(
        (1..=length).map(|value| value as f64).collect::<Vec<_>>(),
    ))
}

fn prepare_plot_y(y: &PreparedNumeric) -> Vec<LineInput> {
    if y.numel() == 0 {
        return Vec::new();
    }
    if y.is_vector() {
        return vec![LineInput {
            x: y.default_x(y.numel()),
            y: y.vector(),
            z: None,
        }];
    }
    let mut lines = Vec::with_capacity(y.columns);
    for column in 0..y.columns {
        lines.push(LineInput {
            x: y.default_x(y.rows),
            y: y.column(column),
            z: None,
        });
    }
    lines
}

fn prepare_plot_xy(
    x: &PreparedNumeric,
    y: &PreparedNumeric,
) -> Result<Vec<LineInput>, BuiltinError> {
    if x.numel() == 0 && y.numel() == 0 {
        return Ok(Vec::new());
    }
    if x.is_vector() && y.is_vector() {
        if x.numel() != y.numel() {
            return Err(plot_dimensions_error());
        }
        return Ok(vec![LineInput {
            x: x.vector(),
            y: y.vector(),
            z: None,
        }]);
    }
    if x.is_vector() && !y.is_vector() && x.numel() == y.rows {
        let mut lines = Vec::with_capacity(y.columns);
        for column in 0..y.columns {
            lines.push(LineInput {
                x: x.vector(),
                y: y.column(column),
                z: None,
            });
        }
        return Ok(lines);
    }
    if !x.is_vector() && !y.is_vector() && x.rows == y.rows && x.columns == y.columns {
        let mut lines = Vec::with_capacity(y.columns);
        for column in 0..y.columns {
            lines.push(LineInput {
                x: x.column(column),
                y: y.column(column),
                z: None,
            });
        }
        return Ok(lines);
    }
    Err(plot_dimensions_error())
}

#[derive(Clone)]
struct PreparedNumeric {
    rows: usize,
    columns: usize,
    values: NumericData,
}

impl PreparedNumeric {
    fn from_value(name: &str, position: usize, value: &Value) -> Result<Self, BuiltinError> {
        match value {
            Value::Double(value) => Ok(Self {
                rows: 1,
                columns: 1,
                values: NumericData::from_f64(Arc::from([*value])),
            }),
            Value::Array(ArrayData::F64(array)) => Self::from_shape(
                name,
                position,
                array.shape().dimensions(),
                NumericData::from_f64(Arc::<[f64]>::from(array.as_slice())),
            ),
            Value::Array(ArrayData::F32(array)) => Self::from_shape(
                name,
                position,
                array.shape().dimensions(),
                NumericData::from_f32(Arc::<[f32]>::from(array.as_slice())),
            ),
            _ => Err(type_error(
                name,
                position,
                "real double or single scalar, vector, or matrix",
                value,
            )),
        }
    }

    fn from_shape(
        name: &str,
        position: usize,
        dimensions: &[u64],
        values: NumericData,
    ) -> Result<Self, BuiltinError> {
        if dimensions.len() != 2 {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                format!("input {position} to `{name}` must be two-dimensional in Plot Engine v1"),
            ));
        }
        let rows = usize::try_from(dimensions[0]).map_err(|_| plot_size_error(name))?;
        let columns = usize::try_from(dimensions[1]).map_err(|_| plot_size_error(name))?;
        Ok(Self {
            rows,
            columns,
            values,
        })
    }

    fn numel(&self) -> usize {
        self.values.len()
    }

    fn is_vector(&self) -> bool {
        self.rows == 1 || self.columns == 1
    }

    fn vector(&self) -> NumericData {
        self.values.clone()
    }

    fn column(&self, column: usize) -> NumericData {
        let start = column * self.rows;
        let end = start + self.rows;
        match &self.values {
            NumericData::F32(values) => {
                NumericData::from_f32(Arc::<[f32]>::from(&values[start..end]))
            }
            NumericData::F64(values) => {
                NumericData::from_f64(Arc::<[f64]>::from(&values[start..end]))
            }
        }
    }

    fn column_f64(&self, column: usize) -> Vec<f64> {
        let mut values = Vec::with_capacity(self.rows);
        self.column(column).visit_f64(|value| values.push(value));
        values
    }

    #[allow(clippy::cast_precision_loss)]
    fn default_x(&self, length: usize) -> NumericData {
        match self.values {
            NumericData::F32(_) => NumericData::from_f32(Arc::from(
                (1..=length).map(|value| value as f32).collect::<Vec<_>>(),
            )),
            NumericData::F64(_) => NumericData::from_f64(Arc::from(
                (1..=length).map(|value| value as f64).collect::<Vec<_>>(),
            )),
        }
    }

    fn vector_f64(&self) -> Vec<f64> {
        let mut values = Vec::with_capacity(self.numel());
        self.values.visit_f64(|value| values.push(value));
        values
    }
}

fn property_values_to_output(values: Vec<GraphicsPropertyValue>, scalar: bool) -> BuiltinResult {
    if scalar {
        let mut values = values.into_iter();
        let value = values
            .next()
            .ok_or_else(|| graphics_state_error("get", "one property value"))?;
        if values.next().is_some() {
            return Err(graphics_state_error("get", "one property value"));
        }
        return Ok(vec![property_value_to_language(&value)?]);
    }
    let values = values
        .iter()
        .map(property_value_to_language)
        .collect::<Result<Vec<_>, _>>()?;
    let length = u64::try_from(values.len()).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Other,
            "`get` property result length exceeds language limits",
        )
    })?;
    let shape = Shape::new([length, 1])
        .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Other, error.to_string()))?;
    let cells = CellArray::from_values(shape, values)
        .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Other, error.to_string()))?;
    Ok(vec![Value::Cell(cells)])
}

fn property_value_to_language(value: &GraphicsPropertyValue) -> Result<Value, BuiltinError> {
    match value {
        GraphicsPropertyValue::Scalar(value) => Ok(Value::Double(*value)),
        GraphicsPropertyValue::Color(color) => rgb_row(*color),
        GraphicsPropertyValue::LineStyle(style) => char_row(match style {
            LineStyle::None => "none",
            LineStyle::Solid => "-",
            LineStyle::Dash => "--",
            LineStyle::Dot => ":",
            LineStyle::DashDot => "-.",
        }),
        GraphicsPropertyValue::Marker(marker) => char_row(match marker {
            Marker::None => "none",
            Marker::Point => ".",
            Marker::Circle => "o",
            Marker::Plus => "+",
            Marker::Star => "*",
            Marker::Cross => "x",
            Marker::Square => "square",
            Marker::Diamond => "diamond",
            Marker::TriangleUp => "^",
            Marker::TriangleDown => "v",
            Marker::TriangleRight => ">",
            Marker::TriangleLeft => "<",
            Marker::HorizontalLine => "_",
            Marker::VerticalLine => "|",
        }),
        GraphicsPropertyValue::Visible(value) => char_row(if *value { "on" } else { "off" }),
        GraphicsPropertyValue::Handle(handle) => Ok(Value::Graphics(*handle)),
        GraphicsPropertyValue::Handles(handles) => handles_to_language(handles),
        GraphicsPropertyValue::NumericMatrix { shape, values } => {
            let shape = Shape::new(*shape).map_err(|error| {
                BuiltinError::new(BuiltinErrorCategory::Other, error.to_string())
            })?;
            DenseArray::from_vec(shape, values.clone())
                .map(ArrayData::F64)
                .map(Value::Array)
                .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Other, error.to_string()))
        }
        GraphicsPropertyValue::Text(code_units) => char_units(code_units),
        GraphicsPropertyValue::TextList(items) => text_list(items),
        GraphicsPropertyValue::TextColumn(items) => text_column(items),
        GraphicsPropertyValue::MarkerIndices {
            values,
            value_class,
        } => marker_indices_row(values, *value_class),
        GraphicsPropertyValue::LimitMode(mode) => char_row(match mode {
            LimitMode::Auto => "auto",
            LimitMode::Manual => "manual",
        }),
        GraphicsPropertyValue::TickMode(mode) => char_row(match mode {
            TickMode::Auto => "auto",
            TickMode::Manual => "manual",
        }),
        GraphicsPropertyValue::Interpreter(value) => char_row(match value {
            Interpreter::Tex => "tex",
            Interpreter::Latex => "latex",
            Interpreter::None => "none",
        }),
        GraphicsPropertyValue::LegendLocation(location) => char_row(match location {
            LegendLocation::Best => "best",
            LegendLocation::Northeast => "northeast",
            LegendLocation::Northwest => "northwest",
            LegendLocation::SouthOutside => "southoutside",
        }),
        GraphicsPropertyValue::SurfaceColor(color) => match color {
            SurfaceColor::Flat => char_row("flat"),
            SurfaceColor::Interp => char_row("interp"),
            SurfaceColor::None => char_row("none"),
            SurfaceColor::Rgba(color) => rgb_row(*color),
        },
        GraphicsPropertyValue::CDataMapping(mapping) => char_row(match mapping {
            CDataMapping::Scaled => "scaled",
            CDataMapping::Direct => "direct",
        }),
        GraphicsPropertyValue::Projection(projection) => char_row(match projection {
            ProjectionMode::Orthographic => "orthographic",
            ProjectionMode::Perspective => "perspective",
        }),
    }
}

fn surface_color_value(
    name: &str,
    position: usize,
    value: &Value,
) -> Result<SurfaceColor, BuiltinError> {
    if let Ok(text) = lowercase_ascii_text(name, position, value) {
        return match text.as_str() {
            "flat" => Ok(SurfaceColor::Flat),
            "interp" => Ok(SurfaceColor::Interp),
            "none" => Ok(SurfaceColor::None),
            _ => named_color(&text).map(SurfaceColor::Rgba).ok_or_else(|| {
                BuiltinError::new(
                    BuiltinErrorCategory::Domain,
                    format!("input {position} to `{name}` is not a supported Surface color"),
                )
            }),
        };
    }
    rgb_color(name, position, "Surface color", value).map(SurfaceColor::Rgba)
}

fn chart_color_value(
    name: &str,
    position: usize,
    property: &str,
    value: &Value,
    allow_auto: bool,
) -> Result<ChartColor, BuiltinError> {
    if let Ok(text) = lowercase_ascii_text(name, position, value) {
        return match text.as_str() {
            "auto" if allow_auto => Ok(ChartColor::Auto),
            "none" => Ok(ChartColor::None),
            _ => named_color(&text).map(ChartColor::Rgba).ok_or_else(|| {
                let expected = if allow_auto {
                    format!("auto, none, or RGB {property}")
                } else {
                    format!("none or RGB {property}")
                };
                type_error(name, position, &expected, value)
            }),
        };
    }
    rgb_color(name, position, property, value).map(ChartColor::Rgba)
}

const fn is_line_chart_class(class: GraphicsClass) -> bool {
    matches!(
        class,
        GraphicsClass::Stair | GraphicsClass::Stem | GraphicsClass::ErrorBar
    )
}

const fn is_patch_chart_class(class: GraphicsClass) -> bool {
    matches!(
        class,
        GraphicsClass::Area | GraphicsClass::Bar | GraphicsClass::Histogram
    )
}

fn handles_to_language(handles: &[GraphicsHandle]) -> Result<Value, BuiltinError> {
    let homogeneous = handles
        .first()
        .is_none_or(|first| handles.iter().all(|handle| handle.class() == first.class()));
    if homogeneous {
        let class = handles
            .first()
            .map_or(GraphicsClass::LineSeries, |handle| handle.class());
        let array = GraphicsHandleArray::column(class, handles.to_vec()).map_err(|error| {
            BuiltinError::new(BuiltinErrorCategory::Graphics, error.to_string())
        })?;
        return Ok(Value::GraphicsArray(array));
    }
    let shape = Shape::new([handles.len() as u64, 1])
        .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Other, error.to_string()))?;
    CellArray::from_values(
        shape,
        handles.iter().copied().map(Value::Graphics).collect(),
    )
    .map(Value::Cell)
    .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Other, error.to_string()))
}

fn char_units(code_units: &[u16]) -> Result<Value, BuiltinError> {
    let shape = Shape::new([1, code_units.len() as u64])
        .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Other, error.to_string()))?;
    DenseArray::from_vec(
        shape,
        code_units.iter().copied().map(CharCodeUnit::new).collect(),
    )
    .map(ArrayData::Char)
    .map(Value::Array)
    .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Other, error.to_string()))
}

fn text_list(items: &[Vec<u16>]) -> Result<Value, BuiltinError> {
    let shape = Shape::new([1, items.len() as u64])
        .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Other, error.to_string()))?;
    let values = items
        .iter()
        .map(|item| char_units(item))
        .collect::<Result<Vec<_>, _>>()?;
    CellArray::from_values(shape, values)
        .map(Value::Cell)
        .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Other, error.to_string()))
}

fn text_column(items: &[Vec<u16>]) -> Result<Value, BuiltinError> {
    let shape = Shape::new([items.len() as u64, 1])
        .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Other, error.to_string()))?;
    let values = items
        .iter()
        .map(|item| char_units(item))
        .collect::<Result<Vec<_>, _>>()?;
    CellArray::from_values(shape, values)
        .map(Value::Cell)
        .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Other, error.to_string()))
}

#[allow(clippy::cast_precision_loss)]
fn marker_indices_row(
    values: &[u64],
    value_class: MarkerIndicesClass,
) -> Result<Value, BuiltinError> {
    let dimensions = if values.is_empty() {
        [0, 0]
    } else {
        [1, values.len() as u64]
    };
    let shape = Shape::new(dimensions)
        .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Other, error.to_string()))?;
    match value_class {
        MarkerIndicesClass::UInt64 => DenseArray::from_vec(shape, values.to_vec())
            .map(IntegerArrayData::from_typed)
            .map(ArrayData::Integer)
            .map(Value::Array)
            .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Other, error.to_string())),
        MarkerIndicesClass::Double => {
            DenseArray::from_vec(shape, values.iter().map(|value| *value as f64).collect())
                .map(ArrayData::F64)
                .map(Value::Array)
                .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Other, error.to_string()))
        }
        MarkerIndicesClass::Single => {
            DenseArray::from_vec(shape, values.iter().map(|value| *value as f32).collect())
                .map(ArrayData::F32)
                .map(Value::Array)
                .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Other, error.to_string()))
        }
    }
}

fn rgb_row(color: [f32; 4]) -> Result<Value, BuiltinError> {
    let shape = Shape::new([1, 3])
        .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Other, error.to_string()))?;
    let values = color[..3]
        .iter()
        .copied()
        .map(f64::from)
        .map(|value| (value * 1_000_000.0).round() / 1_000_000.0)
        .collect();
    let array = DenseArray::from_vec(shape, values)
        .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Other, error.to_string()))?;
    Ok(Value::Array(ArrayData::F64(array)))
}

fn char_row(value: &str) -> Result<Value, BuiltinError> {
    let code_units = value
        .encode_utf16()
        .map(CharCodeUnit::new)
        .collect::<Vec<_>>();
    let length = u64::try_from(code_units.len()).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Other,
            "graphics property text result exceeds language limits",
        )
    })?;
    let shape = Shape::new([1, length])
        .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Other, error.to_string()))?;
    let array = DenseArray::from_vec(shape, code_units)
        .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Other, error.to_string()))?;
    Ok(Value::Array(ArrayData::Char(array)))
}

fn ascii_text(name: &str, position: usize, value: &Value) -> Result<String, BuiltinError> {
    let code_units = text_scalar(name, position, value)?;
    if code_units.iter().any(|unit| *unit > 0x7f) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("input {position} to `{name}` must use ASCII property/style text"),
        ));
    }
    Ok(code_units
        .into_iter()
        .map(|unit| char::from(u8::try_from(unit).expect("ASCII was checked")))
        .collect())
}

fn lowercase_ascii_text(
    name: &str,
    position: usize,
    value: &Value,
) -> Result<String, BuiltinError> {
    Ok(ascii_text(name, position, value)?.to_ascii_lowercase())
}

fn line_style_value(name: &str, position: usize, value: &Value) -> Result<LineStyle, BuiltinError> {
    match lowercase_ascii_text(name, position, value)?.as_str() {
        "none" => Ok(LineStyle::None),
        "-" | "solid" => Ok(LineStyle::Solid),
        "--" | "dashed" => Ok(LineStyle::Dash),
        ":" | "dotted" => Ok(LineStyle::Dot),
        "-." | "dash-dot" => Ok(LineStyle::DashDot),
        _ => Err(BuiltinError::new(
            BuiltinErrorCategory::Type,
            format!(
                "input {position} to `{name}` must be `none`, `-`, `--`, `:`, `-.`, or its supported long name"
            ),
        )),
    }
}

fn marker_value(name: &str, position: usize, value: &Value) -> Result<Marker, BuiltinError> {
    match lowercase_ascii_text(name, position, value)?.as_str() {
        "none" => Ok(Marker::None),
        "." | "point" => Ok(Marker::Point),
        "o" | "circle" => Ok(Marker::Circle),
        "+" | "plus" => Ok(Marker::Plus),
        "*" | "star" => Ok(Marker::Star),
        "x" | "cross" => Ok(Marker::Cross),
        "s" | "square" => Ok(Marker::Square),
        "d" | "diamond" => Ok(Marker::Diamond),
        "^" | "triangleup" => Ok(Marker::TriangleUp),
        "v" | "triangledown" => Ok(Marker::TriangleDown),
        ">" | "triangleright" => Ok(Marker::TriangleRight),
        "<" | "triangleleft" => Ok(Marker::TriangleLeft),
        "_" | "horizontal" => Ok(Marker::HorizontalLine),
        "|" | "vertical" => Ok(Marker::VerticalLine),
        _ => Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("input {position} to `{name}` is not a supported MATLAB 2D marker"),
        )),
    }
}

fn visible_value(name: &str, position: usize, value: &Value) -> Result<bool, BuiltinError> {
    match lowercase_ascii_text(name, position, value)?.as_str() {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("input {position} to `{name}` must be exactly `on` or `off`"),
        )),
    }
}

fn real_scalar(name: &str, position: usize, value: &Value) -> Result<f64, BuiltinError> {
    match value {
        Value::Double(value) => Ok(*value),
        Value::Array(ArrayData::F64(array)) if array.numel() == 1 => Ok(array.as_slice()[0]),
        Value::Array(ArrayData::F32(array)) if array.numel() == 1 => {
            Ok(f64::from(array.as_slice()[0]))
        }
        _ => Err(type_error(
            name,
            position,
            "real double or single scalar",
            value,
        )),
    }
}

#[allow(clippy::cast_possible_truncation)]
fn positive_f32_scalar(
    name: &str,
    position: usize,
    property: &str,
    value: &Value,
) -> Result<f32, BuiltinError> {
    let value = real_scalar(name, position, value)?;
    if !value.is_finite() || value <= 0.0 || value > f64::from(f32::MAX) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Type,
            format!("`{property}` must be a positive finite scalar"),
        ));
    }
    Ok(value as f32)
}

#[allow(clippy::cast_possible_truncation)]
fn finite_f32_scalar(
    name: &str,
    position: usize,
    property: &str,
    value: &Value,
) -> Result<f32, BuiltinError> {
    let value = real_scalar(name, position, value)?;
    if !value.is_finite() || value.abs() > f64::from(f32::MAX) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{property}` must be a finite scalar"),
        ));
    }
    Ok(value as f32)
}

fn font_weight(name: &str, position: usize, value: &Value) -> Result<u16, BuiltinError> {
    if let Ok(text) = lowercase_ascii_text(name, position, value) {
        return match text.as_str() {
            "normal" => Ok(400),
            "bold" => Ok(700),
            _ => Err(type_error(
                name,
                position,
                "normal, bold, or a weight in 1..=1000",
                value,
            )),
        };
    }
    let weight = real_scalar(name, position, value)?;
    if !weight.is_finite() || weight.fract() != 0.0 || !(1.0..=1000.0).contains(&weight) {
        return Err(type_error(
            name,
            position,
            "normal, bold, or a weight in 1..=1000",
            value,
        ));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(weight as u16)
}

#[allow(clippy::cast_possible_truncation)]
fn nonnegative_f32_scalar(
    name: &str,
    position: usize,
    property: &str,
    value: &Value,
) -> Result<f32, BuiltinError> {
    let value = real_scalar(name, position, value)?;
    if !value.is_finite() || value < 0.0 || value > f64::from(f32::MAX) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{property}` must be a nonnegative finite scalar"),
        ));
    }
    Ok(value as f32)
}

#[allow(clippy::cast_possible_truncation)]
fn unit_f32_scalar(
    name: &str,
    position: usize,
    property: &str,
    value: &Value,
) -> Result<f32, BuiltinError> {
    let value = real_scalar(name, position, value)?;
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{property}` must be a finite scalar in [0,1]"),
        ));
    }
    Ok(value as f32)
}

#[allow(clippy::cast_possible_truncation)]
fn rgb_color(
    name: &str,
    position: usize,
    property: &str,
    value: &Value,
) -> Result<[f32; 4], BuiltinError> {
    if matches!(value, Value::String(_) | Value::Array(ArrayData::Char(_))) {
        let color_name = lowercase_ascii_text(name, position, value)?;
        return named_color(&color_name).ok_or_else(|| {
            BuiltinError::new(
                BuiltinErrorCategory::Domain,
                format!("`{property}` does not recognize color `{color_name}`"),
            )
        });
    }
    let numeric = PreparedNumeric::from_value(name, position, value)?;
    if !numeric.is_vector() || numeric.numel() != 3 {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{property}` must be a three-element RGB vector in [0,1]"),
        ));
    }
    let values = numeric.vector_f64();
    if values
        .iter()
        .any(|component| !component.is_finite() || !(0.0..=1.0).contains(component))
    {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{property}` must be a finite RGB vector in [0,1]"),
        ));
    }
    Ok([values[0] as f32, values[1] as f32, values[2] as f32, 1.0])
}

fn axes_background_color(
    name: &str,
    position: usize,
    value: &Value,
) -> Result<Option<[f32; 4]>, BuiltinError> {
    if matches!(value, Value::String(_) | Value::Array(ArrayData::Char(_)))
        && lowercase_ascii_text(name, position, value)? == "none"
    {
        return Ok(None);
    }
    rgb_color(name, position, "Color", value).map(Some)
}

fn named_color(value: &str) -> Option<[f32; 4]> {
    match value {
        "r" | "red" => Some([1.0, 0.0, 0.0, 1.0]),
        "g" | "green" => Some([0.0, 1.0, 0.0, 1.0]),
        "b" | "blue" => Some([0.0, 0.0, 1.0, 1.0]),
        "c" | "cyan" => Some([0.0, 1.0, 1.0, 1.0]),
        "m" | "magenta" => Some([1.0, 0.0, 1.0, 1.0]),
        "y" | "yellow" => Some([1.0, 1.0, 0.0, 1.0]),
        "k" | "black" => Some([0.0, 0.0, 0.0, 1.0]),
        "w" | "white" => Some([1.0, 1.0, 1.0, 1.0]),
        _ => None,
    }
}

fn short_color(value: u8) -> Option<[f32; 4]> {
    match value {
        b'r' => named_color("r"),
        b'g' => named_color("g"),
        b'b' => named_color("b"),
        b'c' => named_color("c"),
        b'm' => named_color("m"),
        b'y' => named_color("y"),
        b'k' => named_color("k"),
        b'w' => named_color("w"),
        _ => None,
    }
}

fn marker_symbol(value: u8) -> Option<Marker> {
    match value {
        b'.' => Some(Marker::Point),
        b'o' => Some(Marker::Circle),
        b'+' => Some(Marker::Plus),
        b'*' => Some(Marker::Star),
        b'x' => Some(Marker::Cross),
        b's' => Some(Marker::Square),
        b'd' => Some(Marker::Diamond),
        b'^' => Some(Marker::TriangleUp),
        b'v' => Some(Marker::TriangleDown),
        b'>' => Some(Marker::TriangleRight),
        b'<' => Some(Marker::TriangleLeft),
        b'_' => Some(Marker::HorizontalLine),
        b'|' => Some(Marker::VerticalLine),
        _ => None,
    }
}

fn optional_axes<'a>(
    name: &str,
    arguments: &'a [Value],
) -> Result<(Option<GraphicsHandle>, &'a [Value]), BuiltinError> {
    let Some(first) = arguments.first() else {
        return Ok((None, arguments));
    };
    if matches!(first, Value::Graphics(_) | Value::GraphicsArray(_)) {
        let axes = graphics_axes_scalar(name, 1, first)?;
        Ok((Some(axes), &arguments[1..]))
    } else {
        Ok((None, arguments))
    }
}

fn optional_polar_axes<'a>(
    name: &str,
    arguments: &'a [Value],
) -> Result<(Option<GraphicsHandle>, &'a [Value]), BuiltinError> {
    let Some(first) = arguments.first() else {
        return Ok((None, arguments));
    };
    if matches!(first, Value::Graphics(_) | Value::GraphicsArray(_)) {
        let axes = graphics_scalar(name, 1, first, GraphicsClass::PolarAxes)?;
        Ok((Some(axes), &arguments[1..]))
    } else {
        Ok((None, arguments))
    }
}

fn required_polar_axes(
    name: &str,
    axes: Option<GraphicsHandle>,
    context: &mut BuiltinContext<'_>,
) -> Result<GraphicsHandle, BuiltinError> {
    if let Some(axes) = axes {
        return Ok(axes);
    }
    let GraphicsResponse::Handle(axes) = context.graphics(GraphicsRequest::CurrentAxes)? else {
        return Err(graphics_state_error(name, "current PolarAxes handle"));
    };
    if axes.class() != GraphicsClass::PolarAxes {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Graphics,
            format!("`{name}` requires a PolarAxes target"),
        ));
    }
    Ok(axes)
}

fn legend_location(
    name: &str,
    position: usize,
    value: &Value,
) -> Result<LegendLocation, BuiltinError> {
    match lowercase_ascii_text(name, position, value)?.as_str() {
        "best" => Ok(LegendLocation::Best),
        "northeast" => Ok(LegendLocation::Northeast),
        "northwest" => Ok(LegendLocation::Northwest),
        "southoutside" => Ok(LegendLocation::SouthOutside),
        _ => Err(type_error(
            name,
            position,
            "supported Legend Location",
            value,
        )),
    }
}

fn legend_orientation(
    name: &str,
    position: usize,
    value: &Value,
) -> Result<LegendOrientation, BuiltinError> {
    match lowercase_ascii_text(name, position, value)?.as_str() {
        "vertical" => Ok(LegendOrientation::Vertical),
        "horizontal" => Ok(LegendOrientation::Horizontal),
        _ => Err(type_error(name, position, "vertical or horizontal", value)),
    }
}

fn tick_direction(
    name: &str,
    position: usize,
    value: &Value,
) -> Result<TickDirection, BuiltinError> {
    match lowercase_ascii_text(name, position, value)?.as_str() {
        "in" => Ok(TickDirection::In),
        "out" => Ok(TickDirection::Out),
        "both" => Ok(TickDirection::Both),
        _ => Err(type_error(name, position, "in, out, or both", value)),
    }
}

fn theta_axis_units(
    name: &str,
    position: usize,
    value: &Value,
) -> Result<ThetaAxisUnits, BuiltinError> {
    match lowercase_ascii_text(name, position, value)?.as_str() {
        "degrees" => Ok(ThetaAxisUnits::Degrees),
        "radians" => Ok(ThetaAxisUnits::Radians),
        _ => Err(type_error(name, position, "degrees or radians", value)),
    }
}

fn theta_direction(
    name: &str,
    position: usize,
    value: &Value,
) -> Result<ThetaDirection, BuiltinError> {
    match lowercase_ascii_text(name, position, value)?.as_str() {
        "counterclockwise" => Ok(ThetaDirection::Counterclockwise),
        "clockwise" => Ok(ThetaDirection::Clockwise),
        _ => Err(type_error(
            name,
            position,
            "counterclockwise or clockwise",
            value,
        )),
    }
}

fn theta_zero_location(
    name: &str,
    position: usize,
    value: &Value,
) -> Result<ThetaZeroLocation, BuiltinError> {
    match lowercase_ascii_text(name, position, value)?.as_str() {
        "right" => Ok(ThetaZeroLocation::Right),
        "top" => Ok(ThetaZeroLocation::Top),
        "left" => Ok(ThetaZeroLocation::Left),
        "bottom" => Ok(ThetaZeroLocation::Bottom),
        _ => Err(type_error(
            name,
            position,
            "right, top, left, or bottom",
            value,
        )),
    }
}

fn is_legend_property_name(name: &str) -> bool {
    matches!(
        name,
        "interpreter"
            | "location"
            | "fontsize"
            | "fontname"
            | "fontweight"
            | "orientation"
            | "numcolumns"
            | "box"
            | "visible"
    )
}

fn interpreter(name: &str, position: usize, value: &Value) -> Result<Interpreter, BuiltinError> {
    match lowercase_ascii_text(name, position, value)?.as_str() {
        "tex" => Ok(Interpreter::Tex),
        "latex" => Ok(Interpreter::Latex),
        "none" => Ok(Interpreter::None),
        _ => Err(type_error(name, position, "tex, latex, or none", value)),
    }
}

fn tick_mode(name: &str, position: usize, value: &Value) -> Result<TickMode, BuiltinError> {
    match lowercase_ascii_text(name, position, value)?.as_str() {
        "auto" => Ok(TickMode::Auto),
        "manual" => Ok(TickMode::Manual),
        _ => Err(type_error(name, position, "auto or manual", value)),
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn marker_indices(
    name: &str,
    position: usize,
    value: &Value,
) -> Result<(Vec<u64>, MarkerIndicesClass), BuiltinError> {
    match value {
        Value::Double(number) => marker_indices_from_f64(name, position, value, [*number])
            .map(|values| (values, MarkerIndicesClass::Double)),
        Value::Array(ArrayData::F64(array)) if vector_shape(array.shape().dimensions()) => {
            marker_indices_from_f64(name, position, value, array.as_slice().iter().copied())
                .map(|values| (values, MarkerIndicesClass::Double))
        }
        Value::Array(ArrayData::F32(array)) if vector_shape(array.shape().dimensions()) => {
            marker_indices_from_f64(
                name,
                position,
                value,
                array.as_slice().iter().copied().map(f64::from),
            )
            .map(|values| (values, MarkerIndicesClass::Single))
        }
        Value::Array(ArrayData::Integer(array))
            if !array.is_complex() && vector_shape(array.shape().dimensions()) =>
        {
            array
                .elements()
                .map(|element| match element.real_component() {
                    IntegerComponent::Signed(component) => u64::try_from(component).ok(),
                    IntegerComponent::Unsigned(component) => u64::try_from(component).ok(),
                })
                .collect::<Option<Vec<_>>>()
                .filter(|indices| indices.iter().all(|index| *index > 0))
                .map(|values| (values, MarkerIndicesClass::UInt64))
                .ok_or_else(|| {
                    type_error(
                        name,
                        position,
                        "positive real integer vector MarkerIndices",
                        value,
                    )
                })
        }
        _ => Err(type_error(
            name,
            position,
            "positive real integer vector MarkerIndices",
            value,
        )),
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn marker_indices_from_f64(
    name: &str,
    position: usize,
    original: &Value,
    values: impl IntoIterator<Item = f64>,
) -> Result<Vec<u64>, BuiltinError> {
    values
        .into_iter()
        .map(|value| {
            (value.is_finite()
                && value >= 1.0
                && value.fract() == 0.0
                && value < U64_EXCLUSIVE_UPPER_BOUND)
                .then_some(value as u64)
        })
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| {
            type_error(
                name,
                position,
                "positive finite integer vector MarkerIndices",
                original,
            )
        })
}

fn tick_values(name: &str, position: usize, value: &Value) -> Result<Vec<f64>, BuiltinError> {
    let values = real_vector_f64(name, position, value)?;
    if values.iter().any(|tick| tick.is_nan()) || values.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(type_error(
            name,
            position,
            "empty or strictly increasing real tick vector without NaN",
            value,
        ));
    }
    Ok(values)
}

#[allow(clippy::cast_precision_loss)]
fn real_vector_f64(name: &str, position: usize, value: &Value) -> Result<Vec<f64>, BuiltinError> {
    if let Ok(numeric) = PreparedNumeric::from_value(name, position, value)
        && (numeric.numel() == 0 || numeric.is_vector())
    {
        return Ok(numeric.vector_f64());
    }
    if let Value::Array(ArrayData::Integer(array)) = value
        && !array.is_complex()
        && vector_shape(array.shape().dimensions())
    {
        return Ok(array
            .elements()
            .map(|element| match element.real_component() {
                IntegerComponent::Signed(component) => component as f64,
                IntegerComponent::Unsigned(component) => component as f64,
            })
            .collect());
    }
    Err(type_error(name, position, "real numeric vector", value))
}

fn vector_shape(dimensions: &[u64]) -> bool {
    dimensions.len() == 2
        && (dimensions[0] == 0 || dimensions[1] == 0 || dimensions[0] == 1 || dimensions[1] == 1)
}

fn tick_labels(name: &str, position: usize, value: &Value) -> Result<Vec<Vec<u16>>, BuiltinError> {
    if let Value::Cell(labels) = value {
        if !vector_shape(labels.shape().dimensions()) {
            return Err(type_error(name, position, "text cell vector", value));
        }
        return labels
            .values()
            .iter()
            .enumerate()
            .map(|(index, label)| text_scalar(name, position + index, label))
            .collect();
    }
    text_scalar(name, position, value).map(|label| vec![label])
}

fn limit_mode(name: &str, position: usize, value: &Value) -> Result<LimitMode, BuiltinError> {
    match lowercase_ascii_text(name, position, value)?.as_str() {
        "auto" => Ok(LimitMode::Auto),
        "manual" => Ok(LimitMode::Manual),
        _ => Err(type_error(name, position, "auto or manual", value)),
    }
}

fn limit_pair(
    name: &str,
    position: usize,
    property: &str,
    value: &Value,
) -> Result<[f64; 2], BuiltinError> {
    let values = PreparedNumeric::from_value(name, position, value)?;
    if !values.is_vector() || values.numel() != 2 {
        return Err(type_error(
            name,
            position,
            "two-element real limit vector",
            value,
        ));
    }
    let values = values.vector_f64();
    let limits = [values[0], values[1]];
    if limits.iter().all(|value| value.is_finite()) && limits[0] < limits[1] {
        Ok(limits)
    } else {
        Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{property}` must contain two finite strictly increasing values"),
        ))
    }
}

fn validated_limit_pair_values(
    property: &str,
    lower: f64,
    upper: f64,
) -> Result<[f64; 2], BuiltinError> {
    if lower.is_finite() && upper.is_finite() && lower < upper {
        Ok([lower, upper])
    } else {
        Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{property}` limits must be finite and strictly increasing"),
        ))
    }
}

fn axis_scale(name: &str, position: usize, value: &Value) -> Result<AxisScale, BuiltinError> {
    match lowercase_ascii_text(name, position, value)?.as_str() {
        "linear" => Ok(AxisScale::Linear),
        "log" => Ok(AxisScale::Log),
        _ => Err(type_error(name, position, "linear or log", value)),
    }
}

fn axis_direction(
    name: &str,
    position: usize,
    value: &Value,
) -> Result<AxisDirection, BuiltinError> {
    match lowercase_ascii_text(name, position, value)?.as_str() {
        "normal" => Ok(AxisDirection::Normal),
        "reverse" => Ok(AxisDirection::Reverse),
        _ => Err(type_error(name, position, "normal or reverse", value)),
    }
}

fn limit_method(name: &str, position: usize, value: &Value) -> Result<LimitMethod, BuiltinError> {
    match lowercase_ascii_text(name, position, value)?.as_str() {
        "tickaligned" => Ok(LimitMethod::TickAligned),
        "tight" => Ok(LimitMethod::Tight),
        "padded" => Ok(LimitMethod::Padded),
        _ => Err(type_error(
            name,
            position,
            "tickaligned, tight, or padded",
            value,
        )),
    }
}

fn aspect_ratio(name: &str, position: usize, value: &Value) -> Result<[f64; 3], BuiltinError> {
    let values = PreparedNumeric::from_value(name, position, value)?;
    if !values.is_vector() || values.numel() != 3 {
        return Err(type_error(
            name,
            position,
            "positive three-element real vector",
            value,
        ));
    }
    let values = values.vector_f64();
    let ratio = [values[0], values[1], values[2]];
    if ratio
        .iter()
        .any(|value| !value.is_finite() || *value <= 0.0)
    {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("input {position} to `{name}` must contain positive finite values"),
        ));
    }
    Ok(ratio)
}

fn projection_mode(
    name: &str,
    position: usize,
    value: &Value,
) -> Result<ProjectionMode, BuiltinError> {
    match lowercase_ascii_text(name, position, value)?.as_str() {
        "orthographic" => Ok(ProjectionMode::Orthographic),
        "perspective" => Ok(ProjectionMode::Perspective),
        _ => Err(type_error(
            name,
            position,
            "'orthographic' or 'perspective'",
            value,
        )),
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn positive_u32(name: &str, position: usize, value: &Value) -> Result<u32, BuiltinError> {
    let number = real_scalar(name, position, value)?;
    if number.is_finite() && number >= 1.0 && number.fract() == 0.0 && number <= f64::from(u32::MAX)
    {
        Ok(number as u32)
    } else {
        Err(type_error(name, position, "positive integer scalar", value))
    }
}

fn nexttile_selection(position: usize, value: &Value) -> Result<NextTileSelection, BuiltinError> {
    let numeric = PreparedNumeric::from_value("nexttile", position, value)?;
    if numeric.numel() == 1 {
        return positive_u32("nexttile", position, value).map(NextTileSelection::Tile);
    }
    if !numeric.is_vector() || numeric.numel() != 2 {
        return Err(type_error(
            "nexttile",
            position,
            "positive tile scalar or two-element span",
            value,
        ));
    }
    let values = numeric.vector_f64();
    if values.iter().any(|value| {
        !value.is_finite() || *value < 1.0 || value.fract() != 0.0 || *value > f64::from(u32::MAX)
    }) {
        return Err(type_error(
            "nexttile",
            position,
            "positive integer span",
            value,
        ));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(NextTileSelection::Span {
        rows: values[0] as u32,
        columns: values[1] as u32,
    })
}

fn tiled_spacing_value(position: usize, value: &Value) -> Result<TileSpacing, BuiltinError> {
    match lowercase_ascii_text("tiledlayout", position, value)?.as_str() {
        "loose" => Ok(TileSpacing::Loose),
        "compact" => Ok(TileSpacing::Compact),
        "none" => Ok(TileSpacing::None),
        _ => Err(type_error(
            "tiledlayout",
            position,
            "'loose', 'compact', or 'none'",
            value,
        )),
    }
}

fn tiled_padding_value(position: usize, value: &Value) -> Result<TilePadding, BuiltinError> {
    match lowercase_ascii_text("tiledlayout", position, value)?.as_str() {
        "loose" => Ok(TilePadding::Loose),
        "compact" => Ok(TilePadding::Compact),
        "tight" => Ok(TilePadding::Tight),
        _ => Err(type_error(
            "tiledlayout",
            position,
            "'loose', 'compact', or 'tight'",
            value,
        )),
    }
}

#[allow(clippy::cast_possible_truncation)]
fn color_order(name: &str, position: usize, value: &Value) -> Result<Vec<[f32; 4]>, BuiltinError> {
    let numeric = PreparedNumeric::from_value(name, position, value)?;
    if numeric.rows == 0 || numeric.columns != 3 {
        return Err(type_error(
            name,
            position,
            "nonempty N-by-3 ColorOrder",
            value,
        ));
    }
    let values = numeric.vector_f64();
    if values
        .iter()
        .any(|component| !component.is_finite() || !(0.0..=1.0).contains(component))
    {
        return Err(type_error(
            name,
            position,
            "finite N-by-3 ColorOrder in [0,1]",
            value,
        ));
    }
    Ok((0..numeric.rows)
        .map(|row| {
            [
                values[row] as f32,
                values[row + numeric.rows] as f32,
                values[row + numeric.rows * 2] as f32,
                1.0,
            ]
        })
        .collect())
}

fn line_style_order(
    name: &str,
    position: usize,
    value: &Value,
) -> Result<Vec<LineStyle>, BuiltinError> {
    let values = if let Value::Cell(cell) = value {
        cell.values()
    } else {
        std::slice::from_ref(value)
    };
    if values.is_empty() {
        return Err(type_error(name, position, "nonempty LineStyleOrder", value));
    }
    values
        .iter()
        .map(|value| line_style_value(name, position, value))
        .collect()
}

fn graphics_handle(value: &Value) -> Option<GraphicsHandle> {
    match value {
        Value::Graphics(handle) => Some(*handle),
        Value::GraphicsArray(array) if array.numel() == 1 => array.as_slice().first().copied(),
        _ => None,
    }
}

fn graphics_scalar(
    name: &str,
    position: usize,
    value: &Value,
    expected: GraphicsClass,
) -> Result<GraphicsHandle, BuiltinError> {
    let handle = graphics_handle(value)
        .ok_or_else(|| type_error(name, position, expected.class_name(), value))?;
    if handle.class() != expected {
        return Err(type_error(name, position, expected.class_name(), value));
    }
    Ok(handle)
}

fn graphics_axes_scalar(
    name: &str,
    position: usize,
    value: &Value,
) -> Result<GraphicsHandle, BuiltinError> {
    let handle = graphics_handle(value)
        .ok_or_else(|| type_error(name, position, "Axes or PolarAxes", value))?;
    if !matches!(
        handle.class(),
        GraphicsClass::Axes2D | GraphicsClass::PolarAxes
    ) {
        return Err(type_error(name, position, "Axes or PolarAxes", value));
    }
    Ok(handle)
}

fn axes_creation_properties(
    arguments: &[Value],
) -> Result<(Option<GraphicsHandle>, Vec<GraphicsPropertyUpdate>), BuiltinError> {
    axes_creation_properties_for("axes", GraphicsClass::Axes2D, arguments)
}

fn axes_creation_properties_for(
    name: &str,
    class: GraphicsClass,
    arguments: &[Value],
) -> Result<(Option<GraphicsHandle>, Vec<GraphicsPropertyUpdate>), BuiltinError> {
    if !arguments.len().is_multiple_of(2) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            format!("`{name}` constructor properties must be supplied as name/value pairs"),
        ));
    }
    let mut figure = None;
    let mut properties = Vec::with_capacity(arguments.len() / 2);
    for (pair, arguments) in arguments.chunks_exact(2).enumerate() {
        let name_position = pair * 2 + 1;
        let value_position = name_position + 1;
        let prefix = lowercase_ascii_text(name, name_position, &arguments[0])?;
        let parent_matches = "parent".starts_with(&prefix);
        let position_matches = "position".starts_with(&prefix);
        if parent_matches && position_matches {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Other,
                format!("`{name}` graphics property prefix `{prefix}` is ambiguous"),
            ));
        }
        if parent_matches {
            figure = Some(graphics_scalar(
                name,
                value_position,
                &arguments[1],
                GraphicsClass::Figure,
            )?);
        } else {
            properties.push(parse_property_update(
                name,
                name_position,
                value_position,
                class,
                &arguments[0],
                &arguments[1],
            )?);
        }
    }
    Ok((figure, properties))
}

fn axes_position(name: &str, position: usize, value: &Value) -> Result<[f64; 4], BuiltinError> {
    let numeric = PreparedNumeric::from_value(name, position, value)?;
    if !numeric.is_vector() || numeric.numel() != 4 {
        return Err(type_error(
            name,
            position,
            "four-element normalized Position vector",
            value,
        ));
    }
    let values = numeric.vector_f64();
    let rectangle = [values[0], values[1], values[2], values[3]];
    let [left, bottom, width, height] = rectangle;
    if rectangle.iter().any(|value| !value.is_finite())
        || left < 0.0
        || bottom < 0.0
        || width <= 0.0
        || height <= 0.0
        || left + width > 1.0
        || bottom + height > 1.0
    {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!(
                "input {position} to `{name}` must be a normalized [left bottom width height] rectangle inside the Figure"
            ),
        ));
    }
    Ok(rectangle)
}

fn figure_position(name: &str, position: usize, value: &Value) -> Result<[f64; 4], BuiltinError> {
    let numeric = PreparedNumeric::from_value(name, position, value)?;
    if !numeric.is_vector() || numeric.numel() != 4 {
        return Err(type_error(
            name,
            position,
            "four-element Figure Position vector",
            value,
        ));
    }
    let values = numeric.vector_f64();
    let rectangle = [values[0], values[1], values[2], values[3]];
    if rectangle.iter().any(|value| !value.is_finite())
        || rectangle[2] <= 0.0
        || rectangle[3] <= 0.0
    {
        return Err(type_error(
            name,
            position,
            "finite Figure Position with positive width and height",
            value,
        ));
    }
    Ok(rectangle)
}

fn subplot_position(rows: u32, columns: u32, value: &Value) -> Result<[f64; 4], BuiltinError> {
    let numeric = PreparedNumeric::from_value("subplot", 3, value)?;
    if !numeric.is_vector() || numeric.numel() == 0 {
        return Err(type_error(
            "subplot",
            3,
            "nonempty positive integer index vector",
            value,
        ));
    }
    let count = f64::from(rows) * f64::from(columns);
    let values = numeric.vector_f64();
    if values
        .iter()
        .any(|index| !index.is_finite() || *index < 1.0 || index.fract() != 0.0 || *index > count)
    {
        return Err(type_error(
            "subplot",
            3,
            "indices within the subplot grid",
            value,
        ));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let indices = values
        .iter()
        .map(|index| (*index as u64) - 1)
        .collect::<Vec<_>>();
    let columns_u64 = u64::from(columns);
    let min_row = u32::try_from(
        indices
            .iter()
            .map(|index| index / columns_u64)
            .min()
            .unwrap(),
    )
    .expect("validated subplot row must fit u32");
    let max_row = u32::try_from(
        indices
            .iter()
            .map(|index| index / columns_u64)
            .max()
            .unwrap(),
    )
    .expect("validated subplot row must fit u32");
    let min_column = u32::try_from(
        indices
            .iter()
            .map(|index| index % columns_u64)
            .min()
            .unwrap(),
    )
    .expect("validated subplot column must fit u32");
    let max_column = u32::try_from(
        indices
            .iter()
            .map(|index| index % columns_u64)
            .max()
            .unwrap(),
    )
    .expect("validated subplot column must fit u32");

    let columns = f64::from(columns);
    let rows = f64::from(rows);
    let cell_width = 0.775 * 19.0 / (25.0 * columns - 6.0);
    let horizontal_gap = cell_width * 6.0 / 19.0;
    let cell_height = 0.815 * 18.0 / (25.0 * rows - 7.0);
    let vertical_gap = cell_height * 7.0 / 18.0;
    let left = 0.13 + f64::from(min_column) * (cell_width + horizontal_gap);
    let bottom = 0.11 + (rows - 1.0 - f64::from(max_row)) * (cell_height + vertical_gap);
    let width = f64::from(max_column - min_column + 1) * cell_width
        + f64::from(max_column - min_column) * horizontal_gap;
    let height = f64::from(max_row - min_row + 1) * cell_height
        + f64::from(max_row - min_row) * vertical_gap;
    Ok([left, bottom, width, height])
}

#[derive(Clone, Copy)]
enum FigureCreationProperty {
    Color,
    Name,
    NumberTitle,
    Visible,
    Position,
}

fn figure_creation_properties(
    arguments: &[Value],
) -> Result<FigureCreationProperties, BuiltinError> {
    if arguments.is_empty() || !arguments.len().is_multiple_of(2) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            "`figure` constructor properties must be supplied as name/value pairs",
        ));
    }
    let mut properties = FigureCreationProperties::default();
    for (pair, arguments) in arguments.chunks_exact(2).enumerate() {
        let name_position = pair * 2 + 1;
        let value_position = name_position + 1;
        let prefix = lowercase_ascii_text("figure", name_position, &arguments[0])?;
        match figure_creation_property(&prefix)? {
            FigureCreationProperty::Color => {
                properties.background_rgba =
                    rgb_color("figure", value_position, "Color", &arguments[1])?;
            }
            FigureCreationProperty::Name => {
                properties.name_utf16 = text_scalar("figure", value_position, &arguments[1])?;
            }
            FigureCreationProperty::Visible => {
                properties.visible = visible_value("figure", value_position, &arguments[1])?;
            }
            FigureCreationProperty::NumberTitle => {
                properties.number_title = visible_value("figure", value_position, &arguments[1])?;
            }
            FigureCreationProperty::Position => {
                properties.position_css_pixels =
                    figure_position("figure", value_position, &arguments[1])?;
            }
        }
    }
    Ok(properties)
}

fn figure_creation_property(prefix: &str) -> Result<FigureCreationProperty, BuiltinError> {
    const PROPERTIES: &[(&str, FigureCreationProperty)] = &[
        ("color", FigureCreationProperty::Color),
        ("name", FigureCreationProperty::Name),
        ("numbertitle", FigureCreationProperty::NumberTitle),
        ("visible", FigureCreationProperty::Visible),
        ("position", FigureCreationProperty::Position),
    ];
    if prefix != "color" && "color".starts_with(prefix) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Other,
            format!("`figure` constructor property prefix `{prefix}` is ambiguous"),
        ));
    }
    if let Some((_, property)) = PROPERTIES.iter().find(|(name, _)| *name == prefix) {
        return Ok(*property);
    }
    let matches = PROPERTIES
        .iter()
        .filter(|(name, _)| name.starts_with(prefix))
        .map(|(_, property)| *property)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [property] => Ok(*property),
        [] => Err(BuiltinError::new(
            BuiltinErrorCategory::Graphics,
            format!("`figure` does not recognize constructor property `{prefix}`"),
        )),
        _ => Err(BuiltinError::new(
            BuiltinErrorCategory::Other,
            format!("`figure` constructor property prefix `{prefix}` is ambiguous"),
        )),
    }
}

fn close_command_request(arguments: &[Value]) -> Result<GraphicsRequest, BuiltinError> {
    expect_argument_count_range("close", arguments, 1, 3)?;
    if lowercase_ascii_text("close", 1, &arguments[0])? != "all" {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "text-form `close` currently requires `all` as its first option",
        ));
    }
    let mut hidden = false;
    let mut force = false;
    for (index, value) in arguments[1..].iter().enumerate() {
        let position = index + 2;
        match lowercase_ascii_text("close", position, value)?.as_str() {
            "hidden" if !hidden => hidden = true,
            "force" if !force => force = true,
            "hidden" | "force" => {
                return Err(BuiltinError::new(
                    BuiltinErrorCategory::Domain,
                    format!("input {position} to `close` repeats an existing option"),
                ));
            }
            option => {
                return Err(BuiltinError::new(
                    BuiltinErrorCategory::Domain,
                    format!("input {position} to `close` is not `hidden` or `force`: `{option}`"),
                ));
            }
        }
    }
    // The retained model has neither HandleVisibility nor CloseRequestFcn.
    // Therefore all representable Figures are in the default `close all` set,
    // while `hidden` and `force` are behaviorally neutral on that exact subset.
    let _ = (hidden, force);
    Ok(GraphicsRequest::CloseAllFigures)
}

fn is_text_value(value: &Value) -> bool {
    matches!(value, Value::String(_) | Value::Array(ArrayData::Char(_)))
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn positive_figure_number(value: &Value) -> Result<u32, BuiltinError> {
    let number = value
        .as_real_number()
        .filter(|number| number.is_finite() && *number >= 1.0 && number.fract() == 0.0)
        .and_then(|number| u32::try_from(number as u64).ok())
        .ok_or_else(|| {
            BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "`figure(n)` requires a positive integer representable by the v1 Figure number",
            )
        })?;
    Ok(number)
}

fn text_scalar(name: &str, position: usize, value: &Value) -> Result<Vec<u16>, BuiltinError> {
    match value {
        Value::String(string_value) => string_value
            .as_scalar()
            .filter(|element| !element.is_missing())
            .map(|element| element.code_units().to_vec())
            .ok_or_else(|| type_error(name, position, "nonmissing text scalar", value)),
        Value::Array(ArrayData::Char(array))
            if array.shape().dimensions() == [0, 0]
                || (array.shape().dimensions().len() == 2
                    && array.shape().dimensions()[0] == 1) =>
        {
            Ok(array.as_slice().iter().map(|unit| unit.get()).collect())
        }
        _ => Err(type_error(
            name,
            position,
            "string scalar or char row vector",
            value,
        )),
    }
}

fn on_off_mode(name: &str, position: usize, value: &Value) -> Result<bool, BuiltinError> {
    let code_units = text_scalar(name, position, value)?;
    if code_units == "on".encode_utf16().collect::<Vec<_>>() {
        Ok(true)
    } else if code_units == "off".encode_utf16().collect::<Vec<_>>() {
        Ok(false)
    } else {
        Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("input {position} to `{name}` must be exactly `on` or `off`"),
        ))
    }
}

fn one_handle(response: &GraphicsResponse, name: &str) -> BuiltinResult {
    let GraphicsResponse::Handle(handle) = response else {
        return Err(graphics_state_error(name, "scalar graphics handle"));
    };
    Ok(vec![Value::Graphics(*handle)])
}

fn created_handle(
    context: &mut BuiltinContext<'_>,
    request: GraphicsRequest,
    name: &str,
) -> BuiltinResult {
    let requested_outputs = context.requested_outputs();
    let response = context.graphics(request)?;
    one_handle_if_requested(&response, name, requested_outputs)
}

fn one_handle_if_requested(
    response: &GraphicsResponse,
    name: &str,
    requested_outputs: usize,
) -> BuiltinResult {
    let GraphicsResponse::Handle(handle) = response else {
        return Err(graphics_state_error(name, "scalar graphics handle"));
    };
    Ok((requested_outputs != 0)
        .then_some(Value::Graphics(*handle))
        .into_iter()
        .collect())
}

fn no_value(response: &GraphicsResponse, name: &str) -> BuiltinResult {
    if *response == GraphicsResponse::None {
        Ok(Vec::new())
    } else {
        Err(graphics_state_error(name, "no language value"))
    }
}

fn graphics_state_error(name: &str, expected: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Graphics,
        format!("`{name}` graphics service did not return the expected {expected}"),
    )
}

fn plot_dimensions_error() -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        "`plot(X,Y)` v1 requires equal vectors, a length-M X vector with M-by-N Y, or equal M-by-N matrices",
    )
}

fn plot_size_error(name: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("`{name}` input shape exceeds host limits"),
    )
}

#[cfg(test)]
mod tests {
    use openmat_runtime::{CancellationToken, VecOutput};
    use openmat_value::{StringElement, StringValue};

    use super::*;

    fn invoke_direct(
        session: &mut openmat_graphics_model::GraphicsSession,
        arguments: &[Value],
        requested_outputs: usize,
        invoke: impl FnOnce(&[Value], &mut BuiltinContext<'_>) -> BuiltinResult,
    ) -> BuiltinResult {
        let cancellation = CancellationToken::new();
        let mut output = VecOutput::new();
        let mut context = BuiltinContext::with_graphics_service(
            requested_outputs,
            &cancellation,
            &mut output,
            session,
        );
        invoke(arguments, &mut context)
    }

    fn text(value: &str) -> Value {
        Value::String(StringValue::Scalar(StringElement::from(value)))
    }

    fn row(values: Vec<f64>) -> Value {
        let shape = Shape::new([1, values.len() as u64]).unwrap();
        Value::Array(ArrayData::F64(DenseArray::from_vec(shape, values).unwrap()))
    }

    fn matrix(rows: u64, columns: u64, values: Vec<f64>) -> Value {
        let shape = Shape::new([rows, columns]).unwrap();
        Value::Array(ArrayData::F64(DenseArray::from_vec(shape, values).unwrap()))
    }

    fn volume(dimensions: [u64; 3], values: Vec<f64>) -> Value {
        let shape = Shape::new(dimensions).unwrap();
        Value::Array(ArrayData::F64(DenseArray::from_vec(shape, values).unwrap()))
    }

    fn text_cell(values: &[&str]) -> Value {
        let shape = Shape::new([1, values.len() as u64]).unwrap();
        Value::Cell(
            CellArray::from_values(shape, values.iter().copied().map(text).collect()).unwrap(),
        )
    }

    #[test]
    fn axes_parent_position_and_subplot_match_r2022b_layout() {
        let mut session = openmat_graphics_model::GraphicsSession::new();
        let figure = invoke_direct(&mut session, &[], 1, figure_builtin).unwrap();
        let Value::Graphics(figure) = figure[0] else {
            panic!()
        };
        let axes = invoke_direct(
            &mut session,
            &[
                text("Parent"),
                Value::Graphics(figure),
                text("Position"),
                row(vec![0.2, 0.25, 0.5, 0.5]),
            ],
            1,
            axes_builtin,
        )
        .unwrap();
        let Value::Graphics(axes) = axes[0] else {
            panic!()
        };
        assert!(matches!(
            session.object(axes),
            Ok(openmat_graphics_model::GraphicsObject::Axes2D(properties))
                if properties
                    .position_normalized
                    .iter()
                    .zip([0.2, 0.25, 0.5, 0.5])
                    .all(|(actual, expected)| (actual - expected).abs() <= f64::EPSILON)
        ));

        let first = invoke_direct(
            &mut session,
            &[Value::Double(2.0), Value::Double(2.0), Value::Double(1.0)],
            1,
            subplot_builtin,
        )
        .unwrap();
        let Value::Graphics(first) = first[0] else {
            panic!()
        };
        let Ok(openmat_graphics_model::GraphicsObject::Axes2D(properties)) = session.object(first)
        else {
            panic!()
        };
        let expected = [
            0.13,
            0.583_837_209_302_325_5,
            0.334_659_090_909_090_9,
            0.341_162_790_697_674_5,
        ];
        assert!(
            properties
                .position_normalized
                .iter()
                .zip(expected)
                .all(|(actual, expected)| (actual - expected).abs() <= 1.0e-14)
        );
        let selected = invoke_direct(
            &mut session,
            &[Value::Double(2.0), Value::Double(2.0), Value::Double(1.0)],
            1,
            subplot_builtin,
        )
        .unwrap();
        assert_eq!(selected, vec![Value::Graphics(first)]);
    }

    #[test]
    fn tiledlayout_and_nexttile_cover_handles_selection_spans_and_options() {
        let mut session = openmat_graphics_model::GraphicsSession::new();
        let layout = invoke_direct(
            &mut session,
            &[
                Value::Double(2.0),
                Value::Double(2.0),
                text("TileSpacing"),
                text("compact"),
                text("Padding"),
                text("tight"),
            ],
            1,
            tiledlayout_builtin,
        )
        .unwrap();
        let Value::Graphics(layout) = layout[0] else {
            panic!()
        };
        assert_eq!(
            layout.class().class_name(),
            "matlab.graphics.layout.TiledChartLayout"
        );
        assert!(matches!(
            session.object(layout),
            Ok(openmat_graphics_model::GraphicsObject::TiledChartLayout(properties))
                if properties.tile_spacing == TileSpacing::Compact
                    && properties.padding == TilePadding::Tight
        ));

        let first = invoke_direct(
            &mut session,
            &[Value::Graphics(layout)],
            1,
            nexttile_builtin,
        )
        .unwrap();
        let Value::Graphics(first) = first[0] else {
            panic!()
        };
        let selected = invoke_direct(
            &mut session,
            &[Value::Graphics(layout), Value::Double(1.0)],
            1,
            nexttile_builtin,
        )
        .unwrap();
        assert_eq!(selected, vec![Value::Graphics(first)]);

        let layout = invoke_direct(
            &mut session,
            &[Value::Double(3.0), Value::Double(4.0)],
            1,
            tiledlayout_builtin,
        )
        .unwrap();
        let Value::Graphics(layout) = layout[0] else {
            panic!()
        };
        let span = invoke_direct(
            &mut session,
            &[Value::Graphics(layout), row(vec![2.0, 2.0])],
            1,
            nexttile_builtin,
        )
        .unwrap();
        let Value::Graphics(span) = span[0] else {
            panic!()
        };
        let next = invoke_direct(
            &mut session,
            &[Value::Graphics(layout)],
            1,
            nexttile_builtin,
        )
        .unwrap();
        let Value::Graphics(next) = next[0] else {
            panic!()
        };
        assert_ne!(span, next);
        let openmat_graphics_model::GraphicsObject::TiledChartLayout(properties) =
            session.object(layout).unwrap()
        else {
            panic!()
        };
        assert_eq!(properties.tile_axes[0], Some(span));
        assert_eq!(properties.tile_axes[1], Some(span));
        assert_eq!(properties.tile_axes[4], Some(span));
        assert_eq!(properties.tile_axes[5], Some(span));
        assert_eq!(properties.tile_axes[2], Some(next));
    }

    #[test]
    fn xtick_shortcuts_and_box_use_the_same_axes_model_properties() {
        let mut session = openmat_graphics_model::GraphicsSession::new();
        invoke_direct(&mut session, &[row(vec![0.0, 5.0, 10.0])], 0, |a, c| {
            xticks_builtin(a, c)
        })
        .unwrap();
        let ticks = invoke_direct(&mut session, &[], 1, xticks_builtin).unwrap();
        let Value::Array(ArrayData::F64(ticks)) = &ticks[0] else {
            panic!()
        };
        assert_eq!(ticks.as_slice(), &[0.0, 5.0, 10.0]);

        invoke_direct(&mut session, &[text_cell(&["A", "B"])], 0, |a, c| {
            xticklabels_builtin(a, c)
        })
        .unwrap();
        let mode = invoke_direct(&mut session, &[text("mode")], 1, |a, c| {
            xticklabels_builtin(a, c)
        })
        .unwrap();
        let Value::Array(ArrayData::Char(mode)) = &mode[0] else {
            panic!()
        };
        assert_eq!(
            mode.as_slice()
                .iter()
                .map(|unit| unit.get())
                .collect::<Vec<_>>(),
            "manual".encode_utf16().collect::<Vec<_>>()
        );

        invoke_direct(&mut session, &[], 0, box_builtin).unwrap();
        let axes = session.current_axes().unwrap();
        assert!(matches!(
            session.object(axes),
            Ok(openmat_graphics_model::GraphicsObject::Axes2D(properties))
                if properties.box_enabled
                    && properties.x_tick_mode == TickMode::Manual
                    && properties.x_tick_label_mode == TickMode::Manual
        ));
        invoke_direct(&mut session, &[text("off")], 0, box_builtin).unwrap();
        assert!(matches!(
            session.object(axes),
            Ok(openmat_graphics_model::GraphicsObject::Axes2D(properties))
                if !properties.box_enabled
        ));
    }

    #[test]
    fn yz_tick_shortcuts_pad_labels_and_preserve_axis_identity() {
        let mut session = openmat_graphics_model::GraphicsSession::new();
        invoke_direct(&mut session, &[row(vec![1.0, 3.0, 7.0])], 0, yticks_builtin).unwrap();
        invoke_direct(
            &mut session,
            &[text_cell(&["u", "v"])],
            0,
            yticklabels_builtin,
        )
        .unwrap();
        invoke_direct(&mut session, &[row(vec![0.0, 2.0])], 0, zticks_builtin).unwrap();
        invoke_direct(
            &mut session,
            &[text_cell(&["low", "high", "unused"])],
            0,
            zticklabels_builtin,
        )
        .unwrap();

        let axes = session.current_axes().unwrap();
        assert!(matches!(
            session.object(axes),
            Ok(openmat_graphics_model::GraphicsObject::Axes2D(properties))
                if properties.y_ticks == [1.0, 3.0, 7.0]
                    && properties.y_tick_labels_utf16.len() == 3
                    && properties.y_tick_labels_utf16[2].is_empty()
                    && properties.z_ticks == [0.0, 2.0]
                    && properties.z_tick_labels_utf16.len() == 2
        ));
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn clim_caxis_and_axis_shortcuts_update_one_axes_transactionally() {
        let mut session = openmat_graphics_model::GraphicsSession::new();
        invoke_direct(&mut session, &[row(vec![0.0, 8.0])], 0, clim_builtin).unwrap();
        let result = invoke_direct(&mut session, &[], 1, caxis_builtin).unwrap();
        let Value::Array(ArrayData::F64(limits)) = &result[0] else {
            panic!()
        };
        assert_eq!(limits.as_slice(), &[0.0, 8.0]);
        invoke_direct(&mut session, &[text("auto")], 0, caxis_builtin).unwrap();

        invoke_direct(
            &mut session,
            &[row(vec![2.0, 4.0, 10.0, 20.0])],
            0,
            axis_builtin,
        )
        .unwrap();
        let query = invoke_direct(&mut session, &[], 1, axis_builtin).unwrap();
        let Value::Array(ArrayData::F64(query)) = &query[0] else {
            panic!()
        };
        assert_eq!(query.as_slice(), &[2.0, 4.0, 10.0, 20.0]);

        invoke_direct(&mut session, &[text("padded")], 0, axis_builtin).unwrap();
        invoke_direct(&mut session, &[text("equal")], 0, axis_builtin).unwrap();
        invoke_direct(&mut session, &[text("off")], 0, axis_builtin).unwrap();
        invoke_direct(&mut session, &[text("ij")], 0, axis_builtin).unwrap();
        let axes = session.current_axes().unwrap();
        assert!(matches!(
            session.object(axes),
            Ok(openmat_graphics_model::GraphicsObject::Axes2D(properties))
                if properties.c_limit_mode == LimitMode::Auto
                    && properties.x_limit_method == LimitMethod::Padded
                    && properties.y_limit_method == LimitMethod::Padded
                    && properties.data_aspect_ratio == [1.0, 1.0, 1.0]
                    && properties.data_aspect_ratio_mode == LimitMode::Manual
                    && properties.plot_box_aspect_ratio_mode == LimitMode::Auto
                    && !properties.visible
                    && properties.y_direction == AxisDirection::Reverse
        ));
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn semilog_shortcuts_create_lines_and_scales_in_one_graphics_request() {
        let x = row(vec![1.0, 10.0, 100.0]);
        let y = row(vec![0.1, 1.0, 10.0]);
        let mut session = openmat_graphics_model::GraphicsSession::new();

        invoke_direct(&mut session, &[x.clone(), y.clone()], 1, semilogx_builtin).unwrap();
        let axes = session.current_axes().unwrap();
        assert!(matches!(
            session.object(axes),
            Ok(openmat_graphics_model::GraphicsObject::Axes2D(properties))
                if properties.x_scale == AxisScale::Log
                    && properties.y_scale == AxisScale::Linear
                    && properties.x_limits == [1.0, 100.0]
        ));

        invoke_direct(&mut session, &[x.clone(), y.clone()], 1, semilogy_builtin).unwrap();
        assert!(matches!(
            session.object(axes),
            Ok(openmat_graphics_model::GraphicsObject::Axes2D(properties))
                if properties.x_scale == AxisScale::Linear
                    && properties.y_scale == AxisScale::Log
                    && properties.y_limits == [0.1, 10.0]
        ));

        invoke_direct(&mut session, &[x, y], 1, loglog_builtin).unwrap();
        assert!(matches!(
            session.object(axes),
            Ok(openmat_graphics_model::GraphicsObject::Axes2D(properties))
                if properties.x_scale == AxisScale::Log
                    && properties.y_scale == AxisScale::Log
        ));
    }

    #[test]
    fn patch_forms_lower_to_one_first_class_patch_object() {
        let mut session = openmat_graphics_model::GraphicsSession::new();
        let output = invoke_direct(
            &mut session,
            &[
                row(vec![0.0, 1.0, 1.0, 0.0]),
                row(vec![0.0, 0.0, 1.0, 1.0]),
                text("red"),
            ],
            1,
            patch_builtin,
        )
        .unwrap();
        let Value::Graphics(handle) = output[0] else {
            panic!()
        };
        let openmat_graphics_model::GraphicsObject::PatchSeries(properties) =
            session.object(handle).unwrap()
        else {
            panic!()
        };
        assert_eq!(properties.faces.shape, [1, 4]);
        assert_eq!(properties.vertices.shape, [4, 2]);
        assert_eq!(properties.face_vertex_cdata.shape, [0, 0]);
        assert_eq!(
            properties.face_color,
            SurfaceColor::Rgba([1.0, 0.0, 0.0, 1.0])
        );

        let output = invoke_direct(
            &mut session,
            &[
                text("Faces"),
                matrix(2, 3, vec![1.0, 1.0, 2.0, 3.0, 3.0, 4.0]),
                text("Vertices"),
                matrix(
                    4,
                    3,
                    vec![0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0],
                ),
                text("FaceVertexCData"),
                matrix(4, 1, vec![1.0, 2.0, 3.0, 4.0]),
                text("FaceColor"),
                text("interp"),
            ],
            1,
            patch_builtin,
        )
        .unwrap();
        let Value::Graphics(handle) = output[0] else {
            panic!()
        };
        assert!(matches!(
            session.object(handle),
            Ok(openmat_graphics_model::GraphicsObject::PatchSeries(properties))
                if properties.vertices.shape == [4, 3]
                    && properties.face_color == SurfaceColor::Interp
        ));
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn surf_accepts_cdata_and_constructor_properties_atomically() {
        let mut session = openmat_graphics_model::GraphicsSession::new();
        let output = invoke_direct(
            &mut session,
            &[
                matrix(2, 2, vec![1.0, 2.0, 3.0, 4.0]),
                matrix(2, 2, vec![10.0, 20.0, 30.0, 40.0]),
                text("EdgeColor"),
                text("none"),
                text("FaceColor"),
                text("interp"),
                text("FaceAlpha"),
                Value::Double(0.25),
                text("LineWidth"),
                Value::Double(2.0),
            ],
            1,
            surf_builtin,
        )
        .unwrap();
        let Value::Graphics(handle) = output[0] else {
            panic!()
        };
        assert!(matches!(
            session.object(handle),
            Ok(openmat_graphics_model::GraphicsObject::SurfaceSeries(properties))
                if properties.c_data.shape == [2, 2]
                    && properties.edge_color == SurfaceColor::None
                    && properties.face_color == SurfaceColor::Interp
                    && properties.face_alpha == 0.25
                    && properties.line_width_points == 2.0
        ));
        assert_eq!(session.take_pending_deltas().len(), 1);
    }

    #[test]
    fn patch_accepts_scalar_isosurface_style_structure() {
        let shape = Shape::new([1, 1]).unwrap();
        let structure = openmat_value::StructArray::from_columns(
            shape,
            vec![
                openmat_value::FieldName::new("vertices").unwrap(),
                openmat_value::FieldName::new("faces").unwrap(),
            ],
            vec![
                vec![matrix(
                    4,
                    3,
                    vec![0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0],
                )],
                vec![matrix(2, 3, vec![1.0, 1.0, 2.0, 3.0, 3.0, 4.0])],
            ],
        )
        .unwrap();
        let mut session = openmat_graphics_model::GraphicsSession::new();
        let output = invoke_direct(
            &mut session,
            &[
                Value::Struct(structure),
                text("FaceColor"),
                text("red"),
                text("EdgeColor"),
                text("none"),
            ],
            1,
            patch_builtin,
        )
        .unwrap();
        let Value::Graphics(handle) = output[0] else {
            panic!()
        };
        assert!(matches!(
            session.object(handle),
            Ok(openmat_graphics_model::GraphicsObject::PatchSeries(properties))
                if properties.vertices.shape == [4, 3]
                    && properties.faces.shape == [2, 3]
                    && properties.face_color == SurfaceColor::Rgba([1.0, 0.0, 0.0, 1.0])
                    && properties.edge_color == SurfaceColor::None
        ));
    }

    #[test]
    fn isonormals_headlight_and_gouraud_lighting_reach_retained_patch_state() {
        let mut session = openmat_graphics_model::GraphicsSession::new();
        let output = invoke_direct(
            &mut session,
            &[
                text("Faces"),
                matrix(2, 3, vec![1.0, 1.0, 2.0, 3.0, 3.0, 4.0]),
                text("Vertices"),
                matrix(
                    4,
                    3,
                    vec![0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0],
                ),
            ],
            1,
            patch_builtin,
        )
        .unwrap();
        let Value::Graphics(patch) = output[0] else {
            panic!()
        };
        let scalar_volume = volume([2, 2, 2], vec![-1.0, 1.0, 1.0, -1.0, 1.0, -1.0, -1.0, 1.0]);
        invoke_direct(
            &mut session,
            &[scalar_volume, Value::Graphics(patch)],
            0,
            isonormals_builtin,
        )
        .unwrap();
        invoke_direct(&mut session, &[text("headlight")], 0, camlight_builtin).unwrap();
        assert!(matches!(
            session.object(patch),
            Ok(openmat_graphics_model::GraphicsObject::PatchSeries(properties))
                if properties.smooth_normals
                    && properties.vertex_normals.is_some()
                    && properties.lighting_enabled
        ));
        invoke_direct(&mut session, &[text("gouraud")], 0, lighting_builtin).unwrap();
        let axes = invoke_direct(&mut session, &[], 1, gca_builtin).unwrap();
        let Value::Graphics(axes) = axes[0] else {
            panic!()
        };
        assert!(matches!(
            session.object(patch),
            Ok(openmat_graphics_model::GraphicsObject::PatchSeries(properties))
                if properties.smooth_normals
                    && properties.vertex_normals.is_some()
                    && properties.lighting_enabled
        ));
        assert!(matches!(
            session.object(axes),
            Ok(openmat_graphics_model::GraphicsObject::Axes2D(properties))
                if properties.headlight_enabled
                    && properties.lighting_mode == LightingMode::Gouraud
        ));
    }

    #[test]
    fn trisurf_and_trimesh_keep_r2022b_default_face_and_edge_modes() {
        let tri = matrix(2, 3, vec![1.0, 1.0, 2.0, 3.0, 3.0, 4.0]);
        let x = row(vec![0.0, 1.0, 1.0, 0.0]);
        let y = row(vec![0.0, 0.0, 1.0, 1.0]);
        let z = row(vec![0.0, 1.0, 0.0, 1.0]);
        let mut session = openmat_graphics_model::GraphicsSession::new();
        let surf = invoke_direct(
            &mut session,
            &[tri.clone(), x.clone(), y.clone(), z.clone()],
            1,
            trisurf_builtin,
        )
        .unwrap();
        let Value::Graphics(surf) = surf[0] else {
            panic!()
        };
        assert!(matches!(
            session.object(surf),
            Ok(openmat_graphics_model::GraphicsObject::PatchSeries(properties))
                if properties.face_color == SurfaceColor::Flat
                    && properties.edge_color == SurfaceColor::Rgba([0.0, 0.0, 0.0, 1.0])
        ));

        let mesh = invoke_direct(&mut session, &[tri, x, y, z], 1, trimesh_builtin).unwrap();
        let Value::Graphics(mesh) = mesh[0] else {
            panic!()
        };
        assert!(matches!(
            session.object(mesh),
            Ok(openmat_graphics_model::GraphicsObject::PatchSeries(properties))
                if properties.face_color == SurfaceColor::Rgba([1.0, 1.0, 1.0, 1.0])
                    && properties.edge_color == SurfaceColor::Flat
        ));
    }

    #[test]
    fn high_frequency_2d_charts_keep_public_class_and_retained_primitives() {
        let x = row(vec![1.0, 2.0, 3.0]);
        let y = row(vec![2.0, 5.0, 1.0]);
        let mut session = openmat_graphics_model::GraphicsSession::new();
        for (expected, invoke) in [
            (
                GraphicsClass::Stair,
                stairs_builtin as fn(&[Value], &mut BuiltinContext<'_>) -> BuiltinResult,
            ),
            (GraphicsClass::Stem, stem_builtin),
            (GraphicsClass::Area, area_builtin),
            (GraphicsClass::Bar, bar_builtin),
        ] {
            let output = invoke_direct(&mut session, &[x.clone(), y.clone()], 1, invoke).unwrap();
            let Value::Graphics(handle) = output[0] else {
                panic!()
            };
            assert_eq!(handle.class(), expected);
            assert!(matches!(
                session.object(handle),
                Ok(openmat_graphics_model::GraphicsObject::ChartGroup(properties))
                    if properties.kind.graphics_class() == expected
            ));
            assert!(!session.children(handle).unwrap().is_empty());
        }
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn multi_plot_grouped_bar_and_filled_contours_retain_independent_primitives() {
        let mut session = openmat_graphics_model::GraphicsSession::new();
        let x = row(vec![1.0, 2.0, 3.0]);
        let plot = invoke_direct(
            &mut session,
            &[
                x.clone(),
                row(vec![1.0, 2.0, 3.0]),
                text("r-o"),
                x,
                row(vec![3.0, 2.0, 1.0]),
                text("g--"),
                text("LineWidth"),
                Value::Double(1.25),
            ],
            1,
            plot_builtin,
        )
        .unwrap();
        let Value::GraphicsArray(lines) = &plot[0] else {
            panic!()
        };
        assert_eq!(lines.as_slice().len(), 2);
        let openmat_graphics_model::GraphicsObject::LineSeries(first) =
            session.object(lines.as_slice()[0]).unwrap()
        else {
            panic!()
        };
        let openmat_graphics_model::GraphicsObject::LineSeries(second) =
            session.object(lines.as_slice()[1]).unwrap()
        else {
            panic!()
        };
        assert_eq!(first.marker, Marker::Circle);
        assert_eq!(first.color_rgba, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(second.line_style, LineStyle::Dash);
        assert_eq!(second.color_rgba, [0.0, 1.0, 0.0, 1.0]);
        assert_eq!(first.line_width_points, 1.25);
        assert_eq!(second.line_width_points, 1.25);

        let bars = invoke_direct(
            &mut session,
            &[
                matrix(
                    4,
                    3,
                    vec![3.0, 4.0, 7.0, 6.0, 5.0, 2.0, 4.0, 3.0, 2.0, 6.0, 5.0, 4.0],
                ),
                Value::Double(0.75),
                text("grouped"),
                text("LineWidth"),
                Value::Double(1.2),
            ],
            1,
            bar_builtin,
        )
        .unwrap();
        let Value::Graphics(bar) = bars[0] else {
            panic!()
        };
        let children = session.children(bar).unwrap();
        assert_eq!(children.len(), 3);
        let colors = children
            .iter()
            .map(|handle| {
                let openmat_graphics_model::GraphicsObject::PatchSeries(properties) =
                    session.object(*handle).unwrap()
                else {
                    panic!()
                };
                assert_eq!(properties.line_width_points, 1.2);
                properties.face_color
            })
            .collect::<Vec<_>>();
        assert!(colors.windows(2).all(|pair| pair[0] != pair[1]));

        let z = matrix(3, 3, vec![1.0, 2.0, 3.0, 2.0, 3.0, 4.0, 3.0, 4.0, 5.0]);
        let filled = invoke_direct(
            &mut session,
            &[z, Value::Double(4.0), text("LineColor"), text("none")],
            2,
            contourf_builtin,
        )
        .unwrap();
        let Value::Graphics(contour) = filled[1] else {
            panic!()
        };
        let child = session.children(contour).unwrap()[0];
        assert!(matches!(
            session.object(child),
            Ok(openmat_graphics_model::GraphicsObject::PatchSeries(properties))
                if properties.face_color == SurfaceColor::Flat
                    && properties.edge_color == SurfaceColor::None
                    && properties.face_vertex_cdata.shape[0] > 0
        ));
    }

    #[test]
    fn histogram_contour_and_imagesc_close_language_model_and_color_mapping() {
        let mut session = openmat_graphics_model::GraphicsSession::new();
        let histogram = invoke_direct(
            &mut session,
            &[
                row(vec![1.0, 1.0, 2.0, 3.0, 3.0, 3.0]),
                text("Normalization"),
                text("probability"),
            ],
            1,
            histogram_builtin,
        )
        .unwrap();
        let Value::Graphics(histogram) = histogram[0] else {
            panic!()
        };
        assert_eq!(histogram.class(), GraphicsClass::Histogram);

        let z = matrix(3, 3, vec![1.0, 2.0, 3.0, 2.0, 3.0, 4.0, 3.0, 4.0, 5.0]);
        let contour = invoke_direct(
            &mut session,
            &[z.clone(), row(vec![2.0, 3.0, 4.0])],
            2,
            contour_builtin,
        )
        .unwrap();
        let Value::Array(ArrayData::F64(matrix)) = &contour[0] else {
            panic!()
        };
        let Value::Graphics(contour_handle) = contour[1] else {
            panic!()
        };
        assert_eq!(matrix.shape().dimensions()[0], 2);
        assert_eq!(contour_handle.class(), GraphicsClass::Contour);
        let axes = session.current_axes().unwrap();
        assert!(matches!(
            session.object(axes),
            Ok(openmat_graphics_model::GraphicsObject::Axes2D(properties))
                if properties.c_limits.map(f64::to_bits) == [2.0_f64.to_bits(), 4.0_f64.to_bits()]
                    && properties.c_limit_mode == LimitMode::Manual
        ));

        let image = invoke_direct(&mut session, &[z], 1, imagesc_builtin).unwrap();
        let Value::Graphics(image) = image[0] else {
            panic!()
        };
        assert_eq!(image.class(), GraphicsClass::Image);
        let primitive = session.children(image).unwrap()[0];
        assert!(matches!(
            session.object(primitive),
            Ok(openmat_graphics_model::GraphicsObject::PatchSeries(properties))
                if properties.c_data_mapping == CDataMapping::Scaled
                    && properties.face_color == SurfaceColor::Flat
                    && properties.edge_color == SurfaceColor::None
        ));
        let axes = session.current_axes().unwrap();
        assert!(matches!(
            session.object(axes),
            Ok(openmat_graphics_model::GraphicsObject::Axes2D(properties))
                if properties.y_direction == AxisDirection::Reverse
        ));
    }

    #[test]
    fn contour_default_and_requested_levels_follow_r2022b_selection_rules() {
        let assert_levels = |actual: Vec<f64>, expected: &[f64]| {
            assert_eq!(actual.len(), expected.len());
            assert!(
                actual
                    .iter()
                    .zip(expected)
                    .all(|(actual, expected)| (actual - expected).abs() <= 1.0e-14)
            );
        };
        let positive = (1..=6).map(f64::from).collect::<Vec<_>>();
        assert_levels(
            automatic_contour_levels(&positive),
            &[1.5, 2.0, 2.5, 3.0, 3.5, 4.0, 4.5, 5.0, 5.5, 6.0],
        );
        assert_levels(
            requested_contour_levels(&positive, 2),
            &[2.666_666_666_666_667, 4.333_333_333_333_334],
        );
        let signed = (0..=10)
            .map(|index| -1.0 + f64::from(index) * 0.2)
            .collect::<Vec<_>>();
        assert_levels(
            automatic_contour_levels(&signed),
            &[-1.0, -0.8, -0.6, -0.4, -0.2, 0.0, 0.2, 0.4, 0.6, 0.8, 1.0],
        );
    }

    #[test]
    #[allow(clippy::cast_precision_loss)]
    fn dense_filled_contours_use_a_shared_grid_below_the_renderer_budget() {
        let rows = 230;
        let columns = 230;
        let levels = (1..=80).map(f64::from).collect::<Vec<_>>();
        let properties = [GraphicsPropertyUpdate::EdgeColor(SurfaceColor::None)];
        assert!(use_compact_filled_contours(
            rows,
            columns,
            levels.len(),
            &properties,
        ));
        assert!(!use_compact_filled_contours(
            rows,
            columns,
            levels.len(),
            &[GraphicsPropertyUpdate::EdgeColor(SurfaceColor::Rgba([
                0.0, 0.0, 0.0, 1.0,
            ]))],
        ));

        let x = (0..columns).map(|value| value as f64).collect::<Vec<_>>();
        let y = (0..rows).map(|value| value as f64).collect::<Vec<_>>();
        let z = (0..rows * columns)
            .map(|index| (index % 97) as f64)
            .collect::<Vec<_>>();
        let patch = compact_filled_contour_patch(&x, &y, &z, rows, columns, &levels).unwrap();
        let expected_faces = 2 * (rows - 1) * (columns - 1);
        assert_eq!(patch.face_rows, expected_faces as u64);
        assert_eq!(patch.face_columns, 3);
        assert_eq!(patch.vertex_rows, (rows * columns) as u64);
        assert_eq!(patch.vertex_columns, 2);
        assert_eq!(patch.cdata_rows, expected_faces as u64);
        assert_eq!(patch.edge_color, SurfaceColor::None);
        assert!(expected_faces * 3 * 40 < (256 << 20));
    }
}
