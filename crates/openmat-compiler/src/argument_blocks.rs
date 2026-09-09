//! Argument declarations lower to ordinary calls and branches. User validators
//! therefore retain normal name resolution, exception handling and heap frames.
use super::{
    BTreeMap, BTreeSet, BinaryOperator, BindingStorage, Constant, Expr, ExprKind, FunctionCompiler,
    FunctionDef, InstructionKind, LocalSlot, Register, Scope, StmtKind, TextRange,
    collect_expression_lexical_names,
};
use openmat_bytecode::{
    ApplyArgument, ArgumentLayout, AssignmentMode, FieldOperand, PlaceStep, ValueSource,
};
use openmat_hir::{ArgumentDeclaration, Name};

impl FunctionCompiler<'_, '_> {
    #[allow(clippy::too_many_lines)]
    pub(super) fn configure_arguments(&mut self, definition: &FunctionDef) {
        let mut layout = ArgumentLayout::default();
        let mut inputs = Vec::new();
        let mut option_names = BTreeSet::new();
        let mut output_names = BTreeSet::new();
        let mut saw_default = false;
        let mut saw_named = false;
        let mut saw_output = false;
        let mut saw_input = false;
        let mut saw_repeating = false;
        let mut saw_repeating_output = false;
        for statement in &definition.body {
            let StmtKind::Arguments(block) = &statement.kind else {
                break;
            };
            self.argument_blocks.push(statement.span);
            let mut output = false;
            let mut attributes = BTreeSet::new();
            for attribute in &block.attributes {
                let name = attribute
                    .name
                    .as_ref()
                    .map_or("", |name| name.text.as_str());
                if attribute.value.is_some()
                    || !attributes.insert(name)
                    || !matches!(name, "Input" | "Output" | "Repeating")
                {
                    self.malformed(
                        attribute.span,
                        "supported arguments attributes are Input, Output and Repeating",
                    );
                }
                output |= name == "Output";
            }
            if attributes.contains("Input") && output {
                self.malformed(
                    statement.span,
                    "an arguments block cannot be both Input and Output",
                );
            }
            let repeating = attributes.contains("Repeating");
            if repeating {
                let seen = if output {
                    &mut saw_repeating_output
                } else {
                    &mut saw_repeating
                };
                if *seen || block.declarations.is_empty() {
                    self.malformed(
                        statement.span,
                        "only one nonempty Repeating block is allowed per direction",
                    );
                }
                *seen = true;
                if !output && saw_named {
                    self.malformed(
                        statement.span,
                        "repeating inputs must precede name-value blocks",
                    );
                }
                if output
                    && (block.declarations.len() != 1
                        || definition.outputs.last().map(|n| &n.text)
                            != block.declarations.first().map(|d| &d.name.text))
                {
                    self.malformed(
                        statement.span,
                        "Repeating output must declare the final output only",
                    );
                }
            } else if output && saw_repeating_output {
                self.malformed(
                    statement.span,
                    "fixed output blocks must precede the Repeating output block",
                );
            }
            if !output && saw_output {
                self.malformed(statement.span, "input blocks must precede output blocks");
            }
            saw_output |= output;
            saw_input |= !output;
            for declaration in &block.declarations {
                let name = &declaration.name.text;
                if output {
                    let mut references = BTreeSet::new();
                    for validator in &declaration.validators {
                        collect_expression_lexical_names(
                            validator,
                            &BTreeSet::new(),
                            &mut references,
                        );
                    }
                    if definition
                        .outputs
                        .iter()
                        .any(|output| output.text != *name && references.contains(&output.text))
                    {
                        self.malformed(
                            declaration.span,
                            "output validators cannot reference other output arguments",
                        );
                    }
                    if declaration.default.is_some()
                        || name.contains('.')
                        || !definition.outputs.iter().any(|output| output.text == *name)
                        || !output_names.insert(name.clone())
                        || (name == "varargout" && !repeating)
                    {
                        self.malformed(declaration.span, "output validation requires a unique declared output without a default; varargout requires Repeating");
                    }
                    if repeating {
                        self.repeating_output = Some(declaration.clone());
                    } else {
                        self.output_validations.push(declaration.clone());
                    }
                    continue;
                }
                let (root, field) = name
                    .split_once('.')
                    .map_or((name.as_str(), None), |(root, field)| (root, Some(field)));
                if root == "~"
                    || (root == "varargin" && (!repeating || block.declarations.len() != 1))
                {
                    self.malformed(
                        declaration.span,
                        "varargin must be the only declaration in a Repeating block",
                    );
                }
                if let Some(field) = field {
                    if repeating {
                        self.malformed(
                            declaration.span,
                            "name-value fields cannot appear inside a Repeating block",
                        );
                    }
                    saw_named = true;
                    if field.contains('.')
                        || !option_names.insert(field.to_owned())
                        || inputs.iter().any(|input: &String| input == field)
                    {
                        self.malformed(
                            declaration.span,
                            "option names must be unique fields, distinct from positional inputs",
                        );
                    }
                    if inputs.last().is_none_or(|name| name != root) {
                        inputs.push(root.to_owned());
                    }
                    let Some(BindingStorage::Local(slot)) = self.binding_storage(root) else {
                        self.malformed(
                            declaration.span,
                            "option structure must be a function input",
                        );
                        continue;
                    };
                    layout.named.push((slot, field.to_owned()));
                } else if repeating {
                    if declaration.default.is_some() {
                        self.malformed(declaration.span, "repeating inputs cannot have defaults");
                    }
                    let mut references = BTreeSet::new();
                    for validator in &declaration.validators {
                        collect_expression_lexical_names(
                            validator,
                            &BTreeSet::new(),
                            &mut references,
                        );
                    }
                    if definition
                        .inputs
                        .iter()
                        .position(|n| n.text == *name)
                        .is_some_and(|position| {
                            definition.inputs[position + 1..]
                                .iter()
                                .any(|n| references.contains(&n.text))
                        })
                    {
                        self.malformed(declaration.span, "repeating validators can reference only the current or preceding inputs");
                    }
                    layout.repeating_count += 1;
                    inputs.push(root.to_owned());
                } else {
                    if saw_named || saw_repeating || (saw_default && declaration.default.is_none())
                    {
                        self.malformed(
                            declaration.span,
                            "required inputs must precede defaults, then name-value declarations",
                        );
                    }
                    saw_default |= declaration.default.is_some();
                    layout.positional_count += 1;
                    layout.required_count += u32::from(!saw_default);
                    inputs.push(root.to_owned());
                }
            }
        }
        if saw_input {
            if inputs
                != definition
                    .inputs
                    .iter()
                    .map(|name| name.text.clone())
                    .collect::<Vec<_>>()
            {
                self.malformed(
                    definition.span,
                    "arguments declarations must cover function inputs in signature order",
                );
            }
            // A validated varargin is a real cell-valued signature slot; the
            // legacy variadic marker is only used by functions without blocks.
            self.function.parameter_count =
                u32::try_from(definition.inputs.len()).unwrap_or(u32::MAX);
            self.function.argument_layout = Some(layout);
        }
    }

