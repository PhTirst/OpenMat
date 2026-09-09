use std::collections::{BTreeMap, BTreeSet};

#[path = "argument_blocks.rs"]
mod argument_blocks;

use openmat_bytecode::{
    AbstractPropertyDefinition, Access as BytecodeAccess, ApplyArgument, AssignmentMode,
    BinaryOperator, BindingTarget, BytecodeModule, ClassDefinition as BytecodeClassDefinition,
    ClassDefinitionId, ClassFeatureDefinition, ClassKind, ClassSemantics, Constant, ConstantId,
    EnumMemberDefinition, EventDefinition, ExceptionHandler, FieldOperand, Function, FunctionId,
    Instruction, InstructionIndex, InstructionKind, LocalSlot, MethodDefinition, MethodKind,
    NamedBindingKind, PackApplyTarget, PackRegister, PersistentSlot, PlaceStep, PropertyDefinition,
    PropertyKind as BytecodePropertyKind, Register, SharedCaptureSource, SourceLocation,
    StatementResultTarget, ValueSource, verify,
};
use openmat_hir::{
    Attribute, BinaryOp, CatchClause, ClearForm, ClearStatement, CommandStatement,
    ConditionalBranch, DeclarationContext, DeclarationForm, DeclarationKind, DeclarationStatement,
    Expr, ExprKind, FunctionDef, HirFile, OtherwiseBranch, Stmt, StmtKind, SwitchCase,
    TransposeKind, TryStatement, UnaryOp,
};
use openmat_source::{SourceId, TextRange};

use crate::{
    BindingDeclarationProblem, BindingOperation, BindingStorageClass, ClassAttributeProblem,
    ClassMemberProblem, CompileDiagnostic, CompileDiagnosticKind, CompileError, CompileResult,
    CompilerResource, InternalCompilerError, UnsupportedFeature,
};

/// Compiles one HIR file into a verified register-bytecode module.
///
/// The returned module always has a synthetic entry function at function index
/// zero. No module is returned when semantic, unsupported-feature, resource, or
/// internal-verification diagnostics are present.
///
/// # Errors
///
/// Returns all diagnostics discovered in deterministic source order. Malformed
/// HIR is diagnosed and never causes a panic.
pub fn compile(file: &HirFile) -> CompileResult {
    compile_with_imports(file, &[])
}

/// Compiles one HIR file while inheriting ordered imports for its synthetic
/// entry workspace. Function and class-method scopes still declare their own
/// lexical imports.
///
/// # Errors
///
/// Returns the same deterministic diagnostics as [`compile`].
#[allow(clippy::too_many_lines)]
pub fn compile_with_imports(file: &HirFile, inherited_imports: &[String]) -> CompileResult {
    let mut diagnostics = Vec::new();
    let discovery = discover_functions(file, &mut diagnostics);
    let class_function_count = class_auxiliary_function_count(file);
    let anonymous_function_base = class_function_count
        .and_then(|count| count.checked_add(discovery.functions.len()))
        .and_then(|count| count.checked_add(1))
        .and_then(|index| u32::try_from(index).ok());
    if anonymous_function_base.is_none() {
        resource_limit(
            &mut diagnostics,
            file.source_id,
            file.span,
            CompilerResource::Functions,
        );
    }
    let mut anonymous_functions = Vec::new();
    let class_compilation = compile_classes(
        file,
        &discovery.by_name,
        discovery.functions.len(),
        anonymous_function_base,
        &mut anonymous_functions,
        &mut diagnostics,
    );
    let entry_analysis = analyse_entry_bindings(file, &mut diagnostics);

    let entry_scope = Scope::Entry {
        bindings: &entry_analysis.bindings,
    };
    let mut entry_compiler = FunctionCompiler::new(
        file.source_id,
        file.span,
        "<entry>",
        entry_scope,
        &discovery.by_name,
        &class_compilation.by_name,
        anonymous_function_base,
        &mut anonymous_functions,
        &mut diagnostics,
        0,
        0,
        0,
    );
    entry_compiler.set_imports(
        merged_imports(inherited_imports, &collect_scope_imports(&file.statements)),
        file.span,
    );
    entry_compiler.lower_statements(&file.statements, true);
    entry_compiler.emit_return(file.span);
    let entry = entry_compiler.finish();

    let local_plans = analyse_discovered_functions(file.source_id, &discovery, &mut diagnostics);
    let mut functions = Vec::with_capacity(discovery.functions.len().saturating_add(1));
    functions.push(entry);
    for (index, discovered) in discovery.functions.iter().enumerate() {
        let plan = &local_plans[index];
        let scope = Scope::Function {
            bindings: &plan.bindings,
            outputs: &plan.output_bindings,
            captures: &plan.shared_captures,
        };
        let mut compiler = FunctionCompiler::new(
            file.source_id,
            discovered.definition.span,
            &discovered.qualified_name,
            scope,
            &discovery.by_name,
            &class_compilation.by_name,
            anonymous_function_base,
            &mut anonymous_functions,
            &mut diagnostics,
            plan.local_count,
            plan.parameter_count,
            plan.persistent_slot_count,
        );
        compiler.set_imports(
            collect_scope_imports(&discovered.definition.body),
            discovered.definition.span,
        );
        compiler.nested_definitions_lowered = true;
        compiler.configure_arguments(discovered.definition);
        if let Some(slot) = plan.variadic_input_slot {
            compiler.emit_variadic_inputs(slot, discovered.definition.span);
        }
        for child in &discovered.children {
            let Some(slot) = plan.nested_function_slots.get(child).copied() else {
                continue;
            };
            let captures = local_plans[*child]
                .shared_captures
                .iter()
                .filter_map(|name| {
                    let source = match plan.bindings.get(name) {
                        Some(BindingStorage::Local(slot)) => SharedCaptureSource::Local(*slot),
                        Some(BindingStorage::Captured) => SharedCaptureSource::Enclosing,
                        _ => return None,
                    };
                    Some((name.clone(), source))
                })
                .collect::<Vec<_>>();
            let function = u32::try_from(child.saturating_add(1))
                .ok()
                .map(FunctionId::new);
            if let Some(function) = function {
                compiler.emit_nested_closure(slot, function, &captures, discovered.definition.span);
            }
        }
        compiler.lower_statements(&discovered.definition.body, false);
        compiler.emit_return(discovered.definition.span);
        functions.push(compiler.finish());
    }
    functions.extend(class_compilation.functions);
    functions.extend(anonymous_functions);

    if !diagnostics.is_empty() {
        diagnostics.sort_by_key(|diagnostic| {
            (
                diagnostic.source_id,
                diagnostic.range.start(),
                diagnostic.range.end(),
            )
        });
        return Err(CompileError::new(diagnostics));
    }

    let module = BytecodeModule::new(functions, FunctionId::new(0))
        .with_classes(class_compilation.definitions)
        .with_class_features(class_compilation.features);
    if let Err(error) = verify(&module) {
        diagnostics.push(CompileDiagnostic::new(
            file.source_id,
            error
                .location
                .and_then(|location| TextRange::new(location.start, location.end).ok())
                .unwrap_or(file.span),
            CompileDiagnosticKind::Internal {
                error: InternalCompilerError::BytecodeVerification(error),
            },
        ));
        return Err(CompileError::new(diagnostics));
    }
    Ok(module)
}

struct Discovery<'hir> {
    by_name: BTreeMap<String, FunctionId>,
    functions: Vec<DiscoveredFunction<'hir>>,
}

struct DiscoveredFunction<'hir> {
    name: String,
    qualified_name: String,
    definition: &'hir FunctionDef,
    parent: Option<usize>,
    children: Vec<usize>,
}

struct ClassCompilation {
    by_name: BTreeMap<String, ClassDefinitionId>,
    definitions: Vec<BytecodeClassDefinition>,
    features: Vec<ClassFeatureDefinition>,
    functions: Vec<Function>,
}

fn class_auxiliary_function_count(file: &HirFile) -> Option<usize> {
    file.statements
        .iter()
        .try_fold(0_usize, |count, statement| {
            let StmtKind::Class(class) = &statement.kind else {
                return Some(count);
            };
            let defaults = class
                .property_blocks
                .iter()
                .flat_map(|block| &block.properties)
                .filter(|property| property.default.is_some())
                .count();
            let methods = class
                .method_blocks
                .iter()
                .try_fold(0_usize, |total, block| {
                    total
                        .checked_add(block.methods.len())?
                        .checked_add(block.declarations.len())
                })?;
            let enumeration_members = class
                .enumeration_blocks
                .iter()
                .map(|block| block.members.len())
                .sum::<usize>();
            count
                .checked_add(defaults)?
                .checked_add(methods)?
                .checked_add(enumeration_members)
        })
}

#[allow(clippy::too_many_lines)]
fn compile_classes(
    file: &HirFile,
    user_functions: &BTreeMap<String, FunctionId>,
    top_level_function_count: usize,
    anonymous_function_base: Option<u32>,
    anonymous_functions: &mut Vec<Function>,
    diagnostics: &mut Vec<CompileDiagnostic>,
) -> ClassCompilation {
    let class_count = file
        .statements
        .iter()
        .filter(|statement| matches!(statement.kind, StmtKind::Class(_)))
        .count();
    let mut compilation = ClassCompilation {
        by_name: BTreeMap::new(),
        definitions: Vec::new(),
        features: Vec::new(),
        functions: Vec::new(),
    };
    if compilation.definitions.try_reserve(class_count).is_err() {
        resource_limit(
            diagnostics,
            file.source_id,
            file.span,
            CompilerResource::Classes,
        );
        return compilation;
    }
    let auxiliary_function_count = class_auxiliary_function_count(file);
    if auxiliary_function_count
        .is_none_or(|count| compilation.functions.try_reserve(count).is_err())
    {
        resource_limit(
            diagnostics,
            file.source_id,
            file.span,
            CompilerResource::Functions,
        );
        return compilation;
    }

    let Some(mut next_function_index) = top_level_function_count.checked_add(1) else {
        resource_limit(
            diagnostics,
            file.source_id,
            file.span,
            CompilerResource::Functions,
        );
        return compilation;
    };

    for statement in &file.statements {
        let StmtKind::Class(class) = &statement.kind else {
            continue;
        };
        let Some(name) = class.name.as_ref() else {
            malformed(
                diagnostics,
                file.source_id,
                class.span,
                "class definition is missing its name",
            );
            continue;
        };
        if name.text.is_empty() {
            malformed(
                diagnostics,
                file.source_id,
                name.span,
                "class definition has an empty name",
            );
            continue;
        }
        let Ok(class_index) = u32::try_from(compilation.definitions.len()) else {
            resource_limit(
                diagnostics,
                file.source_id,
                class.span,
                CompilerResource::Classes,
            );
            continue;
        };
        if compilation
            .by_name
            .insert(name.text.clone(), ClassDefinitionId::new(class_index))
            .is_some()
        {
            malformed(
                diagnostics,
                file.source_id,
                name.span,
                "class name is defined more than once in this source file",
            );
            continue;
        }

        let class_attributes =
            class_attributes(file.source_id, &name.text, &class.attributes, diagnostics);
        let is_enumeration = !class.enumeration_blocks.is_empty();
        let enumeration_base = is_enumeration
            .then(|| class.superclass.as_ref().map(|base| base.text.clone()))
            .flatten();
        let (semantics, superclass) = match class.superclass.as_ref() {
            Some(superclass) if is_enumeration && superclass.text == "handle" => {
                (ClassSemantics::Handle, None)
            }
            Some(_) if is_enumeration => (ClassSemantics::Value, None),
            None => (ClassSemantics::Value, None),
            Some(superclass) if superclass.text == "handle" => (ClassSemantics::Handle, None),
            Some(superclass) => (ClassSemantics::Value, Some(superclass.text.clone())),
        };

        let property_count = class
            .property_blocks
            .iter()
            .try_fold(0_usize, |count, block| {
                count.checked_add(block.properties.len())
            });
        let method_count = class
            .method_blocks
            .iter()
            .try_fold(0_usize, |count, block| {
                count
                    .checked_add(block.methods.len())?
                    .checked_add(block.declarations.len())
            });
        let mut properties = Vec::new();
        let mut methods = Vec::new();
        let mut abstract_properties = Vec::new();
        let mut enumeration_members = Vec::new();
        let mut events = Vec::new();
        let mut sealed_methods = Vec::new();
        let mut member_names = BTreeMap::new();
        if property_count.is_none_or(|count| properties.try_reserve(count).is_err())
            || method_count.is_none_or(|count| methods.try_reserve(count).is_err())
        {
            resource_limit(
                diagnostics,
                file.source_id,
                class.span,
                CompilerResource::Functions,
            );
            continue;
        }

        for block in &class.property_blocks {
            let property_attributes =
                property_attributes(file.source_id, &block.attributes, diagnostics);
            for property in &block.properties {
                let Some(property_name) = property.name.as_ref() else {
                    malformed(
                        diagnostics,
                        file.source_id,
                        property.span,
                        "property declaration is missing its name",
                    );
                    continue;
                };
                register_class_member_name(
                    &mut member_names,
                    diagnostics,
                    file.source_id,
                    property_name.span,
                    &name.text,
                    &property_name.text,
                    if property_attributes.is_abstract {
                        "abstract property"
                    } else {
                        "property"
                    },
                );
                if property_attributes.conflicting_kinds {
                    invalid_class_member(
                        diagnostics,
                        file.source_id,
                        property.span,
                        &name.text,
                        &property_name.text,
                        ClassMemberProblem::ConflictingPropertyKinds,
                    );
                }
                if property_attributes.is_abstract {
                    if property.default.is_some() {
                        invalid_class_member(
                            diagnostics,
                            file.source_id,
                            property.span,
                            &name.text,
                            &property_name.text,
                            ClassMemberProblem::AbstractPropertyInitializer,
                        );
                    }
                    abstract_properties.push(AbstractPropertyDefinition {
                        name: property_name.text.clone(),
                        kind: property_attributes.kind,
                        get_access: Some(property_attributes.get_access),
                        set_access: (property_attributes.kind != BytecodePropertyKind::Constant)
                            .then_some(property_attributes.set_access),
                        location: source_location(file.source_id, property.span),
                    });
                    continue;
                }
                if property_attributes.kind == BytecodePropertyKind::Constant
                    && property.default.is_none()
                {
                    malformed(
                        diagnostics,
                        file.source_id,
                        property.span,
                        "constant property must declare an initializer",
                    );
                }
                let (get_access, set_access) =
                    if property_attributes.kind == BytecodePropertyKind::Dependent {
                        if property.default.is_some() {
                            invalid_class_member(
                                diagnostics,
                                file.source_id,
                                property.span,
                                &name.text,
                                &property_name.text,
                                ClassMemberProblem::DependentPropertyInitializer,
                            );
                        }
                        let readable = class_has_accessor(class, "get", &property_name.text);
                        let writable = class_has_accessor(class, "set", &property_name.text);
                        if !readable && !writable {
                            invalid_class_member(
                                diagnostics,
                                file.source_id,
                                property.span,
                                &name.text,
                                &property_name.text,
                                ClassMemberProblem::MissingDependentAccessors,
                            );
                        }
                        (
                            readable.then_some(property_attributes.get_access),
                            writable.then_some(property_attributes.set_access),
                        )
                    } else if property_attributes.kind == BytecodePropertyKind::Constant {
                        (Some(property_attributes.get_access), None)
                    } else {
                        (
                            Some(property_attributes.get_access),
                            Some(property_attributes.set_access),
                        )
                    };
                let default = property
                    .default
                    .as_ref()
                    .filter(|_| property_attributes.kind != BytecodePropertyKind::Dependent)
                    .and_then(|expression| {
                        let function_id = allocate_function_id(
                            &mut next_function_index,
                            file.source_id,
                            property.span,
                            diagnostics,
                        )?;
                        let empty_locals = BTreeMap::new();
                        let empty_outputs = Vec::new();
                        let empty_captures = BTreeSet::new();
                        let empty_classes = BTreeMap::new();
                        let mut compiler = FunctionCompiler::new(
                            file.source_id,
                            property.span,
                            &format!("{}.<default:{}>", name.text, property_name.text),
                            Scope::Function {
                                bindings: &empty_locals,
                                outputs: &empty_outputs,
                                captures: &empty_captures,
                            },
                            user_functions,
                            &empty_classes,
                            anonymous_function_base,
                            anonymous_functions,
                            diagnostics,
                            0,
                            0,
                            0,
                        );
                        if let Some(value) = compiler.lower_expression(expression) {
                            compiler.emit_values_return(&[value], expression.span);
                        } else {
                            compiler.emit_values_return(&[], expression.span);
                        }
                        compilation.functions.push(compiler.finish());
                        Some(function_id)
                    });
                properties.push(PropertyDefinition {
                    name: property_name.text.clone(),
                    kind: property_attributes.kind,
                    default,
                    get_access,
                    set_access,
                    location: source_location(file.source_id, property.span),
                });
            }
        }

        for block in &class.method_blocks {
            let method_attributes =
                method_attributes(file.source_id, &block.attributes, diagnostics);
            for method in &block.methods {
                let Some(method_name) = method.name.as_ref() else {
                    malformed(
                        diagnostics,
                        file.source_id,
                        method.span,
                        "method definition is missing its name",
                    );
                    continue;
                };
                register_class_member_name(
                    &mut member_names,
                    diagnostics,
                    file.source_id,
                    method_name.span,
                    &name.text,
                    &method_name.text,
                    "method",
                );
                let is_constructor = method_name.text == name.text;
                if method_attributes.is_abstract {
                    invalid_class_member(
                        diagnostics,
                        file.source_id,
                        method.span,
                        &name.text,
                        &method_name.text,
                        ClassMemberProblem::ConcreteMethodInAbstractBlock,
                    );
                }
                let accessor = accessor_parts(&method_name.text);
                if method_name.text.contains('.') {
                    let Some((kind, property_name)) = accessor else {
                        invalid_class_member(
                            diagnostics,
                            file.source_id,
                            method_name.span,
                            &name.text,
                            &method_name.text,
                            ClassMemberProblem::AccessorTargetNotDependent,
                        );
                        continue;
                    };
                    let target_is_dependent = properties
                        .iter()
                        .find(|property| property.name == property_name)
                        .map_or_else(
                            || superclass.is_some(),
                            |property| property.kind == BytecodePropertyKind::Dependent,
                        );
                    if !target_is_dependent {
                        invalid_class_member(
                            diagnostics,
                            file.source_id,
                            method_name.span,
                            &name.text,
                            &method_name.text,
                            ClassMemberProblem::AccessorTargetNotDependent,
                        );
                    }
                    if method_attributes.kind != MethodKind::Instance {
                        invalid_class_member(
                            diagnostics,
                            file.source_id,
                            method_name.span,
                            &name.text,
                            &method_name.text,
                            ClassMemberProblem::MethodMustBeInstance,
                        );
                    }
                    let expected_inputs = if kind == "get" { 1 } else { 2 };
                    validate_class_method_signature(
                        diagnostics,
                        file.source_id,
                        method,
                        &name.text,
                        &method_name.text,
                        expected_inputs,
                        1,
                    );
                }
                if !is_constructor && is_binary_operator_method(&method_name.text) {
                    if method_attributes.kind != MethodKind::Instance {
                        invalid_class_member(
                            diagnostics,
                            file.source_id,
                            method_name.span,
                            &name.text,
                            &method_name.text,
                            ClassMemberProblem::MethodMustBeInstance,
                        );
                    }
                    validate_class_method_signature(
                        diagnostics,
                        file.source_id,
                        method,
                        &name.text,
                        &method_name.text,
                        2,
                        1,
                    );
                } else if !is_constructor && is_operator_method(&method_name.text) {
                    diagnostics.push(CompileDiagnostic::new(
                        file.source_id,
                        method_name.span,
                        CompileDiagnosticKind::Unsupported {
                            feature: UnsupportedFeature::OperatorOverload(method_name.text.clone()),
                        },
                    ));
                }
                if is_constructor && method_attributes.kind == MethodKind::Static {
                    malformed(
                        diagnostics,
                        file.source_id,
                        method.span,
                        "constructor cannot be declared in a static methods block",
                    );
                } else if is_constructor && method.outputs.is_empty() {
                    malformed(
                        diagnostics,
                        file.source_id,
                        method.span,
                        "constructor must declare an object output",
                    );
                } else if !is_constructor
                    && method_attributes.kind == MethodKind::Instance
                    && accessor.is_none()
                    && method.inputs.is_empty()
                {
                    malformed(
                        diagnostics,
                        file.source_id,
                        method.span,
                        "instance method must declare a receiver input",
                    );
                }

                let Some(function_id) = allocate_function_id(
                    &mut next_function_index,
                    file.source_id,
                    method.span,
                    diagnostics,
                ) else {
                    continue;
                };
                let analysis = analyse_locals(file.source_id, method, diagnostics);
                if is_constructor
                    && method_attributes.kind == MethodKind::Instance
                    && let Some(output) = analysis.output_bindings.first()
                    && output.storage.class() != BindingStorageClass::Local
                {
                    diagnostics.push(CompileDiagnostic::new(
                        file.source_id,
                        method
                            .outputs
                            .first()
                            .map_or(method.span, |output| output.span),
                        CompileDiagnosticKind::UnsupportedBindingOperation {
                            name: output.name.clone(),
                            storage: output.storage.class(),
                            operation: BindingOperation::ConstructorOutput,
                        },
                    ));
                }
                let empty_captures = BTreeSet::new();
                let scope = Scope::Function {
                    bindings: &analysis.bindings,
                    outputs: &analysis.output_bindings,
                    captures: &empty_captures,
                };
                let empty_classes = BTreeMap::new();
                let mut compiler = FunctionCompiler::new(
                    file.source_id,
                    method.span,
                    &format!("{}.{}", name.text, method_name.text),
                    scope,
                    user_functions,
                    &empty_classes,
                    anonymous_function_base,
                    anonymous_functions,
                    diagnostics,
                    analysis.local_count,
                    analysis.parameter_count,
                    analysis.persistent_slot_count,
                );
                compiler.set_imports(collect_scope_imports(&method.body), method.span);
                compiler.configure_arguments(method);
                if let Some(slot) = analysis.variadic_input_slot {
                    compiler.emit_variadic_inputs(slot, method.span);
                }
                compiler.constructor_context = is_constructor.then(|| ConstructorContext {
                    class_name: name.text.clone(),
                    direct_superclass: superclass.clone(),
                    object_name: method.outputs.first().map(|output| output.text.clone()),
                    object_slot: analysis.output_abi_slots.first().copied().flatten(),
                });
                compiler.lower_statements(&method.body, false);
                compiler.emit_return(method.span);
                compilation.functions.push(compiler.finish());
                methods.push(MethodDefinition {
                    name: method_name.text.clone(),
                    function: function_id,
                    kind: method_attributes.kind,
                    access: method_attributes.access,
                    is_abstract: false,
                    is_external: false,
                    constructor_output: (is_constructor
                        && method_attributes.kind == MethodKind::Instance)
                        .then(|| analysis.output_abi_slots.first().copied().flatten())
                        .flatten(),
                    location: source_location(file.source_id, method.span),
                });
                if method_attributes.is_sealed {
                    sealed_methods.push(method_name.text.clone());
                }
            }

            for method in &block.declarations {
                let Some(method_name) = method.name.as_ref() else {
                    malformed(
                        diagnostics,
                        file.source_id,
                        method.span,
                        "method declaration is missing its name",
                    );
                    continue;
                };
                register_class_member_name(
                    &mut member_names,
                    diagnostics,
                    file.source_id,
                    method_name.span,
                    &name.text,
                    &method_name.text,
                    if method_attributes.is_abstract {
                        "abstract method"
                    } else {
                        "external method"
                    },
                );
                let is_abstract = method_attributes.is_abstract;
                let is_external = !is_abstract;
                let is_constructor = method_name.text == name.text;
                if is_abstract && is_constructor {
                    invalid_class_member(
                        diagnostics,
                        file.source_id,
                        method.span,
                        &name.text,
                        &method_name.text,
                        ClassMemberProblem::AbstractConstructor,
                    );
                }
                if is_constructor && method_attributes.kind == MethodKind::Static {
                    malformed(
                        diagnostics,
                        file.source_id,
                        method.span,
                        "constructor cannot be declared in a static methods block",
                    );
                } else if is_constructor && method.outputs.is_empty() {
                    malformed(
                        diagnostics,
                        file.source_id,
                        method.span,
                        "constructor must declare an object output",
                    );
                } else if !is_constructor
                    && method_attributes.kind == MethodKind::Instance
                    && method.inputs.is_empty()
                {
                    malformed(
                        diagnostics,
                        file.source_id,
                        method.span,
                        "instance method must declare a receiver input",
                    );
                }
                if !is_constructor && is_binary_operator_method(&method_name.text) {
                    if method_attributes.kind != MethodKind::Instance {
                        invalid_class_member(
                            diagnostics,
                            file.source_id,
                            method_name.span,
                            &name.text,
                            &method_name.text,
                            ClassMemberProblem::MethodMustBeInstance,
                        );
                    }
                    validate_class_method_signature(
                        diagnostics,
                        file.source_id,
                        &FunctionDef {
                            name: method.name.clone(),
                            outputs: method.outputs.clone(),
                            inputs: method.inputs.clone(),
                            body: Vec::new(),
                            span: method.span,
                        },
                        &name.text,
                        &method_name.text,
                        2,
                        1,
                    );
                } else if !is_constructor && is_operator_method(&method_name.text) {
                    diagnostics.push(CompileDiagnostic::new(
                        file.source_id,
                        method_name.span,
                        CompileDiagnosticKind::Unsupported {
                            feature: UnsupportedFeature::OperatorOverload(method_name.text.clone()),
                        },
                    ));
                }

                let Some(function_id) = allocate_function_id(
                    &mut next_function_index,
                    file.source_id,
                    method.span,
                    diagnostics,
                ) else {
                    continue;
                };
                let signature = FunctionDef {
                    name: method.name.clone(),
                    outputs: method.outputs.clone(),
                    inputs: method.inputs.clone(),
                    body: Vec::new(),
                    span: method.span,
                };
                let analysis = analyse_locals(file.source_id, &signature, diagnostics);
                if is_constructor
                    && method_attributes.kind == MethodKind::Instance
                    && let Some(output) = analysis.output_bindings.first()
                    && output.storage.class() != BindingStorageClass::Local
                {
                    diagnostics.push(CompileDiagnostic::new(
                        file.source_id,
                        method
                            .outputs
                            .first()
                            .map_or(method.span, |output| output.span),
                        CompileDiagnosticKind::UnsupportedBindingOperation {
                            name: output.name.clone(),
                            storage: output.storage.class(),
                            operation: BindingOperation::ConstructorOutput,
                        },
                    ));
                }
                let empty_captures = BTreeSet::new();
                let scope = Scope::Function {
                    bindings: &analysis.bindings,
                    outputs: &analysis.output_bindings,
                    captures: &empty_captures,
                };
                let empty_classes = BTreeMap::new();
                let mut compiler = FunctionCompiler::new(
                    file.source_id,
                    method.span,
                    &format!(
                        "{}.<{}:{}>",
                        name.text,
                        if is_abstract { "abstract" } else { "external" },
                        method_name.text
                    ),
                    scope,
                    user_functions,
                    &empty_classes,
                    anonymous_function_base,
                    anonymous_functions,
                    diagnostics,
                    analysis.local_count,
                    analysis.parameter_count,
                    analysis.persistent_slot_count,
                );
                if let Some(slot) = analysis.variadic_input_slot {
                    compiler.emit_variadic_inputs(slot, method.span);
                }
                compiler.emit_return(method.span);
                compilation.functions.push(compiler.finish());
                methods.push(MethodDefinition {
                    name: method_name.text.clone(),
                    function: function_id,
                    kind: method_attributes.kind,
                    access: method_attributes.access,
                    is_abstract,
                    is_external,
                    constructor_output: (is_external
                        && is_constructor
                        && method_attributes.kind == MethodKind::Instance)
                        .then(|| analysis.output_abi_slots.first().copied().flatten())
                        .flatten(),
                    location: source_location(file.source_id, method.span),
                });
                if method_attributes.is_sealed {
                    sealed_methods.push(method_name.text.clone());
                }
            }
        }

        for block in &class.enumeration_blocks {
            for attribute in &block.attributes {
                let attribute_name = attribute
                    .name
                    .as_ref()
                    .map_or("<missing>", |name| name.text.as_str());
                unsupported_attribute(
                    file.source_id,
                    attribute,
                    AttributeContext::Enumeration,
                    attribute_name,
                    diagnostics,
                );
            }
            for member in &block.members {
                let Some(member_name) = member.name.as_ref() else {
                    malformed(
                        diagnostics,
                        file.source_id,
                        member.span,
                        "enumeration member is missing its name",
                    );
                    continue;
                };
                register_class_member_name(
                    &mut member_names,
                    diagnostics,
                    file.source_id,
                    member_name.span,
                    &name.text,
                    &member_name.text,
                    "enumeration member",
                );
                let Some(function_id) = allocate_function_id(
                    &mut next_function_index,
                    file.source_id,
                    member.span,
                    diagnostics,
                ) else {
                    continue;
                };
                let Ok(argument_count) = u32::try_from(member.arguments.len()) else {
                    resource_limit(
                        diagnostics,
                        file.source_id,
                        member.span,
                        CompilerResource::Parameters,
                    );
                    continue;
                };
                let empty_locals = BTreeMap::new();
                let empty_outputs = Vec::new();
                let empty_captures = BTreeSet::new();
                let empty_classes = BTreeMap::new();
                let mut compiler = FunctionCompiler::new(
                    file.source_id,
                    member.span,
                    &format!("{}.<enumeration:{}>", name.text, member_name.text),
                    Scope::Function {
                        bindings: &empty_locals,
                        outputs: &empty_outputs,
                        captures: &empty_captures,
                    },
                    user_functions,
                    &empty_classes,
                    anonymous_function_base,
                    anonymous_functions,
                    diagnostics,
                    0,
                    0,
                    0,
                );
                let arguments = member
                    .arguments
                    .iter()
                    .filter_map(|argument| compiler.lower_expression(argument))
                    .collect::<Vec<_>>();
                compiler.emit_values_return(&arguments, member.span);
                compilation.functions.push(compiler.finish());
                enumeration_members.push(EnumMemberDefinition {
                    name: member_name.text.clone(),
                    argument_initializer: function_id,
                    argument_count,
                    location: source_location(file.source_id, member.span),
                });
            }
        }
        if is_enumeration && enumeration_members.is_empty() {
            invalid_class_member(
                diagnostics,
                file.source_id,
                class.span,
                &name.text,
                "<enumeration>",
                ClassMemberProblem::EmptyEnumeration,
            );
        }

        for block in &class.event_blocks {
            let attributes = event_attributes(file.source_id, &block.attributes, diagnostics);
            for event in &block.events {
                let Some(event_name) = event.name.as_ref() else {
                    malformed(
                        diagnostics,
                        file.source_id,
                        event.span,
                        "event declaration is missing its name",
                    );
                    continue;
                };
                register_class_member_name(
                    &mut member_names,
                    diagnostics,
                    file.source_id,
                    event_name.span,
                    &name.text,
                    &event_name.text,
                    "event",
                );
                events.push(EventDefinition {
                    name: event_name.text.clone(),
                    listen_access: attributes.listen_access,
                    notify_access: attributes.notify_access,
                    hidden: attributes.hidden,
                    location: source_location(file.source_id, event.span),
                });
            }
        }
        if !events.is_empty() && class.superclass.is_none() && semantics != ClassSemantics::Handle {
            invalid_class_member(
                diagnostics,
                file.source_id,
                class.span,
                &name.text,
                "<events>",
                ClassMemberProblem::EventsRequireHandleClass,
            );
        }

        if class_attributes.declared_abstract == Some(false)
            && (methods.iter().any(|method| method.is_abstract) || !abstract_properties.is_empty())
        {
            diagnostics.push(CompileDiagnostic::new(
                file.source_id,
                class.span,
                CompileDiagnosticKind::InvalidClassAttribute {
                    class: name.text.clone(),
                    attribute: "Abstract".to_owned(),
                    problem: ClassAttributeProblem::ExplicitConcreteWithAbstractMethods,
                },
            ));
        }
        if is_enumeration
            && (class_attributes.declared_abstract == Some(true)
                || methods.iter().any(|method| method.is_abstract)
                || !abstract_properties.is_empty())
        {
            diagnostics.push(CompileDiagnostic::new(
                file.source_id,
                class.span,
                CompileDiagnosticKind::InvalidClassAttribute {
                    class: name.text.clone(),
                    attribute: "Abstract".to_owned(),
                    problem: ClassAttributeProblem::EnumerationCannotBeAbstract,
                },
            ));
        }

        compilation.definitions.push(BytecodeClassDefinition {
            name: name.text.clone(),
            semantics,
            superclass,
            declared_abstract: class_attributes.declared_abstract,
            sealed: class_attributes.sealed,
            properties,
            methods,
            location: source_location(file.source_id, class.span),
        });
        if is_enumeration
            || !events.is_empty()
            || !abstract_properties.is_empty()
            || !sealed_methods.is_empty()
        {
            compilation.features.push(ClassFeatureDefinition {
                class: ClassDefinitionId::new(class_index),
                kind: if is_enumeration {
                    ClassKind::Enumeration
                } else {
                    ClassKind::Ordinary
                },
                enumeration_base,
                enumeration_members,
                events,
                abstract_properties,
                sealed_methods,
                location: source_location(file.source_id, class.span),
            });
        }
    }
    let enumeration_names = compilation
        .features
        .iter()
        .filter(|feature| feature.kind == ClassKind::Enumeration)
        .filter_map(|feature| {
            compilation
                .definitions
                .get(feature.class.get() as usize)
                .map(|class| class.name.clone())
        })
        .collect::<BTreeSet<_>>();
    for class in &compilation.definitions {
        if let Some(superclass) = class
            .superclass
            .as_ref()
            .filter(|superclass| enumeration_names.contains(*superclass))
        {
            let span =
                TextRange::new(class.location.start, class.location.end).unwrap_or(file.span);
            invalid_class_member(
                diagnostics,
                file.source_id,
                span,
                &class.name,
                "<superclass>",
                ClassMemberProblem::EnumerationSuperclass {
                    superclass: superclass.clone(),
                },
            );
        }
    }
    compilation
}

