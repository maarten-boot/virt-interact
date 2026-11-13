//! Platform directories (SPEC-004, SPEC-014, SPEC-016) and command-line
//! options (`--config`, TC-025).

use std::path::PathBuf;

use crate::APP_NAME;

/// Default config file path.
///
/// - Linux: `$XDG_CONFIG_HOME/virt-interact/config.toml` (default `~/.config/...`)
/// - Windows: `%APPDATA%\virt-interact\config\config.toml`
/// - macOS: `~/Library/Application Support/virt-interact/config.toml`
pub fn default_config_path() -> Option<PathBuf> {
    let base = dirs::config_dir()?.join(APP_NAME);
    let dir = if cfg!(windows) {
        base.join("config")
    } else {
        base
    };
    Some(dir.join("config.toml"))
}

/// Directory for the inventory cache (SPEC-014).
pub fn cache_dir() -> Option<PathBuf> {
    Some(dirs::cache_dir()?.join(APP_NAME))
}

/// Directory for log files (SPEC-016).
pub fn log_dir() -> Option<PathBuf> {
    Some(dirs::data_local_dir()?.join(APP_NAME).join("logs"))
}

/// Parsed command line.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CliArgs {
    /// `--config <path>` override.
    pub config: Option<PathBuf>,
    pub show_help: bool,
    pub show_version: bool,
}

pub const USAGE: &str = "\
Usage: virt-interact [OPTIONS]

Options:
  --config <PATH>   Use this config file instead of the default location
  -h, --help        Show this help
  -V, --version     Show the version";

impl CliArgs {
    /// Parse arguments (without the program name).
    pub fn parse<I, S>(args: I) -> Result<CliArgs, String>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut out = CliArgs::default();
        let mut it = args.into_iter().map(Into::into);
        while let Some(a) = it.next() {
            match a.as_str() {
                "--config" => {
                    let v = it.next().ok_or("--config needs a path")?;
                    out.config = Some(PathBuf::from(v));
                }
                s if s.starts_with("--config=") => {
                    out.config = Some(PathBuf::from(&s["--config=".len()..]));
                }
                "-h" | "--help" => out.show_help = true,
                "-V" | "--version" => out.show_version = true,
                other => return Err(format!("unknown argument: {other}")),
            }
        }
        Ok(out)
    }

    /// The config path to use: the override, else the platform default.
    pub fn config_path(&self) -> Option<PathBuf> {
        self.config.clone().or_else(default_config_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // TC-025
    #[test]
    fn config_override() {
        let a = CliArgs::parse(["--config", "/tmp/x.toml"]).unwrap();
        assert_eq!(a.config_path(), Some(PathBuf::from("/tmp/x.toml")));
        let a = CliArgs::parse(["--config=/tmp/y.toml"]).unwrap();
        assert_eq!(a.config_path(), Some(PathBuf::from("/tmp/y.toml")));
    }

    #[test]
    fn default_path_ends_with_app_and_file() {
        let a = CliArgs::parse(Vec::<String>::new()).unwrap();
        if let Some(p) = a.config_path() {
            assert!(p.ends_with("config.toml"));
            assert!(p.components().any(|c| c.as_os_str() == APP_NAME));
        }
    }

    #[test]
    fn bad_arguments() {
        assert!(CliArgs::parse(["--config"]).is_err());
        assert!(CliArgs::parse(["--frobnicate"]).is_err());
        assert!(CliArgs::parse(["-h"]).unwrap().show_help);
        assert!(CliArgs::parse(["--version"]).unwrap().show_version);
    }
}
