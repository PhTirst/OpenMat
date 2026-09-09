//! Built-in UI base classes are shipped with the runtime, including packaged builds.
use crate::ResolvedMatlabSource;
use std::path::{Path, PathBuf};

pub(crate) fn resolve(name: &str, generation: u64) -> Option<ResolvedMatlabSource> {
    let (relative, source) = match name {
        "matlab.ui.componentcontainer.ComponentContainer" => (
            "+matlab/+ui/+componentcontainer/ComponentContainer.m",
            include_str!("ui_sources/ComponentContainer.m"),
        ),
        "openmat.ui.Control" => (
            "+openmat/+ui/Control.m",
            include_str!("ui_sources/Control.m"),
        ),
        "openmat.ui.create" => ("+openmat/+ui/create.m", include_str!("ui_sources/create.m")),
        _ => return resolve_control(name, generation),
    };
    Some(ResolvedMatlabSource {
        path: PathBuf::from("openmat-builtin").join(relative),
        source: source.to_owned(),
        search_path_generation: generation,
    })
}

fn resolve_control(name: &str, generation: u64) -> Option<ResolvedMatlabSource> {
    let short = name.strip_prefix("openmat.ui.")?;
    let (properties, events) = match short {
        "Window" => ("Width = 840\nHeight = 560", "Startup"),
        "Button" => (
            "Text = 'Button'\nVariant = 'default'\nButtonPushedFcn = []",
            "Clicked",
        ),
        "TextField" | "TextArea" => (
            "Value = ''\nPlaceholder = ''\nValueChangedFcn = []",
            "ValueChanged",
        ),
        "NumericField" | "Slider" => (
            "Value = 1\nMin = 0\nMax = 100\nStep = 1\nValueChangedFcn = []",
            "ValueChanged",
        ),
        "CheckBox" => (
            "Text = ''\nValue = false\nValueChangedFcn = []",
            "ValueChanged",
        ),
        "RadioGroup" | "DropDown" => (
            "Value = ''\nItems = {}\nValueChangedFcn = []",
            "ValueChanged",
        ),
        "Table" => (
            "Columns = {}\nData = {}\nEditable = false\nCellEditCallback = []\nSelectionChangedFcn = []",
            "CellEdited\nSelectionChanged",
        ),
        "TabGroup" => (
            "SelectedIndex = 1\nSelectionChangedFcn = []",
            "SelectionChanged",
        ),
        "SplitPane" => ("Orientation = 'horizontal'", ""),
        "ScrollPanel" => ("ScrollDirection = 'vertical'", ""),
        "Label" => ("Text = ''\nFontSize = 13", ""),
        "Image" => ("Source = ''\nAlt = ''\nFit = 'contain'", ""),
        "PlotView" => ("FigureIndex = 1", ""),
        "ProgressBar" => ("Value = 0", ""),
        "StatusLamp" => ("Text = ''\nColor = '#247951'", ""),
        "AppBase" | "Panel" | "Tab" | "GridLayout" | "RowLayout" | "ColumnLayout"
        | "ComponentContainer" => ("", ""),
        _ => return None,
    };
    let base = if short == "AppBase" {
        "openmat.ui.Window"
    } else {
        "matlab.ui.componentcontainer.ComponentContainer"
    };
    let properties = if properties.is_empty() {
        String::new()
    } else {
        format!("properties\n{properties}\nend\n")
    };
    let events = if events.is_empty() {
        String::new()
    } else {
        format!("events\n{events}\nend\n")
    };
    let constructor = if short == "AppBase" {
        String::new()
    } else {
        let mode = match short {
            "GridLayout" => "grid",
            "RowLayout" | "SplitPane" => "row",
            _ => "column",
        };
        format!(
            "methods\nfunction obj = {short}()\nobj.Type = '{short}';\nobj.Layout.Mode = '{mode}';\nend\nend\n"
        )
    };
    Some(ResolvedMatlabSource {
        path: PathBuf::from("openmat-builtin").join(format!("+openmat/+ui/{short}.m")),
        source: format!("classdef {short} < {base}\n{properties}{events}{constructor}end\n"),
        search_path_generation: generation,
    })
}

/// Identifies an embedded source path returned by the source resolver.
#[must_use]
pub fn is_builtin_source(path: &Path) -> bool {
    !path.is_absolute() && path.starts_with("openmat-builtin")
}
