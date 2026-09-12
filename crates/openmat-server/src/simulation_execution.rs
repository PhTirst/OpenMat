//! Host-selected native runtimes and client-selected numerical options.
use openmat_sim::numeric::{Kernel, ReferenceKernel};
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
        let kernel: Box<dyn Kernel> = match self.backend {
            Backend::Reference => Box::new(ReferenceKernel::new(plan.program().clone())),
            Backend::Llvm => {
                let path = configured("OPENMAT_SIM_LLVM_LIBRARY")?;
                Box::new(
                    LlvmKernel::compile(plan.program().clone(), &path)
                        .map_err(|e| RunError::new("llvm_compile", e.to_string()))?,
                )
            }
        };
        let count = plan.continuous_state_count();
        let runner = Runner::new(plan, kernel)?;
        match &self.solver {
            Solver::Rk4 => Ok(runner),
            Solver::Cvode { options } => runner.with_solver(Box::new(Cvode::new(
                &configured("OPENMAT_SIM_SUNDIALS_DIRECTORY")?,
                count,
                options.clone(),
            )?)),
        }
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
