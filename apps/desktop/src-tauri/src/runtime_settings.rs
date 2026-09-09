use std::error::Error;
use std::fs;
use std::io::Write as _;
use std::path::Path;

use serde::Deserialize;

pub const CONFIG_ENVIRONMENT: &str = "OPENMAT_RUNTIME_CONFIG";
pub const TEST_PORT_ENVIRONMENT: &str = "OPENMAT_DESKTOP_TEST_PORT";
const DEFAULT_SETTINGS: &str = include_str!("../../runtime.default.json");

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeSettings {
    schema_version: u32,
    kernel_port: u16,
}

impl RuntimeSettings {
    fn parse(source: &str) -> Result<Self, Box<dyn Error>> {
        let settings: Self = serde_json::from_str(source.trim_start_matches('\u{feff}'))?;
        if settings.schema_version != 1 {
            return Err("Unsupported runtime settings schemaVersion; expected 1".into());
        }
        if settings.kernel_port == 0 {
            return Err(
                "kernelPort must be between 1 and 65535; port 0 is reserved for explicit tests"
                    .into(),
            );
        }
        Ok(settings)
    }

    pub fn listen_address(&self, test_port: Option<&str>) -> Result<String, Box<dyn Error>> {
        let port = match test_port {
            Some(value) => value.trim().parse::<u16>().map_err(|_| {
                format!("{TEST_PORT_ENVIRONMENT} must be an integer between 0 and 65535")
            })?,
            None => self.kernel_port,
        };
        Ok(format!("127.0.0.1:{port}"))
    }
}

/// First launch creates defaults once. Existing settings are never overwritten.
pub fn load_or_create(path: &Path) -> Result<RuntimeSettings, Box<dyn Error>> {
    match fs::read_to_string(path) {
        Ok(source) => return RuntimeSettings::parse(&source),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let parent = path
        .parent()
        .ok_or("The runtime settings path has no parent directory")?;
    fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(DEFAULT_SETTINGS.as_bytes())?;
    temporary.as_file().sync_all()?;
    match temporary.persist_noclobber(path) {
        Ok(_) => {}
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.error.into()),
    }
    // Another instance may have created a configuration first; honor that file.
    RuntimeSettings::parse(&fs::read_to_string(path)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_launch_persists_default_and_later_launches_follow_user_edits() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("配置/runtime.json");
        let defaults = load_or_create(&path).unwrap();
        assert_eq!(defaults.listen_address(None).unwrap(), "127.0.0.1:42000");
        let edited = "{\"schemaVersion\":1,\"kernelPort\":42123}";
        fs::write(&path, edited).unwrap();
        for _ in 0..2 {
            assert_eq!(
                load_or_create(&path).unwrap().listen_address(None).unwrap(),
                "127.0.0.1:42123"
            );
            assert_eq!(fs::read_to_string(&path).unwrap(), edited);
        }
    }

    #[test]
    fn invalid_settings_are_rejected_and_preserved() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("runtime.json");
        for source in [
            "{",
            "{}",
            "{\"schemaVersion\":2,\"kernelPort\":42000}",
            "{\"schemaVersion\":1,\"kernelPort\":0}",
            "{\"schemaVersion\":1,\"kernelPort\":65536}",
            "{\"schemaVersion\":1,\"kernelPort\":-1}",
            "{\"schemaVersion\":1,\"kernelPort\":\"42000\"}",
            "{\"schemaVersion\":1,\"kernelPort\":42.5}",
            "{\"schemaVersion\":1,\"kernelPort\":42000,\"port\":42123}",
        ] {
            fs::write(&path, source).unwrap();
            assert!(load_or_create(&path).is_err(), "{source}");
            assert_eq!(fs::read_to_string(&path).unwrap(), source);
        }
    }

    #[test]
    fn only_explicit_test_override_can_select_an_ephemeral_port() {
        let settings = RuntimeSettings::parse(DEFAULT_SETTINGS).unwrap();
        assert_eq!(settings.listen_address(Some("0")).unwrap(), "127.0.0.1:0");
        assert_eq!(
            settings.listen_address(Some("42001")).unwrap(),
            "127.0.0.1:42001"
        );
        assert_eq!(settings.listen_address(None).unwrap(), "127.0.0.1:42000");
        for invalid in ["", "-1", "65536", "1.5", "invalid"] {
            assert!(settings.listen_address(Some(invalid)).is_err());
        }
    }

    #[test]
    fn accepts_config_saved_by_a_utf8_bom_editor() {
        assert!(RuntimeSettings::parse(&format!("\u{feff}{DEFAULT_SETTINGS}")).is_ok());
    }

    #[test]
    fn occupied_configured_port_fails_without_announcing_another_endpoint() {
        let occupied = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = occupied.local_addr().unwrap().port();
        let settings =
            RuntimeSettings::parse(&format!("{{\"schemaVersion\":1,\"kernelPort\":{port}}}"))
                .unwrap();
        let address = settings.listen_address(None).unwrap();
        let mut output = Vec::new();
        let mut errors = Vec::new();
        let exit = openmat_server::run(
            ["openmat-server", "--listen", &address],
            std::io::Cursor::new(Vec::<u8>::new()),
            &mut output,
            &mut errors,
        );
        assert_ne!(exit, 0);
        assert!(output.is_empty());
        assert!(!errors.is_empty());
    }
}