fn allocate_function_id(
    next: &mut usize,
    source_id: SourceId,
    span: TextRange,
    diagnostics: &mut Vec<CompileDiagnostic>,
) -> Option<FunctionId> {
    let Ok(raw) = u32::try_from(*next) else {
        resource_limit(diagnostics, source_id, span, CompilerResource::Functions);
        return None;
    };
    *next = next.checked_add(1).unwrap_or(usize::MAX);
    Some(FunctionId::new(raw))
}

#[derive(Clone, Copy)]
enum AttributeContext {
    Class,
    Property,
    Method,
    Enumeration,
    Event,
}

#[derive(Clone, Copy, Default)]
struct CompiledClassAttributes {
    declared_abstract: Option<bool>,
    sealed: bool,
}

fn class_attributes(
    source_id: SourceId,
    class: &str,
    attributes: &[Attribute],
    diagnostics: &mut Vec<CompileDiagnostic>,
) -> CompiledClassAttributes {
    let mut compiled = CompiledClassAttributes::default();
    let mut seen = BTreeSet::new();
    for attribute in attributes {
        let name = attribute
            .name
            .as_ref()
            .map_or("<missing>", |name| name.text.as_str());
        if !matches!(name, "Abstract" | "Sealed") {
            unsupported_attribute(
                source_id,
                attribute,
                AttributeContext::Class,
                name,
                diagnostics,
            );
            continue;
        }
        if !seen.insert(name) {
            diagnostics.push(CompileDiagnostic::new(
                source_id,
                attribute.span,
                CompileDiagnosticKind::InvalidClassAttribute {
                    class: class.to_owned(),
                    attribute: name.to_owned(),
                    problem: ClassAttributeProblem::Duplicate,
                },
            ));
            continue;
        }
        let Some(value) = boolean_attribute_value(attribute) else {
            diagnostics.push(CompileDiagnostic::new(
                source_id,
                attribute.span,
                CompileDiagnosticKind::InvalidClassAttribute {
                    class: class.to_owned(),
                    attribute: name.to_owned(),
                    problem: ClassAttributeProblem::InvalidBooleanValue,
                },
            ));
            continue;
        };
        match name {
            "Abstract" => compiled.declared_abstract = Some(value),
            "Sealed" => compiled.sealed = value,
            _ => unreachable!("recognized class attribute"),
        }
    }
    compiled
}

fn boolean_attribute_value(attribute: &Attribute) -> Option<bool> {
    match attribute.value.as_ref().map(|value| &value.kind) {
        None => Some(true),
        Some(ExprKind::Name(value)) if value == "true" => Some(true),
        Some(ExprKind::Name(value)) if value == "false" => Some(false),
        _ => None,
    }
}

#[derive(Clone, Copy)]
struct CompiledPropertyAttributes {
    kind: BytecodePropertyKind,
    get_access: BytecodeAccess,
    set_access: BytecodeAccess,
    conflicting_kinds: bool,
    is_abstract: bool,
}

fn property_attributes(
    source_id: SourceId,
    attributes: &[Attribute],
    diagnostics: &mut Vec<CompileDiagnostic>,
) -> CompiledPropertyAttributes {
    let constant = attributes.iter().any(|attribute| {
        attribute
            .name
            .as_ref()
            .is_some_and(|name| name.text == "Constant")
            && attribute.value.is_none()
    });
    let dependent = attributes.iter().any(|attribute| {
        attribute
            .name
            .as_ref()
            .is_some_and(|name| name.text == "Dependent")
            && attribute.value.is_none()
    });
    let is_abstract = attributes.iter().any(|attribute| {
        attribute
            .name
            .as_ref()
            .is_some_and(|name| name.text == "Abstract")
            && boolean_attribute_value(attribute) == Some(true)
    });
    let mut get_access = BytecodeAccess::Public;
    let mut set_access = BytecodeAccess::Public;
    for attribute in attributes {
        let name = attribute
            .name
            .as_ref()
            .map_or("<missing>", |name| name.text.as_str());
        if matches!(name, "Constant" | "Dependent") && attribute.value.is_none() {
            continue;
        }
        if name == "Abstract" && boolean_attribute_value(attribute).is_some() {
            continue;
        }
        let Some(access) = attribute_access(attribute) else {
            unsupported_attribute(
                source_id,
                attribute,
                AttributeContext::Property,
                name,
                diagnostics,
            );
            continue;
        };
        match name {
            "Access" => {
                get_access = access;
                set_access = access;
            }
            "GetAccess" => get_access = access,
            "SetAccess" if !constant => set_access = access,
            _ => unsupported_attribute(
                source_id,
                attribute,
                AttributeContext::Property,
                name,
                diagnostics,
            ),
        }
    }
    CompiledPropertyAttributes {
        kind: if dependent {
            BytecodePropertyKind::Dependent
        } else if constant {
            BytecodePropertyKind::Constant
        } else {
            BytecodePropertyKind::Stored
        },
        get_access,
        set_access,
        conflicting_kinds: constant && dependent,
        is_abstract,
    }
}

fn class_has_accessor(class: &openmat_hir::ClassDef, kind: &str, property: &str) -> bool {
    class.method_blocks.iter().any(|block| {
        block.methods.iter().any(|method| {
            method
                .name
                .as_ref()
                .and_then(|name| accessor_parts(&name.text))
                .is_some_and(|(candidate_kind, candidate_property)| {
                    candidate_kind == kind && candidate_property == property
                })
        })
    })
}

fn accessor_parts(name: &str) -> Option<(&str, &str)> {
    let (kind, property) = name.split_once('.')?;
    (matches!(kind, "get" | "set") && !property.is_empty() && !property.contains('.'))
        .then_some((kind, property))
}

#[allow(clippy::too_many_arguments)]
fn validate_class_method_signature(
    diagnostics: &mut Vec<CompileDiagnostic>,
    source_id: SourceId,
    method: &FunctionDef,
    class: &str,
    member: &str,
    expected_inputs: usize,
    expected_outputs: usize,
) {
    if method.inputs.len() != expected_inputs || method.outputs.len() != expected_outputs {
        invalid_class_member(
            diagnostics,
            source_id,
            method.span,
            class,
            member,
            ClassMemberProblem::InvalidSignature {
                expected_inputs,
                actual_inputs: method.inputs.len(),
                expected_outputs,
                actual_outputs: method.outputs.len(),
            },
        );
    }
}

fn invalid_class_member(
    diagnostics: &mut Vec<CompileDiagnostic>,
    source_id: SourceId,
    span: TextRange,
    class: &str,
    member: &str,
    problem: ClassMemberProblem,
) {
    diagnostics.push(CompileDiagnostic::new(
        source_id,
        span,
        CompileDiagnosticKind::InvalidClassMember {
            class: class.to_owned(),
            member: member.to_owned(),
            problem,
        },
    ));
}

fn register_class_member_name(
    names: &mut BTreeMap<String, &'static str>,
    diagnostics: &mut Vec<CompileDiagnostic>,
    source_id: SourceId,
    span: TextRange,
    class: &str,
    member: &str,
    kind: &'static str,
) {
    if let Some(previous_kind) = names.insert(member.to_owned(), kind) {
        invalid_class_member(
            diagnostics,
            source_id,
            span,
            class,
            member,
            ClassMemberProblem::DuplicateOrConflictingDeclaration { previous_kind },
        );
    }
}

#[derive(Clone, Copy)]
struct CompiledMethodAttributes {
    kind: MethodKind,
    access: BytecodeAccess,
    is_abstract: bool,
    is_sealed: bool,
}

fn method_attributes(
    source_id: SourceId,
    attributes: &[Attribute],
    diagnostics: &mut Vec<CompileDiagnostic>,
) -> CompiledMethodAttributes {
    let is_static = attributes.iter().any(|attribute| {
        attribute
            .name
            .as_ref()
            .is_some_and(|name| name.text == "Static")
            && attribute.value.is_none()
    });
    let is_abstract = attributes.iter().any(|attribute| {
        attribute
            .name
            .as_ref()
            .is_some_and(|name| name.text == "Abstract")
            && boolean_attribute_value(attribute) == Some(true)
    });
    let is_sealed = attributes.iter().any(|attribute| {
        attribute
            .name
            .as_ref()
            .is_some_and(|name| name.text == "Sealed")
            && boolean_attribute_value(attribute) == Some(true)
    });
    let mut access = BytecodeAccess::Public;
    for attribute in attributes {
        let name = attribute
            .name
            .as_ref()
            .map_or("<missing>", |name| name.text.as_str());
        if name == "Static" && attribute.value.is_none() {
            continue;
        }
        if name == "Abstract" && boolean_attribute_value(attribute).is_some() {
            continue;
        }
        if name == "Sealed" && boolean_attribute_value(attribute).is_some() {
            continue;
        }
        if name == "Access"
            && let Some(value) = attribute_access(attribute)
        {
            access = value;
        } else {
            unsupported_attribute(
                source_id,
                attribute,
                AttributeContext::Method,
                name,
                diagnostics,
            );
        }
    }
    CompiledMethodAttributes {
        kind: if is_static {
            MethodKind::Static
        } else {
            MethodKind::Instance
        },
        access,
        is_abstract,
        is_sealed,
    }
}

#[derive(Clone, Copy)]
struct CompiledEventAttributes {
    listen_access: BytecodeAccess,
    notify_access: BytecodeAccess,
    hidden: bool,
}

fn event_attributes(
    source_id: SourceId,
    attributes: &[Attribute],
    diagnostics: &mut Vec<CompileDiagnostic>,
) -> CompiledEventAttributes {
    let mut compiled = CompiledEventAttributes {
        listen_access: BytecodeAccess::Public,
        notify_access: BytecodeAccess::Public,
        hidden: false,
    };
    let mut seen = BTreeSet::new();
    for attribute in attributes {
        let name = attribute
            .name
            .as_ref()
            .map_or("<missing>", |name| name.text.as_str());
        if !seen.insert(name) {
            unsupported_attribute(
                source_id,
                attribute,
                AttributeContext::Event,
                name,
                diagnostics,
            );
            continue;
        }
        match name {
            "ListenAccess" => {
                if let Some(access) = attribute_access(attribute) {
                    compiled.listen_access = access;
                } else {
                    unsupported_attribute(
                        source_id,
                        attribute,
                        AttributeContext::Event,
                        name,
                        diagnostics,
                    );
                }
            }
            "NotifyAccess" => {
                if let Some(access) = attribute_access(attribute) {
                    compiled.notify_access = access;
                } else {
                    unsupported_attribute(
                        source_id,
                        attribute,
                        AttributeContext::Event,
                        name,
                        diagnostics,
                    );
                }
            }
            "Hidden" => {
                if let Some(hidden) = boolean_attribute_value(attribute) {
                    compiled.hidden = hidden;
                } else {
                    unsupported_attribute(
                        source_id,
                        attribute,
                        AttributeContext::Event,
                        name,
                        diagnostics,
                    );
                }
            }
            _ => unsupported_attribute(
                source_id,
                attribute,
                AttributeContext::Event,
                name,
                diagnostics,
            ),
        }
    }
    compiled
}

fn attribute_access(attribute: &Attribute) -> Option<BytecodeAccess> {
    let Some(ExprKind::Name(value)) = attribute.value.as_ref().map(|value| &value.kind) else {
        return None;
    };
    if value.eq_ignore_ascii_case("public") {
        Some(BytecodeAccess::Public)
    } else if value.eq_ignore_ascii_case("protected") {
        Some(BytecodeAccess::Protected)
    } else if value.eq_ignore_ascii_case("private") {
        Some(BytecodeAccess::Private)
    } else {
        None
    }
}

fn unsupported_attribute(
    source_id: SourceId,
    attribute: &Attribute,
    context: AttributeContext,
    name: &str,
    diagnostics: &mut Vec<CompileDiagnostic>,
) {
    let feature = match context {
        AttributeContext::Class => UnsupportedFeature::ClassAttribute(name.to_owned()),
        AttributeContext::Property => UnsupportedFeature::PropertyAttribute(name.to_owned()),
        AttributeContext::Method => UnsupportedFeature::MethodAttribute(name.to_owned()),
        AttributeContext::Enumeration => UnsupportedFeature::EnumerationAttribute(name.to_owned()),
        AttributeContext::Event => UnsupportedFeature::EventAttribute(name.to_owned()),
    };
    diagnostics.push(CompileDiagnostic::new(
        source_id,
        attribute.span,
        CompileDiagnosticKind::Unsupported { feature },
    ));
}

fn is_operator_method(name: &str) -> bool {
    matches!(
        name,
        "plus"
            | "minus"
            | "uminus"
            | "uplus"
            | "mtimes"
            | "times"
            | "mrdivide"
            | "rdivide"
            | "mldivide"
            | "ldivide"
            | "mpower"
            | "power"
            | "eq"
            | "ne"
            | "lt"
            | "le"
            | "gt"
            | "ge"
    )
}

fn is_binary_operator_method(name: &str) -> bool {
    is_operator_method(name) && !matches!(name, "uminus" | "uplus")
}

const fn source_location(source_id: SourceId, span: TextRange) -> SourceLocation {
    SourceLocation::new(source_id.raw(), span.start(), span.end())
}

fn discover_functions<'hir>(
    file: &'hir HirFile,
    diagnostics: &mut Vec<CompileDiagnostic>,
) -> Discovery<'hir> {
    let mut by_name = BTreeMap::new();
    let mut functions = Vec::new();
    for statement in &file.statements {
        let StmtKind::Function(definition) = &statement.kind else {
            continue;
        };
        let Some(name) = definition.name.as_ref() else {
            malformed(
                diagnostics,
                file.source_id,
                definition.span,
                "top-level function is missing its name",
            );
            continue;
        };
        if name.text.is_empty() {
            malformed(
                diagnostics,
                file.source_id,
                name.span,
                "top-level function has an empty name",
            );
            continue;
        }
        if by_name.contains_key(&name.text) {
            diagnostics.push(CompileDiagnostic::new(
                file.source_id,
                name.span,
                CompileDiagnosticKind::DuplicateFunction {
                    name: name.text.clone(),
                },
            ));
            continue;
        }
        let Some(raw_id) = functions
            .len()
            .checked_add(1)
            .and_then(|index| u32::try_from(index).ok())
        else {
            resource_limit(
                diagnostics,
                file.source_id,
                name.span,
                CompilerResource::Functions,
            );
            continue;
        };
        by_name.insert(name.text.clone(), FunctionId::new(raw_id));
        let index = functions.len();
        functions.push(DiscoveredFunction {
            name: name.text.clone(),
            qualified_name: name.text.clone(),
            definition,
            parent: None,
            children: Vec::new(),
        });
        discover_nested_functions(
            file.source_id,
            definition,
            index,
            &mut functions,
            diagnostics,
        );
    }
    Discovery { by_name, functions }
}

