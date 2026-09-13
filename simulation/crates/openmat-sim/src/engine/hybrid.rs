use super::{RunError, Scratch};
use crate::{
    hybrid::{EventPlan, EventRecord, ResetMode},
    model::{Port, SampleTime},
    numeric::ReferenceKernel,
};
use std::sync::atomic::AtomicBool;

pub(super) struct EventState {
    pub kernel: ReferenceKernel,
    pub scratch: Scratch,
    pub signs: Vec<i8>,
    pub zero_edges: Vec<i32>,
    pub records: Vec<EventRecord>,
    pub total_records: usize,
    pub located: Vec<usize>,
    origins: Vec<Port>,
}
impl EventState {
    pub fn new(plan: &EventPlan) -> Self {
        Self {
            kernel: ReferenceKernel::new(plan.program.clone()),
            scratch: Scratch {
                inputs: vec![0.0; plan.program.input_count()],
                outputs: vec![0.0; plan.events.len()],
            },
            signs: vec![0; plan.events.len()],
            zero_edges: vec![0; plan.events.len()],
            records: vec![],
            total_records: 0,
            located: plan
                .events
                .iter()
                .enumerate()
                .filter_map(|(i, e)| e.locate.then_some(i))
                .collect(),
            origins: plan
                .events
                .iter()
                .map(|e| Port {
                    block: e.block.clone(),
                    port: e.surface.clone(),
                })
                .collect(),
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub fn evaluate(
        &mut self,
        time: f64,
        x: &[f64],
        q: &[f64],
        extra: &[f64],
        cancel: &AtomicBool,
    ) -> Result<(), RunError> {
        self.scratch
            .evaluate_with_extra(&mut self.kernel, &self.origins, time, x, q, extra, cancel)
    }
    pub fn initialize(&mut self) {
        self.signs = self.scratch.outputs.iter().map(|&v| sign(v)).collect();
    }
}

pub(super) fn sign(v: f64) -> i8 {
    if v > 0.0 {
        1
    } else if v < 0.0 {
        -1
    } else {
        0
    }
}

pub(super) struct EventCandidate {
    pub signs: Vec<i8>,
    pub zero_edges: Vec<i32>,
    pub records: Vec<EventRecord>,
}
impl EventCandidate {
    pub fn new(state: &EventState) -> Self {
        Self {
            signs: state.signs.clone(),
            zero_edges: state.zero_edges.clone(),
            records: vec![],
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub fn apply(
        &mut self,
        plan: &EventPlan,
        values: &[f64],
        x: &mut [f64],
        q: &mut [f64],
        hit_periods: &[f64],
        roots: &[i32],
        first_pass: bool,
    ) -> Result<bool, RunError> {
        let mut changed = false;
        for (i, event) in plan.events.iter().enumerate() {
            if matches!(event.sample_time, SampleTime::Constant)
                || matches!(event.sample_time, SampleTime::Discrete { period } if !hit_periods.contains(&period))
            {
                continue;
            }
            let old = self.signs[i];
            let mut next = sign(values[i]);
            let located = roots.get(i).copied().unwrap_or(0);
            let mut direction = if next > old && old <= 0 && next >= 0 {
                1
            } else if next < old && old >= 0 && next <= 0 {
                -1
            } else {
                0
            };
            if event.reset.as_ref().is_some_and(|r| r.discrete) {
                direction = if old <= 0 && next > 0 {
                    1
                } else if old > 0 && next <= 0 {
                    -1
                } else {
                    0
                };
            } else if old == 0 && self.zero_edges[i] == direction {
                direction = 0;
            }
            if located != 0 {
                direction = located;
                next = if located > 0 { 1 } else { -1 };
            }
            self.signs[i] = next;
            self.zero_edges[i] = if next == 0 {
                if direction == 0 {
                    if first_pass { 0 } else { self.zero_edges[i] }
                } else {
                    direction
                }
            } else {
                0
            };
            if direction == 0 {
                continue;
            }
            let resets = event.reset.as_ref().is_some_and(|r| match r.mode {
                ResetMode::Rising => direction > 0,
                ResetMode::Falling => direction < 0,
                ResetMode::Either => true,
            });
            let kind = if resets { "reset" } else { "crossing" };
            if self.records.iter().any(|r| {
                r.block == event.block && r.surface == event.surface && r.direction == direction
            }) {
                if resets {
                    return Err(RunError {
                        code: "event_iteration",
                        message:
                            "repeated reset at the same time requires unsupported event iteration"
                                .into(),
                        block: Some(event.block.clone()),
                        port: Some(event.surface.clone()),
                    });
                }
                continue;
            }
            if self.records.len() >= 512 {
                return Err(RunError::new(
                    "event_limit",
                    "event records exceeded 512 per accepted frame",
                ));
            }
            self.records.push(EventRecord {
                block: event.block.clone(),
                surface: event.surface.clone(),
                kind,
                direction,
            });
            if resets {
                let target = event.reset.as_ref().expect("reset target");
                let state = if target.discrete { &mut *q } else { &mut *x };
                state[target.offset..target.offset + target.initial.len()]
                    .copy_from_slice(&target.initial);
                changed = true;
            }
        }
        Ok(changed)
    }
}
