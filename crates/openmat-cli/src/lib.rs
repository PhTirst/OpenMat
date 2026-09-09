#![doc = "Command-line entry points for `OpenMat` release one."]

use std::collections::HashSet;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use openmat_kernel::{
    EngineError, ExecutionEngine, Kernel, KernelSession, KernelSessionError, KernelSessionMessage,
    KernelSessionRequest, RuntimeEngine,
};
use openmat_protocol::{
    Capabilities, Diagnostic, DiagnosticSeverity, Event, ExecuteRequest, ExecutionMode,
    ImplementationInfo, InitializeRequest, InspectRequest, ListWorkspaceRequest,
    MAX_PREVIEW_ELEMENTS, MatrixPreview, MatrixRange, PROTOCOL_V0, PreviewValue, ProtocolError,
    Request, RequestEnvelope, ResponseEnvelope, ResponseResult, ServerMessage, ShutdownRequest,
    VariableSummary,
};
use openmat_protocol::{kernel_v1, kernel_v2};
use serde_json::{Value as JsonValue, json};

const EXIT_SUCCESS: i32 = 0;
const EXIT_USAGE: i32 = 64;
const EXIT_DATA_ERROR: i32 = 65;
const EXIT_INPUT: i32 = 66;
const EXIT_INTERNAL: i32 = 70;
const EXIT_INTERRUPTED: i32 = 130;
const SESSION_ID: &str = "openmat-cli-session";
const CONFORMANCE_RESULT: &str = "openmat_result";
const CONFORMANCE_MANIFEST_FIELDS: [&str; 6] = [
    "schema_version",
    "id",
    "description",
    "source",
    "tags",
    "expected",
];

#[derive(Clone, Debug, PartialEq, Eq)]
struct EngineConfiguration {
    search_paths: Vec<PathBuf>,
    oex_plugins: Vec<PathBuf>,
}

struct RunOptions {
    workspace: bool,
    oex_plugins: Vec<PathBuf>,
}

fn runtime_engine(configuration: EngineConfiguration) -> Result<RuntimeEngine, EngineError> {
    // SAFETY: `--oex-plugin` is an explicit command-line trust decision. Help documents that
    // each selected library executes native code in the OpenMat process.
    unsafe {
        RuntimeEngine::with_search_paths_and_oex_plugins(
            configuration.search_paths,
            configuration.oex_plugins,
        )
    }
}

/// Runs the CLI with injectable output streams and returns a process exit code.
pub fn run<I, S, W, E>(args: I, output: &mut W, errors: &mut E) -> i32
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
    W: Write,
    E: Write,
{
    run_with(
        args,
        output,
        errors,
        |path| fs::read(path),
        canonicalize_utf8_path,
        runtime_engine,
    )
}

fn run_with<I, S, W, E, R, C, F, Engine>(
    args: I,
    output: &mut W,
    errors: &mut E,
    read_file: R,
    canonicalize_path: C,
    engine_factory: F,
) -> i32
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
    W: Write,
    E: Write,
    R: FnMut(&Path) -> io::Result<Vec<u8>>,
    C: FnMut(&Path) -> io::Result<PathBuf>,
    F: FnOnce(EngineConfiguration) -> Result<Engine, EngineError>,
    Engine: ExecutionEngine,
{
    let arguments: Vec<String> = args.into_iter().map(Into::into).collect();
    let mut read_file = read_file;
    let mut canonicalize_path = canonicalize_path;
    let command = match parse_command(arguments.get(1..).unwrap_or_default()) {
        Ok(command) => command,
        Err(message) => {
            let _ = writeln!(errors, "openmat-cli: {message}; use --help");
            return EXIT_USAGE;
        }
    };

    match command {
        Command::Help => {
            print_help(output);
            EXIT_SUCCESS
        }
        Command::Version => {
            let _ = writeln!(output, "openmat-cli {}", env!("CARGO_PKG_VERSION"));
            EXIT_SUCCESS
        }
        Command::ProtocolSelftest => match protocol_selftest() {
            Ok(()) => {
                let _ = writeln!(output, "{PROTOCOL_V0}: selftest passed");
                EXIT_SUCCESS
            }
            Err(error) => {
                let _ = writeln!(
                    errors,
                    "error[cli.protocolSelftest] source=<protocol> bytes=unknown: {error}"
                );
                EXIT_INTERNAL
            }
        },
        Command::Run {
            path,
            workspace,
            oex_plugins,
        } => run_file(
            &path,
            RunOptions {
                workspace,
                oex_plugins,
            },
            &mut read_file,
            &mut canonicalize_path,
            engine_factory,
            output,
            errors,
        ),
        Command::Conformance { manifest } => run_conformance(
            &manifest,
            &mut read_file,
            &mut canonicalize_path,
            engine_factory,
            output,
            errors,
        ),
    }
}

fn run_file<R, C, F, Engine, W, E>(
    path: &Path,
    options: RunOptions,
    read_file: &mut R,
    canonicalize_path: &mut C,
    engine_factory: F,
    output: &mut W,
    errors: &mut E,
) -> i32
where
    R: FnMut(&Path) -> io::Result<Vec<u8>>,
    C: FnMut(&Path) -> io::Result<PathBuf>,
    F: FnOnce(EngineConfiguration) -> Result<Engine, EngineError>,
    Engine: ExecutionEngine,
    W: Write,
    E: Write,
{
    let source_path = match canonicalize_path(path) {
        Ok(path) => path,
        Err(error) => {
            let _ = writeln!(
                errors,
                "error[cli.input] source={} bytes=unknown: {error}",
                stable_source_name(path)
            );
            return EXIT_INPUT;
        }
    };
    let source_name = stable_source_name(&source_path);
    let bytes = match read_file(&source_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            let _ = writeln!(
                errors,
                "error[cli.input] source={source_name} bytes=unknown: {error}"
            );
            return EXIT_INPUT;
        }
    };
    let code = match String::from_utf8(bytes) {
        Ok(code) => code,
        Err(error) => {
            let utf8_error = error.utf8_error();
            let start = utf8_error.valid_up_to();
            let range = utf8_error.error_len().map_or_else(
                || format!("{start}..end"),
                |length| format!("{start}..{}", start + length),
            );
            let _ = writeln!(
                errors,
                "error[cli.utf8] source={source_name} bytes={range}: input is not valid UTF-8"
            );
            return EXIT_INPUT;
        }
    };
    let search_paths = entry_search_paths(&source_path, Vec::new());
    let engine = match engine_factory(EngineConfiguration {
        search_paths,
        oex_plugins: options.oex_plugins,
    }) {
        Ok(engine) => engine,
        Err(error) => {
            let _ = writeln!(
                errors,
                "error[cli.engineInit] source=<engine> bytes=unknown: {error}"
            );
            return EXIT_INTERNAL;
        }
    };
    let mut kernel = Kernel::new(SESSION_ID, engine);
    drive_session(
        &mut kernel,
        ExecuteRequest {
            code,
            source_name,
            mode: ExecutionMode::File,
        },
        options.workspace,
        output,
        errors,
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Command {
    Help,
    Version,
    ProtocolSelftest,
    Run {
        path: PathBuf,
        workspace: bool,
        oex_plugins: Vec<PathBuf>,
    },
    Conformance {
        manifest: PathBuf,
    },
}

fn parse_command(arguments: &[String]) -> Result<Command, String> {
    match arguments {
        [] => Ok(Command::Help),
        [option] if option == "--help" || option == "-h" => Ok(Command::Help),
        [option] if option == "version" || option == "--version" => Ok(Command::Version),
        [group, action] if group == "protocol" && action == "selftest" => {
            Ok(Command::ProtocolSelftest)
        }
        [command, manifest] if command == "conformance" => {
            if !Path::new(manifest)
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
            {
                return Err("conformance requires a .json manifest".to_owned());
            }
            Ok(Command::Conformance {
                manifest: PathBuf::from(manifest),
            })
        }
        [command, rest @ ..] if command == "run" => parse_run_arguments(rest),
        [path, rest @ ..] if has_m_extension(path) => parse_run_arguments_with_path(path, rest),
        _ => Err("unsupported command or arguments".to_owned()),
    }
}

fn parse_run_arguments(arguments: &[String]) -> Result<Command, String> {
    let mut path = None;
    let mut workspace = false;
    let mut oex_plugins = Vec::new();
    let mut index = 0;
    while index < arguments.len() {
        let argument = &arguments[index];
        if argument == "--workspace" {
            if workspace {
                return Err("--workspace may only be specified once".to_owned());
            }
            workspace = true;
        } else if argument == "--oex-plugin" {
            index += 1;
            let plugin = arguments
                .get(index)
                .ok_or_else(|| "--oex-plugin requires a library path".to_owned())?;
            if plugin.is_empty() || plugin.starts_with("--") {
                return Err("--oex-plugin requires a non-empty library path".to_owned());
            }
            oex_plugins.push(PathBuf::from(plugin));
        } else if argument.starts_with('-') {
            return Err(format!("unknown option `{argument}`"));
        } else if path.replace(argument.as_str()).is_some() {
            return Err("run accepts exactly one source file".to_owned());
        }
        index += 1;
    }
    let path = path.ok_or_else(|| "run requires a .m source file".to_owned())?;
    if !has_m_extension(path) {
        return Err("source file must have a .m extension".to_owned());
    }
    Ok(Command::Run {
        path: PathBuf::from(path),
        workspace,
        oex_plugins,
    })
}

fn parse_run_arguments_with_path(path: &str, arguments: &[String]) -> Result<Command, String> {
    let mut combined = Vec::with_capacity(arguments.len().saturating_add(1));
    combined.push(path.to_owned());
    combined.extend(arguments.iter().cloned());
    parse_run_arguments(&combined)
}

fn has_m_extension(path: &str) -> bool {
    Path::new(path)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("m"))
}

fn stable_source_name(path: &Path) -> String {
    path.as_os_str().to_string_lossy().into_owned()
}

fn canonicalize_utf8_path(path: &Path) -> io::Result<PathBuf> {
    let canonical = fs::canonicalize(path)?;
    if canonical.to_str().is_none() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "canonical source path is not valid UTF-8",
        ));
    }
    Ok(canonical)
}

fn entry_search_paths(entry_path: &Path, additional_paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut search_paths = entry_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .into_iter()
        .collect::<Vec<_>>();
    for path in additional_paths {
        if !search_paths.contains(&path) {
            search_paths.push(path);
        }
    }
    search_paths
}

fn print_help<W: Write>(output: &mut W) {
    let _ = writeln!(output, "Usage:");
    let _ = writeln!(
        output,
        "  openmat-cli run <file.m> [--workspace] [--oex-plugin <library>]..."
    );
    let _ = writeln!(
        output,
        "  openmat-cli <file.m> [--workspace] [--oex-plugin <library>]..."
    );
    let _ = writeln!(output, "  openmat-cli version");
    let _ = writeln!(output, "  openmat-cli protocol selftest");
    let _ = writeln!(output, "  openmat-cli conformance <manifest.json>");
    let _ = writeln!(output, "\nOptions:");
    let _ = writeln!(
        output,
        "  --workspace  After execution, print variable name, class, and dimensions"
    );
    let _ = writeln!(
        output,
        "  --oex-plugin <library>  Load a trusted native OEX library; repeat to load more"
    );
    let _ = writeln!(output, "  -h, --help   Show this help");
    let _ = writeln!(output, "  --version    Print the CLI version");
    let _ = writeln!(output, "\nExit codes:");
    let _ = writeln!(
        output,
        "  0 success; 64 usage; 65 compile/runtime; 66 input/UTF-8"
    );
    let _ = writeln!(output, "  70 engine/protocol; 130 interrupted");
}

trait RequestHandler {
    fn handle_request(&mut self, request: &RequestEnvelope) -> Vec<ServerMessage>;
}

impl<Engine: ExecutionEngine> RequestHandler for Kernel<Engine> {
    fn handle_request(&mut self, request: &RequestEnvelope) -> Vec<ServerMessage> {
        Kernel::handle_request(self, request)
    }
}

#[derive(Clone, Copy)]
enum ExpectedResponse {
    Initialize,
    Execute,
    Inspect,
    ListWorkspace,
    Shutdown,
}

enum ExchangeResult {
    Success(ResponseResult),
    Failure(ProtocolError),
}

struct SessionDriver<'a, Handler> {
    handler: &'a mut Handler,
    next_request_id: u64,
    seen_server_ids: HashSet<String>,
}

impl<'a, Handler: RequestHandler> SessionDriver<'a, Handler> {
    fn new(handler: &'a mut Handler) -> Self {
        Self {
            handler,
            next_request_id: 1,
            seen_server_ids: HashSet::new(),
        }
    }

    fn exchange<W: Write, E: Write>(
        &mut self,
        request: Request,
        expected: ExpectedResponse,
        fallback_source: &str,
        output: &mut W,
        errors: &mut E,
    ) -> Result<ExchangeResult, String> {
        let request =
            RequestEnvelope::new(SESSION_ID, format!("cli-{}", self.next_request_id), request);
        self.next_request_id += 1;
        request
            .validate()
            .map_err(|error| format!("invalid outgoing request: {error}"))?;

        let messages = self.handler.handle_request(&request);
        let mut response = None;
        for message in messages {
            match message {
                ServerMessage::Event(envelope) => {
                    envelope
                        .validate()
                        .map_err(|error| format!("invalid event envelope: {error}"))?;
                    if envelope.session_id != SESSION_ID {
                        return Err("event session does not match the CLI session".to_owned());
                    }
                    if !self.seen_server_ids.insert(envelope.message_id) {
                        return Err("kernel reused a server message ID".to_owned());
                    }
                    render_event(&envelope.event, output, errors)
                        .map_err(|error| format!("failed to write kernel event: {error}"))?;
                }
                ServerMessage::Response(envelope) => {
                    envelope
                        .validate()
                        .map_err(|error| format!("invalid response envelope: {error}"))?;
                    if !envelope.is_reply_to(&request) {
                        return Err(format!(
                            "response `{}` is not correlated to request `{}`",
                            envelope.message_id, request.message_id
                        ));
                    }
                    if !self.seen_server_ids.insert(envelope.message_id.clone()) {
                        return Err("kernel reused a server message ID".to_owned());
                    }
                    if response.replace(envelope).is_some() {
                        return Err(format!(
                            "kernel returned multiple responses for request `{}`",
                            request.message_id
                        ));
                    }
                }
            }
        }

        let response = response.ok_or_else(|| {
            format!(
                "kernel returned no response for request `{}`",
                request.message_id
            )
        })?;
        if response.ok {
            let result = response
                .result
                .ok_or_else(|| "successful response omitted its result".to_owned())?;
            if !response_matches(expected, &result) {
                return Err("kernel response type does not match the request".to_owned());
            }
            Ok(ExchangeResult::Success(result))
        } else {
            let error = response
                .error
                .ok_or_else(|| "failed response omitted its error".to_owned())?;
            render_protocol_error(&error, fallback_source, errors)
                .map_err(|write_error| format!("failed to write kernel failure: {write_error}"))?;
            Ok(ExchangeResult::Failure(error))
        }
    }
}

fn response_matches(expected: ExpectedResponse, result: &ResponseResult) -> bool {
    matches!(
        (expected, result),
        (ExpectedResponse::Initialize, ResponseResult::Initialize(_))
            | (ExpectedResponse::Execute, ResponseResult::Execute(_))
            | (ExpectedResponse::Inspect, ResponseResult::Inspect(_))
            | (
                ExpectedResponse::ListWorkspace,
                ResponseResult::ListWorkspace(_)
            )
            | (ExpectedResponse::Shutdown, ResponseResult::Shutdown(_))
    )
}

fn drive_session<Handler, W, E>(
    handler: &mut Handler,
    execute: ExecuteRequest,
    workspace: bool,
    output: &mut W,
    errors: &mut E,
) -> i32
where
    Handler: RequestHandler,
    W: Write,
    E: Write,
{
    let source_name = execute.source_name.clone();
    let mut driver = SessionDriver::new(handler);
    let initialize = Request::Initialize(InitializeRequest {
        client: ImplementationInfo {
            name: "openmat-cli".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
        },
        supported_protocols: vec![PROTOCOL_V0.to_owned()],
        capabilities: Capabilities {
            execution_modes: vec![ExecutionMode::File],
            display_mime_types: vec!["text/plain".to_owned()],
            max_preview_elements: MAX_PREVIEW_ELEMENTS,
            interrupt: false,
            workspace_delta: false,
        },
    });

    let mut exit = match driver.exchange(
        initialize,
        ExpectedResponse::Initialize,
        "<protocol>",
        output,
        errors,
    ) {
        Ok(ExchangeResult::Success(_)) => EXIT_SUCCESS,
        Ok(ExchangeResult::Failure(_)) => EXIT_INTERNAL,
        Err(error) => {
            render_internal_error(&error, errors);
            EXIT_INTERNAL
        }
    };

    if exit == EXIT_SUCCESS {
        match driver.exchange(
            Request::Execute(execute),
            ExpectedResponse::Execute,
            &source_name,
            output,
            errors,
        ) {
            Ok(ExchangeResult::Success(ResponseResult::Execute(result))) => {
                if result.interrupted {
                    exit = EXIT_INTERRUPTED;
                }
            }
            Ok(ExchangeResult::Success(_)) => unreachable!("response type checked by exchange"),
            Ok(ExchangeResult::Failure(error)) => {
                exit = failure_exit_code(&error);
            }
            Err(error) => {
                render_internal_error(&error, errors);
                exit = EXIT_INTERNAL;
            }
        }

        if workspace && exit != EXIT_INTERNAL {
            match driver.exchange(
                Request::ListWorkspace(ListWorkspaceRequest {}),
                ExpectedResponse::ListWorkspace,
                "<workspace>",
                output,
                errors,
            ) {
                Ok(ExchangeResult::Success(ResponseResult::ListWorkspace(summary))) => {
                    if let Err(error) = render_workspace(summary.variables, output) {
                        render_internal_error(
                            &format!("failed to write workspace summary: {error}"),
                            errors,
                        );
                        exit = EXIT_INTERNAL;
                    }
                }
                Ok(ExchangeResult::Success(_)) => {
                    unreachable!("response type checked by exchange")
                }
                Ok(ExchangeResult::Failure(error)) => {
                    let failure_exit = failure_exit_code(&error);
                    if exit == EXIT_SUCCESS || failure_exit == EXIT_INTERNAL {
                        exit = failure_exit;
                    }
                }
                Err(error) => {
                    render_internal_error(&error, errors);
                    exit = EXIT_INTERNAL;
                }
            }
        }
    }

    match driver.exchange(
        Request::Shutdown(ShutdownRequest {}),
        ExpectedResponse::Shutdown,
        "<protocol>",
        output,
        errors,
    ) {
        Ok(ExchangeResult::Success(_)) => {}
        Ok(ExchangeResult::Failure(_)) => exit = EXIT_INTERNAL,
        Err(error) => {
            render_internal_error(&error, errors);
            exit = EXIT_INTERNAL;
        }
    }
    exit
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConformanceSchema {
    V1,
    V2,
}

impl ConformanceSchema {
    const fn version(self) -> u64 {
        match self {
            Self::V1 => 1,
            Self::V2 => 2,
        }
    }
}

#[derive(Debug)]
struct ConformanceCase {
    schema: ConformanceSchema,
    id: String,
    source_path: PathBuf,
    case_directory: PathBuf,
    requires_kernel_v2: bool,
}

#[derive(Debug)]
enum ConformanceRunError {
    Capability(String),
    Protocol(String),
    Internal(String),
}

#[allow(clippy::too_many_lines)]
fn run_conformance<R, C, F, Engine, W, E>(
    manifest_path: &Path,
    read_file: &mut R,
    canonicalize_path: &mut C,
    engine_factory: F,
    output: &mut W,
    errors: &mut E,
) -> i32
where
    R: FnMut(&Path) -> io::Result<Vec<u8>>,
    C: FnMut(&Path) -> io::Result<PathBuf>,
    F: FnOnce(EngineConfiguration) -> Result<Engine, EngineError>,
    Engine: ExecutionEngine,
    W: Write,
    E: Write,
{
    let manifest_path = match canonicalize_path(manifest_path) {
        Ok(path) => path,
        Err(error) => {
            let _ = writeln!(
                errors,
                "error[cli.input] source={} bytes=unknown: {error}",
                stable_source_name(manifest_path)
            );
            return EXIT_INPUT;
        }
    };
    let manifest_bytes = match read_file(&manifest_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            let _ = writeln!(
                errors,
                "error[cli.input] source={} bytes=unknown: {error}",
                stable_source_name(&manifest_path)
            );
            return EXIT_INPUT;
        }
    };
    let case = match parse_conformance_manifest(&manifest_path, &manifest_bytes) {
        Ok(case) => case,
        Err(error) => {
            let _ = writeln!(
                errors,
                "error[cli.manifest] source={} bytes=unknown: {error}",
                stable_source_name(&manifest_path)
            );
            return EXIT_INPUT;
        }
    };
    let source_path = match canonicalize_path(&case.source_path) {
        Ok(path) => path,
        Err(error) => {
            let _ = writeln!(
                errors,
                "error[cli.input] source={} bytes=unknown: {error}",
                stable_source_name(&case.source_path)
            );
            return EXIT_INPUT;
        }
    };
    let source_bytes = match read_file(&source_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            let _ = writeln!(
                errors,
                "error[cli.input] source={} bytes=unknown: {error}",
                stable_source_name(&source_path)
            );
            return EXIT_INPUT;
        }
    };
    let code = match String::from_utf8(source_bytes) {
        Ok(code) => code,
        Err(error) => {
            let start = error.utf8_error().valid_up_to();
            let _ = writeln!(
                errors,
                "error[cli.utf8] source={} bytes={start}: input is not valid UTF-8",
                stable_source_name(&source_path)
            );
            return EXIT_INPUT;
        }
    };
    let support_path = match canonicalize_path(&case.case_directory.join("support")) {
        Ok(path) => path,
        Err(error) => {
            let _ = writeln!(
                errors,
                "error[cli.input] source={} bytes=unknown: {error}",
                stable_source_name(&case.case_directory.join("support"))
            );
            return EXIT_INPUT;
        }
    };
    let search_paths = entry_search_paths(&source_path, vec![support_path]);
    let engine = match engine_factory(EngineConfiguration {
        search_paths,
        oex_plugins: Vec::new(),
    }) {
        Ok(engine) => engine,
        Err(error) => {
            let _ = writeln!(errors, "error[cli.engineInit]: {}", error.category());
            return EXIT_INTERNAL;
        }
    };
    let source_name = stable_source_name(&source_path);
    let observation = match case.schema {
        ConformanceSchema::V1 => {
            let mut kernel = Kernel::new(SESSION_ID, engine);
            execute_conformance_case(&mut kernel, &case.id, code, source_name.as_str())
                .map_err(ConformanceRunError::Internal)
        }
        ConformanceSchema::V2 => {
            let mut session = KernelSession::new(SESSION_ID, engine);
            execute_conformance_case_v2(
                &mut session,
                &case.id,
                code,
                source_name.as_str(),
                case.requires_kernel_v2,
            )
        }
    };
    finish_conformance_run(observation, output, errors)
}

fn finish_conformance_run<W: Write, E: Write>(
    observation: Result<JsonValue, ConformanceRunError>,
    output: &mut W,
    errors: &mut E,
) -> i32 {
    match observation {
        Ok(observation) => write_conformance_observation(&observation, output, errors),
        Err(ConformanceRunError::Capability(detail)) => {
            let _ = writeln!(
                errors,
                "error[cli.capability]: schema-v2 requires openmat-kernel-v1 or newer: {detail}"
            );
            EXIT_INTERNAL
        }
        Err(ConformanceRunError::Protocol(detail)) => {
            let _ = writeln!(
                errors,
                "error[cli.protocol]: schema-v2 conformance session failed: {detail}"
            );
            EXIT_INTERNAL
        }
        Err(ConformanceRunError::Internal(category)) => {
            let _ = writeln!(errors, "error[cli.conformance]: {category}");
            EXIT_INTERNAL
        }
    }
}

