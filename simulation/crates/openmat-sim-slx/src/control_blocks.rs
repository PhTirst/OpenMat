use crate::{Block, Document, Issue, ParameterArray, Parameters, blocks, lower::value};
use openmat_sim::model::{Block as RuntimeBlock, BlockKind};

pub(crate) struct Node<'a> {
    pub source: &'a Block,
    pub kind: Kind,
    pub inputs: usize,
    pub outputs: usize,
}

pub(crate) enum Kind {
    Legacy(RuntimeBlock),
    Subsystem,
    Inport(usize),
    Outport(usize),
    Ground,
    Terminator,
    Bias(Vec<f64>),
    Product(Vec<char>),
    Sine {
        amplitude: Vec<f64>,
        bias: Vec<f64>,
        frequency: Vec<f64>,
        phase: Vec<f64>,
    },
    StateSpace {
        a: ParameterArray,
        b: ParameterArray,
        c: ParameterArray,
        d: ParameterArray,
        initial: Vec<f64>,
    },
    Mux(Vec<Option<usize>>),
    Demux(Vec<Option<usize>>),
    Step {
        time: f64,
        before: Vec<f64>,
        after: Vec<f64>,
    },
}

impl Node<'_> {
    pub fn executable(&self) -> bool {
        !matches!(
            self.kind,
            Kind::Subsystem | Kind::Inport(_) | Kind::Outport(_) | Kind::Terminator
        )
    }
    pub fn input_name(&self, port: usize) -> String {
        if matches!(self.kind, Kind::Product(_) | Kind::Mux(_)) || self.source.block_type == "Sum" {
            format!("in{port}")
        } else {
            "in".into()
        }
    }
    pub fn output_name(&self, port: usize) -> String {
        if matches!(self.kind, Kind::Demux(_)) {
            format!("out{port}")
        } else {
            "out".into()
        }
    }
}

pub(crate) fn prepare<'a>(
    document: &'a Document,
    p: &Parameters,
) -> Result<Vec<Node<'a>>, Vec<Issue>> {
    let mut result = Vec::new();
    let mut issues = Vec::new();
    for system in &document.systems {
        for block in &system.blocks {
            let next =
                prepare_block(block, document, p, system.parent_block.is_none()).and_then(|node| {
                    check(block, &node)?;
                    Ok(node)
                });
            match next {
                Ok(node) => result.push(node),
                Err(e) => issues.push(e.at(block, None)),
            }
        }
    }
    if issues.is_empty() {
        Ok(result)
    } else {
        Err(issues)
    }
}

fn array(block: &Block, p: &Parameters, key: &str, default: &str) -> Result<ParameterArray, Issue> {
    p.evaluate(value(&block.properties, key, default))
        .map_err(|e| e.at(block, Some(key)))
}
fn vector(block: &Block, p: &Parameters, key: &str, default: &str) -> Result<Vec<f64>, Issue> {
    array(block, p, key, default)?
        .vector()
        .map_err(|e| e.at(block, Some(key)))
}
fn scalar(block: &Block, p: &Parameters, key: &str, default: &str) -> Result<f64, Issue> {
    array(block, p, key, default)?
        .scalar()
        .map_err(|e| e.at(block, Some(key)))
}

