use openmat_kernel::{ExecutionEngine, Kernel, RuntimeEngine};
use openmat_protocol::{
    ExecuteRequest, ExecutionMode, InitializeRequest, Request, RequestEnvelope, ServerMessage,
};

fn main() {
    let mut args = std::env::args_os().skip(1);
    let script = args.next().expect("script path");
    let plugins: Vec<_> = args.collect();
    assert!(!plugins.is_empty(), "at least one plugin path required");
    // SAFETY: verification loads only the SDK's locally built plugin fixtures.
    let engine =
        unsafe { RuntimeEngine::with_oex_plugins(plugins) }.expect("load Rust OEX plugins");
    let initialize = Request::Initialize(InitializeRequest {
        client: engine.implementation(),
        supported_protocols: vec![],
        capabilities: engine.capabilities(),
    });
    let mut kernel = Kernel::new("rust-sdk-test", engine);
    let execute = Request::Execute(ExecuteRequest {
        code: std::fs::read_to_string(&script).unwrap(),
        source_name: script.to_string_lossy().into_owned(),
        mode: ExecutionMode::File,
    });
    for (i, request) in [initialize, execute].into_iter().enumerate() {
        let messages = kernel.handle_request(&RequestEnvelope::new(
            "rust-sdk-test",
            i.to_string(),
            request,
        ));
        if messages
            .iter()
            .any(|message| matches!(message, ServerMessage::Response(response) if !response.ok))
        {
            eprintln!("{messages:#?}");
            std::process::exit(1);
        }
    }
    println!("Rust OEX integration passed");
}