fn discover_nested_functions<'hir>(
    source_id: SourceId,
    parent_definition: &'hir FunctionDef,
    parent: usize,
    functions: &mut Vec<DiscoveredFunction<'hir>>,
    diagnostics: &mut Vec<CompileDiagnostic>,
) {
    let mut names = BTreeSet::new();
    for statement in &parent_definition.body {
        let StmtKind::Function(definition) = &statement.kind else {
            continue;
        };
        let Some(name) = definition.name.as_ref() else {
            malformed(
                diagnostics,
                source_id,
                definition.span,
                "nested function is missing its name",
            );
            continue;
        };
        if name.text.is_empty() {
            malformed(
                diagnostics,
                source_id,
                name.span,
                "nested function has an empty name",
            );
            continue;
        }
        if !names.insert(name.text.clone()) {
            diagnostics.push(CompileDiagnostic::new(
                source_id,
                name.span,
                CompileDiagnosticKind::DuplicateFunction {
                    name: name.text.clone(),
                },
            ));
            continue;
        }
        let Some(raw_id) = functions
            .len()
            .checked_add(1)
            .and_then(|index| u32::try_from(index).ok())
        else {
            resource_limit(
                diagnostics,
                source_id,
                name.span,
                CompilerResource::Functions,
            );
            continue;
        };
        let qualified_name = format!("{}>{}", functions[parent].qualified_name, name.text);
        let child = functions.len();
        functions.push(DiscoveredFunction {
            name: name.text.clone(),
            qualified_name,
            definition,
            parent: Some(parent),
            children: Vec::new(),
        });
        functions[parent].children.push(child);
        let _ = raw_id;
        discover_nested_functions(source_id, definition, child, functions, diagnostics);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BindingStorage {
    Local(LocalSlot),
    Captured,
    WorkspaceGlobal { declared: bool },
    Persistent(PersistentSlot),
}

impl BindingStorage {
    const fn class(self) -> BindingStorageClass {
        match self {
            Self::Local(_) | Self::Captured => BindingStorageClass::Local,
            Self::WorkspaceGlobal { .. } => BindingStorageClass::WorkspaceGlobal,
            Self::Persistent(_) => BindingStorageClass::Persistent,
        }
    }
}

struct EntryBindingAnalysis {
    bindings: BTreeMap<String, BindingStorage>,
}

fn analyse_entry_bindings(
    file: &HirFile,
    diagnostics: &mut Vec<CompileDiagnostic>,
) -> EntryBindingAnalysis {
    let empty_names = BTreeSet::new();
    let mut analysis = analyse_declarations(
        file.source_id,
        &file.statements,
        DeclarationContext::Script,
        &empty_names,
        &empty_names,
        diagnostics,
    );
    collect_assigned_names(&file.statements, &mut |name| {
        if !name.is_empty() && name != "~" {
            analysis
                .bindings
                .entry(name.to_owned())
                .or_insert(BindingStorage::WorkspaceGlobal { declared: false });
        }
    });
    EntryBindingAnalysis {
        bindings: analysis.bindings,
    }
}

fn collect_assigned_names(statements: &[Stmt], add: &mut impl FnMut(&str)) {
    for statement in statements {
        match &statement.kind {
            StmtKind::Assignment { target, .. } => collect_target_names(target, add),
            StmtKind::Expr(_) | StmtKind::Command(_) => add("ans"),
            StmtKind::If {
                branches,
                else_body,
            } => {
                for branch in branches {
                    collect_assigned_names(&branch.body, add);
                }
                collect_assigned_names(else_body, add);
            }
            StmtKind::For { variable, body, .. } => {
                collect_target_names(variable, add);
                collect_assigned_names(body, add);
            }
            StmtKind::While { body, .. } => collect_assigned_names(body, add),
            StmtKind::Try(statement) => {
                collect_assigned_names(&statement.body, add);
                if let Some(catch) = &statement.catch {
                    if let Some(variable) = &catch.variable {
                        add(&variable.text);
                    }
                    collect_assigned_names(&catch.body, add);
                }
            }
            StmtKind::Switch {
                cases, otherwise, ..
            } => {
                for case in cases {
                    collect_assigned_names(&case.body, add);
                }
                if let Some(otherwise) = otherwise {
                    collect_assigned_names(&otherwise.body, add);
                }
            }
            _ => {}
        }
    }
}

fn collect_target_names(target: &Expr, add: &mut impl FnMut(&str)) {
    match &target.kind {
        ExprKind::Name(name) => add(name),
        ExprKind::Matrix(rows) => {
            for element in rows.iter().flatten() {
                collect_target_names(element, add);
            }
        }
        ExprKind::ParenApply { target, .. }
        | ExprKind::BraceApply { target, .. }
        | ExprKind::Field { target, .. }
        | ExprKind::DynamicField { target, .. }
        | ExprKind::Paren(target) => collect_target_names(target, add),
        _ => {}
    }
}

struct DeclarationAnalysis {
    bindings: BTreeMap<String, BindingStorage>,
    persistent_slot_count: u32,
}

#[derive(Clone, Copy)]
enum BindingActivity {
    Used,
    Assigned,
}

struct DeclarationCollector<'a> {
    source_id: SourceId,
    expected_context: DeclarationContext,
    inputs: &'a BTreeSet<String>,
    outputs: &'a BTreeSet<String>,
    diagnostics: &'a mut Vec<CompileDiagnostic>,
    bindings: BTreeMap<String, BindingStorage>,
    persistent_declarations: BTreeSet<String>,
    activity: BTreeMap<String, BindingActivity>,
    next_persistent_slot: u32,
}

fn analyse_declarations(
    source_id: SourceId,
    statements: &[Stmt],
    expected_context: DeclarationContext,
    inputs: &BTreeSet<String>,
    outputs: &BTreeSet<String>,
    diagnostics: &mut Vec<CompileDiagnostic>,
) -> DeclarationAnalysis {
    let mut collector = DeclarationCollector {
        source_id,
        expected_context,
        inputs,
        outputs,
        diagnostics,
        bindings: BTreeMap::new(),
        persistent_declarations: BTreeSet::new(),
        activity: BTreeMap::new(),
        next_persistent_slot: 0,
    };
    collector.scan_statements(statements);
    DeclarationAnalysis {
        bindings: collector.bindings,
        persistent_slot_count: collector.next_persistent_slot,
    }
}

impl DeclarationCollector<'_> {
    fn scan_statements(&mut self, statements: &[Stmt]) {
        for statement in statements {
            self.scan_statement(statement);
        }
    }

    fn scan_statement(&mut self, statement: &Stmt) {
        match &statement.kind {
            StmtKind::Arguments(block) => {
                for declaration in &block.declarations {
                    for expression in declaration
                        .dimensions
                        .iter()
                        .chain(&declaration.validators)
                        .chain(declaration.default.iter())
                    {
                        self.scan_expression(expression, &BTreeSet::new());
                    }
                }
            }
            StmtKind::Assignment { target, value } => {
                self.scan_expression(value, &BTreeSet::new());
                self.scan_assignment_target(target);
            }
            StmtKind::Expr(expression) => {
                self.scan_expression(expression, &BTreeSet::new());
                self.record_assigned("ans");
            }
            StmtKind::Command(command) => {
                if let Some(callee) = &command.callee {
                    self.record_used(&callee.text);
                }
                self.record_assigned("ans");
            }
            StmtKind::Clear(clear) => {
                for name in &clear.names {
                    self.record_used(&name.text);
                }
            }
            StmtKind::Declaration(declaration) => {
                self.scan_declaration(declaration, statement.span);
            }
            StmtKind::If {
                branches,
                else_body,
            } => {
                for branch in branches {
                    self.scan_expression(&branch.condition, &BTreeSet::new());
                    self.scan_statements(&branch.body);
                }
                self.scan_statements(else_body);
            }
            StmtKind::For {
                variable,
                iterable,
                body,
            } => {
                self.scan_expression(iterable, &BTreeSet::new());
                self.scan_assignment_target(variable);
                self.scan_statements(body);
            }
            StmtKind::While { condition, body } => {
                self.scan_expression(condition, &BTreeSet::new());
                self.scan_statements(body);
            }
            StmtKind::Try(statement) => {
                self.scan_statements(&statement.body);
                if let Some(catch) = &statement.catch {
                    if let Some(variable) = &catch.variable {
                        self.record_assigned(&variable.text);
                    }
                    self.scan_statements(&catch.body);
                }
            }
            StmtKind::Switch {
                selector,
                cases,
                otherwise,
            } => {
                self.scan_expression(selector, &BTreeSet::new());
                for case in cases {
                    self.scan_expression(&case.expression, &BTreeSet::new());
                    self.scan_statements(&case.body);
                }
                if let Some(otherwise) = otherwise {
                    self.scan_statements(&otherwise.body);
                }
            }
            _ => {}
        }
    }

    fn scan_declaration(&mut self, declaration: &DeclarationStatement, span: TextRange) {
        if declaration.form != DeclarationForm::IdentifierList {
            self.invalid_declaration(
                declaration.kind,
                None,
                span,
                BindingDeclarationProblem::InvalidForm(declaration.form),
            );
            return;
        }
        if declaration.names.is_empty() {
            self.invalid_declaration(
                declaration.kind,
                None,
                span,
                BindingDeclarationProblem::InvalidForm(DeclarationForm::MissingNames),
            );
            return;
        }
        let valid_context = declaration.context == self.expected_context
            && match declaration.kind {
                DeclarationKind::Global => matches!(
                    declaration.context,
                    DeclarationContext::Script | DeclarationContext::Function
                ),
                DeclarationKind::Persistent => declaration.context == DeclarationContext::Function,
            };
        if !valid_context {
            self.invalid_declaration(
                declaration.kind,
                None,
                span,
                BindingDeclarationProblem::InvalidContext(declaration.context),
            );
            return;
        }

        for name in &declaration.names {
            if name.text.is_empty() || name.text == "~" {
                self.invalid_declaration(
                    declaration.kind,
                    Some(name.text.clone()),
                    name.span,
                    BindingDeclarationProblem::EmptyName,
                );
                continue;
            }
            match declaration.kind {
                DeclarationKind::Global => {
                    if matches!(
                        self.bindings.get(&name.text),
                        Some(BindingStorage::Persistent(_))
                    ) {
                        self.invalid_declaration(
                            declaration.kind,
                            Some(name.text.clone()),
                            name.span,
                            BindingDeclarationProblem::ConflictsWithPersistent,
                        );
                    } else {
                        self.bindings.insert(
                            name.text.clone(),
                            BindingStorage::WorkspaceGlobal { declared: true },
                        );
                    }
                }
                DeclarationKind::Persistent => self.scan_persistent_name(name),
            }
        }
    }

    fn scan_persistent_name(&mut self, name: &openmat_hir::Name) {
        if !self.persistent_declarations.insert(name.text.clone()) {
            self.invalid_declaration(
                DeclarationKind::Persistent,
                Some(name.text.clone()),
                name.span,
                BindingDeclarationProblem::DuplicatePersistent,
            );
            return;
        }
        let conflicts_with_global = matches!(
            self.bindings.get(&name.text),
            Some(BindingStorage::WorkspaceGlobal { .. })
        );
        if conflicts_with_global {
            self.invalid_declaration(
                DeclarationKind::Persistent,
                Some(name.text.clone()),
                name.span,
                BindingDeclarationProblem::ConflictsWithGlobal,
            );
        }
        if self.inputs.contains(&name.text) {
            self.invalid_declaration(
                DeclarationKind::Persistent,
                Some(name.text.clone()),
                name.span,
                BindingDeclarationProblem::ConflictsWithInput,
            );
        }
        if self.outputs.contains(&name.text) {
            self.invalid_declaration(
                DeclarationKind::Persistent,
                Some(name.text.clone()),
                name.span,
                BindingDeclarationProblem::ConflictsWithOutput,
            );
        }
        if let Some(activity) = self.activity.get(&name.text).copied() {
            self.invalid_declaration(
                DeclarationKind::Persistent,
                Some(name.text.clone()),
                name.span,
                match activity {
                    BindingActivity::Assigned => {
                        BindingDeclarationProblem::AssignedBeforeDeclaration
                    }
                    BindingActivity::Used => BindingDeclarationProblem::UsedBeforeDeclaration,
                },
            );
        }
        if conflicts_with_global {
            return;
        }
        let slot = PersistentSlot::new(self.next_persistent_slot);
        let Some(next) = self.next_persistent_slot.checked_add(1) else {
            resource_limit(
                self.diagnostics,
                self.source_id,
                name.span,
                CompilerResource::PersistentSlots,
            );
            return;
        };
        self.next_persistent_slot = next;
        self.bindings
            .insert(name.text.clone(), BindingStorage::Persistent(slot));
    }

    fn scan_assignment_target(&mut self, target: &Expr) {
        match &target.kind {
            ExprKind::Name(name) => self.record_assigned(name),
            ExprKind::Matrix(rows) => {
                for element in rows.iter().flatten() {
                    self.scan_assignment_target(element);
                }
            }
            ExprKind::Paren(inner) => self.scan_assignment_target(inner),
            ExprKind::ParenApply { target, arguments }
            | ExprKind::BraceApply { target, arguments } => {
                self.scan_assignment_target(target);
                for argument in arguments {
                    self.scan_expression(argument, &BTreeSet::new());
                }
            }
            ExprKind::Field { target, .. } => self.scan_assignment_target(target),
            ExprKind::DynamicField { target, name } => {
                self.scan_assignment_target(target);
                self.scan_expression(name, &BTreeSet::new());
            }
            _ => self.scan_expression(target, &BTreeSet::new()),
        }
    }

    fn scan_expression(&mut self, expression: &Expr, bound: &BTreeSet<String>) {
        match &expression.kind {
            ExprKind::Name(name) => {
                if !bound.contains(name) {
                    self.record_used(name);
                }
            }
            ExprKind::AnonymousFunction { parameters, body } => {
                let mut nested_bound = bound.clone();
                nested_bound.extend(
                    parameters
                        .iter()
                        .filter(|parameter| !parameter.text.is_empty() && parameter.text != "~")
                        .map(|parameter| parameter.text.clone()),
                );
                self.scan_expression(body, &nested_bound);
            }
            ExprKind::Paren(inner)
            | ExprKind::Unary { operand: inner, .. }
            | ExprKind::Transpose { operand: inner, .. } => {
                self.scan_expression(inner, bound);
            }
            ExprKind::ParenApply { target, arguments }
            | ExprKind::BraceApply { target, arguments } => {
                self.scan_expression(target, bound);
                for argument in arguments {
                    self.scan_expression(argument, bound);
                }
            }
            ExprKind::SuperclassConstructorCall {
                object, arguments, ..
            } => {
                self.scan_expression(object, bound);
                for argument in arguments {
                    self.scan_expression(argument, bound);
                }
            }
            ExprKind::Field { target, .. } => self.scan_expression(target, bound),
            ExprKind::DynamicField { target, name } => {
                self.scan_expression(target, bound);
                self.scan_expression(name, bound);
            }
            ExprKind::Binary { left, right, .. } => {
                self.scan_expression(left, bound);
                self.scan_expression(right, bound);
            }
            ExprKind::Range { start, step, end } => {
                self.scan_expression(start, bound);
                if let Some(step) = step {
                    self.scan_expression(step, bound);
                }
                self.scan_expression(end, bound);
            }
            ExprKind::Matrix(rows) | ExprKind::Cell(rows) => {
                for element in rows.iter().flatten() {
                    self.scan_expression(element, bound);
                }
            }
            _ => {}
        }
    }

    fn record_used(&mut self, name: &str) {
        if !name.is_empty() && name != "~" {
            self.activity
                .entry(name.to_owned())
                .or_insert(BindingActivity::Used);
        }
    }

    fn record_assigned(&mut self, name: &str) {
        if !name.is_empty() && name != "~" {
            self.activity
                .insert(name.to_owned(), BindingActivity::Assigned);
        }
    }

    fn invalid_declaration(
        &mut self,
        kind: DeclarationKind,
        name: Option<String>,
        range: TextRange,
        problem: BindingDeclarationProblem,
    ) {
        self.diagnostics.push(CompileDiagnostic::new(
            self.source_id,
            range,
            CompileDiagnosticKind::InvalidBindingDeclaration {
                kind,
                name,
                problem,
            },
        ));
    }
}

struct LocalAnalysis {
    bindings: BTreeMap<String, BindingStorage>,
    output_bindings: Vec<OutputBinding>,
    output_abi_slots: Vec<Option<LocalSlot>>,
    variadic_input_slot: Option<LocalSlot>,
    shared_captures: BTreeSet<String>,
    nested_function_slots: BTreeMap<usize, LocalSlot>,
    local_count: u32,
    parameter_count: u32,
    persistent_slot_count: u32,
}

#[derive(Clone)]
struct OutputBinding {
    name: String,
    storage: BindingStorage,
}

fn analyse_anonymous_parameters(
    source_id: SourceId,
    parameters: &[openmat_hir::Name],
    span: TextRange,
    diagnostics: &mut Vec<CompileDiagnostic>,
) -> LocalAnalysis {
    // MATLAB treats a final parameter named `varargin` as the variadic tail of
    // an anonymous function. The same name in any earlier position is an
    // ordinary fixed parameter.
    let variadic_input_index = parameters
        .len()
        .checked_sub(1)
        .filter(|index| parameters[*index].text == "varargin");
    let fixed_parameter_len = parameters
        .len()
        .saturating_sub(usize::from(variadic_input_index.is_some()));
    let parameter_count = if let Ok(count) = u32::try_from(fixed_parameter_len) {
        count
    } else {
        resource_limit(diagnostics, source_id, span, CompilerResource::Parameters);
        u32::MAX
    };
    let mut bindings = BTreeMap::new();
    let mut next_local = 0_u32;
    let mut names = BTreeSet::new();
    let mut variadic_input_slot = None;
    for (index, parameter) in parameters.iter().enumerate() {
        let Some(slot) = allocate_local(&mut next_local, diagnostics, source_id, parameter.span)
        else {
            continue;
        };
        if parameter.text.is_empty() {
            malformed(
                diagnostics,
                source_id,
                parameter.span,
                "anonymous-function parameter has an empty name",
            );
        } else if parameter.text != "~" {
            if variadic_input_index == Some(index) {
                variadic_input_slot = Some(slot);
            }
            if names.insert(parameter.text.clone()) {
                bindings.insert(parameter.text.clone(), BindingStorage::Local(slot));
            } else {
                diagnostics.push(CompileDiagnostic::new(
                    source_id,
                    parameter.span,
                    CompileDiagnosticKind::DuplicateParameter {
                        name: parameter.text.clone(),
                    },
                ));
            }
        }
    }
    LocalAnalysis {
        bindings,
        output_bindings: Vec::new(),
        output_abi_slots: Vec::new(),
        variadic_input_slot,
        shared_captures: BTreeSet::new(),
        nested_function_slots: BTreeMap::new(),
        local_count: next_local,
        parameter_count,
        persistent_slot_count: 0,
    }
}

#[derive(Clone)]
struct AnonymousFreeName {
    name: String,
    tombstone_if_missing: bool,
}

fn anonymous_free_names(parameters: &[openmat_hir::Name], body: &Expr) -> Vec<AnonymousFreeName> {
    let mut bound = parameters
        .iter()
        .filter(|parameter| !parameter.text.is_empty() && parameter.text != "~")
        .map(|parameter| parameter.text.clone())
        .collect::<BTreeSet<_>>();
    let mut seen = BTreeMap::new();
    let mut names = Vec::new();
    collect_free_names(body, &mut bound, &mut seen, &mut names);
    names
}

#[allow(clippy::too_many_lines)]
fn collect_free_names(
    expression: &Expr,
    bound: &mut BTreeSet<String>,
    seen: &mut BTreeMap<String, usize>,
    names: &mut Vec<AnonymousFreeName>,
) {
    match &expression.kind {
        ExprKind::Name(name) => {
            add_free_name(name, true, bound, seen, names);
        }
        ExprKind::AnonymousFunction { parameters, body } => {
            let added = parameters
                .iter()
                .filter(|parameter| !parameter.text.is_empty() && parameter.text != "~")
                .filter_map(|parameter| {
                    bound
                        .insert(parameter.text.clone())
                        .then_some(parameter.text.clone())
                })
                .collect::<Vec<_>>();
            collect_free_names(body, bound, seen, names);
            for parameter in added {
                bound.remove(&parameter);
            }
        }
        ExprKind::Paren(inner) => collect_free_names(inner, bound, seen, names),
        ExprKind::ParenApply { target, arguments } => {
            if let ExprKind::Name(name) = &target.kind {
                add_free_name(name, false, bound, seen, names);
            } else {
                collect_free_names(target, bound, seen, names);
            }
            for argument in arguments {
                collect_free_names(argument, bound, seen, names);
            }
        }
        ExprKind::BraceApply { target, arguments } => {
            collect_free_names(target, bound, seen, names);
            for argument in arguments {
                collect_free_names(argument, bound, seen, names);
            }
        }
        ExprKind::SuperclassConstructorCall {
            object, arguments, ..
        } => {
            collect_free_names(object, bound, seen, names);
            for argument in arguments {
                collect_free_names(argument, bound, seen, names);
            }
        }
        ExprKind::Field { target, .. } => collect_free_names(target, bound, seen, names),
        ExprKind::DynamicField { target, name } => {
            collect_free_names(target, bound, seen, names);
            collect_free_names(name, bound, seen, names);
        }
        ExprKind::Unary { operand, .. } | ExprKind::Transpose { operand, .. } => {
            collect_free_names(operand, bound, seen, names);
        }
        ExprKind::Binary { left, right, .. } => {
            collect_free_names(left, bound, seen, names);
            collect_free_names(right, bound, seen, names);
        }
        ExprKind::Range { start, step, end } => {
            collect_free_names(start, bound, seen, names);
            if let Some(step) = step {
                collect_free_names(step, bound, seen, names);
            }
            collect_free_names(end, bound, seen, names);
        }
        ExprKind::Matrix(rows) | ExprKind::Cell(rows) => {
            for element in rows.iter().flatten() {
                collect_free_names(element, bound, seen, names);
            }
        }
        _ => {}
    }
}

fn add_free_name(
    name: &str,
    tombstone_if_missing: bool,
    bound: &BTreeSet<String>,
    seen: &mut BTreeMap<String, usize>,
    names: &mut Vec<AnonymousFreeName>,
) {
    if name.is_empty() || name == "~" || bound.contains(name) {
        return;
    }
    if let Some(index) = seen.get(name).copied() {
        if tombstone_if_missing {
            names[index].tombstone_if_missing = true;
        }
        return;
    }
    seen.insert(name.to_owned(), names.len());
    names.push(AnonymousFreeName {
        name: name.to_owned(),
        tombstone_if_missing,
    });
}

#[allow(clippy::too_many_lines)]
fn analyse_locals(
    source_id: SourceId,
    function: &FunctionDef,
    diagnostics: &mut Vec<CompileDiagnostic>,
) -> LocalAnalysis {
    let variadic_input_index = function
        .inputs
        .iter()
        .position(|input| input.text == "varargin");
    if let Some(index) = variadic_input_index
        && index + 1 != function.inputs.len()
    {
        malformed(
            diagnostics,
            source_id,
            function.inputs[index].span,
            "varargin must be the final function input",
        );
    }
    let fixed_parameter_len = function.inputs.len().saturating_sub(usize::from(
        variadic_input_index == function.inputs.len().checked_sub(1),
    ));
    let parameter_count = if let Ok(count) = u32::try_from(fixed_parameter_len) {
        count
    } else {
        resource_limit(
            diagnostics,
            source_id,
            function.span,
            CompilerResource::Parameters,
        );
        u32::MAX
    };
    let mut input_slots = BTreeMap::new();
    let mut next_local = 0_u32;
    let mut input_names = BTreeSet::new();
    let mut variadic_input_slot = None;
    for input in &function.inputs {
        let Some(slot) = allocate_local(&mut next_local, diagnostics, source_id, input.span) else {
            continue;
        };
        if input.text.is_empty() {
            malformed(
                diagnostics,
                source_id,
                input.span,
                "function parameter has an empty name",
            );
        } else if input.text != "~" {
            if input.text == "varargin"
                && variadic_input_index == function.inputs.len().checked_sub(1)
            {
                variadic_input_slot = Some(slot);
            }
            if input_names.insert(input.text.clone()) {
                input_slots.insert(input.text.clone(), slot);
            } else {
                diagnostics.push(CompileDiagnostic::new(
                    source_id,
                    input.span,
                    CompileDiagnosticKind::DuplicateParameter {
                        name: input.text.clone(),
                    },
                ));
            }
        }
    }

    let mut output_names = BTreeSet::new();
    for (index, output) in function.outputs.iter().enumerate() {
        if output.text.is_empty() || output.text == "~" {
            malformed(
                diagnostics,
                source_id,
                output.span,
                "function output is missing a usable name",
            );
            continue;
        }
        if !output_names.insert(output.text.clone()) {
            diagnostics.push(CompileDiagnostic::new(
                source_id,
                output.span,
                CompileDiagnosticKind::DuplicateOutput {
                    name: output.text.clone(),
                },
            ));
        }
        if output.text == "varargout" && index + 1 != function.outputs.len() {
            malformed(
                diagnostics,
                source_id,
                output.span,
                "varargout must be the final function output",
            );
        }
    }

    let declarations = analyse_declarations(
        source_id,
        &function.body,
        DeclarationContext::Function,
        &input_names,
        &output_names,
        diagnostics,
    );
    let mut bindings = input_slots
        .iter()
        .map(|(name, slot)| (name.clone(), BindingStorage::Local(*slot)))
        .collect::<BTreeMap<_, _>>();
    bindings.extend(declarations.bindings);

    let mut output_bindings = Vec::with_capacity(function.outputs.len());
    let mut output_abi_slots = Vec::with_capacity(function.outputs.len());
    for output in &function.outputs {
        if output.text.is_empty() || output.text == "~" {
            if let Some(slot) = allocate_local(&mut next_local, diagnostics, source_id, output.span)
            {
                output_bindings.push(OutputBinding {
                    name: output.text.clone(),
                    storage: BindingStorage::Local(slot),
                });
                output_abi_slots.push(Some(slot));
            }
            continue;
        }
        if let Some(storage) = bindings.get(&output.text).copied() {
            output_bindings.push(OutputBinding {
                name: output.text.clone(),
                storage,
            });
            output_abi_slots.push(input_slots.get(&output.text).copied());
            continue;
        }
        let Some(slot) = allocate_local(&mut next_local, diagnostics, source_id, output.span)
        else {
            continue;
        };
        let storage = BindingStorage::Local(slot);
        bindings.insert(output.text.clone(), storage);
        output_bindings.push(OutputBinding {
            name: output.text.clone(),
            storage,
        });
        output_abi_slots.push(Some(slot));
    }

    collect_assigned_names(&function.body, &mut |name| {
        if name.is_empty() || name == "~" || bindings.contains_key(name) {
            return;
        }
        if let Some(slot) = allocate_local(&mut next_local, diagnostics, source_id, function.span) {
            bindings.insert(name.to_owned(), BindingStorage::Local(slot));
        }
    });

    LocalAnalysis {
        bindings,
        output_bindings,
        output_abi_slots,
        variadic_input_slot,
        shared_captures: BTreeSet::new(),
        nested_function_slots: BTreeMap::new(),
        local_count: next_local,
        parameter_count,
        persistent_slot_count: declarations.persistent_slot_count,
    }
}