#[allow(clippy::too_many_lines, clippy::many_single_char_names)] // A/B/C/D retain state-space notation.
fn prepare_block<'a>(
    block: &'a Block,
    document: &Document,
    p: &Parameters,
    root: bool,
) -> Result<Node<'a>, Issue> {
    if ["Sin", "Step"].contains(&block.block_type.as_str())
        && scalar(block, p, "SampleTime", "-1")? != 0.0
    {
        return Err(Issue::new("sample_time", "control-v1 Sine Wave and Step require explicit continuous SampleTime=0; inherited and discrete source rates need rate inference").at(block, Some("SampleTime")));
    }
    let numeric: &[(&str, &str)] = match block.block_type.as_str() {
        "Constant" => &[("Value", "1")],
        "Gain" => &[("Gain", "1")],
        "Integrator" | "UnitDelay" => &[("InitialCondition", "0")],
        _ => &[],
    };
    let (kind, inputs, outputs) = match block.block_type.as_str() {
        "Constant" | "Gain" | "Sum" | "Integrator" | "UnitDelay" | "Scope" => {
            let mut resolved = block.clone();
            for &(key, default) in numeric {
                resolved
                    .properties
                    .insert(key.into(), array(block, p, key, default)?.text());
            }
            if block.block_type == "UnitDelay" {
                resolved.properties.insert(
                    "SampleTime".into(),
                    scalar(block, p, "SampleTime", "-1")?.to_string(),
                );
            }
            let runtime = blocks::translate(&resolved)?;
            let inputs = match &runtime.kind {
                BlockKind::Constant { .. } => 0,
                BlockKind::Sum { signs } => signs.len(),
                _ => 1,
            };
            let outputs = usize::from(!matches!(runtime.kind, BlockKind::Scope));
            (Kind::Legacy(runtime), inputs, outputs)
        }
        "SubSystem" => {
            let children: Vec<_> = document
                .systems
                .iter()
                .filter(|s| s.parent_block.as_deref() == Some(&block.sid))
                .collect();
            let [system] = children.as_slice() else {
                return Err(Issue::new(
                    "subsystem",
                    "virtual subsystem must contain exactly one System",
                ));
            };
            let count = |ty: &str| -> Result<usize, Issue> {
                let mut numbers = system
                    .blocks
                    .iter()
                    .filter(|b| b.block_type == ty)
                    .map(|b| port_number(b, p))
                    .collect::<Result<Vec<_>, _>>()?;
                numbers.sort_unstable();
                if numbers.iter().copied().ne(1..=numbers.len()) {
                    return Err(Issue::new(
                        "subsystem_port",
                        "subsystem ports must have unique contiguous numbers starting at 1",
                    ));
                }
                Ok(numbers.len())
            };
            (Kind::Subsystem, count("Inport")?, count("Outport")?)
        }
        "Inport" if !root => (Kind::Inport(port_number(block, p)?), 0, 1),
        "Outport" if !root => (Kind::Outport(port_number(block, p)?), 1, 0),
        "Inport" | "Outport" => {
            return Err(Issue::new(
                "root_port",
                "root ports require external input/output configuration; use Scope observers in control-v1",
            ));
        }
        "Ground" => (Kind::Ground, 0, 1),
        "Terminator" => (Kind::Terminator, 1, 0),
        "Bias" => (Kind::Bias(vector(block, p, "Bias", "0")?), 1, 1),
        "Product" => {
            let text = value(&block.properties, "Inputs", "2");
            let signs = if let Ok(n) = text.parse::<usize>() {
                if !(2..=64).contains(&n) {
                    return Err(Issue::new(
                        "product_inputs",
                        "Product needs 2..64 inputs; single-input reduction is unsupported",
                    ));
                }
                vec!['*'; n]
            } else {
                let signs: Vec<_> = text.chars().collect();
                if !(2..=64).contains(&signs.len()) || signs.iter().any(|c| !['*', '/'].contains(c))
                {
                    return Err(Issue::new(
                        "product_inputs",
                        "invalid elementwise Product inputs",
                    ));
                }
                signs
            };
            let n = signs.len();
            (Kind::Product(signs), n, 1)
        }
        "Sin" => (
            Kind::Sine {
                amplitude: vector(block, p, "Amplitude", "1")?,
                bias: vector(block, p, "Bias", "0")?,
                frequency: vector(block, p, "Frequency", "1")?,
                phase: vector(block, p, "Phase", "0")?,
            },
            0,
            1,
        ),
        "StateSpace" => {
            let a = array(block, p, "A", "1")?;
            let b = array(block, p, "B", "1")?;
            let c = array(block, p, "C", "1")?;
            let d = array(block, p, "D", "1")?;
            let n = a.rows;
            if n == 0
                || n > 32
                || a.columns != n
                || b.rows != n
                || b.columns == 0
                || b.columns > 32
                || c.columns != n
                || c.rows == 0
                || c.rows > 32
                || d.rows != c.rows
                || d.columns != b.columns
            {
                return Err(Issue::new(
                    "state_space_shape",
                    "State-Space requires A(n,n), B(n,m), C(r,n), D(r,m), with n,m,r in 1..32",
                ));
            }
            let initial = expand(vector(block, p, "InitialCondition", "0")?, n)?;
            (
                Kind::StateSpace {
                    a,
                    b,
                    c,
                    d,
                    initial,
                },
                1,
                1,
            )
        }
        "TransferFcn" => (transfer(block, p)?, 1, 1),
        "Mux" => {
            let sizes = port_sizes(block, p, "Inputs", "2")?;
            let n = sizes.len();
            (Kind::Mux(sizes), n, 1)
        }
        "Demux" => {
            let sizes = port_sizes(block, p, "Outputs", "4")?;
            let n = sizes.len();
            (Kind::Demux(sizes), 1, n)
        }
        "Step" => {
            let before = vector(block, p, "Before", "0")?;
            let after = vector(block, p, "After", "1")?;
            let n = before.len().max(after.len());
            (
                Kind::Step {
                    time: scalar(block, p, "Time", "1")?,
                    before: expand(before, n)?,
                    after: expand(after, n)?,
                },
                0,
                1,
            )
        }
        other => {
            return Err(Issue::new(
                "unsupported_block",
                format!("unsupported control-v1 block type {other}"),
            ));
        }
    };
    Ok(Node {
        source: block,
        kind,
        inputs,
        outputs,
    })
}

