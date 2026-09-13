//! Optional CVODE integrator. The `OpenMat` scheduler owns all public state.
mod ffi;

use ffi::{Api, Handle, check};
use openmat_sim::RunError;
use openmat_sim::solver::{ContinuousSolver, OdeEvents, OdeRhs, OdeStep, SolverStats};
use serde::{Deserialize, Serialize};
use std::marker::PhantomData;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::ptr;
use std::rc::Rc;
use std::sync::atomic::Ordering;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Method {
    Adams,
    #[default]
    Bdf,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Options {
    pub method: Method,
    pub relative_tolerance: f64,
    pub absolute_tolerance: f64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            method: Method::Bdf,
            relative_tolerance: 1e-6,
            absolute_tolerance: 1e-9,
        }
    }
}

pub struct Cvode {
    api: Api,
    context: Handle,
    vector: Handle,
    matrix: Handle,
    linear: Handle,
    memory: Handle,
    width: usize,
    options: Options,
    initialized: bool,
    root_count: usize,
    stats: SolverStats,
    _thread_bound: PhantomData<Rc<()>>,
}

impl Cvode {
    /// Load the pinned host-configured runtime. No code is loaded from a model.
    /// # Errors
    /// Rejects unavailable/unsupported runtimes, invalid tolerances and dimensions.
    pub fn new(directory: &Path, width: usize, options: Options) -> Result<Self, RunError> {
        if !(1..=2048).contains(&width)
            || !options.relative_tolerance.is_finite()
            || !options.absolute_tolerance.is_finite()
            || !(1e-14..=0.1).contains(&options.relative_tolerance)
            || options.absolute_tolerance <= 0.0
        {
            return Err(RunError::new(
                "solver_configuration",
                "CVODE requires 1..2048 continuous states, relative tolerance 1e-14..0.1 and finite positive absolute tolerance",
            ));
        }
        let api = Api::load(directory)?;
        let mut solver = Self {
            api,
            context: ptr::null_mut(),
            vector: ptr::null_mut(),
            matrix: ptr::null_mut(),
            linear: ptr::null_mut(),
            memory: ptr::null_mut(),
            width,
            options,
            initialized: false,
            root_count: 0,
            stats: SolverStats::default(),
            _thread_bound: PhantomData,
        };
        // SAFETY: Handles are individually owned, recorded immediately, and
        // released by Drop on both initialization failures and success.
        unsafe {
            check(
                (solver.api.context_create)(0, &raw mut solver.context),
                "create context",
            )?;
            require(solver.context, "context")?;
            check(
                (solver.api.clear_handlers)(solver.context),
                "configure error reporting",
            )?;
            let n = i64::try_from(width)
                .map_err(|_| RunError::new("solver_configuration", "state dimension overflow"))?;
            solver.vector = (solver.api.vector_new)(n, solver.context);
            require(solver.vector, "vector")?;
            solver.matrix = (solver.api.matrix_new)(n, n, solver.context);
            require(solver.matrix, "dense matrix")?;
            solver.linear = (solver.api.linear_new)(solver.vector, solver.matrix, solver.context);
            require(solver.linear, "linear solver")?;
            solver.memory = (solver.api.create)(
                if solver.options.method == Method::Adams {
                    1
                } else {
                    2
                },
                solver.context,
            );
            require(solver.memory, "CVODE memory")?;
        }
        Ok(solver)
    }

    fn native_counts(&self) -> (u64, u64) {
        if !self.initialized {
            return (0, 0);
        }
        let (mut failures, mut iterations) = (0, 0);
        // SAFETY: Live initialized CVODE object, correctly sized C long outputs.
        unsafe {
            (self.api.failures)(self.memory, &raw mut failures);
            (self.api.iterations)(self.memory, &raw mut iterations);
        }
        (
            u64::try_from(failures).unwrap_or(0),
            u64::try_from(iterations).unwrap_or(0),
        )
    }
}

