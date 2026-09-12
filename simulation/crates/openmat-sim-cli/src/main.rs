#![forbid(unsafe_code)]

use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use openmat_sim::model::{BlockKind, Model};
use openmat_sim::numeric::{Kernel, ReferenceKernel};
use openmat_sim::{CollectionLimits, MAX_SOURCE_BYTES, Runner, SourceBundle, compile_with_sources};
use openmat_sim_llvm::{LlvmKernel, emit_llvm};
use openmat_sim_slx::ImportedSlx;
use openmat_sim_sundials::{Cvode, Method, Options as CvodeOptions};
use serde_json::{Value, json};

const USAGE: &str = "OpenMat simulation\n\n  openmat-sim check MODEL.json|MODEL.slx\n  openmat-sim run MODEL.json|MODEL.slx [--backend reference|llvm] [--llvm-library PATH]\n                                    [--output RESULT.json] [--max-samples COUNT]\n                                    [--solver rk4|cvode-adams|cvode-bdf]\n                                    [--sundials-directory PATH] [--rtol VALUE] [--atol VALUE]\n  openmat-sim emit-llvm MODEL [--output KERNEL.ll]\n  openmat-sim inspect-slx MODEL.slx [--output DOCUMENT.json]\n  openmat-sim import-slx MODEL.slx [--output MODEL.json]\n\nSLX execution requires a supported R2022b model configuration.\nLLVM requires an explicit LLVM 22 library path or OPENMAT_SIM_LLVM_LIBRARY.\nOutput files are created without overwriting existing files.\n";

struct Options {
    command: String,
    model: PathBuf,
    output: Option<PathBuf>,
    backend: String,
    llvm_library: Option<PathBuf>,
    max_samples: usize,
    solver: String,
    sundials_directory: Option<PathBuf>,
    cvode: CvodeOptions,
}

fn main() {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    if args.is_empty() || args == [OsString::from("--help")] || args == [OsString::from("-h")] {
        print!("{USAGE}");
        return;
    }
    if let Err(error) = parse_options(&args).and_then(execute) {
        eprintln!("{}", json!({"ok": false, "error": error}));
        std::process::exit(1);
    }
}

fn failure(code: &str, message: impl Into<String>) -> Value {
    json!({"code": code, "message": message.into()})
}

fn parse_options(args: &[OsString]) -> Result<Options, Value> {
    let command = args[0]
        .to_str()
        .filter(|c| ["check", "run", "emit-llvm", "inspect-slx", "import-slx"].contains(c))
        .ok_or_else(|| failure("arguments", USAGE))?;
    let model = args
        .get(1)
        .filter(|a| !a.to_string_lossy().starts_with("--"))
        .ok_or_else(|| failure("arguments", "a model file is required"))?;
    let mut options = Options {
        command: command.into(),
        model: model.into(),
        output: None,
        backend: "reference".into(),
        llvm_library: std::env::var_os("OPENMAT_SIM_LLVM_LIBRARY").map(Into::into),
        max_samples: 100_000,
        solver: "rk4".into(),
        sundials_directory: std::env::var_os("OPENMAT_SIM_SUNDIALS_DIRECTORY").map(Into::into),
        cvode: CvodeOptions::default(),
    };
    let mut index = 2;
    let mut seen = std::collections::BTreeSet::new();
    while index < args.len() {
        let name = args[index]
            .to_str()
            .ok_or_else(|| failure("arguments", "option names must be UTF-8"))?;
        if !seen.insert(name) {
            return Err(failure("arguments", format!("duplicate option {name}")));
        }
        let value = args
            .get(index + 1)
            .ok_or_else(|| failure("arguments", format!("{name} needs a value")))?;
        match name {
            "--output" if command != "check" => options.output = Some(value.into()),
            "--backend" if command == "run" => {
                options.backend = value
                    .to_str()
                    .filter(|b| ["reference", "llvm"].contains(b))
                    .ok_or_else(|| failure("arguments", "backend must be reference or llvm"))?
                    .into();
            }
            "--llvm-library" if command == "run" => options.llvm_library = Some(value.into()),
            "--sundials-directory" if command == "run" => {
                options.sundials_directory = Some(value.into());
            }
            "--solver" if command == "run" => {
                options.solver = value
                    .to_str()
                    .filter(|s| ["rk4", "cvode-adams", "cvode-bdf"].contains(s))
                    .ok_or_else(|| {
                        failure("arguments", "solver must be rk4, cvode-adams or cvode-bdf")
                    })?
                    .into();
            }
            "--rtol" | "--atol" if command == "run" => {
                let tolerance = value
                    .to_str()
                    .and_then(|s| s.parse::<f64>().ok())
                    .filter(|v| v.is_finite() && *v > 0.)
                    .ok_or_else(|| failure("arguments", "tolerance must be finite and positive"))?;
                if name == "--rtol" {
                    options.cvode.relative_tolerance = tolerance;
                } else {
                    options.cvode.absolute_tolerance = tolerance;
                }
            }
            "--max-samples" if command == "run" => {
                options.max_samples = value
                    .to_str()
                    .and_then(|v| v.parse().ok())
                    .filter(|&v| v > 0)
                    .ok_or_else(|| {
                        failure("arguments", "max-samples must be a positive integer")
                    })?;
            }
            _ => {
                return Err(failure(
                    "arguments",
                    format!("unknown option for {command}: {name}"),
                ));
            }
        }
        index += 2;
    }
    if options.backend != "llvm" && seen.contains("--llvm-library") {
        return Err(failure(
            "arguments",
            "--llvm-library requires --backend llvm",
        ));
    }
    if options.solver == "rk4"
        && ["--rtol", "--atol", "--sundials-directory"]
            .iter()
            .any(|name| seen.contains(name))
    {
        return Err(failure(
            "arguments",
            "SUNDIALS options require a CVODE solver",
        ));
    }
    Ok(options)
}

