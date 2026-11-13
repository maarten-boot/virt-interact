//! Inventory cache (SPEC-014): the last successful inventory of each host,
//! as JSON in the platform cache directory, one file per host.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::model::HostInventory;

/// Bumped when the stored format changes; other versions are ignored.
const FORMAT: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CachedInventory {
    format: u32,
    /// Host display name (SPEC-005).
    pub host: String,
    /// When the inventory was taken, seconds since the Unix epoch (UTC).
    pub taken_at: u64,
    pub inventory: HostInventory,
}

impl CachedInventory {
    pub fn new(host: &str, taken_at: u64, inventory: HostInventory) -> CachedInventory {
        CachedInventory {
            format: FORMAT,
            host: host.to_string(),
            taken_at,
            inventory,
        }
    }
}

/// The cache directory.
#[derive(Debug, Clone)]
pub struct Cache {
    dir: PathBuf,
}

impl Cache {
    pub fn new(dir: impl Into<PathBuf>) -> Cache {
        Cache { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// File for a host: a readable part plus a hash of the full name, so
    /// different names never share a file.
    pub fn path_for(&self, host: &str) -> PathBuf {
        let safe: String = host
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .take(40)
            .collect();
        self.dir.join(format!(
            "inventory-{safe}-{:016x}.json",
            fnv1a(host.as_bytes())
        ))
    }

    /// Save atomically (temp file + rename).
    pub fn save(&self, entry: &CachedInventory) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        let path = self.path_for(&entry.host);
        let tmp = path.with_extension("json.tmp");
        let json = serde_json::to_vec(entry).map_err(io::Error::other)?;
        fs::write(&tmp, json)?;
        fs::rename(&tmp, &path)
    }

    /// The cached inventory of `host`. A missing, unreadable or
    /// other-format file gives `None` (the cache is only a convenience).
    pub fn load(&self, host: &str) -> Option<CachedInventory> {
        let data = fs::read(self.path_for(host)).ok()?;
        match serde_json::from_slice::<CachedInventory>(&data) {
            Ok(c) if c.format == FORMAT && c.host == host => Some(c),
            Ok(_) => None,
            Err(e) => {
                tracing::warn!(host, error = %e, "ignoring unreadable cache file");
                None
            }
        }
    }

    pub fn remove(&self, host: &str) -> io::Result<()> {
        match fs::remove_file(self.path_for(host)) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    /// Remove every cache file (TC-072). Returns how many were removed.
    pub fn clear(&self) -> io::Result<usize> {
        let entries = match fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(e),
        };
        let mut n = 0;
        for e in entries {
            let p = e?.path();
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with("inventory-")
                && (name.ends_with(".json") || name.ends_with(".json.tmp"))
            {
                fs::remove_file(&p)?;
                n += 1;
            }
        }
        Ok(n)
    }
}

/// Seconds since the Unix epoch, now.
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// `YYYY-MM-DD HH:MM UTC`.
pub fn format_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02} UTC",
        rem / 3600,
        (rem % 3600) / 60
    )
}

/// Short age: `just now`, `5 min ago`, `3 h ago`, `2 days ago`.
pub fn format_age(then: u64, now: u64) -> String {
    let s = now.saturating_sub(then);
    match s {
        0..60 => "just now".into(),
        60..3600 => format!("{} min ago", s / 60),
        3600..86_400 => format!("{} h ago", s / 3600),
        _ => format!("{} days ago", s / 86_400),
    }
}

fn fnv1a(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Domain, DomainState};

    fn inv() -> HostInventory {
        let mut d = Domain::new("vm 1", DomainState::Running);
        d.uuid = Some("u".into());
        HostInventory {
            library: Some("libvirt 12.0.0".into()),
            hypervisor: None,
            domains: vec![d, Domain::new("vm2", DomainState::ShutOff)],
            warnings: vec!["w".into()],
        }
    }

    // TC-070
    #[test]
    fn round_trip_keeps_timestamp() {
        let dir = tempfile::tempdir().unwrap();
        let c = Cache::new(dir.path().join("cache"));
        let e = CachedInventory::new("lab/1", 1_700_000_000, inv());
        c.save(&e).unwrap();
        assert_eq!(c.load("lab/1"), Some(e));
        assert_eq!(c.load("other"), None);
        // Names that sanitise alike still get separate files.
        assert_ne!(c.path_for("a/b"), c.path_for("a_b"));
    }

    // TC-070: a damaged or foreign file is ignored, not an error.
    #[test]
    fn bad_files_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let c = Cache::new(dir.path());
        fs::write(c.path_for("h"), b"{not json").unwrap();
        assert_eq!(c.load("h"), None);
        let mut e = CachedInventory::new("h", 1, inv());
        e.format = 99;
        fs::write(c.path_for("h"), serde_json::to_vec(&e).unwrap()).unwrap();
        assert_eq!(c.load("h"), None);
    }

    // TC-072
    #[test]
    fn clear_removes_cache_files_only() {
        let dir = tempfile::tempdir().unwrap();
        let c = Cache::new(dir.path());
        c.save(&CachedInventory::new("a", 1, inv())).unwrap();
        c.save(&CachedInventory::new("b", 1, inv())).unwrap();
        fs::write(dir.path().join("keep.txt"), "x").unwrap();
        assert_eq!(c.clear().unwrap(), 2);
        assert_eq!(c.load("a"), None);
        assert!(dir.path().join("keep.txt").exists());
        assert_eq!(Cache::new(dir.path().join("missing")).clear().unwrap(), 0);
        c.save(&CachedInventory::new("a", 1, inv())).unwrap();
        c.remove("a").unwrap();
        c.remove("a").unwrap();
        assert_eq!(c.load("a"), None);
    }

    #[test]
    fn time_formatting() {
        assert_eq!(format_utc(0), "1970-01-01 00:00 UTC");
        assert_eq!(format_utc(1_700_000_000), "2023-11-14 22:13 UTC");
        assert_eq!(format_utc(951_782_400), "2000-02-29 00:00 UTC");
        assert_eq!(format_age(100, 110), "just now");
        assert_eq!(format_age(0, 300), "5 min ago");
        assert_eq!(format_age(0, 7200), "2 h ago");
        assert_eq!(format_age(0, 3 * 86_400), "3 days ago");
        assert_eq!(format_age(10, 0), "just now");
    }
}
