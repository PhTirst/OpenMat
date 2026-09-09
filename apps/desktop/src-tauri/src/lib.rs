use std::error::Error;
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, SyncSender};
use std::thread;
use std::time::Duration;

use serde::Serialize;
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

mod bundled_examples;
mod native_files;
mod runtime_settings;

const SERVER_START_TIMEOUT: Duration = Duration::from_secs(30);
const OEX_PLUGINS_ENVIRONMENT: &str = "OPENMAT_OEX_PLUGINS";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeConfig<'a> {
    kernel_web_socket_url: &'a str,
    platform: &'static str,
    native_file_api_version: u32,
    restore_workspace: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    workspace_identity: Option<String>,
}

fn workspace_root(app: &tauri::App) -> Result<PathBuf, Box<dyn Error>> {
    if let Some(configured) = std::env::var_os("OPENMAT_WORKSPACE_ROOT") {
        let configured = PathBuf::from(configured);
        fs::create_dir_all(&configured)?;
        return Ok(configured.canonicalize()?);
    }
    let root = app.path().document_dir()?.join("OpenMat");
    fs::create_dir_all(&root)?;
    let resources = app
        .path()
        .resolve("resources/examples", tauri::path::BaseDirectory::Resource)?;
    // The initial workspace contains runnable scripts immediately. Subsequent
    // frontend sessions may restore the folder the user selected themselves.
    match bundled_examples::install(&resources, &root) {
        Ok(examples) => Ok(examples.canonicalize()?),
        Err(error) => {
            if let Ok(log) = server_log(app) {
                append_log(
                    &log,
                    format!("Could not prepare bundled examples: {error}").as_bytes(),
                );
            }
            Ok(root.canonicalize()?)
        }
    }
}

fn openblas_dll(app: &tauri::App) -> Result<PathBuf, Box<dyn Error>> {
    let path = app.path().resolve(
        "resources/openblas/libopenblas.dll",
        tauri::path::BaseDirectory::Resource,
    )?;
    if !path.is_file() {
        return Err(format!("OpenBLAS runtime is missing: {}", path.display()).into());
    }
    Ok(path)
}

fn parse_oex_plugin_paths(configured: Option<OsString>) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let Some(configured) = configured else {
        return Ok(Vec::new());
    };
    let plugins = std::env::split_paths(&configured).collect::<Vec<_>>();
    if plugins.is_empty() || plugins.iter().any(|path| path.as_os_str().is_empty()) {
        return Err(format!(
            "{OEX_PLUGINS_ENVIRONMENT} must contain one or more non-empty library paths"
        )
        .into());
    }
    Ok(plugins)
}

fn oex_plugin_paths() -> Result<Vec<PathBuf>, Box<dyn Error>> {
    parse_oex_plugin_paths(std::env::var_os(OEX_PLUGINS_ENVIRONMENT))
}

fn server_log(app: &tauri::App) -> Result<PathBuf, Box<dyn Error>> {
    let directory = app.path().app_log_dir()?;
    fs::create_dir_all(&directory)?;
    Ok(directory.join("openmat-server.log"))
}

fn openblas_thread_count() -> String {
    if let Ok(configured) = std::env::var("OPENMAT_OPENBLAS_NUM_THREADS")
        && let Ok(threads) = configured.parse::<usize>()
        && (1..=64).contains(&threads)
    {
        return threads.to_string();
    }
    std::thread::available_parallelism()
        .map_or(1, usize::from)
        .min(8)
        .to_string()
}

fn append_log(path: &Path, bytes: &[u8]) {
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = file.write_all(bytes);
        if !bytes.ends_with(b"\n") {
            let _ = file.write_all(b"\n");
        }
    }
}

fn parse_kernel_url(bytes: &[u8]) -> Result<String, String> {
    let value = String::from_utf8_lossy(bytes).trim().to_owned();
    let Ok(url) = value.parse::<tauri::Url>() else {
        return Err(format!("server announced an invalid URL: {value}"));
    };
    if url.scheme() != "ws"
        || url.host_str() != Some("127.0.0.1")
        || url.path() != "/kernel"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(format!("server announced an unsupported URL: {value}"));
    }
    Ok(value)
}

