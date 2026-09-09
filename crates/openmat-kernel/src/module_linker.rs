use std::collections::BTreeMap;

use openmat_bytecode::{
    ApplyArgument, BindingTarget, BytecodeModule, ClassDefinitionId, Constant, ConstantId,
    ExceptionHandler, ExceptionHandlerKind, FieldOperand, Function, FunctionId, Instruction,
    InstructionIndex, InstructionKind, LocalSlot, PackApplyTarget, PackRegister, PersistentSlot,
    PlaceStep, Register, StatementResultTarget, ValueSource,
};

use crate::EngineError;

pub(crate) struct MergedModule {
    pub(crate) entry: Function,
    functions: BTreeMap<u32, FunctionId>,
    pub(crate) classes: BTreeMap<String, ClassDefinitionId>,
}

impl MergedModule {
    pub(crate) fn function(&self, original: FunctionId) -> Option<FunctionId> {
        self.functions.get(&original.get()).copied()
    }
}

pub(crate) fn merge_support_module(
    target: &mut BytecodeModule,
    support: &BytecodeModule,
) -> Result<MergedModule, EngineError> {
    if target.version != support.version {
        return Err(link_error("source bytecode versions do not match"));
    }
    let function_base = u32::try_from(target.functions.len())
        .map_err(|_| link_error("linked function table exceeds the bytecode limit"))?;
    let class_base = u32::try_from(target.classes.len())
        .map_err(|_| link_error("linked class table exceeds the bytecode limit"))?;

    let mut functions = BTreeMap::new();
    for original in 1..support.functions.len() {
        let original = u32::try_from(original)
            .map_err(|_| link_error("support function index exceeds the bytecode limit"))?;
        let linked = function_base
            .checked_add(original - 1)
            .ok_or_else(|| link_error("linked function index overflowed"))?;
        functions.insert(original, FunctionId::new(linked));
    }
    let mut class_ids = BTreeMap::new();
    for (index, class) in support.classes.iter().enumerate() {
        let index = u32::try_from(index)
            .map_err(|_| link_error("support class index exceeds the bytecode limit"))?;
        let linked = class_base
            .checked_add(index)
            .ok_or_else(|| link_error("linked class index overflowed"))?;
        class_ids.insert(class.name.clone(), ClassDefinitionId::new(linked));
    }

    let mut entry = support
        .functions
        .get(support.entry.get() as usize)
        .cloned()
        .ok_or_else(|| link_error("support module entry is missing"))?;
    remap_function(&mut entry, &functions, class_base)?;

    for function in support.functions.iter().skip(1) {
        let mut function = function.clone();
        remap_function(&mut function, &functions, class_base)?;
        target.functions.push(function);
    }
    for class in &support.classes {
        let mut class = class.clone();
        for property in &mut class.properties {
            if let Some(default) = property.default {
                property.default = Some(map_function(default, &functions)?);
            }
        }
        for method in &mut class.methods {
            method.function = map_function(method.function, &functions)?;
        }
        target.classes.push(class);
    }

    Ok(MergedModule {
        entry,
        functions,
        classes: class_ids,
    })
}

pub(crate) fn replace_load_with_function(
    function: &mut Function,
    instruction_index: usize,
    destination: Register,
    linked: FunctionId,
) -> Result<(), EngineError> {
    let constant_index = u32::try_from(function.constants.len())
        .map_err(|_| link_error("linked constant table exceeds the bytecode limit"))?;
    function.constants.push(Constant::Function(linked));
    let instruction = function
        .instructions
        .get_mut(instruction_index)
        .ok_or_else(|| link_error("load instruction disappeared during linking"))?;
    instruction.kind = InstructionKind::LoadConstant {
        dst: destination,
        constant: ConstantId::new(constant_index),
    };
    Ok(())
}

pub(crate) fn prepend_entry_instructions(
    function: &mut Function,
    entry: Function,
) -> Result<(), EngineError> {
    splice_entry(function, 0, 0, entry).map(|_| ())
}

pub(crate) fn inline_script_entry(
    function: &mut Function,
    instruction_index: usize,
    entry: Function,
) -> Result<usize, EngineError> {
    splice_entry(function, instruction_index, 1, entry)
}

fn splice_entry(
    target: &mut Function,
    instruction_index: usize,
    removed: usize,
    mut entry: Function,
) -> Result<usize, EngineError> {
    if entry.argument_layout.is_some() {
        return Err(link_error(
            "a validated function cannot be inlined as a script",
        ));
    }
    if instruction_index > target.instructions.len()
        || instruction_index.saturating_add(removed) > target.instructions.len()
    {
        return Err(link_error(
            "instruction splice is outside the caller function",
        ));
    }
    let constant_base = u32::try_from(target.constants.len())
        .map_err(|_| link_error("linked constant table exceeds the bytecode limit"))?;
    let register_base = target.register_count;
    let linked_register_count = target
        .register_count
        .checked_add(entry.register_count)
        .ok_or_else(|| link_error("linked register table exceeds the bytecode limit"))?;
    let pack_register_base = target.pack_register_count;
    let linked_pack_register_count = target
        .pack_register_count
        .checked_add(entry.pack_register_count)
        .ok_or_else(|| link_error("linked pack-register table exceeds the bytecode limit"))?;
    let local_base = target.local_count;
    let linked_local_count = target
        .local_count
        .checked_add(entry.local_count)
        .ok_or_else(|| link_error("linked local table exceeds the bytecode limit"))?;
    let persistent_slot_base = target.persistent_slot_count;
    let linked_persistent_slot_count = target
        .persistent_slot_count
        .checked_add(entry.persistent_slot_count)
        .ok_or_else(|| link_error("linked persistent-slot table exceeds the bytecode limit"))?;

    let final_return = entry
        .instructions
        .last()
        .is_some_and(|instruction| matches!(instruction.kind, InstructionKind::Return { .. }));
    let copied_len = entry
        .instructions
        .len()
        .saturating_sub(usize::from(final_return));
    let next_index = instruction_index
        .checked_add(copied_len)
        .ok_or_else(|| link_error("linked instruction index overflowed"))?;
    let next_index_u32 = u32::try_from(next_index)
        .map_err(|_| link_error("linked instruction table exceeds the bytecode limit"))?;

    for instruction in entry.instructions.iter_mut().take(copied_len) {
        remap_instruction_constants(&mut instruction.kind, constant_base)?;
        remap_instruction_registers(&mut instruction.kind, register_base, pack_register_base)?;
        remap_instruction_locals(&mut instruction.kind, local_base)?;
        remap_instruction_persistent_slots(&mut instruction.kind, persistent_slot_base)?;
        match &mut instruction.kind {
            InstructionKind::Jump { target } | InstructionKind::JumpIfFalse { target, .. } => {
                *target =
                    remap_inlined_target(*target, copied_len, instruction_index, next_index_u32)?;
            }
            InstructionKind::ForEach { exit, .. } => {
                *exit = remap_inlined_target(*exit, copied_len, instruction_index, next_index_u32)?;
            }
            InstructionKind::Return { .. } => {
                instruction.kind = InstructionKind::Jump {
                    target: InstructionIndex::new(next_index_u32),
                };
            }
            _ => {}
        }
    }
    remap_spliced_exception_handlers(
        &mut entry.exception_handlers,
        copied_len,
        instruction_index,
        next_index_u32,
        final_return,
        local_base,
    )?;

    let inserted = copied_len;
    adjust_existing_jumps(
        &mut target.instructions,
        instruction_index,
        removed,
        inserted,
    )?;
    adjust_existing_exception_handlers(
        &mut target.exception_handlers,
        instruction_index,
        removed,
        inserted,
    )?;
    target.constants.extend(entry.constants);
    target.register_count = linked_register_count;
    target.pack_register_count = linked_pack_register_count;
    target.local_count = linked_local_count;
    target.persistent_slot_count = linked_persistent_slot_count;
    target.instructions.splice(
        instruction_index..instruction_index + removed,
        entry.instructions.into_iter().take(copied_len),
    );
    target.exception_handlers.extend(entry.exception_handlers);
    Ok(inserted)
}