    pub(super) fn order_argument_fields(&mut self, span: TextRange) {
        let Some(layout) = &self.function.argument_layout else {
            return;
        };
        let mut groups = BTreeMap::<LocalSlot, Vec<String>>::new();
        for (slot, field) in &layout.named {
            groups.entry(*slot).or_default().push(field.clone());
        }
        for (slot, fields) in groups {
            let Scope::Function { bindings, .. } = &self.scope else {
                return;
            };
            let Some((root, _)) = bindings
                .iter()
                .find(|(_, storage)| **storage == BindingStorage::Local(slot))
            else {
                continue;
            };
            let root = name_expr(root, span);
            let order = Expr {
                kind: ExprKind::Cell(vec![
                    fields.iter().map(|field| text_expr(field, span)).collect(),
                ]),
                span,
            };
            self.lower_assignment(
                &root,
                &call(
                    "__openmat_order_argument_fields",
                    vec![root.clone(), order],
                    span,
                ),
                false,
            );
        }
    }

    /// Validate in call order (group, then declaration). During a group the
    /// current and preceding names denote elements, not their enclosing cells.
    /// Saved cells live in registers, hence remain visible to the normal GC.
    pub(super) fn lower_repeating_validation(
        &mut self,
        declarations: &[ArgumentDeclaration],
        input: bool,
    ) -> Option<()> {
        let span = declarations.first()?.span;
        let mut cells = Vec::with_capacity(declarations.len());
        for declaration in declarations {
            let name = &declaration.name.text;
            let storage = self.binding_storage(name)?;
            let cell = self.load_binding(name, storage, span, false)?;
            if !input {
                // Read the raw output binding: an unassigned repeating output
                // is an empty cell, but an assigned non-cell is invalid even
                // when the caller requests no outputs.
                let function_name =
                    self.add_constant(Constant::String("__openmat_repeating_output".into()), span)?;
                let target = self.allocate_register(span)?;
                self.emit(
                    InstructionKind::LoadFunctionHandleCandidates {
                        dst: target,
                        names: vec![function_name],
                    },
                    self.location(span),
                );
                self.emit(
                    InstructionKind::Apply {
                        outputs: vec![cell],
                        target,
                        arguments: vec![ApplyArgument::Value(cell)],
                    },
                    self.location(span),
                );
                self.store_binding(name, span, storage, cell);
            }
            cells.push((name.clone(), storage, cell));
        }
        let length = self.allocate_register(span)?;
        self.emit(
            InstructionKind::ResolveEnd {
                dst: length,
                target: cells[0].2,
                argument_index: 0,
                argument_count: 1,
            },
            self.location(span),
        );
        let index = self.load_constant(Constant::Double(1.0), span)?;
        let one = self.load_constant(Constant::Double(1.0), span)?;
        let start = self.current_instruction(span)?;
        let condition = self.allocate_register(span)?;
        self.emit(
            InstructionKind::Binary {
                operator: BinaryOperator::LessThanOrEqual,
                dst: condition,
                lhs: index,
                rhs: length,
            },
            self.location(span),
        );
        let exit = self.emit_jump_if_false(condition, span)?;
        for (declaration, (name, storage, cell)) in declarations.iter().zip(&cells) {
            let pack = self.allocate_pack_register(span)?;
            let element = self.allocate_register(span)?;
            self.emit(
                InstructionKind::BraceApply {
                    dst_pack: pack,
                    target: *cell,
                    arguments: vec![ApplyArgument::Value(index)],
                },
                self.location(span),
            );
            self.emit(
                InstructionKind::Unpack {
                    outputs: vec![element],
                    pack,
                },
                self.location(span),
            );
            self.store_binding(name, span, *storage, element);
            self.lower_argument_validation(declaration, input);
        }
        for (name, storage, cell) in &cells {
            let element = self.load_binding(name, *storage, span, false)?;
            self.emit(
                InstructionKind::AssignPlace {
                    dst: *cell,
                    root: *cell,
                    path: vec![PlaceStep::Brace(vec![ApplyArgument::Value(index)])],
                    source: ValueSource::One(element),
                    mode: AssignmentMode::Store,
                },
                self.location(span),
            );
            self.store_binding(name, span, *storage, *cell);
        }
        self.emit(
            InstructionKind::Binary {
                operator: BinaryOperator::Add,
                dst: index,
                lhs: index,
                rhs: one,
            },
            self.location(span),
        );
        self.emit(InstructionKind::Jump { target: start }, self.location(span));
        self.patch_to_current(exit, span);
        Some(())
    }

