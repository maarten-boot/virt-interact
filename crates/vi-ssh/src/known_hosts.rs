//! `known_hosts` checking and learning (SPEC-008).
//!
//! Follows OpenSSH semantics:
//! - Host field: comma list of names or patterns (`*`, `?`, `!negation`),
//!   or hashed entries `|1|salt|hmac-sha1`. Non-22 ports are written as
//!   `[host]:port`.
//! - `@revoked` lines with the presented key reject it; `@cert-authority`
//!   lines are ignored (certificates are not supported).
//! - Known key → OK. Same key type but different key → mismatch.
//!   Only other key types known → unknown (with a note).
//! - Unparsable lines are skipped with a warning; they never fail the check.

use std::fs::{self, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use ring::hmac;
use ring::rand::{SecureRandom, SystemRandom};
use russh::keys::{HashAlg, PublicKey, parse_public_key_base64};

use crate::sshconfig::patterns_match;

/// Result of checking a server key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostKeyStatus {
    /// The key is recorded for this host.
    Known,
    /// No key of this type is recorded. `other_types` lists key types that
    /// are recorded for the host (normally empty).
    Unknown { other_types: Vec<String> },
    /// A different key of the same type is recorded (possible attack).
    Mismatch { line: usize },
    /// The key is marked `@revoked`.
    Revoked { line: usize },
}

/// The host field as written in known_hosts: `host` or `[host]:port`.
pub fn host_field(host: &str, port: u16) -> String {
    if port == 22 {
        host.to_lowercase()
    } else {
        format!("[{}]:{port}", host.to_lowercase())
    }
}

/// `SHA256:...` fingerprint as shown by `ssh-keygen -l`.
pub fn fingerprint(key: &PublicKey) -> String {
    key.fingerprint(HashAlg::Sha256).to_string()
}

/// Key type name, e.g. `ssh-ed25519`.
pub fn key_type(key: &PublicKey) -> String {
    key.algorithm().as_str().to_string()
}

fn hashed_match(entry: &str, host: &str) -> bool {
    let mut parts = entry.split('|').skip(2);
    let (Some(salt), Some(hash)) = (parts.next(), parts.next()) else {
        return false;
    };
    let (Ok(salt), Ok(hash)) = (B64.decode(salt), B64.decode(hash)) else {
        return false;
    };
    let key = hmac::Key::new(hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, &salt);
    hmac::verify(&key, host.as_bytes(), &hash).is_ok()
}

fn hosts_match(field: &str, host: &str) -> bool {
    let mut plain = Vec::new();
    for entry in field.split(',') {
        if entry.starts_with("|1|") {
            if hashed_match(entry, host) {
                return true;
            }
        } else {
            plain.push(entry.to_lowercase());
        }
    }
    !plain.is_empty() && patterns_match(&plain, host)
}

/// Check `key` for `host`:`port` against the text of a known_hosts file.
/// Returns the status and warnings about skipped lines.
pub fn check_text(
    text: &str,
    host: &str,
    port: u16,
    key: &PublicKey,
) -> (HostKeyStatus, Vec<String>) {
    let host = host_field(host, port);
    let mut warnings = Vec::new();
    let mut known = false;
    let mut mismatch = None;
    let mut other_types = Vec::new();

    for (i, raw) in text.lines().enumerate() {
        let line = i + 1;
        let t = raw.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let mut fields = t.split_whitespace();
        let mut first = fields.next().unwrap_or_default();
        let marker = if first.starts_with('@') {
            let m = first;
            first = fields.next().unwrap_or_default();
            Some(m)
        } else {
            None
        };
        let (Some(_ty), Some(b64)) = (fields.next(), fields.next()) else {
            warnings.push(format!(
                "known_hosts line {line}: incomplete entry, skipped"
            ));
            continue;
        };
        if !hosts_match(first, &host) {
            continue;
        }
        let recorded = match parse_public_key_base64(b64) {
            Ok(k) => k,
            Err(_) => {
                warnings.push(format!("known_hosts line {line}: unreadable key, skipped"));
                continue;
            }
        };
        let same = recorded.key_data() == key.key_data();
        match marker {
            Some("@revoked") if same => return (HostKeyStatus::Revoked { line }, warnings),
            Some("@revoked") | Some("@cert-authority") => continue,
            Some(m) => {
                warnings.push(format!(
                    "known_hosts line {line}: unknown marker {m}, skipped"
                ));
                continue;
            }
            None => {}
        }
        if same {
            known = true;
        } else if recorded.algorithm() == key.algorithm() {
            mismatch.get_or_insert(line);
        } else {
            let t = key_type(&recorded);
            if !other_types.contains(&t) {
                other_types.push(t);
            }
        }
    }
    // A matching line wins over a mismatching one (several keys of one type
    // may be listed, e.g. during key rotation).
    let status = if known {
        HostKeyStatus::Known
    } else if let Some(line) = mismatch {
        HostKeyStatus::Mismatch { line }
    } else {
        HostKeyStatus::Unknown { other_types }
    };
    (status, warnings)
}

