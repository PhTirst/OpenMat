use openmat_bytecode::{
    BytecodeModule, BytecodeVersion, ExceptionHandler, ExceptionHandlerKind, Function, FunctionId,
    Instruction, InstructionIndex, InstructionKind, LocalSlot, Register, SourceLocation,
    VerificationErrorKind, verify,
};

fn pc(index: u32) -> InstructionIndex {
    InstructionIndex::new(index)
}

fn operation() -> Instruction {
    Instruction::new(InstructionKind::LoadCallInputCount {
        dst: Register::new(0),
    })
}

fn return_instruction() -> Instruction {
    Instruction::new(InstructionKind::Return { values: Vec::new() })
}

fn module(
    instructions: Vec<Instruction>,
    local_count: u32,
    handlers: Vec<ExceptionHandler>,
) -> BytecodeModule {
    let mut function = Function::new("main", 1, local_count, 0);
    function.instructions = instructions;
    function.exception_handlers = handlers;
    BytecodeModule::new(vec![function], FunctionId::new(0))
}

fn catch_module(error_local: Option<LocalSlot>) -> BytecodeModule {
    module(
        vec![
            operation(),
            Instruction::new(InstructionKind::Jump { target: pc(3) }),
            operation(),
            return_instruction(),
        ],
        u32::from(error_local.is_some()),
        vec![ExceptionHandler::catch(
            pc(0),
            pc(1),
            pc(2),
            pc(3),
            error_local,
        )],
    )
}

#[test]
fn accepts_plain_bound_and_no_catch_forms() {
    assert_eq!(verify(&catch_module(None)), Ok(()));
    assert_eq!(verify(&catch_module(Some(LocalSlot::new(0)))), Ok(()));

    let swallow = module(
        vec![operation(), return_instruction()],
        0,
        vec![ExceptionHandler::swallow(pc(0), pc(1), pc(1))],
    );
    assert_eq!(verify(&swallow), Ok(()));
    assert_eq!(
        swallow.functions[0].exception_handlers[0].kind,
        ExceptionHandlerKind::Swallow
    );
}

#[test]
fn accepts_nested_ranges_independent_of_table_order() {
    let instructions = vec![
        operation(),
        operation(),
        operation(),
        Instruction::new(InstructionKind::Jump { target: pc(5) }),
        operation(),
        operation(),
        operation(),
        Instruction::new(InstructionKind::Jump { target: pc(10) }),
        operation(),
        operation(),
        return_instruction(),
    ];
    let inner = ExceptionHandler::catch(pc(1), pc(3), pc(4), pc(5), None);
    let outer = ExceptionHandler::catch(pc(0), pc(7), pc(8), pc(10), None);

    assert_eq!(
        verify(&module(instructions.clone(), 0, vec![inner, outer])),
        Ok(())
    );
    assert_eq!(verify(&module(instructions, 0, vec![outer, inner])), Ok(()));
}

#[test]
fn accepts_a_call_as_the_protected_fault_pc() {
    let instructions = vec![
        Instruction::new(InstructionKind::Call {
            outputs: Vec::new(),
            callee: Register::new(0),
            arguments: Vec::new(),
        }),
        Instruction::new(InstructionKind::Jump { target: pc(3) }),
        operation(),
        return_instruction(),
    ];
    assert_eq!(
        verify(&module(
            instructions,
            0,
            vec![ExceptionHandler::catch(pc(0), pc(1), pc(2), pc(3), None)]
        )),
        Ok(())
    );
}

#[test]
fn rejects_invalid_protected_ranges_and_targets() {
    let mut invalid = catch_module(None);
    invalid.functions[0].exception_handlers[0].protected_end = pc(0);
    assert!(matches!(
        verify(&invalid).expect_err("empty range must fail").kind,
        VerificationErrorKind::InvalidExceptionProtectedRange { .. }
    ));

    let mut invalid = catch_module(None);
    invalid.functions[0].exception_handlers[0].protected_end = pc(4);
    assert!(matches!(
        verify(&invalid)
            .expect_err("out-of-stream range end must fail")
            .kind,
        VerificationErrorKind::InvalidExceptionProtectedRange { .. }
    ));

    let mut invalid = catch_module(None);
    let ExceptionHandlerKind::Catch { handler, .. } =
        &mut invalid.functions[0].exception_handlers[0].kind
    else {
        unreachable!()
    };
    *handler = pc(4);
    assert!(matches!(
        verify(&invalid)
            .expect_err("out-of-stream handler must fail")
            .kind,
        VerificationErrorKind::InvalidExceptionHandler { .. }
    ));

    let mut invalid = catch_module(None);
    invalid.functions[0].exception_handlers[0].exit = pc(4);
    assert!(matches!(
        verify(&invalid)
            .expect_err("out-of-stream exit must fail")
            .kind,
        VerificationErrorKind::InvalidExceptionExit { .. }
    ));
}

