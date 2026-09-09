use openmat_bytecode::{
    ApplyArgument, BytecodeModule, BytecodeVersion, Constant, ConstantId, Function, FunctionId,
    Instruction, InstructionKind, PackApplyTarget, PackRegister, PlaceStep, Register,
    SourceLocation, VerificationErrorKind, verify,
};

fn module(kind: InstructionKind) -> BytecodeModule {
    let mut function = Function::new("probe", 4, 0, 0);
    function.pack_register_count = 2;
    function.constants = vec![Constant::String("field".into()), Constant::Double(1.0)];
    function.instructions = vec![Instruction::located(kind, SourceLocation::new(1, 10, 20))];
    BytecodeModule::new(vec![function], FunctionId::new(0))
}

fn operations() -> Vec<InstructionKind> {
    vec![
        InstructionKind::RequireSinglePack {
            pack: PackRegister::new(0),
        },
        InstructionKind::RequireDefined {
            value: Register::new(0),
            name: ConstantId::new(0),
        },
        InstructionKind::CountPlaceOutputs {
            dst: Register::new(0),
            root: Register::new(1),
            path: vec![PlaceStep::Brace(vec![ApplyArgument::Expand(
                PackRegister::new(0),
            )])],
        },
        InstructionKind::ApplyOutputPack {
            dst_pack: PackRegister::new(0),
            count: Register::new(1),
            target: PackApplyTarget::Value(Register::new(2)),
            arguments: vec![ApplyArgument::Value(Register::new(3))],
        },
        InstructionKind::ApplyOutputPack {
            dst_pack: PackRegister::new(0),
            count: Register::new(1),
            target: PackApplyTarget::Field {
                object: Register::new(2),
                name: ConstantId::new(0),
            },
            arguments: vec![],
        },
        InstructionKind::ApplyOutputPack {
            dst_pack: PackRegister::new(0),
            count: Register::new(1),
            target: PackApplyTarget::Qualified {
                target: Register::new(2),
                unresolved_suffix: Register::new(3),
                members: vec![ConstantId::new(0)],
            },
            arguments: vec![],
        },
        InstructionKind::SlicePack {
            dst_pack: PackRegister::new(0),
            source: PackRegister::new(1),
            start: Register::new(2),
            count: Register::new(3),
        },
    ]
}

#[test]
fn dynamic_instructions_verify_only_in_v27_and_preserve_locations() {
    for operation in operations() {
        let mut module = module(operation);
        verify(&module).unwrap();
        for major in 24..=26 {
            module.version = BytecodeVersion::new(major, 0);
            let error = verify(&module).unwrap_err();
            assert!(matches!(
                error.kind,
                VerificationErrorKind::DynamicOutputsRequireCurrentVersion { .. }
            ));
            assert_eq!(error.location, Some(SourceLocation::new(1, 10, 20)));
        }
    }
}

#[test]
fn every_new_scalar_and_pack_operand_is_frame_bounded() {
    for mut operation in operations() {
        match &mut operation {
            InstructionKind::RequireSinglePack { .. } => continue,
            InstructionKind::RequireDefined { value, .. } => *value = Register::new(4),
            InstructionKind::CountPlaceOutputs { root, .. } => *root = Register::new(4),
            InstructionKind::ApplyOutputPack { count, .. }
            | InstructionKind::SlicePack { count, .. } => *count = Register::new(4),
            _ => unreachable!(),
        }
        assert!(matches!(
            verify(&module(operation)).unwrap_err().kind,
            VerificationErrorKind::InvalidRegister { .. }
        ));
    }
    for operation in [
        InstructionKind::RequireSinglePack {
            pack: PackRegister::new(2),
        },
        InstructionKind::ApplyOutputPack {
            dst_pack: PackRegister::new(2),
            count: Register::new(0),
            target: PackApplyTarget::Value(Register::new(1)),
            arguments: vec![],
        },
        InstructionKind::SlicePack {
            dst_pack: PackRegister::new(0),
            source: PackRegister::new(2),
            start: Register::new(0),
            count: Register::new(1),
        },
        InstructionKind::CountPlaceOutputs {
            dst: Register::new(0),
            root: Register::new(1),
            path: vec![PlaceStep::Brace(vec![ApplyArgument::Expand(
                PackRegister::new(2),
            )])],
        },
    ] {
        assert!(matches!(
            verify(&module(operation)).unwrap_err().kind,
            VerificationErrorKind::InvalidPackRegister { .. }
        ));
    }
}

#[test]
fn malformed_places_and_non_string_members_are_rejected() {
    let invalid = InstructionKind::CountPlaceOutputs {
        dst: Register::new(0),
        root: Register::new(1),
        path: vec![],
    };
    assert_eq!(
        verify(&module(invalid)).unwrap_err().kind,
        VerificationErrorKind::EmptyPlacePath
    );
    for target in [
        PackApplyTarget::Field {
            object: Register::new(0),
            name: ConstantId::new(1),
        },
        PackApplyTarget::Qualified {
            target: Register::new(0),
            unresolved_suffix: Register::new(1),
            members: vec![ConstantId::new(1)],
        },
    ] {
        assert!(
            verify(&module(InstructionKind::ApplyOutputPack {
                dst_pack: PackRegister::new(0),
                count: Register::new(1),
                target,
                arguments: vec![]
            }))
            .is_err()
        );
    }
}
