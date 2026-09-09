use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

use openmat_bytecode::{BytecodeModule, FunctionId};
use openmat_source::SourceId;

use crate::FileSystemService;

#[derive(Clone, Debug)]
pub(crate) struct LoadedFunctionModule {
    pub(crate) module: Arc<BytecodeModule>,
    pub(crate) function: FunctionId,
    pub(crate) source_id: SourceId,
    pub(crate) source_name: String,
    pub(crate) source: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ModuleLoadErrorKind {
    FileSystem,
    Parse,
    Hir,
    Compile,
    NotFunction,
    SourceIdExhausted,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ModuleLoadError {
    pub(crate) kind: ModuleLoadErrorKind,
    pub(crate) message: String,
}

impl ModuleLoadError {
    pub(crate) const fn identifier(&self) -> &'static str {
        match self.kind {
            ModuleLoadErrorKind::FileSystem => "OpenMat:source:Read",
            ModuleLoadErrorKind::Parse => "OpenMat:source:ParseError",
            ModuleLoadErrorKind::Hir => "OpenMat:source:HirError",
            ModuleLoadErrorKind::Compile => "OpenMat:source:CompileError",
            ModuleLoadErrorKind::NotFunction => "OpenMat:source:NotFunction",
            ModuleLoadErrorKind::SourceIdExhausted => "OpenMat:source:SourceIdExhausted",
        }
    }
}

/// Resolves, compiles, and caches dynamically discovered MATLAB function files.
#[derive(Debug)]
pub(crate) struct ModuleLoader {
    functions: BTreeMap<PathBuf, LoadedFunctionModule>,
    next_source_id: Option<u32>,
}

impl ModuleLoader {
    pub(crate) fn new() -> Self {
        Self {
            functions: BTreeMap::new(),
            next_source_id: Some(u32::MAX),
        }
    }

    pub(crate) fn clear(&mut self) {
        self.functions.clear();
        self.next_source_id = Some(u32::MAX);
    }

    pub(crate) fn allocate_source_id(&mut self) -> Result<SourceId, ModuleLoadError> {
        let source_id = self.next_source_id.ok_or_else(|| ModuleLoadError {
            kind: ModuleLoadErrorKind::SourceIdExhausted,
            message: "dynamic source identifier registry is exhausted".to_owned(),
        })?;
        self.next_source_id = source_id.checked_sub(1);
        Ok(SourceId::new(source_id))
    }

    pub(crate) fn load_function(
        &mut self,
        file_system: &mut dyn FileSystemService,
        caller: Option<&str>,
        name: &str,
    ) -> Result<Option<LoadedFunctionModule>, ModuleLoadError> {
        let source = file_system
            .resolve_matlab_source(caller, name)
            .map_err(|error| ModuleLoadError {
                kind: ModuleLoadErrorKind::FileSystem,
                message: error.message,
            })?;
        let Some(source) = source else {
            return Ok(None);
        };
        if let Some(loaded) = self.functions.get(&source.path)
            && loaded.source == source.source
        {
            return Ok(Some(loaded.clone()));
        }

        let source_id = self.allocate_source_id()?;
        let parsed = openmat_parser::parse(source_id, &source.source);
        if let Some(diagnostic) = parsed.diagnostics.first() {
            return Err(ModuleLoadError {
                kind: ModuleLoadErrorKind::Parse,
                message: diagnostic.message.clone(),
            });
        }
        let lowered = openmat_hir::lower(&parsed.syntax);
        if let Some(diagnostic) = lowered.diagnostics.first() {
            return Err(ModuleLoadError {
                kind: ModuleLoadErrorKind::Hir,
                message: diagnostic.message.clone(),
            });
        }
        let module = openmat_compiler::compile(&lowered.file).map_err(|error| {
            let message = error
                .diagnostics()
                .first()
                .map_or_else(|| error.to_string(), ToString::to_string);
            ModuleLoadError {
                kind: ModuleLoadErrorKind::Compile,
                message,
            }
        })?;
        let primary_name = name.rsplit('.').next().unwrap_or(name);
        let function =
            module_function_named(&module, primary_name).ok_or_else(|| ModuleLoadError {
                kind: ModuleLoadErrorKind::NotFunction,
                message: format!(
                    "source file `{}` does not define primary function `{primary_name}`",
                    source.path.to_string_lossy()
                ),
            })?;
        let loaded = LoadedFunctionModule {
            module: Arc::new(module),
            function,
            source_id,
            source_name: source.path.to_string_lossy().into_owned(),
            source: source.source,
        };
        self.functions.insert(source.path, loaded.clone());
        Ok(Some(loaded))
    }
}

impl Default for ModuleLoader {
    fn default() -> Self {
        Self::new()
    }
}

fn module_function_named(module: &BytecodeModule, name: &str) -> Option<FunctionId> {
    module
        .functions
        .iter()
        .position(|function| function.name == name)
        .and_then(|index| u32::try_from(index).ok())
        .map(FunctionId::new)
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };

    use crate::LocalFileSystem;

    use super::*;

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "openmat-module-loader-{}-{}",
                std::process::id(),
                NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn cache_reuses_exact_source_and_recompiles_changed_content() {
        let directory = TestDirectory::new();
        let source_path = directory.0.join("target.m");
        fs::write(
            &source_path,
            "function value = target(input)\nvalue = input + 1;\nend\n",
        )
        .unwrap();
        let mut file_system = LocalFileSystem::new(&directory.0).unwrap();
        let mut loader = ModuleLoader::new();

        let first = loader
            .load_function(&mut file_system, None, "target")
            .unwrap()
            .unwrap();
        let cached = loader
            .load_function(&mut file_system, None, "target")
            .unwrap()
            .unwrap();
        assert_eq!(first.source_id, cached.source_id);
        assert!(Arc::ptr_eq(&first.module, &cached.module));

        fs::write(
            &source_path,
            "function value = target(input)\nvalue = input + 2;\nend\n",
        )
        .unwrap();
        let changed = loader
            .load_function(&mut file_system, None, "target")
            .unwrap()
            .unwrap();
        assert_ne!(first.source_id, changed.source_id);
        assert!(!Arc::ptr_eq(&first.module, &changed.module));
    }

    #[test]
    fn qualified_package_name_selects_the_leaf_primary_function() {
        let directory = TestDirectory::new();
        fs::create_dir(directory.0.join("+alpha")).unwrap();
        fs::write(
            directory.0.join("+alpha/twice.m"),
            "function value = twice(input)\nvalue = input * 2;\nend\n",
        )
        .unwrap();
        let mut file_system = LocalFileSystem::new(&directory.0).unwrap();
        let mut loader = ModuleLoader::new();

        let package_function = loader
            .load_function(&mut file_system, None, "alpha.twice")
            .unwrap()
            .expect("package function");
        assert_eq!(
            package_function.module.functions[package_function.function.get() as usize].name,
            "twice"
        );
    }
}
