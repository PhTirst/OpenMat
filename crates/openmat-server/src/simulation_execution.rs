//! Host-selected native runtimes and client-selected numerical options.
use openmat_sim::numeric::{Kernel, Program, ReferenceKernel};
use openmat_sim::{CompiledModel, RunError, Runner};
use openmat_sim_llvm::LlvmKernel;
use openmat_sim_sundials::{Cvode, Options};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    #[default]
    Reference,
    Llvm,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum Solver {
    #[default]
    Rk4,
    Cvode {
        #[serde(flatten)]
        options: Options,
    },
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Execution {
    #[serde(default)]
    pub backend: Backend,
    #[serde(default)]
    pub solver: Solver,
}

impl Execution {
    pub fn validate(&self, plan: &CompiledModel) -> Result<(), RunError> {
        if matches!(self.backend, Backend::Llvm) {
            configured("OPENMAT_SIM_LLVM_LIBRARY")?;
        }
        if let Solver::Cvode { options } = &self.solver {
            configured("OPENMAT_SIM_SUNDIALS_DIRECTORY")?;
            if !(1..=2048).contains(&plan.continuous_state_count())
                || !options.relative_tolerance.is_finite()
                || !(1e-14..=0.1).contains(&options.relative_tolerance)
                || !options.absolute_tolerance.is_finite()
                || options.absolute_tolerance <= 0.0
            {
                return Err(RunError::new(
                    "solver_configuration",
                    "CVODE requires 1..2048 continuous states, relative tolerance 1e-14..0.1 and positive absolute tolerance",
                ));
            }
        }
        Ok(())
    }
    pub fn summary(&self) -> Value {
        json!({"backend": match self.backend { Backend::Reference => "reference", Backend::Llvm => "llvm-orc" },
            "solver": match &self.solver { Solver::Rk4 => json!({"type":"rk4"}), Solver::Cvode { options } => json!({"type":"cvode", "options":options, "sundialsVersion":"7.5.0"}) }})
    }

    pub fn runner(&self, plan: CompiledModel) -> Result<Runner<Box<dyn Kernel>>, RunError> {
        self.validate(&plan)?;
        let kernel = self.kernel(plan.program())?;
        let update = plan.update_program().map(|p| self.kernel(p)).transpose()?;
        let count = plan.continuous_state_count();
        let runner = Runner::new_with_update(plan, kernel, update)?;
        match &self.solver {
            Solver::Rk4 => Ok(runner),
            Solver::Cvode { options } => runner.with_solver(Box::new(Cvode::new(
                &configured("OPENMAT_SIM_SUNDIALS_DIRECTORY")?,
                count,
                options.clone(),
            )?)),
        }
    }

    fn kernel(&self, program: &Program) -> Result<Box<dyn Kernel>, RunError> {
        let kernel: Box<dyn Kernel> = match self.backend {
            Backend::Reference => Box::new(ReferenceKernel::new(program.clone())),
            Backend::Llvm => {
                let path = configured("OPENMAT_SIM_LLVM_LIBRARY")?;
                Box::new(
                    LlvmKernel::compile(program.clone(), &path)
                        .map_err(|e| RunError::new("llvm_compile", e.to_string()))?,
                )
            }
        };
        Ok(kernel)
    }
}

fn configured(name: &str) -> Result<PathBuf, RunError> {
    std::env::var_os(name)
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| {
            RunError::new(
                "native_unavailable",
                format!("server host must configure {name}; no fallback was used"),
            )
        })
}

pub fn capabilities() -> Value {
    json!({"reference":true,"rk4":true,
        "llvmConfigured":configured("OPENMAT_SIM_LLVM_LIBRARY").is_ok(),
        "cvodeConfigured":configured("OPENMAT_SIM_SUNDIALS_DIRECTORY").is_ok()})
}

#[cfg(test)]
mod component_tests {
    use super::*;
    use openmat_sim::model::Model;
    use openmat_sim::{CollectionLimits, SourceBundle, compile_with_sources};

    fn fixture(name: &str) -> (Model, SourceBundle) {
        let base =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../simulation/examples");
        let model = serde_json::from_str(
            &std::fs::read_to_string(base.join(format!("{name}.omsim.json"))).unwrap(),
        )
        .unwrap();
        let sources = openmat_sim::component::source_references(&model)
            .into_iter()
            .map(|path| {
                (
                    path.to_owned(),
                    std::fs::read_to_string(base.join(path)).unwrap(),
                )
            })
            .collect();
        (model, sources)
    }

    #[test]
    fn component_examples_run_through_host_execution() {
        for name in ["custom-delay", "mass-spring-damper", "pi-control"] {
            let (model, sources) = fixture(name);
            let plan = compile_with_sources(&model, &sources).unwrap();
            let result = Execution::default()
                .runner(plan)
                .unwrap()
                .collect(CollectionLimits::default())
                .unwrap();
            assert_eq!(
                result.frames.last().unwrap().time.to_bits(),
                model.settings.stop_time.to_bits()
            );
            assert!(
                result
                    .frames
                    .iter()
                    .all(|f| f.values.iter().all(|v| v.is_finite()))
            );
        }
    }

    #[test]
    #[ignore = "requires explicitly configured LLVM 22 and SUNDIALS 7.5 runtimes"]
    fn native_components_match_reference_at_sample_ticks() {
        for target in [1.0, -1.0] {
            let (mut model, sources) = fixture("pi-control");
            for block in &mut model.blocks {
                if let openmat_sim::model::BlockKind::Constant { value } = &mut block.kind {
                    value[0] = target;
                }
                if let openmat_sim::model::BlockKind::Component {
                    component,
                    parameters,
                } = &mut block.kind
                    && component == "om_pi"
                {
                    parameters.insert("limit".into(), vec![0.4]);
                }
            }
            let plan = compile_with_sources(&model, &sources).unwrap();
            let expected = Execution::default()
                .runner(plan.clone())
                .unwrap()
                .collect(CollectionLimits::default())
                .unwrap();
            for backend in [Backend::Reference, Backend::Llvm] {
                for solver in [
                    Solver::Rk4,
                    Solver::Cvode {
                        options: Options {
                            relative_tolerance: 1e-9,
                            absolute_tolerance: 1e-11,
                            ..Options::default()
                        },
                    },
                ] {
                    let execution = Execution { backend, solver };
                    let mut runner = execution.runner(plan.clone()).unwrap();
                    let mut ticks = 1;
                    while let Some(frame) = runner.advance().unwrap() {
                        if !frame.sample_hit {
                            continue;
                        }
                        ticks += 1;
                        let reference = expected
                            .frames
                            .iter()
                            .find(|f| (f.time - frame.time).abs() < 1e-10)
                            .unwrap();
                        for (actual, expected) in frame.values.iter().zip(&reference.values) {
                            assert!(
                                (actual - expected).abs() < 2e-7,
                                "{}: {actual} vs {expected}",
                                frame.time
                            );
                        }
                    }
                    assert_eq!(ticks, 201);
                    if let Some(stats) = runner.solver_statistics() {
                        assert!(stats.reinitializations >= 199);
                    }
                }
            }
        }
    }
}
