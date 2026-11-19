//! VMware vSphere for virt-interact (SPEC-021..023): HTTPS with
//! certificate pinning (`tls`, `known_certs`) and the HTTP transport for
//! the read-only SOAP API (`http`).

pub mod error;
pub mod http;
pub mod known_certs;
pub mod tls;

pub use error::VsphereError;
pub use known_certs::KnownCerts;
pub use tls::{CertInfo, TlsOptions};