fn read_model(path: &Path) -> Result<Model, Value> {
    if is_slx(path) {
        return read_slx(path)?
            .lower()
            .map_err(|issues| json!({"code": "slx_compatibility", "issues": issues}));
    }
    let bytes = read_bytes(path, 16 * 1024 * 1024)?;
    let raw: Value =
        serde_json::from_slice(&bytes).map_err(|e| failure("model_json", e.to_string()))?;
    let model = if raw.get("format").is_some() {
        if raw["format"] != "openmat-simulation"
            || !matches!(raw["schemaVersion"].as_u64(), Some(1 | 2))
        {
            return Err(failure("model_json", "unsupported authoring document"));
        }
        if raw["schemaVersion"] == 1 && raw["model"]["schemaVersion"] != 1 {
            return Err(failure(
                "model_json",
                "schema-1 authoring documents require a schema-1 numerical model",
            ));
        }
        raw["model"].clone()
    } else {
        raw
    };
    serde_json::from_value(model).map_err(|e| failure("model_json", e.to_string()))
}

#[allow(clippy::case_sensitive_file_extension_comparisons)] // Match portable source-bundle path semantics.
fn read_sources(model: &Model, path: &Path) -> Result<SourceBundle, Value> {
    let base = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .canonicalize()
        .map_err(|e| failure("source_file", e.to_string()))?;
    let mut sources = SourceBundle::new();
    for block in &model.blocks {
        let BlockKind::MFunction { source, .. } = &block.kind else {
            continue;
        };
        if sources.contains_key(source) {
            continue;
        }
        if sources.len() >= 64
            || !source.ends_with(".m")
            || source.contains(['\\', ':'])
            || !source
                .split('/')
                .all(|p| !p.is_empty() && p != "." && p != "..")
        {
            return Err(failure(
                "source_file",
                "source requires a relative .m path; at most 64 files are supported",
            ));
        }
        let file = base
            .join(source)
            .canonicalize()
            .map_err(|e| failure("source_file", format!("{source}: {e}")))?;
        if !file.starts_with(&base) {
            return Err(failure(
                "source_file",
                "source resolves outside the model directory",
            ));
        }
        let content = String::from_utf8(read_bytes(&file, MAX_SOURCE_BYTES as u64)?)
            .map_err(|e| failure("source_file", e.to_string()))?;
        sources.insert(source.clone(), content);
    }
    Ok(sources)
}

fn is_slx(path: &Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("slx"))
}
fn read_slx(path: &Path) -> Result<ImportedSlx, Value> {
    if !is_slx(path) {
        return Err(failure("arguments", "expected a .slx model file"));
    }
    let bytes = read_bytes(path, 64 * 1024 * 1024)?;
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("imported-model");
    ImportedSlx::read(&bytes, name).map_err(|e| json!(e))
}
fn read_bytes(path: &Path, limit: u64) -> Result<Vec<u8>, Value> {
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|e| failure("model_file", e.to_string()))?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| failure("model_file", e.to_string()))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > limit {
        return Err(failure("model_limit", "model exceeds input byte limit"));
    }
    Ok(bytes)
}

