//! Backend errors and their classification from virsh output (SPEC-013).

use thiserror::Error;
use vi_ssh::{CommandOutput, SshError};

#[derive(Debug, Error)]
pub enum BackendError {
    /// SSH failure (connection, timeout, host key, authentication).
    #[error(transparent)]
    Ssh(#[from] SshError),

    #[error("virsh was not found on the host (is libvirt-clients installed?)")]
    VirshNotFound { detail: String },

    #[error(
        "permission denied on {uri}: add the remote user to the 'libvirt' group, or set virsh_prefix (e.g. \"sudo -n\")"
    )]
    PermissionDenied { uri: String, detail: String },

    #[error("virsh_prefix failed (sudo needs a password? use NOPASSWD for virsh)")]
    PrefixFailed { detail: String },

    #[error("cannot connect to libvirt at {uri} (is libvirtd/virtqemud running?)")]
    HypervisorUnavailable { uri: String, detail: String },

    #[error("'virsh {subcommand}' failed (exit status {status}): {detail}")]
    CommandFailed {
        subcommand: String,
        status: String,
        detail: String,
    },

    #[error("cannot parse the output of 'virsh {subcommand}': {message}")]
    Parse { subcommand: String, message: String },

    #[error("invalid host configuration: {0}")]
    Config(String),

    /// vSphere API failure (SPEC-021..023).
    #[error(transparent)]
    Vsphere(#[from] vi_vsphere::VsphereError),
}

/// Error category, for the GUI (SPEC-013).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendErrorKind {
    Connection,
    Timeout,
    HostKey,
    Authentication,
    VirshNotFound,
    PermissionDenied,
    Hypervisor,
    Parse,
    Config,
    Other,
}

impl BackendError {
    pub fn kind(&self) -> BackendErrorKind {
        use vi_ssh::SshErrorKind as S;
        match self {
            BackendError::Ssh(e) => match e.kind() {
                S::Connection => BackendErrorKind::Connection,
                S::Timeout => BackendErrorKind::Timeout,
                S::HostKey => BackendErrorKind::HostKey,
                S::Authentication => BackendErrorKind::Authentication,
                S::Other => BackendErrorKind::Other,
            },
            BackendError::VirshNotFound { .. } => BackendErrorKind::VirshNotFound,
            BackendError::PermissionDenied { .. } | BackendError::PrefixFailed { .. } => {
                BackendErrorKind::PermissionDenied
            }
            BackendError::HypervisorUnavailable { .. } => BackendErrorKind::Hypervisor,
            BackendError::Parse { .. } => BackendErrorKind::Parse,
            BackendError::Config(_) => BackendErrorKind::Config,
            BackendError::CommandFailed { .. } => BackendErrorKind::Other,
            BackendError::Vsphere(v) => {
                use vi_vsphere::VsphereError as V;
                match v {
                    V::Connect { .. } | V::Http { .. } => BackendErrorKind::Connection,
                    V::Timeout { .. } => BackendErrorKind::Timeout,
                    V::UnknownCertificate { .. }
                    | V::CertificateChanged { .. }
                    | V::CertificateRejected { .. }
                    | V::Tls { .. } => BackendErrorKind::HostKey,
                    V::LoginCancelled { .. } | V::LoginFailed { .. } => {
                        BackendErrorKind::Authentication
                    }
                    // An expired session: the worker logs in again once.
                    V::Fault { kind, .. } if kind == "NotAuthenticated" => {
                        BackendErrorKind::Connection
                    }
                    V::Fault { kind, .. } if kind == "NoPermission" => {
                        BackendErrorKind::PermissionDenied
                    }
                    V::Parse { .. } => BackendErrorKind::Parse,
                    V::Fault { .. } | V::KnownCerts { .. } => BackendErrorKind::Other,
                }
            }
        }
    }

    /// The underlying detail (stderr), for "show details" in the GUI.
    pub fn detail(&self) -> Option<&str> {
        match self {
            BackendError::VirshNotFound { detail }
            | BackendError::PermissionDenied { detail, .. }
            | BackendError::PrefixFailed { detail }
            | BackendError::HypervisorUnavailable { detail, .. }
            | BackendError::CommandFailed { detail, .. } => Some(detail),
            _ => None,
        }
    }
}

