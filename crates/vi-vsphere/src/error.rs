//! Errors of the vSphere layer (SPEC-021..023).

use thiserror::Error;

use crate::tls::CertInfo;

#[derive(Debug, Error)]
pub enum VsphereError {
    /// TCP connect or read/write failure.
    #[error("cannot connect to {target}: {reason}")]
    Connect { target: String, reason: String },

    /// The server certificate is not trusted by the system and not pinned
    /// yet: the user decides (SPEC-023).
    #[error("the certificate of {target} is not trusted (SHA-256 {})", .cert.fingerprint)]
    UnknownCertificate { target: String, cert: Box<CertInfo> },

    /// The pinned fingerprint does not match: refused, no override.
    #[error(
        "the certificate of {target} has CHANGED: pinned SHA-256 {expected}, now {got}. \
         Check with the server's administrator, then remove the line from {file}"
    )]
    CertificateChanged {
        target: String,
        expected: String,
        got: String,
        file: String,
    },

    /// Any other TLS failure.
    #[error("TLS with {target} failed: {reason}")]
    Tls { target: String, reason: String },

    /// HTTP-level failure (bad status without a SOAP fault, protocol).
    #[error("HTTP error from {target}: {reason}")]
    Http { target: String, reason: String },

    /// No answer in time.
    #[error("{target} did not answer within {secs} s")]
    Timeout { target: String, secs: u64 },

    /// `known_certs` could not be read or written.
    #[error("{file}: {reason}")]
    KnownCerts { file: String, reason: String },
}