fn remap_function(
    function: &mut Function,
    functions: &BTreeMap<u32, FunctionId>,
    class_base: u32,
) -> Result<(), EngineError> {
    for constant in &mut function.constants {
        if let Constant::Function(id) = constant {
            *id = map_function(*id, functions)?;
        }
    }
    for instruction in &mut function.instructions {
        match &mut instruction.kind {
            InstructionKind::RegisterClass { class } => {
                *class = ClassDefinitionId::new(
                    class_base
                        .checked_add(class.get())
                        .ok_or_else(|| link_error("linked class index overflowed"))?,
                );
            }
            InstructionKind::MakeClosure { function, .. }
            | InstructionKind::MakeSharedClosure { function, .. } => {
                *function = map_function(*function, functions)?;
            }
            _ => {}
        }
    }
    Ok(())
}

fn map_function(
    original: FunctionId,
    functions: &BTreeMap<u32, FunctionId>,
) -> Result<FunctionId, EngineError> {
    functions
        .get(&original.get())
        .copied()
        .ok_or_else(|| link_error("support source refers to its synthetic entry as a function"))
}

fn remap_instruction_constants(
    instruction: &mut InstructionKind,
    base: u32,
) -> Result<(), EngineError> {
    let add = |constant: &mut ConstantId| -> Result<(), EngineError> {
        *constant = ConstantId::new(
            base.checked_add(constant.get())
                .ok_or_else(|| link_error("linked constant index overflowed"))?,
        );
        Ok(())
    };
    match instruction {
        InstructionKind::LoadQualifiedTarget {
            root, qualified, ..
        } => {
            add(root)?;
            for candidate in qualified {
                add(candidate)?;
            }
            Ok(())
        }
        InstructionKind::LoadFunctionHandleCandidates { names, .. } => {
            for name in names {
                add(name)?;
            }
            Ok(())
        }
        InstructionKind::ApplyQualified { members, .. }
        | InstructionKind::GetQualifiedPack { members, .. } => {
            for member in members {
                add(member)?;
            }
            Ok(())
        }
        InstructionKind::StatementApplyQualified {
            members, result, ..
        } => {
            for member in members {
                add(member)?;
            }
            if let StatementResultTarget::Global(name) = result {
                add(name)?;
            }
            Ok(())
        }
        InstructionKind::ApplyOutputPack { target, .. } => {
            match target {
                PackApplyTarget::Value(_) => {}
                PackApplyTarget::Field { name, .. } => add(name)?,
                PackApplyTarget::Qualified { members, .. } => {
                    for member in members {
                        add(member)?;
                    }
                }
            }
            Ok(())
        }
        _ => remap_nonqualified_instruction_constants(instruction, base),
    }
}