fn analyse_discovered_functions(
    source_id: SourceId,
    discovery: &Discovery<'_>,
    diagnostics: &mut Vec<CompileDiagnostic>,
) -> Vec<LocalAnalysis> {
    let mut completed = Vec::with_capacity(discovery.functions.len());
    for discovered in &discovery.functions {
        let mut current = analyse_locals(source_id, discovered.definition, diagnostics);
        if let Some(parent) = discovered.parent {
            let outer_bindings = completed
                .get(parent)
                .map(|analysis: &LocalAnalysis| analysis.bindings.clone())
                .unwrap_or_default();
            let shadowed = discovered
                .definition
                .inputs
                .iter()
                .chain(&discovered.definition.outputs)
                .map(|name| name.text.clone())
                .collect::<BTreeSet<_>>();
            let mut lexical_names = BTreeSet::new();
            collect_lexical_names(
                &discovered.definition.body,
                &BTreeSet::new(),
                &mut lexical_names,
            );
            for name in lexical_names {
                if name == "ans" || shadowed.contains(&name) {
                    continue;
                }
                if !matches!(
                    outer_bindings.get(&name),
                    Some(BindingStorage::Local(_) | BindingStorage::Captured)
                ) {
                    continue;
                }
                match current.bindings.get(&name) {
                    Some(
                        BindingStorage::WorkspaceGlobal { .. } | BindingStorage::Persistent(_),
                    ) => {
                        continue;
                    }
                    Some(BindingStorage::Local(_) | BindingStorage::Captured) | None => {}
                }
                current
                    .bindings
                    .insert(name.clone(), BindingStorage::Captured);
                current.shared_captures.insert(name);
            }
        }

        for child in &discovered.children {
            let name = &discovery.functions[*child].name;
            let slot = match current.bindings.get(name).copied() {
                Some(BindingStorage::Local(slot)) => Some(slot),
                Some(_) => {
                    malformed(
                        diagnostics,
                        source_id,
                        discovery.functions[*child].definition.span,
                        "nested function name conflicts with a non-local binding",
                    );
                    None
                }
                None => {
                    let slot = allocate_local(
                        &mut current.local_count,
                        diagnostics,
                        source_id,
                        discovery.functions[*child].definition.span,
                    );
                    if let Some(slot) = slot {
                        current
                            .bindings
                            .insert(name.clone(), BindingStorage::Local(slot));
                    }
                    slot
                }
            };
            if let Some(slot) = slot {
                current.nested_function_slots.insert(*child, slot);
            }
        }
        completed.push(current);
    }
    completed
}

fn collect_lexical_names(
    statements: &[Stmt],
    bound: &BTreeSet<String>,
    names: &mut BTreeSet<String>,
) {
    for statement in statements {
        match &statement.kind {
            StmtKind::Arguments(block) => {
                for declaration in &block.declarations {
                    for expression in declaration
                        .dimensions
                        .iter()
                        .chain(&declaration.validators)
                        .chain(declaration.default.iter())
                    {
                        collect_expression_lexical_names(expression, bound, names);
                    }
                }
            }
            StmtKind::Assignment { target, value } => {
                collect_expression_lexical_names(target, bound, names);
                collect_expression_lexical_names(value, bound, names);
            }
            StmtKind::Expr(expression) => {
                collect_expression_lexical_names(expression, bound, names);
            }
            StmtKind::Command(command) => {
                if let Some(callee) = &command.callee
                    && callee.text != "import"
                    && !bound.contains(&callee.text)
                {
                    names.insert(callee.text.clone());
                }
            }
            StmtKind::If {
                branches,
                else_body,
            } => {
                for branch in branches {
                    collect_expression_lexical_names(&branch.condition, bound, names);
                    collect_lexical_names(&branch.body, bound, names);
                }
                collect_lexical_names(else_body, bound, names);
            }
            StmtKind::While { condition, body } => {
                collect_expression_lexical_names(condition, bound, names);
                collect_lexical_names(body, bound, names);
            }
            StmtKind::Try(statement) => {
                collect_lexical_names(&statement.body, bound, names);
                if let Some(catch) = &statement.catch {
                    collect_lexical_names(&catch.body, bound, names);
                }
            }
            StmtKind::For {
                variable,
                iterable,
                body,
            } => {
                collect_expression_lexical_names(variable, bound, names);
                collect_expression_lexical_names(iterable, bound, names);
                collect_lexical_names(body, bound, names);
            }
            StmtKind::Switch {
                selector,
                cases,
                otherwise,
            } => {
                collect_expression_lexical_names(selector, bound, names);
                for case in cases {
                    collect_expression_lexical_names(&case.expression, bound, names);
                    collect_lexical_names(&case.body, bound, names);
                }
                if let Some(otherwise) = otherwise {
                    collect_lexical_names(&otherwise.body, bound, names);
                }
            }
            _ => {}
        }
    }
}

fn collect_expression_lexical_names(
    expression: &Expr,
    bound: &BTreeSet<String>,
    names: &mut BTreeSet<String>,
) {
    match &expression.kind {
        ExprKind::Name(name) => {
            if !bound.contains(name) {
                names.insert(name.clone());
            }
        }
        ExprKind::FunctionHandle(Some(name)) => {
            if !bound.contains(&name.text) {
                names.insert(name.text.clone());
            }
        }
        ExprKind::AnonymousFunction { parameters, body } => {
            let mut nested_bound = bound.clone();
            nested_bound.extend(parameters.iter().map(|parameter| parameter.text.clone()));
            collect_expression_lexical_names(body, &nested_bound, names);
        }
        ExprKind::Paren(inner)
        | ExprKind::Unary { operand: inner, .. }
        | ExprKind::Transpose { operand: inner, .. } => {
            collect_expression_lexical_names(inner, bound, names);
        }
        ExprKind::ParenApply { target, arguments } | ExprKind::BraceApply { target, arguments } => {
            collect_expression_lexical_names(target, bound, names);
            for argument in arguments {
                collect_expression_lexical_names(argument, bound, names);
            }
        }
        ExprKind::SuperclassConstructorCall {
            object, arguments, ..
        } => {
            collect_expression_lexical_names(object, bound, names);
            for argument in arguments {
                collect_expression_lexical_names(argument, bound, names);
            }
        }
        ExprKind::Field { target, .. } => {
            collect_expression_lexical_names(target, bound, names);
        }
        ExprKind::DynamicField { target, name } => {
            collect_expression_lexical_names(target, bound, names);
            collect_expression_lexical_names(name, bound, names);
        }
        ExprKind::Binary { left, right, .. } => {
            collect_expression_lexical_names(left, bound, names);
            collect_expression_lexical_names(right, bound, names);
        }
        ExprKind::Range { start, step, end } => {
            collect_expression_lexical_names(start, bound, names);
            if let Some(step) = step {
                collect_expression_lexical_names(step, bound, names);
            }
            collect_expression_lexical_names(end, bound, names);
        }
        ExprKind::Matrix(rows) | ExprKind::Cell(rows) => {
            for element in rows.iter().flatten() {
                collect_expression_lexical_names(element, bound, names);
            }
        }
        _ => {}
    }
}

fn collect_scope_imports(statements: &[Stmt]) -> Vec<String> {
    fn collect(statements: &[Stmt], imports: &mut Vec<String>) {
        for statement in statements {
            match &statement.kind {
                StmtKind::Command(command)
                    if command
                        .callee
                        .as_ref()
                        .is_some_and(|callee| callee.text == "import") =>
                {
                    imports.extend(
                        command
                            .arguments
                            .iter()
                            .map(|argument| argument.text.clone()),
                    );
                }
                StmtKind::If {
                    branches,
                    else_body,
                } => {
                    for branch in branches {
                        collect(&branch.body, imports);
                    }
                    collect(else_body, imports);
                }
                StmtKind::While { body, .. } | StmtKind::For { body, .. } => {
                    collect(body, imports);
                }
                StmtKind::Try(statement) => {
                    collect(&statement.body, imports);
                    if let Some(catch) = &statement.catch {
                        collect(&catch.body, imports);
                    }
                }
                StmtKind::Switch {
                    cases, otherwise, ..
                } => {
                    for case in cases {
                        collect(&case.body, imports);
                    }
                    if let Some(otherwise) = otherwise {
                        collect(&otherwise.body, imports);
                    }
                }
                _ => {}
            }
        }
    }

    let mut imports = Vec::new();
    collect(statements, &mut imports);
    imports
}

fn merged_imports(inherited: &[String], declared: &[String]) -> Vec<String> {
    inherited
        .iter()
        .chain(declared)
        .fold(Vec::new(), |mut imports, import| {
            if !imports.contains(import) {
                imports.push(import.clone());
            }
            imports
        })
}

fn valid_import_name(import: &str) -> bool {
    let (base, wildcard) = import
        .strip_suffix(".*")
        .map_or((import, false), |base| (base, true));
    let components = base.split('.').collect::<Vec<_>>();
    components.len() > usize::from(!wildcard)
        && components.iter().all(|component| {
            let mut characters = component.chars();
            characters.next().is_some_and(char::is_alphabetic)
                && characters.all(|character| character == '_' || character.is_alphanumeric())
        })
}

fn allocate_local(
    next_local: &mut u32,
    diagnostics: &mut Vec<CompileDiagnostic>,
    source_id: SourceId,
    range: TextRange,
) -> Option<LocalSlot> {
    let current = *next_local;
    let Some(next) = current.checked_add(1) else {
        resource_limit(diagnostics, source_id, range, CompilerResource::Locals);
        return None;
    };
    *next_local = next;
    Some(LocalSlot::new(current))
}

enum Scope<'symbols> {
    Entry {
        bindings: &'symbols BTreeMap<String, BindingStorage>,
    },
    Function {
        bindings: &'symbols BTreeMap<String, BindingStorage>,
        outputs: &'symbols [OutputBinding],
        captures: &'symbols BTreeSet<String>,
    },
}

struct LoopContext {
    continue_target: InstructionIndex,
    break_patches: Vec<usize>,
}

struct AssignmentName<'hir> {
    text: &'hir str,
    span: TextRange,
}

struct AssignmentBinding<'hir> {
    root: AssignmentName<'hir>,
    target: BindingTarget,
}

#[derive(Clone, Copy)]
enum CatchErrorLocal {
    Plain,
    Direct(LocalSlot),
    Indirect {
        scratch: LocalSlot,
        target: BindingStorage,
    },
}

#[derive(Clone, Copy)]
struct IndexContext {
    target: Register,
    argument_index: u32,
    argument_count: u32,
}