fn parse_conformance_manifest(
    manifest_path: &Path,
    bytes: &[u8],
) -> Result<ConformanceCase, String> {
    let manifest: JsonValue = serde_json::from_slice(bytes)
        .map_err(|error| format!("manifest is not valid JSON: {error}"))?;
    let object = manifest
        .as_object()
        .ok_or_else(|| "manifest root must be an object".to_owned())?;
    if object.len() != CONFORMANCE_MANIFEST_FIELDS.len()
        || object
            .keys()
            .any(|name| !CONFORMANCE_MANIFEST_FIELDS.contains(&name.as_str()))
    {
        return Err("manifest fields do not match the conformance case contract".to_owned());
    }
    let schema = match object.get("schema_version").and_then(JsonValue::as_u64) {
        Some(1) => ConformanceSchema::V1,
        Some(2) => ConformanceSchema::V2,
        Some(version) => return Err(format!("unsupported schema_version {version}")),
        None => return Err("schema_version must be the integer 1 or 2".to_owned()),
    };
    let id = object
        .get("id")
        .and_then(JsonValue::as_str)
        .filter(|value| valid_case_id(value))
        .ok_or_else(|| "id is missing or invalid".to_owned())?
        .to_owned();
    let source = object
        .get("source")
        .and_then(JsonValue::as_str)
        .filter(|value| valid_case_source(value))
        .ok_or_else(|| "source must name one programs/*.m file".to_owned())?;
    object
        .get("description")
        .and_then(JsonValue::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "description must be a non-empty string".to_owned())?;
    let tags = object
        .get("tags")
        .and_then(JsonValue::as_array)
        .filter(|values| !values.is_empty())
        .ok_or_else(|| "tags must be a non-empty array".to_owned())?;
    let mut unique_tags = HashSet::new();
    for tag in tags {
        let tag = tag
            .as_str()
            .filter(|value| valid_case_tag(value))
            .ok_or_else(|| "tags must contain only canonical tag strings".to_owned())?;
        if !unique_tags.insert(tag) {
            return Err("tags must not contain duplicates".to_owned());
        }
    }
    let expected = object
        .get("expected")
        .expect("exact manifest fields include expected");
    validate_common_expected(expected)?;
    let requires_kernel_v2 = schema == ConformanceSchema::V2
        && expected
            .get("value")
            .is_some_and(value_may_contain_aggregate);
    let case_directory = manifest_path
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| "manifest must be inside a manifests directory".to_owned())?
        .to_path_buf();
    let source_path = source
        .split('/')
        .fold(case_directory.clone(), |path, part| path.join(part));
    Ok(ConformanceCase {
        schema,
        id,
        source_path,
        case_directory,
        requires_kernel_v2,
    })
}

fn value_may_contain_aggregate(value: &JsonValue) -> bool {
    match value {
        JsonValue::Array(values) => values.iter().any(value_may_contain_aggregate),
        JsonValue::Object(object) => {
            matches!(
                object.get("kind").and_then(JsonValue::as_str),
                Some("cell" | "struct")
            ) || object.values().any(value_may_contain_aggregate)
        }
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::String(_) => false,
    }
}

fn valid_case_id(value: &str) -> bool {
    let mut characters = value.chars();
    characters
        .next()
        .is_some_and(|character| character.is_ascii_lowercase())
        && characters.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        })
}

fn valid_case_source(value: &str) -> bool {
    let Some(name) = value.strip_prefix("programs/") else {
        return false;
    };
    !name.is_empty()
        && !name.contains(['/', '\\'])
        && Path::new(name)
            .extension()
            .is_some_and(|extension| extension == "m")
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "_.-".contains(character))
}

fn valid_case_tag(value: &str) -> bool {
    let mut characters = value.chars();
    characters
        .next()
        .is_some_and(|character| character.is_ascii_lowercase())
        && characters.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
}

fn validate_common_expected(expected: &JsonValue) -> Result<(), String> {
    let object = expected
        .as_object()
        .ok_or_else(|| "expected must be an object".to_owned())?;
    match object.get("outcome").and_then(JsonValue::as_str) {
        Some("ok") => {
            if object.len() != 2 || !object.contains_key("value") || !object["value"].is_object() {
                return Err("expected ok outcome must contain only outcome and value".to_owned());
            }
        }
        Some("error") => {
            if object.len() != 2 || !object.contains_key("error") {
                return Err("expected error outcome must contain only outcome and error".to_owned());
            }
            let error = object["error"]
                .as_object()
                .filter(|error| error.len() == 1)
                .ok_or_else(|| "expected error must contain only category".to_owned())?;
            let category = error
                .get("category")
                .and_then(JsonValue::as_str)
                .ok_or_else(|| "expected error category must be a string".to_owned())?;
            if !matches!(
                category,
                "access-violation"
                    | "arity"
                    | "assertion-failed"
                    | "dimension-mismatch"
                    | "index-out-of-bounds"
                    | "syntax-error"
                    | "type-error"
                    | "undefined-name"
                    | "other"
            ) {
                return Err("expected error category is unknown".to_owned());
            }
        }
        _ => return Err("expected outcome must be ok or error".to_owned()),
    }
    Ok(())
}

fn execute_conformance_case<Handler: RequestHandler>(
    handler: &mut Handler,
    case_id: &str,
    code: String,
    source_name: &str,
) -> Result<JsonValue, String> {
    let mut driver = SessionDriver::new(handler);
    let mut event_output = io::sink();
    let mut event_errors = io::sink();
    let result = observe_conformance_case(
        &mut driver,
        case_id,
        code,
        source_name,
        &mut event_output,
        &mut event_errors,
    );

    let shutdown = driver.exchange(
        Request::Shutdown(ShutdownRequest {}),
        ExpectedResponse::Shutdown,
        "<protocol>",
        &mut event_output,
        &mut event_errors,
    );
    match shutdown {
        Ok(ExchangeResult::Success(_)) => result,
        Ok(ExchangeResult::Failure(error)) => Err(error.category),
        Err(error) => Err(error),
    }
}

trait KernelSessionHandler {
    fn handle_session(
        &mut self,
        request: KernelSessionRequest<'_>,
    ) -> Result<Vec<KernelSessionMessage>, KernelSessionError>;
}

impl<Engine: ExecutionEngine> KernelSessionHandler for KernelSession<Engine> {
    fn handle_session(
        &mut self,
        request: KernelSessionRequest<'_>,
    ) -> Result<Vec<KernelSessionMessage>, KernelSessionError> {
        self.handle(request)
    }
}

#[derive(Debug)]
enum SchemaV2DriverError {
    Protocol(String),
}

impl From<KernelSessionError> for SchemaV2DriverError {
    fn from(error: KernelSessionError) -> Self {
        Self::Protocol(error.to_string())
    }
}

enum SchemaV2ExchangeResult {
    Success(SchemaV2Response),
    Failure(ProtocolError),
}

enum SchemaV2Response {
    Execute(openmat_protocol::ExecuteResult),
    Inspect(SchemaV2InspectPreview),
    ListWorkspace(openmat_protocol::WorkspaceSummary),
    Shutdown,
}

enum SchemaV2InspectPreview {
    Matrix(kernel_v1::MatrixPreview),
    Aggregate {
        preview: kernel_v2::AggregatePreview,
        limits: kernel_v2::AggregateLimits,
    },
}

enum SchemaV2Request {
    Execute(ExecuteRequest),
    Inspect(kernel_v1::InspectRequest),
    ListWorkspace,
    Shutdown,
}

#[derive(Clone, Copy)]
enum SchemaV2RequestKind {
    Execute,
    Inspect,
    ListWorkspace,
    Shutdown,
}

impl SchemaV2Request {
    const fn kind(&self) -> SchemaV2RequestKind {
        match self {
            Self::Execute(_) => SchemaV2RequestKind::Execute,
            Self::Inspect(_) => SchemaV2RequestKind::Inspect,
            Self::ListWorkspace => SchemaV2RequestKind::ListWorkspace,
            Self::Shutdown => SchemaV2RequestKind::Shutdown,
        }
    }
}

struct SchemaV2SessionDriver<'a, Handler> {
    handler: &'a mut Handler,
    protocol: kernel_v2::ProtocolVersion,
    preview_limits: Option<kernel_v1::PreviewLimits>,
    aggregate_limits: Option<kernel_v2::AggregateLimits>,
    next_request_id: u64,
    seen_server_ids: HashSet<String>,
}

impl<'a, Handler: KernelSessionHandler> SchemaV2SessionDriver<'a, Handler> {
    fn initialize(handler: &'a mut Handler) -> Result<Self, SchemaV2DriverError> {
        let initialize = kernel_v2::InitializeRequest::v2(
            ImplementationInfo {
                name: "openmat-conformance".to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
            },
            kernel_v2::Capabilities {
                execution_modes: vec![ExecutionMode::File],
                display_mime_types: vec!["text/plain".to_owned()],
                max_preview_elements: MAX_PREVIEW_ELEMENTS,
                max_string_element_code_units: Some(kernel_v1::MAX_STRING_ELEMENT_CODE_UNITS),
                max_preview_code_units: Some(kernel_v1::MAX_PREVIEW_CODE_UNITS),
                max_aggregate_nodes: Some(kernel_v2::MAX_AGGREGATE_NODES),
                max_aggregate_elements: Some(kernel_v2::MAX_AGGREGATE_ELEMENTS),
                max_aggregate_depth: Some(kernel_v2::MAX_AGGREGATE_DEPTH),
                interrupt: false,
                workspace_delta: false,
            },
        );
        Self::initialize_with(handler, initialize)
    }

    #[allow(clippy::too_many_lines)]
    fn initialize_with(
        handler: &'a mut Handler,
        initialize: kernel_v2::InitializeRequest,
    ) -> Result<Self, SchemaV2DriverError> {
        let request = kernel_v2::BootstrapRequestEnvelope::new(SESSION_ID, "cli-v2-1", initialize);
        request
            .validate()
            .map_err(|error| SchemaV2DriverError::Protocol(error.to_string()))?;
        let messages = handler.handle_session(KernelSessionRequest::Bootstrap(&request))?;
        let mut messages = messages.into_iter();
        let response = match messages.next() {
            Some(KernelSessionMessage::Bootstrap(response)) => response,
            Some(_) => {
                return Err(SchemaV2DriverError::Protocol(
                    "bootstrap did not return a bootstrap response first".to_owned(),
                ));
            }
            None => {
                return Err(SchemaV2DriverError::Protocol(
                    "bootstrap returned no response".to_owned(),
                ));
            }
        };
        response
            .validate()
            .map_err(|error| SchemaV2DriverError::Protocol(error.to_string()))?;
        if response.session_id != request.session_id || response.reply_to != request.message_id {
            return Err(SchemaV2DriverError::Protocol(
                "bootstrap response is not correlated to the initialize request".to_owned(),
            ));
        }
        let mut seen_server_ids = HashSet::new();
        if !seen_server_ids.insert(response.message_id.clone()) {
            return Err(SchemaV2DriverError::Protocol(
                "kernel reused a bootstrap server message ID".to_owned(),
            ));
        }
        if !response.ok {
            let error = response.error.ok_or_else(|| {
                SchemaV2DriverError::Protocol(
                    "failed bootstrap response omitted its error".to_owned(),
                )
            })?;
            return Err(SchemaV2DriverError::Protocol(format!(
                "{}: {}",
                error.category, error.message
            )));
        }
        let selected = kernel_v2::validate_initialize_exchange(&request, &response)
            .map_err(|error| SchemaV2DriverError::Protocol(error.to_string()))?;
        let Some(kernel_v2::BootstrapResponseResult::Initialize(result)) = response.result.as_ref()
        else {
            return Err(SchemaV2DriverError::Protocol(
                "successful bootstrap response omitted initialize data".to_owned(),
            ));
        };

        for message in messages {
            match (selected, message) {
                (
                    kernel_v2::ProtocolVersion::V2,
                    KernelSessionMessage::V2(kernel_v2::ServerMessage::Event(event)),
                ) => {
                    event
                        .validate()
                        .map_err(|error| SchemaV2DriverError::Protocol(error.to_string()))?;
                    validate_schema_v2_server_identity(
                        &event.session_id,
                        event.message_id,
                        &mut seen_server_ids,
                    )?;
                }
                (
                    kernel_v2::ProtocolVersion::V1,
                    KernelSessionMessage::V1(kernel_v1::ServerMessage::Event(event)),
                ) => {
                    event
                        .validate()
                        .map_err(|error| SchemaV2DriverError::Protocol(error.to_string()))?;
                    validate_schema_v2_server_identity(
                        &event.session_id,
                        event.message_id,
                        &mut seen_server_ids,
                    )?;
                }
                (
                    kernel_v2::ProtocolVersion::V0,
                    KernelSessionMessage::V0(ServerMessage::Event(event)),
                ) => {
                    event
                        .validate()
                        .map_err(|error| SchemaV2DriverError::Protocol(error.to_string()))?;
                    validate_schema_v2_server_identity(
                        &event.session_id,
                        event.message_id,
                        &mut seen_server_ids,
                    )?;
                }
                _ => {
                    return Err(SchemaV2DriverError::Protocol(
                        "bootstrap follow-up did not use the selected event protocol".to_owned(),
                    ));
                }
            }
        }

        let preview_limits = match selected {
            kernel_v2::ProtocolVersion::V0 => None,
            kernel_v2::ProtocolVersion::V3 => {
                return Err(SchemaV2DriverError::Protocol(
                    "the schema-v2 conformance driver does not support kernel v3".to_owned(),
                ));
            }
            kernel_v2::ProtocolVersion::V1 | kernel_v2::ProtocolVersion::V2 => Some(
                kernel_v1::PreviewLimits::new(
                    result.capabilities.max_preview_elements,
                    result
                        .capabilities
                        .max_string_element_code_units
                        .ok_or_else(|| {
                            SchemaV2DriverError::Protocol(
                                "selected v1/v2 capabilities omitted maxStringElementCodeUnits"
                                    .to_owned(),
                            )
                        })?,
                    result.capabilities.max_preview_code_units.ok_or_else(|| {
                        SchemaV2DriverError::Protocol(
                            "selected v1/v2 capabilities omitted maxPreviewCodeUnits".to_owned(),
                        )
                    })?,
                )
                .map_err(|error| SchemaV2DriverError::Protocol(error.to_string()))?,
            ),
        };
        let aggregate_limits = match selected {
            kernel_v2::ProtocolVersion::V2 => Some(
                result
                    .capabilities
                    .aggregate_limits()
                    .map_err(|error| SchemaV2DriverError::Protocol(error.to_string()))?,
            ),
            kernel_v2::ProtocolVersion::V0 | kernel_v2::ProtocolVersion::V1 => None,
            kernel_v2::ProtocolVersion::V3 => {
                return Err(SchemaV2DriverError::Protocol(
                    "the schema-v2 conformance driver does not support kernel v3".to_owned(),
                ));
            }
        };
        Ok(Self {
            handler,
            protocol: selected,
            preview_limits,
            aggregate_limits,
            next_request_id: 2,
            seen_server_ids,
        })
    }

    const fn protocol(&self) -> kernel_v2::ProtocolVersion {
        self.protocol
    }

    fn exchange(
        &mut self,
        request: SchemaV2Request,
    ) -> Result<SchemaV2ExchangeResult, SchemaV2DriverError> {
        match self.protocol {
            kernel_v2::ProtocolVersion::V0 => self.exchange_v0(request),
            kernel_v2::ProtocolVersion::V1 => self.exchange_v1(request),
            kernel_v2::ProtocolVersion::V2 => self.exchange_v2(request),
            kernel_v2::ProtocolVersion::V3 => Err(SchemaV2DriverError::Protocol(
                "the schema-v2 conformance driver does not support kernel v3".to_owned(),
            )),
        }
    }

    fn exchange_v0(
        &mut self,
        request: SchemaV2Request,
    ) -> Result<SchemaV2ExchangeResult, SchemaV2DriverError> {
        let request_kind = request.kind();
        let request = RequestEnvelope::new(
            SESSION_ID,
            self.next_request_message_id(),
            match request {
                SchemaV2Request::Execute(request) => Request::Execute(request),
                SchemaV2Request::Inspect(request) => Request::Inspect(InspectRequest {
                    name: request.name,
                    range: MatrixRange {
                        start: request.range.start,
                        size: request.range.size,
                    },
                    max_elements: request.max_elements,
                }),
                SchemaV2Request::ListWorkspace => Request::ListWorkspace(ListWorkspaceRequest {}),
                SchemaV2Request::Shutdown => Request::Shutdown(ShutdownRequest {}),
            },
        );
        request
            .validate()
            .map_err(|error| SchemaV2DriverError::Protocol(error.to_string()))?;
        let messages = self
            .handler
            .handle_session(KernelSessionRequest::V0(&request))?;
        let mut response = None;
        for message in messages {
            match message {
                KernelSessionMessage::V0(ServerMessage::Event(event)) => {
                    event
                        .validate()
                        .map_err(|error| SchemaV2DriverError::Protocol(error.to_string()))?;
                    validate_schema_v2_server_identity(
                        &event.session_id,
                        event.message_id,
                        &mut self.seen_server_ids,
                    )?;
                }
                KernelSessionMessage::V0(ServerMessage::Response(envelope)) => {
                    envelope
                        .validate()
                        .map_err(|error| SchemaV2DriverError::Protocol(error.to_string()))?;
                    if !envelope.is_reply_to(&request) {
                        return Err(SchemaV2DriverError::Protocol(
                            "v0 response is not correlated to its request".to_owned(),
                        ));
                    }
                    validate_schema_v2_server_identity(
                        &envelope.session_id,
                        envelope.message_id.clone(),
                        &mut self.seen_server_ids,
                    )?;
                    if response.replace(envelope).is_some() {
                        return Err(SchemaV2DriverError::Protocol(format!(
                            "kernel returned multiple responses for request `{}`",
                            request.message_id
                        )));
                    }
                }
                KernelSessionMessage::Bootstrap(_)
                | KernelSessionMessage::V1(_)
                | KernelSessionMessage::V2(_)
                | KernelSessionMessage::V3(_) => {
                    return Err(SchemaV2DriverError::Protocol(
                        "kernel emitted a non-v0 post-bootstrap message".to_owned(),
                    ));
                }
            }
        }
        let response = response.ok_or_else(|| {
            SchemaV2DriverError::Protocol(format!(
                "kernel returned no response for request `{}`",
                request.message_id
            ))
        })?;
        if response.ok {
            let result = response.result.ok_or_else(|| {
                SchemaV2DriverError::Protocol("successful response omitted its result".to_owned())
            })?;
            schema_v2_response_from_v0(request_kind, result).map(SchemaV2ExchangeResult::Success)
        } else {
            response
                .error
                .map(SchemaV2ExchangeResult::Failure)
                .ok_or_else(|| {
                    SchemaV2DriverError::Protocol("failed response omitted its error".to_owned())
                })
        }
    }

    fn exchange_v1(
        &mut self,
        request: SchemaV2Request,
    ) -> Result<SchemaV2ExchangeResult, SchemaV2DriverError> {
        let request_kind = request.kind();
        let limits = self.preview_limits.ok_or_else(|| {
            SchemaV2DriverError::Protocol("v1 session omitted preview limits".to_owned())
        })?;
        let request = kernel_v1::RequestEnvelope::new(
            SESSION_ID,
            self.next_request_message_id(),
            match request {
                SchemaV2Request::Execute(request) => kernel_v1::Request::Execute(request),
                SchemaV2Request::Inspect(request) => kernel_v1::Request::Inspect(request),
                SchemaV2Request::ListWorkspace => {
                    kernel_v1::Request::ListWorkspace(ListWorkspaceRequest {})
                }
                SchemaV2Request::Shutdown => kernel_v1::Request::Shutdown(ShutdownRequest {}),
            },
        );
        request
            .validate(&limits)
            .map_err(|error| SchemaV2DriverError::Protocol(error.to_string()))?;
        let messages = self
            .handler
            .handle_session(KernelSessionRequest::V1(&request))?;
        let mut response = None;
        for message in messages {
            match message {
                KernelSessionMessage::V1(kernel_v1::ServerMessage::Event(event)) => {
                    event
                        .validate()
                        .map_err(|error| SchemaV2DriverError::Protocol(error.to_string()))?;
                    validate_schema_v2_server_identity(
                        &event.session_id,
                        event.message_id,
                        &mut self.seen_server_ids,
                    )?;
                }
                KernelSessionMessage::V1(kernel_v1::ServerMessage::Response(envelope)) => {
                    envelope
                        .validate_for_request(&request, &limits)
                        .map_err(|error| SchemaV2DriverError::Protocol(error.to_string()))?;
                    validate_schema_v2_server_identity(
                        &envelope.session_id,
                        envelope.message_id.clone(),
                        &mut self.seen_server_ids,
                    )?;
                    if response.replace(envelope).is_some() {
                        return Err(SchemaV2DriverError::Protocol(format!(
                            "kernel returned multiple responses for request `{}`",
                            request.message_id
                        )));
                    }
                }
                KernelSessionMessage::Bootstrap(_)
                | KernelSessionMessage::V0(_)
                | KernelSessionMessage::V2(_)
                | KernelSessionMessage::V3(_) => {
                    return Err(SchemaV2DriverError::Protocol(
                        "kernel emitted a non-v1 post-bootstrap message".to_owned(),
                    ));
                }
            }
        }
        let response = response.ok_or_else(|| {
            SchemaV2DriverError::Protocol(format!(
                "kernel returned no response for request `{}`",
                request.message_id
            ))
        })?;
        if response.ok {
            response
                .result
                .ok_or_else(|| {
                    SchemaV2DriverError::Protocol(
                        "successful response omitted its result".to_owned(),
                    )
                })
                .and_then(|result| schema_v2_response_from_v1(request_kind, result))
                .map(SchemaV2ExchangeResult::Success)
        } else {
            response
                .error
                .map(SchemaV2ExchangeResult::Failure)
                .ok_or_else(|| {
                    SchemaV2DriverError::Protocol("failed response omitted its error".to_owned())
                })
        }
    }

    fn exchange_v2(
        &mut self,
        request: SchemaV2Request,
    ) -> Result<SchemaV2ExchangeResult, SchemaV2DriverError> {
        let request_kind = request.kind();
        let limits = self.aggregate_limits.ok_or_else(|| {
            SchemaV2DriverError::Protocol("v2 session omitted aggregate limits".to_owned())
        })?;
        let request = kernel_v2::RequestEnvelope::new(
            SESSION_ID,
            self.next_request_message_id(),
            match request {
                SchemaV2Request::Execute(request) => kernel_v2::Request::Execute(request),
                SchemaV2Request::Inspect(request) => kernel_v2::Request::Inspect(request),
                SchemaV2Request::ListWorkspace => {
                    kernel_v2::Request::ListWorkspace(ListWorkspaceRequest {})
                }
                SchemaV2Request::Shutdown => kernel_v2::Request::Shutdown(ShutdownRequest {}),
            },
        );
        request
            .validate(&limits)
            .map_err(|error| SchemaV2DriverError::Protocol(error.to_string()))?;
        let messages = self
            .handler
            .handle_session(KernelSessionRequest::V2(&request))?;
        let mut response = None;
        for message in messages {
            match message {
                KernelSessionMessage::V2(kernel_v2::ServerMessage::Event(event)) => {
                    event
                        .validate()
                        .map_err(|error| SchemaV2DriverError::Protocol(error.to_string()))?;
                    validate_schema_v2_server_identity(
                        &event.session_id,
                        event.message_id,
                        &mut self.seen_server_ids,
                    )?;
                }
                KernelSessionMessage::V2(kernel_v2::ServerMessage::Response(envelope)) => {
                    envelope
                        .validate_for_request(&request, &limits)
                        .map_err(|error| SchemaV2DriverError::Protocol(error.to_string()))?;
                    validate_schema_v2_server_identity(
                        &envelope.session_id,
                        envelope.message_id.clone(),
                        &mut self.seen_server_ids,
                    )?;
                    if response.replace(envelope).is_some() {
                        return Err(SchemaV2DriverError::Protocol(format!(
                            "kernel returned multiple responses for request `{}`",
                            request.message_id
                        )));
                    }
                }
                KernelSessionMessage::Bootstrap(_)
                | KernelSessionMessage::V0(_)
                | KernelSessionMessage::V1(_)
                | KernelSessionMessage::V3(_) => {
                    return Err(SchemaV2DriverError::Protocol(
                        "kernel emitted a non-v2 post-bootstrap message".to_owned(),
                    ));
                }
            }
        }
        let response = response.ok_or_else(|| {
            SchemaV2DriverError::Protocol(format!(
                "kernel returned no response for request `{}`",
                request.message_id
            ))
        })?;
        if response.ok {
            response
                .result
                .ok_or_else(|| {
                    SchemaV2DriverError::Protocol(
                        "successful response omitted its result".to_owned(),
                    )
                })
                .and_then(|result| schema_v2_response_from_v2(request_kind, result, limits))
                .map(SchemaV2ExchangeResult::Success)
        } else {
            response
                .error
                .map(SchemaV2ExchangeResult::Failure)
                .ok_or_else(|| {
                    SchemaV2DriverError::Protocol("failed response omitted its error".to_owned())
                })
        }
    }

