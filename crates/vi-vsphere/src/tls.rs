//! TLS to a vSphere server with certificate pinning (SPEC-023).
//!
//! - A pinned fingerprint in `known_certs` for `host:port` decides alone:
//!   a match is trusted, anything else is refused (no override).
//! - Without a pin, the system's trusted certificates decide, with the
//!   host name checked. A certificate they reject stops the handshake and
//!   is returned as [`VsphereError::UnknownCertificate`] for the user to
//!   accept (then pinned) or reject; the caller connects again.
//!
//! rustls with the ring provider; TLS 1.2 and 1.3 only.

use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;
use x509_cert::der::Decode;

use crate::error::VsphereError;
use crate::known_certs::{KnownCerts, format_fingerprint, parse_fingerprint};

/// What the user sees about a certificate before accepting it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertInfo {
    /// `SHA256:` and hex.
    pub fingerprint: String,
    pub subject: String,
    pub issuer: String,
    pub not_before: String,
    pub not_after: String,
}

impl CertInfo {
    pub fn from_der(der: &[u8]) -> CertInfo {
        let fingerprint = format_fingerprint(&sha256(der));
        match x509_cert::Certificate::from_der(der) {
            Ok(c) => {
                let t = &c.tbs_certificate;
                CertInfo {
                    fingerprint,
                    subject: t.subject.to_string(),
                    issuer: t.issuer.to_string(),
                    not_before: t.validity.not_before.to_string(),
                    not_after: t.validity.not_after.to_string(),
                }
            }
            Err(_) => CertInfo {
                fingerprint,
                subject: "unknown".into(),
                issuer: "unknown".into(),
                not_before: "unknown".into(),
                not_after: "unknown".into(),
            },
        }
    }
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    let d = ring::digest::digest(&ring::digest::SHA256, data);
    let mut out = [0u8; 32];
    out.copy_from_slice(d.as_ref());
    out
}

/// How to reach one server.
#[derive(Debug, Clone)]
pub struct TlsOptions {
    pub known: KnownCerts,
    /// Trusted in addition to the system's certificates (tests).
    pub extra_roots: Vec<CertificateDer<'static>>,
    pub connect_timeout: Duration,
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// The system's trusted certificates, read once.
fn system_roots() -> &'static Vec<CertificateDer<'static>> {
    static ROOTS: OnceLock<Vec<CertificateDer<'static>>> = OnceLock::new();
    ROOTS.get_or_init(|| {
        let r = rustls_native_certs::load_native_certs();
        for e in &r.errors {
            tracing::warn!("system certificates: {e}");
        }
        r.certs
    })
}

/// What the verifier saw, for the caller after a failed handshake.
#[derive(Debug, Default)]
struct Seen {
    cert: Option<Vec<u8>>,
    /// The system's check rejected the certificate (no pin).
    rejected: bool,
}

#[derive(Debug)]
struct PinVerifier {
    system: Option<Arc<WebPkiServerVerifier>>,
    pinned: Option<[u8; 32]>,
    seen: Arc<Mutex<Seen>>,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for PinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        self.seen.lock().expect("lock").cert = Some(end_entity.to_vec());
        if let Some(pin) = self.pinned {
            return if sha256(end_entity) == pin {
                Ok(ServerCertVerified::assertion())
            } else {
                Err(rustls::Error::General(
                    "certificate does not match the pinned fingerprint".into(),
                ))
            };
        }
        let r = match &self.system {
            Some(v) => v.verify_server_cert(end_entity, intermediates, server_name, ocsp, now),
            None => Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::UnknownIssuer,
            )),
        };
        if r.is_err() {
            self.seen.lock().expect("lock").rejected = true;
        }
        r
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// `host:port`, the key in `known_certs`.
pub fn target(host: &str, port: u16) -> String {
    format!("{host}:{port}")
}

/// Open a TLS connection to `host:port` (SPEC-023).
pub async fn connect(
    host: &str,
    port: u16,
    opts: &TlsOptions,
) -> Result<TlsStream<TcpStream>, VsphereError> {
    let target = target(host, port);
    let pin_text = opts.known.get(&target)?;
    let pinned = pin_text.as_deref().and_then(parse_fingerprint);
    let provider = provider();
    let mut roots = RootCertStore::empty();
    let (_, bad) = roots.add_parsable_certificates(system_roots().iter().cloned());
    if bad > 0 {
        tracing::debug!(
            bad,
            "system certificates that rustls cannot use were skipped"
        );
    }
    roots.add_parsable_certificates(opts.extra_roots.iter().cloned());
    let system = (!roots.is_empty())
        .then(|| {
            WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider.clone())
                .build()
                .ok()
        })
        .flatten();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let verifier = PinVerifier {
        system,
        pinned,
        seen: seen.clone(),
        provider: provider.clone(),
    };
    let tls_err = |reason: String| VsphereError::Tls {
        target: target.clone(),
        reason,
    };
    let config = ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
        .map_err(|e| tls_err(e.to_string()))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    let name = ServerName::try_from(host.to_string())
        .map_err(|e| tls_err(format!("invalid host name: {e}")))?;
    let secs = opts.connect_timeout.as_secs().max(1);
    let timeout = |_| VsphereError::Timeout {
        target: target.clone(),
        secs,
    };
    let tcp = tokio::time::timeout(opts.connect_timeout, TcpStream::connect((host, port)))
        .await
        .map_err(timeout)?
        .map_err(|e| VsphereError::Connect {
            target: target.clone(),
            reason: e.to_string(),
        })?;
    let r = tokio::time::timeout(
        opts.connect_timeout,
        TlsConnector::from(Arc::new(config)).connect(name, tcp),
    )
    .await
    .map_err(timeout)?;
    match r {
        Ok(s) => Ok(s),
        Err(e) => {
            let seen = std::mem::take(&mut *seen.lock().expect("lock"));
            Err(match (pinned, seen.cert) {
                (Some(pin), Some(der)) if sha256(&der) != pin => VsphereError::CertificateChanged {
                    target,
                    expected: format_fingerprint(&pin),
                    got: format_fingerprint(&sha256(&der)),
                    file: opts.known.path().display().to_string(),
                },
                (None, Some(der)) if seen.rejected => VsphereError::UnknownCertificate {
                    target,
                    cert: Box::new(CertInfo::from_der(&der)),
                },
                _ => tls_err(e.to_string()),
            })
        }
    }
}