    #[allow(clippy::too_many_lines)]
    pub(super) fn lower_argument_validation(
        &mut self,
        declaration: &ArgumentDeclaration,
        input: bool,
    ) {
        let span = declaration.span;
        let value = argument_expr(&declaration.name);
        let field = declaration.name.text.split_once('.');
        let exists = if let Some((root, field)) = field {
            call(
                "__openmat_argument_isfield",
                vec![name_expr(root, span), text_expr(field, span)],
                span,
            )
        } else {
            call(
                "exist",
                vec![
                    text_expr(&declaration.name.text, span),
                    text_expr("var", span),
                ],
                span,
            )
        };
        if let Some(default) = &declaration.default {
            let condition = if field.is_none() {
                self.lower_exist_variable(&declaration.name.text, 1, span)
                    .and_then(|registers| registers.first().copied())
            } else {
                self.lower_expression(&exists)
            };
            let Some(condition) = condition else {
                return;
            };
            let condition = self.lower_not_register(condition, span);
            let Some(condition) = condition else {
                return;
            };
            let skip = self.emit_jump_if_false(condition, span);
            self.lower_assignment(&value, default, false);
            if let Some(skip) = skip {
                self.patch_to_current(skip, span);
            }
        }
        // Optional name-value declarations with no default have no field until
        // supplied; their validators are not evaluated when absent.
        let skip = if input && field.is_some() && declaration.default.is_none() {
            self.lower_expression(&exists)
                .and_then(|condition| self.emit_jump_if_false(condition, span))
        } else {
            None
        };
        if let Some(class) = &declaration.class {
            self.lower_argument_class(declaration, &value, class);
        }
        if !declaration.dimensions.is_empty() {
            if declaration.dimensions.len() < 2 {
                self.malformed(span, "argument sizes must declare at least two dimensions");
            }
            let mut dimensions = Vec::new();
            for dimension in &declaration.dimensions {
                let kind = match &dimension.kind {
                    ExprKind::AllIndex => ExprKind::Number("-1".into()),
                    ExprKind::Number(text) if matches!(super::parse_number(text), Some(Constant::Double(n)) if n.is_finite() && n>=0.0 && n.fract()==0.0) => {
                        dimension.kind.clone()
                    }
                    _ => {
                        self.malformed(
                            dimension.span,
                            "argument dimensions must be nonnegative integer literals or colon",
                        );
                        continue;
                    }
                };
                dimensions.push(Expr {
                    kind,
                    span: dimension.span,
                });
            }
            let sizes = Expr {
                kind: ExprKind::Matrix(vec![dimensions]),
                span,
            };
            self.lower_assignment(
                &value,
                &call(
                    "__openmat_validate_argument_size",
                    vec![value.clone(), sizes],
                    span,
                ),
                false,
            );
        }
        for validator in &declaration.validators {
            let (target, arguments) = match &validator.kind {
                ExprKind::Name(_) | ExprKind::Field { .. } => {
                    (validator.clone(), vec![value.clone()])
                }
                ExprKind::ParenApply { target, arguments } => {
                    (target.as_ref().clone(), arguments.clone())
                }
                _ => {
                    self.malformed(
                        validator.span,
                        "argument validator must name or call a function",
                    );
                    continue;
                }
            };
            self.lower_apply(&target, &arguments, 0, validator.span);
        }
        if let Some(skip) = skip {
            self.patch_to_current(skip, span);
        }
    }

