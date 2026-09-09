use openmat_bytecode::{
    Access as BytecodeAccess, ApplyArgument, AssignmentMode, BinaryOperator, BindingTarget,
    ClassKind, Constant, ExceptionHandlerKind, FieldOperand, FunctionId, InstructionKind,
    LocalSlot, MIN_SUPPORTED_BYTECODE_VERSION, MethodKind as BytecodeMethodKind, PersistentSlot,
    PlaceStep, PropertyKind as BytecodePropertyKind, SourceLocation, StatementResultTarget,
    ValueSource, VerificationErrorKind, verify,
};
use openmat_compiler::{
    BindingDeclarationProblem, BindingOperation, BindingStorageClass, ClassAttributeProblem,
    ClassMemberProblem, CompileDiagnosticKind, UnsupportedFeature, compile,
};
use openmat_hir::{
    Attribute, BinaryOp, CatchClause, ClassDef, ClearForm, ClearStatement, CommandArgument,
    CommandStatement, ConditionalBranch, DeclarationContext, DeclarationForm, DeclarationKind,
    DeclarationStatement, EnumMemberDef, EnumerationBlock, EventBlock, EventDef, Expr, ExprKind,
    FunctionDef, HirFile, MethodBlock, MethodDeclaration, Name, OtherwiseBranch, PropertyBlock,
    PropertyDef, Stmt, StmtKind, SwitchCase, TransposeKind, TryStatement, UnaryOp,
};
use openmat_source::{SourceId, TextRange};

const SOURCE: SourceId = SourceId::new(77);

fn range(start: u32, end: u32) -> TextRange {
    TextRange::new(start, end).expect("ordered test range")
}

fn expression(kind: ExprKind, start: u32, end: u32) -> Expr {
    Expr {
        kind,
        span: range(start, end),
    }
}

fn name(text: &str, start: u32, end: u32) -> Expr {
    expression(ExprKind::Name(text.to_owned()), start, end)
}

fn number(text: &str, start: u32, end: u32) -> Expr {
    expression(ExprKind::Number(text.to_owned()), start, end)
}

fn named(text: &str, start: u32, end: u32) -> Name {
    Name {
        text: text.to_owned(),
        span: range(start, end),
    }
}

fn function_handle(text: &str, start: u32, end: u32) -> Expr {
    expression(
        ExprKind::FunctionHandle(Some(named(text, start + 1, end))),
        start,
        end,
    )
}

fn anonymous(parameters: Vec<Name>, body: Expr, start: u32, end: u32) -> Expr {
    expression(
        ExprKind::AnonymousFunction {
            parameters,
            body: Box::new(body),
        },
        start,
        end,
    )
}

fn assignment(target: Expr, value: Expr, start: u32, end: u32) -> Stmt {
    Stmt {
        kind: StmtKind::Assignment { target, value },
        span: range(start, end),
        suppress_output: true,
    }
}

fn command(callee: &str, arguments: &[&str], start: u32, end: u32, suppress_output: bool) -> Stmt {
    Stmt {
        kind: StmtKind::Command(CommandStatement {
            callee: Some(named(
                callee,
                start,
                start + u32::try_from(callee.len()).unwrap(),
            )),
            arguments: arguments
                .iter()
                .map(|text| CommandArgument {
                    text: (*text).to_owned(),
                    span: range(start, end),
                })
                .collect(),
        }),
        span: range(start, end),
        suppress_output,
    }
}

fn declaration(
    kind: DeclarationKind,
    context: DeclarationContext,
    names: Vec<Name>,
    start: u32,
    end: u32,
) -> Stmt {
    Stmt {
        kind: StmtKind::Declaration(DeclarationStatement {
            kind,
            names,
            form: DeclarationForm::IdentifierList,
            context,
        }),
        span: range(start, end),
        suppress_output: true,
    }
}

fn binary(operator: BinaryOp, left: Expr, right: Expr, start: u32, end: u32) -> Expr {
    expression(
        ExprKind::Binary {
            operator,
            left: Box::new(left),
            right: Box::new(right),
        },
        start,
        end,
    )
}

fn call(target: Expr, arguments: Vec<Expr>, start: u32, end: u32) -> Expr {
    expression(
        ExprKind::ParenApply {
            target: Box::new(target),
            arguments,
        },
        start,
        end,
    )
}

fn brace(target: Expr, arguments: Vec<Expr>, start: u32, end: u32) -> Expr {
    expression(
        ExprKind::BraceApply {
            target: Box::new(target),
            arguments,
        },
        start,
        end,
    )
}

fn field(target: Expr, member: &str, start: u32, end: u32) -> Expr {
    let member_len = u32::try_from(member.len()).expect("test field name length fits u32");
    expression(
        ExprKind::Field {
            target: Box::new(target),
            name: Some(named(member, end.saturating_sub(member_len), end)),
        },
        start,
        end,
    )
}

fn dynamic_field(target: Expr, member: Expr, start: u32, end: u32) -> Expr {
    expression(
        ExprKind::DynamicField {
            target: Box::new(target),
            name: Box::new(member),
        },
        start,
        end,
    )
}

fn file(statements: Vec<Stmt>) -> HirFile {
    HirFile {
        source_id: SOURCE,
        span: range(0, 200),
        statements,
    }
}

fn try_statement(
    body: Vec<Stmt>,
    catch: Option<(Option<Name>, Vec<Stmt>)>,
    start: u32,
    end: u32,
) -> Stmt {
    let statement_span = range(start, end);
    Stmt {
        kind: StmtKind::Try(TryStatement {
            body,
            body_span: statement_span,
            catch: catch.map(|(variable, body)| CatchClause {
                variable,
                body,
                body_span: statement_span,
                span: statement_span,
            }),
        }),
        span: statement_span,
        suppress_output: true,
    }
}

#[test]
#[allow(clippy::too_many_lines)]
fn lowers_entry_plain_bound_and_no_catch_with_workspace_binding() {
    let plain_span = range(0, 35);
    let bound_span = range(40, 105);
    let swallow_span = range(115, 145);
    let target = FunctionDef {
        name: Some(named("caught", 165, 171)),
        outputs: Vec::new(),
        inputs: Vec::new(),
        body: Vec::new(),
        span: range(160, 190),
    };
    let hir = file(vec![
        try_statement(
            vec![assignment(
                name("plain_body", 5, 15),
                number("1", 18, 19),
                5,
                20,
            )],
            Some((
                None,
                vec![assignment(
                    name("plain_catch", 24, 35),
                    number("2", 38, 39),
                    24,
                    40,
                )],
            )),
            plain_span.start(),
            plain_span.end(),
        ),
        try_statement(
            vec![assignment(
                name("bound_body", 45, 55),
                number("3", 58, 59),
                45,
                60,
            )],
            Some((
                Some(named("caught", 66, 72)),
                vec![
                    assignment(
                        name("handle", 74, 80),
                        function_handle("caught", 83, 90),
                        74,
                        91,
                    ),
                    assignment(name("copied", 93, 99), name("caught", 102, 108), 93, 109),
                ],
            )),
            bound_span.start(),
            bound_span.end(),
        ),
        assignment(name("after", 106, 111), name("caught", 114, 120), 106, 121),
        try_statement(
            vec![assignment(
                name("swallow_body", 120, 132),
                number("4", 135, 136),
                120,
                137,
            )],
            None,
            swallow_span.start(),
            swallow_span.end(),
        ),
        Stmt {
            kind: StmtKind::Function(target),
            span: range(160, 190),
            suppress_output: true,
        },
    ]);

    let module = compile(&hir).expect("all entry try forms should compile");
    verify(&module).expect("entry exception table should verify");
    let entry = &module.functions[0];
    assert_eq!(entry.exception_handlers.len(), 3);
    assert_eq!(
        entry.local_count, 1,
        "bound entry catch needs one bridge local"
    );

    let plain = &entry.exception_handlers[0];
    let ExceptionHandlerKind::Catch {
        handler: plain_handler,
        error_local: None,
    } = plain.kind
    else {
        panic!("plain catch must discard the error value");
    };
    assert_eq!(plain_handler.get(), plain.protected_end.get() + 1);
    assert!(matches!(
        entry.instructions[plain.protected_end.get() as usize].kind,
        InstructionKind::Jump { target } if target == plain.exit
    ));

    let bound = &entry.exception_handlers[1];
    let ExceptionHandlerKind::Catch {
        handler: bound_handler,
        error_local: Some(error_local),
    } = bound.kind
    else {
        panic!("bound catch must have an error destination");
    };
    assert_eq!(error_local, LocalSlot::new(0));
    assert_eq!(bound_handler.get(), bound.protected_end.get() + 1);
    assert!(matches!(
        entry.instructions[bound_handler.get() as usize].kind,
        InstructionKind::LoadLocal { local, .. } if local == error_local
    ));
    assert!(matches!(
        entry.instructions[bound_handler.get() as usize + 1].kind,
        InstructionKind::StoreGlobal { name, .. }
            if entry.constants[name.get() as usize] == Constant::String("caught".to_owned())
    ));
    assert!(matches!(
        entry.instructions[bound.protected_end.get() as usize].kind,
        InstructionKind::Jump { target } if target == bound.exit
    ));

    let caught_loads = entry
        .instructions
        .iter()
        .filter(|instruction| {
            matches!(
                instruction.kind,
                InstructionKind::LoadGlobal { name, .. }
                    if entry.constants[name.get() as usize] == Constant::String("caught".to_owned())
            )
        })
        .count();
    assert_eq!(
        caught_loads, 2,
        "catch and following statements read workspace state"
    );
    assert!(
        entry
            .constants
            .contains(&Constant::Function(FunctionId::new(1)))
    );

    let swallow = &entry.exception_handlers[2];
    assert_eq!(swallow.kind, ExceptionHandlerKind::Swallow);
    assert_eq!(swallow.protected_end, swallow.exit);
    assert!(matches!(
        entry.instructions[swallow.exit.get() as usize].kind,
        InstructionKind::Jump { target } if target.get() == swallow.exit.get() + 1
    ));
}

#[test]
#[allow(clippy::too_many_lines)]
fn lowers_function_try_forms_with_surrounding_locals_and_free_name_capture() {
    let definition = FunctionDef {
        name: Some(named("exercise", 0, 8)),
        outputs: vec![named("out", 9, 12)],
        inputs: Vec::new(),
        body: vec![
            try_statement(
                vec![assignment(
                    name("plain_local", 20, 31),
                    number("1", 34, 35),
                    20,
                    36,
                )],
                Some((
                    None,
                    vec![assignment(
                        name("plain_catch", 40, 51),
                        number("2", 54, 55),
                        40,
                        56,
                    )],
                )),
                15,
                60,
            ),
            try_statement(
                vec![assignment(
                    name("bound_local", 65, 76),
                    number("3", 79, 80),
                    65,
                    81,
                )],
                Some((
                    Some(named("caught", 84, 90)),
                    vec![assignment(
                        name("capture", 92, 99),
                        anonymous(Vec::new(), name("caught", 104, 110), 101, 110),
                        92,
                        111,
                    )],
                )),
                62,
                115,
            ),
            assignment(name("out", 117, 120), name("caught", 123, 129), 117, 130),
            try_statement(
                vec![assignment(
                    name("swallow_local", 135, 148),
                    number("4", 151, 152),
                    135,
                    153,
                )],
                None,
                132,
                158,
            ),
        ],
        span: range(0, 180),
    };
    let hir = file(vec![Stmt {
        kind: StmtKind::Function(definition),
        span: range(0, 180),
        suppress_output: true,
    }]);

    let first = compile(&hir).expect("function try forms should compile");
    let second = compile(&hir).expect("try lowering should be deterministic");
    assert_eq!(first, second);
    verify(&first).expect("function exception table should verify");
    let function = &first.functions[1];
    assert_eq!(function.local_count, 7);
    assert_eq!(function.exception_handlers.len(), 3);
    assert!(matches!(
        function.exception_handlers[0].kind,
        ExceptionHandlerKind::Catch {
            error_local: None,
            ..
        }
    ));
    let bound = &function.exception_handlers[1];
    assert!(matches!(
        bound.kind,
        ExceptionHandlerKind::Catch {
            error_local: Some(local),
            ..
        } if local == LocalSlot::new(4)
    ));
    assert!(matches!(
        function.instructions[bound.exit.get() as usize].kind,
        InstructionKind::LoadLocal { local, .. } if local == LocalSlot::new(4)
    ));
    assert!(
        function
            .instructions
            .iter()
            .all(|instruction| !matches!(instruction.kind, InstructionKind::StoreGlobal { .. }))
    );
    assert_eq!(
        function.exception_handlers[2].kind,
        ExceptionHandlerKind::Swallow
    );
    assert!(function.instructions.iter().any(|instruction| matches!(
        &instruction.kind,
        InstructionKind::MakeClosure { captures, .. }
            if matches!(captures.as_slice(), [(_, Some(_), true)])
    )));
}

#[test]
fn empty_try_bodies_emit_no_handlers_or_unreachable_catch_code() {
    let hir = file(vec![
        try_statement(
            Vec::new(),
            Some((
                Some(named("caught", 8, 14)),
                vec![assignment(
                    name("unreachable", 18, 29),
                    number("1", 32, 33),
                    18,
                    34,
                )],
            )),
            0,
            40,
        ),
        try_statement(Vec::new(), None, 42, 55),
        assignment(name("after", 58, 63), number("2", 66, 67), 58, 68),
    ]);

    let module = compile(&hir).expect("empty protected bodies are valid");
    verify(&module).expect("empty try lowering should verify");
    let entry = &module.functions[0];
    assert!(entry.exception_handlers.is_empty());
    assert_eq!(entry.local_count, 0);
    assert!(
        !entry
            .constants
            .contains(&Constant::String("unreachable".to_owned()))
    );

    let malformed = file(vec![try_statement(
        vec![Stmt {
            kind: StmtKind::Expr(expression(ExprKind::Error, 5, 6)),
            span: range(5, 7),
            suppress_output: true,
        }],
        Some((None, Vec::new())),
        0,
        20,
    )]);
    let error = compile(&malformed).expect_err("compiler diagnostics are not runtime errors");
    assert!(error.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "OMC0001"
            && matches!(diagnostic.kind, CompileDiagnosticKind::MalformedHir { .. })
    }));
}

#[test]
fn empty_plain_catch_uses_the_continuation_as_its_handler() {
    let hir = file(vec![
        try_statement(
            vec![assignment(
                name("protected", 5, 14),
                number("1", 17, 18),
                5,
                19,
            )],
            Some((None, Vec::new())),
            0,
            30,
        ),
        assignment(name("after", 33, 38), number("2", 41, 42), 33, 43),
    ]);

    let module = compile(&hir).expect("an empty catch body should compile");
    verify(&module).expect("the empty catch continuation should verify");
    let entry = &module.functions[0];
    let [handler] = entry.exception_handlers.as_slice() else {
        panic!("one catch handler expected");
    };
    let ExceptionHandlerKind::Catch {
        handler: catch,
        error_local: None,
    } = handler.kind
    else {
        panic!("plain catch expected");
    };
    assert_eq!(catch, handler.exit);
    assert!(matches!(
        entry.instructions[handler.protected_end.get() as usize].kind,
        InstructionKind::Jump { target } if target == handler.exit
    ));
}

#[test]
fn nested_catch_body_is_outside_itself_and_inside_the_outer_handler() {
    let inner_span = range(10, 65);
    let outer_span = range(0, 100);
    let inner = try_statement(
        vec![assignment(
            name("inner_body", 15, 25),
            number("1", 28, 29),
            15,
            30,
        )],
        Some((
            None,
            vec![Stmt {
                kind: StmtKind::Expr(call(name("fail", 42, 46), Vec::new(), 42, 48)),
                span: range(42, 49),
                suppress_output: true,
            }],
        )),
        inner_span.start(),
        inner_span.end(),
    );
    let hir = file(vec![try_statement(
        vec![inner],
        Some((
            None,
            vec![assignment(
                name("outer_catch", 75, 86),
                number("2", 89, 90),
                75,
                91,
            )],
        )),
        outer_span.start(),
        outer_span.end(),
    )]);

    let module = compile(&hir).expect("nested catches should compile");
    verify(&module).expect("nested catch ranges should verify");
    let entry = &module.functions[0];
    let located = |span: TextRange| {
        entry
            .exception_handlers
            .iter()
            .find(|handler| {
                handler.location
                    == Some(SourceLocation::new(SOURCE.raw(), span.start(), span.end()))
            })
            .expect("source-located exception handler")
    };
    let inner = located(inner_span);
    let outer = located(outer_span);
    assert!(outer.protected_start <= inner.protected_start);
    assert!(inner.protected_end < outer.protected_end);
    assert!(inner.exit <= outer.protected_end);

    let failing_apply = entry
        .instructions
        .iter()
        .position(|instruction| {
            matches!(instruction.kind, InstructionKind::StatementApply { .. })
                && instruction.location == Some(SourceLocation::new(SOURCE.raw(), 42, 48))
        })
        .expect("inner catch failing apply");
    let failing_apply = u32::try_from(failing_apply).expect("test pc fits u32");
    let ExceptionHandlerKind::Catch {
        handler: inner_handler,
        ..
    } = inner.kind
    else {
        panic!("inner handler must be catch");
    };
    assert!(inner_handler.get() <= failing_apply && failing_apply < inner.exit.get());
    assert!(failing_apply >= inner.protected_end.get());
    assert!(
        outer.protected_start.get() <= failing_apply && failing_apply < outer.protected_end.get()
    );
}

#[test]
fn nested_no_catch_ranges_are_strictly_contained_and_verify() {
    let inner = try_statement(
        vec![assignment(
            name("value", 15, 20),
            number("1", 23, 24),
            15,
            25,
        )],
        None,
        10,
        35,
    );
    let hir = file(vec![try_statement(vec![inner], None, 0, 45)]);

    let module = compile(&hir).expect("nested swallowing handlers should compile");
    verify(&module).expect("nested swallowing ranges should verify");
    let handlers = &module.functions[0].exception_handlers;
    assert_eq!(handlers.len(), 2);
    let inner = &handlers[0];
    let outer = &handlers[1];
    assert_eq!(inner.kind, ExceptionHandlerKind::Swallow);
    assert_eq!(outer.kind, ExceptionHandlerKind::Swallow);
    assert!(outer.protected_start <= inner.protected_start);
    assert!(inner.protected_end < outer.protected_end);
    assert!(inner.exit < outer.protected_end);
}

#[test]
fn compiles_arithmetic_workspace_assignments_with_source_locations() {
    let addition_span = range(10, 15);
    let hir = file(vec![
        assignment(name("x", 0, 1), number("2", 4, 5), 0, 6),
        assignment(
            name("y", 7, 8),
            binary(
                BinaryOp::Add,
                name("x", 10, 11),
                number("3", 14, 15),
                addition_span.start(),
                addition_span.end(),
            ),
            7,
            16,
        ),
    ]);

    let module = compile(&hir).expect("arithmetic HIR should compile");
    verify(&module).expect("compiler output must verify");
    let entry = &module.functions[0];
    assert_eq!(module.entry, FunctionId::new(0));
    assert_eq!(
        entry
            .instructions
            .iter()
            .filter(|instruction| matches!(instruction.kind, InstructionKind::StoreGlobal { .. }))
            .count(),
        2
    );
    let addition = entry
        .instructions
        .iter()
        .find(|instruction| {
            matches!(
                instruction.kind,
                InstructionKind::Binary {
                    operator: BinaryOperator::Add,
                    ..
                }
            )
        })
        .expect("addition instruction");
    assert_eq!(
        addition.location,
        Some(SourceLocation::new(
            SOURCE.raw(),
            addition_span.start(),
            addition_span.end()
        ))
    );
    assert!(entry.instructions.iter().all(|instruction| {
        instruction
            .location
            .is_some_and(|location| location.source_id == SOURCE.raw())
    }));
}

