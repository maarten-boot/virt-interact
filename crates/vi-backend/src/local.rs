//! The local machine as a host (SPEC-020): the same allowlisted command
//! lines as over SSH (SPEC-011), run by the local POSIX shell. No login, no
//! host key, no passphrase.

use std::ffi::{OsStr, OsString};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use vi_core::model::ConsoleTarget;
use vi_ssh::prompt::BoxFuture;
use vi_ssh::{CommandOutput, CommandRunner, ConsoleStream, SshError};

/// Directories searched for `virsh` (and the console viewer) after `PATH`.
/// An app started from the macOS Finder or Dock gets a minimal `PATH`
/// (`/usr/bin:/bin:/usr/sbin:/sbin`) without Homebrew's or MacPorts'
/// directories (crosscheck 2026-11-14-02, C1; MacPorts: Plan 6.3).
pub const EXTRA_DIRS: &[&str] = &[
    "/opt/homebrew/bin",
    "/usr/local/bin",
    "/opt/local/bin",
    "/usr/bin",
    "/bin",
];

/// The search path for `virsh`: `path` (normally `$PATH`), then each of
/// [`EXTRA_DIRS`] that is not in it yet. [`find_virsh`] and [`LocalRunner`]
/// both use it, so detection and execution always agree.
pub fn search_path_from(path: Option<&OsStr>) -> OsString {
    let mut dirs: Vec<PathBuf> = path
        .map(|p| std::env::split_paths(p).collect())
        .unwrap_or_default();
    for d in EXTRA_DIRS {
        let d = PathBuf::from(d);
        if !dirs.contains(&d) {
            dirs.push(d);
        }
    }
    // Only fails for a directory containing the separator; such an entry
    // could not have come from a PATH, so fall back to the extra dirs.
    std::env::join_paths(&dirs).unwrap_or_else(|_| {
        std::env::join_paths(EXTRA_DIRS).expect("constant directories are valid")
    })
}

/// [`search_path_from`] with the current `$PATH`.
pub fn search_path() -> OsString {
    search_path_from(std::env::var_os("PATH").as_deref())
}

/// Runs command lines with `sh -c`, each with a time limit.
pub struct LocalRunner {
    timeout: Duration,
    /// `PATH` for the shell: [`search_path`] by default (tests point it at
    /// a fake `virsh`).
    path: OsString,
}

impl LocalRunner {
    pub fn new(timeout: Duration) -> LocalRunner {
        LocalRunner {
            timeout,
            path: search_path(),
        }
    }

    pub fn with_path(mut self, path: impl Into<OsString>) -> LocalRunner {
        self.path = path.into();
        self
    }

    /// Local hosts need a POSIX shell (Linux, macOS, BSD).
    pub fn supported() -> bool {
        cfg!(unix)
    }
}

impl CommandRunner for LocalRunner {
    fn run<'a>(&'a self, command: &'a str) -> BoxFuture<'a, Result<CommandOutput, SshError>> {
        Box::pin(async move {
            if !LocalRunner::supported() {
                return Err(SshError::Connect {
                    addr: "local shell".into(),
                    source: std::io::Error::other(
                        "local hosts need a POSIX shell, which this platform does not have",
                    ),
                });
            }
            tracing::debug!(%command, "local exec");
            let mut c = tokio::process::Command::new("sh");
            c.arg("-c")
                .arg(command)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);
            c.env("PATH", &self.path);
            match tokio::time::timeout(self.timeout, c.output()).await {
                Err(_) => Err(SshError::Timeout {
                    what: format!("running \"{command}\" locally"),
                    secs: self.timeout.as_secs(),
                }),
                Ok(Err(e)) => Err(SshError::Connect {
                    addr: "local shell".into(),
                    source: e,
                }),
                Ok(Ok(o)) => Ok(CommandOutput {
                    exit_status: o.status.code().map(|c| c as u32),
                    stdout: o.stdout,
                    stderr: o.stderr,
                }),
            }
        })
    }

    fn open_stream<'a>(
        &'a self,
        target: &'a ConsoleTarget,
        _origin: SocketAddr,
    ) -> BoxFuture<'a, Result<Box<dyn ConsoleStream>, SshError>> {
        Box::pin(async move {
            match tokio::time::timeout(self.timeout, LocalRunner::connect_console(target)).await {
                Ok(Ok(s)) => Ok(s),
                Ok(Err(e)) => Err(SshError::Forward {
                    target: target.to_string(),
                    reason: e.to_string(),
                }),
                Err(_) => Err(SshError::Timeout {
                    what: format!("connecting to the console at {target}"),
                    secs: self.timeout.as_secs(),
                }),
            }
        })
    }
}

