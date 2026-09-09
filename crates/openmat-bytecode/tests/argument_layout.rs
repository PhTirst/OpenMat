use openmat_bytecode::{
    ArgumentLayout, BytecodeModule, BytecodeVersion, Function, FunctionId, Instruction,
    InstructionKind, LocalSlot, VerificationErrorKind, verify,
};

fn module() -> BytecodeModule {
    let mut function = Function::new("f", 0, 2, 2);
    function.argument_layout = Some(ArgumentLayout {
        positional_count: 1,
        required_count: 1,
        repeating_count: 0,
        named: vec![(LocalSlot::new(1), "Scale".into())],
    });
    function
        .instructions
        .push(Instruction::new(InstructionKind::Return {
            values: Vec::new(),
        }));
    BytecodeModule::new(vec![function], FunctionId::new(0))
}

#[test]
fn argument_layout_is_versioned_and_validated() {
    let valid = module();
    verify(&valid).unwrap();
    let mut legacy = valid.clone();
    legacy.version = BytecodeVersion::new(27, 0);
    assert_eq!(
        verify(&legacy).unwrap_err().kind,
        VerificationErrorKind::InvalidArgumentLayout
    );
    for layout in [
        ArgumentLayout {
            positional_count: 1,
            required_count: 2,
            repeating_count: 0,
            named: vec![(LocalSlot::new(1), "Scale".into())],
        },
        ArgumentLayout {
            positional_count: 1,
            required_count: 1,
            repeating_count: 0,
            named: vec![(LocalSlot::new(2), "Scale".into())],
        },
        ArgumentLayout {
            positional_count: 1,
            required_count: 1,
            repeating_count: 0,
            named: vec![],
        },
        ArgumentLayout {
            positional_count: 1,
            required_count: 1,
            repeating_count: 0,
            named: vec![
                (LocalSlot::new(1), "Scale".into()),
                (LocalSlot::new(1), "Scale".into()),
            ],
        },
    ] {
        let mut invalid = valid.clone();
        invalid.functions[0].argument_layout = Some(layout);
        assert_eq!(
            verify(&invalid).unwrap_err().kind,
            VerificationErrorKind::InvalidArgumentLayout
        );
    }
}

#[test]
fn repeating_layout_requires_v29_and_covers_disjoint_signature_slots() {
    let mut valid = module();
    let function = &mut valid.functions[0];
    function.local_count = 4;
    function.parameter_count = 4;
    function.argument_layout = Some(ArgumentLayout {
        positional_count: 1,
        required_count: 1,
        repeating_count: 2,
        named: vec![(LocalSlot::new(3), "Scale".into())],
    });
    verify(&valid).unwrap();
    let mut old = module();
    old.version = BytecodeVersion::new(28, 0);
    verify(&old).unwrap();
    let mut invalid = valid.clone();
    invalid.version = BytecodeVersion::new(28, 0);
    assert_eq!(
        verify(&invalid).unwrap_err().kind,
        VerificationErrorKind::InvalidArgumentLayout
    );
    for (fixed, repeated, owner) in [(1, 3, 3), (1, 2, 2), (1, u32::MAX, 3), (u32::MAX, 1, 3)] {
        let mut invalid = valid.clone();
        let layout = invalid.functions[0].argument_layout.as_mut().unwrap();
        layout.positional_count = fixed;
        layout.repeating_count = repeated;
        layout.named[0].0 = LocalSlot::new(owner);
        assert_eq!(
            verify(&invalid).unwrap_err().kind,
            VerificationErrorKind::InvalidArgumentLayout
        );
    }
    let mut invalid = valid;
    invalid.functions[0].register_count = 1;
    invalid.functions[0].instructions.insert(
        0,
        Instruction::new(InstructionKind::LoadVariadicInputs {
            dst: openmat_bytecode::Register::new(0),
        }),
    );
    assert_eq!(
        verify(&invalid).unwrap_err().kind,
        VerificationErrorKind::InvalidArgumentLayout
    );
}
