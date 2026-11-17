//! Configuration file (SPEC-004) and host definitions (SPEC-005).
//!
//! A complete, tested example is in `examples/config.example.toml`.
//!
//! ```toml
//! [ui]
//! contrast = "AA"          # "AA" | "AAA" | "Max" | number >= 4.5
//!
//! [ssh]
//! connect_timeout_secs = 10
//! command_timeout_secs = 30
//!
//! [logging]
//! level = "info"
//!
//! [[hosts]]
//! name = "lab"
//! host = "lab.example.lan"
//! user = "ops"
//! # port, identity_file, backend, libvirt_uri, virsh_prefix are optional
//! ```

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::contrast::{ContrastLevel, RawContrast};

pub const DEFAULT_SSH_PORT: u16 = 22;
pub const DEFAULT_LIBVIRT_URI: &str = "qemu:///system";

/// The whole configuration file.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub ui: UiConfig,
    pub ssh: SshSettings,
    pub logging: LoggingConfig,
    /// The local machine, read without SSH (SPEC-020).
    pub local: LocalConfig,
    /// Remote consoles (SPEC-018).
    pub console: ConsoleConfig,
    pub hosts: Vec<HostConfig>,
}

/// `[console]`: the external viewer (SPEC-018).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ConsoleConfig {
    /// Viewer program: a path, or a name searched like `remote-viewer`
    /// (the default when unset).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub viewer: Option<String>,
}

/// When the local libvirt is shown (SPEC-020).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LocalMode {
    /// Shown when `virsh` is found on `PATH`.
    #[default]
    Auto,
    Always,
    Never,
}

/// `[local]`: the local libvirt, read without SSH (SPEC-020).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LocalConfig {
    pub enabled: LocalMode,
    /// URI of the "system" entry.
    pub libvirt_uri: String,
    /// Prefix for the "system" entry, e.g. `"sudo -n"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub virsh_prefix: Option<String>,
    /// Also show the user's own VMs (`qemu:///session`).
    pub session: bool,
}

impl Default for LocalConfig {
    fn default() -> Self {
        LocalConfig {
            enabled: LocalMode::Auto,
            libvirt_uri: DEFAULT_LIBVIRT_URI.into(),
            virsh_prefix: None,
            session: true,
        }
    }
}

/// Internal names of the local entries. Host names starting with `@` are
/// reserved for them.
pub const LOCAL_SYSTEM: &str = "@local-system";
pub const LOCAL_SESSION: &str = "@local-session";
pub const SESSION_URI: &str = "qemu:///session";

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UiConfig {
    /// Minimum contrast (SPEC-001).
    pub contrast: ContrastLevel,
    /// Refresh all hosts when the app starts (SPEC-012). Off by default:
    /// a refresh may ask for host keys or passphrases.
    pub refresh_on_start: bool,
}

/// Timeouts (SPEC-012).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SshSettings {
    pub connect_timeout_secs: u64,
    pub command_timeout_secs: u64,
}

impl Default for SshSettings {
    fn default() -> Self {
        SshSettings {
            connect_timeout_secs: 10,
            command_timeout_secs: 30,
        }
    }
}

/// Logging (SPEC-016).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LoggingConfig {
    /// `error`, `warn`, `info`, `debug` or `trace`.
    pub level: String,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        LoggingConfig {
            level: "info".into(),
        }
    }
}

/// Virtualisation backend (SPEC-017).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    #[default]
    Libvirt,
}

/// One remote host (SPEC-005).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostConfig {
    /// Display name, unique.
    pub name: String,
    /// Hostname, IP address or `~/.ssh/config` alias.
    pub host: String,
    /// SSH port; `None` means "from ssh config, else 22".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    /// Remote user; `None` means "from ssh config, else the local user".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    /// Private key path; `None` means "from ssh config / agent / default keys".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_file: Option<PathBuf>,
    #[serde(default)]
    pub backend: Backend,
    #[serde(default = "default_libvirt_uri")]
    pub libvirt_uri: String,
    /// Command prefix such as `"sudo -n"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub virsh_prefix: Option<String>,
    /// A local entry (SPEC-020): virsh runs on this machine, no SSH. Never
    /// read from or written to the file.
    #[serde(skip)]
    pub local: bool,
}

