//! Versioned, connection-owned simulation jobs. Numerical work never blocks reads.
use std::io;
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::simulation_execution::{self, Execution};
use openmat_sim::model::Model;
use openmat_sim::numeric::Kernel;
use openmat_sim::{CompiledModel, Runner, SourceBundle, compile_with_sources};
use openmat_sim_slx::ImportedSlx;
use serde::Deserialize;
use serde_json::{Value, json};
use tungstenite::Error as WsError;
use tungstenite::protocol::{Message, WebSocket};

use crate::ServerError;

const PROTOCOL: &str = "openmat-simulation-v1";
const PROTOCOL_V2: &str = "openmat-simulation-v2";
const PROTOCOL_V3: &str = "openmat-simulation-v3";
const MAX_MESSAGE: usize = 8 * 1024 * 1024;
const MAX_SLX: usize = 2 * 1024 * 1024;
const MAX_SAMPLES: usize = 100_000;
const MAX_VALUES: usize = 2_000_000;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Request {
    protocol: String,
    #[serde(rename = "requestId")]
    id: String,
    #[serde(flatten)]
    operation: Operation,
}

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "camelCase", deny_unknown_fields)]
enum Operation {
    Catalog,
    Check {
        model: Model,
        revision: String,
        #[serde(default)]
        sources: Option<SourceBundle>,
        #[serde(default)]
        execution: Option<Execution>,
    },
    Run {
        model: Model,
        revision: String,
        #[serde(default)]
        sources: Option<SourceBundle>,
        #[serde(default)]
        execution: Option<Execution>,
    },
    #[serde(rename_all = "camelCase")]
    Cancel {
        run_id: String,
    },
    ImportSlx {
        name: String,
        bytes: Vec<u8>,
    },
}

struct Snapshot {
    model: Model,
    sources: SourceBundle,
    execution: Execution,
}

struct Job {
    id: String,
    revision: String,
    cancel: Arc<AtomicBool>,
    events: Option<Receiver<Value>>,
    worker: Option<JoinHandle<()>>,
    terminal: bool,
    acknowledged: bool,
    sequence: u64,
}