enum PlaceComponent<'hir> {
    Paren(&'hir [Expr], TextRange),
    Brace(&'hir [Expr], TextRange),
    StaticField(&'hir openmat_hir::Name),
    DynamicField(&'hir Expr),
}

#[derive(Clone)]
struct ConstructorContext {
    class_name: String,
    direct_superclass: Option<String>,
    object_name: Option<String>,
    object_slot: Option<LocalSlot>,
}

struct FunctionCompiler<'symbols, 'diagnostics> {
    source_id: SourceId,
    function_span: TextRange,
    function: Function,
    scope: Scope<'symbols>,
    user_functions: &'symbols BTreeMap<String, FunctionId>,
    class_definitions: &'symbols BTreeMap<String, ClassDefinitionId>,
    anonymous_function_base: Option<u32>,
    anonymous_functions: &'diagnostics mut Vec<Function>,
    diagnostics: &'diagnostics mut Vec<CompileDiagnostic>,
    next_register: u32,
    next_pack_register: u32,
    index_contexts: Vec<IndexContext>,
    loops: Vec<LoopContext>,
    constructor_context: Option<ConstructorContext>,
    nested_definitions_lowered: bool,
    imports: Vec<String>,
    argument_blocks: Vec<TextRange>,
    output_validations: Vec<openmat_hir::ArgumentDeclaration>,
    repeating_output: Option<openmat_hir::ArgumentDeclaration>,
    output_return_patches: Vec<usize>,
}

impl<'symbols, 'diagnostics> FunctionCompiler<'symbols, 'diagnostics> {
    #[allow(clippy::too_many_arguments)]
    fn new(
        source_id: SourceId,
        function_span: TextRange,
        name: &str,
        scope: Scope<'symbols>,
        user_functions: &'symbols BTreeMap<String, FunctionId>,
        class_definitions: &'symbols BTreeMap<String, ClassDefinitionId>,
        anonymous_function_base: Option<u32>,
        anonymous_functions: &'diagnostics mut Vec<Function>,
        diagnostics: &'diagnostics mut Vec<CompileDiagnostic>,
        local_count: u32,
        parameter_count: u32,
        persistent_slot_count: u32,
    ) -> Self {
        let mut compiler = Self {
            source_id,
            function_span,
            function: Function::new(name, 0, local_count, parameter_count)
                .with_persistent_slot_count(persistent_slot_count),
            scope,
            user_functions,
            class_definitions,
            anonymous_function_base,
            anonymous_functions,
            diagnostics,
            next_register: 0,
            next_pack_register: 0,
            index_contexts: Vec::new(),
            loops: Vec::new(),
            constructor_context: None,
            nested_definitions_lowered: false,
            imports: Vec::new(),
            argument_blocks: Vec::new(),
            output_validations: Vec::new(),
            repeating_output: None,
            output_return_patches: Vec::new(),
        };
        compiler.emit_named_bindings(function_span);
        compiler
    }

    fn set_imports(&mut self, imports: Vec<String>, span: TextRange) {
        let mut encoded = Vec::new();
        for import in imports {
            if !valid_import_name(&import) {
                self.malformed(span, "import target is not a qualified package name");
                continue;
            }
            if self.imports.contains(&import) {
                continue;
            }
            let Some(constant) = self.add_constant(Constant::String(import.clone()), span) else {
                return;
            };
            self.imports.push(import);
            encoded.push(constant);
        }
        if !encoded.is_empty() {
            self.emit(
                InstructionKind::DeclareImports { imports: encoded },
                self.location(span),
            );
        }
    }

    fn redeclare_imports(&mut self, imports: &[String], span: TextRange) {
        let mut encoded = Vec::new();
        let mut seen = BTreeSet::new();
        for import in imports {
            if !valid_import_name(import) {
                self.malformed(span, "import target is not a qualified package name");
                continue;
            }
            if !seen.insert(import.as_str()) {
                continue;
            }
            let Some(constant) = self.add_constant(Constant::String(import.clone()), span) else {
                return;
            };
            encoded.push(constant);
        }
        if !encoded.is_empty() {
            self.emit(
                InstructionKind::DeclareImports { imports: encoded },
                self.location(span),
            );
        }
    }

    fn emit_named_bindings(&mut self, span: TextRange) {
        let Scope::Function { bindings, .. } = self.scope else {
            return;
        };
        let bindings = bindings
            .iter()
            .map(|(name, storage)| (name.clone(), *storage))
            .collect::<Vec<_>>();
        if bindings.is_empty() {
            return;
        }
        let mut encoded = Vec::with_capacity(bindings.len());
        for (name, storage) in bindings {
            let Some(name) = self.add_constant(Constant::String(name), span) else {
                return;
            };
            let kind = match storage {
                BindingStorage::Local(slot) => NamedBindingKind::Local(slot),
                BindingStorage::Persistent(slot) => NamedBindingKind::Persistent(slot),
                BindingStorage::Captured => NamedBindingKind::Capture,
                BindingStorage::WorkspaceGlobal { .. } => NamedBindingKind::Workspace,
            };
            encoded.push((name, kind));
        }
        self.emit(
            InstructionKind::DeclareNamedBindings { bindings: encoded },
            self.location(span),
        );
    }

    fn finish(mut self) -> Function {
        self.function.register_count = self.next_register;
        self.function.pack_register_count = self.next_pack_register;
        self.function
    }

    fn emit_variadic_inputs(&mut self, slot: LocalSlot, span: TextRange) {
        if self.function.argument_layout.is_some() {
            return;
        }
        let Some(value) = self.allocate_register(span) else {
            return;
        };
        self.emit(
            InstructionKind::LoadVariadicInputs { dst: value },
            self.location(span),
        );
        self.emit(
            InstructionKind::StoreLocal {
                local: slot,
                src: value,
            },
            self.location(span),
        );
    }

    fn emit_nested_closure(
        &mut self,
        slot: LocalSlot,
        function: FunctionId,
        captures: &[(String, SharedCaptureSource)],
        span: TextRange,
    ) {
        let mut encoded = Vec::with_capacity(captures.len());
        for (name, source) in captures {
            let Some(name) = self.add_constant(Constant::String(name.clone()), span) else {
                return;
            };
            encoded.push((name, *source));
        }
        let Some(value) = self.allocate_register(span) else {
            return;
        };
        self.emit(
            InstructionKind::MakeSharedClosure {
                dst: value,
                function,
                captures: encoded,
            },
            self.location(span),
        );
        self.emit(
            InstructionKind::StoreLocal {
                local: slot,
                src: value,
            },
            self.location(span),
        );
    }

    fn lower_statements(&mut self, statements: &[Stmt], top_level: bool) {
        for statement in statements {
            self.lower_statement(statement, top_level);
        }
    }

    #[allow(clippy::too_many_lines)]
    fn lower_statement(&mut self, statement: &Stmt, top_level: bool) {
        match &statement.kind {
            StmtKind::Arguments(block) => {
                if !self.argument_blocks.contains(&statement.span) {
                    self.malformed(
                        statement.span,
                        "arguments blocks must precede the function body",
                    );
                } else if !block.attributes.iter().any(|attribute| {
                    attribute
                        .name
                        .as_ref()
                        .is_some_and(|name| name.text == "Output")
                }) {
                    if block
                        .attributes
                        .iter()
                        .any(|a| a.name.as_ref().is_some_and(|n| n.text == "Repeating"))
                    {
                        self.lower_repeating_validation(&block.declarations, true);
                    } else {
                        for declaration in &block.declarations {
                            self.lower_argument_validation(declaration, true);
                        }
                    }
                    self.order_argument_fields(statement.span);
                }
            }
            StmtKind::Assignment { target, value } => {
                self.lower_assignment(target, value, !statement.suppress_output);
            }
            StmtKind::Expr(expression) => match &expression.kind {
                ExprKind::Name(name) if name == "clearvars" => {
                    self.lower_clearvars_targets(false, &BTreeSet::new(), true, statement.span);
                }
                ExprKind::Name(name) if zero_output_bare_command(name) => {
                    self.lower_statement_apply(
                        expression,
                        &[],
                        !statement.suppress_output,
                        expression.span,
                    );
                }
                ExprKind::ParenApply { target, arguments } => {
                    self.lower_statement_apply(
                        target,
                        arguments,
                        !statement.suppress_output,
                        expression.span,
                    );
                }
                ExprKind::SuperclassConstructorCall {
                    object,
                    superclass,
                    arguments,
                } => self.lower_superclass_constructor_call(
                    object,
                    superclass.as_ref(),
                    arguments,
                    expression.span,
                ),
                _ if is_pack_expression(expression) => {
                    if let Some(pack) = self.lower_pack_expression(expression)
                        && let Some(result) = self.statement_result_target(expression.span)
                    {
                        self.emit(
                            InstructionKind::StatementPack {
                                pack,
                                result,
                                display: !statement.suppress_output,
                            },
                            self.location(expression.span),
                        );
                    }
                }
                _ => {
                    if let Some(register) = self.lower_expression(expression)
                        && let Some(result) = self.statement_result_target(expression.span)
                    {
                        self.emit(
                            InstructionKind::StatementValue {
                                src: register,
                                result,
                                display: !statement.suppress_output,
                            },
                            self.location(expression.span),
                        );
                    }
                }
            },
            StmtKind::Command(command) => {
                self.lower_command_statement(command, !statement.suppress_output, statement.span);
            }
            StmtKind::Clear(clear) => self.lower_clear(clear, statement.span),
            StmtKind::Declaration(declaration) => {
                self.lower_declaration(declaration, statement.span);
            }
            StmtKind::If {
                branches,
                else_body,
            } => self.lower_if(branches, else_body, statement.span),
            StmtKind::While { condition, body } => {
                self.lower_while(condition, body, statement.span);
            }
            StmtKind::Try(try_statement) => self.lower_try(try_statement, statement.span),
            StmtKind::For {
                variable,
                iterable,
                body,
            } => self.lower_for(variable, iterable, body, statement.span),
            StmtKind::Switch {
                selector,
                cases,
                otherwise,
            } => self.lower_switch(selector, cases, otherwise.as_ref(), statement.span),
            StmtKind::Break => self.lower_break(statement.span),
            StmtKind::Continue => self.lower_continue(statement.span),
            StmtKind::Return => {
                if self.output_validations.is_empty() && self.repeating_output.is_none() {
                    self.emit_return(statement.span);
                } else if let Some(patch) = self.emit_jump(statement.span) {
                    self.output_return_patches.push(patch);
                }
            }
            StmtKind::Function(_) if matches!(self.scope, Scope::Entry { .. }) && top_level => {}
            StmtKind::Function(_) if self.nested_definitions_lowered => {}
            StmtKind::Function(_) => {
                self.unsupported(statement.span, UnsupportedFeature::NestedFunction);
            }
            StmtKind::Class(class) if matches!(self.scope, Scope::Entry { .. }) && top_level => {
                let Some(name) = class.name.as_ref() else {
                    self.malformed(statement.span, "class definition is missing its name");
                    return;
                };
                let Some(class) = self.class_definitions.get(&name.text).copied() else {
                    self.malformed(name.span, "class definition was not assigned a bytecode id");
                    return;
                };
                self.emit(
                    InstructionKind::RegisterClass { class },
                    self.location(statement.span),
                );
            }
            StmtKind::Class(_) => {
                self.unsupported(statement.span, UnsupportedFeature::ClassDefinition);
            }
            StmtKind::Error => {
                self.malformed(statement.span, "statement is an explicit error node");
            }
            _ => self.malformed(statement.span, "unknown statement kind"),
        }
    }

    fn lower_declaration(&mut self, declaration: &DeclarationStatement, span: TextRange) {
        if declaration.form != DeclarationForm::IdentifierList || declaration.names.is_empty() {
            return;
        }
        let expected_context = match self.scope {
            Scope::Entry { .. } => DeclarationContext::Script,
            Scope::Function { .. } => DeclarationContext::Function,
        };
        let valid_context = declaration.context == expected_context
            && match declaration.kind {
                DeclarationKind::Global => matches!(
                    declaration.context,
                    DeclarationContext::Script | DeclarationContext::Function
                ),
                DeclarationKind::Persistent => declaration.context == DeclarationContext::Function,
            };
        if !valid_context {
            return;
        }
        for name in &declaration.names {
            if name.text.is_empty() || name.text == "~" {
                continue;
            }
            let Some(storage) = self.binding_storage(&name.text) else {
                continue;
            };
            let kind = match (declaration.kind, storage) {
                (DeclarationKind::Global, BindingStorage::WorkspaceGlobal { .. }) => {
                    let Some(name) =
                        self.add_constant(Constant::String(name.text.clone()), name.span)
                    else {
                        return;
                    };
                    InstructionKind::DeclareGlobal { name }
                }
                (DeclarationKind::Persistent, BindingStorage::Persistent(slot)) => {
                    InstructionKind::DeclarePersistent { slot }
                }
                _ => continue,
            };
            self.emit(kind, self.location(span));
        }
    }

    #[allow(clippy::too_many_lines)]
    fn lower_clear(&mut self, clear: &ClearStatement, span: TextRange) {
        if clear.form == ClearForm::IdentifierList
            && clear.names.len() == 1
            && clear.names[0].text == "import"
        {
            if matches!(self.scope, Scope::Entry { .. }) {
                self.emit(InstructionKind::ClearImports, self.location(span));
            } else {
                self.unsupported(span, UnsupportedFeature::ClearImportScope);
            }
            return;
        }
        match clear.form {
            ClearForm::MissingNames => {
                if matches!(self.scope, Scope::Entry { .. }) {
                    self.emit(InstructionKind::ClearGlobalAll, self.location(span));
                    return;
                }
            }
            ClearForm::UnsupportedArguments => {
                self.unsupported(span, UnsupportedFeature::ClearArguments);
                return;
            }
            ClearForm::IdentifierList => {}
        }
        let targets = if clear.form == ClearForm::MissingNames {
            match &self.scope {
                Scope::Function { bindings, .. } => bindings
                    .iter()
                    .map(|(name, storage)| (name.clone(), *storage, span))
                    .collect::<Vec<_>>(),
                Scope::Entry { .. } => Vec::new(),
            }
        } else {
            clear
                .names
                .iter()
                .filter_map(|name| {
                    if name.text.is_empty() {
                        self.malformed(name.span, "clear name is empty");
                        return None;
                    }
                    let storage = self.binding_storage(&name.text).or_else(|| {
                        matches!(self.scope, Scope::Entry { .. })
                            .then_some(BindingStorage::WorkspaceGlobal { declared: false })
                    });
                    storage.map(|storage| (name.text.clone(), storage, name.span))
                })
                .collect::<Vec<_>>()
        };

        let mut locals = Vec::new();
        let mut captures = Vec::new();
        let mut globals = Vec::new();
        for (name, storage, name_span) in targets {
            match storage {
                BindingStorage::Local(local) => locals.push(local),
                BindingStorage::Captured => captures.push((name, name_span)),
                BindingStorage::WorkspaceGlobal { .. } => globals.push((name, name_span)),
                storage @ BindingStorage::Persistent(_) => {
                    if clear.form == ClearForm::IdentifierList {
                        self.unsupported_binding_operation(
                            name_span,
                            &name,
                            storage,
                            BindingOperation::NamedClear,
                        );
                    }
                }
            }
        }
        if !locals.is_empty() {
            self.emit(InstructionKind::ClearLocal { locals }, self.location(span));
        }
        if !captures.is_empty() {
            let names = captures
                .into_iter()
                .filter_map(|(name, name_span)| {
                    self.add_constant(Constant::String(name), name_span)
                })
                .collect::<Vec<_>>();
            if !names.is_empty() {
                self.emit(InstructionKind::ClearCapture { names }, self.location(span));
            }
        }
        if !globals.is_empty() {
            let names = globals
                .into_iter()
                .filter_map(|(name, name_span)| {
                    self.add_constant(Constant::String(name), name_span)
                })
                .collect::<Vec<_>>();
            if !names.is_empty() {
                self.emit(InstructionKind::ClearGlobal { names }, self.location(span));
            }
        }
        if matches!(self.scope, Scope::Function { .. }) {
            let names = if clear.form == ClearForm::MissingNames {
                Vec::new()
            } else {
                clear
                    .names
                    .iter()
                    .filter(|name| !name.text.is_empty())
                    .filter_map(|name| {
                        self.add_constant(Constant::String(name.text.clone()), name.span)
                    })
                    .collect()
            };
            self.emit(
                InstructionKind::ClearDynamicBindings {
                    names,
                    except: false,
                },
                self.location(span),
            );
        }
    }

    fn lower_assignment(&mut self, target: &Expr, value: &Expr, display: bool) {
        if matrix_contains_aggregate_place(target) {
            self.lower_bracketed_place_assignment(target, value, display);
            return;
        }
        if self.lower_aggregate_assignment(target, value, display) {
            return;
        }
        let Some(targets) = self.assignment_names(target) else {
            return;
        };
        if targets.len() == 1 {
            if targets[0].text == "~" {
                let ExprKind::ParenApply {
                    target: callee,
                    arguments,
                } = &value.kind
                else {
                    self.unsupported(target.span, UnsupportedFeature::AssignmentTarget);
                    return;
                };
                self.lower_apply(callee, arguments, 1, value.span);
                return;
            }
            let Some(register) = self.lower_expression(value) else {
                return;
            };
            self.store_name(&targets[0], register);
            if display {
                self.emit_named_display(targets[0].text, register, target.span);
            }
            return;
        }

        if is_pack_expression(value) {
            let Some(pack) = self.lower_pack_expression(value) else {
                return;
            };
            let mut registers = Vec::new();
            if registers.try_reserve(targets.len()).is_err() {
                self.resource(value.span, CompilerResource::Registers);
                return;
            }
            for _ in 0..targets.len() {
                let Some(register) = self.allocate_register(value.span) else {
                    return;
                };
                registers.push(register);
            }
            self.emit(
                InstructionKind::Unpack {
                    outputs: registers.clone(),
                    pack,
                },
                self.location(value.span),
            );
            for (target, register) in targets.iter().zip(registers) {
                if target.text != "~" {
                    self.store_name(target, register);
                    if display {
                        self.emit_named_display(target.text, register, target.span);
                    }
                }
            }
            return;
        }

        let registers = match &value.kind {
            ExprKind::ParenApply {
                target: callee,
                arguments,
            } => self.lower_apply(callee, arguments, targets.len(), value.span),
            ExprKind::Name(_) => self.lower_apply(value, &[], targets.len(), value.span),
            _ => {
                self.unsupported(value.span, UnsupportedFeature::MultipleAssignmentValue);
                return;
            }
        };
        let Some(registers) = registers else {
            return;
        };
        for (target, register) in targets.iter().zip(registers) {
            if target.text != "~" {
                self.store_name(target, register);
                if display {
                    self.emit_named_display(target.text, register, target.span);
                }
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    fn lower_bracketed_place_assignment(
        &mut self,
        target: &Expr,
        value: &Expr,
        display: bool,
    ) -> Option<()> {
        let ExprKind::Matrix(rows) = &target.kind else {
            return None;
        };
        let [targets] = rows.as_slice() else {
            self.unsupported(target.span, UnsupportedFeature::AssignmentTarget);
            return None;
        };
        // MATLAB evaluates an all-fixed LHS after the RHS. If any destination
        // expands, all LHS selectors run first to determine the output count.
        let expands = targets.iter().any(|target| {
            matches!(
                strip_parens(target).kind,
                ExprKind::BraceApply { .. }
                    | ExprKind::Field { .. }
                    | ExprKind::DynamicField { .. }
            )
        });
        let zero = self.load_constant(Constant::Double(0.0), target.span)?;
        let one = self.load_constant(Constant::Double(1.0), target.span)?;
        let mut total = zero;
        let mut places = Vec::new();
        for target in targets {
            let start = total;
            let prepared = if expands && is_place_expression(strip_parens(target)) {
                let mut components = Vec::new();
                let root = self.collect_place_components(target, &mut components)?;
                let path = self.lower_assignment_path(&root, components, target.span)?;
                Some((root, path))
            } else {
                None
            };
            let count = if let Some((root, path)) = &prepared
                && !matches!(path.last(), Some(PlaceStep::Paren(_)))
            {
                let snapshot = self.load_index_assignment_target(root.text, root.span)?;
                let count = self.allocate_register(target.span)?;
                self.emit(
                    InstructionKind::CountPlaceOutputs {
                        dst: count,
                        root: snapshot,
                        path: path.clone(),
                    },
                    self.location(target.span),
                );
                count
            } else {
                one
            };
            total = self.allocate_register(target.span)?;
            self.emit(
                InstructionKind::Binary {
                    operator: BinaryOperator::Add,
                    dst: total,
                    lhs: start,
                    rhs: count,
                },
                self.location(target.span),
            );
            places.push((target, prepared, start, count));
        }
        let pack = if is_pack_expression(value) {
            self.lower_pack_expression(value)?
        } else {
            match &strip_parens(value).kind {
                ExprKind::ParenApply { target, arguments } => {
                    self.lower_output_pack_apply(target, arguments, total, value.span)?
                }
                ExprKind::Name(name)
                    if !self.has_value_binding(name)
                        && logical_name(name).is_none()
                        && !(matches!(self.scope, Scope::Function { .. })
                            && call_metadata_instruction(name).is_some()) =>
                {
                    self.lower_output_pack_apply(value, &[], total, value.span)?
                }
                _ => {
                    let value_register = self.lower_expression(value)?;
                    self.pack_single_value(value_register, value.span)?
                }
            }
        };
        for (target, prepared, start, count) in places {
            let part = self.allocate_pack_register(target.span)?;
            self.emit(
                InstructionKind::SlicePack {
                    dst_pack: part,
                    source: pack,
                    start,
                    count,
                },
                self.location(target.span),
            );
            if let ExprKind::Name(name) = &strip_parens(target).kind {
                if name != "~" {
                    let value = self.unpack_single(part, target.span)?;
                    self.store_name(
                        &AssignmentName {
                            text: name,
                            span: target.span,
                        },
                        value,
                    );
                    if display {
                        self.emit_named_display(name, value, target.span);
                    }
                }
                continue;
            }
            let (root, path) = if let Some(prepared) = prepared {
                prepared
            } else {
                let mut components = Vec::new();
                let root = self.collect_place_components(target, &mut components)?;
                let path = self.lower_assignment_path(&root, components, target.span)?;
                (root, path)
            };
            let source = if matches!(path.last(), Some(PlaceStep::Paren(_))) {
                ValueSource::One(self.unpack_single(part, target.span)?)
            } else {
                ValueSource::Expand(part)
            };
            // Reload at each write, after RHS side effects and earlier stores.
            // In particular, [a(1),a(2)] must not restore an old root snapshot.
            let snapshot = self.load_index_assignment_target(root.text, root.span)?;
            let storage = self
                .binding_storage(root.text)
                .unwrap_or(BindingStorage::WorkspaceGlobal { declared: false });
            let binding = self.bytecode_binding_target(root.text, storage, root.span)?;
            let result = if display {
                Some(self.allocate_register(target.span)?)
            } else {
                None
            };
            self.emit(
                InstructionKind::AssignBindingPlace {
                    result,
                    binding,
                    root: snapshot,
                    path,
                    source,
                    mode: AssignmentMode::Store,
                },
                self.location(target.span),
            );
            if let Some(result) = result {
                self.emit_named_display(root.text, result, target.span);
            }
        }
        Some(())
    }

    fn unpack_single(&mut self, pack: PackRegister, span: TextRange) -> Option<Register> {
        let value = self.allocate_register(span)?;
        self.emit(
            InstructionKind::Unpack {
                outputs: vec![value],
                pack,
            },
            self.location(span),
        );
        Some(value)
    }

    fn pack_single_value(&mut self, value: Register, span: TextRange) -> Option<PackRegister> {
        let cell = self.allocate_register(span)?;
        let pack = self.allocate_pack_register(span)?;
        self.emit(
            InstructionKind::BuildCell {
                dst: cell,
                rows: vec![vec![ValueSource::One(value)]],
            },
            self.location(span),
        );
        self.emit(
            InstructionKind::BraceApply {
                dst_pack: pack,
                target: cell,
                arguments: vec![ApplyArgument::Colon],
            },
            self.location(span),
        );
        Some(pack)
    }

    fn lower_output_pack_apply(
        &mut self,
        callee: &Expr,
        arguments: &[Expr],
        count: Register,
        span: TextRange,
    ) -> Option<PackRegister> {
        // Preserve compiler intrinsics and table variable-name inference.
        // These forms have a single value; user calls still receive the
        // dynamically computed nargout through ApplyOutputPack below.
        if self.is_class_metadata_target(callee)
            || matches!(&callee.kind, ExprKind::Name(name) if
                (name == "table" || (name == "exist" && exist_variable_name(arguments).is_some()))
                && !self.has_value_binding(name) && !self.user_functions.contains_key(name)
                && !self.class_definitions.contains_key(name))
        {
            let value = self.lower_apply(callee, arguments, 1, span)?[0];
            return self.pack_single_value(value, span);
        }
        let (target, arguments) = if let Some(qualified) = self.qualified_target(callee)
            && !self.has_value_binding(&qualified.root)
            && !self.class_definitions.contains_key(&qualified.root)
        {
            if arguments.iter().any(has_index_bound_end) {
                self.unsupported(span, UnsupportedFeature::EndInMemberApply);
                return None;
            }
            let (target, _, unresolved_suffix, members) =
                self.load_qualified_target(&qualified, callee.span)?;
            let arguments = self.lower_apply_arguments(target, arguments)?;
            (
                PackApplyTarget::Qualified {
                    target,
                    unresolved_suffix,
                    members,
                },
                arguments,
            )
        } else if let ExprKind::Field {
            target: receiver,
            name: Some(name),
        } = &callee.kind
        {
            if arguments.iter().any(has_index_bound_end) {
                self.unsupported(span, UnsupportedFeature::EndInMemberApply);
                return None;
            }
            let object = self.lower_member_receiver(receiver)?;
            let name = self.add_constant(Constant::String(name.text.clone()), name.span)?;
            let arguments = self.lower_apply_arguments(object, arguments)?;
            (PackApplyTarget::Field { object, name }, arguments)
        } else {
            let target = if let ExprKind::Name(name) = &callee.kind {
                self.load_call_target(name, callee.span)?
            } else {
                self.lower_expression(callee)?
            };
            (
                PackApplyTarget::Value(target),
                self.lower_apply_arguments(target, arguments)?,
            )
        };
        let pack = self.allocate_pack_register(span)?;
        self.emit(
            InstructionKind::ApplyOutputPack {
                dst_pack: pack,
                count,
                target,
                arguments,
            },
            self.location(span),
        );
        Some(pack)
    }

    fn lower_assignment_path(
        &mut self,
        root: &AssignmentName<'_>,
        components: Vec<PlaceComponent<'_>>,
        span: TextRange,
    ) -> Option<Vec<PlaceStep>> {
        let last_end = components.iter().rposition(|component| match component {
            PlaceComponent::Paren(arguments, _) | PlaceComponent::Brace(arguments, _) => {
                arguments.iter().any(has_index_bound_end)
            }
            _ => false,
        });
        let mut current = if last_end.is_some() {
            Some(self.load_index_assignment_target(root.text, root.span)?)
        } else {
            None
        };
        let mut path = Vec::new();
        for (index, component) in components.into_iter().enumerate() {
            let step_span = match &component {
                PlaceComponent::Paren(_, span) | PlaceComponent::Brace(_, span) => *span,
                PlaceComponent::StaticField(name) => name.span,
                PlaceComponent::DynamicField(name) => name.span,
            };
            let step = match component {
                PlaceComponent::Paren(arguments, _) | PlaceComponent::Brace(arguments, _) => {
                    let lowered = if let Some(current) = current {
                        self.lower_apply_arguments(current, arguments)?
                    } else {
                        self.lower_apply_arguments_without_target(arguments)?
                    };
                    if matches!(component, PlaceComponent::Paren(_, _)) {
                        PlaceStep::Paren(lowered)
                    } else {
                        PlaceStep::Brace(lowered)
                    }
                }
                PlaceComponent::StaticField(name) => PlaceStep::Field(FieldOperand::Static(
                    self.add_constant(Constant::String(name.text.clone()), name.span)?,
                )),
                PlaceComponent::DynamicField(name) => {
                    PlaceStep::Field(FieldOperand::Dynamic(self.lower_expression(name)?))
                }
            };
            if last_end.is_some_and(|last| index < last) {
                let target = current?;
                current = Some(match &step {
                    PlaceStep::Paren(arguments) => {
                        let value = self.allocate_register(span)?;
                        self.emit(
                            InstructionKind::Apply {
                                outputs: vec![value],
                                target,
                                arguments: arguments.clone(),
                            },
                            self.location(step_span),
                        );
                        value
                    }
                    PlaceStep::Brace(arguments) => {
                        let pack = self.allocate_pack_register(span)?;
                        self.emit(
                            InstructionKind::BraceApply {
                                dst_pack: pack,
                                target,
                                arguments: arguments.clone(),
                            },
                            self.location(step_span),
                        );
                        self.emit(
                            InstructionKind::RequireSinglePack { pack },
                            self.location(step_span),
                        );
                        self.unpack_single(pack, span)?
                    }
                    PlaceStep::Field(field) => {
                        let pack = self.allocate_pack_register(span)?;
                        self.emit(
                            InstructionKind::GetAggregateField {
                                dst_pack: pack,
                                target,
                                field: *field,
                            },
                            self.location(step_span),
                        );
                        self.emit(
                            InstructionKind::RequireSinglePack { pack },
                            self.location(step_span),
                        );
                        self.unpack_single(pack, span)?
                    }
                });
            }
            path.push(step);
        }
        Some(path)
    }

    fn lower_aggregate_assignment(&mut self, target: &Expr, value: &Expr, display: bool) -> bool {
        if !is_place_expression(target) {
            return false;
        }
        if let Some((root, register)) =
            self.lower_place_assignment(target, value, target.span, display)
            && display
        {
            let Some(register) = register else {
                self.malformed(target.span, "displayed aggregate assignment has no result");
                return true;
            };
            self.emit_named_display(root.text, register, target.span);
        }
        true
    }

    fn lower_place_assignment<'hir>(
        &mut self,
        target: &'hir Expr,
        value: &Expr,
        span: TextRange,
        produce_result: bool,
    ) -> Option<(AssignmentName<'hir>, Option<Register>)> {
        let mut components = Vec::new();
        let root = self.collect_place_components(target, &mut components)?;
        let direct_binding = self.binding_storage(root.text).or_else(|| {
            matches!(self.scope, Scope::Entry { .. })
                .then_some(BindingStorage::WorkspaceGlobal { declared: false })
        });
        let can_consume_binding = direct_binding.is_some()
            && components.iter().all(|component| match component {
                PlaceComponent::Paren(arguments, _) | PlaceComponent::Brace(arguments, _) => {
                    !arguments.iter().any(has_index_bound_end)
                }
                PlaceComponent::StaticField(_) | PlaceComponent::DynamicField(_) => true,
            });
        if can_consume_binding {
            let target = self.bytecode_binding_target(
                root.text,
                direct_binding.expect("checked binding storage"),
                root.span,
            )?;
            return self.lower_binding_place_assignment(
                AssignmentBinding { root, target },
                components,
                value,
                span,
                produce_result,
            );
        }
        let path = self.lower_assignment_path(&root, components, span)?;
        let root_register = self.load_index_assignment_target(root.text, root.span)?;

        let delete = is_empty_matrix(value) && matches!(path.last(), Some(PlaceStep::Paren(_)));
        let (source, mode) = if delete {
            let source = self.lower_expression(value)?;
            (ValueSource::One(source), AssignmentMode::Delete)
        } else {
            let source = self.lower_value_source(value)?;
            (source, AssignmentMode::Store)
        };
        let dst = self.allocate_register(span)?;
        self.emit(
            InstructionKind::AssignPlace {
                dst,
                root: root_register,
                path,
                source,
                mode,
            },
            self.location(span),
        );
        self.store_name(&root, dst);
        Some((root, Some(dst)))
    }

    fn lower_binding_place_assignment<'hir>(
        &mut self,
        binding: AssignmentBinding<'hir>,
        components: Vec<PlaceComponent<'hir>>,
        value: &Expr,
        span: TextRange,
        produce_result: bool,
    ) -> Option<(AssignmentName<'hir>, Option<Register>)> {
        let AssignmentBinding { root, target } = binding;
        let mut path = Vec::new();
        if path.try_reserve(components.len()).is_err() {
            self.resource(span, CompilerResource::Registers);
            return None;
        }
        for component in components {
            let step = match component {
                PlaceComponent::Paren(arguments, _) => {
                    PlaceStep::Paren(self.lower_apply_arguments_without_target(arguments)?)
                }
                PlaceComponent::Brace(arguments, _) => {
                    PlaceStep::Brace(self.lower_apply_arguments_without_target(arguments)?)
                }
                PlaceComponent::StaticField(name) => {
                    let name = self.add_constant(Constant::String(name.text.clone()), name.span)?;
                    PlaceStep::Field(FieldOperand::Static(name))
                }
                PlaceComponent::DynamicField(name) => {
                    PlaceStep::Field(FieldOperand::Dynamic(self.lower_expression(name)?))
                }
            };
            path.push(step);
        }
        // MATLAB captures the aggregate root after evaluating every LHS
        // selector but before evaluating the RHS. The direct binding
        // instruction consumes this shallow snapshot at assignment time.
        let root_register = self.load_index_assignment_target(root.text, root.span)?;
        let delete = is_empty_matrix(value) && matches!(path.last(), Some(PlaceStep::Paren(_)));
        let (source, mode) = if delete {
            (
                ValueSource::One(self.lower_expression(value)?),
                AssignmentMode::Delete,
            )
        } else {
            let source = self.lower_value_source(value)?;
            (source, AssignmentMode::Store)
        };
        let result = if produce_result {
            Some(self.allocate_register(span)?)
        } else {
            None
        };
        self.emit(
            InstructionKind::AssignBindingPlace {
                result,
                binding: target,
                root: root_register,
                path,
                source,
                mode,
            },
            self.location(span),
        );
        Some((root, result))
    }

    fn collect_place_components<'hir>(
        &mut self,
        target: &'hir Expr,
        components: &mut Vec<PlaceComponent<'hir>>,
    ) -> Option<AssignmentName<'hir>> {
        match &target.kind {
            ExprKind::Name(name) if !name.is_empty() && name != "~" => Some(AssignmentName {
                text: name,
                span: target.span,
            }),
            ExprKind::Name(_) => {
                self.malformed(target.span, "aggregate assignment root has no usable name");
                None
            }
            ExprKind::Paren(inner) => self.collect_place_components(inner, components),
            ExprKind::ParenApply {
                target: root,
                arguments,
            } => {
                let root_name = self.collect_place_components(root, components)?;
                components.push(PlaceComponent::Paren(arguments, target.span));
                Some(root_name)
            }
            ExprKind::BraceApply {
                target: root,
                arguments,
            } => {
                let root_name = self.collect_place_components(root, components)?;
                components.push(PlaceComponent::Brace(arguments, target.span));
                Some(root_name)
            }
            ExprKind::Field { target: root, name } => {
                let root_name = self.collect_place_components(root, components)?;
                let Some(name) = name.as_ref() else {
                    self.malformed(target.span, "field assignment is missing its member name");
                    return None;
                };
                components.push(PlaceComponent::StaticField(name));
                Some(root_name)
            }
            ExprKind::DynamicField { target: root, name } => {
                let root_name = self.collect_place_components(root, components)?;
                components.push(PlaceComponent::DynamicField(name));
                Some(root_name)
            }
            _ => {
                self.unsupported(target.span, UnsupportedFeature::AssignmentTarget);
                None
            }
        }
    }

    fn lower_superclass_constructor_call(
        &mut self,
        object: &Expr,
        superclass: Option<&openmat_hir::Name>,
        arguments: &[Expr],
        span: TextRange,
    ) {
        let Some(context) = self.constructor_context.clone() else {
            self.malformed(
                span,
                "explicit superclass construction is only valid inside a class constructor",
            );
            return;
        };
        let ExprKind::Name(object_name) = &object.kind else {
            self.malformed(
                object.span,
                "explicit superclass construction must target the constructor output object",
            );
            return;
        };
        let Some(superclass) = superclass else {
            self.malformed(
                span,
                "superclass constructor call is missing its class name",
            );
            return;
        };
        let Some(expected_superclass) = context.direct_superclass.as_deref() else {
            self.malformed(
                superclass.span,
                &format!(
                    "class `{}` has no direct user superclass constructor",
                    context.class_name
                ),
            );
            return;
        };
        if superclass.text != expected_superclass {
            self.malformed(
                superclass.span,
                &format!(
                    "superclass constructor `{}` is not the direct superclass `{expected_superclass}` of `{}`",
                    superclass.text, context.class_name
                ),
            );
            return;
        }
        if context.object_name.as_deref() != Some(object_name.as_str()) {
            self.malformed(
                object.span,
                "explicit superclass construction must target the constructor output object",
            );
            return;
        }
        let Some(object_slot) = context.object_slot else {
            self.malformed(
                object.span,
                "constructor output object has no allocated local slot",
            );
            return;
        };

        let mut lowered_arguments = Vec::new();
        if lowered_arguments.try_reserve(arguments.len()).is_err() {
            self.resource(span, CompilerResource::Registers);
            return;
        }
        for argument in arguments {
            if is_pack_expression(argument) {
                self.unsupported(
                    argument.span,
                    UnsupportedFeature::PackInSuperclassConstructor,
                );
                return;
            }
            let Some(argument) = self.lower_expression(argument) else {
                return;
            };
            lowered_arguments.push(argument);
        }
        let Some(superclass) =
            self.add_constant(Constant::String(superclass.text.clone()), superclass.span)
        else {
            return;
        };
        self.emit(
            InstructionKind::InvokeSuperclassConstructor {
                superclass,
                object: object_slot,
                arguments: lowered_arguments,
            },
            self.location(span),
        );
    }

    fn assignment_names<'hir>(&mut self, target: &'hir Expr) -> Option<Vec<AssignmentName<'hir>>> {
        match &target.kind {
            ExprKind::Name(name) if !name.is_empty() && name != "~" => Some(vec![AssignmentName {
                text: name,
                span: target.span,
            }]),
            ExprKind::Name(_) => {
                self.malformed(target.span, "assignment target has no usable name");
                None
            }
            ExprKind::Matrix(rows) if rows.len() == 1 && !rows[0].is_empty() => {
                let mut names = Vec::with_capacity(rows[0].len());
                for element in &rows[0] {
                    let ExprKind::Name(name) = &element.kind else {
                        self.unsupported(element.span, UnsupportedFeature::AssignmentTarget);
                        return None;
                    };
                    if name.is_empty() {
                        self.malformed(
                            element.span,
                            "multiple-assignment target has an empty name",
                        );
                        return None;
                    }
                    names.push(AssignmentName {
                        text: name,
                        span: element.span,
                    });
                }
                Some(names)
            }
            ExprKind::ParenApply { .. } => {
                self.unsupported(target.span, UnsupportedFeature::DynamicApplyOrIndex);
                None
            }
            _ => {
                self.unsupported(target.span, UnsupportedFeature::AssignmentTarget);
                None
            }
        }
    }

    fn store_name(&mut self, target: &AssignmentName<'_>, register: Register) {
        let storage = self.binding_storage(target.text).or_else(|| {
            matches!(self.scope, Scope::Entry { .. })
                .then_some(BindingStorage::WorkspaceGlobal { declared: false })
        });
        let Some(storage) = storage else {
            self.malformed(
                target.span,
                "assignment name has no allocated binding storage",
            );
            return;
        };
        self.store_binding(target.text, target.span, storage, register);
    }

    fn lower_if(&mut self, branches: &[ConditionalBranch], else_body: &[Stmt], span: TextRange) {
        if branches.is_empty() {
            self.malformed(span, "if statement has no conditional branch");
            self.lower_statements(else_body, false);
            return;
        }
        let mut end_patches = Vec::new();
        for branch in branches {
            let Some(condition) = self.lower_expression(&branch.condition) else {
                continue;
            };
            let false_patch = self.emit_jump_if_false(condition, branch.condition.span);
            self.lower_statements(&branch.body, false);
            if let Some(patch) = self.emit_jump(branch.span) {
                end_patches.push(patch);
            }
            if let Some(patch) = false_patch {
                self.patch_to_current(patch, branch.span);
            }
        }
        self.lower_statements(else_body, false);
        for patch in end_patches {
            self.patch_to_current(patch, span);
        }
    }

    fn lower_try(&mut self, statement: &TryStatement, span: TextRange) {
        let Some(protected_start) = self.current_instruction(span) else {
            return;
        };
        self.lower_statements(&statement.body, false);
        let Some(protected_end) = self.current_instruction(span) else {
            return;
        };
        if protected_start == protected_end {
            return;
        }

        let Some(catch) = statement.catch.as_ref() else {
            let Some(continuation_patch) = self.emit_jump(span) else {
                return;
            };
            self.patch_to_current(continuation_patch, span);
            self.function.exception_handlers.push(
                ExceptionHandler::swallow(protected_start, protected_end, protected_end)
                    .with_location(self.location(span)),
            );
            return;
        };

        let Some(catch_error_local) = self.catch_error_local(catch) else {
            return;
        };
        let error_local = match catch_error_local {
            CatchErrorLocal::Plain => None,
            CatchErrorLocal::Direct(local) => Some(local),
            CatchErrorLocal::Indirect { scratch, .. } => Some(scratch),
        };
        let Some(skip_patch) = self.emit_jump(span) else {
            return;
        };
        let Some(handler) = self.current_instruction(span) else {
            return;
        };

        if let (Some(variable), CatchErrorLocal::Indirect { scratch, target }) =
            (&catch.variable, catch_error_local)
        {
            let Some(value) = self.allocate_register(variable.span) else {
                return;
            };
            if self
                .emit(
                    InstructionKind::LoadLocal {
                        dst: value,
                        local: scratch,
                    },
                    self.location(variable.span),
                )
                .is_none()
            {
                return;
            }
            self.store_binding(&variable.text, variable.span, target, value);
        }

        self.lower_statements(&catch.body, false);
        let Some(exit) = self.current_instruction(span) else {
            return;
        };
        self.patch_to_current(skip_patch, span);
        self.function.exception_handlers.push(
            ExceptionHandler::catch(protected_start, protected_end, handler, exit, error_local)
                .with_location(self.location(span)),
        );
    }

    fn catch_error_local(&mut self, catch: &CatchClause) -> Option<CatchErrorLocal> {
        let Some(variable) = catch.variable.as_ref() else {
            return Some(CatchErrorLocal::Plain);
        };
        if variable.text.is_empty() || variable.text == "~" {
            self.malformed(variable.span, "catch binding has no usable name");
            return None;
        }
        let storage = self.binding_storage(&variable.text).or_else(|| {
            matches!(self.scope, Scope::Entry { .. })
                .then_some(BindingStorage::WorkspaceGlobal { declared: false })
        });
        let Some(storage) = storage else {
            self.malformed(
                variable.span,
                "catch binding has no allocated surrounding-scope storage",
            );
            return None;
        };
        match storage {
            BindingStorage::Local(local) => Some(CatchErrorLocal::Direct(local)),
            target @ (BindingStorage::Captured
            | BindingStorage::WorkspaceGlobal { .. }
            | BindingStorage::Persistent(_)) => self
                .allocate_internal_local(variable.span)
                .map(|scratch| CatchErrorLocal::Indirect { scratch, target }),
        }
    }

    fn lower_switch(
        &mut self,
        selector: &Expr,
        cases: &[SwitchCase],
        otherwise: Option<&OtherwiseBranch>,
        span: TextRange,
    ) {
        let Some(selector) = self.lower_expression(selector) else {
            return;
        };
        let mut end_patches = Vec::new();

        for case in cases {
            let Some(case_value) = self.lower_expression(&case.expression) else {
                continue;
            };
            let Some(matched) = self.allocate_register(case.expression.span) else {
                return;
            };
            if self
                .emit(
                    InstructionKind::SwitchMatch {
                        dst: matched,
                        selector,
                        case_value,
                    },
                    self.location(case.expression.span),
                )
                .is_none()
            {
                return;
            }
            let no_match = self.emit_jump_if_false(matched, case.expression.span);
            self.lower_statements(&case.body, false);
            if let Some(patch) = self.emit_jump(case.span) {
                end_patches.push(patch);
            }
            if let Some(patch) = no_match {
                self.patch_to_current(patch, case.span);
            }
        }

        if let Some(otherwise) = otherwise {
            self.lower_statements(&otherwise.body, false);
        }
        for patch in end_patches {
            self.patch_to_current(patch, span);
        }
    }

    fn lower_while(&mut self, condition: &Expr, body: &[Stmt], span: TextRange) {
        let Some(loop_start) = self.current_instruction(span) else {
            return;
        };
        let Some(condition_register) = self.lower_expression(condition) else {
            return;
        };
        let exit_patch = self.emit_jump_if_false(condition_register, condition.span);
        self.loops.push(LoopContext {
            continue_target: loop_start,
            break_patches: Vec::new(),
        });
        self.lower_statements(body, false);
        let loop_context = self.loops.pop();
        self.emit(
            InstructionKind::Jump { target: loop_start },
            self.location(span),
        );
        if let Some(patch) = exit_patch {
            self.patch_to_current(patch, span);
        }
        if let Some(context) = loop_context {
            for patch in context.break_patches {
                self.patch_to_current(patch, span);
            }
        }
    }

    fn lower_for(&mut self, variable: &Expr, iterable: &Expr, body: &[Stmt], span: TextRange) {
        let ExprKind::Name(name) = &variable.kind else {
            self.unsupported(variable.span, UnsupportedFeature::AssignmentTarget);
            return;
        };
        if name.is_empty() || name == "~" {
            self.malformed(variable.span, "for variable has no usable name");
            return;
        }
        let Some(iterable) = self.lower_expression(iterable) else {
            return;
        };
        let Some(index) = self.load_constant(Constant::Double(0.0), variable.span) else {
            return;
        };
        let Some(loop_start) = self.current_instruction(span) else {
            return;
        };
        let Some(iteration) = self.allocate_register(variable.span) else {
            return;
        };
        let Some(exit_patch) = self.emit(
            InstructionKind::ForEach {
                iterable,
                index,
                dst: iteration,
                exit: InstructionIndex::new(0),
            },
            self.location(span),
        ) else {
            return;
        };
        self.store_name(
            &AssignmentName {
                text: name,
                span: variable.span,
            },
            iteration,
        );
        self.loops.push(LoopContext {
            continue_target: loop_start,
            break_patches: Vec::new(),
        });
        self.lower_statements(body, false);
        let loop_context = self.loops.pop();
        self.emit(
            InstructionKind::Jump { target: loop_start },
            self.location(span),
        );
        self.patch_to_current(exit_patch, span);
        if let Some(context) = loop_context {
            for patch in context.break_patches {
                self.patch_to_current(patch, span);
            }
        }
    }

    fn lower_break(&mut self, span: TextRange) {
        if self.loops.is_empty() {
            self.diagnostics.push(CompileDiagnostic::new(
                self.source_id,
                span,
                CompileDiagnosticKind::LoopControlOutsideLoop { keyword: "break" },
            ));
            return;
        }
        let patch = self.emit_jump(span);
        if let (Some(context), Some(patch)) = (self.loops.last_mut(), patch) {
            context.break_patches.push(patch);
        }
    }

    fn lower_continue(&mut self, span: TextRange) {
        let Some(target) = self.loops.last().map(|context| context.continue_target) else {
            self.diagnostics.push(CompileDiagnostic::new(
                self.source_id,
                span,
                CompileDiagnosticKind::LoopControlOutsideLoop {
                    keyword: "continue",
                },
            ));
            return;
        };
        self.emit(InstructionKind::Jump { target }, self.location(span));
    }

    fn emit_return(&mut self, span: TextRange) {
        // Explicit returns join the same epilogue outside body exception ranges.
        for patch in std::mem::take(&mut self.output_return_patches) {
            self.patch_to_current(patch, span);
        }
        for declaration in self.output_validations.clone() {
            self.lower_argument_validation(&declaration, false);
        }
        if let Some(declaration) = self.repeating_output.clone() {
            self.lower_repeating_validation(&[declaration], false);
        }
        let output_bindings = match &self.scope {
            Scope::Entry { .. } => Vec::new(),
            Scope::Function { outputs, .. } => outputs.to_vec(),
        };
        let variadic = output_bindings.last().is_some_and(|output| {
            output.name == "varargout"
                || self
                    .repeating_output
                    .as_ref()
                    .is_some_and(|d| d.name.text == output.name)
        });
        let fixed_count = output_bindings.len().saturating_sub(usize::from(variadic));
        let mut values = Vec::with_capacity(fixed_count);
        for output in &output_bindings[..fixed_count] {
            let Some(register) = self.load_binding(&output.name, output.storage, span, false)
            else {
                return;
            };
            values.push(register);
        }
        if variadic {
            let output = &output_bindings[fixed_count];
            let Some(variadic) = self.load_binding(&output.name, output.storage, span, false)
            else {
                return;
            };
            self.emit(
                InstructionKind::ReturnVariadic {
                    fixed: values,
                    variadic,
                },
                self.location(span),
            );
        } else {
            self.emit(InstructionKind::Return { values }, self.location(span));
        }
    }

    fn emit_values_return(&mut self, values: &[Register], span: TextRange) {
        self.emit(
            InstructionKind::Return {
                values: values.to_vec(),
            },
            self.location(span),
        );
    }

    #[allow(clippy::too_many_lines)]
    fn lower_expression(&mut self, expression: &Expr) -> Option<Register> {
        if matches!(expression.kind, ExprKind::Field { .. })
            && let Some(qualified) = self.qualified_target(expression)
            && !self.has_value_binding(&qualified.root)
            && !self.class_definitions.contains_key(&qualified.root)
        {
            return self.lower_qualified_value(&qualified, expression.span);
        }
        match &expression.kind {
            ExprKind::Name(name) => self.load_name(name, expression.span),
            ExprKind::Number(literal) => {
                let constant = parse_number(literal).ok_or_else(|| {
                    self.diagnostics.push(CompileDiagnostic::new(
                        self.source_id,
                        expression.span,
                        CompileDiagnosticKind::InvalidNumericLiteral {
                            literal: literal.clone(),
                        },
                    ));
                });
                constant
                    .ok()
                    .and_then(|value| self.load_constant(value, expression.span))
            }
            ExprKind::Char(literal) => self.lower_string(literal, '\'', expression.span),
            ExprKind::String(literal) => self.lower_string(literal, '"', expression.span),
            ExprKind::Paren(inner) => self.lower_expression(inner),
            ExprKind::ParenApply { target, arguments } => self
                .lower_apply(target, arguments, 1, expression.span)
                .and_then(|outputs| outputs.into_iter().next()),
            ExprKind::BraceApply { .. }
            | ExprKind::Field { .. }
            | ExprKind::DynamicField { .. } => self.lower_pack_to_one(expression),
            ExprKind::SuperclassConstructorCall { .. } => {
                self.malformed(
                    expression.span,
                    "explicit superclass construction must be a standalone constructor statement",
                );
                None
            }
            ExprKind::Unary { operator, operand } => {
                self.lower_unary(*operator, operand, expression.span)
            }
            ExprKind::Binary {
                operator,
                left,
                right,
            } => self.lower_binary(*operator, left, right, expression.span),
            ExprKind::Matrix(rows) => self.lower_matrix(rows, expression.span),
            ExprKind::Cell(rows) => self.lower_cell(rows, expression.span),
            ExprKind::AllIndex => {
                self.unsupported(expression.span, UnsupportedFeature::AllIndex);
                None
            }
            ExprKind::EndIndex => self.lower_end_index(expression.span),
            ExprKind::FunctionHandle(name) => {
                self.lower_function_handle(name.as_ref(), expression.span)
            }
            ExprKind::AnonymousFunction { parameters, body } => {
                self.lower_anonymous_function(parameters, body, expression.span)
            }
            ExprKind::Range { start, step, end } => {
                self.lower_range(start, step.as_deref(), end, expression.span)
            }
            ExprKind::Transpose { kind, operand } => {
                self.lower_transpose(*kind, operand, expression.span)
            }
            ExprKind::Error => {
                self.malformed(expression.span, "expression is an explicit error node");
                None
            }
            _ => {
                self.malformed(expression.span, "unknown expression kind");
                None
            }
        }
    }

    fn lower_matrix(&mut self, rows: &[Vec<Expr>], span: TextRange) -> Option<Register> {
        let mut lowered_rows = Vec::with_capacity(rows.len());
        for row in rows {
            let mut lowered = Vec::with_capacity(row.len());
            for element in row {
                lowered.push(self.lower_expression(element)?);
            }
            lowered_rows.push(lowered);
        }
        let dst = self.allocate_register(span)?;
        self.emit(
            InstructionKind::BuildMatrix {
                dst,
                rows: lowered_rows,
            },
            self.location(span),
        );
        Some(dst)
    }

    fn lower_cell(&mut self, rows: &[Vec<Expr>], span: TextRange) -> Option<Register> {
        if let Some(expected) = rows.first().map(Vec::len)
            && rows.iter().skip(1).any(|row| row.len() != expected)
        {
            self.unsupported(span, UnsupportedFeature::UnequalCellSourceRows);
            return None;
        }
        let mut lowered_rows = Vec::new();
        if lowered_rows.try_reserve(rows.len()).is_err() {
            self.resource(span, CompilerResource::Registers);
            return None;
        }
        for row in rows {
            let mut lowered = Vec::new();
            if lowered.try_reserve(row.len()).is_err() {
                self.resource(span, CompilerResource::Registers);
                return None;
            }
            for element in row {
                lowered.push(self.lower_value_source(element)?);
            }
            lowered_rows.push(lowered);
        }
        let dst = self.allocate_register(span)?;
        self.emit(
            InstructionKind::BuildCell {
                dst,
                rows: lowered_rows,
            },
            self.location(span),
        );
        Some(dst)
    }

    fn lower_value_source(&mut self, expression: &Expr) -> Option<ValueSource> {
        if is_pack_expression(expression) {
            self.lower_pack_expression(expression)
                .map(ValueSource::Expand)
        } else {
            self.lower_expression(expression).map(ValueSource::One)
        }
    }

    fn lower_pack_to_one(&mut self, expression: &Expr) -> Option<Register> {
        let pack = self.lower_pack_expression(expression)?;
        let output = self.allocate_register(expression.span)?;
        self.emit(
            InstructionKind::Unpack {
                outputs: vec![output],
                pack,
            },
            self.location(expression.span),
        );
        Some(output)
    }

    fn lower_pack_expression(&mut self, expression: &Expr) -> Option<PackRegister> {
        if matches!(expression.kind, ExprKind::Field { .. })
            && let Some(qualified) = self.qualified_target(expression)
            && !self.has_value_binding(&qualified.root)
            && !self.class_definitions.contains_key(&qualified.root)
        {
            return self.lower_qualified_pack(&qualified, expression.span);
        }
        match &expression.kind {
            ExprKind::Paren(inner) if is_pack_expression(inner) => {
                self.lower_pack_expression(inner)
            }
            ExprKind::BraceApply { target, arguments } => {
                let target = self.lower_expression(target)?;
                let arguments = self.lower_apply_arguments(target, arguments)?;
                let dst_pack = self.allocate_pack_register(expression.span)?;
                self.emit(
                    InstructionKind::BraceApply {
                        dst_pack,
                        target,
                        arguments,
                    },
                    self.location(expression.span),
                );
                Some(dst_pack)
            }
            ExprKind::Field { target, name } => {
                let Some(name) = name.as_ref() else {
                    self.malformed(expression.span, "field access is missing its member name");
                    return None;
                };
                let target = self.lower_member_receiver(target)?;
                let name = self.add_constant(Constant::String(name.text.clone()), name.span)?;
                let dst_pack = self.allocate_pack_register(expression.span)?;
                self.emit(
                    InstructionKind::GetAggregateField {
                        dst_pack,
                        target,
                        field: FieldOperand::Static(name),
                    },
                    self.location(expression.span),
                );
                Some(dst_pack)
            }
            ExprKind::DynamicField { target, name } => {
                let target = self.lower_member_receiver(target)?;
                let name = self.lower_expression(name)?;
                let dst_pack = self.allocate_pack_register(expression.span)?;
                self.emit(
                    InstructionKind::GetAggregateField {
                        dst_pack,
                        target,
                        field: FieldOperand::Dynamic(name),
                    },
                    self.location(expression.span),
                );
                Some(dst_pack)
            }
            _ => {
                self.malformed(
                    expression.span,
                    "expression was selected as a value-pack producer but has no pack lowering",
                );
                None
            }
        }
    }

    fn lower_end_index(&mut self, span: TextRange) -> Option<Register> {
        let Some(context) = self.index_contexts.last().copied() else {
            self.unsupported(span, UnsupportedFeature::EndOutsideIndex);
            return None;
        };
        let dst = self.allocate_register(span)?;
        self.emit(
            InstructionKind::ResolveEnd {
                dst,
                target: context.target,
                argument_index: context.argument_index,
                argument_count: context.argument_count,
            },
            self.location(span),
        );
        Some(dst)
    }

    fn lower_range(
        &mut self,
        start: &Expr,
        step: Option<&Expr>,
        end: &Expr,
        span: TextRange,
    ) -> Option<Register> {
        let start = self.lower_expression(start)?;
        let step = if let Some(step) = step {
            self.lower_expression(step)?
        } else {
            self.load_constant(Constant::Double(1.0), span)?
        };
        let end = self.lower_expression(end)?;
        let dst = self.allocate_register(span)?;
        self.emit(
            InstructionKind::Range {
                dst,
                start,
                step,
                end,
            },
            self.location(span),
        );
        Some(dst)
    }

    fn lower_transpose(
        &mut self,
        kind: TransposeKind,
        operand: &Expr,
        span: TextRange,
    ) -> Option<Register> {
        let conjugate = match kind {
            TransposeKind::Conjugate => true,
            TransposeKind::NonConjugate => false,
            TransposeKind::Unknown => {
                self.unsupported(
                    span,
                    UnsupportedFeature::Operator("unknown transpose".to_owned()),
                );
                return None;
            }
        };
        let operand = self.lower_expression(operand)?;
        let dst = self.allocate_register(span)?;
        self.emit(
            InstructionKind::Transpose {
                dst,
                operand,
                conjugate,
            },
            self.location(span),
        );
        Some(dst)
    }

    fn lower_string(
        &mut self,
        literal: &str,
        delimiter: char,
        span: TextRange,
    ) -> Option<Register> {
        let Some(value) = decode_quoted(literal, delimiter) else {
            self.diagnostics.push(CompileDiagnostic::new(
                self.source_id,
                span,
                CompileDiagnosticKind::InvalidStringLiteral {
                    literal: literal.to_owned(),
                },
            ));
            return None;
        };
        let constant = match delimiter {
            '\'' => Constant::Char(value.encode_utf16().collect()),
            '"' => Constant::String(value),
            _ => {
                self.malformed(span, "quoted literal uses an unsupported delimiter");
                return None;
            }
        };
        self.load_constant(constant, span)
    }

    fn lower_unary(
        &mut self,
        operator: UnaryOp,
        operand: &Expr,
        span: TextRange,
    ) -> Option<Register> {
        let operand = self.lower_expression(operand)?;
        match operator {
            UnaryOp::Plus | UnaryOp::Minus => {
                let factor = if operator == UnaryOp::Plus { 1.0 } else { -1.0 };
                let factor = self.load_constant(Constant::Double(factor), span)?;
                let output = self.allocate_register(span)?;
                self.emit(
                    InstructionKind::Binary {
                        operator: BinaryOperator::Multiply,
                        dst: output,
                        lhs: factor,
                        rhs: operand,
                    },
                    self.location(span),
                );
                Some(output)
            }
            UnaryOp::Not => self.lower_logical_not(operand, span),
            UnaryOp::Unknown => {
                self.unsupported(
                    span,
                    UnsupportedFeature::Operator("unknown unary".to_owned()),
                );
                None
            }
        }
    }

    fn lower_binary(
        &mut self,
        operator: BinaryOp,
        left: &Expr,
        right: &Expr,
        span: TextRange,
    ) -> Option<Register> {
        match operator {
            BinaryOp::And => return self.lower_logical_and(left, right, span, false),
            BinaryOp::ShortCircuitAnd => {
                return self.lower_logical_and(left, right, span, true);
            }
            BinaryOp::Or => return self.lower_logical_or(left, right, span, false),
            BinaryOp::ShortCircuitOr => {
                return self.lower_logical_or(left, right, span, true);
            }
            _ => {}
        }
        let lhs = self.lower_expression(left)?;
        let rhs = self.lower_expression(right)?;
        let bytecode_operator = match operator {
            BinaryOp::Add => BinaryOperator::Add,
            BinaryOp::Subtract => BinaryOperator::Subtract,
            BinaryOp::Multiply => BinaryOperator::Multiply,
            BinaryOp::ElementMultiply => BinaryOperator::ElementMultiply,
            BinaryOp::RightDivide => BinaryOperator::Divide,
            BinaryOp::ElementRightDivide => BinaryOperator::ElementDivide,
            BinaryOp::LeftDivide => BinaryOperator::LeftDivide,
            BinaryOp::ElementLeftDivide => BinaryOperator::ElementLeftDivide,
            BinaryOp::Power => BinaryOperator::Power,
            BinaryOp::ElementPower => BinaryOperator::ElementPower,
            BinaryOp::Equal => BinaryOperator::Equal,
            BinaryOp::NotEqual => BinaryOperator::NotEqual,
            BinaryOp::Less => BinaryOperator::LessThan,
            BinaryOp::LessEqual => BinaryOperator::LessThanOrEqual,
            BinaryOp::Greater => BinaryOperator::GreaterThan,
            BinaryOp::GreaterEqual => BinaryOperator::GreaterThanOrEqual,
            BinaryOp::Unknown => {
                self.unsupported(
                    span,
                    UnsupportedFeature::Operator("unknown binary".to_owned()),
                );
                return None;
            }
            BinaryOp::And | BinaryOp::ShortCircuitAnd | BinaryOp::Or | BinaryOp::ShortCircuitOr => {
                return None;
            }
        };
        let output = self.allocate_register(span)?;
        self.emit(
            InstructionKind::Binary {
                operator: bytecode_operator,
                dst: output,
                lhs,
                rhs,
            },
            self.location(span),
        );
        Some(output)
    }

    fn lower_logical_not(&mut self, operand: Register, span: TextRange) -> Option<Register> {
        let zero = self.load_constant(Constant::Double(0.0), span)?;
        let output = self.allocate_register(span)?;
        self.emit(
            InstructionKind::Binary {
                operator: BinaryOperator::Equal,
                dst: output,
                lhs: operand,
                rhs: zero,
            },
            self.location(span),
        );
        Some(output)
    }

    fn lower_logical_and(
        &mut self,
        left: &Expr,
        right: &Expr,
        span: TextRange,
        short_circuit: bool,
    ) -> Option<Register> {
        if !short_circuit {
            return self.lower_elementwise_logical(left, right, span, true);
        }
        let left = self.lower_expression(left)?;
        let output = self.load_constant(Constant::Logical(false), span)?;
        let left_false = self.emit_jump_if_false(left, span);
        let right = self.lower_expression(right)?;
        let right_false = self.emit_jump_if_false(right, span);
        let true_value = self.load_constant(Constant::Logical(true), span)?;
        self.emit(
            InstructionKind::Move {
                dst: output,
                src: true_value,
            },
            self.location(span),
        );
        if let Some(patch) = left_false {
            self.patch_to_current(patch, span);
        }
        if let Some(patch) = right_false {
            self.patch_to_current(patch, span);
        }
        Some(output)
    }

    fn lower_logical_or(
        &mut self,
        left: &Expr,
        right: &Expr,
        span: TextRange,
        short_circuit: bool,
    ) -> Option<Register> {
        if !short_circuit {
            return self.lower_elementwise_logical(left, right, span, false);
        }
        let left = self.lower_expression(left)?;
        let output = self.load_constant(Constant::Logical(true), span)?;
        let left_false = self.emit_jump_if_false(left, span);
        let end_from_left = self.emit_jump(span);
        if let Some(patch) = left_false {
            self.patch_to_current(patch, span);
        }
        let right = self.lower_expression(right)?;
        let right_false = self.emit_jump_if_false(right, span);
        let end_from_right = self.emit_jump(span);
        if let Some(patch) = right_false {
            self.patch_to_current(patch, span);
        }
        let false_value = self.load_constant(Constant::Logical(false), span)?;
        self.emit(
            InstructionKind::Move {
                dst: output,
                src: false_value,
            },
            self.location(span),
        );
        if let Some(patch) = end_from_left {
            self.patch_to_current(patch, span);
        }
        if let Some(patch) = end_from_right {
            self.patch_to_current(patch, span);
        }
        Some(output)
    }

    fn lower_elementwise_logical(
        &mut self,
        left: &Expr,
        right: &Expr,
        span: TextRange,
        conjunction: bool,
    ) -> Option<Register> {
        let left = self.lower_expression(left)?;
        let right = self.lower_expression(right)?;
        let zero = self.load_constant(Constant::Double(0.0), span)?;
        let left_logical = self.allocate_register(span)?;
        self.emit(
            InstructionKind::Binary {
                operator: BinaryOperator::NotEqual,
                dst: left_logical,
                lhs: left,
                rhs: zero,
            },
            self.location(span),
        );
        let right_logical = self.allocate_register(span)?;
        self.emit(
            InstructionKind::Binary {
                operator: BinaryOperator::NotEqual,
                dst: right_logical,
                lhs: right,
                rhs: zero,
            },
            self.location(span),
        );
        let combined = self.allocate_register(span)?;
        self.emit(
            InstructionKind::Binary {
                operator: if conjunction {
                    BinaryOperator::ElementMultiply
                } else {
                    BinaryOperator::Add
                },
                dst: combined,
                lhs: left_logical,
                rhs: right_logical,
            },
            self.location(span),
        );
        let output = self.allocate_register(span)?;
        self.emit(
            InstructionKind::Binary {
                operator: BinaryOperator::NotEqual,
                dst: output,
                lhs: combined,
                rhs: zero,
            },
            self.location(span),
        );
        Some(output)
    }

    fn lower_apply(
        &mut self,
        target: &Expr,
        arguments: &[Expr],
        output_count: usize,
        span: TextRange,
    ) -> Option<Vec<Register>> {
        if self.is_class_metadata_target(target) {
            return self.lower_class_metadata_apply(arguments, output_count, span);
        }
        if let Some(qualified) = self.qualified_target(target)
            && !self.has_value_binding(&qualified.root)
            && !self.class_definitions.contains_key(&qualified.root)
        {
            return self.lower_qualified_apply(&qualified, target, arguments, output_count, span);
        }
        if let ExprKind::Name(name) = &target.kind
            && name == "exist"
            && !self.has_value_binding(name)
            && let Some(variable) = exist_variable_name(arguments)
        {
            return self.lower_exist_variable(&variable, output_count, span);
        }
        if let ExprKind::Field {
            target: object,
            name,
        } = &target.kind
        {
            let Some(name) = name.as_ref() else {
                self.malformed(target.span, "method application is missing its member name");
                return None;
            };
            let object = self.lower_member_receiver(object)?;
            let name = self.add_constant(Constant::String(name.text.clone()), name.span)?;
            if arguments.iter().any(has_index_bound_end) {
                self.unsupported(span, UnsupportedFeature::EndInMemberApply);
                return None;
            }
            let arguments = self.lower_apply_arguments(object, arguments)?;
            let outputs = self.allocate_output_registers(output_count, span)?;
            self.emit(
                InstructionKind::ApplyField {
                    outputs: outputs.clone(),
                    object,
                    name,
                    arguments,
                },
                self.location(span),
            );
            return Some(outputs);
        }
        if let ExprKind::Name(name) = &target.kind
            && self.binding_storage(name).is_some()
            && !arguments.iter().any(has_index_bound_end)
        {
            let target = self.load_resolved_name(name, target.span, false, false)?;
            let arguments = self.lower_apply_arguments_without_target(arguments)?;
            let outputs = self.allocate_output_registers(output_count, span)?;
            self.emit(
                InstructionKind::ApplyBinding {
                    outputs: outputs.clone(),
                    target,
                    arguments,
                },
                self.location(span),
            );
            return Some(outputs);
        }
        let inferred_table_names = if let ExprKind::Name(name) = &target.kind
            && name == "table"
            && !self.has_value_binding(name)
            && !self.user_functions.contains_key(name)
            && !self.class_definitions.contains_key(name)
        {
            inferred_table_variable_names(arguments)
        } else {
            None
        };
        let target = if let ExprKind::Name(name) = &target.kind {
            self.load_call_target(name, target.span)?
        } else {
            self.lower_expression(target)?
        };
        let mut arguments = self.lower_apply_arguments(target, arguments)?;
        if let Some(names) = inferred_table_names {
            self.append_inferred_table_variable_names(&mut arguments, &names, span)?;
        }
        let outputs = self.allocate_output_registers(output_count, span)?;
        self.emit(
            InstructionKind::Apply {
                outputs: outputs.clone(),
                target,
                arguments,
            },
            self.location(span),
        );
        Some(outputs)
    }

    fn lower_qualified_apply(
        &mut self,
        qualified: &QualifiedTarget,
        expression: &Expr,
        arguments: &[Expr],
        output_count: usize,
        span: TextRange,
    ) -> Option<Vec<Register>> {
        if arguments.iter().any(has_index_bound_end) {
            self.unsupported(span, UnsupportedFeature::EndInMemberApply);
            return None;
        }
        let (target, _root_found, unresolved_suffix, members) =
            self.load_qualified_target(qualified, expression.span)?;
        let arguments = self.lower_apply_arguments(target, arguments)?;
        let outputs = self.allocate_output_registers(output_count, span)?;
        self.emit(
            InstructionKind::ApplyQualified {
                outputs: outputs.clone(),
                target,
                unresolved_suffix,
                members,
                arguments,
            },
            self.location(span),
        );
        Some(outputs)
    }

    fn lower_qualified_value(
        &mut self,
        qualified: &QualifiedTarget,
        span: TextRange,
    ) -> Option<Register> {
        let pack = self.lower_qualified_pack(qualified, span)?;
        let dst = self.allocate_register(span)?;
        self.emit(
            InstructionKind::Unpack {
                outputs: vec![dst],
                pack,
            },
            self.location(span),
        );
        Some(dst)
    }

    fn lower_qualified_pack(
        &mut self,
        qualified: &QualifiedTarget,
        span: TextRange,
    ) -> Option<PackRegister> {
        let (target, root_found, unresolved_suffix, members) =
            self.load_qualified_target(qualified, span)?;
        let dst_pack = self.allocate_pack_register(span)?;
        self.emit(
            InstructionKind::GetQualifiedPack {
                dst_pack,
                target,
                root_found,
                unresolved_suffix,
                members,
            },
            self.location(span),
        );
        Some(dst_pack)
    }

    fn is_class_metadata_target(&self, target: &Expr) -> bool {
        let direct = matches!(&target.kind, ExprKind::Name(name) if name == "metaclass")
            && !self.has_value_binding("metaclass")
            && !self.user_functions.contains_key("metaclass")
            && !self.class_definitions.contains_key("metaclass");
        direct || is_meta_class_from_name(target)
    }

    fn lower_class_metadata_apply(
        &mut self,
        arguments: &[Expr],
        output_count: usize,
        span: TextRange,
    ) -> Option<Vec<Register>> {
        if arguments.len() != 1 || output_count != 1 {
            self.malformed(
                span,
                "class metadata lookup requires one input and one output",
            );
            return None;
        }
        let source = self.lower_expression(&arguments[0])?;
        let dst = self.allocate_register(span)?;
        let name = self.add_constant(
            Constant::String("__openmat_internal:class_metadata".to_owned()),
            span,
        )?;
        self.emit(
            InstructionKind::GetField {
                dst,
                object: source,
                name,
            },
            self.location(span),
        );
        Some(vec![dst])
    }

    fn append_inferred_table_variable_names(
        &mut self,
        arguments: &mut Vec<ApplyArgument>,
        names: &[String],
        span: TextRange,
    ) -> Option<()> {
        let option = self.load_constant(
            Constant::Char("VariableNames".encode_utf16().collect()),
            span,
        )?;
        let mut row = Vec::new();
        if row.try_reserve_exact(names.len()).is_err() {
            self.resource(span, CompilerResource::Registers);
            return None;
        }
        for name in names {
            let value = self.load_constant(Constant::Char(name.encode_utf16().collect()), span)?;
            row.push(ValueSource::One(value));
        }
        let names = self.allocate_register(span)?;
        self.emit(
            InstructionKind::BuildCell {
                dst: names,
                rows: vec![row],
            },
            self.location(span),
        );
        arguments.push(ApplyArgument::Value(option));
        arguments.push(ApplyArgument::Value(names));
        Some(())
    }

    fn lower_exist_variable(
        &mut self,
        variable: &str,
        output_count: usize,
        span: TextRange,
    ) -> Option<Vec<Register>> {
        let value = if let Some(storage) = self.binding_storage(variable) {
            self.load_binding(variable, storage, span, false)?
        } else {
            let dst = self.allocate_register(span)?;
            let name = self.add_constant(Constant::String(variable.to_owned()), span)?;
            self.emit(
                InstructionKind::LoadGlobalOrNothing { dst, name },
                self.location(span),
            );
            dst
        };
        let callee = self.load_global("__openmat_exist_var", span, false)?;
        let outputs = (0..output_count)
            .map(|_| self.allocate_register(span))
            .collect::<Option<Vec<_>>>()?;
        self.emit(
            InstructionKind::Apply {
                outputs: outputs.clone(),
                target: callee,
                arguments: vec![ApplyArgument::Value(value)],
            },
            self.location(span),
        );
        Some(outputs)
    }

    fn lower_statement_apply(
        &mut self,
        target: &Expr,
        arguments: &[Expr],
        display: bool,
        span: TextRange,
    ) {
        let Some(result) = self.statement_result_target(span) else {
            return;
        };
        if let Some(qualified) = self.qualified_target(target)
            && !self.has_value_binding(&qualified.root)
            && !self.class_definitions.contains_key(&qualified.root)
        {
            if arguments.iter().any(has_index_bound_end) {
                self.unsupported(span, UnsupportedFeature::EndInMemberApply);
                return;
            }
            let Some((target, _root_found, unresolved_suffix, members)) =
                self.load_qualified_target(&qualified, target.span)
            else {
                return;
            };
            let Some(arguments) = self.lower_apply_arguments(target, arguments) else {
                return;
            };
            self.emit(
                InstructionKind::StatementApplyQualified {
                    target,
                    unresolved_suffix,
                    members,
                    arguments,
                    result,
                    display,
                },
                self.location(span),
            );
            return;
        }
        if let ExprKind::Field {
            target: object,
            name,
        } = &target.kind
        {
            let Some(name) = name.as_ref() else {
                self.malformed(target.span, "method application is missing its member name");
                return;
            };
            let Some(object) = self.lower_member_receiver(object) else {
                return;
            };
            let Some(name) = self.add_constant(Constant::String(name.text.clone()), name.span)
            else {
                return;
            };
            if arguments.iter().any(has_index_bound_end) {
                self.unsupported(span, UnsupportedFeature::EndInMemberApply);
                return;
            }
            let Some(arguments) = self.lower_apply_arguments(object, arguments) else {
                return;
            };
            self.emit(
                InstructionKind::StatementApplyField {
                    object,
                    name,
                    arguments,
                    result,
                    display,
                },
                self.location(span),
            );
            return;
        }
        let target = if let ExprKind::Name(name) = &target.kind {
            self.load_call_target(name, target.span)
        } else {
            self.lower_expression(target)
        };
        let Some(target) = target else {
            return;
        };
        let Some(arguments) = self.lower_apply_arguments(target, arguments) else {
            return;
        };
        self.emit(
            InstructionKind::StatementApply {
                target,
                arguments,
                result,
                display,
            },
            self.location(span),
        );
    }

    fn lower_command_statement(
        &mut self,
        command: &CommandStatement,
        display: bool,
        span: TextRange,
    ) {
        let Some(callee) = &command.callee else {
            self.malformed(span, "command-form statement is missing its callee");
            return;
        };
        if callee.text.is_empty() {
            self.malformed(callee.span, "command-form callee name is empty");
            return;
        }
        if callee.text == "clearvars" {
            self.lower_clearvars_command(command, span);
            return;
        }
        if callee.text == "import" {
            if command.arguments.is_empty() {
                self.malformed(span, "import requires at least one qualified package name");
                return;
            }
            self.redeclare_imports(
                &command
                    .arguments
                    .iter()
                    .map(|argument| argument.text.clone())
                    .collect::<Vec<_>>(),
                span,
            );
            return;
        }
        let Some(target) = self.load_resolved_name(&callee.text, callee.span, false, false) else {
            return;
        };
        let mut arguments = Vec::with_capacity(command.arguments.len());
        for argument in &command.arguments {
            let Some(value) = self.load_constant(
                Constant::Char(argument.text.encode_utf16().collect()),
                argument.span,
            ) else {
                return;
            };
            arguments.push(ApplyArgument::Value(value));
        }
        let Some(result) = self.statement_result_target(span) else {
            return;
        };
        self.emit(
            InstructionKind::StatementApply {
                target,
                arguments,
                result,
                display,
            },
            self.location(span),
        );
    }

    fn lower_clearvars_command(&mut self, command: &CommandStatement, span: TextRange) {
        let except = command
            .arguments
            .first()
            .is_some_and(|argument| argument.text.eq_ignore_ascii_case("-except"));
        if command
            .arguments
            .iter()
            .enumerate()
            .any(|(index, argument)| argument.text.starts_with('-') && !(except && index == 0))
            || (except && command.arguments.len() == 1)
        {
            self.unsupported(span, UnsupportedFeature::ClearArguments);
            return;
        }
        let requested = command
            .arguments
            .iter()
            .skip(usize::from(except))
            .map(|argument| argument.text.clone())
            .collect::<BTreeSet<_>>();

        self.lower_clearvars_targets(except, &requested, command.arguments.is_empty(), span);
    }

    fn lower_clearvars_targets(
        &mut self,
        except: bool,
        requested: &BTreeSet<String>,
        all: bool,
        span: TextRange,
    ) {
        if matches!(self.scope, Scope::Entry { .. }) && all {
            self.emit(InstructionKind::ClearGlobalAll, self.location(span));
            return;
        }
        if matches!(self.scope, Scope::Entry { .. }) && except {
            self.unsupported(span, UnsupportedFeature::ClearArguments);
            return;
        }

        let bindings = match &self.scope {
            Scope::Entry { bindings } | Scope::Function { bindings, .. } => bindings
                .iter()
                .map(|(name, storage)| (name.clone(), *storage))
                .collect::<Vec<_>>(),
        };
        let selected = bindings
            .into_iter()
            .filter(|(name, _)| {
                if except {
                    !requested.contains(name)
                } else {
                    all || requested.contains(name)
                }
            })
            .collect::<Vec<_>>();
        let locals = selected
            .iter()
            .filter_map(|(_, storage)| match storage {
                BindingStorage::Local(local) => Some(*local),
                _ => None,
            })
            .collect::<Vec<_>>();
        if !locals.is_empty() {
            self.emit(InstructionKind::ClearLocal { locals }, self.location(span));
        }
        let captures = selected
            .iter()
            .filter_map(|(name, storage)| {
                matches!(storage, BindingStorage::Captured).then_some(name.clone())
            })
            .filter_map(|name| self.add_constant(Constant::String(name), span))
            .collect::<Vec<_>>();
        if !captures.is_empty() {
            self.emit(
                InstructionKind::ClearCapture { names: captures },
                self.location(span),
            );
        }
        if matches!(self.scope, Scope::Entry { .. }) {
            let globals = selected
                .into_iter()
                .filter_map(|(name, storage)| {
                    matches!(storage, BindingStorage::WorkspaceGlobal { .. }).then_some(name)
                })
                .filter_map(|name| self.add_constant(Constant::String(name), span))
                .collect::<Vec<_>>();
            if !globals.is_empty() {
                self.emit(
                    InstructionKind::ClearGlobal { names: globals },
                    self.location(span),
                );
            }
        } else {
            let names = requested
                .iter()
                .filter_map(|name| self.add_constant(Constant::String(name.clone()), span))
                .collect();
            self.emit(
                InstructionKind::ClearDynamicBindings { names, except },
                self.location(span),
            );
        }
    }

    fn statement_result_target(&mut self, span: TextRange) -> Option<StatementResultTarget> {
        let storage = self.binding_storage("ans").or_else(|| {
            matches!(self.scope, Scope::Entry { .. })
                .then_some(BindingStorage::WorkspaceGlobal { declared: false })
        });
        let Some(storage) = storage else {
            self.malformed(span, "expression statement has no implicit `ans` binding");
            return None;
        };
        match storage {
            BindingStorage::Local(slot) => Some(StatementResultTarget::Local(slot)),
            BindingStorage::WorkspaceGlobal { .. } => self
                .add_constant(Constant::String("ans".to_owned()), span)
                .map(StatementResultTarget::Global),
            storage @ BindingStorage::Persistent(_) => {
                self.unsupported_binding_operation(
                    span,
                    "ans",
                    storage,
                    BindingOperation::StatementResult,
                );
                None
            }
            BindingStorage::Captured => {
                self.malformed(
                    span,
                    "captured ans storage cannot be used as a statement target",
                );
                None
            }
        }
    }

    fn emit_named_display(&mut self, name: &str, register: Register, span: TextRange) {
        let Some(name) = self.add_constant(Constant::String(name.to_owned()), span) else {
            return;
        };
        self.emit(
            InstructionKind::Display {
                name,
                src: register,
            },
            self.location(span),
        );
    }

    fn lower_return_apply(&mut self, target: &Expr, arguments: &[Expr], span: TextRange) {
        if let Some(qualified) = self.qualified_target(target)
            && !self.has_value_binding(&qualified.root)
            && !self.class_definitions.contains_key(&qualified.root)
        {
            if let Some(outputs) = self.lower_apply(target, arguments, 1, span) {
                self.emit_values_return(&outputs, span);
            } else {
                self.emit_values_return(&[], span);
            }
            return;
        }
        if let ExprKind::Field {
            target: object,
            name,
        } = &target.kind
        {
            let Some(name) = name.as_ref() else {
                self.malformed(target.span, "method application is missing its member name");
                return;
            };
            if arguments.iter().any(has_index_bound_end) {
                self.unsupported(span, UnsupportedFeature::EndInMemberApply);
                return;
            }
            let lowered = self.lower_member_receiver(object).and_then(|object| {
                let name = self.add_constant(Constant::String(name.text.clone()), name.span)?;
                let arguments = self.lower_apply_arguments(object, arguments)?;
                Some((object, name, arguments))
            });
            if let Some((object, name, arguments)) = lowered {
                self.emit(
                    InstructionKind::ReturnApplyField {
                        object,
                        name,
                        arguments,
                    },
                    self.location(span),
                );
            } else {
                self.emit_values_return(&[], span);
            }
            return;
        }
        let target = if let ExprKind::Name(name) = &target.kind {
            self.load_resolved_name(name, target.span, false, false)
        } else {
            self.lower_expression(target)
        };
        let arguments = target.and_then(|target| {
            self.lower_apply_arguments(target, arguments)
                .map(|arguments| (target, arguments))
        });
        if let Some((target, arguments)) = arguments {
            self.emit(
                InstructionKind::ReturnApply { target, arguments },
                self.location(span),
            );
        } else {
            self.emit_values_return(&[], span);
        }
    }

    fn lower_apply_arguments(
        &mut self,
        target: Register,
        arguments: &[Expr],
    ) -> Option<Vec<ApplyArgument>> {
        let Ok(argument_count) = u32::try_from(arguments.len()) else {
            self.resource(self.function_span, CompilerResource::Parameters);
            return None;
        };
        let mut lowered = Vec::with_capacity(arguments.len());
        for (argument_index, argument) in arguments.iter().enumerate() {
            let argument_index = u32::try_from(argument_index).ok()?;
            self.index_contexts.push(IndexContext {
                target,
                argument_index,
                argument_count,
            });
            let result = if matches!(argument.kind, ExprKind::AllIndex) {
                Some(ApplyArgument::Colon)
            } else if is_pack_expression(argument) {
                self.lower_pack_expression(argument)
                    .map(ApplyArgument::Expand)
            } else {
                self.lower_expression(argument).map(ApplyArgument::Value)
            };
            self.index_contexts.pop();
            lowered.push(result?);
        }
        Some(lowered)
    }

    fn lower_apply_arguments_without_target(
        &mut self,
        arguments: &[Expr],
    ) -> Option<Vec<ApplyArgument>> {
        let mut lowered = Vec::with_capacity(arguments.len());
        for argument in arguments {
            let result = if matches!(argument.kind, ExprKind::AllIndex) {
                Some(ApplyArgument::Colon)
            } else if is_pack_expression(argument) {
                self.lower_pack_expression(argument)
                    .map(ApplyArgument::Expand)
            } else {
                self.lower_expression(argument).map(ApplyArgument::Value)
            };
            lowered.push(result?);
        }
        Some(lowered)
    }

    fn lower_function_handle(
        &mut self,
        name: Option<&openmat_hir::Name>,
        span: TextRange,
    ) -> Option<Register> {
        let Some(name) = name else {
            self.malformed(span, "named function handle is missing its target");
            return None;
        };
        if name.text.is_empty() {
            self.malformed(name.span, "named function handle target is empty");
            return None;
        }
        if let Some(storage @ (BindingStorage::Local(_) | BindingStorage::Captured)) =
            self.binding_storage(&name.text)
        {
            return self.load_binding(&name.text, storage, span, false);
        }
        if let Some(function) = self.user_functions.get(&name.text).copied() {
            return self.load_constant(Constant::Function(function), span);
        }
        let candidates = if name.text.contains('.') {
            Vec::new()
        } else {
            self.import_candidates(&name.text)
        };
        if !candidates.is_empty() {
            let names = candidates
                .into_iter()
                .map(|candidate| self.add_constant(Constant::String(candidate), name.span))
                .collect::<Option<Vec<_>>>()?;
            let dst = self.allocate_register(span)?;
            self.emit(
                InstructionKind::LoadFunctionHandleCandidates { dst, names },
                self.location(span),
            );
            return Some(dst);
        }
        let name = self.add_constant(Constant::String(name.text.clone()), name.span)?;
        let dst = self.allocate_register(span)?;
        self.emit(
            InstructionKind::LoadFunctionHandle { dst, name },
            self.location(span),
        );
        Some(dst)
    }

    fn lower_anonymous_function(
        &mut self,
        parameters: &[openmat_hir::Name],
        body: &Expr,
        span: TextRange,
    ) -> Option<Register> {
        let analysis =
            analyse_anonymous_parameters(self.source_id, parameters, span, self.diagnostics);
        let free_names = anonymous_free_names(parameters, body);
        let static_captures = free_names
            .iter()
            .filter(|capture| self.has_value_binding(&capture.name))
            .map(|capture| capture.name.clone())
            .collect::<BTreeSet<_>>();
        let mut captures = Vec::new();
        if captures.try_reserve(free_names.len()).is_err() {
            self.resource(span, CompilerResource::Constants);
            return None;
        }
        for capture in &free_names {
            let name = &capture.name;
            let value = match self.binding_storage(name) {
                Some(
                    storage @ (BindingStorage::Local(_)
                    | BindingStorage::Captured
                    | BindingStorage::Persistent(_)),
                ) => Some(self.load_binding(name, storage, span, false)?),
                Some(BindingStorage::WorkspaceGlobal { .. }) | None => None,
            };
            let name = self.add_constant(Constant::String(name.clone()), span)?;
            captures.push((name, value, capture.tombstone_if_missing));
        }

        let base = self.anonymous_function_base?;
        let anonymous_index = self.anonymous_functions.len();
        let raw_index = u32::try_from(anonymous_index)
            .ok()
            .and_then(|index| base.checked_add(index));
        let Some(raw_index) = raw_index else {
            self.resource(span, CompilerResource::Functions);
            return None;
        };
        if self.anonymous_functions.try_reserve(1).is_err() {
            self.resource(span, CompilerResource::Functions);
            return None;
        }
        self.anonymous_functions
            .push(Function::new("<anonymous-reserved>", 0, 0, 0));

        let anonymous = {
            let mut compiler = FunctionCompiler::new(
                self.source_id,
                span,
                &format!("<anonymous:{}>", span.start()),
                Scope::Function {
                    bindings: &analysis.bindings,
                    outputs: &analysis.output_bindings,
                    captures: &static_captures,
                },
                self.user_functions,
                self.class_definitions,
                self.anonymous_function_base,
                self.anonymous_functions,
                self.diagnostics,
                analysis.local_count,
                analysis.parameter_count,
                0,
            );
            compiler.set_imports(self.imports.clone(), span);
            if let Some(slot) = analysis.variadic_input_slot {
                compiler.emit_variadic_inputs(slot, span);
            }
            if let ExprKind::ParenApply { target, arguments } = &body.kind {
                compiler.lower_return_apply(target, arguments, body.span);
            } else if let Some(value) = compiler.lower_expression(body) {
                compiler.emit_values_return(&[value], body.span);
            } else {
                compiler.emit_values_return(&[], body.span);
            }
            compiler.finish()
        };
        let Some(slot) = self.anonymous_functions.get_mut(anonymous_index) else {
            self.internal(
                span,
                InternalCompilerError::InvalidAnonymousFunctionReservation,
            );
            return None;
        };
        *slot = anonymous;

        let dst = self.allocate_register(span)?;
        self.emit(
            InstructionKind::MakeClosure {
                dst,
                function: FunctionId::new(raw_index),
                captures,
            },
            self.location(span),
        );
        Some(dst)
    }

    fn load_name(&mut self, name: &str, span: TextRange) -> Option<Register> {
        self.load_resolved_name(name, span, true, true)
    }

    fn load_call_target(&mut self, name: &str, span: TextRange) -> Option<Register> {
        if self.binding_storage(name).is_some() || self.user_functions.contains_key(name) {
            return self.load_resolved_name(name, span, false, false);
        }
        let name = self.add_constant(Constant::String(name.to_owned()), span)?;
        let register = self.allocate_register(span)?;
        self.emit(
            InstructionKind::LoadCallTarget {
                dst: register,
                name,
            },
            self.location(span),
        );
        Some(register)
    }

    fn lower_member_receiver(&mut self, target: &Expr) -> Option<Register> {
        if let ExprKind::Name(name) = &target.kind {
            self.load_resolved_name(name, target.span, true, false)
        } else {
            self.lower_expression(target)
        }
    }

    fn load_index_assignment_target(&mut self, name: &str, span: TextRange) -> Option<Register> {
        let storage = self.binding_storage(name).or_else(|| {
            matches!(self.scope, Scope::Entry { .. })
                .then_some(BindingStorage::WorkspaceGlobal { declared: false })
        });
        match storage {
            Some(BindingStorage::WorkspaceGlobal { .. }) => {
                let name = self.add_constant(Constant::String(name.to_owned()), span)?;
                let register = self.allocate_register(span)?;
                self.emit(
                    InstructionKind::LoadGlobalOrNothing {
                        dst: register,
                        name,
                    },
                    self.location(span),
                );
                Some(register)
            }
            Some(storage) => self.load_binding(name, storage, span, false),
            None => {
                self.malformed(span, "aggregate assignment root has no binding storage");
                None
            }
        }
    }

    fn load_resolved_name(
        &mut self,
        name: &str,
        span: TextRange,
        recognize_bare_special: bool,
        construct_if_class: bool,
    ) -> Option<Register> {
        if name.is_empty() {
            self.malformed(span, "name expression is empty");
            return None;
        }
        if let Some(storage) = self.binding_storage(name) {
            let value = self.load_binding(name, storage, span, construct_if_class)?;
            if matches!(storage, BindingStorage::Local(_) | BindingStorage::Captured) {
                let name = self.add_constant(Constant::String(name.to_owned()), span)?;
                self.emit(
                    InstructionKind::RequireDefined { value, name },
                    self.location(span),
                );
            }
            return Some(value);
        }
        if !self.has_value_binding(name) {
            if recognize_bare_special && let Some(value) = logical_name(name) {
                return self.load_constant(Constant::Logical(value), span);
            }
            if recognize_bare_special
                && matches!(self.scope, Scope::Function { .. })
                && let Some(kind) = call_metadata_instruction(name)
            {
                let dst = self.allocate_register(span)?;
                self.emit(kind(dst), self.location(span));
                return Some(dst);
            }
            if let Some(function) = self.user_functions.get(name).copied() {
                let callee = self.load_constant(Constant::Function(function), span)?;
                if construct_if_class {
                    return self.call_zero_argument_value(callee, span);
                }
                return Some(callee);
            }
            let candidates = self.import_candidates(name);
            if !candidates.is_empty() {
                let qualified = QualifiedTarget {
                    root: name.to_owned(),
                    members: Vec::new(),
                    candidates,
                };
                if !construct_if_class {
                    let (target, _, _, _) = self.load_qualified_target(&qualified, span)?;
                    return Some(target);
                }
                return self.lower_qualified_value(&qualified, span);
            }
        }
        self.load_global(name, span, construct_if_class)
    }

    fn binding_storage(&self, name: &str) -> Option<BindingStorage> {
        match &self.scope {
            Scope::Entry { bindings } | Scope::Function { bindings, .. } => {
                bindings.get(name).copied()
            }
        }
    }

    fn bytecode_binding_target(
        &mut self,
        name: &str,
        storage: BindingStorage,
        span: TextRange,
    ) -> Option<BindingTarget> {
        match storage {
            BindingStorage::Local(local) => Some(BindingTarget::Local(local)),
            BindingStorage::Persistent(slot) => Some(BindingTarget::Persistent(slot)),
            BindingStorage::Captured => self
                .add_constant(Constant::String(name.to_owned()), span)
                .map(BindingTarget::Capture),
            BindingStorage::WorkspaceGlobal { .. } => self
                .add_constant(Constant::String(name.to_owned()), span)
                .map(BindingTarget::Workspace),
        }
    }

    fn load_binding(
        &mut self,
        name: &str,
        storage: BindingStorage,
        span: TextRange,
        construct_if_class: bool,
    ) -> Option<Register> {
        let register = self.allocate_register(span)?;
        let kind = match storage {
            BindingStorage::Local(local) => InstructionKind::LoadLocal {
                dst: register,
                local,
            },
            BindingStorage::Captured => {
                let name = self.add_constant(Constant::String(name.to_owned()), span)?;
                InstructionKind::LoadCapture {
                    dst: register,
                    name,
                }
            }
            BindingStorage::WorkspaceGlobal { declared } => {
                let name = self.add_constant(Constant::String(name.to_owned()), span)?;
                InstructionKind::LoadGlobal {
                    dst: register,
                    name,
                    construct_if_class: !declared && construct_if_class,
                }
            }
            BindingStorage::Persistent(slot) => InstructionKind::LoadPersistent {
                dst: register,
                slot,
            },
        };
        self.emit(kind, self.location(span));
        Some(register)
    }

    fn store_binding(
        &mut self,
        name: &str,
        span: TextRange,
        storage: BindingStorage,
        register: Register,
    ) {
        let kind = match storage {
            BindingStorage::Local(local) => InstructionKind::StoreLocal {
                local,
                src: register,
            },
            BindingStorage::Captured => {
                let Some(name) = self.add_constant(Constant::String(name.to_owned()), span) else {
                    return;
                };
                InstructionKind::StoreCapture {
                    name,
                    src: register,
                }
            }
            BindingStorage::WorkspaceGlobal { .. } => {
                let Some(name) = self.add_constant(Constant::String(name.to_owned()), span) else {
                    return;
                };
                InstructionKind::StoreGlobal {
                    name,
                    src: register,
                }
            }
            BindingStorage::Persistent(slot) => InstructionKind::StorePersistent {
                slot,
                src: register,
            },
        };
        self.emit(kind, self.location(span));
    }

    fn load_global(
        &mut self,
        name: &str,
        span: TextRange,
        construct_if_class: bool,
    ) -> Option<Register> {
        let name = self.add_constant(Constant::String(name.to_owned()), span)?;
        let register = self.allocate_register(span)?;
        self.emit(
            InstructionKind::LoadGlobal {
                dst: register,
                name,
                construct_if_class,
            },
            self.location(span),
        );
        Some(register)
    }

    fn load_qualified_target(
        &mut self,
        qualified: &QualifiedTarget,
        span: TextRange,
    ) -> Option<(Register, Register, Register, Vec<ConstantId>)> {
        let root = self.add_constant(Constant::String(qualified.root.clone()), span)?;
        let candidates = qualified
            .candidates
            .iter()
            .map(|candidate| self.add_constant(Constant::String(candidate.clone()), span))
            .collect::<Option<Vec<_>>>()?;
        let members = qualified
            .members
            .iter()
            .map(|member| self.add_constant(Constant::String(member.clone()), span))
            .collect::<Option<Vec<_>>>()?;
        let target = self.allocate_register(span)?;
        let root_found = self.allocate_register(span)?;
        let unresolved_suffix = self.allocate_register(span)?;
        self.emit(
            InstructionKind::LoadQualifiedTarget {
                dst: target,
                root_found,
                unresolved_suffix,
                root,
                qualified: candidates,
                member_count: u32::try_from(members.len()).ok()?,
            },
            self.location(span),
        );
        Some((target, root_found, unresolved_suffix, members))
    }

    fn qualified_target(&self, expression: &Expr) -> Option<QualifiedTarget> {
        if let ExprKind::Name(root) = &expression.kind {
            let candidates = self.import_candidates(root);
            return (!candidates.is_empty()).then(|| QualifiedTarget {
                root: root.clone(),
                members: Vec::new(),
                candidates,
            });
        }

        let qualified = static_qualified_name(expression)?;
        let members = qualified
            .members
            .iter()
            .map(|member| (*member).to_owned())
            .collect::<Vec<_>>();
        let already_qualified = self.imports.iter().any(|import| {
            let imported_name = import.strip_suffix(".*").unwrap_or(import);
            qualified.full_name == imported_name
                || qualified
                    .full_name
                    .strip_prefix(imported_name)
                    .is_some_and(|suffix| suffix.starts_with('.'))
        });
        let imported = if already_qualified {
            Vec::new()
        } else {
            self.import_candidates(qualified.root)
        };
        let candidates = if imported.is_empty() {
            vec![qualified.full_name]
        } else {
            imported
                .into_iter()
                .map(|root| {
                    if members.is_empty() {
                        root
                    } else {
                        format!("{root}.{}", members.join("."))
                    }
                })
                .collect()
        };
        Some(QualifiedTarget {
            root: qualified.root.to_owned(),
            members,
            candidates,
        })
    }

    fn import_candidates(&self, name: &str) -> Vec<String> {
        self.imports
            .iter()
            .filter_map(|import| {
                if let Some(package) = import.strip_suffix(".*") {
                    Some(format!("{package}.{name}"))
                } else {
                    import
                        .rsplit_once('.')
                        .is_some_and(|(_, leaf)| leaf == name)
                        .then(|| import.clone())
                }
            })
            .collect()
    }

    fn has_value_binding(&self, name: &str) -> bool {
        match &self.scope {
            Scope::Entry { bindings } => bindings.contains_key(name),
            Scope::Function {
                bindings, captures, ..
            } => bindings.contains_key(name) || captures.contains(name),
        }
    }

    fn load_constant(&mut self, constant: Constant, span: TextRange) -> Option<Register> {
        let constant = self.add_constant(constant, span)?;
        let register = self.allocate_register(span)?;
        self.emit(
            InstructionKind::LoadConstant {
                dst: register,
                constant,
            },
            self.location(span),
        );
        Some(register)
    }

    fn call_zero_argument_value(&mut self, callee: Register, span: TextRange) -> Option<Register> {
        let output = self.allocate_register(span)?;
        self.emit(
            InstructionKind::Call {
                outputs: vec![output],
                callee,
                arguments: Vec::new(),
            },
            self.location(span),
        );
        Some(output)
    }

    fn add_constant(&mut self, constant: Constant, span: TextRange) -> Option<ConstantId> {
        let Ok(index) = u32::try_from(self.function.constants.len()) else {
            self.resource(span, CompilerResource::Constants);
            return None;
        };
        self.function.constants.push(constant);
        Some(ConstantId::new(index))
    }

    fn allocate_register(&mut self, span: TextRange) -> Option<Register> {
        let current = self.next_register;
        let Some(next) = current.checked_add(1) else {
            self.resource(span, CompilerResource::Registers);
            return None;
        };
        self.next_register = next;
        Some(Register::new(current))
    }

    fn allocate_output_registers(
        &mut self,
        count: usize,
        span: TextRange,
    ) -> Option<Vec<Register>> {
        let mut registers = Vec::new();
        if registers.try_reserve(count).is_err() {
            self.resource(span, CompilerResource::Registers);
            return None;
        }
        for _ in 0..count {
            registers.push(self.allocate_register(span)?);
        }
        Some(registers)
    }

    fn allocate_pack_register(&mut self, span: TextRange) -> Option<PackRegister> {
        let current = self.next_pack_register;
        let Some(next) = current.checked_add(1) else {
            self.resource(span, CompilerResource::PackRegisters);
            return None;
        };
        self.next_pack_register = next;
        Some(PackRegister::new(current))
    }

    fn allocate_internal_local(&mut self, span: TextRange) -> Option<LocalSlot> {
        let current = self.function.local_count;
        let Some(next) = current.checked_add(1) else {
            self.resource(span, CompilerResource::Locals);
            return None;
        };
        self.function.local_count = next;
        Some(LocalSlot::new(current))
    }

    fn emit(&mut self, kind: InstructionKind, location: SourceLocation) -> Option<usize> {
        if u32::try_from(self.function.instructions.len()).is_err() {
            self.resource(self.function_span, CompilerResource::Instructions);
            return None;
        }
        let index = self.function.instructions.len();
        self.function
            .instructions
            .push(Instruction::located(kind, location));
        Some(index)
    }

    fn emit_jump(&mut self, span: TextRange) -> Option<usize> {
        self.emit(
            InstructionKind::Jump {
                target: InstructionIndex::new(0),
            },
            self.location(span),
        )
    }

    fn emit_jump_if_false(&mut self, condition: Register, span: TextRange) -> Option<usize> {
        self.emit(
            InstructionKind::JumpIfFalse {
                condition,
                target: InstructionIndex::new(0),
            },
            self.location(span),
        )
    }

    fn current_instruction(&mut self, span: TextRange) -> Option<InstructionIndex> {
        let Ok(index) = u32::try_from(self.function.instructions.len()) else {
            self.resource(span, CompilerResource::Instructions);
            return None;
        };
        Some(InstructionIndex::new(index))
    }

    fn patch_to_current(&mut self, patch: usize, span: TextRange) {
        let Some(target) = self.current_instruction(span) else {
            return;
        };
        let Some(instruction) = self.function.instructions.get_mut(patch) else {
            self.internal(span, InternalCompilerError::InvalidJumpPatch);
            return;
        };
        match &mut instruction.kind {
            InstructionKind::Jump {
                target: instruction_target,
            }
            | InstructionKind::JumpIfFalse {
                target: instruction_target,
                ..
            }
            | InstructionKind::ForEach {
                exit: instruction_target,
                ..
            } => *instruction_target = target,
            _ => self.internal(span, InternalCompilerError::InvalidJumpPatch),
        }
    }

    const fn location(&self, span: TextRange) -> SourceLocation {
        SourceLocation::new(self.source_id.raw(), span.start(), span.end())
    }

    fn malformed(&mut self, span: TextRange, detail: &str) {
        malformed(self.diagnostics, self.source_id, span, detail);
    }

    fn unsupported(&mut self, span: TextRange, feature: UnsupportedFeature) {
        self.diagnostics.push(CompileDiagnostic::new(
            self.source_id,
            span,
            CompileDiagnosticKind::Unsupported { feature },
        ));
    }

    fn unsupported_binding_operation(
        &mut self,
        span: TextRange,
        name: &str,
        storage: BindingStorage,
        operation: BindingOperation,
    ) {
        self.diagnostics.push(CompileDiagnostic::new(
            self.source_id,
            span,
            CompileDiagnosticKind::UnsupportedBindingOperation {
                name: name.to_owned(),
                storage: storage.class(),
                operation,
            },
        ));
    }

    fn resource(&mut self, span: TextRange, resource: CompilerResource) {
        resource_limit(self.diagnostics, self.source_id, span, resource);
    }

    fn internal(&mut self, span: TextRange, error: InternalCompilerError) {
        self.diagnostics.push(CompileDiagnostic::new(
            self.source_id,
            span,
            CompileDiagnosticKind::Internal { error },
        ));
    }
}