fn default_libvirt_uri() -> String {
    DEFAULT_LIBVIRT_URI.into()
}

impl HostConfig {
    /// A host with only the required fields; everything else default.
    pub fn new(name: impl Into<String>, host: impl Into<String>) -> HostConfig {
        HostConfig {
            name: name.into(),
            host: host.into(),
            port: None,
            user: None,
            identity_file: None,
            backend: Backend::default(),
            libvirt_uri: default_libvirt_uri(),
            virsh_prefix: None,
            local: false,
        }
    }

    /// Name shown in the GUI: the name, or for a local entry `localhost
    /// (system)`, `localhost (session)` or `localhost (<uri>)`.
    pub fn display_name(&self) -> String {
        if !self.local {
            return self.name.clone();
        }
        // From the URI, so `[local] libvirt_uri = "qemu:///session"` or
        // `lxc:///` is labelled as what it reads (crosscheck 02, C2).
        match self.libvirt_uri.as_str() {
            DEFAULT_LIBVIRT_URI => "localhost (system)".into(),
            SESSION_URI => "localhost (session)".into(),
            uri => format!("localhost ({uri})"),
        }
    }

    /// `identity_file` with a leading `~` expanded to the local home
    /// directory (SPEC-005). `~user` forms are not expanded.
    pub fn identity_path(&self) -> Option<PathBuf> {
        self.identity_file.as_deref().map(expand_tilde)
    }

    /// The port when no ssh config value applies.
    pub fn port_or_default(&self) -> u16 {
        self.port.unwrap_or(DEFAULT_SSH_PORT)
    }
}

/// Check a `virsh_prefix` (SPEC-011): plain words only, so it cannot chain
/// or redirect commands. Allowed: letters, digits, space and `_ - . / = : @ +`.
/// The error text completes "virsh_prefix ...".
pub fn check_virsh_prefix(prefix: &str) -> Result<(), String> {
    if prefix.trim().is_empty() {
        return Err("must not be empty (leave it out instead)".into());
    }
    if let Some(c) = prefix
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || " _-./=:@+".contains(*c)))
    {
        return Err(format!(
            "contains {c:?}; only letters, digits, spaces and _ - . / = : @ + are allowed"
        ));
    }
    Ok(())
}

/// Errors from loading, validating or saving the config.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("cannot read {path}: {source}")]
    Read { path: PathBuf, source: io::Error },
    #[error("{path}:{line}:{column}: {message}")]
    Parse {
        path: PathBuf,
        line: usize,
        column: usize,
        message: String,
    },
    #[error("{path}: {message}")]
    Invalid { path: PathBuf, message: String },
    #[error("cannot write {path}: {source}")]
    Write { path: PathBuf, source: io::Error },
    #[error("cannot serialise config: {0}")]
    Serialise(#[from] toml::ser::Error),
    #[error("{path} could not be loaded; refusing to overwrite it. Fix or remove the file first.")]
    ReadOnly { path: PathBuf },
}

/// Result of a successful load.
#[derive(Debug, Clone, PartialEq)]
pub struct Loaded {
    pub config: Config,
    /// `false` if the file does not exist yet (defaults are used).
    pub existed: bool,
    /// Non-fatal problems, e.g. a clamped contrast value (TC-004).
    pub warnings: Vec<String>,
}