pub(crate) fn expand(mut values: Vec<f64>, width: usize) -> Result<Vec<f64>, Issue> {
    if width == 0 || width > 4096 || (values.len() != 1 && values.len() != width) {
        return Err(Issue::new(
            "signal_width",
            "expected scalar expansion or matching fixed vector widths",
        ));
    }
    if values.len() == 1 {
        values.resize(width, values[0]);
    }
    Ok(values)
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn count(value: f64) -> Result<usize, Issue> {
    if !(1.0..=4096.0).contains(&value) || value.fract() != 0.0 {
        return Err(Issue::new(
            "port_count",
            "port counts and widths must be positive bounded integers",
        ));
    }
    Ok(value as usize)
}
fn port_number(block: &Block, p: &Parameters) -> Result<usize, Issue> {
    let n = count(scalar(block, p, "Port", "1")?)?;
    if n > 64 {
        return Err(Issue::new(
            "port_count",
            "at most 64 subsystem ports are supported",
        ));
    }
    Ok(n)
}
#[allow(clippy::float_cmp)] // -1 is an exact syntax sentinel, not an approximate measurement.
fn port_sizes(
    block: &Block,
    p: &Parameters,
    key: &str,
    default: &str,
) -> Result<Vec<Option<usize>>, Issue> {
    let values = vector(block, p, key, default)?;
    if values.len() == 1 {
        let n = count(values[0])?;
        if n > 64 {
            return Err(Issue::new(
                "port_count",
                "at most 64 routing ports are supported",
            ));
        }
        Ok(vec![None; n])
    } else {
        if values.len() > 64 {
            return Err(Issue::new(
                "port_count",
                "at most 64 routing ports are supported",
            ));
        }
        values
            .into_iter()
            .map(|v| {
                if v == -1.0 {
                    Ok(None)
                } else {
                    count(v).map(Some)
                }
            })
            .collect()
    }
}

fn transfer(block: &Block, p: &Parameters) -> Result<Kind, Issue> {
    let numerator = vector(block, p, "Numerator", "[1]")?;
    let denominator = vector(block, p, "Denominator", "[1 2 1]")?;
    if denominator.len() > 33 || denominator[0] == 0.0 || numerator.len() > denominator.len() {
        return Err(Issue::new(
            "transfer_function",
            "Transfer Fcn requires a proper polynomial ratio with 0..32 states and a nonzero leading denominator",
        ));
    }
    let n = denominator.len() - 1;
    let a: Vec<_> = denominator.iter().map(|v| v / denominator[0]).collect();
    let mut b = vec![0.0; denominator.len()];
    for (dst, src) in b
        .iter_mut()
        .skip(denominator.len() - numerator.len())
        .zip(numerator)
    {
        *dst = src / denominator[0];
    }
    let mut av = vec![0.0; n * n];
    let mut bv = vec![0.0; n];
    let mut cv = vec![0.0; n];
    if n > 0 {
        bv[0] = 1.0;
    }
    for i in 0..n {
        av[i * n] = -a[i + 1];
        cv[i] = b[i + 1] - b[0] * a[i + 1];
        if i + 1 < n {
            av[i + 1 + i * n] = 1.0;
        }
    }
    if av.iter().chain(&cv).chain(&b).any(|v| !v.is_finite()) {
        return Err(Issue::new(
            "transfer_function",
            "normalized Transfer Fcn coefficients must remain finite",
        ));
    }
    Ok(Kind::StateSpace {
        a: ParameterArray {
            rows: n,
            columns: n,
            values: av,
        },
        b: ParameterArray {
            rows: n,
            columns: 1,
            values: bv,
        },
        c: ParameterArray {
            rows: 1,
            columns: n,
            values: cv,
        },
        d: ParameterArray {
            rows: 1,
            columns: 1,
            values: vec![b[0]],
        },
        initial: vec![0.0; n],
    })
}

fn check(block: &Block, node: &Node<'_>) -> Result<(), Issue> {
    if matches!(node.kind, Kind::Legacy(_)) {
        return Ok(());
    }
    if block
        .attributes
        .keys()
        .any(|k| !["BlockType", "Name", "SID"].contains(&k.as_str()))
        || block.extra_elements.iter().any(|e| {
            !(["PortCounts", "PortProperties"].contains(&e.as_str())
                || e == "System" && matches!(node.kind, Kind::Subsystem))
        })
    {
        return Err(Issue::new(
            "block_feature",
            "mask, library link or additional execution structure is unsupported",
        ));
    }
    for port in &block.port_properties {
        if !port.extra_elements.is_empty()
            || port
                .attributes
                .keys()
                .any(|k| !["Type", "Index"].contains(&k.as_str()))
            || port.properties.keys().any(|k| k != "Name")
        {
            return Err(Issue::new(
                "port_feature",
                "only display names are supported in port properties",
            ));
        }
    }
    for (key, text) in &block.properties {
        if !blocks::parameter_supported(block, key, text.trim())
            && !parameter_supported(node, key, text.trim())
        {
            return Err(Issue::new(
                "unsupported_parameter",
                format!("unsupported {} parameter {key}={text}", block.block_type),
            )
            .at(block, Some(key)));
        }
    }
    for (port, count) in &block.port_counts {
        let expected = match port.as_str() {
            "in" => node.inputs,
            "out" => node.outputs,
            _ => {
                return Err(Issue::new(
                    "port_kind",
                    "enable, trigger and state ports need separate execution semantics",
                ));
            }
        };
        if count.parse::<usize>().ok() != Some(expected) {
            return Err(Issue::new(
                "port_count",
                "port count disagrees with the supported block configuration",
            ));
        }
    }
    Ok(())
}

fn parameter_supported(node: &Node<'_>, key: &str, text: &str) -> bool {
    match &node.kind {
        Kind::Subsystem => match key {
            "TreatAsAtomicUnit" => text == "off",
            "ContentPreviewEnabled" => ["on", "off"].contains(&text),
            _ => false,
        },
        Kind::Inport(_) | Kind::Outport(_) => match key {
            "Port" => true,
            "PortDimensions" | "SampleTime" => text == "-1",
            "OutDataTypeStr" => ["Inherit: auto", "double"].contains(&text),
            "SignalType" => text == "auto" || text == "real",
            "VarSizeSig" => text == "Inherit",
            "Interpolate" => ["on", "off"].contains(&text),
            "OutputWhenDisabled" => text == "held",
            "InitialOutput" => text == "[]",
            _ => false,
        },
        Kind::Bias(_) => key == "Bias" || (key == "SampleTime" && text == "-1"),
        Kind::Product(_) => match key {
            "Inputs" | "RndMeth" | "SaturateOnIntegerOverflow" => true,
            "Multiplication" => text == "Element-wise(.*)",
            "SampleTime" => text == "-1",
            "OutDataTypeStr" => ["double", "Inherit: Same as input"].contains(&text),
            _ => false,
        },
        Kind::Sine { .. } => match key {
            // SampleTime was evaluated and required to equal zero during preparation.
            "Amplitude" | "Bias" | "Frequency" | "Phase" | "SampleTime" => true,
            "SineType" => text == "Time based",
            "TimeSource" => text == "Use simulation time",
            "VectorParams1D" => text == "on",
            _ => false,
        },
        Kind::StateSpace { .. } => match key {
            "A" | "B" | "C" | "D" | "InitialCondition" => node.source.block_type == "StateSpace",
            "Numerator" | "Denominator" => node.source.block_type == "TransferFcn",
            "AbsoluteTolerance" => text == "auto",
            _ => false,
        },
        Kind::Mux(_) => {
            key == "Inputs"
                || (key == "DisplayOption" && ["none", "bar", "signals"].contains(&text))
        }
        Kind::Demux(_) => {
            key == "Outputs"
                || (key == "DisplayOption" && ["none", "bar", "signals"].contains(&text))
        }
        Kind::Step { .. } => match key {
            // SampleTime was evaluated and required to equal zero during preparation.
            "Time" | "Before" | "After" | "SampleTime" => true,
            "VectorParams1D" => text == "on",
            "ZeroCross" => ["on", "off"].contains(&text),
            _ => false,
        },
        _ => false,
    }
}
