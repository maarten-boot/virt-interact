//! A subset of OpenSSH `~/.ssh/config` (SPEC-007) and resolution of the
//! effective connection parameters for a host (SPEC-005/006).
//!
//! Supported: `Host` blocks with `*`, `?` and `!` patterns (case-insensitive),
//! options before the first `Host` (apply to all hosts), `Key Value` and
//! `Key=Value`, double quotes, and the keywords `HostName`, `User`, `Port`,
//! `IdentityFile`. As in OpenSSH, the first value found for a keyword wins;
//! `IdentityFile` values accumulate.
//!
//! Not supported (a warning is produced): `Match` blocks (their options are
//! skipped), `Include`, `ProxyJump`, `ProxyCommand`.

use std::path::{Path, PathBuf};

use vi_core::config::{DEFAULT_SSH_PORT, HostConfig, expand_tilde};

/// Keywords that change how a connection is made but are not supported.
const UNSUPPORTED: [&str; 3] = ["proxyjump", "proxycommand", "include"];

#[derive(Debug, Clone, PartialEq)]
struct Entry {
    key: String, // lowercase
    args: Vec<String>,
    line: usize,
}

#[derive(Debug, Clone, PartialEq)]
enum BlockKind {
    /// Options before the first `Host`/`Match`.
    Global,
    Host(Vec<String>),
    Match,
}

#[derive(Debug, Clone, PartialEq)]
struct Block {
    kind: BlockKind,
    line: usize,
    entries: Vec<Entry>,
}

/// A parsed ssh config file.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SshConfig {
    blocks: Vec<Block>,
    /// Problems found while parsing (unparsable lines).
    pub parse_warnings: Vec<String>,
}

/// Values found for one host.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SshConfigValues {
    pub hostname: Option<String>,
    pub user: Option<String>,
    pub port: Option<u16>,
    /// Raw `IdentityFile` values (not yet expanded).
    pub identity_files: Vec<String>,
    pub warnings: Vec<String>,
}

impl SshConfig {
    /// Parse config text.
    pub fn parse(text: &str) -> SshConfig {
        let mut cfg = SshConfig {
            blocks: vec![Block {
                kind: BlockKind::Global,
                line: 0,
                entries: Vec::new(),
            }],
            parse_warnings: Vec::new(),
        };
        for (i, raw) in text.lines().enumerate() {
            let line = i + 1;
            let trimmed = raw.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let Some((key, args)) = split_line(trimmed) else {
                cfg.parse_warnings.push(format!(
                    "ssh config line {line}: cannot parse \"{trimmed}\""
                ));
                continue;
            };
            match key.as_str() {
                "host" => cfg.blocks.push(Block {
                    kind: BlockKind::Host(args.iter().map(|a| a.to_lowercase()).collect()),
                    line,
                    entries: Vec::new(),
                }),
                "match" => cfg.blocks.push(Block {
                    kind: BlockKind::Match,
                    line,
                    entries: Vec::new(),
                }),
                _ => cfg
                    .blocks
                    .last_mut()
                    .expect("global block")
                    .entries
                    .push(Entry { key, args, line }),
            }
        }
        cfg
    }

    /// Load from a file. A missing file is an empty config.
    pub fn load(path: &Path) -> std::io::Result<SshConfig> {
        match std::fs::read_to_string(path) {
            Ok(t) => Ok(SshConfig::parse(&t)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(SshConfig::default()),
            Err(e) => Err(e),
        }
    }

    /// Load `~/.ssh/config`.
    pub fn load_default() -> std::io::Result<SshConfig> {
        match dirs::home_dir() {
            Some(h) => SshConfig::load(&h.join(".ssh").join("config")),
            None => Ok(SshConfig::default()),
        }
    }

    /// Collect the values that apply to `host` (as typed by the user).
    pub fn lookup(&self, host: &str) -> SshConfigValues {
        let host_lc = host.to_lowercase();
        let mut v = SshConfigValues::default();
        for b in &self.blocks {
            let applies = match &b.kind {
                BlockKind::Global => true,
                BlockKind::Host(patterns) => patterns_match(patterns, &host_lc),
                BlockKind::Match => {
                    if !b.entries.is_empty() {
                        v.warnings.push(format!(
                            "ssh config line {}: Match blocks are not supported; their options are ignored",
                            b.line
                        ));
                    }
                    false
                }
            };
            if !applies {
                continue;
            }
            for e in &b.entries {
                let first = e.args.first().cloned();
                match e.key.as_str() {
                    "hostname" if v.hostname.is_none() => v.hostname = first,
                    "user" if v.user.is_none() => v.user = first,
                    "port" if v.port.is_none() => match first.as_deref().map(str::parse::<u16>) {
                        Some(Ok(p)) if p > 0 => v.port = Some(p),
                        _ => v.warnings.push(format!(
                            "ssh config line {}: invalid Port \"{}\"",
                            e.line,
                            e.args.join(" ")
                        )),
                    },
                    "identityfile" => {
                        if let Some(f) = first
                            && !f.eq_ignore_ascii_case("none")
                        {
                            v.identity_files.push(f);
                        }
                    }
                    k if UNSUPPORTED.contains(&k) => v.warnings.push(format!(
                        "ssh config line {}: {} is not supported and is ignored",
                        e.line,
                        original_case(k)
                    )),
                    _ => {}
                }
            }
        }
        v
    }
}

fn original_case(k: &str) -> &'static str {
    match k {
        "proxyjump" => "ProxyJump",
        "proxycommand" => "ProxyCommand",
        "include" => "Include",
        _ => "option",
    }
}