impl Config {
    /// Parse config text. `path` is only used in messages.
    pub fn parse(text: &str, path: &Path) -> Result<(Config, Vec<String>), ConfigError> {
        let config: Config = toml::from_str(text).map_err(|e| {
            let (line, column) = e.span().map(|s| line_col(text, s.start)).unwrap_or((0, 0));
            ConfigError::Parse {
                path: path.to_path_buf(),
                line,
                column,
                message: e.message().trim().to_string(),
            }
        })?;

        // Re-read the raw contrast value to report clamping (TC-004).
        let mut warnings = Vec::new();
        if let Ok(table) = toml::from_str::<toml::Table>(text)
            && let Some(raw) = table
                .get("ui")
                .and_then(|u| u.get("contrast"))
                .cloned()
                .and_then(|v| v.try_into::<RawContrast>().ok())
            && let Ok((_, Some(w))) = raw.resolve()
        {
            warnings.push(w);
        }

        config.validate(path)?;
        Ok((config, warnings))
    }

    /// The local entries to show (SPEC-020). `virsh_found`: whether `virsh`
    /// is on `PATH` (used by `enabled = "auto"`).
    pub fn local_hosts(&self, virsh_found: bool) -> Vec<HostConfig> {
        let l = &self.local;
        let show = match l.enabled {
            LocalMode::Never => false,
            LocalMode::Always => true,
            LocalMode::Auto => virsh_found,
        };
        if !show {
            return Vec::new();
        }
        let mut sys = HostConfig::new(LOCAL_SYSTEM, "localhost");
        sys.local = true;
        sys.libvirt_uri = l.libvirt_uri.clone();
        sys.virsh_prefix = l.virsh_prefix.clone();
        let mut out = vec![sys];
        if l.session && l.libvirt_uri != SESSION_URI {
            let mut ses = HostConfig::new(LOCAL_SESSION, "localhost");
            ses.local = true;
            ses.libvirt_uri = SESSION_URI.into();
            out.push(ses);
        }
        out
    }

    /// Check rules that serde cannot express (TC-024).
    pub fn validate(&self, path: &Path) -> Result<(), ConfigError> {
        let invalid = |message: String| ConfigError::Invalid {
            path: path.to_path_buf(),
            message,
        };
        let mut seen = HashSet::new();
        for (i, h) in self.hosts.iter().enumerate() {
            if h.name.trim().is_empty() {
                return Err(invalid(format!("hosts[{i}]: name must not be empty")));
            }
            if h.name.starts_with('@') {
                return Err(invalid(format!(
                    "host \"{}\": names starting with '@' are reserved",
                    h.name
                )));
            }
            if h.host.trim().is_empty() {
                return Err(invalid(format!(
                    "host \"{}\": host must not be empty",
                    h.name
                )));
            }
            if h.port == Some(0) {
                return Err(invalid(format!(
                    "host \"{}\": port must be 1..=65535",
                    h.name
                )));
            }
            if let Some(prefix) = &h.virsh_prefix
                && let Err(why) = check_virsh_prefix(prefix)
            {
                return Err(invalid(format!("host \"{}\": virsh_prefix {why}", h.name)));
            }
            if h.libvirt_uri.trim().is_empty() {
                return Err(invalid(format!(
                    "host \"{}\": libvirt_uri must not be empty",
                    h.name
                )));
            }
            if !seen.insert(h.name.as_str()) {
                return Err(invalid(format!("duplicate host name \"{}\"", h.name)));
            }
        }
        if self.local.libvirt_uri.trim().is_empty() {
            return Err(invalid("local.libvirt_uri must not be empty".into()));
        }
        if let Some(prefix) = &self.local.virsh_prefix
            && let Err(why) = check_virsh_prefix(prefix)
        {
            return Err(invalid(format!("local.virsh_prefix {why}")));
        }
        const LEVELS: [&str; 5] = ["error", "warn", "info", "debug", "trace"];
        if !LEVELS.contains(&self.logging.level.to_ascii_lowercase().as_str()) {
            return Err(invalid(format!(
                "logging.level \"{}\" must be one of {}",
                self.logging.level,
                LEVELS.join(", ")
            )));
        }
        Ok(())
    }