/// Check against a file. A missing file means "unknown".
pub fn check_file(
    path: &Path,
    host: &str,
    port: u16,
    key: &PublicKey,
) -> io::Result<(HostKeyStatus, Vec<String>)> {
    match fs::read_to_string(path) {
        Ok(t) => Ok(check_text(&t, host, port, key)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok((
            HostKeyStatus::Unknown {
                other_types: Vec::new(),
            },
            Vec::new(),
        )),
        Err(e) => Err(e),
    }
}

/// The known_hosts line for a key: `host-field key-type base64`, with the
/// host field hashed (`|1|salt|hash`, like `HashKnownHosts yes`) if `hashed`.
pub fn entry_line(host: &str, port: u16, key: &PublicKey, hashed: bool) -> io::Result<String> {
    let field = host_field(host, port);
    let field = if hashed {
        let mut salt = [0u8; 20];
        SystemRandom::new()
            .fill(&mut salt)
            .map_err(|_| io::Error::other("no random source"))?;
        let k = hmac::Key::new(hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, &salt);
        let tag = hmac::sign(&k, field.as_bytes());
        format!("|1|{}|{}", B64.encode(salt), B64.encode(tag.as_ref()))
    } else {
        field
    };
    let openssh = key
        .to_openssh()
        .map_err(|e| io::Error::other(format!("cannot encode key: {e}")))?;
    // to_openssh gives "type base64 [comment]"; drop any comment.
    let mut parts = openssh.split_whitespace();
    let (ty, b64) = (
        parts.next().unwrap_or_default(),
        parts.next().unwrap_or_default(),
    );
    Ok(format!("{field} {ty} {b64}"))
}

/// Append an entry for `host`:`port` to `path`, creating the file (mode 0600
/// on Unix) and its directory if needed.
pub fn learn(path: &Path, host: &str, port: u16, key: &PublicKey, hashed: bool) -> io::Result<()> {
    let line = entry_line(host, port, key, hashed)?;
    if let Some(dir) = path.parent()
        && !dir.as_os_str().is_empty()
    {
        fs::create_dir_all(dir)?;
    }
    let mut opts = OpenOptions::new();
    opts.read(true).append(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path)?;
    let mut needs_newline = false;
    if f.seek(SeekFrom::End(-1)).is_ok() {
        let mut b = [0u8; 1];
        f.read_exact(&mut b)?;
        needs_newline = b[0] != b'\n';
    }
    if needs_newline {
        f.write_all(b"\n")?;
    }
    f.write_all(line.as_bytes())?;
    f.write_all(b"\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test-only public keys (from tests/fixtures/ssh).
    const ED_A: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIPppDA0rOSaOEHE++x1T7TWMnNGwBKOxH5dp0HYliXdT"; // server_ed25519
    const ED_B: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIB66dIvD+ro2JT6TNDRKOARojl+A52Vp4TDh8NwJp9p3"; // other_ed25519

    fn key(b64: &str) -> PublicKey {
        parse_public_key_base64(b64).unwrap()
    }

    // TC-033
    #[test]
    fn plain_entries() {
        let text = format!(
            "# comment\n\nother.lan ssh-ed25519 {ED_B}\nlab.lan,10.0.0.5 ssh-ed25519 {ED_A} a comment\n"
        );
        let (s, w) = check_text(&text, "lab.lan", 22, &key(ED_A));
        assert_eq!(s, HostKeyStatus::Known);
        assert!(w.is_empty());
        assert_eq!(
            check_text(&text, "LAB.lan", 22, &key(ED_A)).0,
            HostKeyStatus::Known
        );
        assert_eq!(
            check_text(&text, "10.0.0.5", 22, &key(ED_A)).0,
            HostKeyStatus::Known
        );
        // Same type, different key: mismatch with line number.
        assert_eq!(
            check_text(&text, "lab.lan", 22, &key(ED_B)).0,
            HostKeyStatus::Mismatch { line: 4 }
        );
        // Unknown host.
        assert_eq!(
            check_text(&text, "new.lan", 22, &key(ED_A)).0,
            HostKeyStatus::Unknown {
                other_types: vec![]
            }
        );
        // Port: [host]:port is a different host field.
        assert!(matches!(
            check_text(&text, "lab.lan", 2222, &key(ED_A)).0,
            HostKeyStatus::Unknown { .. }
        ));
        let text = format!("[lab.lan]:2222\tssh-ed25519   {ED_A}\n");
        assert_eq!(
            check_text(&text, "lab.lan", 2222, &key(ED_A)).0,
            HostKeyStatus::Known
        );
    }

    // TC-033: hashed entries (salt/hash computed with `ssh-keygen -H` semantics)
    #[test]
    fn hashed_entries() {
        let line = entry_line("lab.lan", 22, &key(ED_A), true).unwrap();
        assert!(line.starts_with("|1|"), "{line}");
        assert!(!line.contains("lab.lan"));
        assert_eq!(
            check_text(&line, "lab.lan", 22, &key(ED_A)).0,
            HostKeyStatus::Known
        );
        assert!(matches!(
            check_text(&line, "other.lan", 22, &key(ED_A)).0,
            HostKeyStatus::Unknown { .. }
        ));
        assert_eq!(
            check_text(&line, "lab.lan", 22, &key(ED_B)).0,
            HostKeyStatus::Mismatch { line: 1 }
        );
        // Entries hashed by OpenSSH `ssh-keygen -H` (interoperability).
        let ossh = include_str!("../../../tests/fixtures/ssh/openssh_hashed_known_hosts");
        assert_eq!(
            check_text(ossh, "lab.lan", 22, &key(ED_A)).0,
            HostKeyStatus::Known
        );
        assert_eq!(
            check_text(ossh, "lab.lan", 2222, &key(ED_A)).0,
            HostKeyStatus::Known
        );
        assert!(matches!(
            check_text(ossh, "lab.lan", 2200, &key(ED_A)).0,
            HostKeyStatus::Unknown { .. }
        ));
        assert_eq!(
            check_text(ossh, "lab.lan", 22, &key(ED_B)).0,
            HostKeyStatus::Mismatch { line: 1 }
        );
    }

    // TC-033: patterns, negation, markers, bad lines
    #[test]
    fn patterns_markers_and_bad_lines() {
        let text = format!(
            "*.lab.lan,!evil.lab.lan ssh-ed25519 {ED_A}\n\
             garbage\n\
             x.lan ssh-ed25519 not-base64!!\n\
             @cert-authority *.lan ssh-ed25519 {ED_B}\n"
        );
        let (s, w) = check_text(&text, "web.lab.lan", 22, &key(ED_A));
        assert_eq!(s, HostKeyStatus::Known);
        assert!(w.iter().any(|w| w.contains("line 2")), "{w:?}");
        assert!(matches!(
            check_text(&text, "evil.lab.lan", 22, &key(ED_A)).0,
            HostKeyStatus::Unknown { .. }
        ));
        let (_, w) = check_text(&text, "x.lan", 22, &key(ED_A));
        assert!(w.iter().any(|w| w.contains("line 3")), "{w:?}");

        let revoked = format!("lab.lan ssh-ed25519 {ED_A}\n@revoked * ssh-ed25519 {ED_A}\n");
        assert_eq!(
            check_text(&revoked, "lab.lan", 22, &key(ED_A)).0,
            HostKeyStatus::Revoked { line: 2 }
        );
    }

    #[test]
    fn known_wins_over_mismatch_of_same_type() {
        let text = format!("lab.lan ssh-ed25519 {ED_B}\nlab.lan ssh-ed25519 {ED_A}\n");
        assert_eq!(
            check_text(&text, "lab.lan", 22, &key(ED_A)).0,
            HostKeyStatus::Known
        );
    }

    // TC-034
    #[test]
    fn learn_appends_and_then_matches() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("known_hosts");
        assert!(matches!(
            check_file(&path, "lab.lan", 2222, &key(ED_A)).unwrap().0,
            HostKeyStatus::Unknown { .. }
        ));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, format!("other.lan ssh-ed25519 {ED_B}")).unwrap(); // no trailing newline
        learn(&path, "lab.lan", 2222, &key(ED_A), false).unwrap();
        learn(&path, "hashed.lan", 22, &key(ED_A), true).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(lines.len(), 3, "{text}");
        assert_eq!(lines[1], format!("[lab.lan]:2222 ssh-ed25519 {ED_A}"));
        assert!(lines[2].starts_with("|1|"));
        assert_eq!(
            check_file(&path, "lab.lan", 2222, &key(ED_A)).unwrap().0,
            HostKeyStatus::Known
        );
        assert_eq!(
            check_file(&path, "hashed.lan", 22, &key(ED_A)).unwrap().0,
            HostKeyStatus::Known
        );
        #[cfg(unix)]
        {
            let dir2 = tempfile::tempdir().unwrap();
            let p2 = dir2.path().join("kh");
            learn(&p2, "a", 22, &key(ED_A), true).unwrap();
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&p2).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn fingerprint_format() {
        let fp = fingerprint(&key(ED_A));
        assert!(fp.starts_with("SHA256:"), "{fp}");
        assert_eq!(key_type(&key(ED_A)), "ssh-ed25519");
    }
}
