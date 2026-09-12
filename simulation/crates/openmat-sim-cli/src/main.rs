#![forbid(unsafe_code)]

use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use openmat_sim::model::Model;
use openmat_sim::numeric::{Kernel, ReferenceKernel};
use openmat_sim::{CollectionLimits, Runner, compile};
use openmat_sim_llvm::{LlvmKernel, emit_llvm};
use serde_json::{Value, json};

const USAGE: &str = "OpenMat simulation kernel v0\n\n  openmat-sim check MODEL\n  openmat-sim run MODEL [--backend reference|llvm] [--llvm-library PATH]\n                         [--output RESULT.json] [--max-samples COUNT]\n  openmat-sim emit-llvm MODEL [--output KERNEL.ll]\n\nLLVM requires an explicit LLVM 22 library path or OPENMAT_SIM_LLVM_LIBRARY.\nOutput files are created without overwriting existing files.\n";

struct Options {
    command: String,
    model: PathBuf,
    output: Option<PathBuf>,
    backend: String,
    llvm_library: Option<PathBuf>,
    max_samples: usize,
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
        .filter(|c| ["check", "run", "emit-llvm"].contains(c))
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
    Ok(options)
}

fn read_model(path: &Path) -> Result<Model, Value> {
    const MAX_BYTES: u64 = 16 * 1024 * 1024;
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|e| failure("model_file", e.to_string()))?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| failure("model_file", e.to_string()))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_BYTES {
        return Err(failure("model_limit", "model JSON exceeds 16 MiB"));
    }
    serde_json::from_slice(&bytes).map_err(|e| failure("model_json", e.to_string()))
}

fn execute(options: Options) -> Result<(), Value> {
    let model = read_model(&options.model)?;
    let plan = compile(&model).map_err(|error| json!(error.0))?;
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
    let result = Runner::new(plan, kernel)
        .and_then(|runner| {
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