    fn next_request_message_id(&mut self) -> String {
        let message_id = format!("cli-v2-{}", self.next_request_id);
        self.next_request_id += 1;
        message_id
    }
}

fn schema_v2_response_type_mismatch() -> SchemaV2DriverError {
    SchemaV2DriverError::Protocol("response type does not match the request".to_owned())
}

fn schema_v2_response_from_v0(
    request: SchemaV2RequestKind,
    result: ResponseResult,
) -> Result<SchemaV2Response, SchemaV2DriverError> {
    match (request, result) {
        (SchemaV2RequestKind::Execute, ResponseResult::Execute(result)) => {
            Ok(SchemaV2Response::Execute(result))
        }
        (SchemaV2RequestKind::Inspect, ResponseResult::Inspect(preview)) => Ok(
            SchemaV2Response::Inspect(SchemaV2InspectPreview::Matrix(kernel_v1::MatrixPreview {
                class: preview.class,
                dimensions: preview.dimensions,
                complex: false,
                selected_range: kernel_v1::MatrixRange {
                    start: preview.selected_range.start,
                    size: preview.selected_range.size,
                },
                values: preview
                    .values
                    .into_iter()
                    .map(|value| match value {
                        PreviewValue::Number { value } => kernel_v1::PreviewValue::Number { value },
                        PreviewValue::Complex { real, imaginary } => {
                            kernel_v1::PreviewValue::Complex { real, imaginary }
                        }
                        PreviewValue::Logical { value } => {
                            kernel_v1::PreviewValue::Logical { value }
                        }
                        PreviewValue::Text { value } => kernel_v1::PreviewValue::Text { value },
                        PreviewValue::Special { value } => {
                            kernel_v1::PreviewValue::Special { value }
                        }
                        PreviewValue::Missing => kernel_v1::PreviewValue::Missing,
                    })
                    .collect(),
                truncation: preview.truncation,
            })),
        ),
        (SchemaV2RequestKind::ListWorkspace, ResponseResult::ListWorkspace(result)) => {
            Ok(SchemaV2Response::ListWorkspace(result))
        }
        (SchemaV2RequestKind::Shutdown, ResponseResult::Shutdown(_)) => {
            Ok(SchemaV2Response::Shutdown)
        }
        _ => Err(schema_v2_response_type_mismatch()),
    }
}

fn schema_v2_response_from_v1(
    request: SchemaV2RequestKind,
    result: kernel_v1::ResponseResult,
) -> Result<SchemaV2Response, SchemaV2DriverError> {
    match (request, result) {
        (SchemaV2RequestKind::Execute, kernel_v1::ResponseResult::Execute(result)) => {
            Ok(SchemaV2Response::Execute(result))
        }
        (SchemaV2RequestKind::Inspect, kernel_v1::ResponseResult::Inspect(preview)) => Ok(
            SchemaV2Response::Inspect(SchemaV2InspectPreview::Matrix(preview)),
        ),
        (SchemaV2RequestKind::ListWorkspace, kernel_v1::ResponseResult::ListWorkspace(result)) => {
            Ok(SchemaV2Response::ListWorkspace(result))
        }
        (SchemaV2RequestKind::Shutdown, kernel_v1::ResponseResult::Shutdown(_)) => {
            Ok(SchemaV2Response::Shutdown)
        }
        _ => Err(schema_v2_response_type_mismatch()),
    }
}

fn schema_v2_response_from_v2(
    request: SchemaV2RequestKind,
    result: kernel_v2::ResponseResult,
    limits: kernel_v2::AggregateLimits,
) -> Result<SchemaV2Response, SchemaV2DriverError> {
    match (request, result) {
        (SchemaV2RequestKind::Execute, kernel_v2::ResponseResult::Execute(result)) => {
            Ok(SchemaV2Response::Execute(result))
        }
        (
            SchemaV2RequestKind::Inspect,
            kernel_v2::ResponseResult::Inspect(kernel_v2::InspectPreview::Matrix(preview)),
        ) => Ok(SchemaV2Response::Inspect(SchemaV2InspectPreview::Matrix(
            preview,
        ))),
        (
            SchemaV2RequestKind::Inspect,
            kernel_v2::ResponseResult::Inspect(kernel_v2::InspectPreview::Aggregate(preview)),
        ) => Ok(SchemaV2Response::Inspect(
            SchemaV2InspectPreview::Aggregate { preview, limits },
        )),
        (SchemaV2RequestKind::ListWorkspace, kernel_v2::ResponseResult::ListWorkspace(result)) => {
            Ok(SchemaV2Response::ListWorkspace(result))
        }
        (SchemaV2RequestKind::Shutdown, kernel_v2::ResponseResult::Shutdown(_)) => {
            Ok(SchemaV2Response::Shutdown)
        }
        _ => Err(schema_v2_response_type_mismatch()),
    }
}

fn validate_schema_v2_server_identity(
    session_id: &str,
    message_id: String,
    seen_server_ids: &mut HashSet<String>,
) -> Result<(), SchemaV2DriverError> {
    if session_id != SESSION_ID {
        return Err(SchemaV2DriverError::Protocol(
            "server message session does not match the CLI session".to_owned(),
        ));
    }
    if !seen_server_ids.insert(message_id) {
        return Err(SchemaV2DriverError::Protocol(
            "kernel reused a server message ID".to_owned(),
        ));
    }
    Ok(())
}

fn map_schema_v2_driver_error(error: SchemaV2DriverError) -> ConformanceRunError {
    match error {
        SchemaV2DriverError::Protocol(detail) => ConformanceRunError::Protocol(detail),
    }
}

fn execute_conformance_case_v2<Handler: KernelSessionHandler>(
    handler: &mut Handler,
    case_id: &str,
    code: String,
    source_name: &str,
    requires_kernel_v2: bool,
) -> Result<JsonValue, ConformanceRunError> {
    let mut driver =
        SchemaV2SessionDriver::initialize(handler).map_err(map_schema_v2_driver_error)?;
    let result = if requires_kernel_v2 && driver.protocol() != kernel_v2::ProtocolVersion::V2 {
        Ok(error_observation_v2(case_id, "unsupported-payload"))
    } else if driver.protocol() == kernel_v2::ProtocolVersion::V0 {
        Err(ConformanceRunError::Capability(format!(
            "kernel selected {}",
            kernel_v2::ProtocolVersion::V0.as_str()
        )))
    } else {
        observe_conformance_case_v2(&mut driver, case_id, code, source_name)
    };
    let shutdown = driver
        .exchange(SchemaV2Request::Shutdown)
        .map_err(map_schema_v2_driver_error);
    match shutdown {
        Ok(SchemaV2ExchangeResult::Success(SchemaV2Response::Shutdown)) => result,
        Ok(SchemaV2ExchangeResult::Success(_)) => Err(ConformanceRunError::Protocol(
            "shutdown response type does not match the request".to_owned(),
        )),
        Ok(SchemaV2ExchangeResult::Failure(error)) => {
            Err(ConformanceRunError::Internal(error.category))
        }
        Err(error) => Err(error),
    }
}

fn observe_conformance_case_v2<Handler: KernelSessionHandler>(
    driver: &mut SchemaV2SessionDriver<'_, Handler>,
    case_id: &str,
    code: String,
    source_name: &str,
) -> Result<JsonValue, ConformanceRunError> {
    match driver
        .exchange(SchemaV2Request::Execute(ExecuteRequest {
            code,
            source_name: source_name.to_owned(),
            mode: ExecutionMode::File,
        }))
        .map_err(map_schema_v2_driver_error)?
    {
        SchemaV2ExchangeResult::Success(SchemaV2Response::Execute(result))
            if result.interrupted =>
        {
            return Ok(error_observation_v2(case_id, "other"));
        }
        SchemaV2ExchangeResult::Success(SchemaV2Response::Execute(_)) => {}
        SchemaV2ExchangeResult::Success(_) => {
            return Err(ConformanceRunError::Protocol(
                "execute response type does not match the request".to_owned(),
            ));
        }
        SchemaV2ExchangeResult::Failure(error) => {
            if is_internal_category(&error.category) {
                return Err(ConformanceRunError::Internal(error.category));
            }
            return Ok(error_observation_v2(
                case_id,
                normalized_error_category(&error.category),
            ));
        }
    }
    inspect_conformance_result_v2(driver, case_id)
}

fn inspect_conformance_result_v2<Handler: KernelSessionHandler>(
    driver: &mut SchemaV2SessionDriver<'_, Handler>,
    case_id: &str,
) -> Result<JsonValue, ConformanceRunError> {
    let variables = match driver
        .exchange(SchemaV2Request::ListWorkspace)
        .map_err(map_schema_v2_driver_error)?
    {
        SchemaV2ExchangeResult::Success(SchemaV2Response::ListWorkspace(summary)) => {
            summary.variables
        }
        SchemaV2ExchangeResult::Success(_) => {
            return Err(ConformanceRunError::Protocol(
                "listWorkspace response type does not match the request".to_owned(),
            ));
        }
        SchemaV2ExchangeResult::Failure(error) => {
            return Err(ConformanceRunError::Internal(error.category));
        }
    };
    let Some(summary) = variables
        .into_iter()
        .find(|variable| variable.name == CONFORMANCE_RESULT)
    else {
        return Ok(error_observation_v2(case_id, "undefined-name"));
    };
    let numel = match conformance_v2_summary_numel(&summary) {
        Ok(numel) => numel,
        Err(category) => return Ok(error_observation_v2(case_id, category)),
    };
    if matches!(summary.class.as_str(), "cell" | "struct" | "table")
        && driver.protocol() != kernel_v2::ProtocolVersion::V2
    {
        return Ok(error_observation_v2(case_id, "unsupported-payload"));
    }
    let preview = match driver
        .exchange(SchemaV2Request::Inspect(kernel_v1::InspectRequest {
            name: CONFORMANCE_RESULT.to_owned(),
            range: kernel_v1::MatrixRange {
                start: vec![1; summary.dimensions.len()],
                size: summary.dimensions.clone(),
            },
            max_elements: conformance_v2_inspect_max_elements(&summary, numel),
        }))
        .map_err(map_schema_v2_driver_error)?
    {
        SchemaV2ExchangeResult::Success(SchemaV2Response::Inspect(preview)) => preview,
        SchemaV2ExchangeResult::Success(_) => {
            return Err(ConformanceRunError::Protocol(
                "inspect response type does not match the request".to_owned(),
            ));
        }
        SchemaV2ExchangeResult::Failure(error) => {
            return map_schema_v2_inspect_error(case_id, error);
        }
    };
    let value = match preview {
        SchemaV2InspectPreview::Matrix(preview) => observation_value_v2(&summary, preview, numel),
        SchemaV2InspectPreview::Aggregate { preview, limits } => {
            aggregate_observation_value_v2(&summary, preview, numel, &limits)
        }
    };
    Ok(match value {
        Ok(value) => bounded_ok_observation_v2(case_id, value),
        Err(error) => error_observation_v2(case_id, error.category()),
    })
}

fn conformance_v2_summary_numel(summary: &VariableSummary) -> Result<u64, &'static str> {
    if summary.dimensions.len() < 2
        || (summary.class == "table" && summary.dimensions.len() != 2)
        || summary
            .dimensions
            .iter()
            .any(|dimension| *dimension > kernel_v1::MAX_SAFE_JSON_INTEGER)
    {
        return Err("invalid-observation");
    }
    let numel = checked_dimension_product(&summary.dimensions).ok_or("payload-limit")?;
    if numel > MAX_PREVIEW_ELEMENTS
        || conformance_v2_inspect_max_elements(summary, numel) > MAX_PREVIEW_ELEMENTS
    {
        return Err("payload-limit");
    }
    Ok(numel)
}

fn conformance_v2_inspect_max_elements(summary: &VariableSummary, numel: u64) -> u64 {
    if summary.class == "table" && summary.dimensions.len() == 2 {
        summary.dimensions[1].max(1)
    } else {
        numel.max(1)
    }
}

fn map_schema_v2_inspect_error(
    case_id: &str,
    error: ProtocolError,
) -> Result<JsonValue, ConformanceRunError> {
    match error.category.as_str() {
        "workspace.unsupportedValue" => Ok(error_observation_v2(case_id, "unsupported-payload")),
        "workspace.previewLimit" | "workspace.previewDepth" => {
            Ok(error_observation_v2(case_id, "payload-limit"))
        }
        "workspace.cyclicValue" => Ok(error_observation_v2(case_id, "cyclic-payload")),
        "engine.invalidPreview" => Ok(error_observation_v2(case_id, "invalid-observation")),
        _ => Err(ConformanceRunError::Internal(error.category)),
    }
}

fn observe_conformance_case<Handler: RequestHandler>(
    driver: &mut SessionDriver<'_, Handler>,
    case_id: &str,
    code: String,
    source_name: &str,
    event_output: &mut io::Sink,
    event_errors: &mut io::Sink,
) -> Result<JsonValue, String> {
    let initialize = Request::Initialize(InitializeRequest {
        client: ImplementationInfo {
            name: "openmat-conformance".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
        },
        supported_protocols: vec![PROTOCOL_V0.to_owned()],
        capabilities: Capabilities {
            execution_modes: vec![ExecutionMode::File],
            display_mime_types: vec!["text/plain".to_owned()],
            max_preview_elements: MAX_PREVIEW_ELEMENTS,
            interrupt: false,
            workspace_delta: false,
        },
    });
    match driver.exchange(
        initialize,
        ExpectedResponse::Initialize,
        "<protocol>",
        event_output,
        event_errors,
    )? {
        ExchangeResult::Success(_) => {}
        ExchangeResult::Failure(error) => return Err(error.category),
    }

    let execute = Request::Execute(ExecuteRequest {
        code,
        source_name: source_name.to_owned(),
        mode: ExecutionMode::File,
    });
    match driver.exchange(
        execute,
        ExpectedResponse::Execute,
        source_name,
        event_output,
        event_errors,
    )? {
        ExchangeResult::Success(ResponseResult::Execute(result)) if result.interrupted => {
            return Ok(error_observation(case_id, "other"));
        }
        ExchangeResult::Success(ResponseResult::Execute(_)) => {}
        ExchangeResult::Success(_) => unreachable!("response type checked by exchange"),
        ExchangeResult::Failure(error) => {
            if is_internal_category(&error.category) {
                return Err(error.category);
            }
            return Ok(error_observation(
                case_id,
                normalized_error_category(&error.category),
            ));
        }
    }
    inspect_conformance_result(driver, case_id, event_output, event_errors)
}

fn inspect_conformance_result<Handler: RequestHandler>(
    driver: &mut SessionDriver<'_, Handler>,
    case_id: &str,
    event_output: &mut io::Sink,
    event_errors: &mut io::Sink,
) -> Result<JsonValue, String> {
    let variables = match driver.exchange(
        Request::ListWorkspace(ListWorkspaceRequest {}),
        ExpectedResponse::ListWorkspace,
        "<workspace>",
        event_output,
        event_errors,
    )? {
        ExchangeResult::Success(ResponseResult::ListWorkspace(summary)) => summary.variables,
        ExchangeResult::Success(_) => unreachable!("response type checked by exchange"),
        ExchangeResult::Failure(error) => return Err(error.category),
    };
    let Some(summary) = variables
        .into_iter()
        .find(|variable| variable.name == CONFORMANCE_RESULT)
    else {
        return Ok(error_observation(case_id, "undefined-name"));
    };
    let Some(numel) = checked_dimension_product(&summary.dimensions) else {
        return Ok(error_observation(case_id, "unsupported-payload"));
    };
    if summary.dimensions.len() < 2 || numel > MAX_PREVIEW_ELEMENTS {
        return Ok(error_observation(case_id, "unsupported-payload"));
    }
    let preview = match driver.exchange(
        Request::Inspect(InspectRequest {
            name: CONFORMANCE_RESULT.to_owned(),
            range: MatrixRange {
                start: vec![1; summary.dimensions.len()],
                size: summary.dimensions.clone(),
            },
            max_elements: numel.max(1),
        }),
        ExpectedResponse::Inspect,
        "<workspace>",
        event_output,
        event_errors,
    )? {
        ExchangeResult::Success(ResponseResult::Inspect(preview)) => preview,
        ExchangeResult::Success(_) => unreachable!("response type checked by exchange"),
        ExchangeResult::Failure(error) if error.category == "workspace.unsupportedValue" => {
            return Ok(error_observation(case_id, "unsupported-payload"));
        }
        ExchangeResult::Failure(error) => return Err(error.category),
    };
    Ok(match observation_value(&summary, preview, numel) {
        Some(value) => ok_observation(case_id, value),
        None => error_observation(case_id, "unsupported-payload"),
    })
}

fn checked_dimension_product(dimensions: &[u64]) -> Option<u64> {
    dimensions
        .iter()
        .try_fold(1_u64, |product, dimension| product.checked_mul(*dimension))
}

fn observation_value(
    summary: &VariableSummary,
    preview: MatrixPreview,
    numel: u64,
) -> Option<JsonValue> {
    if preview.class != summary.class
        || preview.dimensions != summary.dimensions
        || preview.truncation.truncated
        || preview.truncation.omitted_elements != 0
        || u64::try_from(preview.values.len()).ok()? != numel
    {
        return None;
    }
    let ndims = u64::try_from(summary.dimensions.len()).ok()?;
    let mut value = json!({
        "class": summary.class,
        "size": summary.dimensions,
        "ndims": ndims,
        "numel": numel,
    });
    let object = value.as_object_mut()?;
    match summary.class.as_str() {
        "double" | "single" => {
            let mut real = Vec::with_capacity(preview.values.len());
            let mut imaginary = Vec::with_capacity(preview.values.len());
            for item in preview.values {
                match item {
                    PreviewValue::Number { value } => {
                        real.push(finite_number_string_for_class(&summary.class, value)?);
                        imaginary.push("0".to_owned());
                    }
                    PreviewValue::Complex {
                        real: real_value,
                        imaginary: imaginary_value,
                    } => {
                        real.push(finite_number_string_for_class(&summary.class, real_value)?);
                        imaginary.push(finite_number_string_for_class(
                            &summary.class,
                            imaginary_value,
                        )?);
                    }
                    PreviewValue::Special { value } => {
                        real.push(special_number_string(&value)?.to_owned());
                        imaginary.push("0".to_owned());
                    }
                    PreviewValue::Logical { .. }
                    | PreviewValue::Text { .. }
                    | PreviewValue::Missing => return None,
                }
            }
            object.insert("kind".to_owned(), json!("numeric"));
            object.insert("real".to_owned(), json!(real));
            object.insert("imag".to_owned(), json!(imaginary));
        }
        "logical" => {
            let logical = preview
                .values
                .into_iter()
                .map(|item| match item {
                    PreviewValue::Logical { value } => Some(value),
                    _ => None,
                })
                .collect::<Option<Vec<_>>>()?;
            object.insert("kind".to_owned(), json!("logical"));
            object.insert("logical".to_owned(), json!(logical));
        }
        "string" => {
            let strings = preview
                .values
                .into_iter()
                .map(|item| match item {
                    PreviewValue::Text { value } => Some(value),
                    _ => None,
                })
                .collect::<Option<Vec<_>>>()?;
            object.insert("kind".to_owned(), json!("string"));
            object.insert("missing".to_owned(), json!(vec![false; strings.len()]));
            object.insert("string".to_owned(), json!(strings));
        }
        class
            if !is_builtin_observation_class(class)
                && preview
                    .values
                    .iter()
                    .all(|item| matches!(item, PreviewValue::Missing)) =>
        {
            object.insert("kind".to_owned(), json!("object"));
        }
        _ => return None,
    }
    Some(value)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ObservationV2Error {
    UnsupportedPayload,
    PayloadLimit,
    InvalidObservation,
}

impl ObservationV2Error {
    const fn category(self) -> &'static str {
        match self {
            Self::UnsupportedPayload => "unsupported-payload",
            Self::PayloadLimit => "payload-limit",
            Self::InvalidObservation => "invalid-observation",
        }
    }
}

fn observation_value_v2(
    summary: &VariableSummary,
    preview: kernel_v1::MatrixPreview,
    numel: u64,
) -> Result<JsonValue, ObservationV2Error> {
    if summary.dimensions.len() < 2
        || summary
            .dimensions
            .iter()
            .any(|dimension| *dimension > kernel_v1::MAX_SAFE_JSON_INTEGER)
        || checked_dimension_product(&summary.dimensions) != Some(numel)
        || preview.class != summary.class
        || preview.dimensions != summary.dimensions
        || preview.complex != summary.complex
        || preview.selected_range.start != vec![1; summary.dimensions.len()]
        || preview.selected_range.size != summary.dimensions
    {
        return Err(ObservationV2Error::InvalidObservation);
    }
    if numel > MAX_PREVIEW_ELEMENTS
        || preview.truncation.truncated
        || preview.truncation.omitted_elements != 0
    {
        return Err(ObservationV2Error::PayloadLimit);
    }
    if u64::try_from(preview.values.len()).ok() != Some(numel) {
        return Err(ObservationV2Error::InvalidObservation);
    }
    let ndims = u64::try_from(summary.dimensions.len())
        .map_err(|_| ObservationV2Error::InvalidObservation)?;
    let mut value = json!({
        "class": summary.class,
        "size": summary.dimensions,
        "ndims": ndims,
        "numel": numel,
        "complex": summary.complex,
    });
    let object = value
        .as_object_mut()
        .ok_or(ObservationV2Error::InvalidObservation)?;
    if summary.complex && matches!(summary.class.as_str(), "logical" | "char" | "string") {
        return Err(ObservationV2Error::InvalidObservation);
    }
    let payload = match summary.class.as_str() {
        "double" | "single" => normalize_v2_float(&summary.class, summary.complex, preview.values)
            .ok_or(ObservationV2Error::InvalidObservation)?,
        "int8" | "uint8" | "int16" | "uint16" | "int32" | "uint32" | "int64" | "uint64" => {
            normalize_v2_integer(&summary.class, summary.complex, preview.values)
                .ok_or(ObservationV2Error::InvalidObservation)?
        }
        "logical" if !summary.complex => {
            normalize_v2_logical(preview.values).ok_or(ObservationV2Error::InvalidObservation)?
        }
        "char" if !summary.complex => {
            normalize_v2_char(preview.values).ok_or(ObservationV2Error::InvalidObservation)?
        }
        "string" if !summary.complex => {
            normalize_v2_string(preview.values).ok_or(ObservationV2Error::InvalidObservation)?
        }
        _ => return Err(ObservationV2Error::UnsupportedPayload),
    };
    object.extend(payload);
    Ok(value)
}

fn aggregate_observation_value_v2(
    summary: &VariableSummary,
    preview: kernel_v2::AggregatePreview,
    numel: u64,
    limits: &kernel_v2::AggregateLimits,
) -> Result<JsonValue, ObservationV2Error> {
    let request_max_elements = conformance_v2_inspect_max_elements(summary, numel);
    preview
        .validate_for_request(limits, request_max_elements)
        .map_err(|_| ObservationV2Error::InvalidObservation)?;
    let ndims = u64::try_from(summary.dimensions.len())
        .map_err(|_| ObservationV2Error::InvalidObservation)?;
    match preview {
        kernel_v2::AggregatePreview::Cell {
            dimensions,
            selected_range,
            items,
            truncation,
            ..
        } => {
            if summary.class != "cell"
                || summary.complex
                || dimensions != summary.dimensions
                || selected_range.start != vec![1; summary.dimensions.len()]
                || selected_range.size != summary.dimensions
            {
                return Err(ObservationV2Error::InvalidObservation);
            }
            if truncation.truncated || truncation.omitted_elements != 0 {
                return Err(ObservationV2Error::PayloadLimit);
            }
            if u64::try_from(items.len()).ok() != Some(numel) {
                return Err(ObservationV2Error::InvalidObservation);
            }
            let items = items
                .into_iter()
                .map(exact_observation_value_v2)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(json!({
                "class": "cell",
                "size": summary.dimensions,
                "ndims": ndims,
                "numel": numel,
                "complex": false,
                "kind": "cell",
                "items": items,
            }))
        }
        kernel_v2::AggregatePreview::Struct {
            dimensions,
            selected_range,
            fields,
            records,
            truncation,
            ..
        } => {
            if summary.class != "struct"
                || summary.complex
                || dimensions != summary.dimensions
                || selected_range.start != vec![1; summary.dimensions.len()]
                || selected_range.size != summary.dimensions
            {
                return Err(ObservationV2Error::InvalidObservation);
            }
            if truncation.truncated || truncation.omitted_elements != 0 {
                return Err(ObservationV2Error::PayloadLimit);
            }
            if u64::try_from(records.len()).ok() != Some(numel) {
                return Err(ObservationV2Error::InvalidObservation);
            }
            let records = records
                .into_iter()
                .map(|record| exact_struct_record_v2(&fields, record))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(json!({
                "class": "struct",
                "size": summary.dimensions,
                "ndims": ndims,
                "numel": numel,
                "complex": false,
                "kind": "struct",
                "fields": fields,
                "records": records,
            }))
        }
        table @ kernel_v2::AggregatePreview::Table { .. } => {
            table_aggregate_observation_value_v2(summary, table, numel, ndims)
        }
    }
}