fn is_meta_class_from_name(expression: &Expr) -> bool {
    let ExprKind::Field {
        target: class_package,
        name: Some(from_name),
    } = &strip_parens(expression).kind
    else {
        return false;
    };
    if from_name.text != "fromName" {
        return false;
    }
    let ExprKind::Field {
        target: meta_package,
        name: Some(class_name),
    } = &strip_parens(class_package).kind
    else {
        return false;
    };
    class_name.text == "class"
        && matches!(&strip_parens(meta_package).kind, ExprKind::Name(name) if name == "meta")
}

struct StaticQualifiedName<'a> {
    root: &'a str,
    members: Vec<&'a str>,
    full_name: String,
}

struct QualifiedTarget {
    root: String,
    members: Vec<String>,
    candidates: Vec<String>,
}

fn static_qualified_name(expression: &Expr) -> Option<StaticQualifiedName<'_>> {
    fn collect<'a>(expression: &'a Expr, components: &mut Vec<&'a str>) -> Option<()> {
        match &expression.kind {
            ExprKind::Name(name) if !name.is_empty() => components.push(name),
            ExprKind::Field {
                target,
                name: Some(name),
            } if !name.text.is_empty() => {
                collect(target, components)?;
                components.push(&name.text);
            }
            _ => return None,
        }
        Some(())
    }

    let mut components = Vec::new();
    collect(expression, &mut components)?;
    (components.len() > 1).then(|| StaticQualifiedName {
        root: components[0],
        members: components[1..].to_vec(),
        full_name: components.join("."),
    })
}