impl LocalRunner {
    /// Connect straight to a console on this machine (SPEC-018: a local VM
    /// needs no SSH; the same local listener is used, so sockets work too).
    async fn connect_console(target: &ConsoleTarget) -> std::io::Result<Box<dyn ConsoleStream>> {
        match target {
            ConsoleTarget::Tcp { host, port } => {
                let s = tokio::net::TcpStream::connect((host.as_str(), *port)).await?;
                Ok(Box::new(s))
            }
            #[cfg(unix)]
            ConsoleTarget::Socket(path) => {
                let s = tokio::net::UnixStream::connect(path).await?;
                Ok(Box::new(s))
            }
            #[cfg(not(unix))]
            ConsoleTarget::Socket(_) => Err(std::io::Error::other(
                "unix sockets are not available on this platform",
            )),
        }
    }
}

/// `virsh` on the [`search_path`], if any (for `[local] enabled = "auto"`).
pub fn find_virsh() -> Option<PathBuf> {
    if !LocalRunner::supported() {
        return None;
    }
    find_virsh_in(&search_path())
}

/// `virsh` in a `PATH`-style list: the first executable regular file.
pub fn find_virsh_in(path: &OsStr) -> Option<PathBuf> {
    std::env::split_paths(path)
        .map(|d| d.join("virsh"))
        .find(|p| is_executable(p))
}

#[cfg(unix)]
fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    p.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(p: &Path) -> bool {
    p.is_file()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    // TC-103b: a local console is reached directly (TCP and unix socket);
    // nothing listening is a clear error naming the target.
    #[tokio::test]
    async fn local_console_streams() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let r = LocalRunner::new(Duration::from_secs(5));
        let origin: SocketAddr = "127.0.0.1:1".parse().unwrap();

        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut s, _) = l.accept().await.unwrap();
            s.write_all(b"RFB").await.unwrap();
        });
        let t = ConsoleTarget::Tcp {
            host: "127.0.0.1".into(),
            port,
        };
        let Ok(mut s) = r.open_stream(&t, origin).await else {
            panic!("tcp")
        };
        let mut buf = [0u8; 3];
        s.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"RFB");

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.sock");
        let l = tokio::net::UnixListener::bind(&path).unwrap();
        tokio::spawn(async move {
            let (mut s, _) = l.accept().await.unwrap();
            s.write_all(b"SPI").await.unwrap();
        });
        let t = ConsoleTarget::Socket(path.to_string_lossy().into_owned());
        let Ok(mut s) = r.open_stream(&t, origin).await else {
            panic!("socket")
        };
        s.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"SPI");

        let t = ConsoleTarget::Socket(dir.path().join("none.sock").to_string_lossy().into_owned());
        let e = match r.open_stream(&t, origin).await {
            Err(e) => e.to_string(),
            Ok(_) => panic!("connected to nothing"),
        };
        assert!(e.contains("none.sock"), "{e}");
    }

    // TC-111: exit status, stdout, stderr and the time limit.
    #[tokio::test]
    async fn runs_commands_locally() {
        let r = LocalRunner::new(Duration::from_secs(5));
        let o = r.run("printf out; printf err >&2; exit 3").await.unwrap();
        assert_eq!(o.exit_status, Some(3));
        assert_eq!(o.stdout_text(), "out");
        assert_eq!(o.stderr_text(), "err");
        let r = LocalRunner::new(Duration::from_millis(200));
        let e = r.run("sleep 5").await.unwrap_err();
        assert_eq!(e.kind(), vi_ssh::SshErrorKind::Timeout);
    }

    // TC-112
    #[test]
    fn finds_virsh_on_path() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        // Not executable: ignored.
        std::fs::write(a.join("virsh"), "#!/bin/sh\n").unwrap();
        let path = std::env::join_paths([&a, &b]).unwrap();
        assert_eq!(find_virsh_in(&path), None);
        let v = b.join("virsh");
        std::fs::write(&v, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&v, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(find_virsh_in(&path), Some(v));
    }

    // TC-112: the search path adds the well-known directories (macOS
    // Finder/Dock start with a minimal PATH), keeps PATH first, no doubles.
    #[test]
    fn search_path_adds_well_known_dirs() {
        let sp = search_path_from(Some(OsStr::new("/x/bin:/usr/bin")));
        let dirs: Vec<PathBuf> = std::env::split_paths(&sp).collect();
        let want: Vec<PathBuf> = [
            "/x/bin",
            "/usr/bin",
            "/opt/homebrew/bin",
            "/usr/local/bin",
            "/opt/local/bin",
            "/bin",
        ]
        .iter()
        .map(PathBuf::from)
        .collect();
        assert_eq!(dirs, want);
        let none: Vec<PathBuf> = std::env::split_paths(&search_path_from(None)).collect();
        assert_eq!(none.len(), EXTRA_DIRS.len());
    }

    // TC-112: the runner uses the same search path as detection.
    #[tokio::test]
    async fn runner_uses_search_path() {
        let r = LocalRunner::new(Duration::from_secs(5));
        let o = r.run("printf %s \"$PATH\"").await.unwrap();
        assert_eq!(o.stdout_text(), search_path().to_string_lossy());
    }
}