fn table_aggregate_observation_value_v2(
    summary: &VariableSummary,
    preview: kernel_v2::AggregatePreview,
    numel: u64,
    ndims: u64,
) -> Result<JsonValue, ObservationV2Error> {
    let kernel_v2::AggregatePreview::Table {
        dimensions,
        selected_range,
        variable_names,
        variables,
        truncation,
        ..
    } = preview
    else {
        return Err(ObservationV2Error::InvalidObservation);
    };
    if summary.class != "table"
        || summary.complex
        || summary.dimensions.len() != 2
        || dimensions != summary.dimensions
        || selected_range.start != vec![1, 1]
        || selected_range.size != summary.dimensions
    {
        return Err(ObservationV2Error::InvalidObservation);
    }
    if truncation.truncated || truncation.omitted_elements != 0 {
        return Err(ObservationV2Error::PayloadLimit);
    }
    let variable_count = usize::try_from(summary.dimensions[1])
        .map_err(|_| ObservationV2Error::InvalidObservation)?;
    if variable_names.len() != variable_count || variables.len() != variable_count {
        return Err(ObservationV2Error::InvalidObservation);
    }
    let variables = variables
        .into_iter()
        .map(exact_observation_value_v2)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(json!({
        "class": "table",
        "size": summary.dimensions,
        "ndims": ndims,
        "numel": numel,
        "complex": false,
        "kind": "table",
        "variableNames": variable_names,
        "variables": variables,
    }))
}

fn exact_observation_value_v2(
    value: kernel_v2::ExactValue,
) -> Result<JsonValue, ObservationV2Error> {
    let value_class = value.class.clone();
    let mut observation = json!({
        "class": value.class,
        "size": value.size,
        "ndims": value.ndims,
        "numel": value.numel,
        "complex": value.complex,
    });
    let object = observation
        .as_object_mut()
        .ok_or(ObservationV2Error::InvalidObservation)?;
    match value.payload {
        kernel_v2::ExactPayload::Numeric { real, imag } => {
            let real = normalize_exact_number_strings(&value_class, real)?;
            let imag = normalize_exact_number_strings(&value_class, imag)?;
            object.insert("kind".to_owned(), json!("numeric"));
            object.insert("real".to_owned(), json!(real));
            object.insert("imag".to_owned(), json!(imag));
        }
        kernel_v2::ExactPayload::Integer { integer } => {
            let integer = integer
                .into_iter()
                .map(|value| {
                    json!({
                        "real": value.real,
                        "imaginary": value.imaginary,
                    })
                })
                .collect::<Vec<_>>();
            object.insert("kind".to_owned(), json!("integer"));
            object.insert("integer".to_owned(), json!(integer));
        }
        kernel_v2::ExactPayload::Logical { logical } => {
            object.insert("kind".to_owned(), json!("logical"));
            object.insert("logical".to_owned(), json!(logical));
        }
        kernel_v2::ExactPayload::Char { code_units } => {
            object.insert("kind".to_owned(), json!("char"));
            object.insert("code_units".to_owned(), json!(code_units));
        }
        kernel_v2::ExactPayload::String {
            string_code_units,
            missing,
        } => {
            object.insert("kind".to_owned(), json!("string"));
            object.insert("string_code_units".to_owned(), json!(string_code_units));
            object.insert("missing".to_owned(), json!(missing));
        }
        kernel_v2::ExactPayload::Cell { items } => {
            let items = items
                .into_iter()
                .map(exact_observation_value_v2)
                .collect::<Result<Vec<_>, _>>()?;
            object.insert("kind".to_owned(), json!("cell"));
            object.insert("items".to_owned(), json!(items));
        }
        kernel_v2::ExactPayload::Struct { fields, records } => {
            let records = records
                .into_iter()
                .map(|record| exact_struct_record_v2(&fields, record))
                .collect::<Result<Vec<_>, _>>()?;
            object.insert("kind".to_owned(), json!("struct"));
            object.insert("fields".to_owned(), json!(fields));
            object.insert("records".to_owned(), json!(records));
        }
        kernel_v2::ExactPayload::Table {
            variable_names,
            variables,
        } => {
            let variables = variables
                .into_iter()
                .map(exact_observation_value_v2)
                .collect::<Result<Vec<_>, _>>()?;
            object.insert("kind".to_owned(), json!("table"));
            object.insert("variableNames".to_owned(), json!(variable_names));
            object.insert("variables".to_owned(), json!(variables));
        }
    }
    Ok(observation)
}

fn normalize_exact_number_strings(
    class: &str,
    values: Vec<String>,
) -> Result<Vec<String>, ObservationV2Error> {
    values
        .into_iter()
        .map(|value| {
            if matches!(value.as_str(), "NaN" | "+Inf" | "-Inf") {
                return Ok(value);
            }
            match class {
                "double" => value
                    .parse::<f64>()
                    .ok()
                    .filter(|parsed| parsed.is_finite())
                    .map(number_string),
                "single" => value
                    .parse::<f32>()
                    .ok()
                    .filter(|parsed| parsed.is_finite())
                    .map(number_string_f32),
                _ => None,
            }
            .ok_or(ObservationV2Error::InvalidObservation)
        })
        .collect()
}

fn exact_struct_record_v2(
    fields: &[String],
    record: kernel_v2::StructRecord,
) -> Result<JsonValue, ObservationV2Error> {
    if record.entries.len() != fields.len() {
        return Err(ObservationV2Error::InvalidObservation);
    }
    let mut entries = record.entries;
    let mut object = serde_json::Map::new();
    for field in fields {
        let Some(position) = entries.iter().position(|(name, _)| name == field) else {
            return Err(ObservationV2Error::InvalidObservation);
        };
        let (_, value) = entries.remove(position);
        object.insert(field.clone(), exact_observation_value_v2(value)?);
    }
    if !entries.is_empty() {
        return Err(ObservationV2Error::InvalidObservation);
    }
    Ok(JsonValue::Object(object))
}

fn normalize_v2_float(
    class: &str,
    complex: bool,
    values: Vec<kernel_v1::PreviewValue>,
) -> Option<serde_json::Map<String, JsonValue>> {
    let mut real = Vec::with_capacity(values.len());
    let mut imaginary = Vec::with_capacity(values.len());
    for item in values {
        match (complex, item) {
            (
                true,
                kernel_v1::PreviewValue::Complex {
                    real: real_value,
                    imaginary: imaginary_value,
                },
            ) => {
                real.push(finite_number_string_for_class(class, real_value)?);
                imaginary.push(finite_number_string_for_class(class, imaginary_value)?);
            }
            (false, kernel_v1::PreviewValue::Number { value }) => {
                real.push(finite_number_string_for_class(class, value)?);
                imaginary.push("0".to_owned());
            }
            (false, kernel_v1::PreviewValue::Special { value }) => {
                real.push(special_number_string(&value)?.to_owned());
                imaginary.push("0".to_owned());
            }
            _ => return None,
        }
    }
    json!({ "kind": "numeric", "real": real, "imag": imaginary })
        .as_object()
        .cloned()
}

fn normalize_v2_integer(
    class: &str,
    complex: bool,
    values: Vec<kernel_v1::PreviewValue>,
) -> Option<serde_json::Map<String, JsonValue>> {
    let mut integer = Vec::with_capacity(values.len());
    let mut has_nonzero_imaginary = false;
    for item in values {
        let kernel_v1::PreviewValue::Integer { real, imaginary } = item else {
            return None;
        };
        if !valid_integer_component(class, &real) || !valid_integer_component(class, &imaginary) {
            return None;
        }
        has_nonzero_imaginary |= imaginary != "0";
        integer.push(json!({ "real": real, "imaginary": imaginary }));
    }
    if complex != has_nonzero_imaginary {
        return None;
    }
    json!({ "kind": "integer", "integer": integer })
        .as_object()
        .cloned()
}