impl Job {
    fn observe(&mut self, value: &Value) {
        self.acknowledged |= value.get("ok") == Some(&Value::Bool(true));
        self.sequence = value
            .get("sequence")
            .and_then(Value::as_u64)
            .unwrap_or(self.sequence);
        self.terminal |= value.get("ok") == Some(&Value::Bool(false))
            || matches!(
                value.get("event").and_then(Value::as_str),
                Some("finished" | "failed" | "cancelled")
            );
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        // Disconnect before joining: a worker may be waiting on the bounded queue.
        self.events.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn response(id: &str, result: Value) -> Value {
    let mut value = json!({"protocol": PROTOCOL, "requestId": id, "ok": true});
    value["result"] = result;
    value
}

fn failure(id: &str, error: Value) -> Value {
    let mut value = json!({"protocol": PROTOCOL, "requestId": id, "ok": false});
    value["error"] = error;
    value
}

fn error(code: &str, message: &str) -> Value {
    json!({"code": code, "message": message})
}

fn summary(plan: &CompiledModel) -> Value {
    json!({"scopes": plan.scopes(), "settings": plan.settings(),
        "continuousStates": plan.continuous_state_count(),
        "discreteStates": plan.discrete_state_count(),
        "executionOrder": plan.execution_order(), "backend": "reference"})
}

fn catalog() -> Value {
    json!({"schemaVersion":1,"backend":"reference", "maxSamples":MAX_SAMPLES,
    "maxValues":MAX_VALUES, "maxSlxBytes":MAX_SLX,
    "blocks":[
        {"type":"constant","label":"Constant","category":"Sources","icon":"constant","inputs":[],"outputs":["out"],"parameter":"value","default":[1]},
        {"type":"sum","label":"Sum","category":"Math","icon":"sum","inputs":["in0","in1"],"outputs":["out"],"parameter":"signs","default":[1,-1]},
        {"type":"gain","label":"Gain","category":"Math","icon":"gain","inputs":["in"],"outputs":["out"],"parameter":"gain","default":[1]},
        {"type":"integrator","label":"Integrator","category":"Continuous","icon":"integrator","inputs":["in"],"outputs":["out"],"parameter":"initial","default":[0]},
        {"type":"unitDelay","label":"Unit Delay","category":"Discrete","icon":"unitDelay","inputs":["in"],"outputs":["out"],"parameter":"initial","default":[0]},
        {"type":"scope","label":"Scope","category":"Sinks","icon":"scope","inputs":["in"],"outputs":[]}
    ]})
}

fn enqueue(
    sender: &SyncSender<Value>,
    cancel: &AtomicBool,
    mut value: Value,
    terminal: bool,
) -> bool {
    loop {
        if !terminal && cancel.load(Ordering::Relaxed) {
            return false;
        }
        match sender.try_send(value) {
            Ok(()) => return true,
            Err(TrySendError::Disconnected(_)) => return false,
            Err(TrySendError::Full(pending)) => {
                value = pending;
                thread::sleep(Duration::from_millis(2));
            }
        }
    }
}

#[cfg(test)]
fn start_job(id: String, revision: String, model: Model) -> io::Result<Job> {
    start_snapshot(
        id,
        revision,
        Snapshot {
            model,
            sources: SourceBundle::new(),
            execution: Execution::default(),
        },
    )
}

fn start_snapshot(id: String, revision: String, snapshot: Snapshot) -> io::Result<Job> {
    let cancel = Arc::new(AtomicBool::new(false));
    let (sender, events) = mpsc::sync_channel(8);
    let run_id = id.clone();
    let run_revision = revision.clone();
    let cancellation = Arc::clone(&cancel);
    let worker = thread::Builder::new()
        .name("openmat-simulation".into())
        .spawn(move || {
            run_job(&run_id, &run_revision, &snapshot, &sender, &cancellation);
        })?;
    Ok(Job {
        id,
        revision,
        cancel,
        events: Some(events),
        worker: Some(worker),
        terminal: false,
        acknowledged: false,
        sequence: 0,
    })
}

fn prepare_run(
    id: &str,
    revision: &str,
    snapshot: &Snapshot,
    sender: &SyncSender<Value>,
    cancel: &AtomicBool,
) -> Option<Runner<Box<dyn Kernel>>> {
    let started = Instant::now();
    let plan = match compile_with_sources(&snapshot.model, &snapshot.sources) {
        Ok(plan) => plan,
        Err(err) => {
            enqueue(sender, cancel, failure(id, json!(err.0)), true);
            return None;
        }
    };
    let mut result = summary(&plan);
    let execution = snapshot.execution.summary();
    result["backend"] = execution["backend"].clone();
    result["solver"] = execution["solver"].clone();
    let runner = match snapshot.execution.runner(plan) {
        Ok(runner) => runner,
        Err(err) => {
            enqueue(sender, cancel, failure(id, json!(err)), true);
            return None;
        }
    };
    result["runId"] = json!(id);
    result["revision"] = json!(revision);
    result["compileSeconds"] = json!(started.elapsed().as_secs_f64());
    // A run always gets its response even if cancellation arrived during compilation.
    if !enqueue(sender, cancel, response(id, result), true) {
        return None;
    }
    Some(runner)
}

#[allow(clippy::too_many_lines)] // Keep streaming limits, cancellation and the single terminal event together.
fn run_job(
    id: &str,
    revision: &str,
    snapshot: &Snapshot,
    sender: &SyncSender<Value>,
    cancel: &AtomicBool,
) {
    let started = Instant::now();
    let Some(mut runner) = prepare_run(id, revision, snapshot, sender, cancel) else {
        return;
    };
    let mut sequence = 0_u64;
    let mut event = |name: &str, data: Value| {
        sequence += 1;
        json!({"protocol":PROTOCOL,"event":name,"runId":id,"revision":revision,
            "sequence":sequence,"data":data})
    };
    let mut frames = vec![runner.current_frame()];
    let mut samples = 1_usize;
    let frame_width = frames[0].values.len();
    let mut values = frame_width;
    let mut batch_values = values;
    let mut sent = Instant::now();
    let mut terminal_error = None;
    if batch_values >= 8192 {
        if !enqueue(
            sender,
            cancel,
            event("samples", json!({"frames":frames})),
            false,
        ) {
            enqueue(
                sender,
                cancel,
                event("cancelled", json!({"time":runner.time(),"samples":samples})),
                true,
            );
            return;
        }
        frames.clear();
        batch_values = 0;
    }
    while !runner.is_finished() && !cancel.load(Ordering::Relaxed) {
        if samples >= MAX_SAMPLES || values.saturating_add(frame_width) > MAX_VALUES {
            terminal_error = Some(error(
                "result_limit",
                "Simulation result exceeds the interactive sample/value limit. Increase max step or shorten stop time.",
            ));
            break;
        }
        match runner.advance_with_cancel(cancel) {
            Ok(Some(frame)) => {
                samples += 1;
                values += frame.values.len();
                batch_values += frame.values.len();
                frames.push(frame);
            }
            Ok(None) => break,
            Err(err) => {
                terminal_error = Some(json!(err));
                break;
            }
        }
        if frames.len() >= 64 || batch_values >= 8192 || sent.elapsed() >= Duration::from_millis(30)
        {
            if !enqueue(
                sender,
                cancel,
                event("samples", json!({"frames":frames})),
                false,
            ) {
                break;
            }
            frames.clear();
            batch_values = 0;
            sent = Instant::now();
        }
    }
    if !frames.is_empty() && !cancel.load(Ordering::Relaxed) {
        enqueue(
            sender,
            cancel,
            event("samples", json!({"frames":frames})),
            false,
        );
    }
    let name = if cancel.load(Ordering::Relaxed) {
        "cancelled"
    } else if terminal_error.is_some() {
        "failed"
    } else {
        "finished"
    };
    enqueue(
        sender,
        cancel,
        event(
            name,
            json!({"time":runner.time(),"samples":samples,
        "elapsedSeconds":started.elapsed().as_secs_f64(),"solverStats":runner.solver_statistics(),"error":terminal_error}),
        ),
        true,
    );
}

fn import_slx(name: &str, bytes: &[u8]) -> Result<Value, Value> {
    if bytes.len() > MAX_SLX || name.len() > 1024 {
        return Err(error(
            "slx_limit",
            "Interactive SLX import is limited to 2 MiB.",
        ));
    }
    let imported = ImportedSlx::read(bytes, name).map_err(|issue| json!(issue))?;
    Ok(match imported.lower() {
        Ok(model) => {
            json!({"runnable":true,"model":model,"document":imported.document(),"issues":[]})
        }
        Err(issues) => json!({"runnable":false,"document":imported.document(),"issues":issues}),
    })
}

#[allow(clippy::too_many_lines)] // Keep both protocol versions and connection-owned job dispatch together.
fn handle(request: Request, job: &mut Option<Job>) -> Option<Value> {
    let id = &request.id;
    let v3 = request.protocol == PROTOCOL_V3;
    let v2 = request.protocol == PROTOCOL_V2 || v3;
    if (request.protocol != PROTOCOL && !v2) || id.is_empty() || id.len() > 128 {
        return Some(failure(
            if id.len() <= 128 { id } else { "" },
            error(
                "protocol",
                "Expected openmat-simulation-v1 and a requestId of 1..128 bytes.",
            ),
        ));
    }
    Some(match request.operation {
        Operation::Catalog => {
            let mut result = catalog();
            if v2 {
                result["schemaVersion"] = json!(2);
                result["execution"] = simulation_execution::capabilities();
                result["blocks"].as_array_mut().expect("catalog array").push(json!({"type":"mFunction","label":"M Function","category":"Functions","icon":"mFunction","inputs":["u"],"outputs":["out"]}));
            }
            if v3 {
                result["schemaVersion"] = json!(3);
                result["blocks"].as_array_mut().expect("catalog array").push(json!({"type":"component","label":"Component","category":"Components","icon":"component","inputs":[],"outputs":[]}));
            }
            response(id, result)
        }
        Operation::Check {
            model,
            revision,
            sources,
            execution,
        } => {
            if !v3 && model.schema_version >= 3 {
                return Some(failure(
                    id,
                    error("protocol", "stateful components require /simulation/v3"),
                ));
            }
            if !v2 && (sources.is_some() || execution.is_some() || model.schema_version != 1) {
                return Some(failure(
                    id,
                    error(
                        "protocol",
                        "source snapshots and execution options require /simulation/v2",
                    ),
                ));
            }
            if revision.len() > 128 {
                return Some(failure(
                    id,
                    error("revision", "Revision exceeds 128 bytes."),
                ));
            }
            match compile_with_sources(&model, &sources.unwrap_or_default()) {
                Ok(plan) => {
                    let execution = execution.unwrap_or_default();
                    if let Err(error) = execution.validate(&plan) {
                        return Some(failure(id, json!(error)));
                    }
                    let mut result = summary(&plan);
                    let options = execution.summary();
                    result["backend"] = options["backend"].clone();
                    result["solver"] = options["solver"].clone();
                    response(id, json!({"revision":revision,"valid":true,"plan":result}))
                }
                Err(err) => failure(id, json!(err.0)),
            }
        }
        Operation::Run {
            model,
            revision,
            sources,
            execution,
        } => {
            if !v3 && model.schema_version >= 3 {
                return Some(failure(
                    id,
                    error("protocol", "stateful components require /simulation/v3"),
                ));
            }
            if !v2 && (sources.is_some() || execution.is_some() || model.schema_version != 1) {
                return Some(failure(
                    id,
                    error(
                        "protocol",
                        "source snapshots and execution options require /simulation/v2",
                    ),
                ));
            }
            if job.is_some() {
                failure(
                    id,
                    error("busy", "A simulation is already active on this connection."),
                )
            } else if revision.len() > 128 {
                failure(id, error("revision", "Revision exceeds 128 bytes."))
            } else {
                match start_snapshot(
                    id.clone(),
                    revision,
                    Snapshot {
                        model,
                        sources: sources.unwrap_or_default(),
                        execution: execution.unwrap_or_default(),
                    },
                ) {
                    Ok(next) => {
                        *job = Some(next);
                        return None;
                    }
                    Err(err) => failure(id, error("worker", &err.to_string())),
                }
            }
        }
        Operation::Cancel { run_id } => {
            if let Some(active) = job.as_ref().filter(|active| active.id == run_id) {
                active.cancel.store(true, Ordering::Relaxed);
                response(id, json!({"runId":run_id,"cancelling":true}))
            } else {
                failure(
                    id,
                    error("run_not_found", "The run is not active on this connection."),
                )
            }
        }
        Operation::ImportSlx { name, bytes } => match import_slx(&name, &bytes) {
            Ok(value) => response(id, value),
            Err(err) => failure(id, err),
        },
    })
}

fn send(
    socket: &mut WebSocket<TcpStream>,
    value: &Value,
    protocol: &str,
) -> Result<(), ServerError> {
    let mut value = value.clone();
    value["protocol"] = json!(protocol);
    let text = serde_json::to_string(&value).map_err(ServerError::Encode)?;
    if text.len() > MAX_MESSAGE {
        return Err(ServerError::State("simulation output exceeds 8 MiB"));
    }
    socket.send(Message::text(text))?;
    Ok(())
}

pub(crate) fn serve(socket: &mut WebSocket<TcpStream>, version: u32) -> Result<(), ServerError> {
    let protocol = match version {
        3 => PROTOCOL_V3,
        2 => PROTOCOL_V2,
        _ => PROTOCOL,
    };
    let mut job: Option<Job> = None;
    loop {
        if let Some(active) = &mut job {
            // Drain a bounded number so cancel frames continue to be serviced.
            for _ in 0..8 {
                let Ok(value) = active.events.as_ref().expect("active receiver").try_recv() else {
                    break;
                };
                active.observe(&value);
                send(socket, &value, protocol)?;
            }
            if active.terminal {
                job.take();
            } else if active.worker.as_ref().is_some_and(JoinHandle::is_finished) {
                // The queue may still have results. Only an empty disconnected
                // channel without a terminal message means the worker failed.
                if let Ok(value) = active.events.as_ref().expect("active receiver").try_recv() {
                    active.observe(&value);
                    send(socket, &value, protocol)?;
                    if active.terminal {
                        job.take();
                    }
                } else {
                    let issue = error("worker", "Simulation worker exited unexpectedly.");
                    let value = if active.acknowledged {
                        json!({"protocol":PROTOCOL,"event":"failed","runId":active.id,
                                "revision":active.revision,"sequence":active.sequence + 1,"data":{"error":issue}})
                    } else {
                        failure(&active.id, issue)
                    };
                    send(socket, &value, protocol)?;
                    job.take();
                }
            }
        }
        match socket.read() {
            Ok(Message::Text(text)) => {
                if text.len() > MAX_MESSAGE {
                    socket.close(None)?;
                    return Ok(());
                }
                let reply = match serde_json::from_str::<Request>(&text) {
                    Ok(request) if request.protocol == protocol => handle(request, &mut job),
                    Ok(request) => Some(failure(
                        &request.id,
                        error("protocol", "protocol does not match the WebSocket endpoint"),
                    )),
                    Err(err) => {
                        let id = serde_json::from_str::<Value>(&text)
                            .ok()
                            .and_then(|value| {
                                value
                                    .get("requestId")
                                    .and_then(Value::as_str)
                                    .filter(|id| id.len() <= 128)
                                    .map(str::to_owned)
                            })
                            .unwrap_or_default();
                        Some(failure(&id, error("request", &err.to_string())))
                    }
                };
                if let Some(value) = reply {
                    send(socket, &value, protocol)?;
                }
            }
            Ok(Message::Close(_)) => {
                let _ = socket.flush();
                return Ok(());
            }
            Ok(Message::Ping(_) | Message::Pong(_)) => socket.flush()?,
            Ok(_) => {
                socket.close(None)?;
                return Ok(());
            }
            Err(WsError::Io(err))
                if matches!(
                    err.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            Err(WsError::ConnectionClosed | WsError::AlreadyClosed) => return Ok(()),
            Err(err) => return Err(ServerError::WebSocket(err)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn component_protocol_keeps_older_routes_strict() {
        let model: Model = serde_json::from_str(include_str!(
            "../../../simulation/examples/custom-delay.omsim.json"
        ))
        .unwrap();
        for protocol in [PROTOCOL, PROTOCOL_V2] {
            let response = handle(
                Request {
                    protocol: protocol.into(),
                    id: "component".into(),
                    operation: Operation::Check {
                        model: model.clone(),
                        revision: "1".into(),
                        sources: None,
                        execution: None,
                    },
                },
                &mut None,
            )
            .unwrap();
            assert_eq!(response["error"]["code"], "protocol");
        }
        let response = handle(
            Request {
                protocol: PROTOCOL_V3.into(),
                id: "catalog".into(),
                operation: Operation::Catalog,
            },
            &mut None,
        )
        .unwrap();
        assert_eq!(response["result"]["schemaVersion"], 3);
        assert!(
            response["result"]["blocks"]
                .as_array()
                .unwrap()
                .iter()
                .any(|b| b["type"] == "component")
        );
    }

    fn model() -> Model {
        serde_json::from_str(include_str!(
            "../../../simulation/examples/first-order.omsim.json"
        ))
        .unwrap()
    }

    #[test]
    fn request_version_and_fields_are_checked() {
        let request: Request = serde_json::from_value(
            json!({"protocol":PROTOCOL,"requestId":"catalog","operation":"catalog"}),
        )
        .unwrap();
        let response = handle(request, &mut None).unwrap();
        assert_eq!(response["ok"], true);
        assert_eq!(response["result"]["blocks"].as_array().unwrap().len(), 6);
        assert!(serde_json::from_value::<Request>(json!({"protocol":PROTOCOL,"requestId":"bad","operation":"run","model":model(),"revision":"r","extra":1})).is_err());
        let bad: Request = serde_json::from_value(
            json!({"protocol":"future","requestId":"id","operation":"catalog"}),
        )
        .unwrap();
        let rejection = handle(bad, &mut None).unwrap();
        assert_eq!(rejection["error"]["code"], "protocol");
        assert_eq!(rejection["requestId"], "id");
    }

    #[test]
    fn streamed_trajectory_matches_the_analytic_solution() {
        let job = start_job("run".into(), "revision".into(), model()).unwrap();
        let receiver = job.events.as_ref().unwrap();
        let ack = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(ack["result"]["backend"], "reference");
        let mut sequence = 0;
        let mut count = 0;
        loop {
            let event = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
            sequence += 1;
            assert_eq!(event["sequence"], sequence);
            assert_eq!(event["revision"], "revision");
            if event["event"] == "finished" {
                break;
            }
            assert_eq!(event["event"], "samples");
            for frame in event["data"]["frames"].as_array().unwrap() {
                let time = frame["time"].as_f64().unwrap();
                let actual = frame["values"][0].as_f64().unwrap();
                assert!((actual - (1.0 - (-time).exp())).abs() < 1e-7);
                count += 1;
            }
        }
        assert_eq!(count, 21);
    }

    #[test]
    fn a_wide_initial_scope_frame_is_flushed_separately() {
        let mut wide = model();
        for block in &mut wide.blocks {
            match &mut block.kind {
                openmat_sim::model::BlockKind::Integrator { initial } => *initial = vec![0.0; 9000],
                openmat_sim::model::BlockKind::Constant { value } => *value = vec![1.0; 9000],
                _ => {}
            }
        }
        let job = start_job("wide-initial".into(), "r".into(), wide).unwrap();
        let receiver = job.events.as_ref().unwrap();
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(10)).unwrap()["ok"],
            true
        );
        let event = receiver.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(event["event"], "samples");
        let frames = event["data"]["frames"].as_array().unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0]["time"], 0.0);
        assert_eq!(frames[0]["values"].as_array().unwrap().len(), 9000);
    }

    #[test]
    fn value_limit_still_applies_after_a_batch_has_been_flushed() {
        let mut wide = model();
        for block in &mut wide.blocks {
            if let openmat_sim::model::BlockKind::Integrator { initial } = &mut block.kind {
                *initial = vec![0.0; 1300];
            }
            if let openmat_sim::model::BlockKind::Constant { value } = &mut block.kind {
                *value = vec![1.0; 1300];
            }
        }
        wide.settings.stop_time = 100.0;
        let job = start_job("wide".into(), "r".into(), wide).unwrap();
        let receiver = job.events.as_ref().unwrap();
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(10)).unwrap()["ok"],
            true
        );
        let mut values = 0;
        loop {
            let event = receiver.recv_timeout(Duration::from_secs(10)).unwrap();
            if event["event"] == "failed" {
                assert_eq!(event["data"]["error"]["code"], "result_limit");
                break;
            }
            assert_eq!(event["event"], "samples");
            values += event["data"]["frames"].as_array().unwrap().len() * 1300;
            assert!(values <= MAX_VALUES);
        }
        assert_eq!(values, MAX_VALUES / 1300 * 1300);
    }

    #[test]
    fn cancellation_and_disconnect_release_a_backpressured_worker() {
        let mut long = model();
        long.settings.stop_time = 1e6;
        let job = start_job("long".into(), "r".into(), long).unwrap();
        thread::sleep(Duration::from_millis(50));
        let start = Instant::now();
        drop(job);
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn import_rejects_invalid_packages_and_oversized_uploads() {
        assert!(import_slx("bad.slx", b"not a package").is_err());
        assert_eq!(
            import_slx("big.slx", &vec![0; MAX_SLX + 1]).unwrap_err()["code"],
            "slx_limit"
        );
    }
}
