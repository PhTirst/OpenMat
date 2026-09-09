use std::error::Error;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Deserialize)]
struct Catalog {
    examples: Vec<Example>,
}

#[derive(Deserialize)]
struct Example {
    file: String,
}

/// Copies the curated scripts into a writable, versioned workspace without
/// replacing user edits, even when two application instances start together.
pub fn install(resources: &Path, workspace: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let catalog_source = fs::read(resources.join("catalog.json"))?;
    let catalog: Catalog = serde_json::from_slice(&catalog_source)?;
    if catalog.examples.is_empty() {
        return Err("The bundled example catalog is empty".into());
    }
    let mut files = vec!["README.txt"];
    for example in &catalog.examples {
        let name = example.file.as_str();
        if !Path::new(name)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("m"))
            || name.contains(['/', '\\', ':'])
            || name.starts_with('.')
        {
            return Err(format!("Invalid bundled example filename: {name}").into());
        }
        files.push(name);
    }

    let destination = workspace.join("Examples").join(env!("CARGO_PKG_VERSION"));
    fs::create_dir_all(&destination)?;
    for name in files {
        let path = destination.join(name);
        if path.exists() {
            continue;
        }
        let source = fs::read(resources.join(name))?;
        let mut temporary = tempfile::NamedTempFile::new_in(&destination)?;
        temporary.write_all(&source)?;
        match temporary.persist_noclobber(path) {
            Ok(_) => {}
            Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.error.into()),
        }
    }
    Ok(destination)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(resources: &Path) {
        fs::create_dir_all(resources).unwrap();
        fs::write(
            resources.join("catalog.json"),
            r#"{"examples":[{"file":"START_HERE.m"},{"file":"fft_spectrum.m"}]}"#,
        )
        .unwrap();
        fs::write(resources.join("README.txt"), "Open a script and click Run.").unwrap();
        fs::write(resources.join("START_HERE.m"), "plot(1:3);").unwrap();
        fs::write(resources.join("fft_spectrum.m"), "fft([1 0 0 0]);").unwrap();
        // Files outside the curated catalog must never enter the workspace.
        fs::write(resources.join("UnfinishedApp.m"), "classdef UnfinishedApp").unwrap();
    }

    #[test]
    fn first_launch_copies_only_catalogued_examples_into_a_writable_version_folder() {
        let temporary = tempfile::tempdir().unwrap();
        let resources = temporary.path().join("安装目录/resources");
        let workspace = temporary.path().join("用户文档/OpenMat");
        fixture(&resources);
        let copied = install(&resources, &workspace).unwrap();
        assert_eq!(
            copied,
            workspace.join("Examples").join(env!("CARGO_PKG_VERSION"))
        );
        assert_eq!(
            fs::read_to_string(copied.join("START_HERE.m")).unwrap(),
            "plot(1:3);"
        );
        assert!(!copied.join("UnfinishedApp.m").exists());
        fs::write(copied.join("START_HERE.m"), "plot(3:-1:1);").unwrap();
    }

    #[test]
    fn later_launches_preserve_edits_and_old_versions_but_restore_missing_files() {
        let temporary = tempfile::tempdir().unwrap();
        let resources = temporary.path().join("resources");
        let workspace = temporary.path().join("workspace");
        fixture(&resources);
        let old = workspace.join("Examples/0.0.1");
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("START_HERE.m"), "old user work").unwrap();
        let copied = install(&resources, &workspace).unwrap();
        fs::write(copied.join("START_HERE.m"), "my edited example").unwrap();
        fs::remove_file(copied.join("fft_spectrum.m")).unwrap();
        install(&resources, &workspace).unwrap();
        assert_eq!(
            fs::read_to_string(copied.join("START_HERE.m")).unwrap(),
            "my edited example"
        );
        assert_eq!(
            fs::read_to_string(copied.join("fft_spectrum.m")).unwrap(),
            "fft([1 0 0 0]);"
        );
        assert_eq!(
            fs::read_to_string(old.join("START_HERE.m")).unwrap(),
            "old user work"
        );
    }

    #[test]
    fn catalog_cannot_copy_files_outside_the_example_directory() {
        let temporary = tempfile::tempdir().unwrap();
        let resources = temporary.path().join("resources");
        fixture(&resources);
        for name in ["../outside.m", "..\\outside.m", "C:outside.m", "app.omui"] {
            let catalog = serde_json::json!({"examples":[{"file":name}]}).to_string();
            fs::write(resources.join("catalog.json"), catalog).unwrap();
            assert!(install(&resources, &temporary.path().join("workspace")).is_err());
        }
    }
}
