use openmat_bytecode::{BytecodeModule, Function, FunctionId, Instruction, InstructionKind};
use openmat_runtime::{
    BuiltinContext, BuiltinErrorCategory, CancellationToken, GraphicsClass, GraphicsRequest,
    GraphicsResponse, GraphicsSession, Interpreter, OutputError, OutputEvent, OutputSink,
    VecOutput,
};
use openmat_value::Value;

fn empty_module() -> BytecodeModule {
    let mut entry = Function::new("<graphics-test>", 0, 0, 0);
    entry
        .instructions
        .push(Instruction::new(InstructionKind::Return {
            values: Vec::new(),
        }));
    BytecodeModule::new(vec![entry], FunctionId::new(0))
}

#[test]
fn interpreter_owns_graphics_beyond_workspace_clear_and_releases_it_on_session_clear() {
    let mut interpreter = Interpreter::new(empty_module()).unwrap();
    let execution = interpreter
        .graphics_session_mut()
        .execute(GraphicsRequest::CurrentFigure)
        .unwrap();
    let GraphicsResponse::Handle(figure) = execution.response else {
        panic!()
    };
    interpreter.workspace_mut().insert("x", Value::Double(1.0));
    interpreter.workspace_mut().clear();
    assert!(interpreter.graphics_session().is_valid(figure));
    assert_eq!(interpreter.graphics_session().object_count(), 1);

    interpreter.clear_session();
    assert_eq!(interpreter.graphics_session().object_count(), 0);
    assert!(!interpreter.graphics_session().is_valid(figure));
}

#[test]
fn graphics_context_emits_internal_notice_without_data_or_attachment_token() {
    let mut session = GraphicsSession::new();
    let cancellation = CancellationToken::new();
    let mut output = VecOutput::new();
    let mut context =
        BuiltinContext::with_graphics_service(1, &cancellation, &mut output, &mut session);
    let response = context.graphics(GraphicsRequest::CurrentFigure).unwrap();
    let GraphicsResponse::Handle(handle) = response else {
        panic!()
    };
    assert_eq!(handle.class(), GraphicsClass::Figure);
    let [OutputEvent::GraphicsNotice(notice)] = output.events() else {
        panic!("one internal graphics notice expected")
    };
    assert!(notice.discovery);
    assert!(!notice.figure_id.is_empty());
    assert_eq!(notice.revision, 1);
}

struct RejectOutput;

impl OutputSink for RejectOutput {
    fn emit(&mut self, _event: OutputEvent) -> Result<(), OutputError> {
        Err(OutputError::new("test output rejected"))
    }
}

#[test]
fn notice_failure_reports_output_error_but_keeps_committed_graphics_state() {
    let mut session = GraphicsSession::new();
    let cancellation = CancellationToken::new();
    let mut output = RejectOutput;
    let mut context =
        BuiltinContext::with_graphics_service(1, &cancellation, &mut output, &mut session);
    let error = context
        .graphics(GraphicsRequest::CurrentFigure)
        .expect_err("notice output must fail");
    assert_eq!(error.category, BuiltinErrorCategory::Output);
    assert_eq!(session.object_count(), 1);
    assert!(session.current_figure().is_some());
}