/// Split `Key Value...` or `Key=Value` into a lowercase key and arguments.
/// Double quotes group words.
fn split_line(line: &str) -> Option<(String, Vec<String>)> {
    let (key, rest) = match line.find(|c: char| c.is_whitespace() || c == '=') {
        Some(i) => (&line[..i], &line[i..]),
        None => return None,
    };
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('=').unwrap_or(rest).trim_start();
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut has = false;
    for c in rest.chars() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                has = true;
            }
            c if c.is_whitespace() && !in_quotes => {
                if has {
                    args.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            c => {
                cur.push(c);
                has = true;
            }
        }
    }
    if in_quotes {
        return None;
    }
    if has {
        args.push(cur);
    }
    if key.is_empty() || args.is_empty() {
        return None;
    }
    Some((key.to_lowercase(), args))
}

/// OpenSSH pattern-list semantics: matches if any positive pattern matches
/// and no negated pattern matches.
pub(crate) fn patterns_match(patterns: &[String], host: &str) -> bool {
    let mut positive = false;
    for p in patterns {
        if let Some(neg) = p.strip_prefix('!') {
            if glob(neg, host) {
                return false;
            }
        } else if glob(p, host) {
            positive = true;
        }
    }
    positive
}

/// Glob with `*` (any run) and `?` (one char).
pub(crate) fn glob(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// Effective connection parameters for one configured host.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedHost {
    /// Display name from the app config.
    pub name: String,
    /// The `host` value as configured (may be an ssh config alias).
    pub alias: String,
    /// The real host name to connect to.
    pub hostname: String,
    pub port: u16,
    pub user: String,
    /// Key files to try, in order (after the agent).
    pub identity_files: Vec<PathBuf>,
    /// `true` if `identity_files` came from the app or ssh config; `false`
    /// if they are the default key names.
    pub identity_explicit: bool,
    pub warnings: Vec<String>,
}

/// The default key files, in the order they are tried (SPEC-006).
pub fn default_identity_files(home: &Path) -> Vec<PathBuf> {
    ["id_ed25519", "id_ecdsa", "id_rsa"]
        .iter()
        .map(|f| home.join(".ssh").join(f))
        .collect()
}

/// The local user name, from the environment.
pub fn local_user() -> Option<String> {
    ["USER", "USERNAME", "LOGNAME"]
        .iter()
        .find_map(|v| std::env::var(v).ok().filter(|s| !s.is_empty()))
}