impl Cvode {
    #[allow(clippy::too_many_lines, clippy::needless_pass_by_value)] // Keep borrowed callback attachment, native advance, and detachment in one audited transaction.
    fn advance_events(
        &mut self,
        step: OdeStep<'_>,
        rhs: &mut OdeRhs<'_>,
        mut events: Option<OdeEvents<'_, '_>>,
    ) -> Result<f64, RunError> {
        if events
            .as_ref()
            .is_some_and(|e| e.count > 256 || e.count != e.found.len())
        {
            return Err(RunError::new(
                "solver_events",
                "invalid CVODE event buffers",
            ));
        }
        if let Some(e) = &mut events {
            e.found.fill(0);
        }
        if step.state.len() != self.width
            || step.candidate.len() != self.width
            || !step.time.is_finite()
            || !step.boundary.is_finite()
            || step.boundary <= step.time
            || !step.max_step.is_finite()
            || step.max_step <= 0.0
            || step.state.iter().any(|v| !v.is_finite())
        {
            return Err(RunError::new(
                "solver_configuration",
                "invalid CVODE step buffers or time interval",
            ));
        }
        if step.cancel.load(Ordering::Relaxed) {
            return Err(RunError::new("cancelled", "simulation was cancelled"));
        }
        // SAFETY: Serial vectors have exactly width double elements. Native
        // handles remain owned throughout the synchronous call and callback.
        unsafe {
            let data = (self.api.vector_data)(self.vector);
            require(data.cast(), "vector data")?;
            if !self.initialized || step.reinitialize {
                ptr::copy_nonoverlapping(step.state.as_ptr(), data, self.width);
                if self.initialized {
                    let (failures, iterations) = self.native_counts();
                    self.stats.error_test_failures += failures;
                    self.stats.nonlinear_iterations += iterations;
                    check(
                        (self.api.reinit)(self.memory, step.time, self.vector),
                        "reinitialize after sample hit",
                    )?;
                    self.stats.reinitializations += 1;
                } else {
                    check(
                        (self.api.init)(self.memory, callback, step.time, self.vector),
                        "initialize CVODE",
                    )?;
                    self.initialized = true;
                    check(
                        (self.api.set_linear)(self.memory, self.linear, self.matrix),
                        "attach dense linear solver",
                    )?;
                    check(
                        (self.api.tolerances)(
                            self.memory,
                            self.options.relative_tolerance,
                            self.options.absolute_tolerance,
                        ),
                        "set tolerances",
                    )?;
                }
            }
            let root_count = events.as_ref().map_or(0, |e| e.count);
            if root_count != self.root_count {
                check(
                    (self.api.root_init)(
                        self.memory,
                        i32::try_from(root_count).expect("bounded roots"),
                        root_callback,
                    ),
                    "configure root functions",
                )?;
                self.root_count = root_count;
            }
            check(
                (self.api.max_step)(self.memory, step.max_step),
                "set maximum step",
            )?;
            check(
                (self.api.stop_time)(self.memory, step.boundary),
                "set scheduler boundary",
            )?;
            let mut bridge = Bridge {
                rhs,
                roots: events.as_mut().map(|e| &mut *e.evaluate),
                root_count,
                data: self.api.vector_data,
                width: self.width,
                error: None,
                cancel: step.cancel,
                evaluations: 0,
            };
            check(
                (self.api.set_user)(self.memory, (&raw mut bridge).cast()),
                "attach RHS",
            )?;
            let mut time = step.time;
            let status =
                (self.api.advance)(self.memory, step.boundary, self.vector, &raw mut time, 2);
            // Never leave a borrowed Rust callback pointer in the native object.
            let detach = (self.api.set_user)(self.memory, ptr::null_mut());
            self.stats.rhs_evaluations += bridge.evaluations;
            if let Some(error) = bridge.error {
                return Err(error);
            }
            check(detach, "detach RHS")?;
            if status == 2
                && let Some(e) = &mut events
            {
                check(
                    (self.api.root_info)(self.memory, e.found.as_mut_ptr()),
                    "read root directions",
                )?;
            }

            check(status, "advance CVODE")?;
            if step.cancel.load(Ordering::Relaxed) {
                return Err(RunError::new("cancelled", "simulation was cancelled"));
            }
            step.candidate
                .copy_from_slice(std::slice::from_raw_parts(data, self.width));
            self.stats.accepted_steps += 1;
            Ok(time)
        }
    }
}
impl ContinuousSolver for Cvode {
    fn advance(&mut self, step: OdeStep<'_>, rhs: &mut OdeRhs<'_>) -> Result<f64, RunError> {
        self.advance_events(step, rhs, None)
    }
    fn advance_with_events(
        &mut self,
        step: OdeStep<'_>,
        rhs: &mut OdeRhs<'_>,
        events: OdeEvents<'_, '_>,
    ) -> Result<f64, RunError> {
        self.advance_events(step, rhs, Some(events))
    }
    fn statistics(&self) -> SolverStats {
        let mut stats = self.stats.clone();
        let (failures, iterations) = self.native_counts();
        stats.error_test_failures += failures;
        stats.nonlinear_iterations += iterations;
        stats
    }
}

