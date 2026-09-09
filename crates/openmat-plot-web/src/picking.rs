use std::collections::HashMap;

use openmat_plot_mir::{MirCommand, PickingId, PlotFrame};
use openmat_plot_protocol::{FigureSnapshot, GraphicsObject};

use crate::scene::ordered_axes_drawables;
use crate::{PlotWebError, PlotWebErrorKind};

/// Match each pickable primitive to the same drawable order used by scene lowering.
pub(crate) fn assign_picking_ids(
    snapshot: &FigureSnapshot,
    axes_id: &str,
    frame: &mut PlotFrame,
    next_id: &mut u64,
) -> Result<HashMap<u64, String>, PlotWebError> {
    let object_by_id: HashMap<_, _> = snapshot
        .objects
        .iter()
        .map(|object| (object.fields().id.as_str(), object))
        .collect();
    let Some(GraphicsObject::Axes2d { fields, .. }) = object_by_id.get(axes_id).copied() else {
        return Ok(HashMap::new());
    };
    let drawables = ordered_axes_drawables(&fields.children, &object_by_id)?;
    let mut mapping = HashMap::new();
    for operation in &mut frame.operations {
        if !matches!(
            operation.command,
            MirCommand::StrokeMesh(_) | MirCommand::MarkerBatch(_)
        ) {
            continue;
        }
        let Some(object) = operation
            .order
            .0
            .checked_sub(1)
            .and_then(|index| usize::try_from(index).ok())
            .and_then(|index| drawables.get(index))
        else {
            continue;
        };
        let pickable = match object {
            GraphicsObject::LineSeries { properties, .. } => {
                properties.visible && properties.z_data.as_ref().is_none()
            }
            GraphicsObject::ScatterSeries { properties, .. } => {
                properties.visible && properties.z_data.as_ref().is_none()
            }
            _ => false,
        };
        if !pickable {
            continue;
        }
        let picking_id = PickingId::from_u64(*next_id).map_err(|_| exhausted_ids())?;
        operation.picking_id = Some(picking_id);
        mapping.insert(*next_id, object.fields().id.clone());
        *next_id = next_id.checked_add(1).ok_or_else(exhausted_ids)?;
    }
    Ok(mapping)
}

fn exhausted_ids() -> PlotWebError {
    PlotWebError::new(
        PlotWebErrorKind::InvalidScene,
        "Plot picking identifier space was exhausted.",
    )
}