/// Resolve a host: app config > ssh config > defaults (SPEC-005, SPEC-007).
pub fn resolve(host: &HostConfig, ssh: &SshConfig, local_user: &str, home: &Path) -> ResolvedHost {
    let v = ssh.lookup(&host.host);
    let mut warnings = ssh.parse_warnings.clone();
    warnings.extend(v.warnings);

    let hostname = v.hostname.unwrap_or_else(|| host.host.clone());
    let port = host.port.or(v.port).unwrap_or(DEFAULT_SSH_PORT);
    let user = host
        .user
        .clone()
        .or(v.user)
        .unwrap_or_else(|| local_user.to_string());

    let tokens = |s: &str| {
        let s = s
            .replace("%%", "\u{0}")
            .replace("%d", &home.to_string_lossy())
            .replace("%u", local_user)
            .replace("%r", &user)
            .replace("%h", &hostname)
            .replace("%n", &host.host)
            .replace('\u{0}', "%");
        expand_tilde(Path::new(&s))
    };
    let mut identity_files: Vec<PathBuf> = Vec::new();
    if let Some(p) = host.identity_path() {
        identity_files.push(p);
    }
    for f in &v.identity_files {
        let p = tokens(f);
        if !identity_files.contains(&p) {
            identity_files.push(p);
        }
    }
    let identity_explicit = !identity_files.is_empty();
    if !identity_explicit {
        identity_files = default_identity_files(home);
    }

    ResolvedHost {
        name: host.name.clone(),
        alias: host.host.clone(),
        hostname,
        port,
        user,
        identity_files,
        identity_explicit,
        warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CFG: &str = r#"
# global
User globaluser

Host lab
    HostName lab.example.lan
    Port 2222
    IdentityFile ~/.ssh/lab_key

Host *.example.lan !bad.example.lan
    User ops
    IdentityFile=%d/.ssh/id_%h

Host edge
  HostName "edge host.example.lan"
  ProxyJump bastion

Match host foo
  User matched

Host *
  User fallback
  Port 22
"#;

    fn home() -> PathBuf {
        dirs::home_dir().unwrap()
    }

    // TC-030
    #[test]
    fn host_wildcards_and_first_value_wins() {
        let c = SshConfig::parse(CFG);
        let v = c.lookup("lab");
        assert_eq!(v.hostname.as_deref(), Some("lab.example.lan"));
        assert_eq!(v.port, Some(2222));
        // Global User comes first in the file, so it wins.
        assert_eq!(v.user.as_deref(), Some("globaluser"));
        assert_eq!(v.identity_files, vec!["~/.ssh/lab_key"]);

        let v = c.lookup("Web.Example.LAN");
        assert_eq!(v.hostname, None);
        assert_eq!(v.port, Some(22));
        assert_eq!(v.identity_files, vec!["%d/.ssh/id_%h"]);

        // Negated pattern excludes the block.
        let v = c.lookup("bad.example.lan");
        assert!(v.identity_files.is_empty());

        assert!(glob("web-??.lan", "web-01.lan"));
        assert!(!glob("web-??.lan", "web-1.lan"));
        assert!(glob("*", ""));
        assert!(glob("a*b*c", "axxbyyc"));
        assert!(!glob("a*b", "axxc"));
    }

    // TC-030: quoting and Key=Value
    #[test]
    fn quoting_and_equals() {
        assert_eq!(
            split_line("HostName \"a b\""),
            Some(("hostname".into(), vec!["a b".into()]))
        );
        assert_eq!(
            split_line("Port=2222"),
            Some(("port".into(), vec!["2222".into()]))
        );
        assert_eq!(
            split_line("Port = 2222"),
            Some(("port".into(), vec!["2222".into()]))
        );
        assert_eq!(split_line("HostName \"unterminated"), None);
        assert_eq!(split_line("Lonely"), None);
        let c = SshConfig::parse("Port\nHost x\n HostName y\n");
        assert_eq!(c.parse_warnings.len(), 1);
    }

    // TC-031
    #[test]
    fn precedence_app_over_sshconfig_over_defaults() {
        let c = SshConfig::parse(CFG);
        let mut h = HostConfig::new("Lab", "lab");
        let r = resolve(&h, &c, "me", &home());
        assert_eq!(r.hostname, "lab.example.lan");
        assert_eq!(r.port, 2222);
        assert_eq!(r.user, "globaluser");
        assert_eq!(r.identity_files, vec![home().join(".ssh/lab_key")]);
        assert!(r.identity_explicit);

        h.port = Some(2200);
        h.user = Some("appuser".into());
        h.identity_file = Some(PathBuf::from("/keys/app"));
        let r = resolve(&h, &c, "me", &home());
        assert_eq!(r.port, 2200);
        assert_eq!(r.user, "appuser");
        assert_eq!(
            r.identity_files,
            vec![PathBuf::from("/keys/app"), home().join(".ssh/lab_key")]
        );

        // Not in ssh config at all: defaults.
        let r = resolve(
            &HostConfig::new("n", "plain.host"),
            &SshConfig::default(),
            "me",
            &home(),
        );
        assert_eq!(r.hostname, "plain.host");
        assert_eq!(r.port, 22);
        assert_eq!(r.user, "me");
        assert!(!r.identity_explicit);
        assert_eq!(r.identity_files, default_identity_files(&home()));
    }

    // TC-031: tokens in IdentityFile
    #[test]
    fn identity_tokens() {
        let c = SshConfig::parse(CFG);
        let r = resolve(&HostConfig::new("w", "web.example.lan"), &c, "me", &home());
        assert_eq!(
            r.identity_files,
            vec![home().join(".ssh").join("id_web.example.lan")]
        );
        assert_eq!(r.user, "globaluser");
    }

    // TC-032
    #[test]
    fn unsupported_keywords_warn() {
        let c = SshConfig::parse(CFG);
        let v = c.lookup("edge");
        assert_eq!(v.hostname.as_deref(), Some("edge host.example.lan"));
        assert!(
            v.warnings.iter().any(|w| w.contains("ProxyJump")),
            "{:?}",
            v.warnings
        );
        assert!(v.warnings.iter().any(|w| w.contains("Match")));
        let r = resolve(&HostConfig::new("e", "edge"), &c, "me", &home());
        assert!(r.warnings.iter().any(|w| w.contains("ProxyJump")));
        let c = SshConfig::parse("Include config.d/*\n");
        assert!(c.lookup("x").warnings[0].contains("Include"));
    }

    #[test]
    fn missing_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let c = SshConfig::load(&dir.path().join("nope")).unwrap();
        assert_eq!(c.lookup("x"), SshConfigValues::default());
    }
}