struct Bridge<'a, 'b, 'c> {
    rhs: &'a mut OdeRhs<'b>,
    roots: Option<&'a mut OdeRhs<'c>>,
    root_count: usize,
    data: unsafe extern "C" fn(Handle) -> *mut f64,
    width: usize,
    error: Option<RunError>,
    cancel: &'a std::sync::atomic::AtomicBool,
    evaluations: u64,
}

unsafe extern "C" fn callback(time: f64, state: Handle, derivative: Handle, user: Handle) -> i32 {
    if user.is_null() {
        return -1;
    }
    // SAFETY: CVode receives this pointer only for the duration of advance. It
    // calls synchronously on this thread with distinct compatible serial vectors.
    let bridge = unsafe { &mut *user.cast::<Bridge<'_, '_, '_>>() };
    if bridge.error.is_some() {
        return -1;
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        if bridge.cancel.load(Ordering::Relaxed) {
            return Err(RunError::new("cancelled", "simulation was cancelled"));
        }
        bridge.evaluations += 1;
        // SAFETY: CVODE's RHS vectors have the initialized fixed dimension.
        unsafe {
            let x = (bridge.data)(state);
            let dx = (bridge.data)(derivative);
            require(x.cast(), "RHS state")?;
            require(dx.cast(), "RHS derivative")?;
            let output = std::slice::from_raw_parts_mut(dx, bridge.width);
            (bridge.rhs)(time, std::slice::from_raw_parts(x, bridge.width), output)?;
            if output.iter().any(|v| !v.is_finite()) {
                return Err(RunError::new(
                    "non_finite_derivative",
                    "RHS produced a non-finite derivative",
                ));
            }
            Ok(())
        }
    }));
    match result {
        Ok(Ok(())) => 0,
        Ok(Err(error)) => {
            bridge.error = Some(error);
            -1
        }
        Err(_) => {
            bridge.error = Some(RunError::new(
                "rhs_panic",
                "RHS panicked; no Rust unwind crossed the C boundary",
            ));
            -1
        }
    }
}

unsafe extern "C" fn root_callback(
    time: f64,
    state: Handle,
    values: *mut f64,
    user: Handle,
) -> i32 {
    // SAFETY: Attached synchronous bridge; CVODE provides count output doubles.
    let bridge = unsafe { &mut *user.cast::<Bridge<'_, '_, '_>>() };
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        if bridge.cancel.load(Ordering::Relaxed) {
            return Err(RunError::new("cancelled", "simulation was cancelled"));
        }
        unsafe {
            let state = (bridge.data)(state);
            require(state.cast(), "root state")?;
            require(values.cast(), "root values")?;
            let values = std::slice::from_raw_parts_mut(values, bridge.root_count);
            let roots = bridge
                .roots
                .as_mut()
                .ok_or_else(|| RunError::new("solver_events", "root callback missing"))?;
            roots(
                time,
                std::slice::from_raw_parts(state, bridge.width),
                values,
            )?;
            if values.iter().any(|v| !v.is_finite()) {
                return Err(RunError::new(
                    "non_finite_event",
                    "root function produced a non-finite value",
                ));
            }
        }
        Ok(())
    }));
    match outcome {
        Ok(Ok(())) => 0,
        Ok(Err(e)) => {
            bridge.error = Some(e);
            -1
        }
        Err(_) => {
            bridge.error = Some(RunError::new(
                "event_panic",
                "event callback panicked; no unwind crossed C",
            ));
            -1
        }
    }
}

fn require(handle: Handle, description: &str) -> Result<(), RunError> {
    if handle.is_null() {
        Err(RunError::new(
            "sundials_allocation",
            format!("SUNDIALS returned null {description}"),
        ))
    } else {
        Ok(())
    }
}

impl Drop for Cvode {
    fn drop(&mut self) {
        // SAFETY: Each non-null handle has exactly one owner. Free dependents
        // before dependencies and unload libraries only after all native frees.
        unsafe {
            if !self.memory.is_null() {
                (self.api.free)(&raw mut self.memory);
            }
            if !self.linear.is_null() {
                (self.api.linear_free)(self.linear);
            }
            if !self.matrix.is_null() {
                (self.api.matrix_destroy)(self.matrix);
            }
            if !self.vector.is_null() {
                (self.api.vector_destroy)(self.vector);
            }
            if !self.context.is_null() {
                (self.api.context_free)(&raw mut self.context);
            }
        }
    }
}
