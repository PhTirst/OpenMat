use std::io::{BufRead, BufReader};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use serde_json::{Value, json};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket, connect};

type Socket = WebSocket<MaybeTlsStream<TcpStream>>;
struct Server {
    child: Child,
    url: String,
}
impl Server {
    fn start() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_openmat-server"))
            .args(["--listen", "127.0.0.1:0"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let mut url = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut url)
            .unwrap();
        Self {
            child,
            url: url.trim().replace("/kernel", "/simulation/v1"),
        }
    }
    fn connect(&self) -> Socket {
        let (mut socket, _) = connect(&self.url).unwrap();
        if let MaybeTlsStream::Plain(stream) = socket.get_mut() {
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
        }
        socket
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn request(socket: &mut Socket, id: &str, operation: &str, data: Value) {
    let mut message =
        json!({"protocol":"openmat-simulation-v1","requestId":id,"operation":operation});
    let Value::Object(data) = data else {
        panic!("request data must be an object")
    };
    message.as_object_mut().unwrap().extend(data);
    socket.send(Message::text(message.to_string())).unwrap();
}
fn read(socket: &mut Socket) -> Value {
    loop {
        if let Message::Text(text) = socket.read().unwrap() {
            return serde_json::from_str(&text).unwrap();
        }
    }
}
fn model() -> Value {
    serde_json::from_str(include_str!(
        "../../../simulation/examples/first-order.omsim.json"
    ))
    .unwrap()
}

fn request_v2(socket: &mut Socket, id: &str, operation: &str, data: Value) {
    let mut message =
        json!({"protocol":"openmat-simulation-v2","requestId":id,"operation":operation});
    let Value::Object(data) = data else {
        panic!("request data must be an object")
    };
    message.as_object_mut().unwrap().extend(data);
    socket.send(Message::text(message.to_string())).unwrap();
}

fn request_v3(socket: &mut Socket, id: &str, operation: &str, data: Value) {
    let mut message =
        json!({"protocol":"openmat-simulation-v3","requestId":id,"operation":operation});
    let Value::Object(data) = data else {
        panic!("request data must be an object")
    };
    message.as_object_mut().unwrap().extend(data);
    socket.send(Message::text(message.to_string())).unwrap();
}

#[test]
fn v3_runs_stateful_source_snapshots_and_rejects_them_on_v2() {
    let mut server = Server::start();
    server.url = server.url.replace("/simulation/v1", "/simulation/v2");
    let mut legacy = server.connect();
    server.url = server.url.replace("/simulation/v2", "/simulation/v3");
    let mut socket = server.connect();
    request_v3(&mut socket, "catalog", "catalog", json!({}));
    let catalog = read(&mut socket);
    assert_eq!(catalog["protocol"], "openmat-simulation-v3");
    assert_eq!(catalog["result"]["blocks"].as_array().unwrap().len(), 8);
    let model: Value = serde_json::from_str(include_str!(
        "../../../simulation/examples/custom-delay.omsim.json"
    ))
    .unwrap();
    let sources = json!({
        "components/delay/om_delay_initialize.m":include_str!("../../../simulation/examples/components/delay/om_delay_initialize.m"),
        "components/delay/om_delay_outputs.m":include_str!("../../../simulation/examples/components/delay/om_delay_outputs.m"),
        "components/delay/om_delay_update.m":include_str!("../../../simulation/examples/components/delay/om_delay_update.m")
    });
    request_v2(
        &mut legacy,
        "reject",
        "check",
        json!({"model":model,"sources":sources,"revision":"older"}),
    );
    assert_eq!(read(&mut legacy)["error"]["code"], "protocol");
    let mut broken = sources.clone();
    broken["components/delay/om_delay_update.m"] =
        json!("function z = om_delay_update(t,x,q,u,p)\nz=eval('u');\nend");
    request_v3(
        &mut socket,
        "bad-callback",
        "check",
        json!({"model":model,"sources":broken,"revision":"bad"}),
    );
    let error = read(&mut socket);
    assert_eq!(
        error["error"]["sourcePath"],
        "components/delay/om_delay_update.m"
    );
    assert_eq!(error["error"]["line"], 2);
    request_v3(
        &mut socket,
        "stateful",
        "run",
        json!({"model":model,"sources":sources,"revision":"frozen-stateful"}),
    );
    let ack = read(&mut socket);
    assert_eq!(ack["ok"], true, "{ack}");
    assert_eq!(ack["result"]["backend"], "reference");
    let mut samples = false;
    loop {
        let event = read(&mut socket);
        assert_eq!(event["protocol"], "openmat-simulation-v3");
        assert_eq!(event["revision"], "frozen-stateful");
        if event["event"] == "finished" {
            assert_eq!(
                event["data"]["time"].as_f64(),
                model["settings"]["stopTime"].as_f64()
            );
            break;
        }
        assert_eq!(event["event"], "samples", "{event}");
        samples = true;
    }
    assert!(samples);
}

#[test]
fn v2_compiles_source_snapshots_and_preserves_the_legacy_endpoint() {
    let mut server = Server::start();
    let mut legacy = server.connect();
    server.url = server.url.replace("/simulation/v1", "/simulation/v2");
    let mut socket = server.connect();
    request_v2(&mut socket, "catalog-v2", "catalog", json!({}));
    let catalog = read(&mut socket);
    assert_eq!(catalog["protocol"], "openmat-simulation-v2");
    assert_eq!(catalog["result"]["blocks"].as_array().unwrap().len(), 7);
    let model: Value = serde_json::from_str(include_str!(
        "../../../simulation/examples/pendulum.omsim.json"
    ))
    .unwrap();
    request(
        &mut legacy,
        "reject",
        "check",
        json!({"model":model,"revision":"legacy"}),
    );
    assert_eq!(read(&mut legacy)["error"]["code"], "protocol");
    request_v2(
        &mut socket,
        "bad-source",
        "check",
        json!({"model":model,"revision":"bad","sources":{"pendulum.m":"function dx = pendulum(x, p)\ndx = eval('x');\nend"}}),
    );
    let diagnostic = read(&mut socket);
    assert_eq!(diagnostic["error"]["sourcePath"], "pendulum.m");
    assert_eq!(diagnostic["error"]["line"], 2);
    request_v2(
        &mut socket,
        "source-run",
        "run",
        json!({"model":model,"revision":"frozen","sources":{"pendulum.m":include_str!("../../../simulation/examples/pendulum.m")}}),
    );
    assert_eq!(read(&mut socket)["result"]["backend"], "reference");
    loop {
        let event = read(&mut socket);
        assert_eq!(event["protocol"], "openmat-simulation-v2");
        assert_eq!(event["revision"], "frozen");
        if event["event"] == "finished" {
            assert_eq!(event["data"]["time"], 10.0);
            break;
        }
        assert_eq!(event["event"], "samples");
    }
}

#[test]
#[ignore = "requires configured LLVM and SUNDIALS runtimes"]
fn v2_native_solver_selection_acknowledges_actual_execution_and_statistics() {
    assert!(std::env::var_os("OPENMAT_SIM_LLVM_LIBRARY").is_some());
    assert!(std::env::var_os("OPENMAT_SIM_SUNDIALS_DIRECTORY").is_some());
    let mut server = Server::start();
    server.url = server.url.replace("/simulation/v1", "/simulation/v2");
    let mut socket = server.connect();
    let model: Value = serde_json::from_str(include_str!(
        "../../../simulation/examples/pendulum.omsim.json"
    ))
    .unwrap();
    request_v2(
        &mut socket,
        "native",
        "run",
        json!({"model":model,"revision":"native","sources":{"pendulum.m":include_str!("../../../simulation/examples/pendulum.m")},"execution":{"backend":"llvm","solver":{"type":"cvode","method":"bdf","relativeTolerance":1e-7,"absoluteTolerance":1e-10}}}),
    );
    let ack = read(&mut socket);
    assert_eq!(ack["ok"], true, "{ack}");
    assert_eq!(ack["result"]["backend"], "llvm-orc");
    assert_eq!(ack["result"]["solver"]["sundialsVersion"], "7.5.0");
    loop {
        let event = read(&mut socket);
        if event["event"] == "finished" {
            assert!(
                event["data"]["solverStats"]["rhsEvaluations"]
                    .as_u64()
                    .unwrap()
                    > 0
            );
            break;
        }
        assert_eq!(event["event"], "samples", "{event}");
    }
}

#[test]
fn same_listener_serves_catalog_check_run_and_invalid_model_diagnostics() {
    let server = Server::start();
    let mut socket = server.connect();
    request(&mut socket, "catalog", "catalog", json!({}));
    assert_eq!(
        read(&mut socket)["result"]["blocks"]
            .as_array()
            .unwrap()
            .len(),
        6
    );
    let mut invalid = model();
    invalid["connections"][0]["to"]["port"] = json!("missing");
    request(
        &mut socket,
        "bad",
        "check",
        json!({"model":invalid,"revision":"r1"}),
    );
    let rejected = read(&mut socket);
    assert_eq!(rejected["ok"], false);
    assert!(rejected["error"]["block"].is_string());
    request(
        &mut socket,
        "check",
        "check",
        json!({"model":model(),"revision":"r2"}),
    );
    assert_eq!(read(&mut socket)["result"]["revision"], "r2");
    request(
        &mut socket,
        "run",
        "run",
        json!({"model":model(),"revision":"r2"}),
    );
    assert_eq!(read(&mut socket)["result"]["runId"], "run");
    let mut previous = 0;
    loop {
        let event = read(&mut socket);
        assert_eq!(event["revision"], "r2");
        let sequence = event["sequence"].as_u64().unwrap();
        assert!(sequence > previous);
        previous = sequence;
        if event["event"] == "finished" {
            assert_eq!(event["data"]["time"], 1.0);
            break;
        }
        assert_eq!(event["event"], "samples");
    }
    socket.close(None).unwrap();
}

#[test]
fn busy_and_cancellation_are_scoped_to_the_owning_connection() {
    let server = Server::start();
    let mut owner = server.connect();
    let mut other = server.connect();
    let mut long = model();
    long["settings"]["stopTime"] = json!(1e6);
    request(
        &mut owner,
        "long",
        "run",
        json!({"model":long,"revision":"owned"}),
    );
    request(
        &mut owner,
        "second",
        "run",
        json!({"model":model(),"revision":"other"}),
    );
    request(&mut other, "foreign", "cancel", json!({"runId":"long"}));
    assert_eq!(read(&mut other)["error"]["code"], "run_not_found");
    request(&mut owner, "cancel", "cancel", json!({"runId":"long"}));
    let mut busy = false;
    let mut cancellation_ack = false;
    let mut terminal = false;
    for _ in 0..100 {
        let event = read(&mut owner);
        if event["requestId"] == "second" {
            assert_eq!(event["error"]["code"], "busy");
            busy = true;
        }
        if event["requestId"] == "cancel" {
            assert_eq!(event["ok"], true);
            cancellation_ack = true;
        }
        if event["event"] == "cancelled" {
            terminal = true;
        }
        if busy && cancellation_ack && terminal {
            break;
        }
    }
    assert!(busy && cancellation_ack && terminal);
    request(
        &mut owner,
        "again",
        "check",
        json!({"model":model(),"revision":"new"}),
    );
    assert_eq!(read(&mut owner)["result"]["valid"], true);
    owner.close(None).unwrap();
    other.close(None).unwrap();
}