fn is_pack_expression(expression: &Expr) -> bool {
    match &expression.kind {
        ExprKind::BraceApply { .. } | ExprKind::Field { .. } | ExprKind::DynamicField { .. } => {
            true
        }
        ExprKind::Paren(inner) => is_pack_expression(inner),
        _ => false,
    }
}

fn is_place_expression(expression: &Expr) -> bool {
    matches!(
        expression.kind,
        ExprKind::ParenApply { .. }
            | ExprKind::BraceApply { .. }
            | ExprKind::Field { .. }
            | ExprKind::DynamicField { .. }
    )
}

fn strip_parens(mut expression: &Expr) -> &Expr {
    while let ExprKind::Paren(inner) = &expression.kind {
        expression = inner;
    }
    expression
}

fn matrix_contains_aggregate_place(target: &Expr) -> bool {
    let ExprKind::Matrix(rows) = &target.kind else {
        return false;
    };
    rows.iter().flatten().any(is_place_expression)
}

fn is_empty_matrix(expression: &Expr) -> bool {
    match &expression.kind {
        ExprKind::Matrix(rows) => rows.is_empty(),
        ExprKind::Paren(inner) => is_empty_matrix(inner),
        _ => false,
    }
}

fn has_index_bound_end(expression: &Expr) -> bool {
    match &expression.kind {
        ExprKind::EndIndex => true,
        ExprKind::Paren(inner)
        | ExprKind::Unary { operand: inner, .. }
        | ExprKind::Transpose { operand: inner, .. } => has_index_bound_end(inner),
        ExprKind::ParenApply { target, .. } | ExprKind::BraceApply { target, .. } => {
            has_index_bound_end(target)
        }
        ExprKind::Field { target, .. } => has_index_bound_end(target),
        ExprKind::DynamicField { target, name } => {
            has_index_bound_end(target) || has_index_bound_end(name)
        }
        ExprKind::Binary { left, right, .. } => {
            has_index_bound_end(left) || has_index_bound_end(right)
        }
        ExprKind::Range { start, step, end } => {
            has_index_bound_end(start)
                || step.as_deref().is_some_and(has_index_bound_end)
                || has_index_bound_end(end)
        }
        ExprKind::Matrix(rows) | ExprKind::Cell(rows) => {
            rows.iter().flatten().any(has_index_bound_end)
        }
        _ => false,
    }
}