/// Classify a failed virsh command (TC-061). `uri` is used in messages.
pub fn classify(subcommand: &str, uri: &str, out: &CommandOutput) -> BackendError {
    let detail = out.stderr_text().trim().to_string();
    let lower = detail.to_ascii_lowercase();
    let status = out
        .exit_status
        .map_or_else(|| "none".to_string(), |s| s.to_string());

    if out.exit_status == Some(127)
        || lower.contains("virsh: command not found")
        || lower.contains("virsh: not found")
        || lower.contains("virsh: no such file")
    {
        return BackendError::VirshNotFound { detail };
    }
    // 126: found but not executable. Not a libvirt permission problem, so
    // no libvirt-group hint (C2 of crosscheck 03).
    if out.exit_status == Some(126) {
        return BackendError::CommandFailed {
            subcommand: subcommand.to_string(),
            status,
            detail,
        };
    }
    if lower.starts_with("sudo:") || lower.contains("\nsudo:") {
        return BackendError::PrefixFailed { detail };
    }
    if lower.contains("permission denied")
        || lower.contains("authentication unavailable")
        || lower.contains("authentication failed")
        || lower.contains("access denied")
        || lower.contains("not authorized")
    {
        return BackendError::PermissionDenied {
            uri: uri.to_string(),
            detail,
        };
    }
    if lower.contains("failed to connect to the hypervisor") {
        return BackendError::HypervisorUnavailable {
            uri: uri.to_string(),
            detail,
        };
    }
    BackendError::CommandFailed {
        subcommand: subcommand.to_string(),
        status,
        detail,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const URI: &str = "qemu:///system";

    fn kind(code: u32, stderr: &str) -> BackendErrorKind {
        classify("list", URI, &CommandOutput::failed(code, stderr)).kind()
    }

    // TC-061
    #[test]
    fn classify_stderr() {
        assert_eq!(
            kind(127, "bash: line 1: virsh: command not found\n"),
            BackendErrorKind::VirshNotFound
        );
        assert_eq!(
            kind(127, "sh: 1: virsh: not found\n"),
            BackendErrorKind::VirshNotFound
        );
        assert_eq!(
            kind(
                1,
                "error: failed to connect to the hypervisor\nerror: Failed to connect socket to '/var/run/libvirt/libvirt-sock': Permission denied\n"
            ),
            BackendErrorKind::PermissionDenied
        );
        assert_eq!(
            kind(
                1,
                "error: failed to connect to the hypervisor\nerror: authentication unavailable: no polkit agent available to authenticate action 'org.libvirt.unix.manage'\n"
            ),
            BackendErrorKind::PermissionDenied
        );
        assert_eq!(
            kind(
                1,
                "error: failed to connect to the hypervisor\nerror: Failed to connect socket to '/var/run/libvirt/virtqemud-sock-ro': No such file or directory\n"
            ),
            BackendErrorKind::Hypervisor
        );
        // sudo prefix with virsh missing: not found, not a sudo problem.
        assert_eq!(
            kind(1, "sudo: virsh: command not found\n"),
            BackendErrorKind::VirshNotFound
        );
        // Not executable: no libvirt-group hint.
        let e = classify(
            "version",
            URI,
            &CommandOutput::failed(126, "bash: line 1: /usr/bin/virsh: Permission denied\n"),
        );
        assert_eq!(e.kind(), BackendErrorKind::Other);
        assert!(!e.to_string().contains("libvirt' group"), "{e}");
        assert_eq!(
            kind(1, "sudo: a password is required\n"),
            BackendErrorKind::PermissionDenied
        );
        let e = classify("dumpxml", URI, &CommandOutput::failed(1, "error: odd\n"));
        assert_eq!(e.kind(), BackendErrorKind::Other);
        assert_eq!(e.detail(), Some("error: odd"));
        assert!(e.to_string().contains("virsh dumpxml"));
    }

    // TC-061: messages carry hints (SPEC-013).
    #[test]
    fn messages_have_hints() {
        let e = classify("list", URI, &CommandOutput::failed(1, "Permission denied"));
        let m = e.to_string();
        assert!(
            m.contains("libvirt") && m.contains("virsh_prefix") && m.contains(URI),
            "{m}"
        );
        assert!(
            classify(
                "list",
                URI,
                &CommandOutput::failed(1, "sudo: a password is required")
            )
            .to_string()
            .contains("NOPASSWD")
        );
    }
}
