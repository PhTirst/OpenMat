use std::fmt::{self, Display, Formatter};
use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceLoss {
    pub reason: wgpu::DeviceLostReason,
    pub diagnostic: String,
}

impl Display for DeviceLoss {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        if self.diagnostic.is_empty() {
            write!(formatter, "{:?}", self.reason)
        } else {
            write!(formatter, "{:?} ({})", self.reason, self.diagnostic)
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RendererState {
    Idle,
    RequestingAdapter,
    AdapterUnavailable,
    CheckingLimits,
    InsufficientLimits,
    RequestingDevice,
    DeviceCreationFailed,
    CreatingPipelines,
    PipelineCreationFailed,
    Ready,
    DeviceLost(DeviceLoss),
}

#[derive(Clone, Debug)]
pub struct RendererLifecycle {
    state: Arc<Mutex<RendererState>>,
}

impl Default for RendererLifecycle {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(RendererState::Idle)),
        }
    }
}

impl RendererLifecycle {
    #[must_use]
    pub fn state(&self) -> RendererState {
        self.lock_state().clone()
    }

    pub(crate) fn transition(&self, state: RendererState) {
        *self.lock_state() = state;
    }

    pub(crate) fn mark_ready(&self) -> Result<(), DeviceLoss> {
        let mut state = self.lock_state();
        if let RendererState::DeviceLost(loss) = &*state {
            return Err(loss.clone());
        }
        *state = RendererState::Ready;
        Ok(())
    }

    fn lock_state(&self) -> MutexGuard<'_, RendererState> {
        match self.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SurfaceExtent {
    pub width: u32,
    pub height: u32,
}

impl SurfaceExtent {
    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.width == 0 || self.height == 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceState {
    Unconfigured,
    Suspended {
        extent: SurfaceExtent,
    },
    Configured {
        extent: SurfaceExtent,
        generation: u64,
    },
    Outdated {
        extent: SurfaceExtent,
        generation: u64,
    },
    Lost {
        extent: SurfaceExtent,
        generation: u64,
    },
    TimedOut {
        extent: SurfaceExtent,
        generation: u64,
    },
    Occluded {
        extent: SurfaceExtent,
        generation: u64,
    },
    ValidationFailed {
        extent: SurfaceExtent,
        generation: u64,
    },
    DeviceLost,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SurfaceEvent {
    Configure(SurfaceExtent),
    Acquired,
    Suboptimal,
    Outdated,
    Lost,
    Timeout,
    Occluded,
    ValidationFailed,
    DeviceLost,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SurfaceLifecycle {
    state: SurfaceState,
    generation: u64,
    last_nonzero_extent: Option<SurfaceExtent>,
}

impl Default for SurfaceLifecycle {
    fn default() -> Self {
        Self {
            state: SurfaceState::Unconfigured,
            generation: 0,
            last_nonzero_extent: None,
        }
    }
}

impl SurfaceLifecycle {
    pub(crate) fn state(self) -> SurfaceState {
        self.state
    }

    pub(crate) fn transition(&mut self, event: SurfaceEvent) {
        if matches!(
            self.state,
            SurfaceState::Lost { .. } | SurfaceState::DeviceLost
        ) && !matches!(event, SurfaceEvent::DeviceLost)
        {
            return;
        }
        match event {
            SurfaceEvent::Configure(extent) if extent.is_zero() => {
                self.state = SurfaceState::Suspended { extent };
            }
            SurfaceEvent::Configure(extent) => {
                self.generation = self.generation.saturating_add(1);
                self.last_nonzero_extent = Some(extent);
                self.state = SurfaceState::Configured {
                    extent,
                    generation: self.generation,
                };
            }
            SurfaceEvent::DeviceLost => self.state = SurfaceState::DeviceLost,
            event => {
                if let Some(extent) = self.last_nonzero_extent {
                    self.state = match event {
                        SurfaceEvent::Acquired => SurfaceState::Configured {
                            extent,
                            generation: self.generation,
                        },
                        SurfaceEvent::Suboptimal | SurfaceEvent::Outdated => {
                            SurfaceState::Outdated {
                                extent,
                                generation: self.generation,
                            }
                        }
                        SurfaceEvent::Lost => SurfaceState::Lost {
                            extent,
                            generation: self.generation,
                        },
                        SurfaceEvent::Timeout => SurfaceState::TimedOut {
                            extent,
                            generation: self.generation,
                        },
                        SurfaceEvent::Occluded => SurfaceState::Occluded {
                            extent,
                            generation: self.generation,
                        },
                        SurfaceEvent::ValidationFailed => SurfaceState::ValidationFailed {
                            extent,
                            generation: self.generation,
                        },
                        SurfaceEvent::Configure(_) | SurfaceEvent::DeviceLost => self.state,
                    };
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renderer_lifecycle_records_typed_terminal_states() {
        let lifecycle = RendererLifecycle::default();
        lifecycle.transition(RendererState::RequestingAdapter);
        lifecycle.transition(RendererState::AdapterUnavailable);
        assert_eq!(lifecycle.state(), RendererState::AdapterUnavailable);
        lifecycle.transition(RendererState::CheckingLimits);
        lifecycle.transition(RendererState::InsufficientLimits);
        assert_eq!(lifecycle.state(), RendererState::InsufficientLimits);
        lifecycle.transition(RendererState::RequestingDevice);
        lifecycle.transition(RendererState::DeviceCreationFailed);
        assert_eq!(lifecycle.state(), RendererState::DeviceCreationFailed);

        lifecycle.transition(RendererState::DeviceLost(DeviceLoss {
            reason: wgpu::DeviceLostReason::Unknown,
            diagnostic: "driver reset".to_owned(),
        }));
        assert!(matches!(
            lifecycle.state(),
            RendererState::DeviceLost(DeviceLoss {
                reason: wgpu::DeviceLostReason::Unknown,
                ..
            })
        ));
    }

    #[test]
    fn surface_resize_and_recovery_transitions_are_explicit() {
        let mut lifecycle = SurfaceLifecycle::default();
        lifecycle.transition(SurfaceEvent::Configure(SurfaceExtent {
            width: 640,
            height: 480,
        }));
        assert!(matches!(
            lifecycle.state(),
            SurfaceState::Configured { generation: 1, .. }
        ));

        lifecycle.transition(SurfaceEvent::Timeout);
        assert!(matches!(
            lifecycle.state(),
            SurfaceState::TimedOut { generation: 1, .. }
        ));
        lifecycle.transition(SurfaceEvent::Acquired);
        assert!(matches!(
            lifecycle.state(),
            SurfaceState::Configured { generation: 1, .. }
        ));
        lifecycle.transition(SurfaceEvent::Outdated);
        assert!(matches!(
            lifecycle.state(),
            SurfaceState::Outdated { generation: 1, .. }
        ));
        lifecycle.transition(SurfaceEvent::Configure(SurfaceExtent {
            width: 800,
            height: 600,
        }));
        assert!(matches!(
            lifecycle.state(),
            SurfaceState::Configured { generation: 2, .. }
        ));
        lifecycle.transition(SurfaceEvent::Lost);
        lifecycle.transition(SurfaceEvent::Configure(SurfaceExtent {
            width: 1024,
            height: 768,
        }));
        assert!(matches!(
            lifecycle.state(),
            SurfaceState::Lost { generation: 2, .. }
        ));

        let mut lifecycle = SurfaceLifecycle::default();
        lifecycle.transition(SurfaceEvent::Configure(SurfaceExtent {
            width: 0,
            height: 600,
        }));
        assert!(matches!(lifecycle.state(), SurfaceState::Suspended { .. }));
    }
}