struct AnnouncementWriter {
    sender: Option<SyncSender<Result<String, String>>>,
    bytes: Vec<u8>,
}

impl AnnouncementWriter {
    fn new(sender: SyncSender<Result<String, String>>) -> Self {
        Self {
            sender: Some(sender),
            bytes: Vec::new(),
        }
    }

    fn publish_error(&mut self, message: String) {
        if let Some(sender) = self.sender.take() {
            let _ = sender.send(Err(message));
        }
    }

    fn announced(&self) -> bool {
        self.sender.is_none()
    }
}

impl io::Write for AnnouncementWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Some(sender) = self.sender.take() {
            let _ = sender.send(parse_kernel_url(&self.bytes));
        }
        Ok(())
    }
}

fn create_main_window(
    app: &tauri::AppHandle,
    kernel_url: &str,
    workspace: &Path,
) -> Result<(), Box<dyn Error>> {
    let explicit_workspace = std::env::var_os("OPENMAT_WORKSPACE_ROOT").is_some();
    let config = serde_json::to_string(&RuntimeConfig {
        kernel_web_socket_url: kernel_url,
        platform: "desktop",
        native_file_api_version: 1,
        restore_workspace: !explicit_workspace,
        workspace_identity: explicit_workspace.then(|| workspace.to_string_lossy().into_owned()),
    })?;
    let initialization_script = format!(
        "Object.defineProperty(window, '__OPENMAT_RUNTIME__', {{ value: Object.freeze({config}), configurable: false }});"
    );
    WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("OpenMat")
        .inner_size(1440.0, 900.0)
        .min_inner_size(960.0, 640.0)
        .initialization_script(initialization_script)
        .build()?;
    Ok(())
}

fn report_startup_failure(message: &str, log_path: &Path) {
    let detail = format!(
        "OpenMat could not start its local kernel.\n\n{message}\n\nLog: {}",
        log_path.display()
    );
    append_log(log_path, detail.as_bytes());
    show_error_dialog("OpenMat startup failed", &detail);
}