fn normalize_v2_logical(
    values: Vec<kernel_v1::PreviewValue>,
) -> Option<serde_json::Map<String, JsonValue>> {
    let logical = values
        .into_iter()
        .map(|item| match item {
            kernel_v1::PreviewValue::Logical { value } => Some(value),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;
    json!({ "kind": "logical", "logical": logical })
        .as_object()
        .cloned()
}

fn normalize_v2_char(
    values: Vec<kernel_v1::PreviewValue>,
) -> Option<serde_json::Map<String, JsonValue>> {
    let code_units = values
        .into_iter()
        .map(|item| match item {
            kernel_v1::PreviewValue::CharCodeUnit { value } => Some(value),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;
    json!({ "kind": "char", "code_units": code_units })
        .as_object()
        .cloned()
}

fn normalize_v2_string(
    values: Vec<kernel_v1::PreviewValue>,
) -> Option<serde_json::Map<String, JsonValue>> {
    let mut string_code_units = Vec::with_capacity(values.len());
    let mut missing_values = Vec::with_capacity(values.len());
    let mut aggregate_code_units = 0_u64;
    for item in values {
        let kernel_v1::PreviewValue::String {
            code_units,
            missing,
        } = item
        else {
            return None;
        };
        let code_unit_count = u64::try_from(code_units.len()).ok()?;
        if code_unit_count > kernel_v1::MAX_STRING_ELEMENT_CODE_UNITS
            || (missing && !code_units.is_empty())
        {
            return None;
        }
        aggregate_code_units = aggregate_code_units.checked_add(code_unit_count)?;
        if aggregate_code_units > kernel_v1::MAX_PREVIEW_CODE_UNITS {
            return None;
        }
        string_code_units.push(code_units);
        missing_values.push(missing);
    }
    json!({
        "kind": "string",
        "string_code_units": string_code_units,
        "missing": missing_values,
    })
    .as_object()
    .cloned()
}

fn finite_number_string(value: f64) -> Option<String> {
    value.is_finite().then(|| number_string(value))
}

#[allow(clippy::cast_possible_truncation)]
fn finite_number_string_for_class(class: &str, value: f64) -> Option<String> {
    match class {
        "double" => finite_number_string(value),
        "single" if value.is_finite() => {
            // The roundtrip check rejects every wire value that is not an exact
            // widening of one binary32 component.
            let single = value as f32;
            (f64::from(single).to_bits() == value.to_bits()).then(|| number_string_f32(single))
        }
        _ => None,
    }
}

fn valid_integer_component(class: &str, value: &str) -> bool {
    let unsigned = class.starts_with("uint");
    let bytes = value.as_bytes();
    let canonical = if unsigned {
        matches!(bytes, [b'0'] | [b'1'..=b'9', ..]) && bytes.iter().all(u8::is_ascii_digit)
    } else {
        match bytes {
            [b'0'] => true,
            [b'1'..=b'9', rest @ ..] | [b'-', b'1'..=b'9', rest @ ..] => {
                rest.iter().all(u8::is_ascii_digit)
            }
            _ => false,
        }
    };
    canonical
        && match class {
            "int8" => value.parse::<i8>().is_ok(),
            "uint8" => value.parse::<u8>().is_ok(),
            "int16" => value.parse::<i16>().is_ok(),
            "uint16" => value.parse::<u16>().is_ok(),
            "int32" => value.parse::<i32>().is_ok(),
            "uint32" => value.parse::<u32>().is_ok(),
            "int64" => value.parse::<i64>().is_ok(),
            "uint64" => value.parse::<u64>().is_ok(),
            _ => false,
        }
}

fn is_builtin_observation_class(class: &str) -> bool {
    matches!(
        class,
        "array"
            | "cell"
            | "char"
            | "double"
            | "function_handle"
            | "int8"
            | "int16"
            | "int32"
            | "int64"
            | "logical"
            | "nothing"
            | "object"
            | "single"
            | "string"
            | "struct"
            | "uint8"
            | "uint16"
            | "uint32"
            | "uint64"
    )
}

fn number_string(value: f64) -> String {
    if value == 0.0 && value.is_sign_negative() {
        "-0".to_owned()
    } else if !value.is_finite() {
        value.to_string()
    } else {
        // The MATLAB oracle serializes doubles with `sprintf("%.17g", value)`.
        // Rust's Display intentionally emits the shortest round-trippable
        // representation instead, so format all 17 significant digits and
        // apply the `%g` fixed/scientific cutoff explicitly.
        let scientific = format!("{value:.16e}");
        let (mantissa, exponent) = scientific
            .rsplit_once('e')
            .expect("Rust scientific formatting includes an exponent");
        let exponent = exponent
            .parse::<i32>()
            .expect("Rust scientific formatting uses a decimal exponent");

        if (-4..17).contains(&exponent) {
            let fractional_digits =
                usize::try_from(16 - exponent).expect("fixed %.17g precision is non-negative");
            let fixed = format!("{value:.fractional_digits$}");
            trim_decimal_fraction(&fixed).to_owned()
        } else {
            let sign = if exponent.is_negative() { '-' } else { '+' };
            format!(
                "{}e{sign}{:02}",
                trim_decimal_fraction(mantissa),
                exponent.unsigned_abs()
            )
        }
    }
}

fn number_string_f32(value: f32) -> String {
    if value == 0.0 && value.is_sign_negative() {
        "-0".to_owned()
    } else if !value.is_finite() {
        value.to_string()
    } else {
        // Match the oracle's `sprintf("%.9g", value)`: binary32 needs at
        // most nine significant decimal digits, and formatting the exact f64
        // widening keeps the rounding decision independent of host defaults.
        let widened = f64::from(value);
        let scientific = format!("{widened:.8e}");
        let (mantissa, exponent) = scientific
            .rsplit_once('e')
            .expect("Rust scientific formatting includes an exponent");
        let exponent = exponent
            .parse::<i32>()
            .expect("Rust scientific formatting uses a decimal exponent");

        if (-4..9).contains(&exponent) {
            let fractional_digits =
                usize::try_from(8 - exponent).expect("fixed %.9g precision is non-negative");
            let fixed = format!("{widened:.fractional_digits$}");
            trim_decimal_fraction(&fixed).to_owned()
        } else {
            let sign = if exponent.is_negative() { '-' } else { '+' };
            format!(
                "{}e{sign}{:02}",
                trim_decimal_fraction(mantissa),
                exponent.unsigned_abs()
            )
        }
    }
}

fn trim_decimal_fraction(value: &str) -> &str {
    if value.contains('.') {
        value.trim_end_matches('0').trim_end_matches('.')
    } else {
        value
    }
}

fn special_number_string(value: &str) -> Option<&'static str> {
    match value {
        "nan" => Some("NaN"),
        "infinity" => Some("+Inf"),
        "negativeInfinity" => Some("-Inf"),
        _ => None,
    }
}

fn observation_header_for_schema(case_id: &str, schema: ConformanceSchema) -> JsonValue {
    json!({
        "schema_version": schema.version(),
        "case_id": case_id,
        "oracle": {
            "name": "OpenMat",
            "release": env!("CARGO_PKG_VERSION"),
        },
    })
}

fn observation_header(case_id: &str) -> JsonValue {
    observation_header_for_schema(case_id, ConformanceSchema::V1)
}

fn ok_observation(case_id: &str, value: JsonValue) -> JsonValue {
    let mut observation = observation_header(case_id);
    let object = observation
        .as_object_mut()
        .expect("observation header is an object");
    object.insert("outcome".to_owned(), json!("ok"));
    object.insert("value".to_owned(), value);
    observation
}

fn error_observation(case_id: &str, category: &str) -> JsonValue {
    let mut observation = observation_header(case_id);
    let object = observation
        .as_object_mut()
        .expect("observation header is an object");
    object.insert("outcome".to_owned(), json!("error"));
    object.insert("error".to_owned(), json!({ "category": category }));
    observation
}

fn ok_observation_v2(case_id: &str, value: JsonValue) -> JsonValue {
    let mut observation = observation_header_for_schema(case_id, ConformanceSchema::V2);
    let object = observation
        .as_object_mut()
        .expect("observation header is an object");
    object.insert("outcome".to_owned(), json!("ok"));
    object.insert("value".to_owned(), value);
    observation
}

fn bounded_ok_observation_v2(case_id: &str, value: JsonValue) -> JsonValue {
    let observation = ok_observation_v2(case_id, value);
    match serde_json::to_vec_pretty(&observation) {
        Ok(encoded)
            if encoded
                .len()
                .checked_add(1)
                .is_some_and(|length| length <= kernel_v2::MAX_JSON_FRAME_BYTES) =>
        {
            observation
        }
        Ok(_) => error_observation_v2(case_id, "payload-limit"),
        Err(_) => error_observation_v2(case_id, "invalid-observation"),
    }
}

fn error_observation_v2(case_id: &str, category: &str) -> JsonValue {
    let mut observation = observation_header_for_schema(case_id, ConformanceSchema::V2);
    let object = observation
        .as_object_mut()
        .expect("observation header is an object");
    object.insert("outcome".to_owned(), json!("error"));
    object.insert("error".to_owned(), json!({ "category": category }));
    observation
}

fn normalized_error_category(category: &str) -> &'static str {
    match category {
        "runtime.undefinedName" | "runtime.unknownFunctionHandleTarget" | "workspace.notFound" => {
            "undefined-name"
        }
        "runtime.indexOutOfBounds" => "index-out-of-bounds",
        "runtime.dimensionMismatch" => "dimension-mismatch",
        "access-violation" | "runtime.accessViolation" => "access-violation",
        "runtime.inputArity" | "runtime.missingOutputs" => "arity",
        "runtime.notCallable"
        | "runtime.invalidFunction"
        | "runtime.invalidOperands"
        | "runtime.unsupportedArrayOperator"
        | "runtime.invalidCondition" => "type-error",
        value if value.starts_with("compile.") => "syntax-error",
        _ => "other",
    }
}

fn is_internal_category(category: &str) -> bool {
    category.starts_with("protocol.")
        || category.starts_with("kernel.")
        || category == "runtime.initialization"
        || category == "runtime.cancellationBridge"
}

fn write_conformance_observation<W: Write, E: Write>(
    observation: &JsonValue,
    output: &mut W,
    errors: &mut E,
) -> i32 {
    if serde_json::to_writer_pretty(&mut *output, observation).is_err() || writeln!(output).is_err()
    {
        let _ = writeln!(errors, "error[cli.output]: unable to write observation");
        return EXIT_INTERNAL;
    }
    EXIT_SUCCESS
}

fn failure_exit_code(error: &ProtocolError) -> i32 {
    if error.category.starts_with("protocol.") || error.category.starts_with("kernel.") {
        EXIT_INTERNAL
    } else {
        EXIT_DATA_ERROR
    }
}

fn render_event<W: Write, E: Write>(
    event: &Event,
    output: &mut W,
    errors: &mut E,
) -> io::Result<()> {
    match event {
        Event::Stream(stream) => output.write_all(stream.text.as_bytes()),
        Event::Display(display) => {
            if let Some(text) = display.representations.get("text/plain") {
                output.write_all(text.as_bytes())?;
                if !text.ends_with('\n') {
                    writeln!(output)?;
                }
            }
            Ok(())
        }
        Event::Diagnostic(diagnostic) => render_diagnostic(diagnostic, errors),
        Event::Status(_) | Event::WorkspaceDelta(_) | Event::Unknown { .. } => Ok(()),
    }
}

fn render_diagnostic<E: Write>(diagnostic: &Diagnostic, errors: &mut E) -> io::Result<()> {
    let severity = match diagnostic.severity {
        DiagnosticSeverity::Error => "error",
        DiagnosticSeverity::Warning => "warning",
        DiagnosticSeverity::Information => "information",
        DiagnosticSeverity::Hint => "hint",
    };
    let code = diagnostic.code.as_deref().unwrap_or("diagnostic");
    if let Some(range) = &diagnostic.range {
        writeln!(
            errors,
            "{severity}[{code}] source={} bytes={}..{}: {}",
            range.source_name, range.start, range.end, diagnostic.message
        )?;
    } else {
        writeln!(
            errors,
            "{severity}[{code}] source=<unknown> bytes=unknown: {}",
            diagnostic.message
        )?;
    }
    for related in &diagnostic.related {
        writeln!(
            errors,
            "  related source={} bytes={}..{}: {}",
            related.range.source_name, related.range.start, related.range.end, related.message
        )?;
    }
    Ok(())
}

fn render_protocol_error<E: Write>(
    error: &ProtocolError,
    fallback_source: &str,
    errors: &mut E,
) -> io::Result<()> {
    writeln!(
        errors,
        "error[{}] source={fallback_source} bytes=unknown: {}",
        error.category, error.message
    )?;
    for diagnostic in &error.diagnostics {
        render_diagnostic(diagnostic, errors)?;
    }
    Ok(())
}

fn render_internal_error<E: Write>(message: &str, errors: &mut E) {
    let _ = writeln!(
        errors,
        "error[cli.protocol] source=<protocol> bytes=unknown: {message}"
    );
}

fn render_workspace<W: Write>(
    mut variables: Vec<VariableSummary>,
    output: &mut W,
) -> io::Result<()> {
    variables.sort_by(|left, right| left.name.cmp(&right.name));
    for variable in variables {
        let dimensions = variable
            .dimensions
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join("x");
        writeln!(
            output,
            "{}\t{}\t{}",
            variable.name, variable.class, dimensions
        )?;
    }
    Ok(())
}

fn protocol_selftest() -> Result<(), String> {
    protocol_selftest_v0()?;
    protocol_selftest_v1()?;
    protocol_selftest_v0_fallback()
}

fn protocol_selftest_v0() -> Result<(), String> {
    let request = RequestEnvelope::new(
        "cli-selftest",
        "request-1",
        Request::Initialize(InitializeRequest {
            client: ImplementationInfo {
                name: "openmat-cli".to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
            },
            supported_protocols: vec![PROTOCOL_V0.to_owned()],
            capabilities: Capabilities::default(),
        }),
    );
    let request_json = serde_json::to_string(&request).map_err(|error| error.to_string())?;
    let decoded: RequestEnvelope =
        serde_json::from_str(&request_json).map_err(|error| error.to_string())?;
    decoded.validate().map_err(|error| error.to_string())?;

    let response = ResponseEnvelope::success(
        &decoded,
        "response-1",
        ResponseResult::Initialize(openmat_protocol::InitializeResult {
            negotiated_protocol: PROTOCOL_V0.to_owned(),
            implementation: ImplementationInfo {
                name: "selftest-kernel".to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
            },
            capabilities: Capabilities::default(),
        }),
    );
    let response_json = serde_json::to_string(&response).map_err(|error| error.to_string())?;
    let decoded_response: ResponseEnvelope =
        serde_json::from_str(&response_json).map_err(|error| error.to_string())?;
    decoded_response
        .validate()
        .map_err(|error| error.to_string())?;
    if !decoded_response.is_reply_to(&decoded) {
        return Err("response correlation was not preserved".to_owned());
    }
    Ok(())
}

fn protocol_selftest_v1() -> Result<(), String> {
    let v1_capabilities = kernel_v1::Capabilities {
        execution_modes: vec![ExecutionMode::File],
        display_mime_types: vec!["text/plain".to_owned()],
        max_preview_elements: MAX_PREVIEW_ELEMENTS,
        max_string_element_code_units: Some(kernel_v1::MAX_STRING_ELEMENT_CODE_UNITS),
        max_preview_code_units: Some(kernel_v1::MAX_PREVIEW_CODE_UNITS),
        interrupt: false,
        workspace_delta: false,
    };
    let bootstrap = kernel_v1::BootstrapRequestEnvelope::new(
        "cli-selftest-v1",
        "bootstrap-1",
        kernel_v1::InitializeRequest::v1(
            ImplementationInfo {
                name: "openmat-cli".to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
            },
            v1_capabilities.clone(),
        ),
    );
    let bootstrap_json =
        kernel_v1::encode_bootstrap_request(&bootstrap).map_err(|error| error.to_string())?;
    let decoded_bootstrap =
        kernel_v1::decode_bootstrap_request(&bootstrap_json).map_err(|error| error.to_string())?;
    let bootstrap_response = kernel_v1::BootstrapResponseEnvelope::success(
        &decoded_bootstrap,
        "bootstrap-response-1",
        kernel_v1::InitializeResult {
            negotiated_protocol: kernel_v1::PROTOCOL_V1.to_owned(),
            implementation: ImplementationInfo {
                name: "selftest-kernel".to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
            },
            capabilities: v1_capabilities.clone(),
        },
    );
    let bootstrap_response_json = kernel_v1::encode_bootstrap_response(&bootstrap_response)
        .map_err(|error| error.to_string())?;
    let decoded_bootstrap_response = kernel_v1::decode_bootstrap_response(&bootstrap_response_json)
        .map_err(|error| error.to_string())?;
    if kernel_v1::validate_initialize_exchange(&decoded_bootstrap, &decoded_bootstrap_response)
        .map_err(|error| error.to_string())?
        != kernel_v1::ProtocolVersion::V1
    {
        return Err("v1 bootstrap did not preserve negotiation".to_owned());
    }
    let limits = v1_capabilities
        .preview_limits()
        .map_err(|error| error.to_string())?;
    let inspect_request = kernel_v1::RequestEnvelope::new(
        "cli-selftest-v1",
        "request-v1-1",
        kernel_v1::Request::Inspect(kernel_v1::InspectRequest {
            name: "answer".to_owned(),
            range: kernel_v1::MatrixRange {
                start: vec![1, 1],
                size: vec![1, 1],
            },
            max_elements: 1,
        }),
    );
    let inspect_response = kernel_v1::ResponseEnvelope::success(
        &inspect_request,
        "response-v1-1",
        kernel_v1::ResponseResult::Inspect(kernel_v1::MatrixPreview {
            class: "logical".to_owned(),
            dimensions: vec![1, 1],
            complex: false,
            selected_range: kernel_v1::MatrixRange {
                start: vec![1, 1],
                size: vec![1, 1],
            },
            values: vec![kernel_v1::PreviewValue::Logical { value: true }],
            truncation: openmat_protocol::PreviewTruncation {
                truncated: false,
                omitted_elements: 0,
            },
        }),
    );
    let inspect_response_json =
        kernel_v1::encode_response_for_request(&inspect_response, &inspect_request, &limits)
            .map_err(|error| error.to_string())?;
    kernel_v1::decode_response_for_request(&inspect_response_json, &inspect_request, &limits)
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn protocol_selftest_v0_fallback() -> Result<(), String> {
    let fallback = kernel_v1::BootstrapRequestEnvelope::new(
        "cli-selftest-v0",
        "fallback-1",
        kernel_v1::InitializeRequest {
            client: ImplementationInfo {
                name: "openmat-cli".to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
            },
            supported_protocols: vec![PROTOCOL_V0.to_owned()],
            capabilities: kernel_v1::Capabilities::default(),
        },
    );
    let fallback_response = kernel_v1::BootstrapResponseEnvelope::success(
        &fallback,
        "fallback-response-1",
        kernel_v1::InitializeResult {
            negotiated_protocol: PROTOCOL_V0.to_owned(),
            implementation: ImplementationInfo {
                name: "selftest-kernel".to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
            },
            capabilities: kernel_v1::Capabilities::default(),
        },
    );
    if kernel_v1::validate_initialize_exchange(&fallback, &fallback_response)
        .map_err(|error| error.to_string())?
        != kernel_v1::ProtocolVersion::V0
    {
        return Err("v0 fallback negotiation was not preserved".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    use openmat_kernel::{CancellationToken, ExecutionOutput};
    use openmat_protocol::{
        DisplayEvent, ExecuteResult, InitializeResult, PreviewTruncation, RelatedDiagnostic,
        SourceRange, StreamEvent, StreamKind, WorkspaceDeltaEvent, WorkspaceSummary,
    };

    use super::*;

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum EngineCall {
        Initialize,
        Execute(ExecuteRequest),
        InspectV1(kernel_v1::InspectRequest),
        ListWorkspace,
        Shutdown,
    }

    #[derive(Clone)]
    struct FakeEngine {
        calls: Arc<Mutex<Vec<EngineCall>>>,
        execution: Result<ExecutionOutput, EngineError>,
        preview_v1: Result<kernel_v1::MatrixPreview, EngineError>,
        workspace: Result<Vec<VariableSummary>, EngineError>,
        shutdown: Result<(), EngineError>,
    }

    impl FakeEngine {
        fn successful(calls: Arc<Mutex<Vec<EngineCall>>>) -> Self {
            Self {
                calls,
                execution: Ok(ExecutionOutput::default()),
                preview_v1: Err(EngineError::new(
                    "workspace.unsupportedValue",
                    "test v1 preview is not configured",
                )),
                workspace: Ok(Vec::new()),
                shutdown: Ok(()),
            }
        }
    }

    impl ExecutionEngine for FakeEngine {
        fn implementation(&self) -> ImplementationInfo {
            ImplementationInfo {
                name: "fake-engine".to_owned(),
                version: "test".to_owned(),
            }
        }

        fn capabilities(&self) -> Capabilities {
            self.calls
                .lock()
                .expect("call log lock")
                .push(EngineCall::Initialize);
            Capabilities {
                execution_modes: vec![ExecutionMode::File],
                display_mime_types: vec!["text/plain".to_owned()],
                max_preview_elements: MAX_PREVIEW_ELEMENTS,
                interrupt: true,
                workspace_delta: true,
            }
        }

        fn execute(
            &mut self,
            request: &ExecuteRequest,
            _cancellation: &CancellationToken,
        ) -> Result<ExecutionOutput, EngineError> {
            self.calls
                .lock()
                .expect("call log lock")
                .push(EngineCall::Execute(request.clone()));
            self.execution.clone()
        }

        fn inspect(
            &mut self,
            _request: &openmat_protocol::InspectRequest,
        ) -> Result<MatrixPreview, EngineError> {
            Err(EngineError::new("test.unused", "inspect is not used"))
        }

        fn inspect_v1(
            &mut self,
            request: &kernel_v1::InspectRequest,
            _limits: &kernel_v1::PreviewLimits,
        ) -> Result<kernel_v1::MatrixPreview, EngineError> {
            self.calls
                .lock()
                .expect("call log lock")
                .push(EngineCall::InspectV1(request.clone()));
            self.preview_v1.clone()
        }

        fn list_workspace(&mut self) -> Result<Vec<VariableSummary>, EngineError> {
            self.calls
                .lock()
                .expect("call log lock")
                .push(EngineCall::ListWorkspace);
            self.workspace.clone()
        }

        fn shutdown(&mut self) -> Result<(), EngineError> {
            self.calls
                .lock()
                .expect("call log lock")
                .push(EngineCall::Shutdown);
            self.shutdown.clone()
        }
    }

    fn run_fake(
        arguments: &[&str],
        bytes: Result<Vec<u8>, io::Error>,
        engine: Result<FakeEngine, EngineError>,
    ) -> (i32, String, String) {
        let mut output = Vec::new();
        let mut errors = Vec::new();
        let mut bytes = Some(bytes);
        let exit = run_with(
            arguments.iter().copied(),
            &mut output,
            &mut errors,
            |_| {
                bytes
                    .take()
                    .unwrap_or_else(|| Err(io::Error::other("test reader called more than once")))
            },
            |path| Ok(path.to_path_buf()),
            |_| engine,
        );
        (
            exit,
            String::from_utf8(output).expect("UTF-8 output"),
            String::from_utf8(errors).expect("UTF-8 errors"),
        )
    }

    #[test]
    fn parses_explicit_run_and_file_shortcut_equivalently() {
        let explicit = parse_command(&[
            "run".to_owned(),
            r"folder\program.m".to_owned(),
            "--oex-plugin".to_owned(),
            r"plugins\first.oex.dll".to_owned(),
            "--workspace".to_owned(),
            "--oex-plugin".to_owned(),
            r"plugins\second.oex.dll".to_owned(),
        ])
        .expect("explicit run command");
        let shortcut = parse_command(&[
            r"folder\program.m".to_owned(),
            "--oex-plugin".to_owned(),
            r"plugins\first.oex.dll".to_owned(),
            "--workspace".to_owned(),
            "--oex-plugin".to_owned(),
            r"plugins\second.oex.dll".to_owned(),
        ])
        .expect("file shortcut");
        assert_eq!(explicit, shortcut);
        assert_eq!(
            explicit,
            Command::Run {
                path: PathBuf::from(r"folder\program.m"),
                workspace: true,
                oex_plugins: vec![
                    PathBuf::from(r"plugins\first.oex.dll"),
                    PathBuf::from(r"plugins\second.oex.dll"),
                ],
            }
        );
    }

    #[test]
    fn rejects_missing_files_unknown_options_and_non_m_sources() {
        for arguments in [
            vec!["run".to_owned()],
            vec!["run".to_owned(), "file.txt".to_owned()],
            vec!["run".to_owned(), "file.m".to_owned(), "--bad".to_owned()],
            vec![
                "run".to_owned(),
                "file.m".to_owned(),
                "--oex-plugin".to_owned(),
            ],
            vec![
                "run".to_owned(),
                "file.m".to_owned(),
                "--oex-plugin".to_owned(),
                "--workspace".to_owned(),
            ],
            vec!["file.m".to_owned(), "extra.m".to_owned()],
        ] {
            assert!(parse_command(&arguments).is_err(), "{arguments:?}");
        }
    }

    #[test]
    fn help_version_and_protocol_selftest_remain_successful() {
        for arguments in [vec!["openmat-cli"], vec!["openmat-cli", "--help"]] {
            let mut output = Vec::new();
            let mut errors = Vec::new();
            assert_eq!(run(arguments, &mut output, &mut errors), EXIT_SUCCESS);
            let help = String::from_utf8(output).expect("UTF-8 help");
            assert!(help.contains("run <file.m> [--workspace]"));
            assert!(help.contains("130 interrupted"));
            assert!(errors.is_empty());
        }

        let mut output = Vec::new();
        let mut errors = Vec::new();
        assert_eq!(
            run(["openmat-cli", "version"], &mut output, &mut errors),
            EXIT_SUCCESS
        );
        assert!(
            String::from_utf8(output)
                .expect("UTF-8 version")
                .starts_with("openmat-cli ")
        );
        assert!(errors.is_empty());

        let mut output = Vec::new();
        let mut errors = Vec::new();
        assert_eq!(
            run(
                ["openmat-cli", "protocol", "selftest"],
                &mut output,
                &mut errors,
            ),
            EXIT_SUCCESS
        );
        assert_eq!(
            String::from_utf8(output).expect("UTF-8 selftest"),
            "openmat-kernel-v0: selftest passed\n"
        );
        assert!(errors.is_empty());
    }

    #[test]
    fn usage_errors_return_64_without_initializing_an_engine() {
        let (exit, output, errors) = run_fake(
            &["openmat-cli", "unknown"],
            Ok(Vec::new()),
            Err(EngineError::new("must.not.run", "factory was called")),
        );
        assert_eq!(exit, EXIT_USAGE);
        assert!(output.is_empty());
        assert!(errors.contains("unsupported command"));
        assert!(!errors.contains("factory was called"));
    }

    #[test]
    fn file_and_utf8_failures_return_66_before_engine_initialization() {
        let (exit, output, errors) = run_fake(
            &["openmat-cli", "missing.m"],
            Err(io::Error::new(io::ErrorKind::NotFound, "not found")),
            Err(EngineError::new("must.not.run", "factory was called")),
        );
        assert_eq!(exit, EXIT_INPUT);
        assert!(output.is_empty());
        assert!(errors.contains("error[cli.input] source=missing.m bytes=unknown"));
        assert!(!errors.contains("factory was called"));

        let (exit, output, errors) = run_fake(
            &["openmat-cli", "invalid.m"],
            Ok(vec![b'a', 0xff, b'b']),
            Err(EngineError::new("must.not.run", "factory was called")),
        );
        assert_eq!(exit, EXIT_INPUT);
        assert!(output.is_empty());
        assert!(errors.contains("error[cli.utf8] source=invalid.m bytes=1..2"));
        assert!(!errors.contains("factory was called"));
    }

    #[test]
    fn engine_initialization_failure_returns_70() {
        let (exit, output, errors) = run_fake(
            &["openmat-cli", "program.m"],
            Ok(b"answer = 42;\r\n".to_vec()),
            Err(EngineError::new("engine.start", "registration failed")),
        );
        assert_eq!(exit, EXIT_INTERNAL);
        assert!(output.is_empty());
        assert!(errors.contains("error[cli.engineInit]"));
        assert!(errors.contains("engine.start: registration failed"));
    }

    #[test]
    fn embedded_kernel_receives_file_mode_crlf_and_ordered_lifecycle() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut engine = FakeEngine::successful(Arc::clone(&calls));
        engine.workspace = Ok(vec![
            VariableSummary {
                name: "zeta".to_owned(),
                class: "logical".to_owned(),
                dimensions: vec![2, 1],
                complex: false,
                bytes: Some(2),
            },
            VariableSummary {
                name: "alpha".to_owned(),
                class: "double".to_owned(),
                dimensions: vec![1, 3],
                complex: false,
                bytes: Some(24),
            },
        ]);
        let (exit, output, errors) = run_fake(
            &["openmat-cli", "run", r"folder\program.m", "--workspace"],
            Ok(b"answer = 42;\r\n".to_vec()),
            Ok(engine),
        );
        assert_eq!(exit, EXIT_SUCCESS);
        assert_eq!(output, "alpha\tdouble\t1x3\nzeta\tlogical\t2x1\n");
        assert!(errors.is_empty());
        assert_eq!(
            *calls.lock().expect("call log lock"),
            vec![
                EngineCall::Initialize,
                EngineCall::Execute(ExecuteRequest {
                    code: "answer = 42;\r\n".to_owned(),
                    source_name: r"folder\program.m".to_owned(),
                    mode: ExecutionMode::File,
                }),
                EngineCall::ListWorkspace,
                EngineCall::Shutdown,
            ]
        );
    }

    #[test]
    fn renders_stream_plain_text_and_diagnostics_without_json_leaks() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut engine = FakeEngine::successful(calls);
        let mut representations = BTreeMap::new();
        representations.insert("text/plain".to_owned(), "display\n".to_owned());
        representations.insert(
            "application/json".to_owned(),
            "{\"hidden\":true}".to_owned(),
        );
        engine.execution = Ok(ExecutionOutput {
            result: ExecuteResult { interrupted: false },
            events: vec![
                Event::Status(openmat_protocol::StatusEvent {
                    status: openmat_protocol::KernelStatus::Busy,
                }),
                Event::Stream(StreamEvent {
                    stream: StreamKind::Stderr,
                    text: "stream\n".to_owned(),
                }),
                Event::Display(DisplayEvent { representations }),
                Event::Diagnostic(Diagnostic {
                    code: Some("OMC0042".to_owned()),
                    severity: DiagnosticSeverity::Warning,
                    message: "a warning".to_owned(),
                    range: Some(SourceRange {
                        source_name: "program.m".to_owned(),
                        start: 3,
                        end: 8,
                    }),
                    related: vec![RelatedDiagnostic {
                        message: "defined here".to_owned(),
                        range: SourceRange {
                            source_name: "program.m".to_owned(),
                            start: 0,
                            end: 1,
                        },
                    }],
                }),
                Event::WorkspaceDelta(WorkspaceDeltaEvent::default()),
                Event::Unknown {
                    event_type: "futureEvent".to_owned(),
                    data: serde_json::json!({"secret": true}),
                },
            ],
        });

        let (exit, output, errors) = run_fake(
            &["openmat-cli", "program.m"],
            Ok(b"x = 1;".to_vec()),
            Ok(engine),
        );
        assert_eq!(exit, EXIT_SUCCESS);
        assert_eq!(output, "stream\ndisplay\n");
        assert!(!output.contains("hidden"));
        assert!(!output.contains("futureEvent"));
        assert!(errors.contains("warning[OMC0042] source=program.m bytes=3..8: a warning"));
        assert!(errors.contains("related source=program.m bytes=0..1: defined here"));
    }

    #[test]
    fn compile_or_runtime_failure_returns_65_and_still_shuts_down() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut engine = FakeEngine::successful(Arc::clone(&calls));
        engine.execution = Err(EngineError::new("compile.failed", "source did not compile")
            .with_diagnostics(vec![Diagnostic {
                code: Some("OMC0007".to_owned()),
                severity: DiagnosticSeverity::Error,
                message: "unsupported syntax".to_owned(),
                range: Some(SourceRange {
                    source_name: "bad.m".to_owned(),
                    start: 2,
                    end: 5,
                }),
                related: Vec::new(),
            }]));
        let (exit, output, errors) = run_fake(
            &["openmat-cli", "bad.m"],
            Ok(b"bad syntax".to_vec()),
            Ok(engine),
        );
        assert_eq!(exit, EXIT_DATA_ERROR);
        assert!(output.is_empty());
        assert!(
            errors.contains(
                "error[compile.failed] source=bad.m bytes=unknown: source did not compile"
            )
        );
        assert!(errors.contains("error[OMC0007] source=bad.m bytes=2..5"));
        assert_eq!(
            *calls.lock().expect("call log lock"),
            vec![
                EngineCall::Initialize,
                EngineCall::Execute(ExecuteRequest {
                    code: "bad syntax".to_owned(),
                    source_name: "bad.m".to_owned(),
                    mode: ExecutionMode::File,
                }),
                EngineCall::Shutdown,
            ]
        );
    }

    #[test]
    fn interrupted_execution_returns_130_and_still_shuts_down() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut engine = FakeEngine::successful(Arc::clone(&calls));
        engine.execution = Ok(ExecutionOutput {
            result: ExecuteResult { interrupted: true },
            events: Vec::new(),
        });
        let (exit, output, errors) =
            run_fake(&["openmat-cli", "program.m"], Ok(Vec::new()), Ok(engine));
        assert_eq!(exit, EXIT_INTERRUPTED);
        assert!(output.is_empty());
        assert!(errors.is_empty());
        assert_eq!(
            calls.lock().expect("call log lock").last(),
            Some(&EngineCall::Shutdown)
        );
    }

    struct ScriptedHandler {
        messages: Vec<Vec<ServerMessage>>,
        requests: Vec<RequestEnvelope>,
    }

    impl RequestHandler for ScriptedHandler {
        fn handle_request(&mut self, request: &RequestEnvelope) -> Vec<ServerMessage> {
            self.requests.push(request.clone());
            self.messages.remove(0)
        }
    }

    #[derive(Clone, Copy)]
    enum SchemaV2SessionFault {
        Correlation,
        ResponseType,
        SelectedRange,
        RequestLimit,
    }

    struct ScriptedSchemaV2SessionHandler {
        selected: kernel_v2::ProtocolVersion,
        fault: Option<SchemaV2SessionFault>,
    }

    impl KernelSessionHandler for ScriptedSchemaV2SessionHandler {
        fn handle_session(
            &mut self,
            request: KernelSessionRequest<'_>,
        ) -> Result<Vec<KernelSessionMessage>, KernelSessionError> {
            match request {
                KernelSessionRequest::Bootstrap(request) => {
                    Ok(scripted_schema_v2_bootstrap(request, self.selected))
                }
                KernelSessionRequest::V0(request) => Ok(scripted_v0_shutdown(request)),
                KernelSessionRequest::V1(request) => Ok(self.fault.map_or_else(
                    || scripted_v1_shutdown(request),
                    |fault| scripted_v1_fault_response(request, fault),
                )),
                KernelSessionRequest::V2(request) => Ok(self.fault.map_or_else(
                    || scripted_v2_shutdown(request),
                    |fault| scripted_v2_fault_response(request, fault),
                )),
                KernelSessionRequest::V3(_) => panic!("v2 driver sent a v3 request"),
            }
        }
    }

    fn scripted_schema_v2_bootstrap(
        request: &kernel_v2::BootstrapRequestEnvelope,
        selected: kernel_v2::ProtocolVersion,
    ) -> Vec<KernelSessionMessage> {
        let kernel_v2::BootstrapRequest::Initialize(initialize) = &request.request;
        assert_eq!(
            initialize.supported_protocols,
            kernel_v2::production_protocol_offer()
        );
        let mut capabilities = kernel_v2::Capabilities {
            execution_modes: vec![ExecutionMode::File],
            display_mime_types: vec!["text/plain".to_owned()],
            max_preview_elements: MAX_PREVIEW_ELEMENTS,
            max_string_element_code_units: None,
            max_preview_code_units: None,
            max_aggregate_nodes: None,
            max_aggregate_elements: None,
            max_aggregate_depth: None,
            interrupt: false,
            workspace_delta: false,
        };
        if selected != kernel_v2::ProtocolVersion::V0 {
            capabilities.max_string_element_code_units =
                Some(kernel_v1::MAX_STRING_ELEMENT_CODE_UNITS);
            capabilities.max_preview_code_units = Some(kernel_v1::MAX_PREVIEW_CODE_UNITS);
        }
        if matches!(
            selected,
            kernel_v2::ProtocolVersion::V2 | kernel_v2::ProtocolVersion::V3
        ) {
            capabilities.max_aggregate_nodes = Some(kernel_v2::MAX_AGGREGATE_NODES);
            capabilities.max_aggregate_elements = Some(kernel_v2::MAX_AGGREGATE_ELEMENTS);
            capabilities.max_aggregate_depth = Some(kernel_v2::MAX_AGGREGATE_DEPTH);
        }
        let response = kernel_v2::BootstrapResponseEnvelope::success(
            request,
            "scripted-bootstrap-response",
            kernel_v2::InitializeResult {
                negotiated_protocol: selected.as_str().to_owned(),
                implementation: ImplementationInfo {
                    name: "scripted-session".to_owned(),
                    version: "test".to_owned(),
                },
                capabilities,
            },
        );
        let status = Event::Status(openmat_protocol::StatusEvent {
            status: openmat_protocol::KernelStatus::Idle,
        });
        let idle = match selected {
            kernel_v2::ProtocolVersion::V0 => KernelSessionMessage::V0(ServerMessage::Event(
                openmat_protocol::EventEnvelope::new(SESSION_ID, "scripted-idle", status),
            )),
            kernel_v2::ProtocolVersion::V1 => {
                KernelSessionMessage::V1(kernel_v1::ServerMessage::Event(
                    kernel_v1::EventEnvelope::new(SESSION_ID, "scripted-idle", status),
                ))
            }
            kernel_v2::ProtocolVersion::V2 => {
                KernelSessionMessage::V2(kernel_v2::ServerMessage::Event(
                    kernel_v2::EventEnvelope::new(SESSION_ID, "scripted-idle", status),
                ))
            }
            kernel_v2::ProtocolVersion::V3 => {
                KernelSessionMessage::V3(openmat_protocol::kernel_v3::ServerMessage::Event(
                    openmat_protocol::kernel_v3::EventEnvelope::new(
                        SESSION_ID,
                        "scripted-idle",
                        status,
                    ),
                ))
            }
        };
        vec![KernelSessionMessage::Bootstrap(response), idle]
    }

    fn scripted_v0_shutdown(request: &RequestEnvelope) -> Vec<KernelSessionMessage> {
        assert!(matches!(request.request, Request::Shutdown(_)));
        vec![KernelSessionMessage::V0(ServerMessage::Response(
            ResponseEnvelope::success(
                request,
                "scripted-response",
                ResponseResult::Shutdown(openmat_protocol::ShutdownResult {}),
            ),
        ))]
    }

    fn scripted_v1_shutdown(request: &kernel_v1::RequestEnvelope) -> Vec<KernelSessionMessage> {
        assert!(matches!(request.request, kernel_v1::Request::Shutdown(_)));
        vec![KernelSessionMessage::V1(
            kernel_v1::ServerMessage::Response(kernel_v1::ResponseEnvelope::success(
                request,
                "scripted-response",
                kernel_v1::ResponseResult::Shutdown(openmat_protocol::ShutdownResult {}),
            )),
        )]
    }

    fn scripted_v2_shutdown(request: &kernel_v2::RequestEnvelope) -> Vec<KernelSessionMessage> {
        assert!(matches!(request.request, kernel_v2::Request::Shutdown(_)));
        vec![KernelSessionMessage::V2(
            kernel_v2::ServerMessage::Response(kernel_v2::ResponseEnvelope::success(
                request,
                "scripted-response",
                kernel_v2::ResponseResult::Shutdown(openmat_protocol::ShutdownResult {}),
            )),
        )]
    }

    fn scripted_v1_fault_response(
        request: &kernel_v1::RequestEnvelope,
        fault: SchemaV2SessionFault,
    ) -> Vec<KernelSessionMessage> {
        let result = match fault {
            SchemaV2SessionFault::Correlation => {
                kernel_v1::ResponseResult::Execute(ExecuteResult { interrupted: false })
            }
            SchemaV2SessionFault::ResponseType => {
                kernel_v1::ResponseResult::ListWorkspace(WorkspaceSummary::default())
            }
            SchemaV2SessionFault::SelectedRange | SchemaV2SessionFault::RequestLimit => {
                kernel_v1::ResponseResult::Inspect(scripted_matrix_preview(fault))
            }
        };
        let mut response =
            kernel_v1::ResponseEnvelope::success(request, "scripted-response", result);
        if matches!(fault, SchemaV2SessionFault::Correlation) {
            response.reply_to = "wrong-request".to_owned();
        }
        vec![KernelSessionMessage::V1(
            kernel_v1::ServerMessage::Response(response),
        )]
    }

    fn scripted_v2_fault_response(
        request: &kernel_v2::RequestEnvelope,
        fault: SchemaV2SessionFault,
    ) -> Vec<KernelSessionMessage> {
        let result = match fault {
            SchemaV2SessionFault::Correlation => {
                kernel_v2::ResponseResult::Execute(ExecuteResult { interrupted: false })
            }
            SchemaV2SessionFault::ResponseType => {
                kernel_v2::ResponseResult::ListWorkspace(WorkspaceSummary::default())
            }
            SchemaV2SessionFault::SelectedRange | SchemaV2SessionFault::RequestLimit => {
                kernel_v2::ResponseResult::Inspect(kernel_v2::InspectPreview::Matrix(
                    scripted_matrix_preview(fault),
                ))
            }
        };
        let mut response =
            kernel_v2::ResponseEnvelope::success(request, "scripted-response", result);
        if matches!(fault, SchemaV2SessionFault::Correlation) {
            response.reply_to = "wrong-request".to_owned();
        }
        vec![KernelSessionMessage::V2(
            kernel_v2::ServerMessage::Response(response),
        )]
    }

    fn scripted_matrix_preview(fault: SchemaV2SessionFault) -> kernel_v1::MatrixPreview {
        let request_limit = matches!(fault, SchemaV2SessionFault::RequestLimit);
        let size = if request_limit {
            vec![1, 2]
        } else {
            vec![1, 1]
        };
        let selected_range = kernel_v1::MatrixRange {
            start: if matches!(fault, SchemaV2SessionFault::SelectedRange) {
                vec![1, 2]
            } else {
                vec![1, 1]
            },
            size,
        };
        let value_count = if request_limit { 2 } else { 1 };
        kernel_v1::MatrixPreview {
            class: "logical".to_owned(),
            dimensions: vec![1, 2],
            complex: false,
            selected_range,
            values: vec![kernel_v1::PreviewValue::Logical { value: true }; value_count],
            truncation: PreviewTruncation {
                truncated: false,
                omitted_elements: 0,
            },
        }
    }

    fn initialize_success(request: &RequestEnvelope, reply_to: &str) -> ServerMessage {
        let mut response = ResponseEnvelope::success(
            request,
            "server-initialize",
            ResponseResult::Initialize(InitializeResult {
                negotiated_protocol: PROTOCOL_V0.to_owned(),
                implementation: ImplementationInfo {
                    name: "scripted".to_owned(),
                    version: "test".to_owned(),
                },
                capabilities: Capabilities::default(),
            }),
        );
        response.reply_to = reply_to.to_owned();
        ServerMessage::Response(response)
    }

    #[test]
    fn rejects_uncorrelated_responses_with_exit_70_and_attempts_shutdown() {
        let template = RequestEnvelope::new(
            SESSION_ID,
            "template",
            Request::Shutdown(ShutdownRequest {}),
        );
        let bad_initialize = initialize_success(&template, "not-cli-1");
        let shutdown = ServerMessage::Response(ResponseEnvelope::success(
            &RequestEnvelope::new(SESSION_ID, "cli-2", Request::Shutdown(ShutdownRequest {})),
            "server-shutdown",
            ResponseResult::Shutdown(openmat_protocol::ShutdownResult {}),
        ));
        let mut handler = ScriptedHandler {
            messages: vec![vec![bad_initialize], vec![shutdown]],
            requests: Vec::new(),
        };
        let mut output = Vec::new();
        let mut errors = Vec::new();
        let exit = drive_session(
            &mut handler,
            ExecuteRequest {
                code: String::new(),
                source_name: "program.m".to_owned(),
                mode: ExecutionMode::File,
            },
            false,
            &mut output,
            &mut errors,
        );
        assert_eq!(exit, EXIT_INTERNAL);
        assert_eq!(handler.requests.len(), 2);
        assert!(matches!(
            handler.requests[0].request,
            Request::Initialize(_)
        ));
        assert!(matches!(handler.requests[1].request, Request::Shutdown(_)));
        assert!(
            String::from_utf8(errors)
                .expect("UTF-8 errors")
                .contains("not correlated")
        );
    }

    #[test]
    fn rejects_response_type_mismatches_with_exit_70() {
        let initialize_request = RequestEnvelope::new(
            SESSION_ID,
            "cli-1",
            Request::Initialize(InitializeRequest {
                client: ImplementationInfo {
                    name: "test".to_owned(),
                    version: "test".to_owned(),
                },
                supported_protocols: vec![PROTOCOL_V0.to_owned()],
                capabilities: Capabilities::default(),
            }),
        );
        let wrong_response = ServerMessage::Response(ResponseEnvelope::success(
            &initialize_request,
            "server-wrong-type",
            ResponseResult::ListWorkspace(WorkspaceSummary::default()),
        ));
        let shutdown_request =
            RequestEnvelope::new(SESSION_ID, "cli-2", Request::Shutdown(ShutdownRequest {}));
        let shutdown_response = ServerMessage::Response(ResponseEnvelope::success(
            &shutdown_request,
            "server-shutdown",
            ResponseResult::Shutdown(openmat_protocol::ShutdownResult {}),
        ));
        let mut handler = ScriptedHandler {
            messages: vec![vec![wrong_response], vec![shutdown_response]],
            requests: Vec::new(),
        };
        let mut output = Vec::new();
        let mut errors = Vec::new();
        let exit = drive_session(
            &mut handler,
            ExecuteRequest {
                code: String::new(),
                source_name: "program.m".to_owned(),
                mode: ExecutionMode::File,
            },
            false,
            &mut output,
            &mut errors,
        );
        assert_eq!(exit, EXIT_INTERNAL);
        assert!(
            String::from_utf8(errors)
                .expect("UTF-8 errors")
                .contains("response type does not match")
        );
    }

    fn temporary_case_directory(label: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "openmat-cli-{label}-{}-{unique}",
            std::process::id()
        ))
    }

    fn temporary_relative_directory(label: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        let path = PathBuf::from("target").join(format!(
            "openmat-cli-{label}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("relative temporary directory");
        path
    }

    fn write_case(root: &Path, id: &str, source: &str) -> PathBuf {
        let manifests = root.join("manifests");
        let programs = root.join("programs");
        let support = root.join("support");
        fs::create_dir_all(&manifests).expect("manifest directory");
        fs::create_dir_all(&programs).expect("program directory");
        fs::create_dir_all(&support).expect("support directory");
        fs::write(programs.join(format!("{id}.m")), source).expect("source file");
        let manifest = manifests.join(format!("{id}.json"));
        fs::write(
            &manifest,
            serde_json::to_vec(&json!({
                "schema_version": 1,
                "id": id,
                "description": "CLI conformance unit test",
                "source": format!("programs/{id}.m"),
                "tags": ["test"],
                "expected": {
                    "outcome": "ok",
                    "value": {
                        "class": "double",
                        "size": [1, 1],
                        "ndims": 2,
                        "numel": 1,
                        "kind": "numeric",
                        "real": ["42"],
                        "imag": ["0"]
                    }
                }
            }))
            .expect("manifest JSON"),
        )
        .expect("manifest file");
        manifest
    }

    fn write_case_v2(root: &Path, id: &str, source: &str, value: &JsonValue) -> PathBuf {
        let manifests = root.join("manifests");
        let programs = root.join("programs");
        let support = root.join("support");
        fs::create_dir_all(&manifests).expect("manifest directory");
        fs::create_dir_all(&programs).expect("program directory");
        fs::create_dir_all(&support).expect("support directory");
        fs::write(programs.join(format!("{id}.m")), source).expect("source file");
        let manifest = manifests.join(format!("{id}.json"));
        fs::write(
            &manifest,
            serde_json::to_vec(&json!({
                "schema_version": 2,
                "id": id,
                "description": "CLI schema-v2 conformance unit test",
                "source": format!("programs/{id}.m"),
                "tags": ["test", "wire-v2"],
                "expected": { "outcome": "ok", "value": value }
            }))
            .expect("manifest JSON"),
        )
        .expect("manifest file");
        manifest
    }

    fn v1_summary(class: &str, dimensions: Vec<u64>, complex: bool) -> VariableSummary {
        VariableSummary {
            name: CONFORMANCE_RESULT.to_owned(),
            class: class.to_owned(),
            dimensions,
            complex,
            bytes: None,
        }
    }

    fn complete_v1_preview(
        summary: &VariableSummary,
        values: Vec<kernel_v1::PreviewValue>,
    ) -> kernel_v1::MatrixPreview {
        kernel_v1::MatrixPreview {
            class: summary.class.clone(),
            dimensions: summary.dimensions.clone(),
            complex: summary.complex,
            selected_range: kernel_v1::MatrixRange {
                start: vec![1; summary.dimensions.len()],
                size: summary.dimensions.clone(),
            },
            values,
            truncation: PreviewTruncation {
                truncated: false,
                omitted_elements: 0,
            },
        }
    }

    fn exact_double_column(values: &[&str]) -> kernel_v2::ExactValue {
        let rows = u64::try_from(values.len()).expect("test column length");
        kernel_v2::ExactValue {
            class: "double".to_owned(),
            size: vec![rows, 1],
            ndims: 2,
            numel: rows,
            complex: false,
            payload: kernel_v2::ExactPayload::Numeric {
                real: values.iter().map(|value| (*value).to_owned()).collect(),
                imag: vec!["0".to_owned(); values.len()],
            },
        }
    }

    fn exact_logical_column(values: Vec<bool>) -> kernel_v2::ExactValue {
        let rows = u64::try_from(values.len()).expect("test column length");
        kernel_v2::ExactValue {
            class: "logical".to_owned(),
            size: vec![rows, 1],
            ndims: 2,
            numel: rows,
            complex: false,
            payload: kernel_v2::ExactPayload::Logical { logical: values },
        }
    }

    fn complete_table_preview() -> kernel_v2::AggregatePreview {
        kernel_v2::AggregatePreview::Table {
            dimensions: vec![2, 2],
            selected_range: kernel_v2::MatrixRange {
                start: vec![1, 1],
                size: vec![2, 2],
            },
            variable_names: vec!["A".to_owned(), "B".to_owned()],
            variables: vec![
                exact_double_column(&["1", "2"]),
                exact_logical_column(vec![true, false]),
            ],
            truncation: PreviewTruncation {
                truncated: false,
                omitted_elements: 0,
            },
            usage: kernel_v2::PreviewUsage {
                nodes: 3,
                elements: 6,
                code_units: 2,
                depth: 1,
            },
        }
    }

    #[test]
    fn conformance_manifest_routes_v1_and_v2_and_rejects_non_contract_fields() {
        let manifest_path = Path::new("cases/manifests/schema_case.json");
        let base = json!({
            "schema_version": 1,
            "id": "schema_case",
            "description": "Strict shared manifest fields.",
            "source": "programs/schema_case.m",
            "tags": ["r2022b", "schema"],
            "expected": {
                "outcome": "error",
                "error": { "category": "other" }
            }
        });
        let parsed = parse_conformance_manifest(
            manifest_path,
            &serde_json::to_vec(&base).expect("v1 manifest JSON"),
        )
        .expect("schema-v1 manifest");
        assert_eq!(parsed.schema, ConformanceSchema::V1);

        let mut v2 = base.clone();
        v2["schema_version"] = json!(2);
        let parsed = parse_conformance_manifest(
            manifest_path,
            &serde_json::to_vec(&v2).expect("v2 manifest JSON"),
        )
        .expect("schema-v2 manifest");
        assert_eq!(parsed.schema, ConformanceSchema::V2);

        for invalid in [
            {
                let mut value = base.clone();
                value["extra"] = json!(true);
                value
            },
            {
                let mut value = base.clone();
                value["description"] = json!("");
                value
            },
            {
                let mut value = base.clone();
                value["tags"] = json!(["schema", "schema"]);
                value
            },
            {
                let mut value = base.clone();
                value["schema_version"] = json!(3);
                value
            },
            {
                let mut value = base.clone();
                value["expected"]["extra"] = json!(true);
                value
            },
        ] {
            assert!(
                parse_conformance_manifest(
                    manifest_path,
                    &serde_json::to_vec(&invalid).expect("invalid manifest JSON")
                )
                .is_err(),
                "{invalid}"
            );
        }
    }

    #[test]
    fn schema_v2_normalizer_preserves_held_char_and_string_payloads() {
        let char_summary = v1_summary("char", vec![1, 3], false);
        let char_value = observation_value_v2(
            &char_summary,
            complete_v1_preview(
                &char_summary,
                vec![
                    kernel_v1::PreviewValue::CharCodeUnit { value: 65 },
                    kernel_v1::PreviewValue::CharCodeUnit { value: 55_357 },
                    kernel_v1::PreviewValue::CharCodeUnit { value: 66 },
                ],
            ),
            3,
        )
        .expect("isolated surrogate char payload");
        assert_eq!(char_value["code_units"], json!([65, 55_357, 66]));
        assert_eq!(char_value["complex"], false);

        let string_summary = v1_summary("string", vec![1, 3], false);
        let string_value = observation_value_v2(
            &string_summary,
            complete_v1_preview(
                &string_summary,
                vec![
                    kernel_v1::PreviewValue::String {
                        code_units: vec![55_357],
                        missing: false,
                    },
                    kernel_v1::PreviewValue::String {
                        code_units: Vec::new(),
                        missing: false,
                    },
                    kernel_v1::PreviewValue::String {
                        code_units: Vec::new(),
                        missing: true,
                    },
                ],
            ),
            3,
        )
        .expect("surrogate, empty, and missing string payload");
        assert_eq!(string_value["string_code_units"], json!([[55_357], [], []]));
        assert_eq!(string_value["missing"], json!([false, false, true]));
    }

    #[test]
    fn schema_v2_normalizer_preserves_held_integer_payloads() {
        let signed_summary = v1_summary("int64", vec![1, 2], false);
        let signed_value = observation_value_v2(
            &signed_summary,
            complete_v1_preview(
                &signed_summary,
                vec![
                    kernel_v1::PreviewValue::Integer {
                        real: "-9223372036854775808".to_owned(),
                        imaginary: "0".to_owned(),
                    },
                    kernel_v1::PreviewValue::Integer {
                        real: "9223372036854775807".to_owned(),
                        imaginary: "0".to_owned(),
                    },
                ],
            ),
            2,
        )
        .expect("int64 endpoint payload");
        assert_eq!(
            signed_value["integer"],
            json!([
                {"real": "-9223372036854775808", "imaginary": "0"},
                {"real": "9223372036854775807", "imaginary": "0"}
            ])
        );

        let unsigned_summary = v1_summary("uint64", vec![1, 2], false);
        let unsigned_value = observation_value_v2(
            &unsigned_summary,
            complete_v1_preview(
                &unsigned_summary,
                vec![
                    kernel_v1::PreviewValue::Integer {
                        real: "0".to_owned(),
                        imaginary: "0".to_owned(),
                    },
                    kernel_v1::PreviewValue::Integer {
                        real: "18446744073709551615".to_owned(),
                        imaginary: "0".to_owned(),
                    },
                ],
            ),
            2,
        )
        .expect("uint64 maximum payload");
        assert_eq!(unsigned_value["integer"][1]["real"], "18446744073709551615");

        let complex_summary = v1_summary("int16", vec![1, 2], true);
        let complex_value = observation_value_v2(
            &complex_summary,
            complete_v1_preview(
                &complex_summary,
                vec![
                    kernel_v1::PreviewValue::Integer {
                        real: "-32768".to_owned(),
                        imaginary: "1".to_owned(),
                    },
                    kernel_v1::PreviewValue::Integer {
                        real: "7".to_owned(),
                        imaginary: "-9".to_owned(),
                    },
                ],
            ),
            2,
        )
        .expect("complex int16 payload");
        assert_eq!(complex_value["complex"], true);
        assert_eq!(
            complex_value["integer"],
            json!([
                {"real": "-32768", "imaginary": "1"},
                {"real": "7", "imaginary": "-9"}
            ])
        );
    }

    #[test]
    fn schema_v2_normalizer_accepts_empty_arrays_and_canonical_floats() {
        let empty_summary = v1_summary("double", vec![0, 3], false);
        let empty = observation_value_v2(
            &empty_summary,
            complete_v1_preview(&empty_summary, Vec::new()),
            0,
        )
        .expect("empty numeric payload");
        assert_eq!(empty["size"], json!([0, 3]));
        assert_eq!(empty["numel"], 0);
        assert_eq!(empty["real"], json!([]));
        assert_eq!(empty["imag"], json!([]));

        let float_summary = v1_summary("single", vec![1, 6], false);
        let floats = observation_value_v2(
            &float_summary,
            complete_v1_preview(
                &float_summary,
                vec![
                    kernel_v1::PreviewValue::Number { value: -0.0 },
                    kernel_v1::PreviewValue::Number { value: 2.5 },
                    kernel_v1::PreviewValue::Number {
                        value: f64::from(0.1_f32),
                    },
                    kernel_v1::PreviewValue::Special {
                        value: "nan".to_owned(),
                    },
                    kernel_v1::PreviewValue::Special {
                        value: "infinity".to_owned(),
                    },
                    kernel_v1::PreviewValue::Special {
                        value: "negativeInfinity".to_owned(),
                    },
                ],
            ),
            6,
        )
        .expect("canonical real floating payload");
        assert_eq!(
            floats["real"],
            json!(["-0", "2.5", "0.100000001", "NaN", "+Inf", "-Inf"])
        );
        assert_eq!(floats["imag"], json!(["0", "0", "0", "0", "0", "0"]));

        let malformed_single = v1_summary("single", vec![1, 1], false);
        assert_eq!(
            observation_value_v2(
                &malformed_single,
                complete_v1_preview(
                    &malformed_single,
                    vec![kernel_v1::PreviewValue::Number { value: 0.1 }],
                ),
                1,
            ),
            Err(ObservationV2Error::InvalidObservation)
        );

        let complex_summary = v1_summary("double", vec![1, 2], true);
        let complex = observation_value_v2(
            &complex_summary,
            complete_v1_preview(
                &complex_summary,
                vec![
                    kernel_v1::PreviewValue::Complex {
                        real: 1.0,
                        imaginary: -2.0,
                    },
                    kernel_v1::PreviewValue::Complex {
                        real: -0.0,
                        imaginary: 0.0,
                    },
                ],
            ),
            2,
        )
        .expect("finite complex floating payload");
        assert_eq!(complex["real"], json!(["1", "-0"]));
        assert_eq!(complex["imag"], json!(["-2", "0"]));

        let logical_summary = v1_summary("logical", vec![1, 2], false);
        let logical = observation_value_v2(
            &logical_summary,
            complete_v1_preview(
                &logical_summary,
                vec![
                    kernel_v1::PreviewValue::Logical { value: true },
                    kernel_v1::PreviewValue::Logical { value: false },
                ],
            ),
            2,
        )
        .expect("logical payload");
        assert_eq!(logical["kind"], "logical");
        assert_eq!(logical["logical"], json!([true, false]));
    }

    #[test]
    fn schema_v2_integer_normalizer_accepts_all_fixed_width_classes() {
        for (class, minimum, maximum) in [
            ("int8", "-128", "127"),
            ("uint8", "0", "255"),
            ("int16", "-32768", "32767"),
            ("uint16", "0", "65535"),
            ("int32", "-2147483648", "2147483647"),
            ("uint32", "0", "4294967295"),
            ("int64", "-9223372036854775808", "9223372036854775807"),
            ("uint64", "0", "18446744073709551615"),
        ] {
            let summary = v1_summary(class, vec![1, 2], false);
            let value = observation_value_v2(
                &summary,
                complete_v1_preview(
                    &summary,
                    vec![
                        kernel_v1::PreviewValue::Integer {
                            real: minimum.to_owned(),
                            imaginary: "0".to_owned(),
                        },
                        kernel_v1::PreviewValue::Integer {
                            real: maximum.to_owned(),
                            imaginary: "0".to_owned(),
                        },
                    ],
                ),
                2,
            )
            .expect("fixed-width integer endpoints");
            assert_eq!(value["class"], class);
            assert_eq!(value["integer"][0]["real"], minimum);
            assert_eq!(value["integer"][1]["real"], maximum);
        }
    }

    #[test]
    fn schema_v2_normalizer_rejects_lossy_or_inconsistent_payloads() {
        let logical_summary = v1_summary("logical", vec![1, 1], false);
        let good_logical = complete_v1_preview(
            &logical_summary,
            vec![kernel_v1::PreviewValue::Logical { value: true }],
        );

        let mut truncated = good_logical.clone();
        truncated.values.clear();
        truncated.truncation = PreviewTruncation {
            truncated: true,
            omitted_elements: 1,
        };
        assert_eq!(
            observation_value_v2(&logical_summary, truncated, 1),
            Err(ObservationV2Error::PayloadLimit)
        );

        let mut wrong_class = good_logical.clone();
        wrong_class.class = "double".to_owned();
        assert!(observation_value_v2(&logical_summary, wrong_class, 1).is_err());

        let mut wrong_shape = good_logical.clone();
        wrong_shape.dimensions = vec![1, 2];
        assert!(observation_value_v2(&logical_summary, wrong_shape, 1).is_err());

        let mut wrong_range = good_logical.clone();
        wrong_range.selected_range.start = vec![1, 2];
        assert!(observation_value_v2(&logical_summary, wrong_range, 1).is_err());

        let mut wrong_complex = good_logical.clone();
        wrong_complex.complex = true;
        assert!(observation_value_v2(&logical_summary, wrong_complex, 1).is_err());

        let wrong_kind = complete_v1_preview(
            &logical_summary,
            vec![kernel_v1::PreviewValue::CharCodeUnit { value: 1 }],
        );
        assert!(observation_value_v2(&logical_summary, wrong_kind, 1).is_err());

        let complex_float_summary = v1_summary("double", vec![1, 1], true);
        let nonfinite_complex = complete_v1_preview(
            &complex_float_summary,
            vec![kernel_v1::PreviewValue::Complex {
                real: f64::NAN,
                imaginary: 0.0,
            }],
        );
        assert!(observation_value_v2(&complex_float_summary, nonfinite_complex, 1).is_err());

        let integer_summary = v1_summary("int8", vec![1, 1], false);
        for real in ["01", "128", "-0"] {
            let preview = complete_v1_preview(
                &integer_summary,
                vec![kernel_v1::PreviewValue::Integer {
                    real: real.to_owned(),
                    imaginary: "0".to_owned(),
                }],
            );
            assert!(observation_value_v2(&integer_summary, preview, 1).is_err());
        }

        let string_summary = v1_summary("string", vec![1, 1], false);
        let invalid_missing = complete_v1_preview(
            &string_summary,
            vec![kernel_v1::PreviewValue::String {
                code_units: vec![65],
                missing: true,
            }],
        );
        assert!(observation_value_v2(&string_summary, invalid_missing, 1).is_err());
        let oversized_string = complete_v1_preview(
            &string_summary,
            vec![kernel_v1::PreviewValue::String {
                code_units: vec![65; 16_385],
                missing: false,
            }],
        );
        assert!(observation_value_v2(&string_summary, oversized_string, 1).is_err());

        let aggregate_summary = v1_summary("string", vec![1, 5], false);
        let aggregate_string = complete_v1_preview(
            &aggregate_summary,
            (0..5)
                .map(|_| kernel_v1::PreviewValue::String {
                    code_units: vec![65; 16_384],
                    missing: false,
                })
                .collect(),
        );
        assert!(observation_value_v2(&aggregate_summary, aggregate_string, 5).is_err());

        let unknown_summary = v1_summary("future_exact_kind", vec![1, 1], false);
        let legacy_payload =
            complete_v1_preview(&unknown_summary, vec![kernel_v1::PreviewValue::Missing]);
        assert_eq!(
            observation_value_v2(&unknown_summary, legacy_payload, 1),
            Err(ObservationV2Error::UnsupportedPayload)
        );
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn schema_v2_aggregate_conversion_is_recursive_exact_and_complete() {
        let empty_cell = kernel_v2::ExactValue {
            class: "cell".to_owned(),
            size: vec![0, 3],
            ndims: 2,
            numel: 0,
            complex: false,
            payload: kernel_v2::ExactPayload::Cell { items: Vec::new() },
        };
        let nested_struct = kernel_v2::ExactValue {
            class: "struct".to_owned(),
            size: vec![1, 1],
            ndims: 2,
            numel: 1,
            complex: false,
            payload: kernel_v2::ExactPayload::Struct {
                fields: vec!["beta".to_owned(), "alpha".to_owned(), "count".to_owned()],
                records: vec![kernel_v2::StructRecord::new(vec![
                    (
                        "count".to_owned(),
                        kernel_v2::ExactValue {
                            class: "uint64".to_owned(),
                            size: vec![1, 1],
                            ndims: 2,
                            numel: 1,
                            complex: true,
                            payload: kernel_v2::ExactPayload::Integer {
                                integer: vec![kernel_v2::IntegerValue {
                                    real: "18446744073709551615".to_owned(),
                                    imaginary: "1".to_owned(),
                                }],
                            },
                        },
                    ),
                    (
                        "beta".to_owned(),
                        kernel_v2::ExactValue {
                            class: "double".to_owned(),
                            size: vec![1, 1],
                            ndims: 2,
                            numel: 1,
                            complex: true,
                            payload: kernel_v2::ExactPayload::Numeric {
                                real: vec!["NaN".to_owned()],
                                imag: vec!["+Inf".to_owned()],
                            },
                        },
                    ),
                    (
                        "alpha".to_owned(),
                        kernel_v2::ExactValue {
                            class: "char".to_owned(),
                            size: vec![1, 1],
                            ndims: 2,
                            numel: 1,
                            complex: false,
                            payload: kernel_v2::ExactPayload::Char {
                                code_units: vec![55_357],
                            },
                        },
                    ),
                ])],
            },
        };
        let missing_string = kernel_v2::ExactValue {
            class: "string".to_owned(),
            size: vec![1, 1],
            ndims: 2,
            numel: 1,
            complex: false,
            payload: kernel_v2::ExactPayload::String {
                string_code_units: vec![Vec::new()],
                missing: vec![true],
            },
        };
        let summary = v1_summary("cell", vec![1, 3], false);
        let preview = kernel_v2::AggregatePreview::Cell {
            dimensions: vec![1, 3],
            selected_range: kernel_v2::MatrixRange {
                start: vec![1, 1],
                size: vec![1, 3],
            },
            items: vec![empty_cell, nested_struct, missing_string],
            truncation: PreviewTruncation {
                truncated: false,
                omitted_elements: 0,
            },
            usage: kernel_v2::PreviewUsage {
                nodes: 7,
                elements: 8,
                code_units: 15,
                depth: 2,
            },
        };
        let value = aggregate_observation_value_v2(
            &summary,
            preview,
            3,
            &kernel_v2::AggregateLimits::default(),
        )
        .expect("complete recursive aggregate");
        assert_eq!(value["items"][0]["size"], json!([0, 3]));
        assert_eq!(
            value["items"][1]["fields"],
            json!(["beta", "alpha", "count"])
        );
        assert_eq!(value["items"][1]["records"][0]["beta"]["real"][0], "NaN");
        assert_eq!(
            value["items"][1]["records"][0]["count"]["integer"][0]["real"],
            "18446744073709551615"
        );
        assert_eq!(value["items"][2]["missing"], json!([true]));

        let empty_struct_summary = v1_summary("struct", vec![0, 3], false);
        let empty_struct = kernel_v2::AggregatePreview::Struct {
            dimensions: vec![0, 3],
            selected_range: kernel_v2::MatrixRange {
                start: vec![1, 1],
                size: vec![0, 3],
            },
            fields: vec!["f".to_owned()],
            records: Vec::new(),
            truncation: PreviewTruncation {
                truncated: false,
                omitted_elements: 0,
            },
            usage: kernel_v2::PreviewUsage {
                nodes: 1,
                elements: 0,
                code_units: 1,
                depth: 0,
            },
        };
        let value = aggregate_observation_value_v2(
            &empty_struct_summary,
            empty_struct,
            0,
            &kernel_v2::AggregateLimits::default(),
        )
        .expect("fielded shaped-empty struct");
        assert_eq!(value["size"], json!([0, 3]));
        assert_eq!(value["fields"], json!(["f"]));
        assert_eq!(value["records"], json!([]));
    }

    #[test]
    fn schema_v2_table_aggregate_conversion_is_canonical_and_complete() {
        let summary = v1_summary("table", vec![2, 2], false);
        let value = aggregate_observation_value_v2(
            &summary,
            complete_table_preview(),
            4,
            &kernel_v2::AggregateLimits::default(),
        )
        .expect("complete table aggregate");

        assert_eq!(value["class"], "table");
        assert_eq!(value["size"], json!([2, 2]));
        assert_eq!(value["ndims"], 2);
        assert_eq!(value["numel"], 4);
        assert_eq!(value["complex"], false);
        assert_eq!(value["kind"], "table");
        assert_eq!(value["variableNames"], json!(["A", "B"]));
        assert_eq!(value["variables"][0]["kind"], "numeric");
        assert_eq!(value["variables"][0]["real"], json!(["1", "2"]));
        assert_eq!(value["variables"][1]["kind"], "logical");
        assert_eq!(value["variables"][1]["logical"], json!([true, false]));
    }

    #[test]
    fn schema_v2_table_aggregate_observes_zero_by_n_and_n_by_zero_shapes() {
        let zero_by_n_summary = v1_summary("table", vec![0, 2], false);
        let zero_by_n = kernel_v2::AggregatePreview::Table {
            dimensions: vec![0, 2],
            selected_range: kernel_v2::MatrixRange {
                start: vec![1, 1],
                size: vec![0, 2],
            },
            variable_names: vec!["A".to_owned(), "B".to_owned()],
            variables: vec![exact_double_column(&[]), exact_logical_column(Vec::new())],
            truncation: PreviewTruncation {
                truncated: false,
                omitted_elements: 0,
            },
            usage: kernel_v2::PreviewUsage {
                nodes: 3,
                elements: 2,
                code_units: 2,
                depth: 1,
            },
        };
        let value = aggregate_observation_value_v2(
            &zero_by_n_summary,
            zero_by_n,
            0,
            &kernel_v2::AggregateLimits::default(),
        )
        .expect("zero-row table retains variables");
        assert_eq!(value["size"], json!([0, 2]));
        assert_eq!(value["numel"], 0);
        assert_eq!(value["variableNames"], json!(["A", "B"]));
        assert_eq!(value["variables"][0]["size"], json!([0, 1]));
        assert_eq!(
            conformance_v2_inspect_max_elements(&zero_by_n_summary, 0),
            2
        );

        let n_by_zero_summary = v1_summary("table", vec![3, 0], false);
        let n_by_zero = kernel_v2::AggregatePreview::Table {
            dimensions: vec![3, 0],
            selected_range: kernel_v2::MatrixRange {
                start: vec![1, 1],
                size: vec![3, 0],
            },
            variable_names: Vec::new(),
            variables: Vec::new(),
            truncation: PreviewTruncation {
                truncated: false,
                omitted_elements: 0,
            },
            usage: kernel_v2::PreviewUsage {
                nodes: 1,
                elements: 0,
                code_units: 0,
                depth: 0,
            },
        };
        let value = aggregate_observation_value_v2(
            &n_by_zero_summary,
            n_by_zero,
            0,
            &kernel_v2::AggregateLimits::default(),
        )
        .expect("zero-variable table retains rows");
        assert_eq!(value["size"], json!([3, 0]));
        assert_eq!(value["numel"], 0);
        assert_eq!(value["variableNames"], json!([]));
        assert_eq!(value["variables"], json!([]));
    }

    #[test]
    fn schema_v2_table_aggregate_maps_a_valid_variable_prefix_to_payload_limit() {
        let summary = v1_summary("table", vec![2, 2], false);
        let mut preview = complete_table_preview();
        let kernel_v2::AggregatePreview::Table {
            variable_names,
            variables,
            truncation,
            usage,
            ..
        } = &mut preview
        else {
            unreachable!()
        };
        variable_names.pop();
        variables.pop();
        *truncation = PreviewTruncation {
            truncated: true,
            omitted_elements: 1,
        };
        *usage = kernel_v2::PreviewUsage {
            nodes: 2,
            elements: 3,
            code_units: 1,
            depth: 1,
        };

        assert_eq!(
            aggregate_observation_value_v2(
                &summary,
                preview,
                4,
                &kernel_v2::AggregateLimits::default(),
            ),
            Err(ObservationV2Error::PayloadLimit)
        );
    }

    #[test]
    fn schema_v2_table_aggregate_rejects_summary_shape_and_range_mismatches() {
        let summary = v1_summary("table", vec![2, 2], false);
        for wrong_summary in [
            v1_summary("cell", vec![2, 2], false),
            v1_summary("table", vec![2, 2], true),
            v1_summary("table", vec![2, 2, 1], false),
        ] {
            assert_eq!(
                aggregate_observation_value_v2(
                    &wrong_summary,
                    complete_table_preview(),
                    4,
                    &kernel_v2::AggregateLimits::default(),
                ),
                Err(ObservationV2Error::InvalidObservation)
            );
        }

        let mut wrong_shape = complete_table_preview();
        let kernel_v2::AggregatePreview::Table { dimensions, .. } = &mut wrong_shape else {
            unreachable!()
        };
        *dimensions = vec![2, 3];
        assert_eq!(
            aggregate_observation_value_v2(
                &summary,
                wrong_shape,
                4,
                &kernel_v2::AggregateLimits::default(),
            ),
            Err(ObservationV2Error::InvalidObservation)
        );

        let mut partial_range = complete_table_preview();
        let kernel_v2::AggregatePreview::Table {
            selected_range,
            variable_names,
            variables,
            usage,
            ..
        } = &mut partial_range
        else {
            unreachable!()
        };
        *selected_range = kernel_v2::MatrixRange {
            start: vec![1, 2],
            size: vec![2, 1],
        };
        variable_names.remove(0);
        variables.remove(0);
        *usage = kernel_v2::PreviewUsage {
            nodes: 2,
            elements: 3,
            code_units: 1,
            depth: 1,
        };
        assert_eq!(
            aggregate_observation_value_v2(
                &summary,
                partial_range,
                4,
                &kernel_v2::AggregateLimits::default(),
            ),
            Err(ObservationV2Error::InvalidObservation)
        );
    }

    #[test]
    fn schema_v2_table_aggregate_rejects_name_and_variable_row_mismatches() {
        let summary = v1_summary("table", vec![2, 2], false);
        let mut mismatched_names = complete_table_preview();
        let kernel_v2::AggregatePreview::Table { variable_names, .. } = &mut mismatched_names
        else {
            unreachable!()
        };
        variable_names.pop();
        assert_eq!(
            aggregate_observation_value_v2(
                &summary,
                mismatched_names,
                4,
                &kernel_v2::AggregateLimits::default(),
            ),
            Err(ObservationV2Error::InvalidObservation)
        );

        let mut mismatched_rows = complete_table_preview();
        let kernel_v2::AggregatePreview::Table {
            variables, usage, ..
        } = &mut mismatched_rows
        else {
            unreachable!()
        };
        variables[0] = exact_double_column(&["1"]);
        *usage = kernel_v2::PreviewUsage {
            nodes: 3,
            elements: 5,
            code_units: 2,
            depth: 1,
        };
        assert_eq!(
            aggregate_observation_value_v2(
                &summary,
                mismatched_rows,
                4,
                &kernel_v2::AggregateLimits::default(),
            ),
            Err(ObservationV2Error::InvalidObservation)
        );
    }

    #[test]
    fn schema_v2_routes_aggregate_limits_and_inspect_failures() {
        let summary = v1_summary("cell", vec![1, 2], false);
        let preview = kernel_v2::AggregatePreview::Cell {
            dimensions: vec![1, 2],
            selected_range: kernel_v2::MatrixRange {
                start: vec![1, 1],
                size: vec![1, 2],
            },
            items: vec![kernel_v2::ExactValue {
                class: "logical".to_owned(),
                size: vec![1, 1],
                ndims: 2,
                numel: 1,
                complex: false,
                payload: kernel_v2::ExactPayload::Logical {
                    logical: vec![true],
                },
            }],
            truncation: PreviewTruncation {
                truncated: true,
                omitted_elements: 1,
            },
            usage: kernel_v2::PreviewUsage {
                nodes: 2,
                elements: 2,
                code_units: 0,
                depth: 1,
            },
        };
        assert_eq!(
            aggregate_observation_value_v2(
                &summary,
                preview,
                2,
                &kernel_v2::AggregateLimits::default(),
            ),
            Err(ObservationV2Error::PayloadLimit)
        );
        for (category, expected) in [
            ("workspace.unsupportedValue", "unsupported-payload"),
            ("workspace.previewLimit", "payload-limit"),
            ("workspace.previewDepth", "payload-limit"),
            ("workspace.cyclicValue", "cyclic-payload"),
            ("engine.invalidPreview", "invalid-observation"),
        ] {
            let observation =
                map_schema_v2_inspect_error("routing", ProtocolError::new(category, "test"))
                    .expect("mapped synthetic result");
            assert_eq!(observation["error"]["category"], expected);
        }
    }

    #[test]
    fn schema_v2_codec_rejects_unknown_preview_kinds() {
        let limits = kernel_v1::PreviewLimits::default();
        let request = kernel_v1::RequestEnvelope::new(
            SESSION_ID,
            "unknown-kind-request",
            kernel_v1::Request::Inspect(kernel_v1::InspectRequest {
                name: CONFORMANCE_RESULT.to_owned(),
                range: kernel_v1::MatrixRange {
                    start: vec![1, 1],
                    size: vec![1, 1],
                },
                max_elements: 1,
            }),
        );
        let response = json!({
            "protocol": kernel_v1::PROTOCOL_V1,
            "sessionId": SESSION_ID,
            "messageId": "unknown-kind-response",
            "kind": "response",
            "replyTo": "unknown-kind-request",
            "ok": true,
            "result": {
                "type": "inspect",
                "data": {
                    "class": "char",
                    "dimensions": [1, 1],
                    "complex": false,
                    "selectedRange": { "start": [1, 1], "size": [1, 1] },
                    "values": [{ "kind": "futureExactKind", "value": 65 }],
                    "truncation": { "truncated": false, "omittedElements": 0 }
                }
            }
        });
        assert!(
            kernel_v1::decode_response_for_request(&response.to_string(), &request, &limits)
                .is_err()
        );
    }

    #[test]
    fn schema_v2_offer_and_aggregate_downgrade_are_exact() {
        for selected in [
            kernel_v2::ProtocolVersion::V1,
            kernel_v2::ProtocolVersion::V0,
        ] {
            let mut handler = ScriptedSchemaV2SessionHandler {
                selected,
                fault: None,
            };
            let observation = execute_conformance_case_v2(
                &mut handler,
                "aggregate_fallback",
                "openmat_result = {};".to_owned(),
                "programs/aggregate_fallback.m",
                true,
            )
            .expect("aggregate downgrade is a synthetic observation");
            assert_eq!(observation["outcome"], "error");
            assert_eq!(observation["error"]["category"], "unsupported-payload");
        }

        let mut handler = ScriptedSchemaV2SessionHandler {
            selected: kernel_v2::ProtocolVersion::V0,
            fault: None,
        };
        assert!(matches!(
            execute_conformance_case_v2(
                &mut handler,
                "scalar_fallback",
                "openmat_result = 1;".to_owned(),
                "programs/scalar_fallback.m",
                false,
            ),
            Err(ConformanceRunError::Capability(detail)) if detail.contains(PROTOCOL_V0)
        ));
    }

    #[test]
    fn schema_v2_driver_rejects_unoffered_v3_selection() {
        let mut handler = ScriptedSchemaV2SessionHandler {
            selected: kernel_v2::ProtocolVersion::V3,
            fault: None,
        };
        assert!(matches!(
            SchemaV2SessionDriver::initialize(&mut handler),
            Err(SchemaV2DriverError::Protocol(_))
        ));
    }

    #[test]
    fn schema_v2_driver_rejects_v3_messages_after_older_negotiation() {
        struct MixedProtocolHandler(kernel_v2::ProtocolVersion);

        impl KernelSessionHandler for MixedProtocolHandler {
            fn handle_session(
                &mut self,
                request: KernelSessionRequest<'_>,
            ) -> Result<Vec<KernelSessionMessage>, KernelSessionError> {
                if let KernelSessionRequest::Bootstrap(request) = request {
                    return Ok(scripted_schema_v2_bootstrap(request, self.0));
                }
                Ok(vec![KernelSessionMessage::V3(
                    openmat_protocol::kernel_v3::ServerMessage::Event(
                        openmat_protocol::kernel_v3::EventEnvelope::new(
                            SESSION_ID,
                            "unexpected-v3-status",
                            Event::Status(openmat_protocol::StatusEvent {
                                status: openmat_protocol::KernelStatus::Idle,
                            }),
                        ),
                    ),
                )])
            }
        }

        for selected in [
            kernel_v2::ProtocolVersion::V0,
            kernel_v2::ProtocolVersion::V1,
            kernel_v2::ProtocolVersion::V2,
        ] {
            let mut handler = MixedProtocolHandler(selected);
            let mut driver = SchemaV2SessionDriver::initialize(&mut handler)
                .expect("supported protocol negotiation");
            assert_eq!(driver.protocol(), selected);
            assert!(matches!(
                driver.exchange(SchemaV2Request::Shutdown),
                Err(SchemaV2DriverError::Protocol(message)) if message.contains("post-bootstrap")
            ));
        }
    }

    #[test]
    fn schema_v2_driver_validates_request_aware_response_invariants() {
        for selected in [
            kernel_v2::ProtocolVersion::V2,
            kernel_v2::ProtocolVersion::V1,
        ] {
            for fault in [
                SchemaV2SessionFault::Correlation,
                SchemaV2SessionFault::ResponseType,
                SchemaV2SessionFault::SelectedRange,
                SchemaV2SessionFault::RequestLimit,
            ] {
                let mut handler = ScriptedSchemaV2SessionHandler {
                    selected,
                    fault: Some(fault),
                };
                let mut driver =
                    SchemaV2SessionDriver::initialize(&mut handler).expect("v2-capable bootstrap");
                let request = match fault {
                    SchemaV2SessionFault::Correlation | SchemaV2SessionFault::ResponseType => {
                        SchemaV2Request::Execute(ExecuteRequest {
                            code: "answer = 1;".to_owned(),
                            source_name: "request-aware.m".to_owned(),
                            mode: ExecutionMode::File,
                        })
                    }
                    SchemaV2SessionFault::SelectedRange => {
                        SchemaV2Request::Inspect(kernel_v1::InspectRequest {
                            name: CONFORMANCE_RESULT.to_owned(),
                            range: kernel_v1::MatrixRange {
                                start: vec![1, 1],
                                size: vec![1, 1],
                            },
                            max_elements: 1,
                        })
                    }
                    SchemaV2SessionFault::RequestLimit => {
                        SchemaV2Request::Inspect(kernel_v1::InspectRequest {
                            name: CONFORMANCE_RESULT.to_owned(),
                            range: kernel_v1::MatrixRange {
                                start: vec![1, 1],
                                size: vec![1, 2],
                            },
                            max_elements: 1,
                        })
                    }
                };
                assert!(matches!(
                    driver.exchange(request),
                    Err(SchemaV2DriverError::Protocol(_))
                ));
            }
        }
    }

    #[test]
    fn schema_v2_shutdown_failure_overrides_a_completed_observation() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut engine = FakeEngine::successful(Arc::clone(&calls));
        engine.shutdown = Err(EngineError::new(
            "runtime.shutdown",
            "test shutdown failure",
        ));
        let mut session = KernelSession::new(SESSION_ID, engine);
        let result = execute_conformance_case_v2(
            &mut session,
            "shutdown_case",
            "openmat_result = 1;".to_owned(),
            "programs/shutdown_case.m",
            false,
        );
        assert!(matches!(
            result,
            Err(ConformanceRunError::Internal(category)) if category == "runtime.shutdown"
        ));
        assert_eq!(
            calls.lock().expect("call log lock").as_slice(),
            [
                EngineCall::Initialize,
                EngineCall::Execute(ExecuteRequest {
                    code: "openmat_result = 1;".to_owned(),
                    source_name: "programs/shutdown_case.m".to_owned(),
                    mode: ExecutionMode::File,
                }),
                EngineCall::ListWorkspace,
                EngineCall::Shutdown,
            ]
        );
    }

    #[test]
    fn schema_v2_conformance_uses_kernel_session_and_emits_exact_observation() {
        let root = temporary_case_directory("schema-v2-session");
        let expected = json!({
            "class": "char",
            "size": [1, 3],
            "ndims": 2,
            "numel": 3,
            "complex": false,
            "kind": "char",
            "code_units": [65, 55_357, 66]
        });
        let manifest = write_case_v2(
            &root,
            "schema_v2_char",
            "openmat_result = char([65, 55357, 66]);\n",
            &expected,
        );
        let calls = Arc::new(Mutex::new(Vec::new()));
        let summary = v1_summary("char", vec![1, 3], false);
        let mut engine = FakeEngine::successful(Arc::clone(&calls));
        engine.workspace = Ok(vec![summary.clone()]);
        engine.preview_v1 = Ok(complete_v1_preview(
            &summary,
            vec![
                kernel_v1::PreviewValue::CharCodeUnit { value: 65 },
                kernel_v1::PreviewValue::CharCodeUnit { value: 55_357 },
                kernel_v1::PreviewValue::CharCodeUnit { value: 66 },
            ],
        ));
        let mut output = Vec::new();
        let mut errors = Vec::new();
        let exit = run_with(
            [
                "openmat-cli".to_owned(),
                "conformance".to_owned(),
                manifest.to_string_lossy().into_owned(),
            ],
            &mut output,
            &mut errors,
            |path: &Path| fs::read(path),
            canonicalize_utf8_path,
            |_| Ok(engine),
        );

        assert_eq!(exit, EXIT_SUCCESS);
        assert!(errors.is_empty(), "{}", String::from_utf8_lossy(&errors));
        let observation: JsonValue = serde_json::from_slice(&output).expect("schema-v2 JSON");
        assert_eq!(observation["schema_version"], 2);
        assert_eq!(observation["case_id"], "schema_v2_char");
        assert_eq!(observation["value"], expected);
        assert!(matches!(
            calls.lock().expect("call log lock").as_slice(),
            [
                EngineCall::Initialize,
                EngineCall::Execute(_),
                EngineCall::ListWorkspace,
                EngineCall::InspectV1(_),
                EngineCall::Shutdown,
            ]
        ));
        fs::remove_dir_all(root).expect("remove temporary schema-v2 case");
    }

    #[test]
    fn run_normalizes_relative_unicode_entry_for_search_and_request() {
        let root = temporary_relative_directory("入口-path");
        let source_path = root.join("入口.m");
        let code = "answer = 42;\r\n";
        fs::write(&source_path, code).expect("entry source");
        let canonical_source = fs::canonicalize(&source_path).expect("canonical entry source");
        let calls = Arc::new(Mutex::new(Vec::new()));
        let engine = FakeEngine::successful(Arc::clone(&calls));
        let configured_paths = Arc::new(Mutex::new(Vec::new()));
        let observed_paths = Arc::clone(&configured_paths);
        let mut output = Vec::new();
        let mut errors = Vec::new();
        let exit = run_with(
            [
                "openmat-cli".to_owned(),
                source_path.to_string_lossy().into_owned(),
            ],
            &mut output,
            &mut errors,
            |path: &Path| fs::read(path),
            canonicalize_utf8_path,
            move |configuration| {
                assert!(configuration.oex_plugins.is_empty());
                *observed_paths.lock().expect("search path lock") = configuration.search_paths;
                Ok(engine)
            },
        );

        assert_eq!(exit, EXIT_SUCCESS);
        assert!(output.is_empty());
        assert!(errors.is_empty());
        assert_eq!(
            configured_paths
                .lock()
                .expect("search path lock")
                .as_slice(),
            [canonical_source
                .parent()
                .expect("entry parent")
                .to_path_buf()]
        );
        assert_eq!(
            calls.lock().expect("call log lock").as_slice(),
            [
                EngineCall::Initialize,
                EngineCall::Execute(ExecuteRequest {
                    code: code.to_owned(),
                    source_name: stable_source_name(&canonical_source),
                    mode: ExecutionMode::File,
                }),
                EngineCall::Shutdown,
            ]
        );
        fs::remove_dir_all(root).expect("remove relative temporary directory");
    }

    #[test]
    fn direct_file_executes_same_directory_sources_and_lists_workspace() {
        let root = temporary_relative_directory("同目录-e2e");
        fs::create_dir_all(&root).expect("entry directory");
        fs::write(
            root.join("triple.m"),
            "function value = triple(input)\nvalue = input * 3;\nend\n",
        )
        .expect("helper function");
        fs::write(root.join("setup.m"), "from_script = triple(5);\n").expect("helper script");
        fs::write(
            root.join("BoxValue.m"),
            "classdef BoxValue\nproperties\nvalue = 4\nend\nend\n",
        )
        .expect("helper class");
        let main_path = root.join("入口.m");
        fs::write(
            &main_path,
            "setup\nitem = BoxValue();\nanswer = from_script + item.value;\ndisp(answer);\n",
        )
        .expect("entry script");
        let mut output = Vec::new();
        let mut errors = Vec::new();
        let exit = run(
            [
                "openmat-cli".to_owned(),
                main_path.to_string_lossy().into_owned(),
                "--workspace".to_owned(),
            ],
            &mut output,
            &mut errors,
        );

        assert_eq!(exit, EXIT_SUCCESS);
        assert!(errors.is_empty());
        let output = String::from_utf8(output).expect("UTF-8 output");
        assert!(output.starts_with("    19\n"), "{output:?}");
        assert!(output.contains("answer\tdouble\t1x1\n"), "{output:?}");
        assert!(output.contains("from_script\tdouble\t1x1\n"), "{output:?}");
        assert!(output.contains("item\tBoxValue\t1x1\n"), "{output:?}");
        fs::remove_dir_all(root).expect("remove temporary entry directory");
    }

    #[cfg(windows)]
    #[test]
    fn direct_file_loads_a_real_oex_plugin_from_the_command_line() {
        if std::process::Command::new("gcc")
            .arg("--version")
            .output()
            .is_err()
        {
            eprintln!("skipping real CLI OEX fixture because gcc is unavailable");
            return;
        }
        let root = temporary_case_directory("oex-e2e");
        fs::create_dir_all(&root).expect("plugin test directory");
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("CLI crate beneath repository root");
        let test_executable = std::env::current_exe().expect("locate CLI test executable");
        let dependency_directory = test_executable
            .parent()
            .expect("CLI test beneath target dependency directory");
        let profile_directory = dependency_directory
            .parent()
            .expect("CLI test beneath target profile directory");
        let bridge = [
            dependency_directory.join("openmat_oex.dll"),
            profile_directory.join("openmat_oex.dll"),
        ]
        .into_iter()
        .find(|path| path.is_file())
        .expect("cargo must build the OEX bridge DLL");
        fs::copy(&bridge, root.join("openmat_oex.dll")).expect("copy OEX bridge beside plugin");
        let plugin = root.join("arithmetic.oex.dll");
        let status = std::process::Command::new("gcc")
            .arg("-shared")
            .arg("-std=c11")
            .arg("-O2")
            .arg("-Wall")
            .arg("-Wextra")
            .arg("-Werror")
            .arg("-I")
            .arg(repository.join("include"))
            .arg(repository.join("crates/openmat-oex/tests/fixtures/arithmetic_plugin.c"))
            .arg(&bridge)
            .arg("-o")
            .arg(&plugin)
            .status()
            .expect("compile CLI OEX fixture");
        assert!(status.success(), "CLI OEX fixture must compile cleanly");
        let source = root.join("plugin_call.m");
        fs::write(&source, "result = add2(7, 8);\n").expect("plugin entry source");

        let mut output = Vec::new();
        let mut errors = Vec::new();
        let exit = run(
            [
                "openmat-cli".to_owned(),
                "run".to_owned(),
                source.to_string_lossy().into_owned(),
                "--oex-plugin".to_owned(),
                plugin.to_string_lossy().into_owned(),
                "--workspace".to_owned(),
            ],
            &mut output,
            &mut errors,
        );
        assert_eq!(exit, EXIT_SUCCESS, "{}", String::from_utf8_lossy(&errors));
        assert!(errors.is_empty());
        assert_eq!(
            String::from_utf8(output).expect("UTF-8 workspace"),
            "result\tdouble\t1x1\n"
        );
        fs::remove_dir_all(root).expect("remove CLI OEX fixture");
    }

    #[test]
    fn run_reports_invalid_utf8_dependency_as_runtime_failure() {
        let root = temporary_case_directory("support-utf8");
        fs::create_dir_all(&root).expect("entry directory");
        let main_path = root.join("main.m");
        fs::write(&main_path, "answer = invalid_helper();\n").expect("entry script");
        fs::write(root.join("invalid_helper.m"), [0xff, 0xfe]).expect("invalid helper");
        let mut output = Vec::new();
        let mut errors = Vec::new();
        let exit = run(
            [
                "openmat-cli".to_owned(),
                "run".to_owned(),
                main_path.to_string_lossy().into_owned(),
            ],
            &mut output,
            &mut errors,
        );

        assert_eq!(exit, EXIT_DATA_ERROR);
        assert!(output.is_empty());
        let errors = String::from_utf8(errors).expect("UTF-8 errors");
        assert!(errors.contains("error[source.invalidUtf8]"), "{errors}");
        fs::remove_dir_all(root).expect("remove temporary entry directory");
    }

    #[test]
    fn conformance_command_uses_runtime_kernel_and_emits_complete_observation() {
        let root = temporary_case_directory("observation");
        let manifest = write_case(&root, "scalar_case", "openmat_result = 42;\n");
        let mut output = Vec::new();
        let mut errors = Vec::new();
        let exit = run(
            [
                "openmat-cli".to_owned(),
                "conformance".to_owned(),
                manifest.to_string_lossy().into_owned(),
            ],
            &mut output,
            &mut errors,
        );
        assert_eq!(exit, EXIT_SUCCESS);
        assert!(errors.is_empty());
        let observation: JsonValue = serde_json::from_slice(&output).expect("observation is JSON");
        assert_eq!(observation["case_id"], "scalar_case");
        assert_eq!(observation["oracle"]["name"], "OpenMat");
        assert_eq!(observation["outcome"], "ok");
        assert_eq!(observation["value"]["class"], "double");
        assert_eq!(observation["value"]["size"], json!([1, 1]));
        assert_eq!(observation["value"]["ndims"], 2);
        assert_eq!(observation["value"]["numel"], 1);
        assert_eq!(observation["value"]["real"], json!(["42"]));
        assert_eq!(observation["value"]["imag"], json!(["0"]));
        fs::remove_dir_all(root).expect("remove temporary case");
    }

    #[test]
    fn conformance_runtime_error_contains_only_normalized_category() {
        let root = temporary_case_directory("error");
        let manifest = write_case(&root, "error_case", "missing_name;\n");
        let mut output = Vec::new();
        let mut errors = Vec::new();
        let exit = run(
            [
                "openmat-cli".to_owned(),
                "conformance".to_owned(),
                manifest.to_string_lossy().into_owned(),
            ],
            &mut output,
            &mut errors,
        );
        assert_eq!(exit, EXIT_SUCCESS);
        assert!(errors.is_empty());
        let observation: JsonValue = serde_json::from_slice(&output).expect("observation is JSON");
        assert_eq!(observation["outcome"], "error");
        assert_eq!(observation["error"], json!({"category": "undefined-name"}));
        fs::remove_dir_all(root).expect("remove temporary case");
    }

    #[test]
    fn conformance_normalizes_index_category_without_reading_error_prose() {
        let root = temporary_case_directory("index-error");
        let manifest = write_case(&root, "index_error_case", "openmat_result = value(2);\n");
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut engine = FakeEngine::successful(Arc::clone(&calls));
        engine.execution = Err(EngineError::new(
            "runtime.indexOutOfBounds",
            "undefined name prose must not select the observation category",
        ));
        let mut output = Vec::new();
        let mut errors = Vec::new();
        let exit = run_with(
            [
                "openmat-cli".to_owned(),
                "conformance".to_owned(),
                manifest.to_string_lossy().into_owned(),
            ],
            &mut output,
            &mut errors,
            |path: &Path| fs::read(path),
            canonicalize_utf8_path,
            |_| Ok(engine),
        );

        assert_eq!(exit, EXIT_SUCCESS);
        assert!(errors.is_empty());
        let observation: JsonValue = serde_json::from_slice(&output).expect("observation is JSON");
        assert_eq!(observation["outcome"], "error");
        assert_eq!(
            observation["error"],
            json!({"category": "index-out-of-bounds"})
        );
        assert_eq!(
            calls.lock().expect("call log lock").as_slice(),
            [
                EngineCall::Initialize,
                EngineCall::Execute(ExecuteRequest {
                    code: "openmat_result = value(2);\n".to_owned(),
                    source_name: stable_source_name(
                        &fs::canonicalize(root.join("programs").join("index_error_case.m"))
                            .expect("canonical source path"),
                    ),
                    mode: ExecutionMode::File,
                }),
                EngineCall::Shutdown,
            ]
        );
        fs::remove_dir_all(root).expect("remove temporary case");
    }

    #[test]
    fn conformance_normalizes_dimension_category_without_reading_error_prose() {
        let root = temporary_case_directory("dimension-error");
        let manifest = write_case(
            &root,
            "dimension_error_case",
            "openmat_result = left / right;\n",
        );
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut engine = FakeEngine::successful(Arc::clone(&calls));
        engine.execution = Err(EngineError::new(
            "runtime.dimensionMismatch",
            "undefined name prose must not select the observation category",
        ));
        let mut output = Vec::new();
        let mut errors = Vec::new();
        let exit = run_with(
            [
                "openmat-cli".to_owned(),
                "conformance".to_owned(),
                manifest.to_string_lossy().into_owned(),
            ],
            &mut output,
            &mut errors,
            |path: &Path| fs::read(path),
            canonicalize_utf8_path,
            |_| Ok(engine),
        );

        assert_eq!(exit, EXIT_SUCCESS);
        assert!(errors.is_empty());
        let observation: JsonValue = serde_json::from_slice(&output).expect("observation is JSON");
        assert_eq!(observation["outcome"], "error");
        assert_eq!(
            observation["error"],
            json!({"category": "dimension-mismatch"})
        );
        assert_eq!(
            calls.lock().expect("call log lock").as_slice(),
            [
                EngineCall::Initialize,
                EngineCall::Execute(ExecuteRequest {
                    code: "openmat_result = left / right;\n".to_owned(),
                    source_name: stable_source_name(
                        &fs::canonicalize(root.join("programs").join("dimension_error_case.m"))
                            .expect("canonical source path"),
                    ),
                    mode: ExecutionMode::File,
                }),
                EngineCall::Shutdown,
            ]
        );
        fs::remove_dir_all(root).expect("remove temporary case");
    }

    #[test]
    fn conformance_normalizes_aggregate_assignment_cardinality_as_index_error() {
        let root = temporary_case_directory("aggregate-shape-error");
        let manifest = write_case(
            &root,
            "aggregate_shape_error_case",
            "cells = {1, 2}; cells(1:2) = {3, 4, 5};\n",
        );
        let mut output = Vec::new();
        let mut errors = Vec::new();
        let exit = run(
            [
                "openmat-cli".to_owned(),
                "conformance".to_owned(),
                manifest.to_string_lossy().into_owned(),
            ],
            &mut output,
            &mut errors,
        );

        assert_eq!(exit, EXIT_SUCCESS);
        assert!(errors.is_empty());
        let observation: JsonValue = serde_json::from_slice(&output).expect("observation is JSON");
        assert_eq!(observation["outcome"], "error");
        assert_eq!(
            observation["error"],
            json!({"category": "index-out-of-bounds"})
        );
        fs::remove_dir_all(root).expect("remove temporary case");
    }

    #[test]
    fn conformance_preserves_structured_access_violation_categories() {
        assert_eq!(
            normalized_error_category("access-violation"),
            "access-violation"
        );
        assert_eq!(
            normalized_error_category("runtime.accessViolation"),
            "access-violation"
        );
        assert_eq!(
            normalized_error_category("runtime.unknownFunctionHandleTarget"),
            "undefined-name"
        );
        assert_eq!(normalized_error_category("runtime.object"), "other");
    }

    #[test]
    fn conformance_unknown_function_handle_target_is_undefined_name() {
        let root = temporary_case_directory("unknown-function-handle");
        let manifest = write_case(
            &root,
            "unknown_function_handle",
            "handle = @absent_target;\nopenmat_result = handle();\n",
        );
        let mut output = Vec::new();
        let mut errors = Vec::new();
        let exit = run(
            [
                "openmat-cli".to_owned(),
                "conformance".to_owned(),
                manifest.to_string_lossy().into_owned(),
            ],
            &mut output,
            &mut errors,
        );

        assert_eq!(exit, EXIT_SUCCESS);
        assert!(errors.is_empty());
        let observation: JsonValue = serde_json::from_slice(&output).expect("observation is JSON");
        assert_eq!(observation["outcome"], "error");
        assert_eq!(observation["error"], json!({"category": "undefined-name"}));
        fs::remove_dir_all(root).expect("remove temporary case");
    }

    #[test]
    fn conformance_schema_v2_observes_single_with_oracle_spelling() {
        let root = temporary_case_directory("single-observation");
        let manifest = write_case_v2(
            &root,
            "single_observation",
            "openmat_result = complex(single(0.1), single(-2));\n",
            &json!({
                "class": "single",
                "size": [1, 1],
                "ndims": 2,
                "numel": 1,
                "complex": true,
                "kind": "numeric",
                "real": ["0.100000001"],
                "imag": ["-2"]
            }),
        );
        let mut output = Vec::new();
        let mut errors = Vec::new();
        let exit = run(
            [
                "openmat-cli".to_owned(),
                "conformance".to_owned(),
                manifest.to_string_lossy().into_owned(),
            ],
            &mut output,
            &mut errors,
        );

        assert_eq!(exit, EXIT_SUCCESS);
        assert!(errors.is_empty());
        let observation: JsonValue = serde_json::from_slice(&output).expect("observation is JSON");
        assert_eq!(observation["outcome"], "ok");
        assert_eq!(observation["value"]["kind"], "numeric");
        assert_eq!(observation["value"]["class"], "single");
        assert_eq!(observation["value"]["complex"], true);
        assert_eq!(observation["value"]["real"], json!(["0.100000001"]));
        assert_eq!(observation["value"]["imag"], json!(["-2"]));
        fs::remove_dir_all(root).expect("remove temporary case");
    }

    #[test]
    fn conformance_executes_function_script_and_class_from_support_directory() {
        let root = temporary_case_directory("support");
        let manifest = write_case(
            &root,
            "support_case",
            "support_setup\nitem = SupportBox();\nopenmat_result = support_triple(from_script) + item.value;\n",
        );
        fs::write(
            root.join("support").join("support_triple.m"),
            "function value = support_triple(input)\nvalue = input * 3;\nend\n",
        )
        .expect("support function");
        fs::write(
            root.join("support").join("support_setup.m"),
            "from_script = 5;\n",
        )
        .expect("support script");
        fs::write(
            root.join("support").join("SupportBox.m"),
            "classdef SupportBox\nproperties\nvalue = 4\nend\nend\n",
        )
        .expect("support class");
        let mut output = Vec::new();
        let mut errors = Vec::new();
        let exit = run(
            [
                "openmat-cli".to_owned(),
                "conformance".to_owned(),
                manifest.to_string_lossy().into_owned(),
            ],
            &mut output,
            &mut errors,
        );
        assert_eq!(exit, EXIT_SUCCESS);
        assert!(errors.is_empty());
        let observation: JsonValue = serde_json::from_slice(&output).expect("observation is JSON");
        assert_eq!(observation["outcome"], "ok");
        assert_eq!(observation["value"]["real"], json!(["19"]));
        fs::remove_dir_all(root).expect("remove temporary case");
    }

    #[test]
    fn conformance_prefers_entry_directory_before_support_directory() {
        let root = temporary_case_directory("search-priority");
        let manifest = write_case(&root, "priority_case", "openmat_result = chosen_value();\n");
        fs::write(
            root.join("programs").join("chosen_value.m"),
            "function value = chosen_value()\nvalue = 1;\nend\n",
        )
        .expect("entry-directory function");
        fs::write(
            root.join("support").join("chosen_value.m"),
            "function value = chosen_value()\nvalue = 2;\nend\n",
        )
        .expect("support-directory function");
        let mut output = Vec::new();
        let mut errors = Vec::new();
        let exit = run(
            [
                "openmat-cli".to_owned(),
                "conformance".to_owned(),
                manifest.to_string_lossy().into_owned(),
            ],
            &mut output,
            &mut errors,
        );

        assert_eq!(exit, EXIT_SUCCESS);
        assert!(errors.is_empty());
        let observation: JsonValue = serde_json::from_slice(&output).expect("observation is JSON");
        assert_eq!(observation["outcome"], "ok");
        assert_eq!(observation["value"]["real"], json!(["1"]));
        fs::remove_dir_all(root).expect("remove temporary case");
    }

    #[test]
    fn conformance_payload_preserves_column_major_complex_special_and_signed_zero_values() {
        let summary = VariableSummary {
            name: CONFORMANCE_RESULT.to_owned(),
            class: "double".to_owned(),
            dimensions: vec![2, 2],
            complex: true,
            bytes: Some(64),
        };
        let preview = MatrixPreview {
            class: "double".to_owned(),
            dimensions: vec![2, 2],
            selected_range: MatrixRange {
                start: vec![1, 1],
                size: vec![2, 2],
            },
            values: vec![
                PreviewValue::Number { value: 1.0 },
                PreviewValue::Complex {
                    real: 2.0,
                    imaginary: -3.0,
                },
                PreviewValue::Special {
                    value: "nan".to_owned(),
                },
                PreviewValue::Number { value: -0.0 },
            ],
            truncation: PreviewTruncation {
                truncated: false,
                omitted_elements: 0,
            },
        };
        let value = observation_value(&summary, preview, 4).expect("supported payload");
        assert_eq!(value["real"], json!(["1", "2", "NaN", "-0"]));
        assert_eq!(value["imag"], json!(["0", "-3", "0", "0"]));
    }

    #[test]
    fn conformance_payload_formats_single_components_from_exact_binary32() {
        let summary = VariableSummary {
            name: CONFORMANCE_RESULT.to_owned(),
            class: "single".to_owned(),
            dimensions: vec![1, 2],
            complex: true,
            bytes: Some(16),
        };
        let preview = MatrixPreview {
            class: summary.class.clone(),
            dimensions: summary.dimensions.clone(),
            selected_range: MatrixRange {
                start: vec![1, 1],
                size: vec![1, 2],
            },
            values: vec![
                PreviewValue::Complex {
                    real: f64::from(0.1_f32),
                    imaginary: -2.0,
                },
                PreviewValue::Complex {
                    real: -0.0,
                    imaginary: f64::from(0.2_f32),
                },
            ],
            truncation: PreviewTruncation {
                truncated: false,
                omitted_elements: 0,
            },
        };
        let value = observation_value(&summary, preview, 2).expect("exact single payload");
        assert_eq!(value["kind"], "numeric");
        assert_eq!(value["class"], "single");
        assert_eq!(value["real"], json!(["0.100000001", "-0"]));
        assert_eq!(value["imag"], json!(["-2", "0.200000003"]));

        let malformed = MatrixPreview {
            class: summary.class.clone(),
            dimensions: vec![1, 1],
            selected_range: MatrixRange {
                start: vec![1, 1],
                size: vec![1, 1],
            },
            values: vec![PreviewValue::Number { value: 0.1 }],
            truncation: PreviewTruncation {
                truncated: false,
                omitted_elements: 0,
            },
        };
        let scalar_summary = VariableSummary {
            dimensions: vec![1, 1],
            complex: false,
            bytes: Some(4),
            ..summary
        };
        assert!(observation_value(&scalar_summary, malformed, 1).is_none());
    }

    #[test]
    fn schema_v2_exact_doubles_are_normalized_to_oracle_precision() {
        let observation = exact_observation_value_v2(kernel_v2::ExactValue {
            class: "double".to_owned(),
            size: vec![1, 2],
            ndims: 2,
            numel: 2,
            complex: true,
            payload: kernel_v2::ExactPayload::Numeric {
                real: vec!["0.2".to_owned(), "1e-5".to_owned()],
                imag: vec!["-0".to_owned(), "0".to_owned()],
            },
        })
        .unwrap();

        assert_eq!(
            observation["real"],
            json!(["0.20000000000000001", "1.0000000000000001e-05"])
        );
        assert_eq!(observation["imag"], json!(["-0", "0"]));
    }

    #[test]
    fn schema_v2_exact_tables_preserve_ordered_names_and_variable_values() {
        let column = |values: [&str; 2]| kernel_v2::ExactValue {
            class: "double".to_owned(),
            size: vec![2, 1],
            ndims: 2,
            numel: 2,
            complex: false,
            payload: kernel_v2::ExactPayload::Numeric {
                real: values.into_iter().map(str::to_owned).collect(),
                imag: vec!["0".to_owned(), "0".to_owned()],
            },
        };
        let observation = exact_observation_value_v2(kernel_v2::ExactValue {
            class: "table".to_owned(),
            size: vec![2, 2],
            ndims: 2,
            numel: 4,
            complex: false,
            payload: kernel_v2::ExactPayload::Table {
                variable_names: vec!["A".to_owned(), "B".to_owned()],
                variables: vec![column(["1", "2"]), column(["3", "4"])],
            },
        })
        .unwrap();

        assert_eq!(observation["kind"], "table");
        assert_eq!(observation["variableNames"], json!(["A", "B"]));
        assert_eq!(observation["variables"][0]["real"], json!(["1", "2"]));
        assert_eq!(observation["variables"][1]["real"], json!(["3", "4"]));
    }

    #[test]
    fn double_number_strings_match_matlab_seventeen_significant_digit_format() {
        let cases = [
            (0.0, "0"),
            (-0.0, "-0"),
            (0.1, "0.10000000000000001"),
            (0.2, "0.20000000000000001"),
            (1.0 / 3.0, "0.33333333333333331"),
            (1.0e-4, "0.0001"),
            (1.0e-5, "1.0000000000000001e-05"),
            (1.0e16, "10000000000000000"),
            (1.0e17, "1e+17"),
        ];

        for (value, expected) in cases {
            assert_eq!(number_string(value), expected, "value {value:?}");
        }
    }

    #[test]
    fn single_number_strings_match_matlab_nine_significant_digit_format() {
        let cases = [
            (0.0_f32, "0"),
            (-0.0_f32, "-0"),
            (0.1_f32, "0.100000001"),
            (1.0_f32 / 3.0_f32, "0.333333343"),
            (1.0e-3_f32, "0.00100000005"),
            (1.0e-4_f32, "9.99999975e-05"),
            (1.0e8_f32, "100000000"),
            (1.0e9_f32, "1e+09"),
            (1.0e10_f32, "1e+10"),
            (1.0e-10_f32, "1.00000001e-10"),
            (12.5_f32, "12.5"),
            (123_456_789.0_f32, "123456792"),
            (f32::MIN_POSITIVE, "1.17549435e-38"),
            (f32::MAX, "3.40282347e+38"),
        ];

        for (value, expected) in cases {
            assert_eq!(number_string_f32(value), expected, "value {value:?}");
        }
        assert_eq!(number_string(f64::from(0.1_f32)), "0.10000000149011612");
        assert_eq!(finite_number_string_for_class("single", 0.1), None);
    }

    #[test]
    fn conformance_payload_reports_dynamic_class_object_arrays_without_fake_elements() {
        let summary = VariableSummary {
            name: CONFORMANCE_RESULT.to_owned(),
            class: "OpenMatValueCounter".to_owned(),
            dimensions: vec![1, 2],
            complex: false,
            bytes: Some(16),
        };
        let preview = MatrixPreview {
            class: summary.class.clone(),
            dimensions: summary.dimensions.clone(),
            selected_range: MatrixRange {
                start: vec![1, 1],
                size: vec![1, 2],
            },
            values: vec![PreviewValue::Missing, PreviewValue::Missing],
            truncation: PreviewTruncation {
                truncated: false,
                omitted_elements: 0,
            },
        };

        let value = observation_value(&summary, preview, 2).expect("object metadata payload");
        assert_eq!(value["class"], "OpenMatValueCounter");
        assert_eq!(value["size"], json!([1, 2]));
        assert_eq!(value["ndims"], 2);
        assert_eq!(value["numel"], 2);
        assert_eq!(value["kind"], "object");
        assert!(value.get("real").is_none());
        assert!(value.get("logical").is_none());

        let malformed = MatrixPreview {
            class: summary.class.clone(),
            dimensions: summary.dimensions.clone(),
            selected_range: MatrixRange {
                start: vec![1, 1],
                size: vec![1, 2],
            },
            values: vec![PreviewValue::Missing, PreviewValue::Number { value: 1.0 }],
            truncation: PreviewTruncation {
                truncated: false,
                omitted_elements: 0,
            },
        };
        assert!(observation_value(&summary, malformed, 2).is_none());
    }
}
