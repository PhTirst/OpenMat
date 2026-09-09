use std::collections::BTreeSet;

use openmat_bytecode::BytecodeModule;
use openmat_hir::{ExprKind, StmtKind};
use openmat_source::SourceId;

const META_SOURCE_ID: SourceId = SourceId::new(u32::MAX);

#[derive(Debug)]
pub(crate) struct EvalProgram {
    pub(crate) module: BytecodeModule,
    pub(crate) output_names: Vec<String>,
}

#[derive(Debug)]
pub(crate) struct EvalCompileError {
    pub(crate) identifier: &'static str,
    pub(crate) message: String,
}

pub(crate) fn compile(
    source_id: SourceId,
    source: &str,
    requested_outputs: usize,
    occupied_names: &BTreeSet<&str>,
    inherited_imports: &[String],
) -> Result<EvalProgram, EvalCompileError> {
    let output_names = output_names(source, requested_outputs, occupied_names)?;
    let executable = if output_names.is_empty() {
        source.to_owned()
    } else {
        format!("[{}] = {source};", output_names.join(","))
    };

    let parsed = openmat_parser::parse(source_id, &executable);
    if let Some(diagnostic) = parsed.diagnostics.first() {
        return Err(EvalCompileError {
            identifier: "OpenMat:eval:ParseError",
            message: diagnostic.message.clone(),
        });
    }
    let lowered = openmat_hir::lower(&parsed.syntax);
    if let Some(diagnostic) = lowered.diagnostics.first() {
        return Err(EvalCompileError {
            identifier: "OpenMat:eval:HirError",
            message: diagnostic.message.clone(),
        });
    }
    if requested_outputs == 0
        && lowered
            .file
            .statements
            .iter()
            .any(|statement| matches!(statement.kind, StmtKind::Function(_) | StmtKind::Class(_)))
    {
        return Err(EvalCompileError {
            identifier: "OpenMat:eval:UnsupportedDefinition",
            message: "eval does not accept function or class definitions".to_owned(),
        });
    }
    let module = openmat_compiler::compile_with_imports(&lowered.file, inherited_imports).map_err(
        |error| {
            let first = error
                .diagnostics()
                .first()
                .map_or_else(|| error.to_string(), ToString::to_string);
            EvalCompileError {
                identifier: "OpenMat:eval:CompileError",
                message: first,
            }
        },
    )?;
    Ok(EvalProgram {
        module,
        output_names,
    })
}

pub(crate) fn is_variable_name(name: &str) -> bool {
    let mut characters = name.chars();
    if name.chars().count() > 63 || !characters.next().is_some_and(char::is_alphabetic) {
        return false;
    }
    if !characters.all(|character| character == '_' || character.is_alphanumeric()) {
        return false;
    }
    let parsed = openmat_parser::parse(META_SOURCE_ID, &format!("{name} = 0;"));
    if !parsed.diagnostics.is_empty() {
        return false;
    }
    let lowered = openmat_hir::lower(&parsed.syntax);
    matches!(
        lowered.file.statements.as_slice(),
        [openmat_hir::Stmt {
            kind: StmtKind::Assignment { target, .. },
            ..
        }] if matches!(&target.kind, ExprKind::Name(target) if target == name)
    ) && lowered.diagnostics.is_empty()
}

fn output_names(
    source: &str,
    requested_outputs: usize,
    occupied_names: &BTreeSet<&str>,
) -> Result<Vec<String>, EvalCompileError> {
    if requested_outputs == 0 {
        return Ok(Vec::new());
    }
    for salt in 0_u64.. {
        let prefix = format!("__openmat_eval_{salt}_output_");
        if source.contains(&prefix) {
            continue;
        }
        let names = (0..requested_outputs)
            .map(|index| format!("{prefix}{index}"))
            .collect::<Vec<_>>();
        if names
            .iter()
            .all(|name| !occupied_names.contains(name.as_str()))
        {
            return Ok(names);
        }
    }
    Err(EvalCompileError {
        identifier: "OpenMat:eval:ResourceLimit",
        message: "eval could not allocate private output bindings".to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use openmat_source::SourceId;

    use super::compile;

    const SOURCE: SourceId = SourceId::new(u32::MAX);

    #[test]
    fn output_expression_is_lowered_through_private_bindings() {
        let program =
            compile(SOURCE, "pair(1)", 2, &BTreeSet::new(), &[]).expect("valid expression");
        assert_eq!(program.output_names.len(), 2);
        assert_eq!(program.module.classes.len(), 0);
    }

    #[test]
    fn rejects_definitions_in_statement_mode() {
        let error = compile(
            SOURCE,
            "function y = f(); y = 1; end",
            0,
            &BTreeSet::new(),
            &[],
        )
        .expect_err("definitions are not eval statements");
        assert_eq!(error.identifier, "OpenMat:eval:UnsupportedDefinition");
    }

    #[test]
    fn validates_assignin_variable_names() {
        assert!(super::is_variable_name("alpha_2"));
        assert!(super::is_variable_name("变量2"));
        assert!(!super::is_variable_name("_private"));
        assert!(!super::is_variable_name("if"));
        assert!(!super::is_variable_name("two words"));
    }
}
