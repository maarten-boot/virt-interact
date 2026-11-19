//! `known_certs`: accepted server certificates, by SHA-256 fingerprint,
//! per `host:port` (SPEC-023). One entry per line:
//!
//! ```text
//! esx.example.lan:443 SHA256:3f2a…(64 hex digits)
//! ```
//!
//! Lines starting with `#` and blank lines are ignored. The file is
//! created with mode 0600 on Unix.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::error::VsphereError;

#[derive(Debug, Clone)]
pub struct KnownCerts {
    path: PathBuf,
}

impl KnownCerts {
    pub fn new(path: impl Into<PathBuf>) -> KnownCerts {
        KnownCerts { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn err(&self, reason: impl ToString) -> VsphereError {
        VsphereError::KnownCerts {
            file: self.path.display().to_string(),
            reason: reason.to_string(),
        }
    }

    /// All entries; a missing file is empty. Unreadable lines are skipped
    /// with a warning.
    pub fn load(&self) -> Result<HashMap<String, String>, VsphereError> {
        let text = match std::fs::read_to_string(&self.path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(HashMap::new()),
            Err(e) => return Err(self.err(e)),
        };
        let mut map = HashMap::new();
        for (n, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            match line.split_whitespace().collect::<Vec<_>>()[..] {
                [target, fp] if parse_fingerprint(fp).is_some() => {
                    map.insert(target.to_string(), normalise(fp));
                }
                _ => {
                    tracing::warn!(file = %self.path.display(), line = n + 1, "known_certs: unreadable line skipped")
                }
            }
        }
        Ok(map)
    }

    /// The pinned fingerprint of `target` (`host:port`), if any.
    pub fn get(&self, target: &str) -> Result<Option<String>, VsphereError> {
        Ok(self.load()?.remove(target))
    }

    /// Pin `fingerprint` for `target`, replacing an older entry for it.
    pub fn add(&self, target: &str, fingerprint: &str) -> Result<(), VsphereError> {
        let old = match std::fs::read_to_string(&self.path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(self.err(e)),
        };
        let mut out: String = old
            .lines()
            .filter(|l| l.split_whitespace().next() != Some(target))
            .map(|l| format!("{l}\n"))
            .collect();
        out.push_str(&format!("{target} {}\n", normalise(fingerprint)));
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| self.err(e))?;
        }
        write_private(&self.path, out.as_bytes()).map_err(|e| self.err(e))
    }
}

#[cfg(unix)]
fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(data)?;
    // An existing file keeps its mode on open; make it private too.
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, data)
}

/// `SHA256:` plus 64 hex digits (case-insensitive), as raw bytes.
pub fn parse_fingerprint(s: &str) -> Option<[u8; 32]> {
    let hex = s
        .strip_prefix("SHA256:")
        .or_else(|| s.strip_prefix("sha256:"))?;
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

/// The text form of a SHA-256 digest: `SHA256:` and lowercase hex.
pub fn format_fingerprint(digest: &[u8]) -> String {
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("SHA256:{hex}")
}

fn normalise(fp: &str) -> String {
    parse_fingerprint(fp).map_or_else(|| fp.to_string(), |d| format_fingerprint(&d))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FP: &str = "SHA256:00112233445566778899AABBCCDDEEFF00112233445566778899aabbccddeeff";

    // TC-134 (file part): add, replace, read; 0600; bad lines skipped.
    #[test]
    fn add_and_get() {
        let d = tempfile::tempdir().unwrap();
        let k = KnownCerts::new(d.path().join("sub").join("known_certs"));
        assert_eq!(k.get("a:443").unwrap(), None);
        k.add("a:443", FP).unwrap();
        k.add("b:8443", FP).unwrap();
        let low = FP.to_lowercase().replace("sha256:", "SHA256:");
        assert_eq!(k.get("a:443").unwrap().as_deref(), Some(low.as_str()));
        let other = format!("SHA256:{}", "1".repeat(64));
        k.add("a:443", &other).unwrap();
        assert_eq!(k.get("a:443").unwrap(), Some(other));
        assert!(k.get("b:8443").unwrap().is_some());
        let text = std::fs::read_to_string(k.path()).unwrap();
        assert_eq!(text.lines().count(), 2, "{text}");
        std::fs::write(
            k.path(),
            format!("# comment\n\ngarbage\nc:443 {FP}\nd:443 SHA256:12\n"),
        )
        .unwrap();
        let m = k.load().unwrap();
        assert_eq!(m.len(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            k.add("e:443", FP).unwrap();
            let mode = std::fs::metadata(k.path()).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn fingerprints() {
        let d = parse_fingerprint(FP).unwrap();
        assert_eq!(d[0], 0x00);
        assert_eq!(d[10], 0xaa);
        assert_eq!(
            format_fingerprint(&d),
            FP.to_lowercase().replace("sha256:", "SHA256:")
        );
        assert!(parse_fingerprint("SHA256:zz").is_none());
        assert!(parse_fingerprint("MD5:00").is_none());
    }
}