#[test]
fn rejects_invalid_catch_local_and_structured_layout() {
    let invalid = catch_module(Some(LocalSlot::new(1)));
    assert!(matches!(
        verify(&invalid)
            .expect_err("out-of-frame catch local must fail")
            .kind,
        VerificationErrorKind::InvalidLocal { .. }
    ));

    let mut invalid = catch_module(None);
    let ExceptionHandlerKind::Catch { handler, .. } =
        &mut invalid.functions[0].exception_handlers[0].kind
    else {
        unreachable!()
    };
    *handler = pc(1);
    assert!(matches!(
        verify(&invalid)
            .expect_err("catch must immediately follow the skip jump")
            .kind,
        VerificationErrorKind::InvalidExceptionCatchOrder { .. }
    ));

    let mut invalid = catch_module(None);
    invalid.functions[0].instructions[1] = operation();
    assert!(matches!(
        verify(&invalid)
            .expect_err("normal completion must skip catch")
            .kind,
        VerificationErrorKind::InvalidExceptionCatchSkip { .. }
    ));

    let invalid = module(
        vec![operation(), operation(), return_instruction()],
        0,
        vec![ExceptionHandler::swallow(pc(0), pc(1), pc(2))],
    );
    assert!(matches!(
        verify(&invalid)
            .expect_err("no-catch continuation must be the protected end")
            .kind,
        VerificationErrorKind::InvalidExceptionSwallowExit { .. }
    ));
}

#[test]
fn rejects_duplicate_and_crossing_ranges_but_accepts_containment() {
    let instructions = vec![
        operation(),
        operation(),
        operation(),
        operation(),
        operation(),
        return_instruction(),
    ];
    let outer = ExceptionHandler::swallow(pc(0), pc(5), pc(5));
    let inner = ExceptionHandler::swallow(pc(1), pc(3), pc(3));
    assert_eq!(
        verify(&module(instructions.clone(), 0, vec![outer, inner])),
        Ok(())
    );

    let duplicate = module(instructions.clone(), 0, vec![inner, inner]);
    assert!(matches!(
        verify(&duplicate)
            .expect_err("equal ranges must be unambiguous")
            .kind,
        VerificationErrorKind::DuplicateExceptionRange { .. }
    ));

    let crossing = module(
        instructions,
        0,
        vec![
            ExceptionHandler::swallow(pc(0), pc(3), pc(3)),
            ExceptionHandler::swallow(pc(2), pc(5), pc(5)),
        ],
    );
    assert!(matches!(
        verify(&crossing)
            .expect_err("crossing protected ranges must fail")
            .kind,
        VerificationErrorKind::CrossingExceptionRanges { .. }
    ));

    let escaping = module(
        vec![
            operation(),
            operation(),
            operation(),
            Instruction::new(InstructionKind::Jump { target: pc(7) }),
            operation(),
            operation(),
            operation(),
            return_instruction(),
        ],
        0,
        vec![
            ExceptionHandler::swallow(pc(0), pc(5), pc(5)),
            ExceptionHandler::catch(pc(1), pc(3), pc(4), pc(7), None),
        ],
    );
    assert!(matches!(
        verify(&escaping)
            .expect_err("nested catch and continuation must remain protected by the outer range")
            .kind,
        VerificationErrorKind::NestedExceptionHandlerEscapes { .. }
    ));
}

#[test]
fn preserves_handler_metadata_and_rejects_v15_artifacts_diagnostically() {
    let location = SourceLocation::new(7, 11, 29);
    let handler = ExceptionHandler::catch(pc(0), pc(1), pc(2), pc(3), Some(LocalSlot::new(0)))
        .with_location(location);
    let handlers = vec![handler];
    let function = Function::new("roundtrip", 1, 1, 0).with_exception_handlers(handlers.clone());
    assert_eq!(function.exception_handlers, handlers);
    assert_eq!(function.exception_handlers[0].location, Some(location));

    let mut old = catch_module(None);
    old.version = BytecodeVersion::new(15, 0);
    let error = verify(&old).expect_err("v15 lacks the current bytecode contract");
    assert_eq!(
        error.kind,
        VerificationErrorKind::UnsupportedVersion {
            found: BytecodeVersion::new(15, 0),
            supported: openmat_bytecode::CURRENT_BYTECODE_VERSION,
        }
    );
    assert!(
        error
            .to_string()
            .contains("unsupported bytecode version 15.0")
    );
}

#[test]
fn reports_handler_source_location_for_metadata_errors() {
    let location = SourceLocation::new(9, 40, 55);
    let invalid = module(
        vec![operation(), return_instruction()],
        0,
        vec![ExceptionHandler::swallow(pc(0), pc(0), pc(1)).with_location(location)],
    );
    let error = verify(&invalid).expect_err("invalid metadata must retain try provenance");
    assert_eq!(error.function, Some(FunctionId::new(0)));
    assert_eq!(error.instruction, None);
    assert_eq!(error.location, Some(location));
}
