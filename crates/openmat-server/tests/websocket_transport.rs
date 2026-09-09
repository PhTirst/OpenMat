use std::io::{BufRead, BufReader};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use openmat_protocol::kernel_v1::{self, PreviewValue as V1PreviewValue};
use openmat_protocol::{
    Capabilities, Event, EventEnvelope, ExecuteRequest, ExecutionMode, ImplementationInfo,
    InitializeRequest, InspectRequest, InterruptRequest, KernelStatus, ListWorkspaceRequest,
    MatrixRange, PROTOCOL_V0, PreviewValue, Request, RequestEnvelope, ResponseEnvelope,
    ResponseResult, ServerMessage, ShutdownRequest, StatusEvent,
};
use openmat_protocol::{kernel_v2, kernel_v3};
use tungstenite::client::IntoClientRequest;
use tungstenite::http::{HeaderValue, StatusCode};
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Error as WebSocketError, Message, WebSocket, connect};

type ClientSocket = WebSocket<MaybeTlsStream<TcpStream>>;

struct ServerProcess {
    child: Child,
    url: String,
}

impl ServerProcess {
    fn start() -> Self {
        Self::start_with_workspace(None)
    }

    fn start_with_workspace(workspace_root: Option<&Path>) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_openmat-server"));
        command.args(["--listen", "127.0.0.1:0"]);
        if let Some(root) = workspace_root {
            command.arg("--workspace-root").arg(root);
        }
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("start openmat-server");
        let stdout = child.stdout.take().expect("server stdout");
        let mut reader = BufReader::new(stdout);
        let mut url = String::new();
        let bytes = reader.read_line(&mut url).expect("read listener URL");
        assert!(bytes > 0, "server exited before announcing its URL");
        let url = url.trim().to_owned();
        assert!(url.starts_with("ws://127.0.0.1:"));
        assert!(url.ends_with("/kernel"));
        Self { child, url }
    }

    fn connect(&self) -> ClientSocket {
        let (mut socket, response) = connect(&self.url).expect("connect to kernel WebSocket");
        assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
        let MaybeTlsStream::Plain(stream) = socket.get_mut() else {
            panic!("loopback ws:// connection must be plain TCP");
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("set client read timeout");
        socket
    }

    fn connect_workspace(&self) -> ClientSocket {
        let url = self.url.replace("/kernel", "/workspace/v1");
        let (mut socket, response) = connect(url).expect("connect to workspace WebSocket");
        assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
        let MaybeTlsStream::Plain(stream) = socket.get_mut() else {
            panic!("loopback ws:// connection must be plain TCP");
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("set workspace client read timeout");
        socket
    }

    fn connect_workspace_v2(&self) -> ClientSocket {
        let url = self.url.replace("/kernel", "/workspace/v2");
        let (mut socket, response) = connect(url).expect("connect to workspace-v2 WebSocket");
        assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
        let MaybeTlsStream::Plain(stream) = socket.get_mut() else {
            panic!("loopback ws:// connection must be plain TCP");
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("set workspace-v2 client read timeout");
        socket
    }
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct TemporaryWorkspace(PathBuf);

impl TemporaryWorkspace {
    fn new() -> Self {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time after epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "openmat-server-workspace-integration-{}-{suffix}",
            std::process::id()
        ));
        std::fs::create_dir(&root).expect("create temporary workspace");
        Self(root)
    }
}

impl Drop for TemporaryWorkspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn request(session_id: &str, message_id: &str, request: Request) -> RequestEnvelope {
    RequestEnvelope::new(session_id, message_id, request)
}

fn initialize(session_id: &str) -> RequestEnvelope {
    request(
        session_id,
        "initialize",
        Request::Initialize(InitializeRequest {
            client: ImplementationInfo {
                name: "websocket-integration-test".to_owned(),
                version: "1".to_owned(),
            },
            supported_protocols: vec![PROTOCOL_V0.to_owned()],
            capabilities: Capabilities {
                execution_modes: vec![ExecutionMode::Repl],
                display_mime_types: vec!["text/plain".to_owned()],
                max_preview_elements: 32,
                interrupt: true,
                workspace_delta: true,
            },
        }),
    )
}

fn initialize_v1(session_id: &str) -> kernel_v1::BootstrapRequestEnvelope {
    kernel_v1::BootstrapRequestEnvelope::new(
        session_id,
        "initialize-v1",
        kernel_v1::InitializeRequest::v1(
            ImplementationInfo {
                name: "websocket-v1-integration-test".to_owned(),
                version: "1".to_owned(),
            },
            kernel_v1::Capabilities {
                execution_modes: vec![ExecutionMode::Repl],
                display_mime_types: vec!["text/plain".to_owned()],
                max_preview_elements: 32,
                max_string_element_code_units: Some(1024),
                max_preview_code_units: Some(4096),
                interrupt: true,
                workspace_delta: true,
            },
        ),
    )
}

fn negotiate_v1(socket: &mut ClientSocket, session_id: &str) -> kernel_v1::PreviewLimits {
    let request = initialize_v1(session_id);
    send_text(
        socket,
        kernel_v1::encode_bootstrap_request(&request).expect("encode v1 bootstrap"),
    );
    assert!(matches!(
        read_message(socket),
        ServerMessage::Event(EventEnvelope {
            event: Event::Status(StatusEvent {
                status: KernelStatus::Starting
            }),
            ..
        })
    ));
    let response = kernel_v1::decode_bootstrap_response(&read_text(socket))
        .expect("decode bootstrap response");
    let Some(kernel_v1::BootstrapResponseResult::Initialize(result)) = response.result else {
        panic!("v1 initialize result");
    };
    assert_eq!(result.negotiated_protocol, kernel_v1::PROTOCOL_V1);
    let limits = result
        .capabilities
        .preview_limits()
        .expect("negotiated limits");
    assert!(matches!(
        kernel_v1::decode_event(&read_text(socket))
            .expect("decode first v1 idle")
            .event,
        Event::Status(StatusEvent {
            status: KernelStatus::Idle
        })
    ));
    limits
}

fn negotiate_v3(socket: &mut ClientSocket, session_id: &str) -> kernel_v2::AggregateLimits {
    let capabilities = kernel_v2::Capabilities {
        execution_modes: vec![ExecutionMode::Repl],
        display_mime_types: vec!["text/plain".to_owned()],
        max_preview_elements: 32,
        max_string_element_code_units: Some(1024),
        max_preview_code_units: Some(4096),
        max_aggregate_nodes: Some(256),
        max_aggregate_elements: Some(1024),
        max_aggregate_depth: Some(8),
        interrupt: true,
        workspace_delta: true,
    };
    let request = kernel_v2::BootstrapRequestEnvelope::new(
        session_id,
        "initialize-v3",
        kernel_v2::InitializeRequest {
            client: ImplementationInfo {
                name: "websocket-v3-integration-test".to_owned(),
                version: "3".to_owned(),
            },
            supported_protocols: vec![kernel_v3::PROTOCOL_V3.to_owned()],
            capabilities: capabilities.clone(),
        },
    );
    send_text(
        socket,
        kernel_v2::encode_bootstrap_request(&request).expect("encode v3 bootstrap"),
    );
    assert!(matches!(
        read_message(socket),
        ServerMessage::Event(EventEnvelope {
            event: Event::Status(StatusEvent {
                status: KernelStatus::Starting
            }),
            ..
        })
    ));
    let response = kernel_v2::decode_bootstrap_response(&read_text(socket))
        .expect("decode v3 bootstrap response");
    let Some(kernel_v2::BootstrapResponseResult::Initialize(result)) = response.result else {
        panic!("v3 initialize result");
    };
    assert_eq!(result.negotiated_protocol, kernel_v3::PROTOCOL_V3);
    let limits = result
        .capabilities
        .aggregate_limits()
        .expect("negotiated v3 aggregate limits");
    assert!(matches!(
        kernel_v3::decode_event(&read_text(socket))
            .expect("decode first v3 idle")
            .event,
        Event::Status(StatusEvent {
            status: KernelStatus::Idle
        })
    ));
    limits
}

fn send(socket: &mut ClientSocket, request: &RequestEnvelope) {
    socket
        .send(Message::text(
            serde_json::to_string(request).expect("serialize request"),
        ))
        .expect("send request");
}

fn send_text(socket: &mut ClientSocket, text: String) {
    socket.send(Message::text(text)).expect("send text frame");
}

fn read_text(socket: &mut ClientSocket) -> String {
    loop {
        match socket.read().expect("read server message") {
            Message::Text(text) => return text.to_string(),
            Message::Ping(_) | Message::Pong(_) => {}
            message => panic!("expected text server message, got {message:?}"),
        }
    }
}

fn read_message(socket: &mut ClientSocket) -> ServerMessage {
    let text = read_text(socket);
    let message = serde_json::from_str::<ServerMessage>(&text).expect("decode server message");
    match &message {
        ServerMessage::Response(response) => response.validate().expect("response"),
        ServerMessage::Event(event) => event.validate().expect("event"),
    }
    message
}

fn exchange(socket: &mut ClientSocket, request: &RequestEnvelope) -> Vec<ServerMessage> {
    send(socket, request);
    let terminal_status = if matches!(request.request, Request::Shutdown(_)) {
        KernelStatus::Dead
    } else {
        KernelStatus::Idle
    };
    let mut messages = Vec::new();
    let mut saw_response = false;
    for _ in 0..64 {
        let message = read_message(socket);
        if matches!(
            &message,
            ServerMessage::Response(ResponseEnvelope { reply_to, .. })
                if reply_to == &request.message_id
        ) {
            saw_response = true;
        }
        let terminal = saw_response
            && matches!(
                &message,
                ServerMessage::Event(EventEnvelope {
                    event: Event::Status(StatusEvent { status }),
                    ..
                }) if *status == terminal_status
            );
        messages.push(message);
        if terminal {
            return messages;
        }
    }
    panic!("request did not reach its terminal status");
}

fn exchange_v1(
    socket: &mut ClientSocket,
    request: &kernel_v1::RequestEnvelope,
    limits: &kernel_v1::PreviewLimits,
) -> (kernel_v1::ResponseEnvelope, Vec<kernel_v1::EventEnvelope>) {
    send_text(
        socket,
        kernel_v1::encode_request(request, limits).expect("encode v1 request"),
    );
    let terminal_status = if matches!(request.request, kernel_v1::Request::Shutdown(_)) {
        KernelStatus::Dead
    } else {
        KernelStatus::Idle
    };
    let mut response = None;
    let mut events = Vec::new();
    for _ in 0..64 {
        let frame = read_text(socket);
        if let Ok(decoded) = kernel_v1::decode_response_for_request(&frame, request, limits) {
            response = Some(decoded);
        } else {
            let event = kernel_v1::decode_event(&frame).expect("v1 response or event");
            let terminal = response.is_some()
                && matches!(
                    event.event,
                    Event::Status(StatusEvent { status }) if status == terminal_status
                );
            events.push(event);
            if terminal {
                return (response.expect("correlated response"), events);
            }
        }
    }
    panic!("v1 request did not reach its terminal status");
}

fn exchange_v3(
    socket: &mut ClientSocket,
    request: &kernel_v3::RequestEnvelope,
    limits: &kernel_v2::AggregateLimits,
) -> (kernel_v3::ResponseEnvelope, Vec<kernel_v3::EventEnvelope>) {
    send_text(
        socket,
        kernel_v3::encode_request(request, limits).expect("encode v3 request"),
    );
    let terminal_status = if matches!(request.request, kernel_v3::Request::Shutdown(_)) {
        KernelStatus::Dead
    } else {
        KernelStatus::Idle
    };
    let mut response = None;
    let mut events = Vec::new();
    for _ in 0..64 {
        let frame = read_text(socket);
        if let Ok(decoded) = kernel_v3::decode_response_for_request(&frame, request, limits) {
            let failed = !decoded.ok;
            response = Some(decoded);
            if failed {
                return (response.expect("correlated failure"), events);
            }
        } else {
            let event = kernel_v3::decode_event(&frame).expect("v3 response or event");
            let terminal = response.is_some()
                && matches!(
                    event.event,
                    Event::Status(StatusEvent { status }) if status == terminal_status
                );
            events.push(event);
            if terminal {
                return (response.expect("correlated response"), events);
            }
        }
    }
    panic!("v3 request did not reach its terminal status");
}

fn response_for<'a>(messages: &'a [ServerMessage], reply_to: &str) -> &'a ResponseEnvelope {
    messages
        .iter()
        .find_map(|message| match message {
            ServerMessage::Response(response) if response.reply_to == reply_to => Some(response),
            ServerMessage::Response(_) | ServerMessage::Event(_) => None,
        })
        .expect("correlated response")
}