#[test]
fn command_form_lowers_text_arguments_to_an_ordinary_statement_apply() {
    let hir = file(vec![
        command("hold", &["on"], 0, 8, true),
        command("grid", &["off"], 9, 17, false),
        command("box", &["on"], 18, 24, true),
    ]);

    let module = compile(&hir).expect("command-form HIR should compile");
    verify(&module).expect("command-form bytecode must verify");
    let entry = &module.functions[0];
    assert!(
        entry
            .constants
            .contains(&Constant::Char(vec![u16::from(b'o'), u16::from(b'n')]))
    );
    assert!(entry.constants.contains(&Constant::Char(vec![
        u16::from(b'o'),
        u16::from(b'f'),
        u16::from(b'f')
    ])));
    let applications = entry
        .instructions
        .iter()
        .filter_map(|instruction| match &instruction.kind {
            InstructionKind::StatementApply {
                arguments, display, ..
            } => Some((arguments, *display)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(applications.len(), 3);
    assert!(
        applications
            .iter()
            .all(|(arguments, _)| { matches!(arguments.as_slice(), [ApplyArgument::Value(_)]) })
    );
    assert_eq!(
        applications
            .iter()
            .map(|(_, display)| *display)
            .collect::<Vec<_>>(),
        [false, true, false]
    );
}

#[test]
fn bare_name_statement_uses_zero_argument_statement_application() {
    let hir = file(vec![Stmt {
        kind: StmtKind::Expr(name("clc", 0, 3)),
        span: range(0, 3),
        suppress_output: true,
    }]);

    let module = compile(&hir).expect("bare statement name should compile");
    verify(&module).expect("bare statement bytecode must verify");
    let entry = &module.functions[0];
    assert!(
        entry
            .instructions
            .iter()
            .any(|instruction| matches!(instruction.kind, InstructionKind::LoadCallTarget { .. }))
    );
    assert!(entry.instructions.iter().any(|instruction| matches!(
        &instruction.kind,
        InstructionKind::StatementApply {
            arguments,
            display: false,
            ..
        } if arguments.is_empty()
    )));
    assert!(
        !entry
            .instructions
            .iter()
            .any(|instruction| matches!(instruction.kind, InstructionKind::StatementValue { .. }))
    );
}

#[test]
fn unresolved_bare_calls_preserve_argument_sensitive_runtime_dispatch() {
    let hir = file(vec![assignment(
        name("result", 0, 6),
        call(name("dispatch", 9, 17), vec![number("1", 18, 19)], 9, 20),
        0,
        21,
    )]);

    let module = compile(&hir).expect("unresolved call should compile");
    verify(&module).expect("named call-target bytecode must verify");
    let entry = &module.functions[0];
    let target = entry
        .instructions
        .iter()
        .find_map(|instruction| {
            let InstructionKind::LoadCallTarget { name, .. } = instruction.kind else {
                return None;
            };
            Some(&entry.constants[name.get() as usize])
        })
        .expect("unresolved bare call should retain a named call target");
    assert_eq!(target, &Constant::String("dispatch".to_owned()));
}

#[test]
fn bare_timer_statements_request_zero_outputs() {
    let hir = file(vec![
        Stmt {
            kind: StmtKind::Expr(name("tic", 0, 3)),
            span: range(0, 3),
            suppress_output: true,
        },
        Stmt {
            kind: StmtKind::Expr(name("toc", 4, 7)),
            span: range(4, 7),
            suppress_output: true,
        },
    ]);

    let module = compile(&hir).expect("bare timer statements should compile");
    verify(&module).expect("bare timer bytecode must verify");
    let applications = module.functions[0]
        .instructions
        .iter()
        .filter(|instruction| {
            matches!(
                instruction.kind,
                InstructionKind::StatementApply { display: false, .. }
            )
        })
        .count();
    assert_eq!(applications, 2);
}

#[test]
fn clearvars_except_lowers_to_lexical_clears_instead_of_a_callable() {
    let definition = FunctionDef {
        name: Some(named("clear_except", 70, 82)),
        outputs: vec![named("result", 60, 66)],
        inputs: Vec::new(),
        body: vec![
            assignment(name("keep", 85, 89), number("17", 92, 94), 85, 95),
            assignment(name("drop", 96, 100), number("29", 103, 105), 96, 106),
            command("clearvars", &["-except", "keep"], 107, 129, true),
            assignment(name("result", 130, 136), name("keep", 139, 143), 130, 144),
        ],
        span: range(55, 150),
    };
    let hir = file(vec![Stmt {
        kind: StmtKind::Function(definition),
        span: range(55, 150),
        suppress_output: true,
    }]);

    let module = compile(&hir).expect("clearvars -except should compile");
    verify(&module).expect("clearvars bytecode must verify");
    let function = &module.functions[1];
    assert!(
        function.instructions.iter().any(|instruction| matches!(
            &instruction.kind,
            InstructionKind::ClearLocal { locals } if locals.len() == 3
        )),
        "{:?}",
        function.instructions
    );
    assert!(
        !function
            .instructions
            .iter()
            .any(|instruction| matches!(instruction.kind, InstructionKind::StatementApply { .. }))
    );
    assert!(!function.constants.iter().any(|constant| matches!(
        constant,
        Constant::String(name) if name == "clearvars"
    )));
}

#[test]
fn bare_clearvars_clears_the_entry_workspace_without_callable_resolution() {
    let hir = file(vec![
        assignment(name("value", 0, 5), number("1", 8, 9), 0, 10),
        Stmt {
            kind: StmtKind::Expr(name("clearvars", 11, 20)),
            span: range(11, 20),
            suppress_output: true,
        },
    ]);

    let module = compile(&hir).expect("bare clearvars should compile");
    verify(&module).expect("bare clearvars bytecode must verify");
    let entry = &module.functions[0];
    assert!(
        entry
            .instructions
            .iter()
            .any(|instruction| matches!(instruction.kind, InstructionKind::ClearGlobalAll))
    );
    assert!(!entry.constants.iter().any(|constant| matches!(
        constant,
        Constant::String(name) if name == "clearvars"
    )));
}

#[test]
fn bare_true_and_false_names_lower_to_logical_constants() {
    let hir = file(vec![
        assignment(name("t", 0, 1), name("true", 4, 8), 0, 9),
        assignment(name("f", 10, 11), name("false", 14, 19), 10, 20),
    ]);

    let module = compile(&hir).expect("unbound logical names should compile as constants");
    let entry = &module.functions[0];
    assert!(entry.constants.contains(&Constant::Logical(true)));
    assert!(entry.constants.contains(&Constant::Logical(false)));
    assert!(!entry.constants.iter().any(|constant| matches!(
        constant,
        Constant::String(name) if name == "true" || name == "false"
    )));
}

#[test]
fn entry_and_function_bindings_shadow_logical_constant_names() {
    let shadow = FunctionDef {
        name: Some(named("shadow", 80, 86)),
        outputs: vec![named("false", 70, 75)],
        inputs: vec![named("true", 87, 91)],
        body: vec![assignment(
            name("false", 94, 99),
            name("true", 102, 106),
            94,
            107,
        )],
        span: range(65, 115),
    };
    let hir = file(vec![
        assignment(name("true", 0, 4), number("1", 7, 8), 0, 9),
        assignment(name("entry_t", 10, 17), name("true", 20, 24), 10, 25),
        assignment(name("false", 26, 31), number("0", 34, 35), 26, 36),
        assignment(name("entry_f", 37, 44), name("false", 47, 52), 37, 53),
        Stmt {
            kind: StmtKind::Function(shadow),
            span: range(65, 115),
            suppress_output: true,
        },
    ]);

    let module = compile(&hir).expect("value bindings should shadow logical constants");
    let entry = &module.functions[0];
    let loaded_globals: Vec<_> = entry
        .instructions
        .iter()
        .filter_map(|instruction| match instruction.kind {
            InstructionKind::LoadGlobal { name, .. } => {
                match entry.constants.get(name.get() as usize) {
                    Some(Constant::String(name)) => Some(name.as_str()),
                    _ => None,
                }
            }
            _ => None,
        })
        .collect();
    assert_eq!(loaded_globals, vec!["true", "false"]);
    assert!(
        !entry
            .constants
            .iter()
            .any(|constant| matches!(constant, Constant::Logical(_)))
    );

    let function = &module.functions[1];
    assert_eq!(function.parameter_count, 1);
    assert_eq!(function.local_count, 2);
    assert!(
        function
            .instructions
            .iter()
            .any(|instruction| matches!(instruction.kind, InstructionKind::LoadLocal { .. }))
    );
    assert!(
        function
            .instructions
            .iter()
            .any(|instruction| matches!(instruction.kind, InstructionKind::StoreLocal { .. }))
    );
    assert!(
        !function
            .instructions
            .iter()
            .any(|instruction| matches!(instruction.kind, InstructionKind::LoadGlobal { .. }))
    );
    assert!(
        !function
            .constants
            .iter()
            .any(|constant| matches!(constant, Constant::Logical(_)))
    );
}

#[test]
fn true_and_false_apply_targets_remain_dynamic_names() {
    let hir = file(vec![
        assignment(
            name("t", 0, 1),
            call(name("true", 4, 8), vec![number("2", 9, 10)], 4, 11),
            0,
            12,
        ),
        assignment(
            name("f", 13, 14),
            call(name("false", 17, 22), vec![number("2", 23, 24)], 17, 25),
            13,
            26,
        ),
    ]);

    let module = compile(&hir).expect("logical constructor names should remain dynamic applies");
    let entry = &module.functions[0];
    assert_eq!(
        entry
            .instructions
            .iter()
            .filter(|instruction| matches!(instruction.kind, InstructionKind::Apply { .. }))
            .count(),
        2
    );
    assert!(
        entry
            .constants
            .iter()
            .any(|constant| matches!(constant, Constant::String(name) if name == "true"))
    );
    assert!(
        entry
            .constants
            .iter()
            .any(|constant| matches!(constant, Constant::String(name) if name == "false"))
    );
    assert!(
        !entry
            .constants
            .iter()
            .any(|constant| matches!(constant, Constant::Logical(_)))
    );
}

#[test]
fn logical_short_circuit_skips_rhs_apply_and_builds_matrix_result() {
    let false_and_missing = binary(
        BinaryOp::ShortCircuitAnd,
        name("false", 4, 9),
        call(name("missing", 13, 20), Vec::new(), 13, 22),
        4,
        22,
    );
    let true_or_missing = binary(
        BinaryOp::ShortCircuitOr,
        name("true", 24, 28),
        call(name("missing", 32, 39), Vec::new(), 32, 41),
        24,
        41,
    );
    let matrix = expression(
        ExprKind::Matrix(vec![vec![false_and_missing, true_or_missing]]),
        3,
        42,
    );
    let hir = file(vec![assignment(name("result", 0, 1), matrix, 0, 43)]);

    let module = compile(&hir).expect("short-circuit matrix should compile");
    verify(&module).expect("short-circuit matrix bytecode should verify");
    let entry = &module.functions[0];
    let instructions = &entry.instructions;
    let apply_indices: Vec<_> = instructions
        .iter()
        .enumerate()
        .filter_map(|(index, instruction)| {
            matches!(instruction.kind, InstructionKind::Apply { .. }).then_some(index)
        })
        .collect();
    assert_eq!(apply_indices.len(), 2);
    let build_index = instructions
        .iter()
        .position(|instruction| matches!(instruction.kind, InstructionKind::BuildMatrix { .. }))
        .expect("matrix construction instruction");

    assert!(instructions.iter().enumerate().any(|(index, instruction)| {
        matches!(
            instruction.kind,
            InstructionKind::JumpIfFalse { target, .. }
                if index < apply_indices[0] && target.get() as usize > apply_indices[0]
        )
    }));
    assert!(instructions.iter().enumerate().any(|(index, instruction)| {
        matches!(
            instruction.kind,
            InstructionKind::Jump { target }
                if index > apply_indices[0]
                    && index < apply_indices[1]
                    && target.get() as usize > apply_indices[1]
        )
    }));
    assert!(matches!(
        &instructions[build_index].kind,
        InstructionKind::BuildMatrix { rows, .. }
            if matches!(rows.as_slice(), [row] if row.len() == 2)
    ));

    let loaded_calls: Vec<_> = instructions
        .iter()
        .filter_map(|instruction| match instruction.kind {
            InstructionKind::LoadCallTarget { name, .. } => {
                match entry.constants.get(name.get() as usize) {
                    Some(Constant::String(name)) => Some(name.as_str()),
                    _ => None,
                }
            }
            _ => None,
        })
        .collect();
    assert_eq!(loaded_calls, vec!["missing", "missing"]);
}

#[test]
fn compiles_if_elseif_else_with_fully_patched_jumps() {
    let first = ConditionalBranch {
        condition: number("0", 3, 4),
        body: vec![assignment(name("x", 8, 9), number("1", 12, 13), 8, 14)],
        span: range(0, 14),
    };
    let second = ConditionalBranch {
        condition: number("1", 20, 21),
        body: vec![assignment(name("x", 25, 26), number("2", 29, 30), 25, 31)],
        span: range(15, 31),
    };
    let hir = file(vec![Stmt {
        kind: StmtKind::If {
            branches: vec![first, second],
            else_body: vec![assignment(name("x", 38, 39), number("3", 42, 43), 38, 44)],
        },
        span: range(0, 48),
        suppress_output: true,
    }]);

    let module = compile(&hir).expect("branch HIR should compile");
    let instructions = &module.functions[0].instructions;
    assert_eq!(
        instructions
            .iter()
            .filter(|instruction| matches!(instruction.kind, InstructionKind::JumpIfFalse { .. }))
            .count(),
        2
    );
    for instruction in instructions {
        match instruction.kind {
            InstructionKind::Jump { target } | InstructionKind::JumpIfFalse { target, .. } => {
                assert!((target.get() as usize) < instructions.len());
            }
            _ => {}
        }
    }
}

#[test]
fn compiles_while_break_and_continue() {
    let increment = assignment(
        name("x", 20, 21),
        binary(
            BinaryOp::Add,
            name("x", 24, 25),
            number("1", 28, 29),
            24,
            29,
        ),
        20,
        30,
    );
    let break_if = Stmt {
        kind: StmtKind::If {
            branches: vec![ConditionalBranch {
                condition: binary(
                    BinaryOp::Equal,
                    name("x", 34, 35),
                    number("3", 39, 40),
                    34,
                    40,
                ),
                body: vec![Stmt {
                    kind: StmtKind::Break,
                    span: range(44, 49),
                    suppress_output: true,
                }],
                span: range(31, 49),
            }],
            else_body: Vec::new(),
        },
        span: range(31, 53),
        suppress_output: true,
    };
    let while_statement = Stmt {
        kind: StmtKind::While {
            condition: binary(
                BinaryOp::Less,
                name("x", 10, 11),
                number("10", 14, 16),
                10,
                16,
            ),
            body: vec![
                increment,
                break_if,
                Stmt {
                    kind: StmtKind::Continue,
                    span: range(54, 62),
                    suppress_output: true,
                },
            ],
        },
        span: range(6, 66),
        suppress_output: true,
    };
    let hir = file(vec![
        assignment(name("x", 0, 1), number("0", 4, 5), 0, 6),
        while_statement,
    ]);

    let module = compile(&hir).expect("loop HIR should compile");
    let instructions = &module.functions[0].instructions;
    let jumps: Vec<_> = instructions
        .iter()
        .enumerate()
        .filter_map(|(index, instruction)| match instruction.kind {
            InstructionKind::Jump { target } => Some((index, target.get() as usize)),
            _ => None,
        })
        .collect();
    assert!(jumps.iter().any(|(index, target)| target < index));
    assert!(jumps.iter().any(|(index, target)| target > index));
    verify(&module).expect("loop jumps must verify");
}

#[test]
fn resolves_parameters_locals_outputs_and_user_function_calls() {
    let definition = FunctionDef {
        name: Some(named("pair", 80, 84)),
        outputs: vec![named("sum", 70, 73), named("product", 74, 81)],
        inputs: vec![named("a", 85, 86), named("b", 88, 89)],
        body: vec![
            assignment(
                name("sum", 92, 95),
                binary(
                    BinaryOp::Add,
                    name("a", 98, 99),
                    name("b", 102, 103),
                    98,
                    103,
                ),
                92,
                104,
            ),
            assignment(
                name("product", 106, 113),
                binary(
                    BinaryOp::Multiply,
                    name("a", 116, 117),
                    name("b", 120, 121),
                    116,
                    121,
                ),
                106,
                122,
            ),
            Stmt {
                kind: StmtKind::Return,
                span: range(124, 130),
                suppress_output: true,
            },
        ],
        span: range(65, 130),
    };
    let targets = expression(
        ExprKind::Matrix(vec![vec![name("s", 1, 2), name("p", 4, 5)]]),
        0,
        6,
    );
    let invoke = call(
        name("pair", 9, 13),
        vec![number("2", 14, 15), number("3", 17, 18)],
        9,
        19,
    );
    let hir = file(vec![
        assignment(targets, invoke, 0, 20),
        Stmt {
            kind: StmtKind::Function(definition),
            span: range(65, 130),
            suppress_output: true,
        },
    ]);

    let module = compile(&hir).expect("function HIR should compile");
    assert_eq!(module.functions.len(), 2);
    let entry = &module.functions[0];
    assert!(
        entry
            .constants
            .contains(&Constant::Function(FunctionId::new(1)))
    );
    assert!(entry.instructions.iter().any(|instruction| matches!(
        &instruction.kind,
        InstructionKind::Apply { outputs, .. } if outputs.len() == 2
    )));
    let pair = &module.functions[1];
    assert_eq!(pair.name, "pair");
    assert_eq!(pair.parameter_count, 2);
    assert_eq!(pair.local_count, 4);
    assert!(pair.instructions.iter().any(|instruction| matches!(
        &instruction.kind,
        InstructionKind::Return { values } if values.len() == 2
    )));
}

#[test]
fn lowers_named_varargin_and_varargout_as_dynamic_function_abi() {
    let definition = FunctionDef {
        name: Some(named("forward", 80, 87)),
        outputs: vec![named("head", 60, 64), named("varargout", 66, 75)],
        inputs: vec![named("head", 88, 92), named("varargin", 94, 102)],
        body: vec![assignment(
            name("varargout", 110, 119),
            name("varargin", 122, 130),
            110,
            131,
        )],
        span: range(55, 140),
    };
    let hir = file(vec![Stmt {
        kind: StmtKind::Function(definition),
        span: range(55, 140),
        suppress_output: true,
    }]);

    let module = compile(&hir).expect("named variadic function should compile");
    let function = &module.functions[1];
    assert_eq!(function.parameter_count, 1);
    assert_eq!(function.local_count, 3);
    assert!(matches!(
        function
            .instructions
            .iter()
            .find(|instruction| {
                !matches!(
                    instruction.kind,
                    InstructionKind::DeclareNamedBindings { .. }
                )
            })
            .map(|instruction| &instruction.kind),
        Some(InstructionKind::LoadVariadicInputs { .. })
    ));
    assert!(matches!(
        function.instructions.last().map(|instruction| &instruction.kind),
        Some(InstructionKind::ReturnVariadic { fixed, .. }) if fixed.len() == 1
    ));
}

#[test]
fn lowers_nested_function_handles_with_shared_lexical_storage() {
    let nested = FunctionDef {
        name: Some(named("increment", 130, 139)),
        outputs: vec![named("value", 120, 125)],
        inputs: Vec::new(),
        body: vec![
            assignment(
                name("count", 145, 150),
                binary(
                    BinaryOp::Add,
                    name("count", 153, 158),
                    number("1", 161, 162),
                    153,
                    162,
                ),
                145,
                163,
            ),
            assignment(name("value", 165, 170), name("count", 173, 178), 165, 179),
        ],
        span: range(115, 185),
    };
    let parent = FunctionDef {
        name: Some(named("factory", 80, 87)),
        outputs: vec![named("handle", 70, 76)],
        inputs: Vec::new(),
        body: vec![
            assignment(name("count", 90, 95), number("0", 98, 99), 90, 100),
            assignment(
                name("handle", 102, 108),
                function_handle("increment", 111, 121),
                102,
                122,
            ),
            Stmt {
                kind: StmtKind::Function(nested),
                span: range(115, 185),
                suppress_output: true,
            },
        ],
        span: range(65, 195),
    };
    let hir = file(vec![Stmt {
        kind: StmtKind::Function(parent),
        span: range(65, 195),
        suppress_output: true,
    }]);

    let module = compile(&hir).expect("nested function should compile");
    assert_eq!(module.functions.len(), 3);
    let parent = &module.functions[1];
    assert!(parent.instructions.iter().any(|instruction| matches!(
        &instruction.kind,
        InstructionKind::MakeSharedClosure { function, captures, .. }
            if *function == FunctionId::new(2) && captures.len() == 1
    )));
    let nested = &module.functions[2];
    assert!(
        nested
            .instructions
            .iter()
            .any(|instruction| matches!(instruction.kind, InstructionKind::LoadCapture { .. }))
    );
    assert!(
        nested
            .instructions
            .iter()
            .any(|instruction| matches!(instruction.kind, InstructionKind::StoreCapture { .. }))
    );
}

#[test]
fn rejects_nonfinal_named_variadic_parameters_and_outputs() {
    let definition = FunctionDef {
        name: Some(named("invalid", 80, 87)),
        outputs: vec![named("varargout", 50, 59), named("tail", 61, 65)],
        inputs: vec![named("varargin", 88, 96), named("tail", 98, 102)],
        body: Vec::new(),
        span: range(45, 110),
    };
    let hir = file(vec![Stmt {
        kind: StmtKind::Function(definition),
        span: range(45, 110),
        suppress_output: true,
    }]);

    let error = compile(&hir).expect_err("nonfinal variadic names must be diagnosed");
    assert_eq!(
        error
            .diagnostics()
            .iter()
            .filter(|diagnostic| matches!(
                diagnostic.kind,
                CompileDiagnosticKind::MalformedHir { .. }
            ))
            .count(),
        2
    );
}

#[test]
fn lowers_named_local_and_unresolved_function_handles_without_value_lookup() {
    let local = FunctionDef {
        name: Some(named("local_target", 100, 112)),
        outputs: vec![named("result", 90, 96)],
        inputs: vec![named("value", 113, 118)],
        body: vec![assignment(
            name("result", 120, 126),
            name("value", 129, 134),
            120,
            135,
        )],
        span: range(85, 140),
    };
    let hir = file(vec![
        assignment(name("shadowed", 0, 8), number("9", 11, 12), 0, 13),
        assignment(
            name("local_handle", 14, 26),
            function_handle("local_target", 29, 42),
            14,
            43,
        ),
        assignment(
            name("external_handle", 44, 59),
            function_handle("external_target", 62, 78),
            44,
            79,
        ),
        assignment(
            name("shadow_handle", 80, 93),
            function_handle("shadowed", 96, 105),
            80,
            106,
        ),
        Stmt {
            kind: StmtKind::Function(local),
            span: range(85, 140),
            suppress_output: true,
        },
    ]);

    let module = compile(&hir).expect("named function handles should compile");
    let entry = &module.functions[0];
    assert!(
        entry
            .constants
            .contains(&Constant::Function(FunctionId::new(1)))
    );
    let unresolved = entry
        .instructions
        .iter()
        .filter_map(|instruction| {
            let InstructionKind::LoadFunctionHandle { name, .. } = instruction.kind else {
                return None;
            };
            let Constant::String(name) = &entry.constants[name.get() as usize] else {
                panic!("verified handle name must be a string");
            };
            Some((name.as_str(), instruction.location))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        unresolved,
        [
            (
                "external_target",
                Some(SourceLocation::new(SOURCE.raw(), 62, 78))
            ),
            ("shadowed", Some(SourceLocation::new(SOURCE.raw(), 96, 105)))
        ]
    );
    assert_eq!(
        entry
            .instructions
            .iter()
            .filter(|instruction| matches!(instruction.kind, InstructionKind::LoadGlobal { .. }))
            .count(),
        0
    );

    let missing = file(vec![assignment(
        name("bad", 0, 3),
        expression(ExprKind::FunctionHandle(None), 6, 7),
        0,
        8,
    )]);
    let error = compile(&missing).expect_err("missing handle target must be diagnosed");
    assert!(matches!(
        error.diagnostics()[0].kind,
        CompileDiagnosticKind::MalformedHir { .. }
    ));
}

#[test]
fn bare_local_function_value_is_invoked_but_explicit_handle_is_not() {
    let local = FunctionDef {
        name: Some(named("local_value", 80, 91)),
        outputs: vec![named("result", 70, 76)],
        inputs: Vec::new(),
        body: vec![assignment(
            name("result", 94, 100),
            number("7", 103, 104),
            94,
            105,
        )],
        span: range(65, 110),
    };
    let hir = file(vec![
        assignment(name("value", 0, 5), name("local_value", 8, 19), 0, 20),
        assignment(
            name("handle", 21, 27),
            function_handle("local_value", 30, 42),
            21,
            43,
        ),
        Stmt {
            kind: StmtKind::Function(local),
            span: range(65, 110),
            suppress_output: true,
        },
    ]);

    let module = compile(&hir).expect("bare local function values should compile");
    verify(&module).expect("bare local function bytecode must verify");
    let entry = &module.functions[0];
    assert_eq!(
        entry
            .instructions
            .iter()
            .filter(|instruction| matches!(instruction.kind, InstructionKind::Call { .. }))
            .count(),
        1
    );
    assert!(entry.instructions.iter().any(|instruction| matches!(
        &instruction.kind,
        InstructionKind::Call {
            outputs,
            arguments,
            ..
        } if outputs.len() == 1 && arguments.is_empty()
    )));
    assert_eq!(
        entry
            .constants
            .iter()
            .filter(|constant| **constant == Constant::Function(FunctionId::new(1)))
            .count(),
        2
    );
}

#[test]
fn compiles_anonymous_function_parameters_captures_and_body_function() {
    let captured = anonymous(
        vec![named("x", 18, 19)],
        binary(
            BinaryOp::Add,
            name("x", 21, 22),
            name("scale", 25, 30),
            21,
            30,
        ),
        16,
        30,
    );
    let hir = file(vec![
        assignment(name("scale", 0, 5), number("3", 8, 9), 0, 10),
        assignment(name("f", 11, 12), captured, 11, 31),
    ]);

    let module = compile(&hir).expect("anonymous closure should compile");
    assert_eq!(module.functions.len(), 2);
    let entry = &module.functions[0];
    let closure = &module.functions[1];
    let make = entry
        .instructions
        .iter()
        .find_map(|instruction| match &instruction.kind {
            InstructionKind::MakeClosure {
                function, captures, ..
            } => Some((*function, captures.clone(), instruction.location)),
            _ => None,
        })
        .expect("MakeClosure instruction");
    assert_eq!(make.0, FunctionId::new(1));
    assert_eq!(make.2, Some(SourceLocation::new(SOURCE.raw(), 16, 30)));
    assert_eq!(make.1.len(), 1);
    assert!(make.1[0].1.is_none());
    assert!(make.1[0].2);
    assert!(matches!(
        &entry.constants[make.1[0].0.get() as usize],
        Constant::String(name) if name == "scale"
    ));
    assert_eq!(closure.parameter_count, 1);
    assert_eq!(closure.local_count, 1);
    assert!(closure.instructions.iter().any(|instruction| matches!(
        instruction.kind,
        InstructionKind::LoadGlobal { name, .. }
            if matches!(&closure.constants[name.get() as usize], Constant::String(value) if value == "scale")
    )));
    assert!(matches!(
        closure.instructions.last().map(|instruction| &instruction.kind),
        Some(InstructionKind::Return { values }) if values.len() == 1
    ));
}

#[test]
fn anonymous_parameters_shadow_captures_and_function_locals_use_register_operands() {
    let shadowed = file(vec![
        assignment(name("scale", 0, 5), number("4", 8, 9), 0, 10),
        assignment(
            name("f", 11, 12),
            anonymous(vec![named("scale", 15, 20)], name("scale", 22, 27), 13, 27),
            11,
            28,
        ),
    ]);
    let module = compile(&shadowed).expect("parameter shadowing should compile");
    assert!(
        module.functions[0]
            .instructions
            .iter()
            .any(|instruction| matches!(
                &instruction.kind,
                InstructionKind::MakeClosure { captures, .. } if captures.is_empty()
            ))
    );

    let make = FunctionDef {
        name: Some(named("make", 40, 44)),
        outputs: vec![named("handle", 30, 36)],
        inputs: vec![named("offset", 45, 51)],
        body: vec![assignment(
            name("handle", 53, 59),
            anonymous(
                vec![named("x", 62, 63)],
                binary(
                    BinaryOp::Add,
                    name("x", 65, 66),
                    name("offset", 69, 75),
                    65,
                    75,
                ),
                60,
                75,
            ),
            53,
            76,
        )],
        span: range(30, 80),
    };
    let module = compile(&file(vec![Stmt {
        kind: StmtKind::Function(make),
        span: range(30, 80),
        suppress_output: true,
    }]))
    .expect("function-local capture should compile");
    assert_eq!(module.functions.len(), 3);
    assert!(
        module.functions[1]
            .instructions
            .iter()
            .any(|instruction| matches!(
                &instruction.kind,
                InstructionKind::MakeClosure { function, captures, .. }
                    if *function == FunctionId::new(2)
                        && matches!(captures.as_slice(), [(_, Some(_), true)])
            ))
    );
}

#[test]
fn anonymous_top_level_apply_uses_dynamic_requested_output_tail_return() {
    let hir = file(vec![assignment(
        name("shape", 0, 5),
        anonymous(
            vec![named("value", 8, 13)],
            call(name("size", 15, 19), vec![name("value", 20, 25)], 15, 26),
            6,
            26,
        ),
        0,
        27,
    )]);
    let module = compile(&hir).expect("tail apply closure should compile");
    let entry = &module.functions[0];
    let closure = &module.functions[1];
    assert!(entry.instructions.iter().any(|instruction| matches!(
        &instruction.kind,
        InstructionKind::MakeClosure { captures, .. }
            if matches!(captures.as_slice(), [(_, None, false)])
    )));
    assert!(matches!(
        closure.instructions.last().map(|instruction| &instruction.kind),
        Some(InstructionKind::ReturnApply { arguments, .. }) if arguments.len() == 1
    ));
}

#[test]
fn anonymous_member_apply_forwards_dynamic_output_count() {
    let hir = file(vec![assignment(
        name("callback", 0, 8),
        anonymous(
            vec![named("obj", 10, 13)],
            call(field(name("obj", 15, 18), "run", 15, 22), vec![], 15, 24),
            9,
            24,
        ),
        0,
        25,
    )]);
    let module = compile(&hir).expect("member callback compiles");
    assert!(matches!(
        module.functions[1].instructions.last().map(|instruction| &instruction.kind),
        Some(InstructionKind::ReturnApplyField { arguments, .. }) if arguments.is_empty()
    ));
}

#[test]
fn duplicate_anonymous_parameters_are_structured_compiler_diagnostics() {
    let hir = file(vec![assignment(
        name("f", 0, 1),
        anonymous(
            vec![named("x", 4, 5), named("x", 7, 8)],
            name("x", 10, 11),
            2,
            11,
        ),
        0,
        12,
    )]);
    let error = compile(&hir).expect_err("duplicate parameters must fail compilation");
    assert!(error.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "OMC0003"
            && diagnostic.range == range(7, 8)
            && matches!(
                &diagnostic.kind,
                CompileDiagnosticKind::DuplicateParameter { name } if name == "x"
            )
    }));
}

#[test]
fn lowers_final_anonymous_varargin_through_the_named_variadic_abi() {
    let variadic = file(vec![assignment(
        name("f", 0, 1),
        anonymous(
            vec![named("head", 4, 8), named("varargin", 10, 18)],
            name("varargin", 20, 28),
            2,
            28,
        ),
        0,
        29,
    )]);
    let module = compile(&variadic).expect("final anonymous varargin should compile");
    verify(&module).expect("anonymous variadic bytecode should verify");
    let closure = &module.functions[1];
    assert_eq!(closure.parameter_count, 1);
    assert_eq!(closure.local_count, 2);
    assert!(matches!(
        closure
            .instructions
            .iter()
            .find(|instruction| !matches!(
                instruction.kind,
                InstructionKind::DeclareNamedBindings { .. }
            ))
            .map(|instruction| &instruction.kind),
        Some(InstructionKind::LoadVariadicInputs { .. })
    ));
    assert!(closure.instructions.iter().any(|instruction| matches!(
        instruction.kind,
        InstructionKind::StoreLocal { local, .. } if local == LocalSlot::new(1)
    )));
}

#[test]
fn treats_nonfinal_anonymous_varargin_as_an_ordinary_fixed_parameter() {
    let fixed = file(vec![assignment(
        name("f", 0, 1),
        anonymous(
            vec![named("varargin", 4, 12), named("tail", 14, 18)],
            name("varargin", 20, 28),
            2,
            28,
        ),
        0,
        29,
    )]);
    let module = compile(&fixed).expect("nonfinal anonymous varargin should remain fixed");
    let closure = &module.functions[1];
    assert_eq!(closure.parameter_count, 2);
    assert_eq!(closure.local_count, 2);
    assert!(
        !closure.instructions.iter().any(|instruction| matches!(
            instruction.kind,
            InstructionKind::LoadVariadicInputs { .. }
        ))
    );
}

#[test]
fn lowers_bare_nargin_nargout_and_preserves_call_ambiguity() {
    let definition = FunctionDef {
        name: Some(named("metadata", 100, 108)),
        outputs: vec![named("head", 90, 94), named("tail", 95, 99)],
        inputs: vec![named("left", 109, 113), named("right", 115, 120)],
        body: vec![
            assignment(name("head", 121, 125), name("nargin", 128, 134), 121, 135),
            assignment(name("tail", 136, 140), name("nargout", 143, 150), 136, 151),
            Stmt {
                kind: StmtKind::Expr(call(name("nargin", 152, 158), Vec::new(), 152, 160)),
                span: range(152, 161),
                suppress_output: true,
            },
        ],
        span: range(85, 170),
    };
    let multiple_target = expression(
        ExprKind::Matrix(vec![vec![name("a", 20, 21), name("b", 23, 24)]]),
        19,
        25,
    );
    let hir = file(vec![
        assignment(
            name("single", 0, 6),
            call(
                name("metadata", 9, 17),
                vec![number("1", 18, 19), number("2", 20, 21)],
                9,
                22,
            ),
            0,
            23,
        ),
        assignment(
            multiple_target,
            call(
                name("metadata", 28, 36),
                vec![number("1", 37, 38), number("2", 40, 41)],
                28,
                42,
            ),
            19,
            43,
        ),
        Stmt {
            kind: StmtKind::Function(definition),
            span: range(85, 170),
            suppress_output: true,
        },
    ]);

    let module = compile(&hir).expect("call metadata should compile");
    let output_counts = module.functions[0]
        .instructions
        .iter()
        .filter_map(|instruction| match &instruction.kind {
            InstructionKind::Apply { outputs, .. } => Some(outputs.len()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(output_counts, [1, 2]);
    let function = &module.functions[1];
    assert_eq!(
        function
            .instructions
            .iter()
            .filter(|instruction| matches!(
                instruction.kind,
                InstructionKind::LoadCallInputCount { .. }
            ))
            .count(),
        1
    );
    assert_eq!(
        function
            .instructions
            .iter()
            .filter(|instruction| matches!(
                instruction.kind,
                InstructionKind::LoadCallOutputCount { .. }
            ))
            .count(),
        1
    );
    assert!(
        function
            .constants
            .iter()
            .any(|constant| matches!(constant, Constant::String(name) if name == "nargin"))
    );
}

#[test]
fn function_bindings_shadow_nargin_and_nargout_metadata() {
    let definition = FunctionDef {
        name: Some(named("shadow", 80, 86)),
        outputs: vec![named("result", 70, 76)],
        inputs: vec![named("nargin", 87, 93), named("nargout", 95, 102)],
        body: vec![assignment(
            name("result", 104, 110),
            binary(
                BinaryOp::Add,
                name("nargin", 113, 119),
                name("nargout", 122, 129),
                113,
                129,
            ),
            104,
            130,
        )],
        span: range(65, 140),
    };
    let module = compile(&file(vec![Stmt {
        kind: StmtKind::Function(definition),
        span: range(65, 140),
        suppress_output: true,
    }]))
    .expect("metadata names may be shadowed");
    let function = &module.functions[1];
    assert!(
        function
            .instructions
            .iter()
            .filter(|instruction| matches!(instruction.kind, InstructionKind::LoadLocal { .. }))
            .count()
            >= 2
    );
    assert!(!function.instructions.iter().any(|instruction| matches!(
        instruction.kind,
        InstructionKind::LoadCallInputCount { .. } | InstructionKind::LoadCallOutputCount { .. }
    )));
}

#[test]
fn lowers_named_and_all_clear_forms_for_entry_and_function_scopes() {
    let clear = Stmt {
        kind: StmtKind::Clear(ClearStatement {
            names: vec![named("first", 6, 11), named("second", 12, 18)],
            form: ClearForm::IdentifierList,
        }),
        span: range(0, 18),
        suppress_output: true,
    };
    let module = compile(&file(vec![clear.clone()])).expect("entry clear should compile");
    let entry = &module.functions[0];
    let names = entry
        .instructions
        .iter()
        .find_map(|instruction| match &instruction.kind {
            InstructionKind::ClearGlobal { names } => Some(
                names
                    .iter()
                    .map(|name| &entry.constants[name.get() as usize])
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .expect("clear instruction");
    assert_eq!(
        names,
        [
            &Constant::String("first".to_owned()),
            &Constant::String("second".to_owned())
        ]
    );

    let local_clear = FunctionDef {
        name: Some(named("local_clear", 40, 51)),
        outputs: Vec::new(),
        inputs: vec![named("first", 52, 57), named("second", 58, 64)],
        body: vec![clear],
        span: range(35, 70),
    };
    let module = compile(&file(vec![Stmt {
        kind: StmtKind::Function(local_clear),
        span: range(35, 70),
        suppress_output: true,
    }]))
    .expect("function-local named clear should compile");
    assert!(module.functions[1].instructions.iter().any(|instruction| matches!(
        &instruction.kind,
        InstructionKind::ClearLocal { locals } if locals == &[LocalSlot::new(0), LocalSlot::new(1)]
    )));

    let missing = Stmt {
        kind: StmtKind::Clear(ClearStatement {
            names: Vec::new(),
            form: ClearForm::MissingNames,
        }),
        span: range(0, 5),
        suppress_output: true,
    };
    let module = compile(&file(vec![missing])).expect("bare clear should compile");
    assert!(matches!(
        module.functions[0].instructions[0].kind,
        InstructionKind::ClearGlobalAll
    ));
}

#[test]
fn unresolved_global_calls_distinguish_assigned_and_statement_application() {
    let assigned = assignment(
        name("answer", 0, 6),
        call(name("mystery", 9, 16), vec![number("41", 17, 19)], 9, 20),
        0,
        21,
    );
    let discarded = Stmt {
        kind: StmtKind::Expr(call(name("touch", 23, 28), Vec::new(), 23, 30)),
        span: range(23, 31),
        suppress_output: true,
    };

    let module = compile(&file(vec![assigned, discarded])).expect("global calls should compile");
    let entry = &module.functions[0];
    assert!(
        entry
            .constants
            .contains(&Constant::String("mystery".to_owned()))
    );
    assert!(
        entry
            .constants
            .contains(&Constant::String("touch".to_owned()))
    );
    assert_eq!(
        entry
            .instructions
            .iter()
            .filter(|instruction| {
                matches!(instruction.kind, InstructionKind::LoadCallTarget { .. })
            })
            .count(),
        2
    );
    let output_counts: Vec<_> = entry
        .instructions
        .iter()
        .filter_map(|instruction| match &instruction.kind {
            InstructionKind::Apply { outputs, .. } => Some(outputs.len()),
            _ => None,
        })
        .collect();
    assert_eq!(output_counts, vec![1]);
    assert!(entry.instructions.iter().any(|instruction| matches!(
        instruction.kind,
        InstructionKind::StatementApply { display: false, .. }
    )));
}

#[test]
fn bare_unresolved_name_defers_implicit_class_construction_to_runtime() {
    let hir = file(vec![
        assignment(name("bare", 0, 4), name("MaybeClass", 7, 17), 0, 18),
        assignment(
            name("explicit", 20, 28),
            call(name("MaybeClass", 31, 41), Vec::new(), 31, 43),
            20,
            44,
        ),
    ]);
    let module = compile(&hir).expect("bare and explicit unresolved names should compile");
    let flags = module.functions[0]
        .instructions
        .iter()
        .filter_map(|instruction| match instruction.kind {
            InstructionKind::LoadGlobal {
                construct_if_class, ..
            } => Some(construct_if_class),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(flags, vec![true]);
    assert_eq!(
        module.functions[0]
            .instructions
            .iter()
            .filter(|instruction| {
                matches!(instruction.kind, InstructionKind::LoadCallTarget { .. })
            })
            .count(),
        1
    );
}

#[test]
fn lowers_char_literals_to_exact_utf16_constants() {
    let hir = file(vec![
        assignment(
            name("ascii", 0, 5),
            expression(ExprKind::Char("'abc'".to_owned()), 8, 13),
            0,
            14,
        ),
        assignment(
            name("bmp", 15, 18),
            expression(ExprKind::Char("'中'".to_owned()), 21, 26),
            15,
            27,
        ),
        assignment(
            name("supplementary", 28, 41),
            expression(ExprKind::Char("'🙂'".to_owned()), 44, 50),
            28,
            51,
        ),
        assignment(
            name("escaped", 52, 59),
            expression(ExprKind::Char("'don''t'".to_owned()), 62, 70),
            52,
            71,
        ),
        assignment(
            name("empty", 72, 77),
            expression(ExprKind::Char("''".to_owned()), 80, 82),
            72,
            83,
        ),
    ]);

    let module = compile(&hir).expect("char literals should compile");
    let constants = &module.functions[0].constants;
    assert!(constants.contains(&Constant::Char(vec![0x0061, 0x0062, 0x0063])));
    assert!(constants.contains(&Constant::Char(vec![0x4e2d])));
    assert!(constants.contains(&Constant::Char(vec![0xd83d, 0xde42])));
    assert!(constants.contains(&Constant::Char(vec![
        0x0064, 0x006f, 0x006e, 0x0027, 0x0074
    ])));
    assert!(constants.contains(&Constant::Char(Vec::new())));
}

#[test]
fn keeps_char_and_string_literal_constants_distinct() {
    let hir = file(vec![
        assignment(
            name("c", 0, 1),
            expression(ExprKind::Char("'same'".to_owned()), 4, 10),
            0,
            11,
        ),
        assignment(
            name("s", 12, 13),
            expression(ExprKind::String("\"same\"".to_owned()), 16, 22),
            12,
            23,
        ),
        assignment(
            name("escaped", 24, 31),
            expression(ExprKind::String("\"a\"\"b\"".to_owned()), 34, 40),
            24,
            41,
        ),
    ]);

    let module = compile(&hir).expect("char and string literals should compile");
    let constants = &module.functions[0].constants;
    assert!(constants.contains(&Constant::Char(vec![0x0073, 0x0061, 0x006d, 0x0065])));
    assert!(constants.contains(&Constant::String("same".to_owned())));
    assert!(constants.contains(&Constant::String("a\"b".to_owned())));
}

#[test]
fn invalid_and_unclosed_quoted_literals_keep_structured_diagnostics() {
    let hir = file(vec![
        assignment(
            name("invalid", 0, 7),
            expression(ExprKind::Char("'bad'quote'".to_owned()), 10, 21),
            0,
            22,
        ),
        assignment(
            name("unclosed", 23, 31),
            expression(ExprKind::String("\"unterminated".to_owned()), 34, 47),
            23,
            48,
        ),
    ]);

    let error = compile(&hir).expect_err("malformed quoted literals must be diagnosed");
    assert_eq!(error.diagnostics().len(), 2);
    assert_eq!(error.diagnostics()[0].code(), "OMC0006");
    assert_eq!(error.diagnostics()[0].range, range(10, 21));
    assert!(matches!(
        error.diagnostics()[0].kind,
        CompileDiagnosticKind::InvalidStringLiteral { .. }
    ));
    assert_eq!(error.diagnostics()[1].code(), "OMC0006");
    assert_eq!(error.diagnostics()[1].range, range(34, 47));
    assert!(matches!(
        error.diagnostics()[1].kind,
        CompileDiagnosticKind::InvalidStringLiteral { .. }
    ));
}

#[test]
fn decodes_complex_constants() {
    let hir = file(vec![assignment(
        name("z", 0, 1),
        number("2.5i", 4, 8),
        0,
        9,
    )]);

    let module = compile(&hir).expect("complex constants should compile");
    let constants = &module.functions[0].constants;
    assert!(constants.contains(&Constant::Complex {
        real: 0.0,
        imaginary: 2.5,
    }));
}

#[test]
fn compiles_scalar_unary_logical_and_left_divide_operations() {
    let negated = expression(
        ExprKind::Unary {
            operator: UnaryOp::Minus,
            operand: Box::new(number("4", 4, 5)),
        },
        3,
        5,
    );
    let logical = binary(
        BinaryOp::ShortCircuitAnd,
        number("1", 10, 11),
        expression(
            ExprKind::Unary {
                operator: UnaryOp::Not,
                operand: Box::new(number("0", 15, 16)),
            },
            14,
            16,
        ),
        10,
        16,
    );
    let division = binary(
        BinaryOp::LeftDivide,
        number("2", 22, 23),
        number("8", 26, 27),
        22,
        27,
    );
    let hir = file(vec![
        assignment(name("a", 0, 1), negated, 0, 6),
        assignment(name("b", 7, 8), logical, 7, 17),
        assignment(name("c", 19, 20), division, 19, 28),
    ]);

    let module = compile(&hir).expect("scalar operators should compile");
    let entry = &module.functions[0];
    assert!(entry.instructions.iter().any(|instruction| matches!(
        instruction.kind,
        InstructionKind::Binary {
            operator: BinaryOperator::Multiply,
            ..
        }
    )));
    assert!(entry.instructions.iter().any(|instruction| matches!(
        instruction.kind,
        InstructionKind::Binary {
            operator: BinaryOperator::LeftDivide,
            ..
        }
    )));
    assert!(
        entry
            .instructions
            .iter()
            .any(|instruction| matches!(instruction.kind, InstructionKind::JumpIfFalse { .. }))
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn preserves_operand_order_location_and_vocabulary_for_all_divisions() {
    let hir = file(vec![
        assignment(
            name("right", 0, 5),
            binary(
                BinaryOp::RightDivide,
                number("2", 10, 11),
                number("8", 14, 15),
                10,
                15,
            ),
            0,
            16,
        ),
        assignment(
            name("element_right", 17, 18),
            binary(
                BinaryOp::ElementRightDivide,
                number("3", 20, 21),
                number("12", 24, 26),
                20,
                26,
            ),
            17,
            27,
        ),
        assignment(
            name("left", 28, 29),
            binary(
                BinaryOp::LeftDivide,
                number("4", 30, 31),
                number("20", 34, 36),
                30,
                36,
            ),
            28,
            37,
        ),
        assignment(
            name("element_left", 38, 39),
            binary(
                BinaryOp::ElementLeftDivide,
                number("5", 40, 41),
                number("30", 45, 47),
                40,
                47,
            ),
            38,
            48,
        ),
    ]);

    let module = compile(&hir).expect("all four division operators should compile");
    verify(&module).expect("division bytecode should verify");
    let entry = &module.functions[0];
    let constants_by_register = entry
        .instructions
        .iter()
        .filter_map(|instruction| match instruction.kind {
            InstructionKind::LoadConstant { dst, constant } => {
                Some((dst, &entry.constants[constant.get() as usize]))
            }
            _ => None,
        })
        .collect::<std::collections::HashMap<_, _>>();
    let divisions = entry
        .instructions
        .iter()
        .filter_map(|instruction| match instruction.kind {
            InstructionKind::Binary {
                operator, lhs, rhs, ..
            } if matches!(
                operator,
                BinaryOperator::Divide
                    | BinaryOperator::ElementDivide
                    | BinaryOperator::LeftDivide
                    | BinaryOperator::ElementLeftDivide
            ) =>
            {
                Some((operator, lhs, rhs, instruction.location))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let expected = [
        (BinaryOperator::Divide, 10, 15, 2.0, 8.0),
        (BinaryOperator::ElementDivide, 20, 26, 3.0, 12.0),
        (BinaryOperator::LeftDivide, 30, 36, 4.0, 20.0),
        (BinaryOperator::ElementLeftDivide, 40, 47, 5.0, 30.0),
    ];

    assert_eq!(divisions.len(), expected.len());
    for ((operator, lhs, rhs, location), (expected_operator, start, end, left, right)) in
        divisions.iter().zip(expected)
    {
        assert_eq!(*operator, expected_operator);
        assert_eq!(
            *location,
            Some(SourceLocation::new(SOURCE.raw(), start, end))
        );
        assert_eq!(
            constants_by_register.get(lhs),
            Some(&&Constant::Double(left))
        );
        assert_eq!(
            constants_by_register.get(rhs),
            Some(&&Constant::Double(right))
        );
    }
}

#[test]
fn unary_sign_uses_signed_factors_and_multiply_lowering() {
    let positive = expression(
        ExprKind::Unary {
            operator: UnaryOp::Plus,
            operand: Box::new(number("-0", 4, 6)),
        },
        3,
        6,
    );
    let negative = expression(
        ExprKind::Unary {
            operator: UnaryOp::Minus,
            operand: Box::new(number("0", 12, 13)),
        },
        11,
        13,
    );
    let hir = file(vec![
        assignment(name("p", 0, 1), positive, 0, 7),
        assignment(name("n", 8, 9), negative, 8, 14),
    ]);

    let module = compile(&hir).expect("unary signs should compile");
    let entry = &module.functions[0];
    let factors: Vec<_> = entry
        .instructions
        .iter()
        .filter_map(|instruction| match instruction.kind {
            InstructionKind::Binary {
                operator: BinaryOperator::Multiply,
                lhs,
                ..
            } => entry
                .instructions
                .iter()
                .rev()
                .find_map(|candidate| match candidate.kind {
                    InstructionKind::LoadConstant { dst, constant } if dst == lhs => {
                        match entry.constants.get(constant.get() as usize) {
                            Some(Constant::Double(value)) => Some(*value),
                            _ => None,
                        }
                    }
                    _ => None,
                }),
            _ => None,
        })
        .collect();

    assert_eq!(factors, vec![1.0, -1.0]);
    assert_eq!(
        entry
            .instructions
            .iter()
            .filter(|instruction| matches!(
                instruction.kind,
                InstructionKind::Binary {
                    operator: BinaryOperator::Multiply,
                    ..
                }
            ))
            .count(),
        2
    );
    assert!(!entry.instructions.iter().any(|instruction| matches!(
        instruction.kind,
        InstructionKind::Binary {
            operator: BinaryOperator::Add | BinaryOperator::Subtract,
            ..
        }
    )));
}

#[test]
fn accepts_array_index_and_all_index_and_validates_class_nodes_structurally() {
    let matrix_span = range(4, 9);
    let dynamic_target_span = range(20, 21);
    let all_index_span = range(35, 36);
    let class_span = range(50, 60);
    let hir = file(vec![
        assignment(
            name("m", 0, 1),
            expression(
                ExprKind::Matrix(vec![vec![number("1", 5, 6), number("2", 7, 8)]]),
                matrix_span.start(),
                matrix_span.end(),
            ),
            0,
            10,
        ),
        assignment(name("a", 11, 12), number("1", 15, 16), 11, 17),
        assignment(
            name("x", 18, 19),
            call(
                name("a", dynamic_target_span.start(), dynamic_target_span.end()),
                vec![number("1", 22, 23)],
                20,
                24,
            ),
            18,
            25,
        ),
        assignment(
            name("y", 27, 28),
            call(
                name("unknown", 30, 34),
                vec![expression(
                    ExprKind::AllIndex,
                    all_index_span.start(),
                    all_index_span.end(),
                )],
                30,
                37,
            ),
            27,
            38,
        ),
        Stmt {
            kind: StmtKind::Expr(expression(ExprKind::Cell(Vec::new()), 40, 42)),
            span: range(40, 43),
            suppress_output: true,
        },
        Stmt {
            kind: StmtKind::Class(ClassDef {
                name: None,
                superclass: None,
                attributes: Vec::new(),
                property_blocks: Vec::new(),
                method_blocks: Vec::new(),
                enumeration_blocks: Vec::new(),
                event_blocks: Vec::new(),
                span: class_span,
            }),
            span: class_span,
            suppress_output: true,
        },
    ]);

    let error = compile(&hir).expect_err("unsupported HIR must not emit a module");
    let unsupported: Vec<_> = error
        .diagnostics()
        .iter()
        .filter_map(|diagnostic| match &diagnostic.kind {
            CompileDiagnosticKind::Unsupported { feature } => {
                Some((diagnostic.range, feature.clone()))
            }
            _ => None,
        })
        .collect();
    assert!(!unsupported.contains(&(matrix_span, UnsupportedFeature::MatrixLiteral)));
    assert!(!unsupported.contains(&(dynamic_target_span, UnsupportedFeature::DynamicApplyOrIndex)));
    assert!(!unsupported.contains(&(all_index_span, UnsupportedFeature::AllIndex)));
    assert!(!unsupported.contains(&(range(40, 42), UnsupportedFeature::CellLiteral)));
    assert!(!unsupported.contains(&(class_span, UnsupportedFeature::ClassDefinition)));
    assert!(error.diagnostics().iter().any(|diagnostic| {
        diagnostic.range == class_span
            && matches!(diagnostic.kind, CompileDiagnosticKind::MalformedHir { .. })
    }));
}

#[test]
#[allow(clippy::too_many_lines)]
fn lowers_switch_once_in_source_order_with_dedicated_matching_and_otherwise() {
    let selector_span = range(10, 18);
    let case_spans = [range(25, 26), range(51, 54), range(80, 92)];
    let hir = file(vec![Stmt {
        kind: StmtKind::Switch {
            selector: call(
                name("choose", 10, 16),
                Vec::new(),
                selector_span.start(),
                selector_span.end(),
            ),
            cases: vec![
                SwitchCase {
                    expression: number("1", case_spans[0].start(), case_spans[0].end()),
                    body: vec![assignment(
                        name("answer", 30, 36),
                        number("10", 39, 41),
                        30,
                        42,
                    )],
                    span: range(20, 45),
                },
                SwitchCase {
                    expression: expression(
                        ExprKind::Char("'x'".to_owned()),
                        case_spans[1].start(),
                        case_spans[1].end(),
                    ),
                    body: vec![assignment(
                        name("answer", 58, 64),
                        number("20", 67, 69),
                        58,
                        70,
                    )],
                    span: range(46, 72),
                },
                SwitchCase {
                    expression: expression(
                        ExprKind::String("\"case-value\"".to_owned()),
                        case_spans[2].start(),
                        case_spans[2].end(),
                    ),
                    body: vec![assignment(
                        name("answer", 96, 102),
                        number("30", 105, 107),
                        96,
                        108,
                    )],
                    span: range(73, 110),
                },
            ],
            otherwise: Some(OtherwiseBranch {
                body: vec![assignment(
                    name("answer", 120, 126),
                    number("0", 129, 130),
                    120,
                    131,
                )],
                span: range(111, 135),
            }),
        },
        span: range(5, 140),
        suppress_output: true,
    }]);

    let module = compile(&hir).expect("numeric, char, and string switch cases should compile");
    verify(&module).expect("switch lowering must produce verified bytecode");
    let entry = &module.functions[0];
    let instructions = &entry.instructions;
    let selector_applies = instructions
        .iter()
        .filter(|instruction| {
            matches!(instruction.kind, InstructionKind::Apply { .. })
                && instruction.location
                    == Some(SourceLocation::new(
                        SOURCE.raw(),
                        selector_span.start(),
                        selector_span.end(),
                    ))
        })
        .collect::<Vec<_>>();
    assert_eq!(selector_applies.len(), 1, "selector must be evaluated once");
    let InstructionKind::Apply { outputs, .. } = &selector_applies[0].kind else {
        unreachable!("filtered selector apply")
    };
    let selector = outputs[0];

    let matches = instructions
        .iter()
        .enumerate()
        .filter_map(|(index, instruction)| match instruction.kind {
            InstructionKind::SwitchMatch {
                dst,
                selector,
                case_value,
            } => Some((index, dst, selector, case_value, instruction.location)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(matches.len(), 3);
    for (ordinal, (index, matched, actual_selector, case_value, location)) in
        matches.iter().copied().enumerate()
    {
        assert_eq!(actual_selector, selector);
        assert_eq!(
            location,
            Some(SourceLocation::new(
                SOURCE.raw(),
                case_spans[ordinal].start(),
                case_spans[ordinal].end(),
            ))
        );
        assert!(matches!(
            instructions[index - 1].kind,
            InstructionKind::LoadConstant { dst, .. } if dst == case_value
        ));
        assert!(matches!(
            instructions[index + 1].kind,
            InstructionKind::JumpIfFalse { condition, .. } if condition == matched
        ));
    }

    let case_constants = matches
        .iter()
        .map(|(index, ..)| match instructions[index - 1].kind {
            InstructionKind::LoadConstant { constant, .. } => {
                &entry.constants[constant.get() as usize]
            }
            _ => unreachable!("case value load"),
        })
        .collect::<Vec<_>>();
    assert_eq!(case_constants[0], &Constant::Double(1.0));
    assert_eq!(case_constants[1], &Constant::Char(vec![u16::from(b'x')]));
    assert_eq!(
        case_constants[2],
        &Constant::String("case-value".to_owned())
    );
    assert!(!instructions.iter().any(|instruction| matches!(
        instruction.kind,
        InstructionKind::Binary {
            operator: BinaryOperator::Equal,
            ..
        }
    )));

    let false_targets = matches
        .iter()
        .map(|(index, ..)| match instructions[index + 1].kind {
            InstructionKind::JumpIfFalse { target, .. } => target.get() as usize,
            _ => unreachable!("match branch"),
        })
        .collect::<Vec<_>>();
    assert_eq!(false_targets[0], matches[1].0 - 1);
    assert_eq!(false_targets[1], matches[2].0 - 1);
    let end_targets = instructions
        .iter()
        .filter_map(|instruction| match instruction.kind {
            InstructionKind::Jump { target } => Some(target.get() as usize),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(end_targets.len(), 3);
    assert!(end_targets.iter().all(|target| *target == end_targets[0]));
    assert!(false_targets[2] < end_targets[0]);
}

#[test]
fn switch_selector_and_cases_preserve_unresolved_apply_or_index_ambiguity() {
    let selector_span = range(5, 15);
    let first_case_span = range(25, 35);
    let second_case_span = range(45, 56);
    let hir = file(vec![Stmt {
        kind: StmtKind::Switch {
            selector: call(
                name("selector", 5, 13),
                Vec::new(),
                selector_span.start(),
                selector_span.end(),
            ),
            cases: vec![
                SwitchCase {
                    expression: call(
                        name("firstCase", 25, 34),
                        Vec::new(),
                        first_case_span.start(),
                        first_case_span.end(),
                    ),
                    body: Vec::new(),
                    span: range(20, 38),
                },
                SwitchCase {
                    expression: call(
                        name("secondCase", 45, 55),
                        Vec::new(),
                        second_case_span.start(),
                        second_case_span.end(),
                    ),
                    body: Vec::new(),
                    span: range(40, 60),
                },
            ],
            otherwise: None,
        },
        span: range(0, 65),
        suppress_output: true,
    }]);

    let module = compile(&hir).expect("unresolved switch applications should compile");
    let instructions = &module.functions[0].instructions;
    let apply_indices = [selector_span, first_case_span, second_case_span].map(|span| {
        instructions
            .iter()
            .position(|instruction| {
                instruction.location
                    == Some(SourceLocation::new(SOURCE.raw(), span.start(), span.end()))
                    && matches!(instruction.kind, InstructionKind::Apply { .. })
            })
            .expect("source-located unresolved apply")
    });
    assert!(apply_indices[0] < apply_indices[1] && apply_indices[1] < apply_indices[2]);
    let first_match = instructions
        .iter()
        .position(|instruction| {
            instruction.location
                == Some(SourceLocation::new(
                    SOURCE.raw(),
                    first_case_span.start(),
                    first_case_span.end(),
                ))
                && matches!(instruction.kind, InstructionKind::SwitchMatch { .. })
        })
        .expect("first case match");
    assert!(matches!(
        instructions[first_match + 1].kind,
        InstructionKind::JumpIfFalse { target, .. }
            if target.get() as usize <= apply_indices[2]
    ));
    assert!(matches!(
        instructions[first_match + 2].kind,
        InstructionKind::Jump { target }
            if target.get() as usize > apply_indices[2]
    ));
}

#[test]
#[allow(clippy::too_many_lines)]
fn patches_empty_and_nested_switch_control_flow() {
    let inner_case_span = range(75, 76);
    let inner_case_clause_span = range(72, 82);
    let outer_second_clause_span = range(60, 95);
    let nested = Stmt {
        kind: StmtKind::Switch {
            selector: name("inner", 68, 73),
            cases: vec![SwitchCase {
                expression: number("3", inner_case_span.start(), inner_case_span.end()),
                body: Vec::new(),
                span: inner_case_clause_span,
            }],
            otherwise: Some(OtherwiseBranch {
                body: Vec::new(),
                span: range(83, 88),
            }),
        },
        span: range(65, 90),
        suppress_output: true,
    };
    let hir = file(vec![
        Stmt {
            kind: StmtKind::Switch {
                selector: number("0", 2, 3),
                cases: Vec::new(),
                otherwise: None,
            },
            span: range(0, 8),
            suppress_output: true,
        },
        Stmt {
            kind: StmtKind::Switch {
                selector: name("outer", 15, 20),
                cases: vec![
                    SwitchCase {
                        expression: number("1", 27, 28),
                        body: Vec::new(),
                        span: range(22, 35),
                    },
                    SwitchCase {
                        expression: number("2", 42, 43),
                        body: vec![nested],
                        span: outer_second_clause_span,
                    },
                ],
                otherwise: None,
            },
            span: range(10, 100),
            suppress_output: true,
        },
    ]);

    let module = compile(&hir).expect("empty and nested switches should compile");
    verify(&module).expect("all nested switch patches must be in range");
    let instructions = &module.functions[0].instructions;
    assert_eq!(
        instructions
            .iter()
            .filter(|instruction| matches!(instruction.kind, InstructionKind::SwitchMatch { .. }))
            .count(),
        3
    );

    let first_outer_match = instructions
        .iter()
        .position(|instruction| {
            matches!(instruction.kind, InstructionKind::SwitchMatch { .. })
                && instruction.location == Some(SourceLocation::new(SOURCE.raw(), 27, 28))
        })
        .expect("first outer match");
    assert!(matches!(
        instructions[first_outer_match + 1].kind,
        InstructionKind::JumpIfFalse { .. }
    ));
    assert!(matches!(
        instructions[first_outer_match + 2].kind,
        InstructionKind::Jump { .. }
    ));

    let inner_end_target = instructions
        .iter()
        .find_map(|instruction| {
            (instruction.location
                == Some(SourceLocation::new(
                    SOURCE.raw(),
                    inner_case_clause_span.start(),
                    inner_case_clause_span.end(),
                )))
            .then(|| match instruction.kind {
                InstructionKind::Jump { target } => Some(target.get() as usize),
                _ => None,
            })
            .flatten()
        })
        .expect("inner case end jump");
    let outer_second_end = instructions
        .iter()
        .position(|instruction| {
            instruction.location
                == Some(SourceLocation::new(
                    SOURCE.raw(),
                    outer_second_clause_span.start(),
                    outer_second_clause_span.end(),
                ))
                && matches!(instruction.kind, InstructionKind::Jump { .. })
        })
        .expect("outer second case end jump");
    assert_eq!(inner_end_target, outer_second_end);
}

#[test]
#[allow(clippy::too_many_lines)]
fn switch_inside_loop_preserves_break_continue_and_return_targets() {
    let condition_span = range(5, 6);
    let break_span = range(35, 40);
    let continue_span = range(55, 63);
    let return_span = range(78, 84);
    let switch = Stmt {
        kind: StmtKind::Switch {
            selector: name("choice", 15, 21),
            cases: vec![
                SwitchCase {
                    expression: number("1", 28, 29),
                    body: vec![Stmt {
                        kind: StmtKind::Break,
                        span: break_span,
                        suppress_output: true,
                    }],
                    span: range(24, 42),
                },
                SwitchCase {
                    expression: number("2", 48, 49),
                    body: vec![Stmt {
                        kind: StmtKind::Continue,
                        span: continue_span,
                        suppress_output: true,
                    }],
                    span: range(44, 65),
                },
            ],
            otherwise: Some(OtherwiseBranch {
                body: vec![Stmt {
                    kind: StmtKind::Return,
                    span: return_span,
                    suppress_output: true,
                }],
                span: range(67, 86),
            }),
        },
        span: range(10, 90),
        suppress_output: true,
    };
    let hir = file(vec![Stmt {
        kind: StmtKind::While {
            condition: number("1", condition_span.start(), condition_span.end()),
            body: vec![switch],
        },
        span: range(0, 95),
        suppress_output: true,
    }]);

    let module = compile(&hir).expect("switch loop control should compile");
    verify(&module).expect("switch loop control targets must verify");
    let instructions = &module.functions[0].instructions;
    let located_jump = |span: TextRange| {
        instructions
            .iter()
            .find_map(|instruction| {
                (instruction.location
                    == Some(SourceLocation::new(SOURCE.raw(), span.start(), span.end())))
                .then(|| match instruction.kind {
                    InstructionKind::Jump { target } => Some(target.get() as usize),
                    _ => None,
                })
                .flatten()
            })
            .expect("located loop-control jump")
    };
    let loop_start = instructions
        .iter()
        .position(|instruction| {
            instruction.location
                == Some(SourceLocation::new(
                    SOURCE.raw(),
                    condition_span.start(),
                    condition_span.end(),
                ))
                && matches!(instruction.kind, InstructionKind::LoadConstant { .. })
        })
        .expect("while condition start");
    let loop_exit = instructions
        .iter()
        .find_map(|instruction| match instruction.kind {
            InstructionKind::JumpIfFalse { target, .. }
                if instruction.location
                    == Some(SourceLocation::new(
                        SOURCE.raw(),
                        condition_span.start(),
                        condition_span.end(),
                    )) =>
            {
                Some(target.get() as usize)
            }
            _ => None,
        })
        .expect("while exit");
    assert_eq!(located_jump(continue_span), loop_start);
    assert_eq!(located_jump(break_span), loop_exit);
    assert!(instructions.iter().any(|instruction| {
        instruction.location
            == Some(SourceLocation::new(
                SOURCE.raw(),
                return_span.start(),
                return_span.end(),
            ))
            && matches!(instruction.kind, InstructionKind::Return { .. })
    }));
}

#[test]
fn switch_assignments_are_allocated_as_function_locals() {
    let definition = FunctionDef {
        name: Some(named("select", 0, 6)),
        outputs: vec![named("out", 7, 10)],
        inputs: vec![named("choice", 11, 17)],
        body: vec![
            Stmt {
                kind: StmtKind::Switch {
                    selector: name("choice", 25, 31),
                    cases: vec![SwitchCase {
                        expression: number("1", 38, 39),
                        body: vec![assignment(
                            name("temporary", 43, 52),
                            number("10", 55, 57),
                            43,
                            58,
                        )],
                        span: range(34, 60),
                    }],
                    otherwise: Some(OtherwiseBranch {
                        body: vec![assignment(
                            name("temporary", 70, 79),
                            number("0", 82, 83),
                            70,
                            84,
                        )],
                        span: range(62, 86),
                    }),
                },
                span: range(20, 90),
                suppress_output: true,
            },
            assignment(name("out", 92, 95), name("temporary", 98, 107), 92, 108),
        ],
        span: range(0, 115),
    };

    let module = compile(&file(vec![Stmt {
        kind: StmtKind::Function(definition),
        span: range(0, 115),
        suppress_output: true,
    }]))
    .expect("switch body locals should be discovered before lowering");
    verify(&module).expect("function switch bytecode should verify");
    let function = &module.functions[1];
    assert_eq!(function.local_count, 3);
    assert!(
        function
            .instructions
            .iter()
            .any(|instruction| matches!(instruction.kind, InstructionKind::SwitchMatch { .. }))
    );
    assert!(function.instructions.iter().any(|instruction| matches!(
        instruction.kind,
        InstructionKind::StoreLocal { local, .. }
            if local == openmat_bytecode::LocalSlot::new(2)
    )));
}

#[test]
fn class_method_switch_preserves_auxiliary_and_anonymous_function_ids() {
    let method_switch = Stmt {
        kind: StmtKind::Switch {
            selector: name("choice", 80, 86),
            cases: vec![SwitchCase {
                expression: number("1", 93, 94),
                body: vec![assignment(
                    name("result", 98, 104),
                    anonymous(vec![named("x", 108, 109)], name("x", 111, 112), 106, 112),
                    98,
                    113,
                )],
                span: range(89, 115),
            }],
            otherwise: Some(OtherwiseBranch {
                body: vec![assignment(
                    name("result", 123, 129),
                    number("0", 132, 133),
                    123,
                    134,
                )],
                span: range(117, 136),
            }),
        },
        span: range(75, 140),
        suppress_output: true,
    };
    let class = ClassDef {
        name: Some(named("Chooser", 0, 7)),
        superclass: None,
        enumeration_blocks: Vec::new(),
        event_blocks: Vec::new(),
        attributes: Vec::new(),
        property_blocks: vec![PropertyBlock {
            attributes: Vec::new(),
            properties: vec![PropertyDef {
                name: Some(named("Seed", 15, 19)),
                default: Some(number("7", 22, 23)),
                span: range(15, 23),
            }],
            span: range(10, 28),
        }],
        method_blocks: vec![MethodBlock {
            attributes: Vec::new(),
            methods: vec![FunctionDef {
                name: Some(named("select", 35, 41)),
                outputs: vec![named("result", 42, 48)],
                inputs: vec![named("choice", 49, 55)],
                body: vec![method_switch],
                span: range(35, 145),
            }],
            declarations: Vec::new(),
            span: range(30, 150),
        }],
        span: range(0, 155),
    };

    let module = compile(&file(vec![Stmt {
        kind: StmtKind::Class(class),
        span: range(0, 155),
        suppress_output: true,
    }]))
    .expect("class method switch with anonymous body should compile");
    verify(&module).expect("class switch module should verify");
    assert_eq!(module.functions.len(), 4);
    assert_eq!(
        module.classes[0].properties[0].default,
        Some(FunctionId::new(1))
    );
    assert_eq!(module.classes[0].methods[0].function, FunctionId::new(2));
    assert!(
        module.functions[2]
            .instructions
            .iter()
            .any(|instruction| matches!(
                &instruction.kind,
                InstructionKind::MakeClosure { function, .. } if *function == FunctionId::new(3)
            ))
    );
    assert!(
        module.functions[2]
            .instructions
            .iter()
            .any(|instruction| matches!(instruction.kind, InstructionKind::SwitchMatch { .. }))
    );
}

#[test]
fn malformed_and_cell_switch_cases_keep_structured_diagnostics() {
    let cell_span = range(45, 51);
    let malformed_case_span = range(60, 61);
    let hir = file(vec![
        Stmt {
            kind: StmtKind::Switch {
                selector: expression(ExprKind::Error, 5, 6),
                cases: Vec::new(),
                otherwise: None,
            },
            span: range(0, 10),
            suppress_output: true,
        },
        Stmt {
            kind: StmtKind::Switch {
                selector: number("1", 25, 26),
                cases: vec![
                    SwitchCase {
                        expression: expression(
                            ExprKind::Cell(vec![vec![number("1", 46, 47), number("2", 49, 50)]]),
                            cell_span.start(),
                            cell_span.end(),
                        ),
                        body: Vec::new(),
                        span: range(40, 53),
                    },
                    SwitchCase {
                        expression: expression(
                            ExprKind::Error,
                            malformed_case_span.start(),
                            malformed_case_span.end(),
                        ),
                        body: Vec::new(),
                        span: range(55, 65),
                    },
                ],
                otherwise: None,
            },
            span: range(20, 70),
            suppress_output: true,
        },
    ]);

    let error = compile(&hir).expect_err("malformed and cell cases must not emit bytecode");
    assert!(error.diagnostics().iter().any(|diagnostic| {
        diagnostic.range == range(5, 6)
            && matches!(diagnostic.kind, CompileDiagnosticKind::MalformedHir { .. })
    }));
    assert!(!error.diagnostics().iter().any(|diagnostic| {
        diagnostic.range == cell_span
            && matches!(
                diagnostic.kind,
                CompileDiagnosticKind::Unsupported {
                    feature: UnsupportedFeature::CellLiteral
                }
            )
    }));
    assert!(error.diagnostics().iter().any(|diagnostic| {
        diagnostic.range == malformed_case_span
            && matches!(diagnostic.kind, CompileDiagnosticKind::MalformedHir { .. })
    }));
    assert!(!error.diagnostics().iter().any(|diagnostic| matches!(
        diagnostic.kind,
        CompileDiagnosticKind::Unsupported {
            feature: UnsupportedFeature::SwitchStatement
        }
    )));
}

#[test]
fn compiles_matrix_range_and_transpose_to_located_verified_instructions() {
    let matrix_span = range(4, 17);
    let range_span = range(24, 29);
    let transpose_span = range(36, 38);
    let matrix = expression(
        ExprKind::Matrix(vec![
            vec![number("1", 5, 6), number("2", 7, 8)],
            vec![number("3", 10, 11), number("4", 12, 13)],
        ]),
        matrix_span.start(),
        matrix_span.end(),
    );
    let range_expression = expression(
        ExprKind::Range {
            start: Box::new(number("5", 24, 25)),
            step: Some(Box::new(number("-2", 26, 28))),
            end: Box::new(number("1", 28, 29)),
        },
        range_span.start(),
        range_span.end(),
    );
    let transpose = expression(
        ExprKind::Transpose {
            kind: TransposeKind::Conjugate,
            operand: Box::new(name("m", 36, 37)),
        },
        transpose_span.start(),
        transpose_span.end(),
    );
    let hir = file(vec![
        assignment(
            name("empty", 0, 1),
            expression(ExprKind::Matrix(Vec::new()), 2, 4),
            0,
            4,
        ),
        assignment(name("m", 18, 19), matrix, 18, 20),
        assignment(name("r", 21, 22), range_expression, 21, 30),
        assignment(name("t", 32, 33), transpose, 32, 39),
    ]);

    let module = compile(&hir).expect("array expressions should compile");
    verify(&module).expect("array expression bytecode should verify");
    let instructions = &module.functions[0].instructions;
    assert_eq!(
        instructions
            .iter()
            .filter(|instruction| matches!(instruction.kind, InstructionKind::BuildMatrix { .. }))
            .count(),
        2
    );
    assert!(instructions.iter().any(|instruction| {
        matches!(instruction.kind, InstructionKind::Range { .. })
            && instruction.location
                == Some(SourceLocation::new(
                    SOURCE.raw(),
                    range_span.start(),
                    range_span.end(),
                ))
    }));
    assert!(instructions.iter().any(|instruction| matches!(
        instruction.kind,
        InstructionKind::Transpose {
            conjugate: true,
            ..
        }
    )));
}

#[test]
fn compiles_workspace_indexing_without_temporary_array_copies() {
    let matrix = expression(
        ExprKind::Matrix(vec![vec![number("1", 4, 5), number("2", 6, 7)]]),
        3,
        8,
    );
    let indexed = call(
        name("a", 12, 13),
        vec![expression(ExprKind::AllIndex, 14, 15), number("1", 16, 17)],
        12,
        18,
    );
    let indexed_target = call(name("a", 24, 25), vec![number("1", 26, 27)], 24, 28);
    let hir = file(vec![
        assignment(name("a", 0, 1), matrix, 0, 9),
        assignment(name("selected", 10, 11), indexed, 10, 19),
        assignment(indexed_target, number("9", 31, 32), 24, 33),
    ]);

    let module = compile(&hir).expect("dynamic indexing should compile");
    verify(&module).expect("dynamic indexing bytecode should verify");
    let instructions = &module.functions[0].instructions;
    assert!(instructions.iter().any(|instruction| matches!(
        &instruction.kind,
        InstructionKind::ApplyBinding { arguments, .. }
            if matches!(arguments.as_slice(), [ApplyArgument::Colon, ApplyArgument::Value(_)])
    )));
    assert!(instructions.iter().any(|instruction| matches!(
        &instruction.kind,
        InstructionKind::AssignBindingPlace {
            result: None,
            path,
            source: ValueSource::One(_),
            mode: AssignmentMode::Store,
            ..
        } if matches!(path.as_slice(), [PlaceStep::Paren(arguments)]
            if matches!(arguments.as_slice(), [ApplyArgument::Value(_)]))
    )));
    assert_eq!(
        instructions
            .iter()
            .filter(|instruction| matches!(
                instruction.kind,
                InstructionKind::LoadGlobalOrNothing { .. }
            ))
            .count(),
        1,
        "the write must capture one shallow root snapshot after its selector"
    );
    assert!(
        !instructions
            .iter()
            .any(|instruction| matches!(instruction.kind, InstructionKind::AssignPlace { .. }))
    );
}

#[test]
fn distinguishes_matrix_and_elementwise_binary_operators() {
    let operands = || {
        (
            expression(ExprKind::Matrix(vec![vec![number("2", 4, 5)]]), 3, 6),
            expression(ExprKind::Matrix(vec![vec![number("3", 9, 10)]]), 8, 11),
        )
    };
    let (left, right) = operands();
    let matrix = binary(BinaryOp::Multiply, left, right, 3, 11);
    let (left, right) = operands();
    let element = binary(BinaryOp::ElementMultiply, left, right, 3, 11);
    let hir = file(vec![
        assignment(name("matrix", 0, 1), matrix, 0, 12),
        assignment(name("element", 13, 14), element, 13, 25),
    ]);

    let module = compile(&hir).expect("binary array operators should compile");
    let instructions = &module.functions[0].instructions;
    assert!(instructions.iter().any(|instruction| matches!(
        instruction.kind,
        InstructionKind::Binary {
            operator: BinaryOperator::Multiply,
            ..
        }
    )));
    assert!(instructions.iter().any(|instruction| matches!(
        instruction.kind,
        InstructionKind::Binary {
            operator: BinaryOperator::ElementMultiply,
            ..
        }
    )));
}

#[test]
fn compiles_for_iteration_with_verified_exit_and_loop_control() {
    let iterable = expression(
        ExprKind::Range {
            start: Box::new(number("1", 8, 9)),
            step: None,
            end: Box::new(number("3", 10, 11)),
        },
        8,
        11,
    );
    let statement = Stmt {
        kind: StmtKind::For {
            variable: name("i", 4, 5),
            iterable,
            body: vec![Stmt {
                kind: StmtKind::Continue,
                span: range(14, 22),
                suppress_output: true,
            }],
        },
        span: range(0, 26),
        suppress_output: true,
    };

    let module = compile(&file(vec![statement])).expect("for loop should compile");
    verify(&module).expect("for loop bytecode should verify");
    let instructions = &module.functions[0].instructions;
    assert!(
        instructions
            .iter()
            .enumerate()
            .any(|(index, instruction)| match instruction.kind {
                InstructionKind::ForEach { exit, .. } => exit.get() as usize > index,
                _ => false,
            })
    );
    assert!(
        instructions
            .iter()
            .enumerate()
            .any(|(index, instruction)| match instruction.kind {
                InstructionKind::Jump { target } => target.get() as usize <= index,
                _ => false,
            })
    );
}

#[test]
fn malformed_hir_returns_stable_diagnostics_without_panicking() {
    let missing_function = FunctionDef {
        name: None,
        outputs: Vec::new(),
        inputs: Vec::new(),
        body: Vec::new(),
        span: range(30, 40),
    };
    let hir = file(vec![
        Stmt {
            kind: StmtKind::Error,
            span: range(0, 3),
            suppress_output: true,
        },
        assignment(name("x", 4, 5), number("12e", 8, 11), 4, 12),
        Stmt {
            kind: StmtKind::Break,
            span: range(14, 19),
            suppress_output: true,
        },
        Stmt {
            kind: StmtKind::Expr(expression(ExprKind::Error, 21, 22)),
            span: range(21, 23),
            suppress_output: true,
        },
        Stmt {
            kind: StmtKind::Function(missing_function),
            span: range(30, 40),
            suppress_output: true,
        },
    ]);

    let error = compile(&hir).expect_err("malformed HIR must be diagnosed");
    assert!(error.diagnostics().len() >= 5);
    assert!(
        error
            .diagnostics()
            .iter()
            .all(|diagnostic| diagnostic.source_id == SOURCE)
    );
    assert!(error.diagnostics().iter().any(|diagnostic| matches!(
        diagnostic.kind,
        CompileDiagnosticKind::InvalidNumericLiteral { .. }
    )));
    assert!(error.diagnostics().iter().any(|diagnostic| matches!(
        diagnostic.kind,
        CompileDiagnosticKind::LoopControlOutsideLoop { keyword: "break" }
    )));
    let shared = error.diagnostics()[0].to_source_diagnostic();
    assert_eq!(shared.source_id, SOURCE);
    assert_eq!(shared.code.as_deref(), Some(error.diagnostics()[0].code()));
}

#[test]
fn duplicate_function_parameter_and_output_names_are_deterministic_errors() {
    let duplicate_one = FunctionDef {
        name: Some(named("same", 0, 4)),
        outputs: vec![named("out", 5, 8), named("out", 9, 12)],
        inputs: vec![named("x", 13, 14), named("x", 15, 16)],
        body: Vec::new(),
        span: range(0, 30),
    };
    let duplicate_two = FunctionDef {
        name: Some(named("same", 40, 44)),
        outputs: Vec::new(),
        inputs: Vec::new(),
        body: Vec::new(),
        span: range(35, 50),
    };
    let hir = file(vec![
        Stmt {
            kind: StmtKind::Function(duplicate_one),
            span: range(0, 30),
            suppress_output: true,
        },
        Stmt {
            kind: StmtKind::Function(duplicate_two),
            span: range(35, 50),
            suppress_output: true,
        },
    ]);

    let first = compile(&hir).expect_err("duplicates should fail");
    let second = compile(&hir).expect_err("duplicates should fail deterministically");
    assert_eq!(first, second);
    assert!(first.diagnostics().iter().any(|diagnostic| matches!(
        diagnostic.kind,
        CompileDiagnosticKind::DuplicateFunction { .. }
    )));
    assert!(first.diagnostics().iter().any(|diagnostic| matches!(
        diagnostic.kind,
        CompileDiagnosticKind::DuplicateParameter { .. }
    )));
    assert!(first.diagnostics().iter().any(|diagnostic| matches!(
        diagnostic.kind,
        CompileDiagnosticKind::DuplicateOutput { .. }
    )));
}

#[test]
fn successful_compilation_is_byte_for_byte_deterministic_in_memory() {
    let hir = file(vec![assignment(
        name("answer", 0, 6),
        binary(
            BinaryOp::Multiply,
            number("6", 9, 10),
            number("7", 13, 14),
            9,
            14,
        ),
        0,
        15,
    )]);

    let first = compile(&hir).expect("first compilation");
    let second = compile(&hir).expect("second compilation");
    assert_eq!(first, second);
}

#[test]
fn dependent_and_dotted_accessor_compile_to_read_only_metadata_and_code() {
    let class = ClassDef {
        name: Some(named("Box", 0, 3)),
        superclass: None,
        enumeration_blocks: Vec::new(),
        event_blocks: Vec::new(),
        attributes: Vec::new(),
        property_blocks: vec![PropertyBlock {
            attributes: vec![Attribute {
                name: Some(named("Dependent", 10, 19)),
                value: None,
                span: range(10, 19),
            }],
            properties: vec![PropertyDef {
                name: Some(named("Twice", 20, 25)),
                default: None,
                span: range(20, 25),
            }],
            span: range(8, 30),
        }],
        method_blocks: vec![MethodBlock {
            attributes: Vec::new(),
            methods: vec![FunctionDef {
                name: Some(named("get.Twice", 40, 49)),
                outputs: vec![named("value", 32, 37)],
                inputs: vec![named("obj", 50, 53)],
                body: Vec::new(),
                span: range(32, 60),
            }],
            declarations: Vec::new(),
            span: range(31, 65),
        }],
        span: range(0, 70),
    };
    let module = compile(&file(vec![Stmt {
        kind: StmtKind::Class(class),
        span: range(0, 70),
        suppress_output: true,
    }]))
    .expect("dependent getter should compile");

    verify(&module).expect("dependent bytecode should verify");
    let property = &module.classes[0].properties[0];
    assert_eq!(property.kind, BytecodePropertyKind::Dependent);
    assert_eq!(property.get_access, Some(BytecodeAccess::Public));
    assert_eq!(property.set_access, None);
    assert_eq!(module.classes[0].methods[0].name, "get.Twice");
    assert_eq!(
        module.classes[0].methods[0].kind,
        BytecodeMethodKind::Instance
    );
}

#[test]
fn invalid_dependent_accessor_signature_is_structured() {
    let method_span = range(30, 60);
    let class = ClassDef {
        name: Some(named("Box", 0, 3)),
        superclass: None,
        enumeration_blocks: Vec::new(),
        event_blocks: Vec::new(),
        attributes: Vec::new(),
        property_blocks: vec![PropertyBlock {
            attributes: vec![Attribute {
                name: Some(named("Dependent", 10, 19)),
                value: None,
                span: range(10, 19),
            }],
            properties: vec![PropertyDef {
                name: Some(named("Twice", 20, 25)),
                default: None,
                span: range(20, 25),
            }],
            span: range(8, 28),
        }],
        method_blocks: vec![MethodBlock {
            attributes: Vec::new(),
            methods: vec![FunctionDef {
                name: Some(named("set.Twice", 32, 41)),
                outputs: Vec::new(),
                inputs: vec![named("obj", 42, 45)],
                body: Vec::new(),
                span: method_span,
            }],
            declarations: Vec::new(),
            span: range(29, 65),
        }],
        span: range(0, 70),
    };
    let error = compile(&file(vec![Stmt {
        kind: StmtKind::Class(class),
        span: range(0, 70),
        suppress_output: true,
    }]))
    .expect_err("bad setter signature must fail");

    assert!(error.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "OMC0010"
            && diagnostic.range == method_span
            && matches!(
                &diagnostic.kind,
                CompileDiagnosticKind::InvalidClassMember {
                    member,
                    problem: ClassMemberProblem::InvalidSignature {
                        expected_inputs: 2,
                        actual_inputs: 1,
                        expected_outputs: 1,
                        actual_outputs: 0,
                    },
                    ..
                } if member == "set.Twice"
            )
    }));
}

#[test]
fn inherited_dependent_accessor_override_compiles_without_redeclaring_property() {
    let class = ClassDef {
        name: Some(named("ChildBox", 0, 8)),
        superclass: Some(named("BaseBox", 11, 18)),
        enumeration_blocks: Vec::new(),
        event_blocks: Vec::new(),
        attributes: Vec::new(),
        property_blocks: Vec::new(),
        method_blocks: vec![MethodBlock {
            attributes: Vec::new(),
            methods: vec![FunctionDef {
                name: Some(named("get.Twice", 30, 39)),
                outputs: vec![named("value", 20, 25)],
                inputs: vec![named("obj", 40, 43)],
                body: Vec::new(),
                span: range(20, 50),
            }],
            declarations: Vec::new(),
            span: range(19, 55),
        }],
        span: range(0, 60),
    };
    let module = compile(&file(vec![Stmt {
        kind: StmtKind::Class(class),
        span: range(0, 60),
        suppress_output: true,
    }]))
    .expect("inherited dependent accessor override should compile");

    verify(&module).expect("inherited accessor bytecode should verify");
    assert_eq!(module.classes[0].superclass.as_deref(), Some("BaseBox"));
    assert!(module.classes[0].properties.is_empty());
    assert_eq!(module.classes[0].methods[0].name, "get.Twice");
}

#[test]
fn plus_operator_method_compiles_as_executable_instance_code() {
    let plus = FunctionDef {
        name: Some(named("plus", 20, 24)),
        outputs: vec![named("result", 25, 31)],
        inputs: vec![named("left", 32, 36), named("right", 38, 43)],
        body: vec![assignment(
            name("result", 45, 51),
            number("12", 54, 56),
            45,
            57,
        )],
        span: range(20, 60),
    };
    let class = ClassDef {
        name: Some(named("Addend", 0, 6)),
        superclass: None,
        enumeration_blocks: Vec::new(),
        event_blocks: Vec::new(),
        attributes: Vec::new(),
        property_blocks: Vec::new(),
        method_blocks: vec![MethodBlock {
            attributes: Vec::new(),
            methods: vec![plus],
            declarations: Vec::new(),
            span: range(15, 65),
        }],
        span: range(0, 70),
    };
    let module = compile(&file(vec![Stmt {
        kind: StmtKind::Class(class),
        span: range(0, 70),
        suppress_output: true,
    }]))
    .expect("plus overload should compile");

    verify(&module).expect("plus bytecode should verify");
    assert_eq!(module.classes[0].methods[0].name, "plus");
    assert_eq!(
        module.classes[0].methods[0].kind,
        BytecodeMethodKind::Instance
    );
    let function = &module.functions[module.classes[0].methods[0].function.get() as usize];
    assert_eq!(function.parameter_count, 2);
}

#[test]
fn non_plus_binary_operator_method_compiles_as_executable_instance_code() {
    let minus = FunctionDef {
        name: Some(named("minus", 20, 25)),
        outputs: vec![named("result", 26, 32)],
        inputs: vec![named("left", 33, 37), named("right", 39, 44)],
        body: vec![assignment(
            name("result", 46, 52),
            number("2", 55, 56),
            46,
            57,
        )],
        span: range(20, 60),
    };
    let class = ClassDef {
        name: Some(named("Subtractend", 0, 10)),
        superclass: None,
        enumeration_blocks: Vec::new(),
        event_blocks: Vec::new(),
        attributes: Vec::new(),
        property_blocks: Vec::new(),
        method_blocks: vec![MethodBlock {
            attributes: Vec::new(),
            methods: vec![minus],
            declarations: Vec::new(),
            span: range(15, 65),
        }],
        span: range(0, 70),
    };
    let module = compile(&file(vec![Stmt {
        kind: StmtKind::Class(class),
        span: range(0, 70),
        suppress_output: true,
    }]))
    .expect("minus overload should compile");

    verify(&module).expect("minus bytecode should verify");
    assert_eq!(module.classes[0].methods[0].name, "minus");
    assert_eq!(
        module.classes[0].methods[0].kind,
        BytecodeMethodKind::Instance
    );
    let function = &module.functions[module.classes[0].methods[0].function.get() as usize];
    assert_eq!(function.parameter_count, 2);
}

#[test]
fn compiles_public_value_class_to_verified_class_and_field_bytecode() {
    let field = expression(
        ExprKind::Field {
            target: Box::new(name("obj", 50, 53)),
            name: Some(named("x", 54, 55)),
        },
        50,
        55,
    );
    let class = ClassDef {
        name: Some(named("Point", 0, 5)),
        superclass: None,
        enumeration_blocks: Vec::new(),
        event_blocks: Vec::new(),
        attributes: Vec::new(),
        property_blocks: vec![PropertyBlock {
            attributes: Vec::new(),
            properties: vec![PropertyDef {
                name: Some(named("x", 10, 11)),
                default: Some(number("1", 14, 15)),
                span: range(10, 15),
            }],
            span: range(8, 20),
        }],
        method_blocks: vec![MethodBlock {
            attributes: Vec::new(),
            methods: vec![FunctionDef {
                name: Some(named("read", 30, 34)),
                outputs: vec![named("value", 35, 40)],
                inputs: vec![named("obj", 41, 44)],
                body: vec![assignment(name("value", 45, 50), field, 45, 56)],
                span: range(30, 60),
            }],
            declarations: Vec::new(),
            span: range(25, 65),
        }],
        span: range(0, 70),
    };
    let module = compile(&file(vec![Stmt {
        kind: StmtKind::Class(class),
        span: range(0, 70),
        suppress_output: true,
    }]))
    .expect("public value class should compile");

    verify(&module).expect("class bytecode should verify");
    assert_eq!(module.classes.len(), 1);
    assert_eq!(module.classes[0].name, "Point");
    assert_eq!(module.classes[0].properties.len(), 1);
    assert_eq!(module.classes[0].methods.len(), 1);
    assert_eq!(
        module.classes[0].properties[0].get_access,
        Some(BytecodeAccess::Public)
    );
    assert_eq!(
        module.classes[0].properties[0].set_access,
        Some(BytecodeAccess::Public)
    );
    assert_eq!(module.classes[0].methods[0].access, BytecodeAccess::Public);
    assert!(
        module.functions[0]
            .instructions
            .iter()
            .any(|instruction| matches!(instruction.kind, InstructionKind::RegisterClass { .. }))
    );
    let method = &module.functions[module.classes[0].methods[0].function.get() as usize];
    assert!(
        method.instructions.iter().any(|instruction| matches!(
            instruction.kind,
            InstructionKind::GetAggregateField { .. }
        ))
    );
    assert!(
        method
            .instructions
            .iter()
            .any(|instruction| matches!(instruction.kind, InstructionKind::Unpack { .. }))
    );
    assert!(
        !method
            .instructions
            .iter()
            .any(|instruction| matches!(instruction.kind, InstructionKind::GetField { .. }))
    );
}

#[test]
fn compiles_member_access_and_direct_superclass_metadata() {
    let class = ClassDef {
        name: Some(named("Child", 0, 5)),
        superclass: Some(named("Base", 8, 12)),
        enumeration_blocks: Vec::new(),
        event_blocks: Vec::new(),
        attributes: Vec::new(),
        property_blocks: vec![
            PropertyBlock {
                attributes: vec![Attribute {
                    name: Some(named("Access", 20, 26)),
                    value: Some(name("private", 29, 36)),
                    span: range(20, 36),
                }],
                properties: vec![PropertyDef {
                    name: Some(named("secret", 40, 46)),
                    default: None,
                    span: range(40, 46),
                }],
                span: range(18, 50),
            },
            PropertyBlock {
                attributes: vec![
                    Attribute {
                        name: Some(named("GetAccess", 106, 115)),
                        value: Some(name("protected", 118, 127)),
                        span: range(106, 127),
                    },
                    Attribute {
                        name: Some(named("SetAccess", 129, 138)),
                        value: Some(name("private", 141, 148)),
                        span: range(129, 148),
                    },
                ],
                properties: vec![PropertyDef {
                    name: Some(named("split", 150, 155)),
                    default: None,
                    span: range(150, 155),
                }],
                span: range(106, 160),
            },
        ],
        method_blocks: vec![MethodBlock {
            attributes: vec![Attribute {
                name: Some(named("Access", 52, 58)),
                value: Some(name("protected", 61, 70)),
                span: range(52, 70),
            }],
            methods: vec![FunctionDef {
                name: Some(named("read", 72, 76)),
                outputs: vec![named("value", 77, 82)],
                inputs: vec![named("object", 83, 89)],
                body: Vec::new(),
                span: range(72, 95),
            }],
            declarations: Vec::new(),
            span: range(51, 100),
        }],
        span: range(0, 180),
    };
    let module = compile(&file(vec![Stmt {
        kind: StmtKind::Class(class),
        span: range(0, 180),
        suppress_output: true,
    }]))
    .expect("access-controlled subclass should compile");
    verify(&module).expect("access-controlled class bytecode should verify");

    let class = &module.classes[0];
    assert_eq!(class.superclass.as_deref(), Some("Base"));
    assert_eq!(
        class.properties[0].get_access,
        Some(BytecodeAccess::Private)
    );
    assert_eq!(
        class.properties[0].set_access,
        Some(BytecodeAccess::Private)
    );
    assert_eq!(
        class.properties[1].get_access,
        Some(BytecodeAccess::Protected)
    );
    assert_eq!(
        class.properties[1].set_access,
        Some(BytecodeAccess::Private)
    );
    assert_eq!(class.methods[0].access, BytecodeAccess::Protected);
}

fn derived_constructor_hir(superclass_call: &str) -> HirFile {
    let call = expression(
        ExprKind::SuperclassConstructorCall {
            object: Box::new(name("object", 80, 86)),
            superclass: Some(named(superclass_call, 87, 91)),
            arguments: vec![name("value", 92, 97)],
        },
        80,
        98,
    );
    let class = ClassDef {
        name: Some(named("Derived", 0, 7)),
        superclass: Some(named("Base", 10, 14)),
        enumeration_blocks: Vec::new(),
        event_blocks: Vec::new(),
        attributes: Vec::new(),
        property_blocks: Vec::new(),
        method_blocks: vec![MethodBlock {
            attributes: Vec::new(),
            methods: vec![FunctionDef {
                name: Some(named("Derived", 40, 47)),
                outputs: vec![named("object", 30, 36)],
                inputs: vec![named("value", 48, 53)],
                body: vec![Stmt {
                    kind: StmtKind::Expr(call),
                    span: range(80, 99),
                    suppress_output: true,
                }],
                span: range(25, 110),
            }],
            declarations: Vec::new(),
            span: range(20, 120),
        }],
        span: range(0, 130),
    };
    file(vec![Stmt {
        kind: StmtKind::Class(class),
        span: range(0, 130),
        suppress_output: true,
    }])
}

#[test]
fn compiles_direct_superclass_constructor_on_seeded_output_object() {
    let module = compile(&derived_constructor_hir("Base"))
        .expect("direct superclass constructor should compile");
    verify(&module).expect("compiler output should verify");

    let constructor = &module.functions[module.classes[0].methods[0].function.get() as usize];
    let instruction = constructor
        .instructions
        .iter()
        .find_map(|instruction| match &instruction.kind {
            InstructionKind::InvokeSuperclassConstructor {
                superclass,
                object,
                arguments,
            } => Some((*superclass, *object, arguments.as_slice())),
            _ => None,
        })
        .expect("superclass constructor bytecode");
    assert_eq!(
        constructor.constants[instruction.0.get() as usize],
        Constant::String("Base".to_owned())
    );
    assert_eq!(instruction.1, openmat_bytecode::LocalSlot::new(1));
    assert_eq!(instruction.2.len(), 1);
}

#[test]
fn rejects_non_direct_and_non_constructor_superclass_calls() {
    let wrong = compile(&derived_constructor_hir("Other"))
        .expect_err("only the direct superclass constructor is legal");
    assert!(wrong.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "OMC0001"
            && diagnostic.range == range(87, 91)
            && matches!(diagnostic.kind, CompileDiagnosticKind::MalformedHir { .. })
    }));

    let call = expression(
        ExprKind::SuperclassConstructorCall {
            object: Box::new(name("object", 10, 16)),
            superclass: Some(named("Base", 17, 21)),
            arguments: Vec::new(),
        },
        10,
        23,
    );
    let outside = compile(&file(vec![Stmt {
        kind: StmtKind::Expr(call),
        span: range(10, 24),
        suppress_output: true,
    }]))
    .expect_err("superclass construction outside a constructor must fail");
    assert!(outside.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "OMC0001"
            && diagnostic.range == range(10, 23)
            && matches!(diagnostic.kind, CompileDiagnosticKind::MalformedHir { .. })
    }));
}

#[test]
fn supported_sealed_metadata_does_not_hide_invalid_static_plus_signature() {
    let class = ClassDef {
        name: Some(named("Deferred", 0, 8)),
        superclass: Some(named("Base", 10, 14)),
        enumeration_blocks: Vec::new(),
        event_blocks: Vec::new(),
        attributes: vec![Attribute {
            name: Some(named("Sealed", 15, 21)),
            value: None,
            span: range(15, 21),
        }],
        property_blocks: vec![PropertyBlock {
            attributes: vec![
                Attribute {
                    name: Some(named("Constant", 22, 30)),
                    value: None,
                    span: range(22, 30),
                },
                Attribute {
                    name: Some(named("Access", 31, 37)),
                    value: Some(name("private", 40, 47)),
                    span: range(31, 47),
                },
            ],
            properties: vec![PropertyDef {
                name: Some(named("value", 48, 53)),
                default: Some(number("1", 56, 57)),
                span: range(48, 57),
            }],
            span: range(22, 60),
        }],
        method_blocks: vec![MethodBlock {
            attributes: vec![Attribute {
                name: Some(named("Static", 61, 67)),
                value: None,
                span: range(61, 67),
            }],
            methods: vec![FunctionDef {
                name: Some(named("plus", 70, 74)),
                outputs: vec![named("out", 75, 78)],
                inputs: vec![named("obj", 79, 82)],
                body: Vec::new(),
                span: range(70, 90),
            }],
            declarations: Vec::new(),
            span: range(61, 95),
        }],
        span: range(0, 100),
    };
    let error = compile(&file(vec![Stmt {
        kind: StmtKind::Class(class),
        span: range(0, 100),
        suppress_output: true,
    }]))
    .expect_err("invalid static plus signature must not silently compile");

    assert!(!error.diagnostics().iter().any(|diagnostic| matches!(
        &diagnostic.kind,
        CompileDiagnosticKind::Unsupported {
            feature: UnsupportedFeature::ClassAttribute(attribute)
        } if attribute == "Sealed"
    )));
    assert!(error.diagnostics().iter().any(|diagnostic| matches!(
        &diagnostic.kind,
        CompileDiagnosticKind::InvalidClassMember { member, .. } if member == "plus"
    )));
}

#[test]
#[allow(clippy::too_many_lines)]
fn compiles_constant_property_and_static_method_without_instance_construction() {
    let constant_field = || {
        expression(
            ExprKind::Field {
                target: Box::new(name("Scale", 80, 85)),
                name: Some(named("Factor", 86, 92)),
            },
            80,
            92,
        )
    };
    let scale_call = call(
        expression(
            ExprKind::Field {
                target: Box::new(name("Scale", 120, 125)),
                name: Some(named("scale", 126, 131)),
            },
            120,
            131,
        ),
        vec![number("5", 132, 133)],
        120,
        134,
    );
    let class = ClassDef {
        name: Some(named("Scale", 0, 5)),
        superclass: None,
        enumeration_blocks: Vec::new(),
        event_blocks: Vec::new(),
        attributes: Vec::new(),
        property_blocks: vec![PropertyBlock {
            attributes: vec![Attribute {
                name: Some(named("Constant", 10, 18)),
                value: None,
                span: range(10, 18),
            }],
            properties: vec![PropertyDef {
                name: Some(named("Factor", 20, 26)),
                default: Some(number("3", 29, 30)),
                span: range(20, 30),
            }],
            span: range(8, 35),
        }],
        method_blocks: vec![MethodBlock {
            attributes: vec![Attribute {
                name: Some(named("Static", 40, 46)),
                value: None,
                span: range(40, 46),
            }],
            methods: vec![FunctionDef {
                name: Some(named("scale", 48, 53)),
                outputs: vec![named("result", 54, 60)],
                inputs: vec![named("value", 61, 66)],
                body: vec![assignment(
                    name("result", 70, 76),
                    binary(
                        BinaryOp::Multiply,
                        constant_field(),
                        name("value", 95, 100),
                        80,
                        100,
                    ),
                    70,
                    101,
                )],
                span: range(48, 105),
            }],
            declarations: Vec::new(),
            span: range(40, 110),
        }],
        span: range(0, 115),
    };
    let result = assignment(
        name("openmat_result", 116, 130),
        expression(
            ExprKind::Matrix(vec![vec![constant_field(), scale_call]]),
            80,
            135,
        ),
        116,
        136,
    );
    let module = compile(&file(vec![
        Stmt {
            kind: StmtKind::Class(class),
            span: range(0, 115),
            suppress_output: true,
        },
        result,
    ]))
    .expect("constant and static members should compile");

    verify(&module).expect("constant/static bytecode should verify");
    let class = &module.classes[0];
    assert_eq!(class.properties[0].kind, BytecodePropertyKind::Constant);
    assert_eq!(class.properties[0].set_access, None);
    assert_eq!(class.methods[0].kind, BytecodeMethodKind::Static);
    let method = &module.functions[class.methods[0].function.get() as usize];
    assert_eq!(method.parameter_count, 1);
    assert!(method.instructions.iter().any(|instruction| matches!(
        instruction.kind,
        InstructionKind::LoadQualifiedTarget { .. }
    )));
    assert!(
        method.instructions.iter().any(|instruction| matches!(
            instruction.kind,
            InstructionKind::GetQualifiedPack { .. }
        ))
    );
    assert!(
        module.functions[0]
            .instructions
            .iter()
            .any(|instruction| matches!(instruction.kind, InstructionKind::ApplyField { .. }))
    );
}

#[test]
fn aggregate_pack_register_files_are_independent_per_function() {
    let entry_value = call(
        name("sink", 5, 9),
        vec![
            brace(
                name("C", 10, 11),
                vec![expression(ExprKind::AllIndex, 12, 13)],
                10,
                14,
            ),
            field(name("S", 15, 16), "f", 15, 18),
        ],
        5,
        19,
    );
    let aggregate_function = FunctionDef {
        name: Some(named("read_field", 30, 40)),
        outputs: vec![named("out", 41, 44)],
        inputs: vec![named("value", 45, 50)],
        body: vec![assignment(
            name("out", 51, 54),
            field(name("value", 57, 62), "f", 57, 64),
            51,
            65,
        )],
        span: range(30, 70),
    };
    let ordinary_function = FunctionDef {
        name: Some(named("identity", 80, 88)),
        outputs: vec![named("out", 89, 92)],
        inputs: vec![named("value", 93, 98)],
        body: vec![assignment(
            name("out", 100, 103),
            name("value", 106, 111),
            100,
            112,
        )],
        span: range(80, 120),
    };
    let module = compile(&file(vec![
        assignment(name("result", 0, 4), entry_value, 0, 20),
        Stmt {
            kind: StmtKind::Function(aggregate_function),
            span: range(30, 70),
            suppress_output: true,
        },
        Stmt {
            kind: StmtKind::Function(ordinary_function),
            span: range(80, 120),
            suppress_output: true,
        },
    ]))
    .expect("aggregate pack registers should be allocated per function");

    verify(&module).expect("per-function pack-register bytecode should verify");
    assert_eq!(module.functions[0].pack_register_count, 2);
    assert_eq!(module.functions[1].pack_register_count, 1);
    assert_eq!(module.functions[2].pack_register_count, 0);
    assert!(
        module.functions[0]
            .instructions
            .iter()
            .any(|instruction| matches!(
                &instruction.kind,
                InstructionKind::BraceApply { dst_pack, .. } if dst_pack.get() == 0
            ))
    );
    assert!(
        module.functions[1]
            .instructions
            .iter()
            .any(|instruction| matches!(
                &instruction.kind,
                InstructionKind::GetAggregateField { dst_pack, .. } if dst_pack.get() == 0
            ))
    );
}

#[test]
fn qualified_function_calls_load_one_name_unless_the_root_is_a_value_binding() {
    let qualified_target = field(field(name("alpha", 0, 5), "nested", 0, 12), "inc", 0, 16);
    let module = compile(&file(vec![assignment(
        name("result", 17, 23),
        call(qualified_target, vec![number("4", 24, 25)], 0, 26),
        0,
        27,
    )]))
    .expect("qualified package call should compile");
    verify(&module).expect("qualified package-call bytecode should verify");
    let entry = &module.functions[module.entry.get() as usize];
    assert!(entry.instructions.iter().any(|instruction| {
        let InstructionKind::LoadQualifiedTarget { qualified, .. } = &instruction.kind else {
            return false;
        };
        matches!(
            qualified
                .first()
                .and_then(|qualified| entry.constants.get(qualified.get() as usize)),
            Some(Constant::String(value)) if value == "alpha.nested.inc"
        )
    }));
    assert!(
        !entry
            .instructions
            .iter()
            .any(|instruction| matches!(instruction.kind, InstructionKind::ApplyField { .. }))
    );

    let bound_target = field(
        field(name("alpha", 30, 35), "nested", 30, 42),
        "inc",
        30,
        46,
    );
    let bound = compile(&file(vec![
        assignment(name("alpha", 28, 33), number("1", 36, 37), 28, 38),
        assignment(
            name("result", 47, 53),
            call(bound_target, vec![number("4", 54, 55)], 30, 56),
            30,
            57,
        ),
    ]))
    .expect("value-root member call should compile");
    verify(&bound).expect("value-root bytecode should verify");
    let entry = &bound.functions[bound.entry.get() as usize];
    assert!(
        entry
            .instructions
            .iter()
            .any(|instruction| matches!(instruction.kind, InstructionKind::ApplyField { .. }))
    );
}

#[test]
fn imports_are_lexical_and_preserve_candidate_declaration_order() {
    let module = compile(&file(vec![
        assignment(
            name("result", 0, 6),
            call(name("twice", 9, 14), vec![number("4", 15, 16)], 9, 17),
            0,
            18,
        ),
        command("import", &["alpha.*"], 19, 33, true),
        command("import", &["beta.*"], 34, 47, true),
        assignment(name("origin", 48, 54), name("zero", 57, 61), 48, 62),
    ]))
    .expect("lexical imports should compile");
    verify(&module).expect("import bytecode should verify");
    let entry = &module.functions[module.entry.get() as usize];

    let declared = entry.instructions.iter().find_map(|instruction| {
        let InstructionKind::DeclareImports { imports } = &instruction.kind else {
            return None;
        };
        Some(
            imports
                .iter()
                .filter_map(|import| match entry.constants.get(import.get() as usize) {
                    Some(Constant::String(import)) => Some(import.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>(),
        )
    });
    assert_eq!(declared, Some(vec!["alpha.*", "beta.*"]));

    let candidates = entry.instructions.iter().find_map(|instruction| {
        let InstructionKind::LoadQualifiedTarget { qualified, .. } = &instruction.kind else {
            return None;
        };
        Some(
            qualified
                .iter()
                .filter_map(
                    |candidate| match entry.constants.get(candidate.get() as usize) {
                        Some(Constant::String(candidate)) => Some(candidate.as_str()),
                        _ => None,
                    },
                )
                .collect::<Vec<_>>(),
        )
    });
    assert_eq!(candidates, Some(vec!["alpha.twice", "beta.twice"]));
    assert!(
        entry.instructions.iter().any(|instruction| matches!(
            instruction.kind,
            InstructionKind::GetQualifiedPack { .. }
        ))
    );
}

#[test]
fn clear_import_resets_entry_resolution_and_can_be_followed_by_a_new_import() {
    let clear_import = Stmt {
        kind: StmtKind::Clear(ClearStatement {
            names: vec![named("import", 20, 26)],
            form: ClearForm::IdentifierList,
        }),
        span: range(14, 26),
        suppress_output: true,
    };
    let module = compile(&file(vec![
        command("import", &["alpha.*"], 0, 13, true),
        clear_import,
        assignment(
            name("plain", 27, 32),
            call(name("twice", 35, 40), vec![number("2", 41, 42)], 35, 43),
            27,
            44,
        ),
        command("import", &["beta.*"], 45, 58, true),
        assignment(
            name("qualified", 59, 68),
            call(name("twice", 71, 76), vec![number("3", 77, 78)], 71, 79),
            59,
            80,
        ),
    ]))
    .expect("base-workspace import clearing should compile");
    verify(&module).expect("mutable import bytecode should verify");
    let entry = &module.functions[module.entry.get() as usize];
    assert!(
        entry
            .instructions
            .iter()
            .any(|instruction| matches!(instruction.kind, InstructionKind::ClearImports))
    );
    let qualified_candidates = entry
        .instructions
        .iter()
        .filter_map(|instruction| {
            let InstructionKind::LoadQualifiedTarget { qualified, .. } = &instruction.kind else {
                return None;
            };
            Some(
                qualified
                    .iter()
                    .filter_map(
                        |candidate| match entry.constants.get(candidate.get() as usize) {
                            Some(Constant::String(candidate)) => Some(candidate.as_str()),
                            _ => None,
                        },
                    )
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        qualified_candidates,
        vec![
            vec!["alpha.twice", "beta.twice"],
            vec!["alpha.twice", "beta.twice"]
        ]
    );
    let clear_index = entry
        .instructions
        .iter()
        .position(|instruction| matches!(instruction.kind, InstructionKind::ClearImports))
        .unwrap();
    assert!(
        entry.instructions[clear_index + 1..]
            .iter()
            .any(|instruction| {
                matches!(instruction.kind, InstructionKind::DeclareImports { .. })
            })
    );
}

#[test]
fn compiles_bodyless_concrete_method_as_external_class_folder_slot() {
    let class = ClassDef {
        name: Some(named("FolderBox", 0, 9)),
        superclass: None,
        attributes: Vec::new(),
        property_blocks: Vec::new(),
        method_blocks: vec![MethodBlock {
            attributes: Vec::new(),
            methods: Vec::new(),
            declarations: vec![MethodDeclaration {
                name: Some(named("bump", 30, 34)),
                outputs: vec![named("value", 22, 27)],
                inputs: vec![named("object", 35, 41), named("amount", 43, 49)],
                span: range(22, 50),
            }],
            span: range(15, 55),
        }],
        enumeration_blocks: Vec::new(),
        event_blocks: Vec::new(),
        span: range(0, 60),
    };
    let module = compile(&file(vec![Stmt {
        kind: StmtKind::Class(class),
        span: range(0, 60),
        suppress_output: true,
    }]))
    .expect("body-less concrete method should compile for @Class linking");
    let method = &module.classes[0].methods[0];
    assert!(!method.is_abstract);
    assert!(method.is_external);
    assert_eq!(
        module.functions[method.function.get() as usize].parameter_count,
        2
    );
    let mut legacy = module;
    legacy.version = MIN_SUPPORTED_BYTECODE_VERSION;
    assert!(matches!(
        verify(&legacy).unwrap_err().kind,
        VerificationErrorKind::ExternalMethodRequiresCurrentVersion { .. }
    ));
}

#[test]
fn aggregate_postfix_names_are_preserved_as_anonymous_function_captures() {
    let body = dynamic_field(
        brace(name("C", 40, 41), vec![name("k", 42, 43)], 40, 44),
        name("field_name", 46, 56),
        40,
        57,
    );
    let hir = file(vec![
        assignment(
            name("C", 0, 1),
            expression(ExprKind::Cell(vec![vec![number("1", 5, 6)]]), 4, 7),
            0,
            8,
        ),
        assignment(
            name("field_name", 10, 20),
            expression(ExprKind::String("\"f\"".to_owned()), 23, 26),
            10,
            27,
        ),
        assignment(
            name("reader", 30, 36),
            anonymous(vec![named("k", 38, 39)], body, 37, 58),
            30,
            59,
        ),
    ]);
    let module = compile(&hir).expect("aggregate anonymous function should compile");

    verify(&module).expect("aggregate closure bytecode should verify");
    assert_eq!(module.functions.len(), 2);
    let captures = module.functions[0]
        .instructions
        .iter()
        .find_map(|instruction| match &instruction.kind {
            InstructionKind::MakeClosure { captures, .. } => Some(captures),
            _ => None,
        })
        .expect("entry should construct the aggregate closure");
    assert_eq!(captures.len(), 2);
    let anonymous = &module.functions[1];
    assert_eq!(anonymous.pack_register_count, 2);
    assert!(
        anonymous
            .instructions
            .iter()
            .any(|instruction| matches!(instruction.kind, InstructionKind::BraceApply { .. }))
    );
    assert!(
        anonymous.instructions.iter().any(|instruction| matches!(
            instruction.kind,
            InstructionKind::GetAggregateField { .. }
        ))
    );
}

#[test]
fn lowers_nested_cells_and_static_dynamic_field_splices_without_scalar_special_cases() {
    let nested_one = expression(ExprKind::Cell(vec![vec![number("1", 10, 11)]]), 9, 12);
    let nested_empty = expression(ExprKind::Cell(Vec::new()), 30, 32);
    let outer = expression(
        ExprKind::Cell(vec![
            vec![nested_one, field(name("S", 15, 16), "f", 15, 18)],
            vec![
                nested_empty,
                dynamic_field(name("D", 35, 36), name("field_name", 38, 48), 35, 49),
            ],
        ]),
        5,
        50,
    );
    let module = compile(&file(vec![assignment(name("result", 0, 4), outer, 0, 51)]))
        .expect("nested cell and field splice lowering should compile");

    verify(&module).expect("nested aggregate bytecode should verify");
    let function = &module.functions[0];
    assert_eq!(function.pack_register_count, 2);
    let cells = function
        .instructions
        .iter()
        .filter_map(|instruction| match &instruction.kind {
            InstructionKind::BuildCell { rows, .. } => Some(rows),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(cells.len(), 3);
    assert!(cells.iter().any(|rows| rows.is_empty()));
    let outer_rows = cells
        .iter()
        .find(|rows| rows.len() == 2)
        .expect("two source rows must remain distinct");
    assert!(matches!(
        outer_rows.as_slice(),
        [first, second]
            if matches!(first.as_slice(), [ValueSource::One(_), ValueSource::Expand(_)])
                && matches!(second.as_slice(), [ValueSource::One(_), ValueSource::Expand(_)])
    ));
    assert!(function.instructions.iter().any(|instruction| matches!(
        instruction.kind,
        InstructionKind::GetAggregateField {
            field: FieldOperand::Static(_),
            ..
        } | InstructionKind::GetQualifiedPack { .. }
    )));
    assert!(function.instructions.iter().any(|instruction| matches!(
        instruction.kind,
        InstructionKind::GetAggregateField {
            field: FieldOperand::Dynamic(_),
            ..
        }
    )));
}

#[test]
fn brace_and_field_packs_expand_in_outputs_calls_and_index_arguments() {
    let multiple_target = expression(
        ExprKind::Matrix(vec![vec![name("a", 0, 1), name("b", 3, 4)]]),
        0,
        5,
    );
    let all = || expression(ExprKind::AllIndex, 10, 11);
    let zero_output_span = range(70, 73);
    let hir = file(vec![
        assignment(
            multiple_target,
            brace(name("C", 7, 8), vec![all()], 7, 12),
            0,
            13,
        ),
        assignment(
            name("x", 20, 21),
            field(name("S", 24, 25), "f", 24, 27),
            20,
            28,
        ),
        assignment(
            name("y", 30, 31),
            dynamic_field(name("D", 34, 35), name("key", 37, 40), 34, 41),
            30,
            42,
        ),
        assignment(
            name("z", 50, 51),
            call(
                name("f", 54, 55),
                vec![
                    brace(name("R", 56, 57), vec![all()], 56, 60),
                    field(name("T", 62, 63), "g", 62, 65),
                ],
                54,
                66,
            ),
            50,
            67,
        ),
        Stmt {
            kind: StmtKind::Expr(field(
                name("U", 70, 71),
                "h",
                zero_output_span.start(),
                zero_output_span.end(),
            )),
            span: range(70, 74),
            suppress_output: true,
        },
    ]);
    let module = compile(&hir).expect("pack consumers should compile");

    verify(&module).expect("pack consumer bytecode should verify");
    let function = &module.functions[0];
    assert_eq!(function.pack_register_count, 6);
    let unpack_widths = function
        .instructions
        .iter()
        .filter_map(|instruction| match &instruction.kind {
            InstructionKind::Unpack { outputs, .. } => Some(outputs.len()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(unpack_widths, vec![2, 1, 1]);
    assert!(function.instructions.iter().any(|instruction| matches!(
        &instruction.kind,
        InstructionKind::Apply { arguments, .. }
            if matches!(arguments.as_slice(), [ApplyArgument::Expand(_), ApplyArgument::Expand(_)])
    )));
    assert!(
        !function
            .instructions
            .iter()
            .any(|instruction| matches!(instruction.kind, InstructionKind::GetField { .. }))
    );
    let zero_output_location = Some(SourceLocation::new(
        SOURCE.raw(),
        zero_output_span.start(),
        zero_output_span.end(),
    ));
    assert!(function.instructions.iter().any(|instruction| {
        matches!(
            instruction.kind,
            InstructionKind::GetAggregateField { .. } | InstructionKind::GetQualifiedPack { .. }
        ) && instruction.location == zero_output_location
    }));
    assert!(!function.instructions.iter().any(|instruction| {
        matches!(instruction.kind, InstructionKind::Unpack { .. })
            && instruction.location == zero_output_location
    }));
}

#[test]
#[allow(clippy::too_many_lines)]
fn bracketed_aggregate_places_count_rhs_outputs_and_slice_once() {
    // These HIR shapes are the direct lowering of:
    // [cells{[1, 4]}] = deal(101, 404);
    // [records([1, 3]).alpha] = deal('left', 'right')
    let cell_selector = expression(
        ExprKind::Matrix(vec![vec![number("1", 8, 9), number("4", 11, 12)]]),
        7,
        13,
    );
    let cell_target = expression(
        ExprKind::Matrix(vec![vec![brace(
            name("cells", 1, 6),
            vec![cell_selector],
            1,
            14,
        )]]),
        0,
        15,
    );
    let cell_rhs = call(
        name("deal", 18, 22),
        vec![number("101", 23, 26), number("404", 28, 31)],
        18,
        32,
    );

    let record_selector = expression(
        ExprKind::Matrix(vec![vec![number("1", 45, 46), number("3", 48, 49)]]),
        44,
        50,
    );
    let record_target = expression(
        ExprKind::Matrix(vec![vec![field(
            call(name("records", 36, 43), vec![record_selector], 36, 51),
            "alpha",
            36,
            57,
        )]]),
        35,
        58,
    );
    let record_rhs = call(
        name("deal", 61, 65),
        vec![
            expression(ExprKind::Char("'left'".to_owned()), 66, 72),
            expression(ExprKind::Char("'right'".to_owned()), 74, 81),
        ],
        61,
        82,
    );

    let hir = file(vec![
        assignment(cell_target, cell_rhs, 0, 33),
        Stmt {
            kind: StmtKind::Assignment {
                target: record_target,
                value: record_rhs,
            },
            span: range(35, 82),
            suppress_output: false,
        },
    ]);
    let module = compile(&hir).expect("static aggregate comma assignment should compile");
    verify(&module).expect("aggregate comma assignment bytecode should verify");
    let function = &module.functions[0];
    let instructions = &function.instructions;

    let applies = instructions
        .iter()
        .enumerate()
        .filter_map(|(index, instruction)| match &instruction.kind {
            InstructionKind::ApplyOutputPack { dst_pack, .. } => Some((index, *dst_pack)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(applies.len(), 2, "each RHS must be evaluated by one apply");
    assert_eq!(
        instructions
            .iter()
            .filter(|instruction| matches!(
                instruction.kind,
                InstructionKind::CountPlaceOutputs { .. }
            ))
            .count(),
        2
    );

    let assignments = instructions
        .iter()
        .enumerate()
        .filter_map(|(index, instruction)| match &instruction.kind {
            InstructionKind::AssignBindingPlace {
                result,
                path,
                source: ValueSource::Expand(pack),
                mode: AssignmentMode::Store,
                ..
            } => Some((index, *result, path, *pack)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(assignments.len(), 2);
    assert_eq!(assignments[0].1, None);
    assert!(assignments[1].1.is_some());
    assert_eq!(function.pack_register_count, 4);

    for ((apply_index, output_pack), (assign_index, _, path, source_pack)) in
        applies.iter().zip(&assignments)
    {
        let pack_index = instructions
            .iter()
            .enumerate()
            .find_map(|(index, instruction)| match &instruction.kind {
                InstructionKind::SlicePack {
                    dst_pack, source, ..
                } if index > *apply_index
                    && index < *assign_index
                    && *dst_pack == *source_pack
                    && *source == *output_pack =>
                {
                    Some(index)
                }
                _ => None,
            })
            .expect("the output pack must be sliced once without a temporary cell");
        assert!(*apply_index < pack_index);
        assert!(pack_index < *assign_index);
        assert!(!path.is_empty());
    }

    let cell_path = assignments[0].2;
    assert!(matches!(
        cell_path.as_slice(),
        [PlaceStep::Brace(arguments)]
            if matches!(arguments.as_slice(), [ApplyArgument::Value(_)])
    ));
    let record_path = assignments[1].2;
    assert!(matches!(
        record_path.as_slice(),
        [PlaceStep::Paren(arguments), PlaceStep::Field(FieldOperand::Static(field))]
            if matches!(arguments.as_slice(), [ApplyArgument::Value(_)])
                && function.constants.get(
                    usize::try_from(field.get()).expect("constant id fits usize")
                ) == Some(&Constant::String("alpha".to_owned()))
    ));

    let displays = instructions
        .iter()
        .filter_map(|instruction| match instruction.kind {
            InstructionKind::Display { name, src } => Some((name, src)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        displays.len(),
        1,
        "the semicolon must suppress only display"
    );
    assert_eq!(displays[0].1, assignments[1].1.unwrap());
    assert_eq!(
        function
            .constants
            .get(usize::try_from(displays[0].0.get()).expect("constant id fits usize")),
        Some(&Constant::String("records".to_owned()))
    );
}

#[test]
fn column_major_expanded_place_preserves_selector_rows_and_rhs_output_order() {
    let selector = expression(
        ExprKind::Matrix(vec![
            vec![number("1", 9, 10), number("3", 12, 13)],
            vec![number("2", 15, 16), number("4", 18, 19)],
        ]),
        8,
        20,
    );
    let target = expression(
        ExprKind::Matrix(vec![vec![brace(
            name("cells", 1, 6),
            vec![selector],
            1,
            21,
        )]]),
        0,
        22,
    );
    let rhs = call(name("producer", 25, 33), Vec::new(), 25, 35);
    let module = compile(&file(vec![assignment(target, rhs, 0, 36)]))
        .expect("column-major aggregate place should compile");
    verify(&module).expect("column-major aggregate place bytecode should verify");
    let function = &module.functions[0];

    let output_pack = function
        .instructions
        .iter()
        .find_map(|instruction| match &instruction.kind {
            InstructionKind::ApplyOutputPack { dst_pack, .. } => Some(dst_pack),
            _ => None,
        })
        .expect("the selected places must request a runtime-sized output pack");
    assert!(function.instructions.iter().any(|instruction| matches!(
        &instruction.kind,
        InstructionKind::SlicePack { source, .. } if source == output_pack
    )));
    let selector_rows = function
        .instructions
        .iter()
        .find_map(|instruction| match &instruction.kind {
            InstructionKind::BuildMatrix { rows, .. }
                if rows.len() == 2 && rows.iter().all(|row| row.len() == 2) =>
            {
                Some(rows)
            }
            _ => None,
        })
        .expect("the selector must preserve its two source rows");
    let selector_values = selector_rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|register| {
                    let constant = function
                        .instructions
                        .iter()
                        .find_map(|instruction| match instruction.kind {
                            InstructionKind::LoadConstant { dst, constant } if dst == *register => {
                                Some(constant)
                            }
                            _ => None,
                        })
                        .expect("selector register must come from a constant");
                    function
                        .constants
                        .get(usize::try_from(constant.get()).expect("constant id fits usize"))
                        .cloned()
                        .expect("selector constant must exist")
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        selector_values,
        vec![
            vec![Constant::Double(1.0), Constant::Double(3.0)],
            vec![Constant::Double(2.0), Constant::Double(4.0)]
        ]
    );
}

#[test]
fn ordinary_multi_assignment_and_discard_keep_fixed_output_lowering() {
    let target = expression(
        ExprKind::Matrix(vec![vec![
            name("a", 1, 2),
            name("~", 4, 5),
            name("b", 7, 8),
        ]]),
        0,
        9,
    );
    let statement = Stmt {
        kind: StmtKind::Assignment {
            target,
            value: call(name("f", 12, 13), Vec::new(), 12, 15),
        },
        span: range(0, 15),
        suppress_output: false,
    };
    let module = compile(&file(vec![statement])).expect("ordinary multi-assignment should compile");
    let function = &module.functions[0];
    assert!(function.instructions.iter().any(|instruction| matches!(
        &instruction.kind,
        InstructionKind::Apply { outputs, .. } if outputs.len() == 3
    )));
    assert_eq!(
        function
            .instructions
            .iter()
            .filter(|instruction| matches!(instruction.kind, InstructionKind::StoreGlobal { .. }))
            .count(),
        2
    );
    assert_eq!(
        function
            .instructions
            .iter()
            .filter(|instruction| matches!(instruction.kind, InstructionKind::Display { .. }))
            .count(),
        2
    );
    assert_eq!(function.pack_register_count, 0);
}

#[test]
fn bare_zero_argument_call_supports_multiple_outputs_and_discard() {
    let target = expression(
        ExprKind::Matrix(vec![vec![name("warning_text", 1, 13), name("~", 15, 16)]]),
        0,
        17,
    );
    let statement = assignment(target, name("lastwarn", 20, 28), 0, 29);

    let module = compile(&file(vec![statement]))
        .expect("bare zero-argument calls should support multiple outputs");
    verify(&module).expect("bare multi-output call bytecode should verify");
    let function = &module.functions[0];
    assert!(
        function
            .instructions
            .iter()
            .any(|instruction| matches!(instruction.kind, InstructionKind::LoadCallTarget { .. }))
    );
    assert!(function.instructions.iter().any(|instruction| matches!(
        &instruction.kind,
        InstructionKind::Apply {
            outputs,
            arguments,
            ..
        } if outputs.len() == 2 && arguments.is_empty()
    )));
    assert_eq!(
        function
            .instructions
            .iter()
            .filter(|instruction| matches!(instruction.kind, InstructionKind::StoreGlobal { .. }))
            .count(),
        1
    );
}

#[test]
fn runtime_sized_bracketed_aggregate_call_uses_counted_pack_outputs() {
    let target = expression(
        ExprKind::Matrix(vec![vec![brace(
            name("cells", 1, 6),
            vec![name("indices", 7, 14)],
            1,
            15,
        )]]),
        0,
        16,
    );
    let module = compile(&file(vec![assignment(
        target,
        call(name("producer", 19, 27), Vec::new(), 19, 29),
        0,
        30,
    )]))
    .expect("v27 supports runtime-sized call outputs");
    verify(&module).unwrap();
    assert!(
        module.functions[0]
            .instructions
            .iter()
            .any(|instruction| matches!(instruction.kind, InstructionKind::ApplyOutputPack { .. }))
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn end_and_transactional_aggregate_places_lower_to_verified_v13_operations() {
    let end = || expression(ExprKind::EndIndex, 10, 13);
    let selected = call(
        name("A", 5, 6),
        vec![
            end(),
            binary(BinaryOp::Subtract, end(), number("1", 14, 15), 10, 15),
        ],
        5,
        16,
    );
    let delete_target = call(name("C", 25, 26), vec![end()], 25, 31);
    let brace_store = brace(name("C", 40, 41), vec![number("1", 42, 43)], 40, 44);
    let nested_place = call(
        dynamic_field(
            brace(name("C", 60, 61), vec![number("1", 62, 63)], 60, 64),
            name("field_name", 66, 76),
            60,
            77,
        ),
        vec![number("2", 79, 80)],
        60,
        81,
    );
    let empty = || expression(ExprKind::Matrix(Vec::new()), 32, 34);
    let hir = file(vec![
        assignment(name("x", 0, 1), selected, 0, 17),
        assignment(delete_target, empty(), 25, 35),
        assignment(brace_store, empty(), 40, 50),
        assignment(
            nested_place,
            brace(
                name("R", 85, 86),
                vec![expression(ExprKind::AllIndex, 87, 88)],
                85,
                89,
            ),
            60,
            90,
        ),
        assignment(
            field(name("obj", 95, 98), "x", 95, 100),
            number("7", 103, 104),
            95,
            105,
        ),
    ]);
    let module = compile(&hir).expect("end and aggregate place lowering should compile");

    verify(&module).expect("aggregate place bytecode should verify");
    let function = &module.functions[0];
    let end_metadata = function
        .instructions
        .iter()
        .filter_map(|instruction| match instruction.kind {
            InstructionKind::ResolveEnd {
                argument_index,
                argument_count,
                ..
            } => Some((argument_index, argument_count)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(end_metadata, vec![(0, 2), (1, 2), (0, 1)]);
    assert!(function.instructions.iter().any(|instruction| matches!(
        &instruction.kind,
        InstructionKind::AssignPlace {
            path,
            source: ValueSource::One(_),
            mode: AssignmentMode::Delete,
            ..
        } if matches!(path.as_slice(), [PlaceStep::Paren(_)])
    )));
    assert!(function.instructions.iter().any(|instruction| matches!(
        &instruction.kind,
        InstructionKind::AssignBindingPlace {
            path,
            source: ValueSource::One(_),
            mode: AssignmentMode::Store,
            ..
        } if matches!(path.as_slice(), [PlaceStep::Brace(_)])
    )));
    assert!(function.instructions.iter().any(|instruction| matches!(
        &instruction.kind,
        InstructionKind::AssignBindingPlace {
            path,
            source: ValueSource::Expand(_),
            mode: AssignmentMode::Store,
            ..
        } if matches!(
            path.as_slice(),
            [PlaceStep::Brace(_), PlaceStep::Field(FieldOperand::Dynamic(_)), PlaceStep::Paren(_)]
        )
    )));
    assert!(function.instructions.iter().any(|instruction| matches!(
        &instruction.kind,
        InstructionKind::AssignBindingPlace {
            path,
            source: ValueSource::One(_),
            mode: AssignmentMode::Store,
            ..
        } if matches!(path.as_slice(), [PlaceStep::Field(FieldOperand::Static(_))])
    )));
    assert!(!function.instructions.iter().any(|instruction| matches!(
        instruction.kind,
        InstructionKind::IndexAssign { .. } | InstructionKind::SetField { .. }
    )));
}

#[test]
fn nested_end_resolvers_bind_to_their_immediate_unresolved_apply_targets() {
    let inner = call(
        name("B", 10, 11),
        vec![expression(ExprKind::EndIndex, 12, 15)],
        10,
        16,
    );
    let outer = call(
        name("A", 5, 6),
        vec![inner, expression(ExprKind::EndIndex, 18, 21)],
        5,
        22,
    );
    let module = compile(&file(vec![assignment(name("x", 0, 1), outer, 0, 23)]))
        .expect("nested end contexts should compile");

    verify(&module).expect("nested end bytecode should verify");
    let instructions = &module.functions[0].instructions;
    let (inner_target, outer_target) = instructions
        .iter()
        .filter_map(|instruction| match &instruction.kind {
            InstructionKind::Apply {
                target, arguments, ..
            } => Some((*target, arguments.len())),
            _ => None,
        })
        .fold(
            (None, None),
            |(inner, outer), (target, count)| match count {
                1 => (Some(target), outer),
                2 => (inner, Some(target)),
                _ => (inner, outer),
            },
        );
    let inner_target = inner_target.expect("inner unresolved apply");
    let outer_target = outer_target.expect("outer unresolved apply");
    assert!(instructions.iter().any(|instruction| matches!(
        instruction.kind,
        InstructionKind::ResolveEnd {
            target,
            argument_index: 0,
            argument_count: 1,
            ..
        } if target == inner_target
    )));
    assert!(instructions.iter().any(|instruction| matches!(
        instruction.kind,
        InstructionKind::ResolveEnd {
            target,
            argument_index: 1,
            argument_count: 2,
            ..
        } if target == outer_target
    )));
    assert_ne!(inner_target, outer_target);
}

#[test]
fn unexpressible_aggregate_combinations_report_stable_diagnostics_without_panicking() {
    let nested_end_place = call(
        brace(name("C", 0, 1), vec![number("1", 2, 3)], 0, 4),
        vec![expression(ExprKind::EndIndex, 5, 8)],
        0,
        9,
    );
    let member_end = call(
        field(name("obj", 20, 23), "method", 20, 30),
        vec![expression(ExprKind::EndIndex, 31, 34)],
        20,
        35,
    );
    let multiple_places = expression(
        ExprKind::Matrix(vec![vec![
            field(
                call(name("S", 50, 51), vec![number("1", 52, 53)], 50, 54),
                "f",
                50,
                56,
            ),
            field(
                call(name("S", 58, 59), vec![number("2", 60, 61)], 58, 62),
                "f",
                58,
                64,
            ),
        ]]),
        49,
        65,
    );
    let hir = file(vec![
        Stmt {
            kind: StmtKind::Expr(expression(ExprKind::EndIndex, 70, 73)),
            span: range(70, 74),
            suppress_output: true,
        },
        assignment(nested_end_place, number("1", 12, 13), 0, 14),
        assignment(name("x", 16, 17), member_end, 16, 36),
        assignment(multiple_places, number("3", 67, 68), 49, 69),
        assignment(
            name("bad", 80, 83),
            dynamic_field(
                name("S", 86, 87),
                expression(ExprKind::Error, 89, 90),
                86,
                91,
            ),
            80,
            92,
        ),
        assignment(
            name("ragged", 100, 106),
            expression(
                ExprKind::Cell(vec![
                    vec![number("1", 110, 111)],
                    vec![number("2", 113, 114), number("3", 116, 117)],
                ]),
                108,
                119,
            ),
            100,
            120,
        ),
    ]);
    let error = compile(&hir).expect_err("invalid aggregate combinations must be diagnosed");

    let unsupported = error
        .diagnostics()
        .iter()
        .filter_map(|diagnostic| match &diagnostic.kind {
            CompileDiagnosticKind::Unsupported { feature } => Some(feature),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(unsupported.contains(&&UnsupportedFeature::EndOutsideIndex));
    assert!(!unsupported.contains(&&UnsupportedFeature::NestedPlaceEnd));
    assert!(unsupported.contains(&&UnsupportedFeature::EndInMemberApply));
    assert!(unsupported.contains(&&UnsupportedFeature::UnequalCellSourceRows));
    assert!(!unsupported.contains(&&UnsupportedFeature::MultipleAggregateAssignmentTargets));
    assert!(
        error.diagnostics().iter().any(|diagnostic| matches!(
            diagnostic.kind,
            CompileDiagnosticKind::MalformedHir { .. }
        ))
    );
    assert!(
        error
            .diagnostics()
            .iter()
            .all(|diagnostic| { matches!(diagnostic.code(), "OMC0001" | "OMC0007") })
    );
}

#[test]
fn global_bindings_share_workspace_instruction_shape_across_functions() {
    let writer = FunctionDef {
        name: Some(named("writer", 10, 16)),
        outputs: Vec::new(),
        inputs: Vec::new(),
        body: vec![
            declaration(
                DeclarationKind::Global,
                DeclarationContext::Function,
                vec![named("shared", 25, 31)],
                18,
                31,
            ),
            assignment(name("shared", 34, 40), number("7", 43, 44), 34, 45),
        ],
        span: range(5, 50),
    };
    let reader = FunctionDef {
        name: Some(named("reader", 70, 76)),
        outputs: vec![named("result", 60, 66)],
        inputs: Vec::new(),
        body: vec![
            declaration(
                DeclarationKind::Global,
                DeclarationContext::Function,
                vec![named("shared", 85, 91)],
                78,
                91,
            ),
            assignment(name("result", 94, 100), name("shared", 103, 109), 94, 110),
        ],
        span: range(55, 115),
    };
    let module = compile(&file(vec![
        Stmt {
            kind: StmtKind::Function(writer),
            span: range(5, 50),
            suppress_output: true,
        },
        Stmt {
            kind: StmtKind::Function(reader),
            span: range(55, 115),
            suppress_output: true,
        },
    ]))
    .expect("shared global declarations should compile");

    let writer = &module.functions[1];
    let reader = &module.functions[2];
    assert_eq!(writer.local_count, 0);
    assert!(
        writer
            .instructions
            .iter()
            .any(|instruction| matches!(instruction.kind, InstructionKind::DeclareGlobal { .. }))
    );
    assert!(
        writer
            .instructions
            .iter()
            .any(|instruction| matches!(instruction.kind, InstructionKind::StoreGlobal { .. }))
    );
    assert!(
        reader
            .instructions
            .iter()
            .any(|instruction| matches!(instruction.kind, InstructionKind::DeclareGlobal { .. }))
    );
    assert!(reader.instructions.iter().any(|instruction| matches!(
        instruction.kind,
        InstructionKind::LoadGlobal {
            construct_if_class: false,
            ..
        }
    )));
    assert!(!writer.instructions.iter().any(|instruction| matches!(
        instruction.kind,
        InstructionKind::StoreLocal { .. } | InstructionKind::LoadLocal { .. }
    )));
    verify(&module).expect("cross-function global bytecode should verify");
}

#[test]
fn persistent_slots_are_stable_and_conditional_declarations_keep_their_location() {
    let declaration_span = range(25, 39);
    let conditional = Stmt {
        kind: StmtKind::If {
            branches: vec![ConditionalBranch {
                condition: name("true", 18, 22),
                body: vec![declaration(
                    DeclarationKind::Persistent,
                    DeclarationContext::Function,
                    vec![named("first", 30, 35), named("second", 36, 42)],
                    declaration_span.start(),
                    declaration_span.end(),
                )],
                span: range(15, 45),
            }],
            else_body: Vec::new(),
        },
        span: range(15, 48),
        suppress_output: true,
    };
    let definition = FunctionDef {
        name: Some(named("stateful", 5, 13)),
        outputs: vec![named("result", 0, 6)],
        inputs: Vec::new(),
        body: vec![
            conditional,
            assignment(name("first", 50, 55), number("1", 58, 59), 50, 60),
            assignment(name("second", 62, 68), name("first", 71, 76), 62, 77),
            assignment(name("result", 79, 85), name("second", 88, 94), 79, 95),
        ],
        span: range(0, 100),
    };
    let module = compile(&file(vec![Stmt {
        kind: StmtKind::Function(definition),
        span: range(0, 100),
        suppress_output: true,
    }]))
    .expect("persistent bindings should compile");
    let function = &module.functions[1];
    assert_eq!(function.persistent_slot_count, 2);
    let declarations = function
        .instructions
        .iter()
        .filter_map(|instruction| match instruction.kind {
            InstructionKind::DeclarePersistent { slot } => Some((slot, instruction.location)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        declarations,
        vec![
            (
                PersistentSlot::new(0),
                Some(SourceLocation::new(
                    SOURCE.raw(),
                    declaration_span.start(),
                    declaration_span.end()
                ))
            ),
            (
                PersistentSlot::new(1),
                Some(SourceLocation::new(
                    SOURCE.raw(),
                    declaration_span.start(),
                    declaration_span.end()
                ))
            )
        ]
    );
    assert!(function.instructions.iter().any(|instruction| matches!(
        instruction.kind,
        InstructionKind::StorePersistent {
            slot,
            ..
        } if slot == PersistentSlot::new(0)
    )));
    assert!(function.instructions.iter().any(|instruction| matches!(
        instruction.kind,
        InstructionKind::LoadPersistent {
            slot,
            ..
        } if slot == PersistentSlot::new(1)
    )));
    verify(&module).expect("persistent bytecode should verify");
}

#[test]
fn global_and_persistent_aggregate_roots_use_their_declared_storage() {
    let global_target = call(
        name("global_array", 40, 52),
        vec![number("1", 53, 54)],
        40,
        55,
    );
    let persistent_target = call(
        name("persistent_array", 65, 81),
        vec![number("1", 82, 83)],
        65,
        84,
    );
    let definition = FunctionDef {
        name: Some(named("aggregate", 0, 9)),
        outputs: Vec::new(),
        inputs: Vec::new(),
        body: vec![
            declaration(
                DeclarationKind::Global,
                DeclarationContext::Function,
                vec![named("global_array", 17, 29)],
                10,
                29,
            ),
            declaration(
                DeclarationKind::Persistent,
                DeclarationContext::Function,
                vec![named("persistent_array", 31, 47)],
                30,
                47,
            ),
            assignment(global_target, number("2", 58, 59), 40, 60),
            assignment(persistent_target, number("3", 87, 88), 65, 89),
        ],
        span: range(0, 95),
    };
    let module = compile(&file(vec![Stmt {
        kind: StmtKind::Function(definition),
        span: range(0, 95),
        suppress_output: true,
    }]))
    .expect("aggregate declared storage should compile");
    let function = &module.functions[1];
    assert_eq!(function.local_count, 0);
    assert_eq!(function.persistent_slot_count, 1);
    assert!(function.instructions.iter().any(|instruction| matches!(
        instruction.kind,
        InstructionKind::AssignBindingPlace {
            result: None,
            binding: BindingTarget::Workspace(_),
            ..
        }
    )));
    assert!(function.instructions.iter().any(|instruction| matches!(
        instruction.kind,
        InstructionKind::LoadPersistent { slot, .. }
            if slot == PersistentSlot::new(0)
    )));
    assert!(function.instructions.iter().any(|instruction| matches!(
        instruction.kind,
        InstructionKind::AssignBindingPlace {
            result: None,
            binding: BindingTarget::Persistent(slot),
            ..
        } if slot == PersistentSlot::new(0)
    )));
    assert_eq!(
        function
            .instructions
            .iter()
            .filter(|instruction| matches!(instruction.kind, InstructionKind::AssignPlace { .. }))
            .count(),
        0
    );
}

#[test]
fn global_outputs_catch_for_clear_and_ans_all_use_workspace_storage() {
    let protected = assignment(name("temporary", 50, 59), number("1", 62, 63), 50, 64);
    let caught = assignment(name("out", 75, 78), name("problem", 81, 88), 75, 89);
    let definition = FunctionDef {
        name: Some(named("mixed", 8, 13)),
        outputs: vec![named("out", 0, 3)],
        inputs: Vec::new(),
        body: vec![
            declaration(
                DeclarationKind::Global,
                DeclarationContext::Function,
                vec![
                    named("out", 21, 24),
                    named("problem", 25, 32),
                    named("ans", 33, 36),
                ],
                14,
                36,
            ),
            try_statement(
                vec![protected],
                Some((Some(named("problem", 67, 74)), vec![caught])),
                45,
                92,
            ),
            Stmt {
                kind: StmtKind::For {
                    variable: name("problem", 96, 103),
                    iterable: number("2", 106, 107),
                    body: Vec::new(),
                },
                span: range(94, 112),
                suppress_output: true,
            },
            Stmt {
                kind: StmtKind::Clear(ClearStatement {
                    names: vec![named("problem", 119, 126)],
                    form: ClearForm::IdentifierList,
                }),
                span: range(113, 126),
                suppress_output: true,
            },
            Stmt {
                kind: StmtKind::Expr(number("9", 130, 131)),
                span: range(130, 132),
                suppress_output: false,
            },
        ],
        span: range(0, 140),
    };
    let module = compile(&file(vec![Stmt {
        kind: StmtKind::Function(definition),
        span: range(0, 140),
        suppress_output: true,
    }]))
    .expect("global storage consumers should compile");
    let function = &module.functions[1];
    assert_eq!(function.persistent_slot_count, 0);
    assert_eq!(function.local_count, 2);
    assert!(
        function
            .instructions
            .iter()
            .any(|instruction| matches!(instruction.kind, InstructionKind::ForEach { .. }))
    );
    assert!(
        function
            .instructions
            .iter()
            .any(|instruction| matches!(instruction.kind, InstructionKind::ClearGlobal { .. }))
    );
    assert!(function.instructions.iter().any(|instruction| matches!(
        instruction.kind,
        InstructionKind::StatementValue {
            result: StatementResultTarget::Global(_),
            display: true,
            ..
        }
    )));
    assert!(
        function
            .instructions
            .iter()
            .any(|instruction| matches!(instruction.kind, InstructionKind::StoreGlobal { .. }))
    );
    assert!(
        function
            .instructions
            .iter()
            .any(|instruction| matches!(instruction.kind, InstructionKind::LoadGlobal { .. }))
    );
    assert!(matches!(
        function.exception_handlers[0].kind,
        ExceptionHandlerKind::Catch {
            error_local: Some(_),
            ..
        }
    ));
    verify(&module).expect("global catch/output bytecode should verify");
}

#[test]
fn persistent_catch_and_for_bindings_bridge_through_persistent_storage() {
    let definition = FunctionDef {
        name: Some(named("stateful_control", 0, 16)),
        outputs: Vec::new(),
        inputs: Vec::new(),
        body: vec![
            declaration(
                DeclarationKind::Persistent,
                DeclarationContext::Function,
                vec![named("problem", 28, 35), named("item", 36, 40)],
                18,
                40,
            ),
            try_statement(
                vec![assignment(name("x", 45, 46), number("1", 49, 50), 45, 51)],
                Some((
                    Some(named("problem", 58, 65)),
                    vec![assignment(
                        name("x", 68, 69),
                        name("problem", 72, 79),
                        68,
                        80,
                    )],
                )),
                42,
                82,
            ),
            Stmt {
                kind: StmtKind::For {
                    variable: name("item", 87, 91),
                    iterable: number("3", 94, 95),
                    body: Vec::new(),
                },
                span: range(84, 100),
                suppress_output: true,
            },
        ],
        span: range(0, 105),
    };
    let module = compile(&file(vec![Stmt {
        kind: StmtKind::Function(definition),
        span: range(0, 105),
        suppress_output: true,
    }]))
    .expect("persistent control-flow bindings should compile");
    let function = &module.functions[1];
    assert_eq!(function.persistent_slot_count, 2);
    assert!(function.instructions.iter().any(|instruction| matches!(
        instruction.kind,
        InstructionKind::StorePersistent { slot, .. }
            if slot == PersistentSlot::new(0)
    )));
    assert!(function.instructions.iter().any(|instruction| matches!(
        instruction.kind,
        InstructionKind::StorePersistent { slot, .. }
            if slot == PersistentSlot::new(1)
    )));
    assert!(function.instructions.iter().any(|instruction| matches!(
        instruction.kind,
        InstructionKind::LoadPersistent { slot, .. }
            if slot == PersistentSlot::new(0)
    )));
    verify(&module).expect("persistent catch/for bytecode should verify");
}

#[test]
fn unsupported_persistent_ans_and_clear_paths_are_structured() {
    let ans_function = FunctionDef {
        name: Some(named("persistent_ans", 0, 14)),
        outputs: Vec::new(),
        inputs: Vec::new(),
        body: vec![
            declaration(
                DeclarationKind::Persistent,
                DeclarationContext::Function,
                vec![named("ans", 26, 29)],
                16,
                29,
            ),
            Stmt {
                kind: StmtKind::Expr(number("1", 32, 33)),
                span: range(32, 34),
                suppress_output: true,
            },
        ],
        span: range(0, 40),
    };
    let ans_error = compile(&file(vec![Stmt {
        kind: StmtKind::Function(ans_function),
        span: range(0, 40),
        suppress_output: true,
    }]))
    .expect_err("persistent ans needs a dedicated bytecode target");
    assert!(ans_error.diagnostics().iter().any(|diagnostic| matches!(
        diagnostic.kind,
        CompileDiagnosticKind::UnsupportedBindingOperation {
            storage: BindingStorageClass::Persistent,
            operation: BindingOperation::StatementResult,
            ..
        }
    )));

    let clear_function = FunctionDef {
        name: Some(named("persistent_clear", 0, 16)),
        outputs: Vec::new(),
        inputs: Vec::new(),
        body: vec![
            declaration(
                DeclarationKind::Persistent,
                DeclarationContext::Function,
                vec![named("state", 28, 33)],
                18,
                33,
            ),
            Stmt {
                kind: StmtKind::Clear(ClearStatement {
                    names: vec![named("state", 40, 45)],
                    form: ClearForm::IdentifierList,
                }),
                span: range(34, 45),
                suppress_output: true,
            },
        ],
        span: range(0, 50),
    };
    let clear_error = compile(&file(vec![Stmt {
        kind: StmtKind::Function(clear_function),
        span: range(0, 50),
        suppress_output: true,
    }]))
    .expect_err("persistent clear needs a runtime reset contract");
    assert!(clear_error.diagnostics().iter().any(|diagnostic| matches!(
        diagnostic.kind,
        CompileDiagnosticKind::UnsupportedBindingOperation {
            storage: BindingStorageClass::Persistent,
            operation: BindingOperation::NamedClear,
            ..
        }
    )));
}

#[test]
fn persistent_conflicts_have_specific_binding_diagnostics() {
    let definition = FunctionDef {
        name: Some(named("conflicts", 10, 19)),
        outputs: vec![named("output_name", 0, 11)],
        inputs: vec![named("input_name", 20, 30)],
        body: vec![
            declaration(
                DeclarationKind::Global,
                DeclarationContext::Function,
                vec![named("global_name", 38, 49)],
                31,
                49,
            ),
            assignment(name("assigned", 51, 59), number("1", 62, 63), 51, 64),
            Stmt {
                kind: StmtKind::Expr(name("used", 66, 70)),
                span: range(66, 71),
                suppress_output: true,
            },
            declaration(
                DeclarationKind::Persistent,
                DeclarationContext::Function,
                vec![
                    named("input_name", 82, 92),
                    named("output_name", 93, 104),
                    named("global_name", 105, 116),
                    named("assigned", 117, 125),
                    named("used", 126, 130),
                    named("duplicate", 131, 140),
                    named("later_global", 141, 153),
                ],
                72,
                153,
            ),
            declaration(
                DeclarationKind::Persistent,
                DeclarationContext::Function,
                vec![named("duplicate", 165, 174)],
                155,
                174,
            ),
            declaration(
                DeclarationKind::Global,
                DeclarationContext::Function,
                vec![named("later_global", 182, 194)],
                176,
                194,
            ),
        ],
        span: range(0, 198),
    };
    let error = compile(&file(vec![Stmt {
        kind: StmtKind::Function(definition),
        span: range(0, 198),
        suppress_output: true,
    }]))
    .expect_err("persistent conflicts must be diagnosed");
    let problems = error
        .diagnostics()
        .iter()
        .filter_map(|diagnostic| match diagnostic.kind {
            CompileDiagnosticKind::InvalidBindingDeclaration { problem, .. } => Some(problem),
            _ => None,
        })
        .collect::<Vec<_>>();
    for expected in [
        BindingDeclarationProblem::ConflictsWithInput,
        BindingDeclarationProblem::ConflictsWithOutput,
        BindingDeclarationProblem::ConflictsWithGlobal,
        BindingDeclarationProblem::AssignedBeforeDeclaration,
        BindingDeclarationProblem::UsedBeforeDeclaration,
        BindingDeclarationProblem::DuplicatePersistent,
        BindingDeclarationProblem::ConflictsWithPersistent,
    ] {
        assert!(problems.contains(&expected), "missing {expected:?}");
    }
    assert!(
        error
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.code() == "OMC0011")
            .count()
            >= 7
    );
}

#[test]
fn declaration_context_and_form_are_checked_before_emission() {
    let script_persistent = declaration(
        DeclarationKind::Persistent,
        DeclarationContext::Script,
        vec![named("state", 11, 16)],
        0,
        16,
    );
    let error = compile(&file(vec![script_persistent]))
        .expect_err("persistent declarations are invalid in scripts");
    assert!(matches!(
        error.diagnostics()[0].kind,
        CompileDiagnosticKind::InvalidBindingDeclaration {
            problem: BindingDeclarationProblem::InvalidContext(DeclarationContext::Script),
            ..
        }
    ));

    let malformed = Stmt {
        kind: StmtKind::Declaration(DeclarationStatement {
            kind: DeclarationKind::Global,
            names: vec![named("recovered", 20, 29)],
            form: DeclarationForm::InvalidItems,
            context: DeclarationContext::Function,
        }),
        span: range(10, 30),
        suppress_output: true,
    };
    let definition = FunctionDef {
        name: Some(named("bad_form", 0, 8)),
        outputs: Vec::new(),
        inputs: Vec::new(),
        body: vec![malformed],
        span: range(0, 35),
    };
    let error = compile(&file(vec![Stmt {
        kind: StmtKind::Function(definition),
        span: range(0, 35),
        suppress_output: true,
    }]))
    .expect_err("malformed declaration form must not emit bytecode");
    assert!(matches!(
        error.diagnostics()[0].kind,
        CompileDiagnosticKind::InvalidBindingDeclaration {
            problem: BindingDeclarationProblem::InvalidForm(DeclarationForm::InvalidItems),
            ..
        }
    ));
}

#[test]
fn script_global_and_function_global_after_use_compile_without_fake_warning() {
    let declaration_span = range(0, 13);
    let script = file(vec![
        declaration(
            DeclarationKind::Global,
            DeclarationContext::Script,
            vec![named("shared", 7, 13)],
            declaration_span.start(),
            declaration_span.end(),
        ),
        assignment(name("shared", 15, 21), number("4", 24, 25), 15, 26),
        assignment(name("copy", 28, 32), name("shared", 35, 41), 28, 42),
        assignment(name("ordinary", 44, 52), number("1", 55, 56), 44, 57),
        assignment(
            name("ordinary_copy", 59, 72),
            name("ordinary", 75, 83),
            59,
            84,
        ),
    ]);
    let module = compile(&script).expect("script global should compile");
    let entry = &module.functions[0];
    assert!(entry.instructions.iter().any(|instruction| {
        matches!(instruction.kind, InstructionKind::DeclareGlobal { .. })
            && instruction.location
                == Some(SourceLocation::new(
                    SOURCE.raw(),
                    declaration_span.start(),
                    declaration_span.end(),
                ))
    }));
    assert!(
        entry
            .instructions
            .iter()
            .any(|instruction| matches!(instruction.kind, InstructionKind::StoreGlobal { .. }))
    );
    assert!(
        entry
            .instructions
            .iter()
            .any(|instruction| matches!(instruction.kind, InstructionKind::LoadGlobal { .. }))
    );
    assert!(entry.instructions.iter().any(|instruction| matches!(
        instruction.kind,
        InstructionKind::LoadGlobal {
            construct_if_class: true,
            ..
        }
    )));

    let definition = FunctionDef {
        name: Some(named("late_global", 60, 71)),
        outputs: Vec::new(),
        inputs: Vec::new(),
        body: vec![
            assignment(name("late", 75, 79), number("1", 82, 83), 75, 84),
            declaration(
                DeclarationKind::Global,
                DeclarationContext::Function,
                vec![named("late", 92, 96)],
                85,
                96,
            ),
        ],
        span: range(55, 100),
    };
    let module = compile(&file(vec![Stmt {
        kind: StmtKind::Function(definition),
        span: range(55, 100),
        suppress_output: true,
    }]))
    .expect("global-after-use warning channel is intentionally absent");
    let function = &module.functions[1];
    let store = function
        .instructions
        .iter()
        .position(|instruction| matches!(instruction.kind, InstructionKind::StoreGlobal { .. }))
        .expect("pre-declaration global store");
    let declaration = function
        .instructions
        .iter()
        .position(|instruction| matches!(instruction.kind, InstructionKind::DeclareGlobal { .. }))
        .expect("located global declaration");
    assert!(store < declaration);
    assert_eq!(function.local_count, 0);
}

#[test]
fn anonymous_closures_capture_persistent_values_without_local_fallback() {
    let definition = FunctionDef {
        name: Some(named("factory", 0, 7)),
        outputs: vec![named("handle", 8, 14)],
        inputs: Vec::new(),
        body: vec![
            declaration(
                DeclarationKind::Persistent,
                DeclarationContext::Function,
                vec![named("state", 26, 31)],
                16,
                31,
            ),
            assignment(name("state", 34, 39), number("5", 42, 43), 34, 44),
            assignment(
                name("handle", 46, 52),
                anonymous(Vec::new(), name("state", 59, 64), 55, 64),
                46,
                65,
            ),
        ],
        span: range(0, 70),
    };
    let module = compile(&file(vec![Stmt {
        kind: StmtKind::Function(definition),
        span: range(0, 70),
        suppress_output: true,
    }]))
    .expect("persistent closure capture should compile");
    let factory = &module.functions[1];
    assert_eq!(factory.persistent_slot_count, 1);
    assert!(factory.instructions.iter().any(|instruction| matches!(
        instruction.kind,
        InstructionKind::LoadPersistent { slot, .. }
            if slot == PersistentSlot::new(0)
    )));
    assert!(factory.instructions.iter().any(|instruction| matches!(
        &instruction.kind,
        InstructionKind::MakeClosure { captures, .. }
            if matches!(captures.as_slice(), [(_, Some(_), true)])
    )));
    verify(&module).expect("persistent capture bytecode should verify");
}

#[test]
fn compiles_abstract_sealed_class_and_bodyless_static_signature() {
    let class = ClassDef {
        name: Some(named("Contract", 0, 8)),
        superclass: None,
        enumeration_blocks: Vec::new(),
        event_blocks: Vec::new(),
        attributes: vec![
            Attribute {
                name: Some(named("Abstract", 10, 18)),
                value: None,
                span: range(10, 18),
            },
            Attribute {
                name: Some(named("Sealed", 20, 26)),
                value: Some(name("true", 29, 33)),
                span: range(20, 33),
            },
        ],
        property_blocks: Vec::new(),
        method_blocks: vec![MethodBlock {
            attributes: vec![
                Attribute {
                    name: Some(named("Abstract", 40, 48)),
                    value: None,
                    span: range(40, 48),
                },
                Attribute {
                    name: Some(named("Static", 50, 56)),
                    value: None,
                    span: range(50, 56),
                },
                Attribute {
                    name: Some(named("Access", 58, 64)),
                    value: Some(name("protected", 67, 76)),
                    span: range(58, 76),
                },
            ],
            methods: Vec::new(),
            declarations: vec![MethodDeclaration {
                name: Some(named("make", 90, 94)),
                outputs: vec![named("value", 80, 85)],
                inputs: vec![named("input", 95, 100)],
                span: range(80, 101),
            }],
            span: range(35, 110),
        }],
        span: range(0, 115),
    };

    let module = compile(&file(vec![Stmt {
        kind: StmtKind::Class(class),
        span: range(0, 115),
        suppress_output: true,
    }]))
    .expect("abstract sealed class should compile");
    verify(&module).expect("abstract class bytecode should verify");

    let class = &module.classes[0];
    assert_eq!(class.declared_abstract, Some(true));
    assert!(class.sealed);
    let method = &class.methods[0];
    assert!(method.is_abstract);
    assert_eq!(method.kind, BytecodeMethodKind::Static);
    assert_eq!(method.access, BytecodeAccess::Protected);
    let signature = &module.functions[method.function.get() as usize];
    assert_eq!(signature.parameter_count, 1);
}

#[test]
fn diagnoses_duplicate_and_case_mismatched_class_attributes_structurally() {
    let class = ClassDef {
        name: Some(named("Broken", 0, 6)),
        superclass: None,
        attributes: vec![
            Attribute {
                name: Some(named("Abstract", 10, 18)),
                value: None,
                span: range(10, 18),
            },
            Attribute {
                name: Some(named("Abstract", 20, 28)),
                value: None,
                span: range(20, 28),
            },
            Attribute {
                name: Some(named("sealed", 30, 36)),
                value: None,
                span: range(30, 36),
            },
        ],
        property_blocks: Vec::new(),
        method_blocks: Vec::new(),
        enumeration_blocks: Vec::new(),
        event_blocks: Vec::new(),
        span: range(0, 40),
    };
    let error = compile(&file(vec![Stmt {
        kind: StmtKind::Class(class),
        span: range(0, 40),
        suppress_output: true,
    }]))
    .expect_err("invalid attributes must fail compilation");

    assert!(error.diagnostics().iter().any(|diagnostic| matches!(
        diagnostic.kind,
        CompileDiagnosticKind::InvalidClassAttribute {
            problem: ClassAttributeProblem::Duplicate,
            ..
        }
    )));
    assert!(error.diagnostics().iter().any(|diagnostic| matches!(
        &diagnostic.kind,
        CompileDiagnosticKind::Unsupported {
            feature: UnsupportedFeature::ClassAttribute(name)
        } if name == "sealed"
    )));
}

#[test]
fn diagnoses_explicit_concrete_class_with_abstract_signature() {
    let class = ClassDef {
        name: Some(named("ConcreteClaim", 0, 13)),
        superclass: None,
        enumeration_blocks: Vec::new(),
        event_blocks: Vec::new(),
        attributes: vec![Attribute {
            name: Some(named("Abstract", 15, 23)),
            value: Some(name("false", 26, 31)),
            span: range(15, 31),
        }],
        property_blocks: Vec::new(),
        method_blocks: vec![MethodBlock {
            attributes: vec![Attribute {
                name: Some(named("Abstract", 35, 43)),
                value: None,
                span: range(35, 43),
            }],
            methods: Vec::new(),
            declarations: vec![MethodDeclaration {
                name: Some(named("run", 55, 58)),
                outputs: vec![named("value", 47, 52)],
                inputs: vec![named("object", 59, 65)],
                span: range(47, 66),
            }],
            span: range(33, 70),
        }],
        span: range(0, 75),
    };
    let error = compile(&file(vec![Stmt {
        kind: StmtKind::Class(class),
        span: range(0, 75),
        suppress_output: true,
    }]))
    .expect_err("Abstract=false must reject a local abstract slot");

    assert!(error.diagnostics().iter().any(|diagnostic| matches!(
        diagnostic.kind,
        CompileDiagnosticKind::InvalidClassAttribute {
            problem: ClassAttributeProblem::ExplicitConcreteWithAbstractMethods,
            ..
        }
    )));
}

#[test]
fn compiles_enum_members_and_event_access_into_versioned_class_features() {
    let class = ClassDef {
        name: Some(named("Traffic", 0, 7)),
        superclass: Some(named("handle", 10, 16)),
        attributes: Vec::new(),
        property_blocks: Vec::new(),
        method_blocks: Vec::new(),
        enumeration_blocks: vec![EnumerationBlock {
            attributes: Vec::new(),
            members: vec![
                EnumMemberDef {
                    name: Some(named("Stop", 30, 34)),
                    arguments: vec![number("0", 36, 37)],
                    span: range(30, 38),
                },
                EnumMemberDef {
                    name: Some(named("Go", 40, 42)),
                    arguments: vec![number("2", 44, 45)],
                    span: range(40, 46),
                },
            ],
            span: range(20, 50),
        }],
        event_blocks: vec![EventBlock {
            attributes: vec![
                Attribute {
                    name: Some(named("ListenAccess", 55, 67)),
                    value: Some(name("protected", 70, 79)),
                    span: range(55, 79),
                },
                Attribute {
                    name: Some(named("NotifyAccess", 81, 93)),
                    value: Some(name("private", 96, 103)),
                    span: range(81, 103),
                },
                Attribute {
                    name: Some(named("Hidden", 105, 111)),
                    value: None,
                    span: range(105, 111),
                },
            ],
            events: vec![EventDef {
                name: Some(named("Changed", 115, 122)),
                span: range(115, 122),
            }],
            span: range(52, 125),
        }],
        span: range(0, 130),
    };

    let module = compile(&file(vec![Stmt {
        kind: StmtKind::Class(class),
        span: range(0, 130),
        suppress_output: true,
    }]))
    .expect("enum and event metadata should compile");

    verify(&module).expect("class features should verify");
    assert_eq!(module.version.major, 29);
    assert_eq!(module.class_features.len(), 1);
    let feature = &module.class_features[0];
    assert_eq!(feature.kind, ClassKind::Enumeration);
    assert_eq!(feature.enumeration_base.as_deref(), Some("handle"));
    assert_eq!(feature.enumeration_members.len(), 2);
    assert_eq!(feature.enumeration_members[0].argument_count, 1);
    assert_eq!(feature.events[0].listen_access, BytecodeAccess::Protected);
    assert_eq!(feature.events[0].notify_access, BytecodeAccess::Private);
    assert!(feature.events[0].hidden);
    assert!(!module.classes[0].sealed);
}

#[test]
fn compiles_abstract_property_and_abstract_sealed_method_slots() {
    let class = ClassDef {
        name: Some(named("Contract", 0, 8)),
        superclass: None,
        attributes: vec![Attribute {
            name: Some(named("Abstract", 10, 18)),
            value: None,
            span: range(10, 18),
        }],
        property_blocks: vec![
            PropertyBlock {
                attributes: vec![
                    Attribute {
                        name: Some(named("Abstract", 25, 33)),
                        value: None,
                        span: range(25, 33),
                    },
                    Attribute {
                        name: Some(named("Dependent", 35, 44)),
                        value: None,
                        span: range(35, 44),
                    },
                ],
                properties: vec![PropertyDef {
                    name: Some(named("Value", 46, 51)),
                    default: None,
                    span: range(46, 51),
                }],
                span: range(20, 52),
            },
            PropertyBlock {
                attributes: vec![
                    Attribute {
                        name: Some(named("Abstract", 53, 61)),
                        value: None,
                        span: range(53, 61),
                    },
                    Attribute {
                        name: Some(named("Constant", 62, 70)),
                        value: None,
                        span: range(62, 70),
                    },
                ],
                properties: vec![PropertyDef {
                    name: Some(named("Code", 71, 75)),
                    default: None,
                    span: range(71, 75),
                }],
                span: range(53, 76),
            },
        ],
        method_blocks: vec![MethodBlock {
            attributes: vec![
                Attribute {
                    name: Some(named("Abstract", 60, 68)),
                    value: None,
                    span: range(60, 68),
                },
                Attribute {
                    name: Some(named("Sealed", 70, 76)),
                    value: None,
                    span: range(70, 76),
                },
            ],
            methods: Vec::new(),
            declarations: vec![MethodDeclaration {
                name: Some(named("run", 85, 88)),
                outputs: vec![named("value", 79, 84)],
                inputs: vec![named("object", 89, 95)],
                span: range(79, 96),
            }],
            span: range(58, 100),
        }],
        enumeration_blocks: Vec::new(),
        event_blocks: Vec::new(),
        span: range(0, 105),
    };

    let module = compile(&file(vec![Stmt {
        kind: StmtKind::Class(class),
        span: range(0, 105),
        suppress_output: true,
    }]))
    .expect("abstract property and sealed abstract method should compile");
    verify(&module).expect("abstract feature metadata should verify");
    let feature = &module.class_features[0];
    assert_eq!(feature.abstract_properties[0].name, "Value");
    assert_eq!(
        feature.abstract_properties[0].kind,
        BytecodePropertyKind::Dependent
    );
    assert_eq!(feature.abstract_properties[1].name, "Code");
    assert_eq!(
        feature.abstract_properties[1].kind,
        BytecodePropertyKind::Constant
    );
    assert_eq!(feature.abstract_properties[1].set_access, None);
    assert_eq!(feature.sealed_methods, ["run"]);
}

#[test]
fn diagnoses_empty_duplicate_and_nonhandle_class_features() {
    let class = ClassDef {
        name: Some(named("Broken", 0, 6)),
        superclass: None,
        attributes: vec![Attribute {
            name: Some(named("Abstract", 7, 15)),
            value: None,
            span: range(7, 15),
        }],
        property_blocks: Vec::new(),
        method_blocks: Vec::new(),
        enumeration_blocks: vec![EnumerationBlock {
            attributes: Vec::new(),
            members: Vec::new(),
            span: range(10, 20),
        }],
        event_blocks: vec![EventBlock {
            attributes: Vec::new(),
            events: vec![EventDef {
                name: Some(named("Changed", 25, 32)),
                span: range(25, 32),
            }],
            span: range(22, 35),
        }],
        span: range(0, 40),
    };
    let error = compile(&file(vec![Stmt {
        kind: StmtKind::Class(class),
        span: range(0, 40),
        suppress_output: true,
    }]))
    .expect_err("empty enum and value-class events must fail");

    assert!(error.diagnostics().iter().any(|diagnostic| matches!(
        diagnostic.kind,
        CompileDiagnosticKind::InvalidClassMember {
            problem: ClassMemberProblem::EmptyEnumeration,
            ..
        }
    )));
    assert!(error.diagnostics().iter().any(|diagnostic| matches!(
        diagnostic.kind,
        CompileDiagnosticKind::InvalidClassMember {
            problem: ClassMemberProblem::EventsRequireHandleClass,
            ..
        }
    )));
    assert!(error.diagnostics().iter().any(|diagnostic| matches!(
        diagnostic.kind,
        CompileDiagnosticKind::InvalidClassAttribute {
            problem: ClassAttributeProblem::EnumerationCannotBeAbstract,
            ..
        }
    )));
}

#[test]
fn diagnoses_class_feature_name_conflicts_and_local_enum_inheritance() {
    let conflicting = ClassDef {
        name: Some(named("Signals", 0, 7)),
        superclass: Some(named("handle", 10, 16)),
        attributes: Vec::new(),
        property_blocks: Vec::new(),
        method_blocks: Vec::new(),
        enumeration_blocks: vec![EnumerationBlock {
            attributes: Vec::new(),
            members: vec![EnumMemberDef {
                name: Some(named("Changed", 20, 27)),
                arguments: Vec::new(),
                span: range(20, 27),
            }],
            span: range(18, 30),
        }],
        event_blocks: vec![EventBlock {
            attributes: Vec::new(),
            events: vec![EventDef {
                name: Some(named("Changed", 35, 42)),
                span: range(35, 42),
            }],
            span: range(32, 45),
        }],
        span: range(0, 50),
    };
    let conflict_error = compile(&file(vec![Stmt {
        kind: StmtKind::Class(conflicting),
        span: range(0, 50),
        suppress_output: true,
    }]))
    .expect_err("enum and event names share a class namespace");
    assert!(
        conflict_error
            .diagnostics()
            .iter()
            .any(|diagnostic| matches!(
                diagnostic.kind,
                CompileDiagnosticKind::InvalidClassMember {
                    problem: ClassMemberProblem::DuplicateOrConflictingDeclaration { .. },
                    ..
                }
            ))
    );

    let base = ClassDef {
        name: Some(named("State", 60, 65)),
        superclass: None,
        attributes: Vec::new(),
        property_blocks: Vec::new(),
        method_blocks: Vec::new(),
        enumeration_blocks: vec![EnumerationBlock {
            attributes: Vec::new(),
            members: vec![EnumMemberDef {
                name: Some(named("Ready", 70, 75)),
                arguments: Vec::new(),
                span: range(70, 75),
            }],
            span: range(68, 78),
        }],
        event_blocks: Vec::new(),
        span: range(60, 80),
    };
    let child = ClassDef {
        name: Some(named("ChildState", 82, 92)),
        superclass: Some(named("State", 95, 100)),
        attributes: Vec::new(),
        property_blocks: Vec::new(),
        method_blocks: Vec::new(),
        enumeration_blocks: Vec::new(),
        event_blocks: Vec::new(),
        span: range(82, 105),
    };
    let inheritance_error = compile(&file(vec![
        Stmt {
            kind: StmtKind::Class(base),
            span: range(60, 80),
            suppress_output: true,
        },
        Stmt {
            kind: StmtKind::Class(child),
            span: range(82, 105),
            suppress_output: true,
        },
    ]))
    .expect_err("enumeration classes cannot be inherited");
    assert!(
        inheritance_error
            .diagnostics()
            .iter()
            .any(|diagnostic| matches!(
                diagnostic.kind,
                CompileDiagnosticKind::InvalidClassMember {
                    problem: ClassMemberProblem::EnumerationSuperclass { .. },
                    ..
                }
            ))
    );
}