    fn lower_argument_class(
        &mut self,
        declaration: &ArgumentDeclaration,
        value: &Expr,
        class: &Name,
    ) -> Option<()> {
        let span = declaration.span;
        let primitive = matches!(
            class.text.as_str(),
            "double"
                | "single"
                | "logical"
                | "char"
                | "string"
                | "cell"
                | "struct"
                | "table"
                | "function_handle"
                | "int8"
                | "uint8"
                | "int16"
                | "uint16"
                | "int32"
                | "uint32"
                | "int64"
                | "uint64"
        );
        let explicit = self
            .imports
            .iter()
            .find(|import| {
                import
                    .rsplit_once('.')
                    .is_some_and(|(_, leaf)| leaf == class.text)
            })
            .cloned();
        if !primitive
            && !class.text.contains('.')
            && explicit.is_none()
            && self.imports.iter().any(|import| import.ends_with(".*"))
        {
            self.malformed(class.span, "class constraints under wildcard imports currently require a fully qualified class name");
            return None;
        }
        let class_name = if primitive || class.text.contains('.') {
            class.text.clone()
        } else {
            explicit.unwrap_or_else(|| class.text.clone())
        };
        let names = vec![self.add_constant(Constant::String(class_name.clone()), span)?];
        let callee = self.allocate_register(span)?;
        // A statically named constructor remains visible to the source linker,
        // and does not accidentally resolve a same-named parameter as a value.
        let matches = self.lower_expression(&call(
            "__openmat_argument_isa",
            vec![value.clone(), text_expr(&class_name, span)],
            span,
        ))?;
        let needs_conversion = self.lower_not_register(matches, span)?;
        let skip = self.emit_jump_if_false(needs_conversion, span);
        self.emit(
            InstructionKind::LoadFunctionHandleCandidates { dst: callee, names },
            self.location(span),
        );
        let source = self.lower_expression(value)?;
        let converted = self.allocate_register(span)?;
        self.emit(
            InstructionKind::Apply {
                outputs: vec![converted],
                target: callee,
                arguments: vec![ApplyArgument::Value(source)],
            },
            self.location(span),
        );
        let (root, field) = declaration
            .name
            .text
            .split_once('.')
            .map_or((declaration.name.text.as_str(), None), |(root, field)| {
                (root, Some(field))
            });
        let storage = self.binding_storage(root)?;
        if let Some(field) = field {
            let binding = self.bytecode_binding_target(root, storage, span)?;
            let root = self.load_binding(root, storage, span, false)?;
            let field = self.add_constant(Constant::String(field.into()), span)?;
            self.emit(
                InstructionKind::AssignBindingPlace {
                    result: None,
                    binding,
                    root,
                    path: vec![PlaceStep::Field(FieldOperand::Static(field))],
                    source: ValueSource::One(converted),
                    mode: AssignmentMode::Store,
                },
                self.location(span),
            );
        } else {
            self.store_binding(root, span, storage, converted);
        }
        if let Some(skip) = skip {
            self.patch_to_current(skip, span);
        }
        Some(())
    }