fn expect_close(socket: &mut ClientSocket, expected: CloseCode) {
    match socket.read().expect("read close frame") {
        Message::Close(Some(frame)) => assert_eq!(frame.code, expected),
        message => panic!("expected close frame, got {message:?}"),
    }
}

fn workspace_exchange(
    socket: &mut ClientSocket,
    request_id: &str,
    request: &serde_json::Value,
) -> serde_json::Value {
    socket
        .send(Message::text(
            serde_json::json!({
                "protocol": "openmat-workspace-v1",
                "requestId": request_id,
                "request": request,
            })
            .to_string(),
        ))
        .expect("send workspace request");
    serde_json::from_str(&read_text(socket)).expect("decode workspace response")
}

fn workspace_exchange_v2(
    socket: &mut ClientSocket,
    request_id: &str,
    request: &serde_json::Value,
) -> serde_json::Value {
    socket
        .send(Message::text(
            serde_json::json!({
                "protocol": "openmat-workspace-v2",
                "requestId": request_id,
                "request": request,
            })
            .to_string(),
        ))
        .expect("send workspace-v2 request");
    for _ in 0..16 {
        let response: serde_json::Value =
            serde_json::from_str(&read_text(socket)).expect("decode workspace-v2 frame");
        if response["requestId"] == request_id {
            return response;
        }
    }
    panic!("workspace-v2 request did not receive a correlated response");
}