fn execute(options: Options) -> Result<(), Value> {
    if ["inspect-slx", "import-slx"].contains(&options.command.as_str()) {
        let imported = read_slx(&options.model)?;
        let lowered = imported.lower();
        let value = if options.command == "inspect-slx" {
            json!({"ok": true, "format": "slx", "document": imported.document(),
                "runnable": lowered.is_ok(), "issues": lowered.err().unwrap_or_default()})
        } else {
            json!(lowered.map_err(|issues| json!({"code": "slx_compatibility", "issues": issues}))?)
        };
        return write_output(
            options.output.as_deref(),
            &serde_json::to_string_pretty(&value)
                .map_err(|e| failure("result_json", e.to_string()))?,
        );
    }
    let model = read_model(&options.model)?;
    let sources = read_sources(&model, &options.model)?;
    let plan = compile_with_sources(&model, &sources).map_err(|error| json!(error.0))?;
    if options.command == "check" {
        return write_output(None, &serde_json::to_string_pretty(&json!({
            "ok": true, "model": plan.name(), "continuousStates": plan.continuous_state_count(),
            "discreteStates": plan.discrete_state_count(), "scopes": plan.scopes(),
            "executionOrder": plan.execution_order(), "numericalInstructions": plan.program().instructions().len(),
        })).map_err(|e| failure("result_json", e.to_string()))?);
    }
    if options.command == "emit-llvm" {
        return write_output(options.output.as_deref(), &emit_llvm(plan.program()));
    }
    let mut execution = json!({"backend": "reference"});
    let kernel: Box<dyn Kernel> = if options.backend == "llvm" {
        let library = options.llvm_library.ok_or_else(|| {
            failure(
                "llvm_library",
                "set --llvm-library or OPENMAT_SIM_LLVM_LIBRARY to the LLVM 22 shared library",
            )
        })?;
        let kernel = LlvmKernel::compile(plan.program().clone(), &library)
            .map_err(|e| failure("llvm_compile", e.to_string()))?;
        kernel
            .verify_abi_guards()
            .map_err(|e| failure("llvm_abi", e.to_string()))?;
        execution = json!({"backend": "llvm-orc", "llvmVersion": kernel.version(), "targetTriple": kernel.target_triple()});
        Box::new(kernel)
    } else {
        Box::new(ReferenceKernel::new(plan.program().clone()))
    };
    let count = plan.continuous_state_count();
    let result = Runner::new(plan, kernel)
        .and_then(|mut runner| {
            execution["solver"] = json!(options.solver);
            if options.solver != "rk4" {
                let directory = options.sundials_directory.as_deref().ok_or_else(|| {
                    openmat_sim::RunError::new(
                        "native_unavailable",
                        "set --sundials-directory or OPENMAT_SIM_SUNDIALS_DIRECTORY",
                    )
                })?;
                let mut cvode = options.cvode.clone();
                cvode.method = if options.solver == "cvode-adams" {
                    Method::Adams
                } else {
                    Method::Bdf
                };
                runner = runner.with_solver(Box::new(Cvode::new(directory, count, cvode)?))?;
            }
            runner.collect(CollectionLimits {
                max_samples: options.max_samples,
                ..CollectionLimits::default()
            })
        })
        .map_err(|e| json!(e))?;
    let text = serde_json::to_string_pretty(
        &json!({"ok": true, "execution": execution, "result": result}),
    )
    .map_err(|e| failure("result_json", e.to_string()))?;
    write_output(options.output.as_deref(), &text)
}

fn write_output(path: Option<&Path>, text: &str) -> Result<(), Value> {
    if let Some(path) = path {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|e| failure("output_file", e.to_string()))?;
        file.write_all(text.as_bytes())
            .and_then(|()| file.write_all(b"\n"))
            .map_err(|e| failure("output_file", e.to_string()))?;
    } else {
        let stdout = std::io::stdout();
        let mut output = stdout.lock();
        writeln!(output, "{text}").map_err(|e| failure("output_stream", e.to_string()))?;
    }
    Ok(())
}