    /// Load from `path`. A missing file gives defaults and is *not* created
    /// (TC-021).
    pub fn load(path: &Path) -> Result<Loaded, ConfigError> {
        match fs::read_to_string(path) {
            Ok(text) => {
                let (config, warnings) = Config::parse(&text, path)?;
                Ok(Loaded {
                    config,
                    existed: true,
                    warnings,
                })
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Loaded {
                config: Config::default(),
                existed: false,
                warnings: Vec::new(),
            }),
            Err(source) => Err(ConfigError::Read {
                path: path.to_path_buf(),
                source,
            }),
        }
    }

    /// Serialise to TOML text.
    pub fn to_toml(&self) -> Result<String, ConfigError> {
        Ok(toml::to_string_pretty(self)?)
    }

    /// Validate and write atomically (temp file + rename). Creates the parent
    /// directory if needed.
    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        self.validate(path)?;
        let text = self.to_toml()?;
        let werr = |source| ConfigError::Write {
            path: path.to_path_buf(),
            source,
        };
        if let Some(dir) = path.parent()
            && !dir.as_os_str().is_empty()
        {
            fs::create_dir_all(dir).map_err(werr)?;
        }
        let tmp = path.with_extension("toml.tmp");
        fs::write(&tmp, text).map_err(werr)?;
        fs::rename(&tmp, path).map_err(werr)
    }
}

/// The config together with where it lives and whether it may be written.
///
/// If the file exists but could not be loaded, the store is read-only so the
/// user's file is never overwritten (SPEC-004, TC-022).
#[derive(Debug, Clone)]
pub struct ConfigStore {
    pub path: PathBuf,
    pub config: Config,
    pub writable: bool,
}

impl ConfigStore {
    /// Load, falling back to defaults (read-only) on error. Returns the store
    /// plus messages to show/log: warnings, or the load error.
    pub fn open(path: PathBuf) -> (ConfigStore, Vec<String>, Option<ConfigError>) {
        match Config::load(&path) {
            Ok(l) => (
                ConfigStore {
                    path,
                    config: l.config,
                    writable: true,
                },
                l.warnings,
                None,
            ),
            Err(e) => (
                ConfigStore {
                    path,
                    config: Config::default(),
                    writable: false,
                },
                Vec::new(),
                Some(e),
            ),
        }
    }

    pub fn save(&self) -> Result<(), ConfigError> {
        if !self.writable {
            return Err(ConfigError::ReadOnly {
                path: self.path.clone(),
            });
        }
        self.config.save(&self.path)
    }
}

/// Expand a leading `~` (alone, or followed by `/` or `\\`) to the home
/// directory. Other paths are returned unchanged.
pub fn expand_tilde(path: &Path) -> PathBuf {
    let Some(s) = path.to_str() else {
        return path.to_path_buf();
    };
    let rest = if s == "~" {
        Some("")
    } else {
        s.strip_prefix("~/").or_else(|| s.strip_prefix("~\\"))
    };
    match (rest, dirs::home_dir()) {
        (Some(""), Some(home)) => home,
        (Some(rest), Some(home)) => home.join(rest),
        _ => path.to_path_buf(),
    }
}