#[allow(clippy::too_many_lines)]
fn remap_nonqualified_instruction_constants(
    instruction: &mut InstructionKind,
    base: u32,
) -> Result<(), EngineError> {
    let add = |constant: &mut ConstantId| -> Result<(), EngineError> {
        *constant = ConstantId::new(
            base.checked_add(constant.get())
                .ok_or_else(|| link_error("linked constant index overflowed"))?,
        );
        Ok(())
    };
    match instruction {
        InstructionKind::DeclareNamedBindings { bindings } => {
            for (name, _) in bindings {
                add(name)?;
            }
            Ok(())
        }
        InstructionKind::DeclareImports { imports } => {
            for import in imports {
                add(import)?;
            }
            Ok(())
        }
        InstructionKind::LoadConstant { constant, .. } => add(constant),
        InstructionKind::RequireDefined { name, .. }
        | InstructionKind::DeclareGlobal { name }
        | InstructionKind::LoadGlobal { name, .. }
        | InstructionKind::LoadCallTarget { name, .. }
        | InstructionKind::LoadFunctionHandle { name, .. }
        | InstructionKind::LoadGlobalOrNothing { name, .. }
        | InstructionKind::StoreGlobal { name, .. }
        | InstructionKind::GetField { name, .. }
        | InstructionKind::SetField { name, .. }
        | InstructionKind::ApplyField { name, .. }
        | InstructionKind::ReturnApplyField { name, .. }
        | InstructionKind::Display { name, .. }
        | InstructionKind::InvokeSuperclassConstructor {
            superclass: name, ..
        } => add(name),
        InstructionKind::StatementApply { result, .. }
        | InstructionKind::StatementPack { result, .. }
        | InstructionKind::StatementValue { result, .. } => {
            if let StatementResultTarget::Global(name) = result {
                add(name)?;
            }
            Ok(())
        }
        InstructionKind::StatementApplyField { name, result, .. } => {
            add(name)?;
            if let StatementResultTarget::Global(name) = result {
                add(name)?;
            }
            Ok(())
        }
        InstructionKind::ClearGlobal { names }
        | InstructionKind::ClearCapture { names }
        | InstructionKind::ClearDynamicBindings { names, .. } => {
            for name in names {
                add(name)?;
            }
            Ok(())
        }
        InstructionKind::MakeClosure { captures, .. } => {
            for (name, _, _) in captures {
                add(name)?;
            }
            Ok(())
        }
        InstructionKind::MakeSharedClosure { captures, .. } => {
            for (name, _) in captures {
                add(name)?;
            }
            Ok(())
        }
        InstructionKind::LoadCapture { name, .. } | InstructionKind::StoreCapture { name, .. } => {
            add(name)
        }
        InstructionKind::GetAggregateField { field, .. } => {
            if let FieldOperand::Static(name) = field {
                add(name)?;
            }
            Ok(())
        }
        InstructionKind::AssignPlace { path, .. }
        | InstructionKind::CountPlaceOutputs { path, .. } => {
            for step in path {
                if let PlaceStep::Field(FieldOperand::Static(name)) = step {
                    add(name)?;
                }
            }
            Ok(())
        }
        InstructionKind::AssignBindingPlace { binding, path, .. } => {
            if let BindingTarget::Capture(name) | BindingTarget::Workspace(name) = binding {
                add(name)?;
            }
            for step in path {
                if let PlaceStep::Field(FieldOperand::Static(name)) = step {
                    add(name)?;
                }
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn remap_instruction_persistent_slots(
    instruction: &mut InstructionKind,
    base: u32,
) -> Result<(), EngineError> {
    let add = |slot: &mut PersistentSlot| -> Result<(), EngineError> {
        *slot = PersistentSlot::new(
            base.checked_add(slot.get())
                .ok_or_else(|| link_error("linked persistent-slot index overflowed"))?,
        );
        Ok(())
    };
    match instruction {
        InstructionKind::DeclareNamedBindings { bindings } => {
            for (_, kind) in bindings {
                if let openmat_bytecode::NamedBindingKind::Persistent(slot) = kind {
                    add(slot)?;
                }
            }
            Ok(())
        }
        InstructionKind::DeclarePersistent { slot }
        | InstructionKind::LoadPersistent { slot, .. }
        | InstructionKind::StorePersistent { slot, .. }
        | InstructionKind::AssignBindingPlace {
            binding: BindingTarget::Persistent(slot),
            ..
        } => add(slot),
        _ => Ok(()),
    }
}

fn remap_instruction_locals(
    instruction: &mut InstructionKind,
    base: u32,
) -> Result<(), EngineError> {
    let add = |local: &mut LocalSlot| -> Result<(), EngineError> {
        *local = LocalSlot::new(
            base.checked_add(local.get())
                .ok_or_else(|| link_error("linked local index overflowed"))?,
        );
        Ok(())
    };
    let add_result = |result: &mut StatementResultTarget| -> Result<(), EngineError> {
        if let StatementResultTarget::Local(local) = result {
            add(local)?;
        }
        Ok(())
    };
    match instruction {
        InstructionKind::DeclareNamedBindings { bindings } => {
            for (_, kind) in bindings {
                if let openmat_bytecode::NamedBindingKind::Local(local) = kind {
                    add(local)?;
                }
            }
            Ok(())
        }
        InstructionKind::LoadLocal { local, .. }
        | InstructionKind::StoreLocal { local, .. }
        | InstructionKind::AssignBindingPlace {
            binding: BindingTarget::Local(local),
            ..
        } => add(local),
        InstructionKind::MakeSharedClosure { captures, .. } => {
            for (_, source) in captures {
                if let openmat_bytecode::SharedCaptureSource::Local(local) = source {
                    add(local)?;
                }
            }
            Ok(())
        }
        InstructionKind::ClearLocal { locals } => {
            for local in locals {
                add(local)?;
            }
            Ok(())
        }
        InstructionKind::InvokeSuperclassConstructor { object, .. } => add(object),
        InstructionKind::StatementApply { result, .. }
        | InstructionKind::StatementApplyField { result, .. }
        | InstructionKind::StatementApplyQualified { result, .. }
        | InstructionKind::StatementPack { result, .. }
        | InstructionKind::StatementValue { result, .. } => add_result(result),
        _ => Ok(()),
    }
}

#[allow(clippy::too_many_lines)]
fn remap_instruction_registers(
    instruction: &mut InstructionKind,
    register_base: u32,
    pack_register_base: u32,
) -> Result<(), EngineError> {
    let add = |register: &mut Register| -> Result<(), EngineError> {
        *register = Register::new(
            register_base
                .checked_add(register.get())
                .ok_or_else(|| link_error("linked register index overflowed"))?,
        );
        Ok(())
    };
    let add_pack = |register: &mut PackRegister| -> Result<(), EngineError> {
        *register = PackRegister::new(
            pack_register_base
                .checked_add(register.get())
                .ok_or_else(|| link_error("linked pack-register index overflowed"))?,
        );
        Ok(())
    };
    let add_arguments = |arguments: &mut [ApplyArgument]| -> Result<(), EngineError> {
        for argument in arguments {
            match argument {
                ApplyArgument::Value(register) => add(register)?,
                ApplyArgument::Expand(register) => add_pack(register)?,
                ApplyArgument::Colon => {}
            }
        }
        Ok(())
    };
    let add_value_source = |source: &mut ValueSource| -> Result<(), EngineError> {
        match source {
            ValueSource::One(register) => add(register),
            ValueSource::Expand(register) => add_pack(register),
        }
    };
    let add_field = |field: &mut FieldOperand| -> Result<(), EngineError> {
        if let FieldOperand::Dynamic(register) = field {
            add(register)?;
        }
        Ok(())
    };
    match instruction {
        InstructionKind::RequireDefined { value, .. } => add(value),
        InstructionKind::SlicePack {
            dst_pack,
            source,
            start,
            count,
        } => {
            add_pack(dst_pack)?;
            add_pack(source)?;
            add(start)?;
            add(count)
        }
        InstructionKind::CountPlaceOutputs { dst, root, path } => {
            add(dst)?;
            add(root)?;
            for step in path {
                match step {
                    PlaceStep::Paren(arguments) | PlaceStep::Brace(arguments) => {
                        add_arguments(arguments)?;
                    }
                    PlaceStep::Field(field) => add_field(field)?,
                }
            }
            Ok(())
        }
        InstructionKind::ApplyOutputPack {
            dst_pack,
            count,
            target,
            arguments,
        } => {
            add_pack(dst_pack)?;
            add(count)?;
            match target {
                PackApplyTarget::Value(value) => add(value)?,
                PackApplyTarget::Field { object, .. } => add(object)?,
                PackApplyTarget::Qualified {
                    target,
                    unresolved_suffix,
                    ..
                } => {
                    add(target)?;
                    add(unresolved_suffix)?;
                }
            }
            add_arguments(arguments)
        }
        InstructionKind::LoadConstant { dst, .. }
        | InstructionKind::LoadLocal { dst, .. }
        | InstructionKind::LoadPersistent { dst, .. }
        | InstructionKind::LoadGlobal { dst, .. }
        | InstructionKind::LoadCallTarget { dst, .. }
        | InstructionKind::LoadFunctionHandle { dst, .. }
        | InstructionKind::LoadFunctionHandleCandidates { dst, .. }
        | InstructionKind::LoadGlobalOrNothing { dst, .. }
        | InstructionKind::LoadCallInputCount { dst }
        | InstructionKind::LoadCallOutputCount { dst }
        | InstructionKind::LoadVariadicInputs { dst }
        | InstructionKind::LoadCapture { dst, .. }
        | InstructionKind::MakeSharedClosure { dst, .. } => add(dst),
        InstructionKind::LoadQualifiedTarget {
            dst,
            root_found,
            unresolved_suffix,
            ..
        } => {
            add(dst)?;
            add(root_found)?;
            add(unresolved_suffix)
        }
        InstructionKind::MakeClosure { dst, captures, .. } => {
            add(dst)?;
            for (_, value, _) in captures {
                if let Some(value) = value {
                    add(value)?;
                }
            }
            Ok(())
        }
        InstructionKind::Move { dst, src } => {
            add(dst)?;
            add(src)
        }
        InstructionKind::Binary { dst, lhs, rhs, .. } => {
            add(dst)?;
            add(lhs)?;
            add(rhs)
        }
        InstructionKind::SwitchMatch {
            dst,
            selector,
            case_value,
        } => {
            add(dst)?;
            add(selector)?;
            add(case_value)
        }
        InstructionKind::BuildMatrix { dst, rows } => {
            add(dst)?;
            for register in rows.iter_mut().flatten() {
                add(register)?;
            }
            Ok(())
        }
        InstructionKind::BuildCell { dst, rows } => {
            add(dst)?;
            for source in rows.iter_mut().flatten() {
                add_value_source(source)?;
            }
            Ok(())
        }
        InstructionKind::Range {
            dst,
            start,
            step,
            end,
        } => {
            add(dst)?;
            add(start)?;
            add(step)?;
            add(end)
        }
        InstructionKind::Transpose { dst, operand, .. } => {
            add(dst)?;
            add(operand)
        }
        InstructionKind::StoreLocal { src, .. }
        | InstructionKind::StorePersistent { src, .. }
        | InstructionKind::StoreGlobal { src, .. }
        | InstructionKind::StoreCapture { src, .. }
        | InstructionKind::StatementValue { src, .. }
        | InstructionKind::Display { src, .. } => add(src),
        InstructionKind::DeclareNamedBindings { .. }
        | InstructionKind::DeclareImports { .. }
        | InstructionKind::ClearImports
        | InstructionKind::DeclareGlobal { .. }
        | InstructionKind::DeclarePersistent { .. }
        | InstructionKind::ClearGlobal { .. }
        | InstructionKind::ClearGlobalAll
        | InstructionKind::ClearLocal { .. }
        | InstructionKind::ClearCapture { .. }
        | InstructionKind::ClearDynamicBindings { .. }
        | InstructionKind::RegisterClass { .. }
        | InstructionKind::Jump { .. } => Ok(()),
        InstructionKind::InvokeSuperclassConstructor { arguments, .. } => {
            for argument in arguments {
                add(argument)?;
            }
            Ok(())
        }
        InstructionKind::GetField { dst, object, .. } => {
            add(dst)?;
            add(object)
        }
        InstructionKind::SetField {
            dst, object, value, ..
        } => {
            add(dst)?;
            add(object)?;
            add(value)
        }
        InstructionKind::ApplyField {
            outputs,
            object,
            arguments,
            ..
        } => {
            for output in outputs {
                add(output)?;
            }
            add(object)?;
            add_arguments(arguments)
        }
        InstructionKind::ApplyQualified {
            outputs,
            target,
            unresolved_suffix,
            arguments,
            ..
        } => {
            for output in outputs {
                add(output)?;
            }
            add(target)?;
            add(unresolved_suffix)?;
            add_arguments(arguments)
        }
        InstructionKind::GetQualifiedPack {
            dst_pack,
            target,
            root_found,
            unresolved_suffix,
            ..
        } => {
            add_pack(dst_pack)?;
            add(target)?;
            add(root_found)?;
            add(unresolved_suffix)
        }
        InstructionKind::BraceApply {
            dst_pack,
            target,
            arguments,
        } => {
            add_pack(dst_pack)?;
            add(target)?;
            add_arguments(arguments)
        }
        InstructionKind::GetAggregateField {
            dst_pack,
            target,
            field,
        } => {
            add_pack(dst_pack)?;
            add(target)?;
            add_field(field)
        }
        InstructionKind::AssignPlace {
            dst,
            root,
            path,
            source,
            ..
        } => {
            add(dst)?;
            add(root)?;
            for step in path {
                match step {
                    PlaceStep::Paren(arguments) | PlaceStep::Brace(arguments) => {
                        add_arguments(arguments)?;
                    }
                    PlaceStep::Field(field) => add_field(field)?,
                }
            }
            add_value_source(source)
        }
        InstructionKind::ResolveEnd { dst, target, .. } => {
            add(dst)?;
            add(target)
        }
        InstructionKind::Unpack { outputs, pack } => {
            for output in outputs {
                add(output)?;
            }
            add_pack(pack)
        }
        InstructionKind::JumpIfFalse { condition, .. } => add(condition),
        InstructionKind::Call {
            outputs,
            callee,
            arguments,
        } => {
            for output in outputs {
                add(output)?;
            }
            add(callee)?;
            for argument in arguments {
                add(argument)?;
            }
            Ok(())
        }
        InstructionKind::Apply {
            outputs,
            target,
            arguments,
        }
        | InstructionKind::ApplyBinding {
            outputs,
            target,
            arguments,
        } => {
            for output in outputs {
                add(output)?;
            }
            add(target)?;
            add_arguments(arguments)
        }
        InstructionKind::AssignBindingPlace {
            result,
            root,
            path,
            source,
            ..
        } => {
            if let Some(result) = result {
                add(result)?;
            }
            add(root)?;
            for step in path {
                match step {
                    PlaceStep::Paren(arguments) | PlaceStep::Brace(arguments) => {
                        add_arguments(arguments)?;
                    }
                    PlaceStep::Field(field) => add_field(field)?,
                }
            }
            add_value_source(source)
        }
        InstructionKind::StatementApply {
            target, arguments, ..
        }
        | InstructionKind::ReturnApply { target, arguments } => {
            add(target)?;
            add_arguments(arguments)
        }
        InstructionKind::StatementApplyField {
            object, arguments, ..
        }
        | InstructionKind::ReturnApplyField {
            object, arguments, ..
        } => {
            add(object)?;
            add_arguments(arguments)
        }
        InstructionKind::StatementApplyQualified {
            target,
            unresolved_suffix,
            arguments,
            ..
        } => {
            add(target)?;
            add(unresolved_suffix)?;
            add_arguments(arguments)
        }
        InstructionKind::StatementPack { pack, .. }
        | InstructionKind::RequireSinglePack { pack } => add_pack(pack),
        InstructionKind::IndexAssign {
            dst,
            target,
            arguments,
            value,
        } => {
            add(dst)?;
            add(target)?;
            add_arguments(arguments)?;
            add(value)
        }
        InstructionKind::ForEach {
            iterable,
            index,
            dst,
            ..
        } => {
            add(iterable)?;
            add(index)?;
            add(dst)
        }
        InstructionKind::Return { values } => {
            for value in values {
                add(value)?;
            }
            Ok(())
        }
        InstructionKind::ReturnVariadic { fixed, variadic } => {
            for value in fixed {
                add(value)?;
            }
            add(variadic)
        }
    }
}

fn remap_inlined_target(
    target: InstructionIndex,
    copied_len: usize,
    insertion: usize,
    next: u32,
) -> Result<InstructionIndex, EngineError> {
    let target = target.get() as usize;
    if target >= copied_len {
        return Ok(InstructionIndex::new(next));
    }
    let linked = insertion
        .checked_add(target)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| link_error("linked jump target overflowed"))?;
    Ok(InstructionIndex::new(linked))
}

fn remap_spliced_exception_handlers(
    handlers: &mut [ExceptionHandler],
    copied_len: usize,
    insertion: usize,
    next: u32,
    final_return: bool,
    local_base: u32,
) -> Result<(), EngineError> {
    for handler in handlers {
        handler.protected_start = remap_spliced_handler_pc(
            handler.protected_start,
            copied_len,
            insertion,
            next,
            final_return,
            false,
        )?;
        handler.protected_end = remap_spliced_handler_pc(
            handler.protected_end,
            copied_len,
            insertion,
            next,
            final_return,
            true,
        )?;
        if let ExceptionHandlerKind::Catch {
            handler: catch,
            error_local,
        } = &mut handler.kind
        {
            *catch =
                remap_spliced_handler_pc(*catch, copied_len, insertion, next, final_return, true)?;
            if let Some(local) = error_local {
                *local = LocalSlot::new(
                    local_base
                        .checked_add(local.get())
                        .ok_or_else(|| link_error("linked catch local index overflowed"))?,
                );
            }
        }
        handler.exit = remap_spliced_handler_pc(
            handler.exit,
            copied_len,
            insertion,
            next,
            final_return,
            true,
        )?;
    }
    Ok(())
}

fn remap_spliced_handler_pc(
    target: InstructionIndex,
    copied_len: usize,
    insertion: usize,
    next: u32,
    final_return: bool,
    allow_omitted_return: bool,
) -> Result<InstructionIndex, EngineError> {
    let target = target.get() as usize;
    if target < copied_len {
        let linked = insertion
            .checked_add(target)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| link_error("linked exception handler target overflowed"))?;
        return Ok(InstructionIndex::new(linked));
    }
    if allow_omitted_return && final_return && target == copied_len {
        return Ok(InstructionIndex::new(next));
    }
    Err(link_error(
        "exception handler refers outside the spliced entry body",
    ))
}

fn adjust_existing_jumps(
    instructions: &mut [Instruction],
    splice_at: usize,
    removed: usize,
    inserted: usize,
) -> Result<(), EngineError> {
    for instruction in instructions {
        let target = match &mut instruction.kind {
            InstructionKind::Jump { target } | InstructionKind::JumpIfFalse { target, .. } => {
                Some(target)
            }
            InstructionKind::ForEach { exit, .. } => Some(exit),
            _ => None,
        };
        let Some(target) = target else {
            continue;
        };
        let old = target.get() as usize;
        if old > splice_at || (removed == 0 && old == splice_at) {
            let new = old
                .checked_sub(removed)
                .and_then(|value| value.checked_add(inserted))
                .and_then(|value| u32::try_from(value).ok())
                .ok_or_else(|| link_error("caller jump target overflowed during linking"))?;
            *target = InstructionIndex::new(new);
        }
    }
    Ok(())
}

fn adjust_existing_exception_handlers(
    handlers: &mut Vec<ExceptionHandler>,
    splice_at: usize,
    removed: usize,
    inserted: usize,
) -> Result<(), EngineError> {
    for handler in handlers.iter_mut() {
        handler.protected_start =
            adjust_existing_handler_pc(handler.protected_start, splice_at, removed, inserted)?;
        handler.protected_end =
            adjust_existing_handler_pc(handler.protected_end, splice_at, removed, inserted)?;
        if let ExceptionHandlerKind::Catch { handler: catch, .. } = &mut handler.kind {
            *catch = adjust_existing_handler_pc(*catch, splice_at, removed, inserted)?;
        }
        handler.exit = adjust_existing_handler_pc(handler.exit, splice_at, removed, inserted)?;
    }
    handlers.retain(|handler| handler.protected_start < handler.protected_end);
    Ok(())
}

fn adjust_existing_handler_pc(
    target: InstructionIndex,
    splice_at: usize,
    removed: usize,
    inserted: usize,
) -> Result<InstructionIndex, EngineError> {
    let old = target.get() as usize;
    if old > splice_at || (removed == 0 && old == splice_at) {
        let new = old
            .checked_sub(removed)
            .and_then(|value| value.checked_add(inserted))
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| {
                link_error("caller exception handler target overflowed during linking")
            })?;
        Ok(InstructionIndex::new(new))
    } else {
        Ok(target)
    }
}

fn link_error(message: &'static str) -> EngineError {
    EngineError::new("source.link", message)
}

#[cfg(test)]
mod tests {
    use openmat_bytecode::{ExceptionHandler, LocalSlot, SourceLocation, verify};

    use super::*;

    fn pc(index: u32) -> InstructionIndex {
        InstructionIndex::new(index)
    }

    fn operation() -> Instruction {
        Instruction::new(InstructionKind::LoadCallInputCount {
            dst: Register::new(0),
        })
    }

    #[test]
    fn relocates_all_v27_output_operands_and_checked_names() {
        let operations = |register_base: u32, pack_base: u32, constant_base: u32| {
            let r = |index: u32| Register::new(register_base + index);
            let p = |index: u32| PackRegister::new(pack_base + index);
            let c = |index: u32| ConstantId::new(constant_base + index);
            vec![
                InstructionKind::RequireDefined {
                    value: r(0),
                    name: c(1),
                },
                InstructionKind::RequireSinglePack { pack: p(1) },
                InstructionKind::CountPlaceOutputs {
                    dst: r(0),
                    root: r(1),
                    path: vec![
                        PlaceStep::Brace(vec![
                            ApplyArgument::Expand(p(0)),
                            ApplyArgument::Value(r(2)),
                        ]),
                        PlaceStep::Field(FieldOperand::Dynamic(r(3))),
                        PlaceStep::Field(FieldOperand::Static(c(0))),
                    ],
                },
                InstructionKind::ApplyOutputPack {
                    dst_pack: p(0),
                    count: r(1),
                    target: PackApplyTarget::Value(r(2)),
                    arguments: vec![ApplyArgument::Expand(p(1))],
                },
                InstructionKind::ApplyOutputPack {
                    dst_pack: p(0),
                    count: r(1),
                    target: PackApplyTarget::Field {
                        object: r(2),
                        name: c(1),
                    },
                    arguments: vec![ApplyArgument::Value(r(3))],
                },
                InstructionKind::ApplyOutputPack {
                    dst_pack: p(0),
                    count: r(1),
                    target: PackApplyTarget::Qualified {
                        target: r(2),
                        unresolved_suffix: r(3),
                        members: vec![c(0), c(1)],
                    },
                    arguments: vec![],
                },
                InstructionKind::SlicePack {
                    dst_pack: p(0),
                    source: p(1),
                    start: r(0),
                    count: r(1),
                },
            ]
        };
        for (mut actual, expected) in operations(0, 0, 0).into_iter().zip(operations(7, 11, 13)) {
            remap_instruction_registers(&mut actual, 7, 11).unwrap();
            remap_instruction_constants(&mut actual, 13).unwrap();
            assert_eq!(actual, expected);
        }
    }

    fn return_instruction() -> Instruction {
        Instruction::new(InstructionKind::Return { values: Vec::new() })
    }

    #[test]
    fn support_module_merge_preserves_entry_and_whole_function_exception_tables() {
        let entry_location = SourceLocation::new(41, 3, 19);
        let function_location = SourceLocation::new(42, 5, 23);
        let make_function = |name: &str, location| {
            let mut function = Function::new(name, 1, 0, 0).with_persistent_slot_count(2);
            function.instructions = vec![
                operation(),
                Instruction::new(InstructionKind::Jump { target: pc(3) }),
                operation(),
                return_instruction(),
            ];
            function.exception_handlers = vec![
                ExceptionHandler::catch(pc(0), pc(1), pc(2), pc(3), None).with_location(location),
            ];
            function
        };
        let mut target = BytecodeModule::new(
            vec![Function {
                name: "target".to_owned(),
                register_count: 0,
                pack_register_count: 0,
                local_count: 0,
                persistent_slot_count: 0,
                parameter_count: 0,
                argument_layout: None,
                constants: Vec::new(),
                instructions: vec![return_instruction()],
                exception_handlers: Vec::new(),
            }],
            FunctionId::new(0),
        );
        let support_entry = make_function("support-entry", entry_location);
        let support_function = make_function("support-function", function_location);
        let support = BytecodeModule::new(
            vec![support_entry.clone(), support_function.clone()],
            FunctionId::new(0),
        );

        let merged = merge_support_module(&mut target, &support).expect("merge support module");

        assert_eq!(
            merged.entry.exception_handlers,
            support_entry.exception_handlers
        );
        assert_eq!(
            target.functions[1].exception_handlers,
            support_function.exception_handlers
        );
        assert_eq!(merged.entry.persistent_slot_count, 2);
        assert_eq!(target.functions[1].persistent_slot_count, 2);
        verify(&target).expect("whole-function merge remains verifiable");
        let entry_module = BytecodeModule::new(vec![merged.entry], FunctionId::new(0));
        verify(&entry_module).expect("detached support entry remains verifiable");
    }

    #[test]
    fn prepended_entry_remaps_persistent_slots_without_losing_count() {
        let mut target = Function::new("target", 0, 0, 0).with_persistent_slot_count(1);
        target.instructions = vec![return_instruction()];
        let mut entry = Function::new("support-entry", 1, 0, 0).with_persistent_slot_count(2);
        entry.instructions = vec![
            Instruction::new(InstructionKind::DeclarePersistent {
                slot: PersistentSlot::new(0),
            }),
            Instruction::new(InstructionKind::LoadPersistent {
                dst: Register::new(0),
                slot: PersistentSlot::new(1),
            }),
            Instruction::new(InstructionKind::StorePersistent {
                slot: PersistentSlot::new(1),
                src: Register::new(0),
            }),
            return_instruction(),
        ];

        prepend_entry_instructions(&mut target, entry).expect("prepend support entry");

        assert_eq!(target.persistent_slot_count, 3);
        assert!(matches!(
            target.instructions[0].kind,
            InstructionKind::DeclarePersistent { slot } if slot == PersistentSlot::new(1)
        ));
        assert!(matches!(
            target.instructions[1].kind,
            InstructionKind::LoadPersistent { slot, .. } if slot == PersistentSlot::new(2)
        ));
        assert!(matches!(
            target.instructions[2].kind,
            InstructionKind::StorePersistent { slot, .. } if slot == PersistentSlot::new(2)
        ));
        verify(&BytecodeModule::new(vec![target], FunctionId::new(0)))
            .expect("persistent-slot remapping remains verifiable");
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn inline_script_remaps_nested_handlers_locals_and_existing_handler() {
        let existing_location = SourceLocation::new(50, 10, 30);
        let inner_location = SourceLocation::new(51, 2, 12);
        let outer_location = SourceLocation::new(51, 0, 40);
        let swallow_location = SourceLocation::new(51, 14, 22);
        let retained_instruction_location = SourceLocation::new(50, 31, 35);
        let script_instruction_location = SourceLocation::new(51, 23, 28);
        let mut target = Function {
            name: "entry".to_owned(),
            register_count: 1,
            pack_register_count: 0,
            local_count: 2,
            persistent_slot_count: 0,
            parameter_count: 0,
            argument_layout: None,
            constants: vec![Constant::String("script".to_owned())],
            instructions: vec![
                operation(),
                Instruction::new(InstructionKind::LoadGlobal {
                    dst: Register::new(0),
                    name: ConstantId::new(0),
                    construct_if_class: false,
                }),
                Instruction::located(
                    InstructionKind::LoadLocal {
                        dst: Register::new(0),
                        local: LocalSlot::new(1),
                    },
                    retained_instruction_location,
                ),
                Instruction::new(InstructionKind::Jump { target: pc(7) }),
                Instruction::new(InstructionKind::StoreLocal {
                    local: LocalSlot::new(1),
                    src: Register::new(0),
                }),
                Instruction::new(InstructionKind::LoadCallOutputCount {
                    dst: Register::new(0),
                }),
                Instruction::new(InstructionKind::StoreLocal {
                    local: LocalSlot::new(0),
                    src: Register::new(0),
                }),
                return_instruction(),
            ],
            exception_handlers: vec![
                ExceptionHandler::catch(pc(0), pc(3), pc(4), pc(7), Some(LocalSlot::new(0)))
                    .with_location(existing_location),
            ],
        };
        let script = Function {
            name: "script".to_owned(),
            register_count: 1,
            pack_register_count: 0,
            local_count: 3,
            persistent_slot_count: 0,
            parameter_count: 0,
            argument_layout: None,
            constants: Vec::new(),
            instructions: vec![
                Instruction::new(InstructionKind::LoadLocal {
                    dst: Register::new(0),
                    local: LocalSlot::new(0),
                }),
                Instruction::new(InstructionKind::LoadLocal {
                    dst: Register::new(0),
                    local: LocalSlot::new(1),
                }),
                Instruction::new(InstructionKind::Jump { target: pc(4) }),
                Instruction::new(InstructionKind::StoreLocal {
                    local: LocalSlot::new(2),
                    src: Register::new(0),
                }),
                operation(),
                Instruction::new(InstructionKind::LoadCallOutputCount {
                    dst: Register::new(0),
                }),
                operation(),
                Instruction::located(
                    InstructionKind::StatementValue {
                        src: Register::new(0),
                        result: StatementResultTarget::Local(LocalSlot::new(1)),
                        display: false,
                    },
                    script_instruction_location,
                ),
                Instruction::new(InstructionKind::Jump { target: pc(11) }),
                operation(),
                Instruction::new(InstructionKind::StoreLocal {
                    local: LocalSlot::new(2),
                    src: Register::new(0),
                }),
                return_instruction(),
            ],
            exception_handlers: vec![
                ExceptionHandler::catch(pc(1), pc(2), pc(3), pc(4), None)
                    .with_location(inner_location),
                ExceptionHandler::catch(pc(0), pc(8), pc(9), pc(11), Some(LocalSlot::new(2)))
                    .with_location(outer_location),
                ExceptionHandler::swallow(pc(5), pc(7), pc(7)).with_location(swallow_location),
            ],
        };

        let inserted = inline_script_entry(&mut target, 1, script).expect("inline script");

        assert_eq!(inserted, 11);
        assert_eq!(target.register_count, 2);
        assert_eq!(target.local_count, 5);
        assert_eq!(
            target.instructions[8].location,
            Some(script_instruction_location)
        );
        assert_eq!(
            target.instructions[12].location,
            Some(retained_instruction_location)
        );
        assert!(matches!(
            target.instructions[1].kind,
            InstructionKind::LoadLocal { local, .. } if local == LocalSlot::new(2)
        ));
        assert!(matches!(
            target.instructions[2].kind,
            InstructionKind::LoadLocal { local, .. } if local == LocalSlot::new(3)
        ));
        assert!(matches!(
            target.instructions[4].kind,
            InstructionKind::StoreLocal { local, .. } if local == LocalSlot::new(4)
        ));
        assert!(matches!(
            target.instructions[8].kind,
            InstructionKind::StatementValue {
                result: StatementResultTarget::Local(local),
                ..
            } if local == LocalSlot::new(3)
        ));
        assert!(matches!(
            target.instructions[12].kind,
            InstructionKind::LoadLocal { local, .. } if local == LocalSlot::new(1)
        ));
        assert_eq!(
            target.exception_handlers,
            vec![
                ExceptionHandler::catch(pc(0), pc(13), pc(14), pc(17), Some(LocalSlot::new(0)),)
                    .with_location(existing_location),
                ExceptionHandler::catch(pc(2), pc(3), pc(4), pc(5), None)
                    .with_location(inner_location),
                ExceptionHandler::catch(pc(1), pc(9), pc(10), pc(12), Some(LocalSlot::new(4)),)
                    .with_location(outer_location),
                ExceptionHandler::swallow(pc(6), pc(8), pc(8)).with_location(swallow_location),
            ]
        );
        let linked = BytecodeModule::new(vec![target], FunctionId::new(0));
        verify(&linked).expect("linked inline script remains verifiable");
    }

    #[test]
    fn prepended_entry_remaps_its_swallow_and_existing_catch_handler() {
        let existing_location = SourceLocation::new(60, 5, 25);
        let inserted_location = SourceLocation::new(61, 1, 9);
        let mut target = Function {
            name: "entry".to_owned(),
            register_count: 1,
            pack_register_count: 0,
            local_count: 0,
            persistent_slot_count: 0,
            parameter_count: 0,
            argument_layout: None,
            constants: Vec::new(),
            instructions: vec![
                operation(),
                Instruction::new(InstructionKind::Jump { target: pc(3) }),
                operation(),
                return_instruction(),
            ],
            exception_handlers: vec![
                ExceptionHandler::catch(pc(0), pc(1), pc(2), pc(3), None)
                    .with_location(existing_location),
            ],
        };
        let entry = Function {
            name: "class-entry".to_owned(),
            register_count: 1,
            pack_register_count: 0,
            local_count: 0,
            persistent_slot_count: 0,
            parameter_count: 0,
            argument_layout: None,
            constants: Vec::new(),
            instructions: vec![operation(), return_instruction()],
            exception_handlers: vec![
                ExceptionHandler::swallow(pc(0), pc(1), pc(1)).with_location(inserted_location),
            ],
        };

        prepend_entry_instructions(&mut target, entry).expect("prepend entry");

        assert_eq!(
            target.exception_handlers,
            vec![
                ExceptionHandler::catch(pc(1), pc(2), pc(3), pc(4), None)
                    .with_location(existing_location),
                ExceptionHandler::swallow(pc(0), pc(1), pc(1)).with_location(inserted_location),
            ]
        );
        verify(&BytecodeModule::new(vec![target], FunctionId::new(0)))
            .expect("prepended entry remains verifiable");
    }

    #[test]
    fn inlined_scripts_remap_metadata_handles_registers_and_every_clear_name() {
        let mut target = Function {
            name: "entry".to_owned(),
            register_count: 1,
            pack_register_count: 0,
            local_count: 0,
            persistent_slot_count: 0,
            parameter_count: 0,
            argument_layout: None,
            constants: vec![Constant::Double(1.0)],
            instructions: vec![
                Instruction::new(InstructionKind::LoadConstant {
                    dst: Register::new(0),
                    constant: ConstantId::new(0),
                }),
                Instruction::new(InstructionKind::Return { values: Vec::new() }),
            ],
            exception_handlers: Vec::new(),
        };
        let script = Function {
            name: "script".to_owned(),
            register_count: 1,
            pack_register_count: 0,
            local_count: 0,
            persistent_slot_count: 0,
            parameter_count: 0,
            argument_layout: None,
            constants: vec![
                Constant::String("first".to_owned()),
                Constant::String("second".to_owned()),
            ],
            instructions: vec![
                Instruction::new(InstructionKind::LoadCallInputCount {
                    dst: Register::new(0),
                }),
                Instruction::new(InstructionKind::LoadFunctionHandle {
                    dst: Register::new(0),
                    name: ConstantId::new(0),
                }),
                Instruction::new(InstructionKind::ClearGlobal {
                    names: vec![ConstantId::new(0), ConstantId::new(1)],
                }),
                Instruction::new(InstructionKind::Return { values: Vec::new() }),
            ],
            exception_handlers: Vec::new(),
        };

        let inserted = inline_script_entry(&mut target, 0, script).expect("inline script");

        assert_eq!(inserted, 3);
        assert_eq!(target.register_count, 2);
        assert!(matches!(
            target.instructions[0].kind,
            InstructionKind::LoadCallInputCount { dst } if dst == Register::new(1)
        ));
        assert!(matches!(
            &target.instructions[1].kind,
            InstructionKind::LoadFunctionHandle { dst, name }
                if *dst == Register::new(1) && *name == ConstantId::new(1)
        ));
        assert!(matches!(
            &target.instructions[2].kind,
            InstructionKind::ClearGlobal { names }
                if names == &[ConstantId::new(1), ConstantId::new(2)]
        ));
    }

    #[test]
    fn inlined_scripts_remap_every_switch_match_register() {
        let mut target = Function {
            name: "entry".to_owned(),
            register_count: 4,
            pack_register_count: 0,
            local_count: 0,
            persistent_slot_count: 0,
            parameter_count: 0,
            argument_layout: None,
            constants: Vec::new(),
            instructions: vec![Instruction::new(InstructionKind::Return {
                values: Vec::new(),
            })],
            exception_handlers: Vec::new(),
        };
        let script = Function {
            name: "script".to_owned(),
            register_count: 6,
            pack_register_count: 0,
            local_count: 0,
            persistent_slot_count: 0,
            parameter_count: 0,
            argument_layout: None,
            constants: Vec::new(),
            instructions: vec![
                Instruction::new(InstructionKind::SwitchMatch {
                    dst: Register::new(1),
                    selector: Register::new(3),
                    case_value: Register::new(5),
                }),
                Instruction::new(InstructionKind::Return { values: Vec::new() }),
            ],
            exception_handlers: Vec::new(),
        };

        let inserted = inline_script_entry(&mut target, 0, script).expect("inline script");

        assert_eq!(inserted, 1);
        assert_eq!(target.register_count, 10);
        assert!(matches!(
            target.instructions[0].kind,
            InstructionKind::SwitchMatch {
                dst,
                selector,
                case_value,
            } if dst == Register::new(5)
                && selector == Register::new(7)
                && case_value == Register::new(9)
        ));
    }

    #[test]
    fn support_module_merge_preserves_switch_match_fields_in_entry_and_functions() {
        let make_function = |name: &str, dst: u32, selector: u32, case_value: u32| Function {
            name: name.to_owned(),
            register_count: 6,
            pack_register_count: 0,
            local_count: 0,
            persistent_slot_count: 0,
            parameter_count: 0,
            argument_layout: None,
            constants: Vec::new(),
            instructions: vec![
                Instruction::new(InstructionKind::SwitchMatch {
                    dst: Register::new(dst),
                    selector: Register::new(selector),
                    case_value: Register::new(case_value),
                }),
                Instruction::new(InstructionKind::Return { values: Vec::new() }),
            ],
            exception_handlers: Vec::new(),
        };
        let mut target =
            BytecodeModule::new(vec![make_function("target", 0, 1, 2)], FunctionId::new(0));
        let support = BytecodeModule::new(
            vec![
                make_function("support_entry", 1, 3, 5),
                make_function("named", 5, 2, 0),
                make_function("anonymous", 4, 1, 3),
            ],
            FunctionId::new(0),
        );

        let merged = merge_support_module(&mut target, &support).expect("merge support module");

        assert!(matches!(
            merged.entry.instructions[0].kind,
            InstructionKind::SwitchMatch {
                dst,
                selector,
                case_value,
            } if dst == Register::new(1)
                && selector == Register::new(3)
                && case_value == Register::new(5)
        ));
        assert!(matches!(
            target.functions[1].instructions[0].kind,
            InstructionKind::SwitchMatch {
                dst,
                selector,
                case_value,
            } if dst == Register::new(5)
                && selector == Register::new(2)
                && case_value == Register::new(0)
        ));
        assert!(matches!(
            target.functions[2].instructions[0].kind,
            InstructionKind::SwitchMatch {
                dst,
                selector,
                case_value,
            } if dst == Register::new(4)
                && selector == Register::new(1)
                && case_value == Register::new(3)
        ));
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn inlined_scripts_remap_every_bytecode_v12_operand_space_and_location() {
        let mut target = Function {
            name: "entry".to_owned(),
            register_count: 10,
            pack_register_count: 5,
            local_count: 0,
            persistent_slot_count: 0,
            parameter_count: 0,
            argument_layout: None,
            constants: vec![
                Constant::String("target-a".to_owned()),
                Constant::String("target-b".to_owned()),
            ],
            instructions: vec![Instruction::new(InstructionKind::Return {
                values: Vec::new(),
            })],
            exception_handlers: Vec::new(),
        };
        let location = openmat_bytecode::SourceLocation::new(7, 11, 19);
        let script = Function {
            name: "script".to_owned(),
            register_count: 8,
            pack_register_count: 7,
            local_count: 0,
            persistent_slot_count: 0,
            parameter_count: 0,
            argument_layout: None,
            constants: vec![
                Constant::String("static-read".to_owned()),
                Constant::String("static-write".to_owned()),
            ],
            instructions: vec![
                Instruction::located(
                    InstructionKind::BuildCell {
                        dst: Register::new(0),
                        rows: vec![vec![
                            ValueSource::One(Register::new(1)),
                            ValueSource::Expand(PackRegister::new(2)),
                        ]],
                    },
                    location,
                ),
                Instruction::new(InstructionKind::BraceApply {
                    dst_pack: PackRegister::new(3),
                    target: Register::new(2),
                    arguments: vec![
                        ApplyArgument::Value(Register::new(3)),
                        ApplyArgument::Expand(PackRegister::new(4)),
                        ApplyArgument::Colon,
                    ],
                }),
                Instruction::new(InstructionKind::GetAggregateField {
                    dst_pack: PackRegister::new(5),
                    target: Register::new(4),
                    field: FieldOperand::Dynamic(Register::new(5)),
                }),
                Instruction::new(InstructionKind::GetAggregateField {
                    dst_pack: PackRegister::new(0),
                    target: Register::new(6),
                    field: FieldOperand::Static(ConstantId::new(0)),
                }),
                Instruction::new(InstructionKind::AssignPlace {
                    dst: Register::new(6),
                    root: Register::new(7),
                    path: vec![
                        PlaceStep::Paren(vec![
                            ApplyArgument::Value(Register::new(0)),
                            ApplyArgument::Expand(PackRegister::new(1)),
                            ApplyArgument::Colon,
                        ]),
                        PlaceStep::Brace(vec![
                            ApplyArgument::Expand(PackRegister::new(2)),
                            ApplyArgument::Value(Register::new(3)),
                        ]),
                        PlaceStep::Field(FieldOperand::Static(ConstantId::new(1))),
                        PlaceStep::Field(FieldOperand::Dynamic(Register::new(4))),
                    ],
                    source: ValueSource::Expand(PackRegister::new(6)),
                    mode: openmat_bytecode::AssignmentMode::Store,
                }),
                Instruction::new(InstructionKind::ResolveEnd {
                    dst: Register::new(1),
                    target: Register::new(2),
                    argument_index: 0,
                    argument_count: 2,
                }),
                Instruction::new(InstructionKind::Unpack {
                    outputs: vec![Register::new(3), Register::new(4)],
                    pack: PackRegister::new(0),
                }),
                Instruction::new(InstructionKind::Return { values: Vec::new() }),
            ],
            exception_handlers: Vec::new(),
        };

        let inserted = inline_script_entry(&mut target, 0, script).expect("inline v12 script");

        assert_eq!(inserted, 7);
        assert_eq!(target.register_count, 18);
        assert_eq!(target.pack_register_count, 12);
        assert_eq!(target.instructions[0].location, Some(location));
        assert!(matches!(
            &target.instructions[0].kind,
            InstructionKind::BuildCell { dst, rows }
                if *dst == Register::new(10)
                    && rows == &vec![vec![
                        ValueSource::One(Register::new(11)),
                        ValueSource::Expand(PackRegister::new(7)),
                    ]]
        ));
        assert!(matches!(
            &target.instructions[1].kind,
            InstructionKind::BraceApply { dst_pack, target, arguments }
                if *dst_pack == PackRegister::new(8)
                    && *target == Register::new(12)
                    && arguments == &vec![
                        ApplyArgument::Value(Register::new(13)),
                        ApplyArgument::Expand(PackRegister::new(9)),
                        ApplyArgument::Colon,
                    ]
        ));
        assert!(matches!(
            target.instructions[2].kind,
            InstructionKind::GetAggregateField {
                dst_pack,
                target,
                field: FieldOperand::Dynamic(field),
            } if dst_pack == PackRegister::new(10)
                && target == Register::new(14)
                && field == Register::new(15)
        ));
        assert!(matches!(
            target.instructions[3].kind,
            InstructionKind::GetAggregateField {
                dst_pack,
                target,
                field: FieldOperand::Static(field),
            } if dst_pack == PackRegister::new(5)
                && target == Register::new(16)
                && field == ConstantId::new(2)
        ));
        let InstructionKind::AssignPlace {
            dst,
            root,
            path,
            source,
            ..
        } = &target.instructions[4].kind
        else {
            panic!("remapped assign place");
        };
        assert_eq!(*dst, Register::new(16));
        assert_eq!(*root, Register::new(17));
        assert_eq!(*source, ValueSource::Expand(PackRegister::new(11)));
        assert_eq!(
            path,
            &vec![
                PlaceStep::Paren(vec![
                    ApplyArgument::Value(Register::new(10)),
                    ApplyArgument::Expand(PackRegister::new(6)),
                    ApplyArgument::Colon,
                ]),
                PlaceStep::Brace(vec![
                    ApplyArgument::Expand(PackRegister::new(7)),
                    ApplyArgument::Value(Register::new(13)),
                ]),
                PlaceStep::Field(FieldOperand::Static(ConstantId::new(3))),
                PlaceStep::Field(FieldOperand::Dynamic(Register::new(14))),
            ]
        );
        assert!(matches!(
            target.instructions[5].kind,
            InstructionKind::ResolveEnd { dst, target, .. }
                if dst == Register::new(11) && target == Register::new(12)
        ));
        assert!(matches!(
            &target.instructions[6].kind,
            InstructionKind::Unpack { outputs, pack }
                if outputs == &vec![Register::new(13), Register::new(14)]
                    && *pack == PackRegister::new(5)
        ));
    }

    #[test]
    fn inlined_scripts_remap_v19_direct_binding_operands() {
        let mut target = Function {
            name: "entry".to_owned(),
            register_count: 4,
            pack_register_count: 0,
            local_count: 0,
            persistent_slot_count: 0,
            parameter_count: 0,
            argument_layout: None,
            constants: vec![
                Constant::String("target-a".to_owned()),
                Constant::String("target-b".to_owned()),
            ],
            instructions: vec![return_instruction()],
            exception_handlers: Vec::new(),
        };
        let script = Function {
            name: "script".to_owned(),
            register_count: 3,
            pack_register_count: 0,
            local_count: 0,
            persistent_slot_count: 0,
            parameter_count: 0,
            argument_layout: None,
            constants: vec![
                Constant::String("root".to_owned()),
                Constant::String("field".to_owned()),
            ],
            instructions: vec![
                Instruction::new(InstructionKind::ApplyBinding {
                    outputs: vec![Register::new(0)],
                    target: Register::new(1),
                    arguments: vec![ApplyArgument::Value(Register::new(2))],
                }),
                Instruction::new(InstructionKind::AssignBindingPlace {
                    result: Some(Register::new(2)),
                    binding: BindingTarget::Workspace(ConstantId::new(0)),
                    root: Register::new(1),
                    path: vec![
                        PlaceStep::Paren(vec![ApplyArgument::Value(Register::new(1))]),
                        PlaceStep::Field(FieldOperand::Static(ConstantId::new(1))),
                    ],
                    source: ValueSource::One(Register::new(0)),
                    mode: openmat_bytecode::AssignmentMode::Store,
                }),
                return_instruction(),
            ],
            exception_handlers: Vec::new(),
        };

        assert_eq!(
            inline_script_entry(&mut target, 0, script).expect("inline v19 script"),
            2
        );
        assert!(matches!(
            &target.instructions[0].kind,
            InstructionKind::ApplyBinding { outputs, target, arguments }
                if outputs == &vec![Register::new(4)]
                    && *target == Register::new(5)
                    && arguments == &vec![ApplyArgument::Value(Register::new(6))]
        ));
        assert!(matches!(
            &target.instructions[1].kind,
            InstructionKind::AssignBindingPlace {
                result,
                binding: BindingTarget::Workspace(name),
                root,
                path,
                source,
                ..
            }
                if *result == Some(Register::new(6))
                    && *name == ConstantId::new(2)
                    && *root == Register::new(5)
                    && *source == ValueSource::One(Register::new(4))
                    && path == &vec![
                        PlaceStep::Paren(vec![ApplyArgument::Value(Register::new(5))]),
                        PlaceStep::Field(FieldOperand::Static(ConstantId::new(3))),
                    ]
        ));
        verify(&BytecodeModule::new(vec![target], FunctionId::new(0)))
            .expect("linked direct binding bytecode must verify");
    }
}
