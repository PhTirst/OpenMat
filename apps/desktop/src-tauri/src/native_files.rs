//! Native dialogs are the boundary for choosing paths outside Current Folder.
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use serde::Serialize;
use tauri_plugin_dialog::DialogExt;

const MAX_SAVE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileLocation {
    path: String,
    directory: String,
    name: String,
}

fn location(path: &Path) -> Result<FileLocation, String> {
    let directory = path.parent().ok_or("The file has no parent directory")?;
    let name = path.file_name().ok_or("The file has no name")?;
    Ok(FileLocation {
        path: path.to_string_lossy().into_owned(),
        directory: directory.to_string_lossy().into_owned(),
        name: name.to_string_lossy().into_owned(),
    })
}

fn require_main(window: &tauri::WebviewWindow) -> Result<(), String> {
    if window.label() != "main" {
        return Err("Native file operations are available only in the desktop IDE".into());
    }
    Ok(())
}

fn valid_file_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty()
        || name == "."
        || name == ".."
        || name.contains(['/', '\\', ':', '\0'])
    {
        return Err("Expected a file name without a directory".into());
    }
    Ok(())
}

fn reveal_target(root: &Path, relative: &Path) -> Result<PathBuf, String> {
    if relative
        .components()
        .any(|part| !matches!(part, Component::Normal(_) | Component::CurDir))
    {
        return Err("Expected a path inside Current Folder".into());
    }
    let root = root.canonicalize().map_err(|error| error.to_string())?;
    let target = root
        .join(relative)
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if !target.starts_with(&root) {
        return Err("The selected path is outside Current Folder".into());
    }
    Ok(target)
}

fn save_atomically(path: &Path, contents: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("The file has no parent directory")?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|error| error.to_string())?;
    file.write_all(contents)
        .map_err(|error| error.to_string())?;
    file.as_file()
        .sync_all()
        .map_err(|error| error.to_string())?;
    file.persist(path).map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn desktop_pick_directory(
    window: tauri::WebviewWindow,
    initial_directory: Option<String>,
) -> Result<Option<String>, String> {
    require_main(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let mut dialog = window
            .dialog()
            .file()
            .set_parent(&window)
            .set_title("Open Folder");
        if let Some(directory) = initial_directory {
            dialog = dialog.set_directory(directory);
        }
        dialog
            .blocking_pick_folder()
            .map(|file| {
                file.into_path()
                    .map(|path| path.to_string_lossy().into_owned())
                    .map_err(|error| error.to_string())
            })
            .transpose()
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn desktop_pick_file(
    window: tauri::WebviewWindow,
    initial_directory: Option<String>,
) -> Result<Option<FileLocation>, String> {
    require_main(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let mut dialog = window
            .dialog()
            .file()
            .set_parent(&window)
            .set_title("Open File")
            .add_filter(
                "OpenMat source, apps and models",
                &["m", "omui", "omsim", "slx", "json"],
            )
            .add_filter("All files", &["*"]);
        if let Some(directory) = initial_directory {
            dialog = dialog.set_directory(directory);
        }
        dialog
            .blocking_pick_file()
            .map(|file| {
                let path = file.into_path().map_err(|error| error.to_string())?;
                location(&path)
            })
            .transpose()
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn desktop_reveal_path(
    window: tauri::WebviewWindow,
    root_path: String,
    relative_path: String,
) -> Result<(), String> {
    require_main(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let path = reveal_target(Path::new(&root_path), Path::new(&relative_path))?;
        if path.is_dir() {
            tauri_plugin_opener::open_path(path, None::<&str>).map_err(|error| error.to_string())
        } else {
            tauri_plugin_opener::reveal_item_in_dir(path).map_err(|error| error.to_string())
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn desktop_save_file(
    window: tauri::WebviewWindow,
    initial_directory: Option<String>,
    suggested_name: String,
    contents: Vec<u8>,
) -> Result<Option<FileLocation>, String> {
    require_main(&window)?;
    valid_file_name(&suggested_name)?;
    if contents.len() > MAX_SAVE_BYTES {
        return Err("Native exports are limited to 64 MiB".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let mut dialog = window
            .dialog()
            .file()
            .set_parent(&window)
            .set_title("Save As")
            .set_file_name(&suggested_name);
        if let Some(directory) = initial_directory {
            dialog = dialog.set_directory(directory);
        }
        let Some(file) = dialog.blocking_save_file() else {
            return Ok(None);
        };
        let path = file.into_path().map_err(|error| error.to_string())?;
        let result = location(&path)?;
        save_atomically(&path, &contents)?;
        Ok(Some(result))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacement_preserves_exact_bytes_and_leaves_no_temporary_files() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("计算 test.m");
        std::fs::write(&file, "old content").unwrap();
        save_atomically(&file, "x = '你好';\n".as_bytes()).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "x = '你好';\n");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
        let details = location(&file).unwrap();
        assert_eq!(details.name, "计算 test.m");
    }

    #[test]
    fn failed_save_does_not_remove_existing_target() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("existing directory");
        std::fs::create_dir(&destination).unwrap();
        assert!(save_atomically(&destination, b"new").is_err());
        assert!(destination.is_dir());
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn reveal_rejects_absolute_and_parent_paths() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("a.m"), "").unwrap();
        assert!(reveal_target(directory.path(), Path::new("a.m")).is_ok());
        assert!(reveal_target(directory.path(), Path::new("")).is_ok());
        assert!(reveal_target(directory.path(), Path::new("../elsewhere")).is_err());
        assert!(reveal_target(directory.path(), directory.path()).is_err());
    }

    #[test]
    fn suggested_names_cannot_contain_directories() {
        for invalid in ["", " ", ".", "..", "a/b.m", "a\\b.m", "C:script.m", "a\0"] {
            assert!(valid_file_name(invalid).is_err(), "{invalid:?}");
        }
        assert!(valid_file_name("计算 test.m").is_ok());
    }
}