    fn lower_not_register(&mut self, register: Register, span: TextRange) -> Option<Register> {
        let zero = self.load_constant(Constant::Double(0.0), span)?;
        let dst = self.allocate_register(span)?;
        self.emit(
            InstructionKind::Binary {
                operator: BinaryOperator::Equal,
                dst,
                lhs: register,
                rhs: zero,
            },
            self.location(span),
        );
        Some(dst)
    }
}

fn name_expr(name: &str, span: TextRange) -> Expr {
    Expr {
        kind: ExprKind::Name(name.into()),
        span,
    }
}
fn text_expr(text: &str, span: TextRange) -> Expr {
    Expr {
        kind: ExprKind::Char(format!("'{text}'")),
        span,
    }
}
fn call(name: &str, arguments: Vec<Expr>, span: TextRange) -> Expr {
    Expr {
        kind: ExprKind::ParenApply {
            target: Box::new(name_expr(name, span)),
            arguments,
        },
        span,
    }
}
fn argument_expr(name: &Name) -> Expr {
    let mut parts = name.text.split('.');
    let mut expression = name_expr(parts.next().unwrap_or(""), name.span);
    for field in parts {
        expression = Expr {
            kind: ExprKind::Field {
                target: Box::new(expression),
                name: Some(Name {
                    text: field.into(),
                    span: name.span,
                }),
            },
            span: name.span,
        };
    }
    expression
}
