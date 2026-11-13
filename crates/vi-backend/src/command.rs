//! The only place where remote command lines are built (SPEC-011).
//!
//! [`VirshCommand`] is a closed set of read-only virsh subcommands; there is
//! no way to build any other command. Arguments are single-quoted for a
//! POSIX shell. Every command runs as
//!
//! ```text
//! LC_ALL=C [virsh_prefix ]virsh -r -c '<uri>' <subcommand> '<arg>'...
//! ```
//!
//! - `LC_ALL=C` keeps the output in English, so it can be parsed.
//! - `-r` opens a read-only libvirt connection: libvirt itself refuses any
//!   change, as a second guard behind the allowlist.

use vi_core::config::{HostConfig, check_virsh_prefix};

use crate::error::BackendError;

/// The allowlist: every remote command the libvirt backend can run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VirshCommand {
    Version,
    /// `list --all`
    ListAll,
    DomInfo(String),
    DumpXml(String),
    DomBlkInfo {
        domain: String,
        target: String,
    },
    DomState(String),
    DomDisplay(String),
}

impl VirshCommand {
    /// The virsh subcommand name.
    pub fn subcommand(&self) -> &'static str {
        match self {
            VirshCommand::Version => "version",
            VirshCommand::ListAll => "list",
            VirshCommand::DomInfo(_) => "dominfo",
            VirshCommand::DumpXml(_) => "dumpxml",
            VirshCommand::DomBlkInfo { .. } => "domblkinfo",
            VirshCommand::DomState(_) => "domstate",
            VirshCommand::DomDisplay(_) => "domdisplay",
        }
    }

    fn args(&self) -> Vec<String> {
        match self {
            VirshCommand::Version => vec![],
            VirshCommand::ListAll => vec!["--all".into()],
            VirshCommand::DomInfo(d)
            | VirshCommand::DumpXml(d)
            | VirshCommand::DomState(d)
            | VirshCommand::DomDisplay(d) => vec![shell_quote(d)],
            VirshCommand::DomBlkInfo { domain, target } => {
                vec![shell_quote(domain), shell_quote(target)]
            }
        }
    }
}

/// How to reach virsh on one host: URI and optional prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Virsh {
    uri: String,
    prefix: Option<String>,
}

impl Virsh {
    /// Validates the prefix (TC-053) and the URI.
    pub fn new(uri: &str, prefix: Option<&str>) -> Result<Virsh, BackendError> {
        if uri.trim().is_empty() {
            return Err(BackendError::Config("libvirt_uri must not be empty".into()));
        }
        if let Some(p) = prefix {
            check_virsh_prefix(p)
                .map_err(|why| BackendError::Config(format!("virsh_prefix {why}")))?;
        }
        Ok(Virsh {
            uri: uri.to_string(),
            prefix: prefix.map(|p| p.trim().to_string()),
        })
    }

    pub fn from_host(host: &HostConfig) -> Result<Virsh, BackendError> {
        Virsh::new(&host.libvirt_uri, host.virsh_prefix.as_deref())
    }

    /// The full command line for `cmd`.
    pub fn line(&self, cmd: &VirshCommand) -> String {
        let mut s = String::from("LC_ALL=C ");
        if let Some(p) = &self.prefix {
            s.push_str(p);
            s.push(' ');
        }
        s.push_str("virsh -r -c ");
        s.push_str(&shell_quote(&self.uri));
        s.push(' ');
        s.push_str(cmd.subcommand());
        for a in cmd.args() {
            s.push(' ');
            s.push_str(&a);
        }
        s
    }
}

/// Quote `s` as one word for a POSIX shell: `'...'`, with `'` as `'\''`.
pub fn shell_quote(s: &str) -> String {
    let mut q = String::with_capacity(s.len() + 2);
    q.push('\'');
    for c in s.chars() {
        if c == '\'' {
            q.push_str("'\\''");
        } else {
            q.push(c);
        }
    }
    q.push('\'');
    q
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v() -> Virsh {
        Virsh::new("qemu:///system", None).unwrap()
    }

    // TC-050
    #[test]
    fn only_allowlisted_subcommands() {
        let all = [
            VirshCommand::Version,
            VirshCommand::ListAll,
            VirshCommand::DomInfo("d".into()),
            VirshCommand::DumpXml("d".into()),
            VirshCommand::DomBlkInfo {
                domain: "d".into(),
                target: "vda".into(),
            },
            VirshCommand::DomState("d".into()),
            VirshCommand::DomDisplay("d".into()),
        ];
        const ALLOWED: [&str; 7] = [
            "version",
            "list",
            "dominfo",
            "dumpxml",
            "domblkinfo",
            "domstate",
            "domdisplay",
        ];
        for c in &all {
            assert!(ALLOWED.contains(&c.subcommand()));
            let line = v().line(c);
            assert!(
                line.starts_with("LC_ALL=C virsh -r -c 'qemu:///system' "),
                "{line}"
            );
        }
        assert_eq!(
            v().line(&VirshCommand::ListAll),
            "LC_ALL=C virsh -r -c 'qemu:///system' list --all"
        );
        assert_eq!(
            v().line(&VirshCommand::DomBlkInfo {
                domain: "vm 1".into(),
                target: "vda".into()
            }),
            "LC_ALL=C virsh -r -c 'qemu:///system' domblkinfo 'vm 1' 'vda'"
        );
    }

    // TC-051
    #[test]
    fn hostile_names_are_quoted() {
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_quote("x; rm -rf /"), "'x; rm -rf /'");
        assert_eq!(shell_quote("$(reboot)"), "'$(reboot)'");
        assert_eq!(shell_quote("`id`"), "'`id`'");
        assert_eq!(shell_quote(""), "''");
        let line = v().line(&VirshCommand::DumpXml("a'; reboot; '".into()));
        assert_eq!(
            line,
            "LC_ALL=C virsh -r -c 'qemu:///system' dumpxml 'a'\\''; reboot; '\\'''"
        );
    }

    // TC-051: check the quoting with a real POSIX shell where one exists.
    #[cfg(unix)]
    #[test]
    fn quoting_round_trips_through_sh() {
        for s in [
            "plain",
            "a b",
            "it's",
            "x; y",
            "$(id) `id` \"q\" \\ *",
            "''",
        ] {
            let out = std::process::Command::new("sh")
                .arg("-c")
                .arg(format!("printf %s {}", shell_quote(s)))
                .output()
                .expect("sh");
            assert_eq!(String::from_utf8_lossy(&out.stdout), s);
        }
    }

    // TC-053
    #[test]
    fn prefix_prepended_exactly_and_validated() {
        let v = Virsh::new("qemu:///system", Some("sudo -n")).unwrap();
        assert_eq!(
            v.line(&VirshCommand::Version),
            "LC_ALL=C sudo -n virsh -r -c 'qemu:///system' version"
        );
        for bad in ["sudo; reboot", "a & b", "a | b", "$(x)"] {
            assert!(matches!(
                Virsh::new("qemu:///system", Some(bad)),
                Err(BackendError::Config(_))
            ));
        }
        assert!(Virsh::new(" ", None).is_err());
        let uri = Virsh::new("qemu+ssh://x/system'; id", None).unwrap();
        assert!(
            uri.line(&VirshCommand::Version)
                .contains("-c 'qemu+ssh://x/system'\\''; id' version")
        );
    }
}