fn browse_directory_names(socket: &mut ClientSocket, path: &Path) -> Vec<String> {
    let browsed = workspace_exchange_v2(
        socket,
        "browse-directories",
        &serde_json::json!({
            "type": "browseDirectories",
            "params": { "path": path.to_string_lossy() }
        }),
    );
    assert_eq!(browsed["ok"], true);
    browsed["result"]["data"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["name"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn real_server_workspace_v1_lists_and_creates_inside_the_explicit_root() {
    let workspace = TemporaryWorkspace::new();
    let server = ServerProcess::start_with_workspace(Some(&workspace.0));
    let mut socket = server.connect_workspace();

    let initial = workspace_exchange(
        &mut socket,
        "list-initial",
        &serde_json::json!({ "type": "list", "params": {} }),
    );
    assert_eq!(initial["protocol"], "openmat-workspace-v1");
    assert_eq!(initial["ok"], true);
    assert_eq!(initial["result"]["data"]["entries"], serde_json::json!([]));

    let file = workspace_exchange(
        &mut socket,
        "create-file",
        &serde_json::json!({
            "type": "create",
            "params": { "name": "analysis.m", "kind": "file" }
        }),
    );
    assert_eq!(file["ok"], true);
    assert!(workspace.0.join("analysis.m").is_file());

    let directory = workspace_exchange(
        &mut socket,
        "create-directory",
        &serde_json::json!({
            "type": "create",
            "params": { "name": "results", "kind": "directory" }
        }),
    );
    assert_eq!(directory["ok"], true);
    assert!(workspace.0.join("results").is_dir());

    let conflict = workspace_exchange(
        &mut socket,
        "create-conflict",
        &serde_json::json!({
            "type": "create",
            "params": { "name": "analysis.m", "kind": "file" }
        }),
    );
    assert_eq!(conflict["error"]["code"], "workspace.conflict");

    let escape_name = format!(
        "{}-escape.m",
        workspace
            .0
            .file_name()
            .expect("temporary workspace has a file name")
            .to_string_lossy()
    );
    let traversal_name = format!("../{escape_name}");
    for (index, (name, code)) in [
        ("..", "workspace.pathEscape"),
        (traversal_name.as_str(), "workspace.pathEscape"),
        ("C:\\outside.m", "workspace.pathEscape"),
        ("CON", "workspace.reservedWindowsName"),
    ]
    .into_iter()
    .enumerate()
    {
        let rejected = workspace_exchange(
            &mut socket,
            &format!("rejected-{index}"),
            &serde_json::json!({
                "type": "create",
                "params": { "name": name, "kind": "file" }
            }),
        );
        assert_eq!(rejected["error"]["code"], code);
    }
    assert!(!workspace.0.parent().unwrap().join(escape_name).exists());

    let refreshed = workspace_exchange(
        &mut socket,
        "list-refreshed",
        &serde_json::json!({ "type": "list", "params": {} }),
    );
    assert_eq!(
        refreshed["result"]["data"]["entries"],
        serde_json::json!([
            { "name": "results", "kind": "directory" },
            { "name": "analysis.m", "kind": "file" }
        ])
    );
}

#[test]
fn current_folder_is_shared_between_real_kernel_and_workspace_v2_connections() {
    let workspace = TemporaryWorkspace::new();
    let first = workspace.0.join("first");
    let second = workspace.0.join("second");
    std::fs::create_dir(&first).unwrap();
    std::fs::create_dir(&second).unwrap();
    std::fs::write(first.join("first-only.txt"), "first").unwrap();
    std::fs::write(second.join("second-only.txt"), "second").unwrap();
    let server = ServerProcess::start_with_workspace(Some(&first));
    let mut kernel = server.connect();
    let mut folders = server.connect_workspace_v2();
    let session_id = "current-folder-session";
    exchange(&mut kernel, &initialize(session_id));

    let current = workspace_exchange_v2(
        &mut folders,
        "current-initial",
        &serde_json::json!({ "type": "currentDirectory", "params": {} }),
    );
    assert_eq!(current["result"]["data"]["rootName"], "first");

    assert_eq!(
        browse_directory_names(&mut folders, &workspace.0),
        ["first", "second"]
    );
    let second_path = second.to_string_lossy().replace('\'', "''");
    let changed_by_kernel = request(
        session_id,
        "kernel-cd",
        Request::Execute(ExecuteRequest {
            code: format!("preserved = 41; cd('{second_path}');"),
            source_name: "Command Window".to_owned(),
            mode: ExecutionMode::Repl,
        }),
    );
    assert!(response_for(&exchange(&mut kernel, &changed_by_kernel), "kernel-cd").ok);

    let event: serde_json::Value =
        serde_json::from_str(&read_text(&mut folders)).expect("decode directory event");
    assert_eq!(event["event"]["type"], "currentDirectoryChanged");
    assert_eq!(event["event"]["data"]["rootName"], "second");
    let listed = workspace_exchange_v2(
        &mut folders,
        "list-second",
        &serde_json::json!({
            "type": "list", "params": { "path": "", "recursive": false }
        }),
    );
    assert_eq!(
        listed["result"]["data"]["entries"][0]["name"],
        "second-only.txt"
    );

    let changed_by_web = workspace_exchange_v2(
        &mut folders,
        "change-first",
        &serde_json::json!({
            "type": "changeDirectory",
            "params": { "path": first.to_string_lossy() }
        }),
    );
    assert_eq!(changed_by_web["ok"], true);
    let read_after_web_change = request(
        session_id,
        "kernel-read-after-web-change",
        Request::Execute(ExecuteRequest {
            code: "first_text = fileread('first-only.txt'); still_preserved = preserved;"
                .to_owned(),
            source_name: "Command Window".to_owned(),
            mode: ExecutionMode::Repl,
        }),
    );
    assert!(
        response_for(
            &exchange(&mut kernel, &read_after_web_change),
            "kernel-read-after-web-change"
        )
        .ok
    );
    let listed_kernel = request(
        session_id,
        "list-after-directory-switches",
        Request::ListWorkspace(ListWorkspaceRequest {}),
    );
    let messages = exchange(&mut kernel, &listed_kernel);
    let Some(ResponseResult::ListWorkspace(result)) =
        &response_for(&messages, "list-after-directory-switches").result
    else {
        panic!("listWorkspace result");
    };
    let names = result
        .variables
        .iter()
        .map(|variable| variable.name.as_str())
        .collect::<Vec<_>>();
    assert!(names.contains(&"preserved"));
    assert!(names.contains(&"still_preserved"));
    assert!(names.contains(&"first_text"));
}

#[test]
fn search_path_is_shared_between_workspace_v2_and_real_kernel_connections() {
    let workspace = TemporaryWorkspace::new();
    let toolbox = workspace.0.join("toolbox");
    std::fs::create_dir(&toolbox).unwrap();
    std::fs::write(
        toolbox.join("path_target.m"),
        "function value = path_target(input)\nvalue = input + 10;\nend\n",
    )
    .unwrap();
    let server = ServerProcess::start_with_workspace(Some(&workspace.0));
    let mut kernel = server.connect();
    let mut folders = server.connect_workspace_v2();
    let session_id = "search-path-session";
    exchange(&mut kernel, &initialize(session_id));

    let added = workspace_exchange_v2(
        &mut folders,
        "add-search-path",
        &serde_json::json!({
            "type": "addSearchPath",
            "params": { "path": "toolbox", "recursive": false, "position": "begin" }
        }),
    );
    assert_eq!(added["ok"], true);
    assert_eq!(
        added["result"]["data"]["directories"][0]["workspacePath"],
        "toolbox"
    );

    let execute_path_function = request(
        session_id,
        "execute-path-function",
        Request::Execute(ExecuteRequest {
            code: "answer = path_target(32);".to_owned(),
            source_name: "Command Window".to_owned(),
            mode: ExecutionMode::Repl,
        }),
    );
    assert!(
        response_for(
            &exchange(&mut kernel, &execute_path_function),
            "execute-path-function"
        )
        .ok
    );

    let inspect_answer = request(
        session_id,
        "inspect-path-answer",
        Request::Inspect(InspectRequest {
            name: "answer".to_owned(),
            range: MatrixRange {
                start: vec![1, 1],
                size: vec![1, 1],
            },
            max_elements: 1,
        }),
    );
    let inspected = exchange(&mut kernel, &inspect_answer);
    let Some(ResponseResult::Inspect(preview)) =
        &response_for(&inspected, "inspect-path-answer").result
    else {
        panic!("inspect result");
    };
    assert_eq!(preview.values, vec![PreviewValue::Number { value: 42.0 }]);
}

#[test]
fn real_tcp_websocket_runs_execute_list_inspect_and_shutdown() {
    let server = ServerProcess::start();
    let mut socket = server.connect();
    let session_id = "lifecycle-session";

    let initialized = exchange(&mut socket, &initialize(session_id));
    assert!(matches!(
        initialized.first(),
        Some(ServerMessage::Event(EventEnvelope {
            event: Event::Status(StatusEvent {
                status: KernelStatus::Starting
            }),
            ..
        }))
    ));
    let Some(ResponseResult::Initialize(result)) = &response_for(&initialized, "initialize").result
    else {
        panic!("initialize result");
    };
    assert!(result.capabilities.interrupt);

    let execute = request(
        session_id,
        "execute",
        Request::Execute(ExecuteRequest {
            code: "answer = 6 * 7;\n".to_owned(),
            source_name: "websocket-lifecycle.m".to_owned(),
            mode: ExecutionMode::Repl,
        }),
    );
    let executed = exchange(&mut socket, &execute);
    assert!(response_for(&executed, "execute").ok);

    let list = request(
        session_id,
        "list",
        Request::ListWorkspace(ListWorkspaceRequest {}),
    );
    let listed = exchange(&mut socket, &list);
    let Some(ResponseResult::ListWorkspace(workspace)) = &response_for(&listed, "list").result
    else {
        panic!("workspace result");
    };
    assert_eq!(workspace.variables[0].name, "answer");

    let inspect = request(
        session_id,
        "inspect",
        Request::Inspect(InspectRequest {
            name: "answer".to_owned(),
            range: MatrixRange {
                start: vec![1, 1],
                size: vec![1, 1],
            },
            max_elements: 1,
        }),
    );
    let inspected = exchange(&mut socket, &inspect);
    let Some(ResponseResult::Inspect(preview)) = &response_for(&inspected, "inspect").result else {
        panic!("inspect result");
    };
    assert_eq!(preview.values, vec![PreviewValue::Number { value: 42.0 }]);

    let shutdown = request(
        session_id,
        "shutdown",
        Request::Shutdown(ShutdownRequest {}),
    );
    let stopped = exchange(&mut socket, &shutdown);
    assert!(response_for(&stopped, "shutdown").ok);
    expect_close(&mut socket, CloseCode::Normal);
}

#[test]
fn real_tcp_websocket_v3_writes_one_element_and_rejects_a_stale_revision() {
    let server = ServerProcess::start();
    let mut socket = server.connect();
    let session_id = "v3-variable-editor-session";
    let limits = negotiate_v3(&mut socket, session_id);

    let execute = kernel_v3::RequestEnvelope::new(
        session_id,
        "execute-v3",
        kernel_v3::Request::Execute(ExecuteRequest {
            code: "A = [1 2; 3 4];\n".to_owned(),
            source_name: "websocket-v3-edit.m".to_owned(),
            mode: ExecutionMode::Repl,
        }),
    );
    let (executed, _) = exchange_v3(&mut socket, &execute, &limits);
    assert!(executed.ok, "execute failed: {:?}", executed.error);

    let inspect = kernel_v3::RequestEnvelope::new(
        session_id,
        "inspect-v3-before",
        kernel_v3::Request::Inspect(kernel_v2::InspectRequest {
            name: "A".to_owned(),
            range: kernel_v2::MatrixRange {
                start: vec![1, 1],
                size: vec![2, 2],
            },
            max_elements: 4,
        }),
    );
    let (inspected, _) = exchange_v3(&mut socket, &inspect, &limits);
    let Some(kernel_v3::ResponseResult::Inspect(before)) = inspected.result else {
        panic!("versioned inspect result")
    };
    assert_eq!(before.revision, 1);

    let write = kernel_v3::RequestEnvelope::new(
        session_id,
        "set-v3",
        kernel_v3::Request::SetVariableElement(kernel_v3::SetVariableElementRequest {
            name: "A".to_owned(),
            indices: vec![2, 1],
            value: kernel_v3::NumericScalar {
                real: "9".to_owned(),
                imaginary: "0".to_owned(),
            },
            expected_revision: before.revision,
        }),
    );
    let (written, _) = exchange_v3(&mut socket, &write, &limits);
    let Some(kernel_v3::ResponseResult::SetVariableElement(written)) = written.result else {
        panic!("element write result")
    };
    assert_eq!(written.revision, 2);

    let stale = kernel_v3::RequestEnvelope::new(
        session_id,
        "set-v3-stale",
        kernel_v3::Request::SetVariableElement(kernel_v3::SetVariableElementRequest {
            name: "A".to_owned(),
            indices: vec![1, 1],
            value: kernel_v3::NumericScalar {
                real: "100".to_owned(),
                imaginary: "0".to_owned(),
            },
            expected_revision: before.revision,
        }),
    );
    let (conflict, _) = exchange_v3(&mut socket, &stale, &limits);
    assert_eq!(
        conflict.error.expect("stale write error").category,
        "workspace.revisionConflict"
    );

    let inspect_after = kernel_v3::RequestEnvelope::new(
        session_id,
        "inspect-v3-after",
        kernel_v3::Request::Inspect(kernel_v2::InspectRequest {
            name: "A".to_owned(),
            range: kernel_v2::MatrixRange {
                start: vec![1, 1],
                size: vec![2, 2],
            },
            max_elements: 4,
        }),
    );
    let (inspected, _) = exchange_v3(&mut socket, &inspect_after, &limits);
    let Some(kernel_v3::ResponseResult::Inspect(after)) = inspected.result else {
        panic!("post-write inspect result")
    };
    assert_eq!(after.revision, 2);
    let kernel_v2::InspectPreview::Matrix(matrix) = after.preview else {
        panic!("matrix preview")
    };
    assert_eq!(
        matrix.values,
        vec![
            V1PreviewValue::Number { value: 1.0 },
            V1PreviewValue::Number { value: 9.0 },
            V1PreviewValue::Number { value: 2.0 },
            V1PreviewValue::Number { value: 4.0 },
        ]
    );
}

#[test]
#[allow(clippy::too_many_lines)] // This exercises one real process/socket v1 lifecycle.
fn real_tcp_websocket_v1_preserves_exact_runtime_values_and_rejects_downgrade() {
    let server = ServerProcess::start();
    let mut socket = server.connect();
    let session_id = "v1-exact-session";
    let limits = negotiate_v1(&mut socket, session_id);

    let execute = kernel_v1::RequestEnvelope::new(
        session_id,
        "execute-v1",
        kernel_v1::Request::Execute(ExecuteRequest {
            code: "wide = uint64(18446744073709551615); isolated = char(uint16(55357)); text = \"ok\";\n".to_owned(),
            source_name: "websocket-v1-exact.m".to_owned(),
            mode: ExecutionMode::Repl,
        }),
    );
    let (response, events) = exchange_v1(&mut socket, &execute, &limits);
    assert!(response.ok, "execute failed: {:?}", response.error);
    assert_eq!(
        events
            .iter()
            .filter(|event| {
                matches!(
                    event.event,
                    Event::Status(StatusEvent {
                        status: KernelStatus::Busy
                    })
                )
            })
            .count(),
        1,
        "transport and kernel busy events must be de-duplicated"
    );

    let list = kernel_v1::RequestEnvelope::new(
        session_id,
        "list-v1",
        kernel_v1::Request::ListWorkspace(ListWorkspaceRequest {}),
    );
    let (response, _) = exchange_v1(&mut socket, &list, &limits);
    let Some(kernel_v1::ResponseResult::ListWorkspace(workspace)) = response.result else {
        panic!("v1 workspace result");
    };
    assert_eq!(
        workspace
            .variables
            .iter()
            .map(|variable| variable.name.as_str())
            .collect::<Vec<_>>(),
        ["isolated", "text", "wide"]
    );

    let inspect = |message_id: &str, name: &str| {
        kernel_v1::RequestEnvelope::new(
            session_id,
            message_id,
            kernel_v1::Request::Inspect(kernel_v1::InspectRequest {
                name: name.to_owned(),
                range: kernel_v1::MatrixRange {
                    start: vec![1, 1],
                    size: vec![1, 1],
                },
                max_elements: 1,
            }),
        )
    };
    let wide = inspect("inspect-wide", "wide");
    let (response, _) = exchange_v1(&mut socket, &wide, &limits);
    let Some(kernel_v1::ResponseResult::Inspect(preview)) = response.result else {
        panic!("wide inspect result");
    };
    assert_eq!(preview.class, "uint64");
    assert_eq!(
        preview.values,
        [V1PreviewValue::Integer {
            real: u64::MAX.to_string(),
            imaginary: "0".to_owned(),
        }]
    );

    let isolated = inspect("inspect-isolated", "isolated");
    let (response, _) = exchange_v1(&mut socket, &isolated, &limits);
    let Some(kernel_v1::ResponseResult::Inspect(preview)) = response.result else {
        panic!("isolated char inspect result");
    };
    assert_eq!(preview.class, "char");
    assert_eq!(
        preview.values,
        [V1PreviewValue::CharCodeUnit { value: 0xD83D }]
    );

    let text = inspect("inspect-text", "text");
    let (response, _) = exchange_v1(&mut socket, &text, &limits);
    let Some(kernel_v1::ResponseResult::Inspect(preview)) = response.result else {
        panic!("string inspect result");
    };
    assert_eq!(preview.class, "string");
    assert_eq!(
        preview.values,
        [V1PreviewValue::String {
            code_units: vec![111, 107],
            missing: false,
        }]
    );

    let mixed = request(
        session_id,
        "mixed-v0",
        Request::ListWorkspace(ListWorkspaceRequest {}),
    );
    send(&mut socket, &mixed);
    expect_close(&mut socket, CloseCode::Policy);
}

#[test]
fn interrupt_uses_the_independent_control_path_during_execution() {
    let server = ServerProcess::start();
    let mut socket = server.connect();
    let session_id = "interrupt-session";
    exchange(&mut socket, &initialize(session_id));

    let execute = request(
        session_id,
        "execute-forever",
        Request::Execute(ExecuteRequest {
            code: "while 1\nend\n".to_owned(),
            source_name: "interrupt.m".to_owned(),
            mode: ExecutionMode::Repl,
        }),
    );
    send(&mut socket, &execute);
    loop {
        if matches!(
            read_message(&mut socket),
            ServerMessage::Event(EventEnvelope {
                event: Event::Status(StatusEvent {
                    status: KernelStatus::Busy
                }),
                ..
            })
        ) {
            break;
        }
    }

    let interrupt = request(
        session_id,
        "interrupt",
        Request::Interrupt(InterruptRequest {}),
    );
    send(&mut socket, &interrupt);
    let mut interrupt_accepted = false;
    let mut execute_interrupted = false;
    let mut idle = false;
    for _ in 0..32 {
        match read_message(&mut socket) {
            ServerMessage::Response(response) if response.reply_to == "interrupt" => {
                let Some(ResponseResult::Interrupt(result)) = response.result else {
                    panic!("interrupt result");
                };
                interrupt_accepted = result.accepted;
            }
            ServerMessage::Response(response) if response.reply_to == "execute-forever" => {
                let Some(ResponseResult::Execute(result)) = response.result else {
                    panic!("execute result");
                };
                execute_interrupted = result.interrupted;
            }
            ServerMessage::Event(EventEnvelope {
                event:
                    Event::Status(StatusEvent {
                        status: KernelStatus::Idle,
                    }),
                ..
            }) => idle = true,
            ServerMessage::Response(_) | ServerMessage::Event(_) => {}
        }
        if interrupt_accepted && execute_interrupted && idle {
            break;
        }
    }
    assert!(interrupt_accepted);
    assert!(execute_interrupted);
    assert!(idle);
}

#[test]
fn handshake_origin_path_and_unsafe_frames_are_rejected() {
    let server = ServerProcess::start();

    let mut localhost_request = server.url.as_str().into_client_request().unwrap();
    localhost_request
        .headers_mut()
        .insert("origin", HeaderValue::from_static("http://localhost:3000"));
    let (mut allowed, _) = connect(localhost_request).expect("localhost Origin is allowed");
    allowed.close(None).unwrap();

    let mut tauri_request = server.url.as_str().into_client_request().unwrap();
    tauri_request
        .headers_mut()
        .insert("origin", HeaderValue::from_static("http://tauri.localhost"));
    let (mut allowed, _) = connect(tauri_request).expect("Tauri Origin is allowed");
    allowed.close(None).unwrap();

    let mut foreign_request = server.url.as_str().into_client_request().unwrap();
    foreign_request.headers_mut().insert(
        "origin",
        HeaderValue::from_static("https://example.invalid"),
    );
    match connect(foreign_request) {
        Err(WebSocketError::Http(response)) => {
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
        }
        _ => panic!("foreign Origin must fail with HTTP 403"),
    }

    let wrong_url = server.url.replace("/kernel", "/wrong");
    match connect(wrong_url) {
        Err(WebSocketError::Http(response)) => {
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        }
        _ => panic!("wrong endpoint must fail with HTTP 404"),
    }

    let mut binary = server.connect();
    binary
        .send(Message::binary(vec![1_u8, 2, 3]))
        .expect("send binary frame");
    expect_close(&mut binary, CloseCode::Unsupported);

    let mut invalid_json = server.connect();
    invalid_json
        .send(Message::text("not-json"))
        .expect("send invalid JSON");
    expect_close(&mut invalid_json, CloseCode::Invalid);

    let mut invalid_envelope = server.connect();
    send(&mut invalid_envelope, &initialize(""));
    expect_close(&mut invalid_envelope, CloseCode::Policy);

    let mut oversized = server.connect();
    oversized
        .send(Message::text("x".repeat(1024 * 1024 + 1)))
        .expect("send oversized frame");
    expect_close(&mut oversized, CloseCode::Size);
}

#[test]
fn bootstrap_is_mandatory_and_v0_rejects_repeat_initialize_and_v1_mixing() {
    let server = ServerProcess::start();

    let mut uninitialized = server.connect();
    send(
        &mut uninitialized,
        &request(
            "uninitialized",
            "list-too-early",
            Request::ListWorkspace(ListWorkspaceRequest {}),
        ),
    );
    expect_close(&mut uninitialized, CloseCode::Invalid);

    let mut socket = server.connect();
    let session_id = "locked-v0-session";
    exchange(&mut socket, &initialize(session_id));
    send(&mut socket, &initialize(session_id));
    let ServerMessage::Response(response) = read_message(&mut socket) else {
        panic!("repeat initialize response");
    };
    assert!(!response.ok);
    assert_eq!(
        response.error.expect("repeat initialize error").category,
        "kernel.alreadyInitialized"
    );

    let mixed = kernel_v1::RequestEnvelope::new(
        session_id,
        "mixed-v1",
        kernel_v1::Request::ListWorkspace(ListWorkspaceRequest {}),
    );
    send_text(
        &mut socket,
        serde_json::to_string(&mixed).expect("serialize mixed v1 request"),
    );
    expect_close(&mut socket, CloseCode::Policy);
}

#[test]
fn invalid_v1_capability_offer_is_structured_and_retryable() {
    let server = ServerProcess::start();
    let mut socket = server.connect();
    let session_id = "capability-retry-session";
    let mut invalid = initialize_v1(session_id);
    invalid.message_id = "invalid-initialize".to_owned();
    let kernel_v1::BootstrapRequest::Initialize(parameters) = &mut invalid.request;
    parameters.capabilities.max_string_element_code_units = None;
    parameters.capabilities.max_preview_code_units = None;
    send_text(&mut socket, serde_json::to_string(&invalid).unwrap());
    assert!(matches!(
        read_message(&mut socket),
        ServerMessage::Event(EventEnvelope {
            event: Event::Status(StatusEvent {
                status: KernelStatus::Starting
            }),
            ..
        })
    ));
    let failure = kernel_v1::decode_bootstrap_response(&read_text(&mut socket)).unwrap();
    assert_eq!(failure.reply_to, "invalid-initialize");
    assert_eq!(
        failure.error.expect("invalid capability error").category,
        "protocol.invalidCapabilities"
    );

    let retry = initialize_v1(session_id);
    send_text(
        &mut socket,
        kernel_v1::encode_bootstrap_request(&retry).unwrap(),
    );
    let response = kernel_v1::decode_bootstrap_response(&read_text(&mut socket)).unwrap();
    let Some(kernel_v1::BootstrapResponseResult::Initialize(result)) = response.result else {
        panic!("retry initialize result");
    };
    assert_eq!(result.negotiated_protocol, kernel_v1::PROTOCOL_V1);
    let limits = result.capabilities.preview_limits().unwrap();
    assert!(matches!(
        kernel_v1::decode_event(&read_text(&mut socket))
            .unwrap()
            .event,
        Event::Status(StatusEvent {
            status: KernelStatus::Idle
        })
    ));
    let shutdown = kernel_v1::RequestEnvelope::new(
        session_id,
        "retry-shutdown",
        kernel_v1::Request::Shutdown(ShutdownRequest {}),
    );
    assert!(exchange_v1(&mut socket, &shutdown, &limits).0.ok);
    expect_close(&mut socket, CloseCode::Normal);
}

#[test]
fn session_is_pinned_by_first_valid_request_and_non_loopback_bind_is_refused() {
    let server = ServerProcess::start();
    let mut socket = server.connect();
    exchange(&mut socket, &initialize("pinned-session"));
    send(
        &mut socket,
        &request(
            "other-session",
            "mismatch",
            Request::ListWorkspace(ListWorkspaceRequest {}),
        ),
    );
    expect_close(&mut socket, CloseCode::Policy);

    let output = Command::new(env!("CARGO_BIN_EXE_openmat-server"))
        .args(["--listen", "0.0.0.0:0"])
        .output()
        .expect("run non-loopback bind attempt");
    assert_eq!(output.status.code(), Some(64));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .expect("UTF-8 stderr")
            .contains("must be loopback")
    );
}
