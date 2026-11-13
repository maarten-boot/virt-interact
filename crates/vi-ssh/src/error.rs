//! SSH errors, grouped so the GUI can explain them in plain language
//! (SPEC-013).

use std::io;
use std::path::PathBuf;

use russh::keys::PublicKey;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SshError {
    #[error("cannot connect to {addr}: {source}")]
    Connect { addr: String, source: io::Error },

    #[error("timed out after {secs} s while {what}")]
    Timeout { what: String, secs: u64 },

    #[error(
        "the host key of {host} has CHANGED (it differs from known_hosts line {line} in {}). \
         This can mean someone is intercepting the connection. Connection refused. \
         Presented key: {fingerprint}",
        known_hosts.display()
    )]
    HostKeyMismatch {
        host: String,
        line: usize,
        fingerprint: String,
        known_hosts: PathBuf,
    },

    #[error(
        "the host key of {host} is marked @revoked (known_hosts line {line}). Connection refused."
    )]
    HostKeyRevoked { host: String, line: usize },

    #[error(
        "the host key of {host} changed between the first and second connection attempt \
         (now {fingerprint}). Connection refused."
    )]
    HostKeyChangedDuringConnect { host: String, fingerprint: String },

    #[error("the host key of {host} ({fingerprint}) was not accepted")]
    HostKeyRejected { host: String, fingerprint: String },

    /// Internal: raised during the handshake so the user can be asked
    /// outside of any network timeout.
    #[error("unknown host key")]
    UnknownHostKey {
        key: Box<PublicKey>,
        other_types: Vec<String>,
    },

    #[error("the host presented a certificate; host certificates are not supported")]
    HostCertificate,

    #[error("cannot read {}: {source}", path.display())]
    KnownHosts { path: PathBuf, source: io::Error },

    #[error("authentication failed for {user}@{host}. Tried: {}", if tried.is_empty() { "no keys (no agent and no key files)".to_string() } else { tried.join("; ") })]
    AuthFailed {
        user: String,
        host: String,
        tried: Vec<String>,
    },

    #[error("ssh: {0}")]
    Protocol(String),
}

impl From<russh::Error> for SshError {
    fn from(e: russh::Error) -> Self {
        SshError::Protocol(e.to_string())
    }
}

/// Coarse error categories for the GUI (SPEC-013).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SshErrorKind {
    Connection,
    Timeout,
    HostKey,
    Authentication,
    Other,
}

impl SshError {
    pub fn kind(&self) -> SshErrorKind {
        match self {
            SshError::Connect { .. } => SshErrorKind::Connection,
            SshError::Timeout { .. } => SshErrorKind::Timeout,
            SshError::HostKeyMismatch { .. }
            | SshError::HostKeyRevoked { .. }
            | SshError::HostKeyRejected { .. }
            | SshError::HostKeyChangedDuringConnect { .. }
            | SshError::UnknownHostKey { .. }
            | SshError::HostCertificate
            | SshError::KnownHosts { .. } => SshErrorKind::HostKey,
            SshError::AuthFailed { .. } => SshErrorKind::Authentication,
            SshError::Protocol(_) => SshErrorKind::Other,
        }
    }
}