/// 1-based line and column of a byte offset.
fn line_col(text: &str, offset: usize) -> (usize, usize) {
    let before = &text[..offset.min(text.len())];
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    (line, column)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWO_HOSTS: &str = r#"
[ui]
contrast = "AAA"

[[hosts]]
name = "lab"
host = "lab.example.lan"
user = "ops"
port = 2222

[[hosts]]
name = "edge"
host = "edge-alias"
identity_file = "/home/ops/.ssh/id_ed25519"
libvirt_uri = "qemu:///session"
virsh_prefix = "sudo -n"
"#;

    fn p() -> &'static Path {
        Path::new("test.toml")
    }

    // TC-020
    #[test]
    fn round_trip_two_hosts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let (cfg, w) = Config::parse(TWO_HOSTS, p()).unwrap();
        assert!(w.is_empty());
        assert_eq!(cfg.hosts.len(), 2);
        assert_eq!(cfg.ui.contrast, ContrastLevel::Aaa);
        cfg.save(&path).unwrap();
        let again = Config::load(&path).unwrap();
        assert!(again.existed);
        assert_eq!(again.config, cfg);
    }

    // TC-020: custom contrast survives a round trip as a number
    #[test]
    fn round_trip_custom_contrast() {
        let mut cfg = Config::default();
        cfg.ui.contrast = ContrastLevel::Custom(9.5);
        let text = cfg.to_toml().unwrap();
        assert!(text.contains("contrast = 9.5"), "{text}");
        let (back, _) = Config::parse(&text, p()).unwrap();
        assert_eq!(back, cfg);
    }

    // TC-021
    #[test]
    fn missing_file_gives_defaults_and_is_not_created() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("config.toml");
        let l = Config::load(&path).unwrap();
        assert!(!l.existed);
        assert_eq!(l.config, Config::default());
        assert!(!path.exists());
        assert!(!path.parent().unwrap().exists());

        // C5: the first save creates the missing directory and the file.
        let mut store = ConfigStore::open(path.clone()).0;
        store.config.hosts.push(HostConfig::new("a", "h"));
        store.save().unwrap();
        assert!(path.exists());
        assert_eq!(Config::load(&path).unwrap().config, store.config);
    }

    // TC-022
    #[test]
    fn invalid_toml_reports_line_and_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let bad = "[ui]\ncontrast = \"AA\"\n\n[[hosts]]\nname = \"x\"\nhost = \n";
        fs::write(&path, bad).unwrap();
        let (store, _, err) = ConfigStore::open(path.clone());
        match err {
            Some(ConfigError::Parse { line, .. }) => assert_eq!(line, 6),
            other => panic!("expected parse error, got {other:?}"),
        }
        assert!(!store.writable);
        assert!(matches!(store.save(), Err(ConfigError::ReadOnly { .. })));
        assert_eq!(fs::read_to_string(&path).unwrap(), bad);
    }

    // TC-022: unknown keys are reported with their line
    #[test]
    fn unknown_key_is_an_error_with_line() {
        let text = "[ui]\ncontrast = \"AA\"\ncolour = \"red\"\n";
        match Config::parse(text, p()) {
            Err(ConfigError::Parse { line, message, .. }) => {
                assert_eq!(line, 3);
                assert!(message.contains("colour"), "{message}");
            }
            other => panic!("expected parse error, got {other:?}"),
        }
    }

    // TC-023
    #[test]
    fn host_defaults() {
        let (cfg, _) = Config::parse("[[hosts]]\nname = \"a\"\nhost = \"h\"\n", p()).unwrap();
        let h = &cfg.hosts[0];
        assert_eq!(h.port, None);
        assert_eq!(h.port_or_default(), 22);
        assert_eq!(h.backend, Backend::Libvirt);
        assert_eq!(h.libvirt_uri, "qemu:///system");
        assert_eq!(h.virsh_prefix, None);
        assert_eq!(*h, HostConfig::new("a", "h"));
        assert_eq!(cfg.ssh, SshSettings::default());
        assert_eq!(cfg.ui.contrast, ContrastLevel::Aa);
        assert_eq!(cfg.logging.level, "info");
    }

    // TC-024
    #[test]
    fn duplicate_host_name_rejected() {
        let text =
            "[[hosts]]\nname = \"a\"\nhost = \"h1\"\n[[hosts]]\nname = \"a\"\nhost = \"h2\"\n";
        match Config::parse(text, p()) {
            Err(ConfigError::Invalid { message, .. }) => assert!(message.contains("duplicate")),
            other => panic!("expected invalid, got {other:?}"),
        }
    }

    // TC-024 (C4)
    #[test]
    fn port_zero_rejected() {
        let text = "[[hosts]]\nname = \"a\"\nhost = \"h\"\nport = 0\n";
        match Config::parse(text, p()) {
            Err(ConfigError::Invalid { message, .. }) => assert!(message.contains("port")),
            other => panic!("expected invalid, got {other:?}"),
        }
    }

    // TC-053 (config side)
    #[test]
    fn virsh_prefix_validated() {
        assert!(check_virsh_prefix("sudo -n").is_ok());
        assert!(check_virsh_prefix("sudo -n -u libvirt-admin").is_ok());
        for bad in [
            "sudo;rm", "a && b", "a | b", "$(x)", "`x`", "a > f", "a\nb", "'q'", "  ",
        ] {
            assert!(check_virsh_prefix(bad).is_err(), "{bad:?} accepted");
        }
        let text = "[[hosts]]\nname = \"a\"\nhost = \"h\"\nvirsh_prefix = \"sudo; rm\"\n";
        match Config::parse(text, p()) {
            Err(ConfigError::Invalid { message, .. }) => {
                assert!(message.contains("virsh_prefix"), "{message}")
            }
            other => panic!("expected invalid, got {other:?}"),
        }
    }

    // TC-110: [local] defaults, modes, reserved names.
    #[test]
    fn local_section() {
        let (cfg, _) = Config::parse("", p()).unwrap();
        assert_eq!(cfg.local, LocalConfig::default());
        assert!(cfg.local_hosts(false).is_empty(), "auto without virsh");
        let l = cfg.local_hosts(true);
        assert_eq!(l.len(), 2);
        assert!(l.iter().all(|h| h.local));
        assert_eq!(
            (l[0].name.as_str(), l[0].libvirt_uri.as_str()),
            (LOCAL_SYSTEM, "qemu:///system")
        );
        assert_eq!(
            (l[1].name.as_str(), l[1].libvirt_uri.as_str()),
            (LOCAL_SESSION, SESSION_URI)
        );
        assert_eq!(l[0].display_name(), "localhost (system)");
        assert_eq!(l[1].display_name(), "localhost (session)");

        let text = "[local]\nenabled = \"always\"\nsession = false\nvirsh_prefix = \"sudo -n\"\n";
        let (cfg, _) = Config::parse(text, p()).unwrap();
        let l = cfg.local_hosts(false);
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].virsh_prefix.as_deref(), Some("sudo -n"));
        let (cfg, _) = Config::parse("[local]\nenabled = \"never\"\n", p()).unwrap();
        assert!(cfg.local_hosts(true).is_empty());
        // The session URI as main URI gives one entry.
        let (cfg, _) = Config::parse("[local]\nlibvirt_uri = \"qemu:///session\"\n", p()).unwrap();
        let l = cfg.local_hosts(true);
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].display_name(), "localhost (session)");
        // Any other URI is shown as it is.
        let (cfg, _) = Config::parse("[local]\nlibvirt_uri = \"lxc:///\"\n", p()).unwrap();
        assert_eq!(
            cfg.local_hosts(true)[0].display_name(),
            "localhost (lxc:///)"
        );

        for bad in [
            "[local]\nenabled = \"sometimes\"\n",
            "[local]\nvirsh_prefix = \"a;b\"\n",
            "[local]\nlibvirt_uri = \"\"\n",
            "[[hosts]]\nname = \"@me\"\nhost = \"h\"\n",
        ] {
            assert!(Config::parse(bad, p()).is_err(), "{bad}");
        }
        // Local entries are never written to the file.
        let mut cfg = Config::default();
        cfg.hosts = cfg.local_hosts(true);
        assert!(!cfg.to_toml().unwrap().contains("local = "));
    }

    // TC-031a
    #[test]
    fn identity_file_tilde_expansion() {
        let home = dirs::home_dir().unwrap();
        let mut h = HostConfig::new("a", "h");
        assert_eq!(h.identity_path(), None);
        h.identity_file = Some(PathBuf::from("~/.ssh/id_ed25519"));
        assert_eq!(h.identity_path(), Some(home.join(".ssh/id_ed25519")));
        h.identity_file = Some(PathBuf::from("~"));
        assert_eq!(h.identity_path(), Some(home.clone()));
        h.identity_file = Some(PathBuf::from("/abs/key"));
        assert_eq!(h.identity_path(), Some(PathBuf::from("/abs/key")));
        h.identity_file = Some(PathBuf::from("~other/key"));
        assert_eq!(h.identity_path(), Some(PathBuf::from("~other/key")));
        h.identity_file = Some(PathBuf::from("keys/~/x"));
        assert_eq!(h.identity_path(), Some(PathBuf::from("keys/~/x")));
    }

    // TC-004 via the config file
    #[test]
    fn low_contrast_in_file_is_clamped_with_warning() {
        let (cfg, w) = Config::parse("[ui]\ncontrast = 3.0\n", p()).unwrap();
        assert_eq!(cfg.ui.contrast, ContrastLevel::Custom(4.5));
        assert_eq!(w.len(), 1);
        let (cfg, w) = Config::parse("[ui]\ncontrast = 3\n", p()).unwrap();
        assert_eq!(cfg.ui.contrast, ContrastLevel::Custom(4.5));
        assert_eq!(w.len(), 1);
    }

    // TC-005 via the config file
    #[test]
    fn contrast_values_in_file() {
        for (v, want) in [
            ("\"AA\"", ContrastLevel::Aa),
            ("\"AAA\"", ContrastLevel::Aaa),
            ("\"Max\"", ContrastLevel::Max),
            ("7.5", ContrastLevel::Custom(7.5)),
        ] {
            let (cfg, w) = Config::parse(&format!("[ui]\ncontrast = {v}\n"), p()).unwrap();
            assert_eq!(cfg.ui.contrast, want, "{v}");
            assert!(w.is_empty());
        }
        assert!(matches!(
            Config::parse("[ui]\ncontrast = \"high\"\n", p()),
            Err(ConfigError::Parse { line: 2, .. })
        ));
    }

    // TC-028: the example file in the repo must stay valid.
    #[test]
    fn example_config_parses() {
        let text = include_str!("../../../examples/config.example.toml");
        let (cfg, w) = Config::parse(text, Path::new("config.example.toml")).unwrap();
        assert!(w.is_empty());
        assert_eq!(cfg.hosts.len(), 2);
        assert_eq!(cfg.hosts[0], HostConfig::new("lab", "lab.example.lan"));
        assert_eq!(cfg.hosts[1].port, Some(2222));
        assert_eq!(cfg.hosts[1].virsh_prefix.as_deref(), Some("sudo -n"));
        assert!(!cfg.hosts[1].identity_path().unwrap().starts_with("~"));
        assert!(!cfg.ui.refresh_on_start);
        let (cfg, _) = Config::parse("[ui]\nrefresh_on_start = true\n", Path::new("c")).unwrap();
        assert!(cfg.ui.refresh_on_start);
        // [console] (SPEC-018): unset by default; a viewer path is kept.
        assert_eq!(cfg.console.viewer, None);
        let (cfg, _) = Config::parse(
            "[console]\nviewer = \"/opt/v/remote-viewer\"\n",
            Path::new("c"),
        )
        .unwrap();
        assert_eq!(cfg.console.viewer.as_deref(), Some("/opt/v/remote-viewer"));
    }

    // TC-028: every ```toml block in the README must parse.
    #[test]
    fn readme_config_examples_parse() {
        // Normalise line endings: a Windows checkout may have CRLF.
        let readme = include_str!("../../../README.md").replace("\r\n", "\n");
        let mut found = 0;
        let mut rest = readme.as_str();
        while let Some(start) = rest.find("```toml\n") {
            let body = &rest[start + 8..];
            let end = body.find("```").expect("unterminated toml block");
            Config::parse(&body[..end], Path::new("README.md")).unwrap();
            found += 1;
            rest = &body[end + 3..];
        }
        assert!(found >= 1, "no toml example in README");
    }

    // TC-084
    #[test]
    fn invalid_log_level_rejected() {
        assert!(matches!(
            Config::parse("[logging]\nlevel = \"loud\"\n", p()),
            Err(ConfigError::Invalid { .. })
        ));
    }
}
