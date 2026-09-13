//! Native virtual hierarchy and standard real-valued authoring blocks (schema 7).
use crate::{ModelError, model::BlockKind};
use serde::{Deserialize, Serialize};

mod hierarchy;
pub(crate) use hierarchy::flatten;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimeSeries {
    pub times: Vec<f64>,
    pub values: Vec<Vec<f64>>,
}

impl TimeSeries {
    /// # Errors
    /// Requires an ordered finite table with a bounded fixed signal width.
    pub fn validate(&self) -> Result<(), ModelError> {
        let width = self.values.first().map_or(0, Vec::len);
        if self.times.is_empty()
            || self.times.len() > 4096
            || self.times.len() != self.values.len()
            || !(1..=64).contains(&width)
            || self.times.len() * width > 65_536
            || self.times.iter().any(|t| !t.is_finite() || *t < 0.0)
            || self.times.windows(2).any(|w| w[0] >= w[1])
            || self
                .values
                .iter()
                .any(|r| r.len() != width || r.iter().any(|v| !v.is_finite()))
        {
            return Err(ModelError::new(
                "input_data",
                "input needs 1..4096 increasing nonnegative times, 1..64 finite channels and at most 65536 values",
            ));
        }
        for (ts, rows) in self.times.windows(2).zip(self.values.windows(2)) {
            for (&a, &b) in rows[0].iter().zip(&rows[1]) {
                if !((b - a) / (ts[1] - ts[0])).is_finite() {
                    return Err(ModelError::new(
                        "input_data",
                        "input interpolation slope is not finite",
                    ));
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum StandardOp {
    Product {
        operations: String,
    },
    Mux {
        inputs: usize,
    },
    Demux {
        widths: Vec<usize>,
    },
    StateSpace {
        a: Vec<Vec<f64>>,
        b: Vec<Vec<f64>>,
        c: Vec<Vec<f64>>,
        d: Vec<Vec<f64>>,
        initial: Vec<f64>,
    },
    TransferFcn {
        numerator: Vec<f64>,
        denominator: Vec<f64>,
    },
}

pub(crate) struct Realization {
    pub a: Vec<Vec<f64>>,
    pub b: Vec<Vec<f64>>,
    pub c: Vec<Vec<f64>>,
    pub d: Vec<Vec<f64>>,
    pub initial: Vec<f64>,
}

impl StandardOp {
    pub(crate) fn input_names(&self) -> Vec<String> {
        match self {
            Self::Product { operations } => {
                (0..operations.len()).map(|i| format!("in{i}")).collect()
            }
            Self::Mux { inputs } => (0..*inputs).map(|i| format!("in{i}")).collect(),
            _ => vec!["in".into()],
        }
    }
    pub(crate) fn output_names(&self) -> Vec<String> {
        match self {
            Self::Demux { widths } => (0..widths.len()).map(|i| format!("out{i}")).collect(),
            _ => vec!["out".into()],
        }
    }
    /// # Errors
    /// Rejects unsupported shapes or non-finite parameters before allocation/lowering.
    pub fn validate(&self) -> Result<usize, ModelError> {
        let bad = || {
            ModelError::new(
                "standard_parameter",
                "invalid standard block parameters or dimensions",
            )
        };
        let matrix = |m: &[Vec<f64>], rows: usize, cols: usize| {
            m.len() == rows
                && m.iter()
                    .all(|r| r.len() == cols && r.iter().all(|v| v.is_finite()))
        };
        match self {
            Self::Product { operations } => {
                if !(2..=64).contains(&operations.len())
                    || !operations.bytes().all(|c| c == b'*' || c == b'/')
                {
                    return Err(bad());
                }
                Ok(operations.len())
            }
            Self::Mux { inputs } => {
                if !(2..=64).contains(inputs) {
                    return Err(bad());
                }
                Ok(*inputs)
            }
            Self::Demux { widths } => {
                if !(2..=64).contains(&widths.len())
                    || widths.iter().any(|n| !(1..=4096).contains(n))
                    || widths.iter().sum::<usize>() > 4096
                {
                    return Err(bad());
                }
                Ok(widths.len())
            }
            Self::StateSpace {
                a,
                b,
                c,
                d,
                initial,
            } => {
                let n = a.len();
                let m = b.first().map_or(0, Vec::len);
                let r = c.len();
                if [n, m, r].iter().any(|n| !(1..=32).contains(n))
                    || !matrix(a, n, n)
                    || !matrix(b, n, m)
                    || !matrix(c, r, n)
                    || !matrix(d, r, m)
                    || initial.len() != n
                    || initial.iter().any(|v| !v.is_finite())
                {
                    return Err(bad());
                }
                Ok(n * n + n * m + r * n + r * m + n)
            }
            Self::TransferFcn {
                numerator,
                denominator,
            } => {
                if denominator.is_empty()
                    || denominator.len() > 33
                    || numerator.is_empty()
                    || numerator.len() > denominator.len()
                    || denominator[0] == 0.0
                    || numerator
                        .iter()
                        .chain(denominator)
                        .any(|v| !v.is_finite() || !(v / denominator[0]).is_finite())
                {
                    return Err(bad());
                }
                if self
                    .realization()
                    .is_some_and(|r| r.c.iter().flatten().any(|v| !v.is_finite()))
                {
                    return Err(bad());
                }
                Ok(numerator.len() + denominator.len())
            }
        }
    }
    pub(crate) fn realization(&self) -> Option<Realization> {
        match self {
            Self::StateSpace {
                a,
                b,
                c,
                d,
                initial,
            } => Some(Realization {
                a: a.clone(),
                b: b.clone(),
                c: c.clone(),
                d: d.clone(),
                initial: initial.clone(),
            }),
            Self::TransferFcn {
                numerator,
                denominator,
            } => {
                let n = denominator.len() - 1;
                let a0 = denominator[0];
                let den: Vec<_> = denominator.iter().map(|v| v / a0).collect();
                let mut num = vec![0.0; denominator.len()];
                for (to, from) in num[denominator.len() - numerator.len()..]
                    .iter_mut()
                    .zip(numerator)
                {
                    *to = from / a0;
                }
                let mut a = vec![vec![0.0; n]; n];
                let mut b = vec![vec![0.0; 1]; n];
                if n > 0 {
                    for (to, from) in a[0].iter_mut().zip(&den[1..]) {
                        *to = -*from;
                    }
                    b[0][0] = 1.0;
                    for (i, row) in a.iter_mut().enumerate().skip(1) {
                        row[i - 1] = 1.0;
                    }
                }
                Some(Realization {
                    a,
                    b,
                    c: vec![(1..=n).map(|i| num[i] - num[0] * den[i]).collect()],
                    d: vec![vec![num[0]]],
                    initial: vec![0.0; n],
                })
            }
            _ => None,
        }
    }
}

pub(crate) fn output_names(kind: &BlockKind) -> Vec<String> {
    match kind {
        BlockKind::Subsystem { outputs, .. } => (1..=*outputs).map(|i| format!("out{i}")).collect(),
        BlockKind::Standard { operation } => operation.output_names(),
        BlockKind::Scope | BlockKind::Outport { .. } => vec![],
        _ => vec!["out".into()],
    }
}
