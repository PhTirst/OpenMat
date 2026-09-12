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
