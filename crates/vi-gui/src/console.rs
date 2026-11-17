//! Remote consoles (SPEC-018): which viewer to run and with which URL. The
//! worker runs it ([`crate::worker::Worker::open_console`]) with a tunnel
//! that lives as long as the viewer.

use std::ffi::{OsStr, OsString};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

/// The viewer used when `[console] viewer` is not set (virt-viewer).
pub const DEFAULT_VIEWER: &str = "remote-viewer";

/// One console: a graphics device (by its position in the domain XML) of a
/// domain on a host.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConsoleKey {
    pub host: String,
    pub domain: String,
    pub index: usize,
}

/// What the GUI shows for a console.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsoleState {
    /// Connecting, reading the live graphics, starting the viewer.
    Opening,
    /// The viewer runs; it connects to `url` (the local end of the tunnel).
    Open { url: String },
    /// Ended: the viewer exited or was closed. `error` if it failed.
    Closed { error: Option<String> },
}

/// The URL the viewer opens: `vnc://127.0.0.1:<port>` or `spice://…`.
pub fn viewer_url(kind: &str, local: SocketAddr) -> String {
    format!("{kind}://{local}")
}

/// How to get the viewer on this platform (shown when it is not found).
pub fn install_hint() -> &'static str {
    if cfg!(windows) {
        "install virt-viewer for Windows (the MSI from virt-manager.org)"
    } else if cfg!(target_os = "macos") {
        "install it with Homebrew: brew install virt-viewer"
    } else {
        "install the virt-viewer package (apt install virt-viewer, dnf install virt-viewer)"
    }
}

/// The viewer program: `[console] viewer` (a path, or a name searched like
/// the default), else `remote-viewer`. Searched on `PATH` plus the
/// directories of [`vi_backend::local::search_path`] (Homebrew) and, on
/// Windows, `Program Files\VirtViewer*\bin`.
pub fn find_viewer(configured: Option<&str>) -> Result<PathBuf, String> {
    find_viewer_in(
        configured,
        &vi_backend::local::search_path(),
        &windows_dirs(),
    )
}

/// [`find_viewer`] with an explicit search path (tests).
pub fn find_viewer_in(
    configured: Option<&str>,
    path: &OsStr,
    extra: &[PathBuf],
) -> Result<PathBuf, String> {
    let name = configured.map(str::trim).filter(|s| !s.is_empty());
    let name = name.unwrap_or(DEFAULT_VIEWER);
    let p = Path::new(name);
    if p.is_absolute() || p.components().count() > 1 {
        return if p.is_file() {
            Ok(p.to_path_buf())
        } else {
            Err(format!(
                "the console viewer {name} ([console] viewer) does not exist"
            ))
        };
    }
    let dirs = std::env::split_paths(path).chain(extra.iter().cloned());
    for dir in dirs {
        for file in candidates(name) {
            let c = dir.join(&file);
            if c.is_file() {
                return Ok(c);
            }
        }
    }
    Err(format!(
        "the console viewer \"{name}\" was not found; {}, or set [console] viewer to its path",
        install_hint()
    ))
}

/// File names for a program name: on Windows also with `.exe`.
fn candidates(name: &str) -> Vec<OsString> {
    let mut v = vec![OsString::from(name)];
    if cfg!(windows) && Path::new(name).extension().is_none() {
        v.push(OsString::from(format!("{name}.exe")));
    }
    v
}

/// `Program Files\VirtViewer v…\bin`, newest version first (Windows only).
fn windows_dirs() -> Vec<PathBuf> {
    if !cfg!(windows) {
        return Vec::new();
    }
    let mut dirs = Vec::new();
    for var in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"] {
        let Some(base) = std::env::var_os(var) else {
            continue;
        };
        dirs.extend(virt_viewer_dirs(Path::new(&base)));
    }
    dirs.dedup();
    dirs
}

/// `<base>\VirtViewer*\bin`, newest version first (`v11.0` before `v9.0`).
pub fn virt_viewer_dirs(base: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(base) else {
        return Vec::new();
    };
    let mut v: Vec<(Vec<u64>, PathBuf)> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.starts_with("VirtViewer")
                .then(|| (version_key(&name), e.path().join("bin")))
        })
        .collect();
    v.sort_by(|a, b| b.cmp(a));
    v.into_iter().map(|(_, p)| p).collect()
}

/// The numbers in a name, for version order: "VirtViewer v11.0-256" → [11, 0, 256].
fn version_key(name: &str) -> Vec<u64> {
    name.split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse().ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(p: &Path) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b"").unwrap();
    }

    // TC-105: the viewer is found on the search path, in extra directories,
    // by a configured path; a missing one gives the install hint.
    #[test]
    fn viewer_search() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a");
        let b = d.path().join("b");
        let exe = if cfg!(windows) {
            "remote-viewer.exe"
        } else {
            "remote-viewer"
        };
        touch(&b.join(exe));
        let path = std::env::join_paths([&a, &b]).unwrap();
        assert_eq!(find_viewer_in(None, &path, &[]).unwrap(), b.join(exe));

        let empty = std::env::join_paths([&a]).unwrap();
        let e = find_viewer_in(None, &empty, &[]).unwrap_err();
        assert!(
            e.contains("remote-viewer") && e.contains(install_hint()),
            "{e}"
        );
        assert_eq!(
            find_viewer_in(None, &empty, std::slice::from_ref(&b)).unwrap(),
            b.join(exe)
        );

        // A configured path is used as is; a configured name is searched.
        let own = d.path().join("own").join("my-viewer");
        touch(&own);
        let s = own.to_string_lossy();
        assert_eq!(find_viewer_in(Some(&s), &empty, &[]).unwrap(), own);
        let own_dir = std::env::join_paths([own.parent().unwrap()]).unwrap();
        assert_eq!(
            find_viewer_in(Some("my-viewer"), &own_dir, &[]).unwrap(),
            own
        );
        let e = find_viewer_in(Some("/no/such/viewer"), &empty, &[]).unwrap_err();
        assert!(e.contains("does not exist"), "{e}");
        // Blank means the default.
        assert_eq!(find_viewer_in(Some("  "), &path, &[]).unwrap(), b.join(exe));
    }

    // TC-105: Program Files\VirtViewer*\bin, newest first.
    #[test]
    fn virt_viewer_install_dirs() {
        let d = tempfile::tempdir().unwrap();
        for n in ["VirtViewer v9.0-256", "VirtViewer v11.0-256", "Other"] {
            std::fs::create_dir_all(d.path().join(n).join("bin")).unwrap();
        }
        let v = virt_viewer_dirs(d.path());
        assert_eq!(
            v,
            vec![
                d.path().join("VirtViewer v11.0-256").join("bin"),
                d.path().join("VirtViewer v9.0-256").join("bin"),
            ]
        );
    }

    // TC-100: the viewer URL points at the local end of the tunnel.
    #[test]
    fn urls() {
        let a: SocketAddr = "127.0.0.1:40123".parse().unwrap();
        assert_eq!(viewer_url("vnc", a), "vnc://127.0.0.1:40123");
        assert_eq!(viewer_url("spice", a), "spice://127.0.0.1:40123");
    }
}