#[cfg(windows)]
fn show_error_dialog(title: &str, message: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    let title = wide(title);
    let message = wide(message);
    // SAFETY: Both strings are NUL-terminated and remain alive for the call.
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            message.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

#[cfg(not(windows))]
fn show_error_dialog(title: &str, message: &str) {
    eprintln!("{title}: {message}");
}

fn start_server(app: &mut tauri::App) -> Result<(), Box<dyn Error>> {
    let settings_path = match std::env::var_os(runtime_settings::CONFIG_ENVIRONMENT) {
        Some(path) if !path.is_empty() => std::path::absolute(PathBuf::from(path))?,
        Some(_) => return Err("OPENMAT_RUNTIME_CONFIG must be a nonempty file path".into()),
        None => app.path().app_config_dir()?.join("runtime.json"),
    };
    let settings = runtime_settings::load_or_create(&settings_path).map_err(|error| {
        format!(
            "Could not load runtime settings {}: {error}",
            settings_path.display()
        )
    })?;
    let test_port = match std::env::var(runtime_settings::TEST_PORT_ENVIRONMENT) {
        Ok(value) => Some(value),
        Err(std::env::VarError::NotPresent) => None,
        Err(error) => return Err(error.into()),
    };
    let listen_address = settings.listen_address(test_port.as_deref())?;
    let workspace = workspace_root(app)?;
    let openblas = openblas_dll(app)?;
    let log_path = server_log(app)?;
    let errors = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;
    let mut arguments = vec![
        "openmat-server".to_owned(),
        "--listen".to_owned(),
        listen_address.clone(),
        "--workspace-root".to_owned(),
        workspace.to_string_lossy().into_owned(),
        "--openblas-dll".to_owned(),
        openblas.to_string_lossy().into_owned(),
    ];
    for plugin in oex_plugin_paths()? {
        arguments.push("--oex-plugin".to_owned());
        arguments.push(plugin.to_string_lossy().into_owned());
    }
    let (sender, receiver) = mpsc::sync_channel(1);
    let handle = app.handle().clone();
    let thread_log_path = log_path.clone();
    thread::Builder::new()
        .name("openmat-in-process-server".to_owned())
        .spawn(move || {
            let mut announcement = AnnouncementWriter::new(sender);
            let mut errors = errors;
            let exit = openmat_server::run(
                arguments,
                io::Cursor::new(Vec::<u8>::new()),
                &mut announcement,
                &mut errors,
            );
            drop(errors);
            if announcement.announced() {
                report_startup_failure(
                    &format!("the in-process OpenMat server stopped with code {exit}"),
                    &thread_log_path,
                );
                handle.exit(1);
            } else {
                announcement.publish_error(format!(
                    "the in-process OpenMat server exited during startup with code {exit}"
                ));
            }
        })?;

    let kernel_url = receiver
        .recv_timeout(SERVER_START_TIMEOUT)
        .map_err(|error| {
            format!("the in-process OpenMat server did not become ready: {error}")
        })?
        .map_err(|error| format!(
            "Could not start the kernel at {listen_address}: {error}.\nCheck that the configured port is available.\nSettings: {}\nLog: {}",
            settings_path.display(), log_path.display(),
        ))?;
    create_main_window(app.handle(), &kernel_url, &workspace)?;
    Ok(())
}

fn configure_openblas_threads() {
    let threads = openblas_thread_count();
    // SAFETY: `run` calls this before Tauri or OpenBLAS starts any worker
    // threads, so no concurrent environment access exists within OpenMat.
    unsafe {
        std::env::set_var("OPENBLAS_NUM_THREADS", threads);
    }
}

/// Starts the desktop runtime and enters the application event loop.
///
/// # Panics
///
/// Panics if the desktop application cannot be built or initialized.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    configure_openblas_threads();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            native_files::desktop_pick_directory,
            native_files::desktop_pick_file,
            native_files::desktop_reveal_path,
            native_files::desktop_save_file,
        ])
        .setup(|app| {
            if let Err(error) = start_server(app) {
                show_error_dialog(
                    "OpenMat startup failed",
                    &format!("OpenMat could not initialize its desktop runtime.\n\n{error}"),
                );
                return Err(error);
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build OpenMat desktop application")
        .run(|_app, _event| {});
}

#[cfg(test)]
mod tests {
    use super::{AnnouncementWriter, parse_kernel_url, parse_oex_plugin_paths};
    use std::io::Write as _;
    use std::path::PathBuf;
    use std::sync::mpsc;

    #[test]
    fn kernel_url_accepts_only_the_private_loopback_endpoint() {
        assert_eq!(
            parse_kernel_url(b"ws://127.0.0.1:42000/kernel\n").expect("valid URL"),
            "ws://127.0.0.1:42000/kernel"
        );
        assert!(parse_kernel_url(b"ws://localhost:42000/kernel").is_err());
        assert!(parse_kernel_url(b"ws://127.0.0.1:42000/lsp").is_err());
    }

    #[test]
    fn announcement_writer_publishes_one_validated_url_on_flush() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let mut writer = AnnouncementWriter::new(sender);
        writeln!(writer, "ws://127.0.0.1:42000/kernel").expect("write URL");
        writer.flush().expect("flush URL");

        assert_eq!(
            receiver.recv().expect("announcement").expect("valid URL"),
            "ws://127.0.0.1:42000/kernel"
        );
        assert!(writer.announced());
    }

    #[test]
    fn configured_oex_plugins_preserve_platform_path_order() {
        let configured =
            std::env::join_paths(["first.oex.dll", "second.oex.dll"]).expect("join plugin paths");
        assert_eq!(
            parse_oex_plugin_paths(Some(configured)).expect("plugin paths"),
            [
                PathBuf::from("first.oex.dll"),
                PathBuf::from("second.oex.dll")
            ]
        );
        assert!(parse_oex_plugin_paths(None).unwrap().is_empty());
    }
}
