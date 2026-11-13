//! Logging to a file in the platform log directory (SPEC-016).
//!
//! Secrets (passphrases, key material) must never be passed to `tracing`
//! macros; the SSH layer is responsible for that (TC-039).

use std::io;
use std::path::Path;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;

/// Log file name prefix; files rotate daily: `virt-interact.log.YYYY-MM-DD`.
pub const LOG_FILE_PREFIX: &str = "virt-interact.log";

/// Our own crates; they log at the configured level, everything else at `warn`.
const OWN_CRATES: [&str; 4] = ["virt_interact", "vi_core", "vi_ssh", "vi_backend"];

/// Filter directives for `level`, e.g. `warn,virt_interact=info,vi_core=info,...`.
pub fn directives(level: &str) -> String {
    let mut d = String::from("warn");
    for c in OWN_CRATES {
        d.push_str(&format!(",{c}={level}"));
    }
    d
}

/// Build the filter: `RUST_LOG` wins over the configured level.
pub fn filter(level: &str) -> EnvFilter {
    EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(directives(level)))
}

/// Create a non-blocking daily-rotating file writer in `dir`.
/// Keep the returned guard alive for as long as logging is needed; dropping
/// it flushes the file.
pub fn file_writer(
    dir: &Path,
) -> io::Result<(tracing_appender::non_blocking::NonBlocking, WorkerGuard)> {
    std::fs::create_dir_all(dir)?;
    let appender = tracing_appender::rolling::daily(dir, LOG_FILE_PREFIX);
    Ok(tracing_appender::non_blocking(appender))
}

/// Install the global subscriber: file in `dir` at `level`, plus stderr for
/// warnings and errors.
pub fn init(dir: &Path, level: &str) -> io::Result<WorkerGuard> {
    use tracing_subscriber::fmt;
    use tracing_subscriber::prelude::*;

    let (writer, guard) = file_writer(dir)?;
    tracing_subscriber::registry()
        .with(
            fmt::layer()
                .with_writer(writer)
                .with_ansi(false)
                .with_filter(filter(level)),
        )
        .with(
            fmt::layer()
                .with_writer(io::stderr)
                .with_filter(EnvFilter::new("warn")),
        )
        .try_init()
        .map_err(io::Error::other)?;
    Ok(guard)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber::prelude::*;

    #[test]
    fn other_crates_only_warn() {
        let d = directives("debug");
        assert!(d.starts_with("warn,"));
        assert!(d.contains("vi_core=debug") && d.contains("virt_interact=debug"));
    }

    // TC-083
    #[test]
    fn writes_log_file_at_level() {
        let dir = tempfile::tempdir().unwrap();
        let (writer, guard) = file_writer(dir.path()).unwrap();
        let sub = tracing_subscriber::registry().with(
            tracing_subscriber::fmt::layer()
                .with_writer(writer)
                .with_ansi(false)
                .with_filter(EnvFilter::new("info")),
        );
        tracing::subscriber::with_default(sub, || {
            tracing::info!("hello-info");
            tracing::debug!("hello-debug");
        });
        drop(guard); // flush

        let files: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert_eq!(files.len(), 1, "{files:?}");
        let name = files[0].file_name().unwrap().to_string_lossy().to_string();
        assert!(name.starts_with(LOG_FILE_PREFIX), "{name}");
        let text = std::fs::read_to_string(&files[0]).unwrap();
        assert!(text.contains("hello-info"));
        assert!(!text.contains("hello-debug"));
    }
}