fn parse_number(literal: &str) -> Option<Constant> {
    if let Some(imaginary) = literal
        .strip_suffix('i')
        .or_else(|| literal.strip_suffix('j'))
    {
        return imaginary
            .parse::<f64>()
            .ok()
            .map(|imaginary| Constant::Complex {
                real: 0.0,
                imaginary,
            });
    }
    literal.parse::<f64>().ok().map(Constant::Double)
}

fn logical_name(name: &str) -> Option<bool> {
    match name {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn call_metadata_instruction(name: &str) -> Option<fn(Register) -> InstructionKind> {
    match name {
        "nargin" => Some(|dst| InstructionKind::LoadCallInputCount { dst }),
        "nargout" => Some(|dst| InstructionKind::LoadCallOutputCount { dst }),
        _ => None,
    }
}

fn zero_output_bare_command(name: &str) -> bool {
    matches!(
        name,
        "clc"
            | "close"
            | "cla"
            | "clf"
            | "hold"
            | "grid"
            | "box"
            | "rotate3d"
            | "shading"
            | "camlight"
            | "lighting"
            | "axis"
            | "format"
            | "who"
            | "whos"
            | "tic"
            | "toc"
    )
}

fn decode_quoted(literal: &str, delimiter: char) -> Option<String> {
    if literal.len() < 2 || !literal.starts_with(delimiter) || !literal.ends_with(delimiter) {
        return None;
    }
    let delimiter_len = delimiter.len_utf8();
    let inner = literal.get(delimiter_len..literal.len().checked_sub(delimiter_len)?)?;
    let mut decoded = String::with_capacity(inner.len());
    let mut characters = inner.chars().peekable();
    while let Some(character) = characters.next() {
        if character != delimiter {
            decoded.push(character);
            continue;
        }
        if characters.peek().copied() != Some(delimiter) {
            return None;
        }
        characters.next();
        decoded.push(delimiter);
    }
    Some(decoded)
}

fn exist_variable_name(arguments: &[Expr]) -> Option<String> {
    let [name, kind] = arguments else {
        return None;
    };
    let ExprKind::Char(name) = &name.kind else {
        return None;
    };
    let ExprKind::Char(kind) = &kind.kind else {
        return None;
    };
    decode_quoted(kind, '\'')?
        .eq_ignore_ascii_case("var")
        .then(|| decode_quoted(name, '\''))
        .flatten()
}

fn inferred_table_variable_names(arguments: &[Expr]) -> Option<Vec<String>> {
    let mut variable_count = arguments.len();
    while variable_count >= 2 {
        let ExprKind::Char(option) = &arguments[variable_count - 2].kind else {
            break;
        };
        let Some(option) = decode_quoted(option, '\'') else {
            break;
        };
        if option.eq_ignore_ascii_case("VariableNames") {
            return None;
        }
        if !option.eq_ignore_ascii_case("RowNames") {
            break;
        }
        variable_count -= 2;
    }
    Some(
        arguments[..variable_count]
            .iter()
            .enumerate()
            .map(|(index, argument)| match &argument.kind {
                ExprKind::Name(name) => name.clone(),
                _ => format!("Var{}", index + 1),
            })
            .collect(),
    )
}

fn malformed(
    diagnostics: &mut Vec<CompileDiagnostic>,
    source_id: SourceId,
    range: TextRange,
    detail: &str,
) {
    diagnostics.push(CompileDiagnostic::new(
        source_id,
        range,
        CompileDiagnosticKind::MalformedHir {
            detail: detail.to_owned(),
        },
    ));
}

fn resource_limit(
    diagnostics: &mut Vec<CompileDiagnostic>,
    source_id: SourceId,
    range: TextRange,
    resource: CompilerResource,
) {
    diagnostics.push(CompileDiagnostic::new(
        source_id,
        range,
        CompileDiagnosticKind::ResourceLimit { resource },
    ));
}
