//! TC-134: certificate trust against a local TLS server (SPEC-023).

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use rcgen::{BasicConstraints, CertificateParams, IsCa, Issuer, KeyPair};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use vi_vsphere::http::SoapHttp;
use vi_vsphere::known_certs::format_fingerprint;
use vi_vsphere::tls::{self, sha256};
use vi_vsphere::{KnownCerts, TlsOptions, VsphereError};

/// A TLS server on 127.0.0.1 that answers every HTTP request with the
/// request's Cookie header in the body, and sets a session cookie.
async fn server(cert: CertificateDer<'static>, key: PrivateKeyDer<'static>) -> SocketAddr {
    let cfg = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(vec![cert], key)
    .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(cfg));
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((tcp, _)) = l.accept().await else {
                return;
            };
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                let Ok(mut s) = acceptor.accept(tcp).await else {
                    return;
                };
                loop {
                    let mut buf = Vec::new();
                    let mut b = [0u8; 4096];
                    // Read one request (headers + Content-Length body).
                    let req = loop {
                        let Ok(n) = s.read(&mut b).await else { return };
                        if n == 0 {
                            return;
                        }
                        buf.extend_from_slice(&b[..n]);
                        let text = String::from_utf8_lossy(&buf).into_owned();
                        if let Some(end) = text.find("\r\n\r\n") {
                            let len: usize = text
                                .lines()
                                .find_map(|l| {
                                    l.to_ascii_lowercase()
                                        .strip_prefix("content-length:")
                                        .map(|v| v.trim().parse().unwrap())
                                })
                                .unwrap_or(0);
                            if buf.len() >= end + 4 + len {
                                break text;
                            }
                        }
                    };
                    let cookie = req
                        .lines()
                        .find_map(|l| {
                            l.strip_prefix("cookie: ")
                                .or_else(|| l.strip_prefix("Cookie: "))
                        })
                        .unwrap_or("none")
                        .to_string();
                    let body = format!("cookie={cookie}");
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/xml\r\nSet-Cookie: vmware_soap_session=\"s1\"; Path=/; HttpOnly\r\nContent-Length: {}\r\n\r\n{body}",
                        body.len()
                    );
                    if s.write_all(resp.as_bytes()).await.is_err() {
                        return;
                    }
                }
            });
        }
    });
    addr
}

fn self_signed() -> (CertificateDer<'static>, PrivateKeyDer<'static>) {
    let k =
        rcgen::generate_simple_self_signed(vec!["localhost".into(), "127.0.0.1".into()]).unwrap();
    (
        k.cert.der().clone(),
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(k.signing_key.serialize_der())),
    )
}

fn opts(dir: &std::path::Path) -> TlsOptions {
    TlsOptions {
        known: KnownCerts::new(dir.join("known_certs")),
        extra_roots: Vec::new(),
        connect_timeout: Duration::from_secs(5),
    }
}

#[tokio::test]
async fn unknown_then_pinned_then_changed() {
    let d = tempfile::tempdir().unwrap();
    let (cert, key) = self_signed();
    let fp = format_fingerprint(&sha256(&cert));
    let addr = server(cert, key).await;
    let o = opts(d.path());

    // Unknown: stopped, with the details to show.
    match tls::connect("127.0.0.1", addr.port(), &o).await {
        Err(VsphereError::UnknownCertificate { target, cert }) => {
            assert_eq!(target, format!("127.0.0.1:{}", addr.port()));
            assert_eq!(cert.fingerprint, fp);
            assert!(cert.subject.contains("rcgen"), "{}", cert.subject);
            assert!(!cert.not_after.is_empty());
        }
        other => panic!("{other:?}"),
    }

    // Accepted: pinned, then trusted; HTTP through it, cookie kept.
    o.known
        .add(&tls::target("127.0.0.1", addr.port()), &fp)
        .unwrap();
    let http = SoapHttp::new("127.0.0.1", addr.port(), o.clone(), Duration::from_secs(5));
    let r = http.post("<x/>", "urn:vim25/8.0").await.unwrap();
    assert_eq!((r.status, r.body.as_str()), (200, "cookie=none"));
    assert_eq!(http.cookie().as_deref(), Some("vmware_soap_session=\"s1\""));
    let r = http.post("<x/>", "urn:vim25/8.0").await.unwrap();
    assert_eq!(r.body, "cookie=vmware_soap_session=\"s1\"");

    // A different pin: the certificate "changed" and is refused.
    o.known
        .add(
            &tls::target("127.0.0.1", addr.port()),
            &format!("SHA256:{}", "0".repeat(64)),
        )
        .unwrap();
    match tls::connect("127.0.0.1", addr.port(), &o).await {
        Err(VsphereError::CertificateChanged { got, file, .. }) => {
            assert_eq!(got, fp);
            assert!(file.ends_with("known_certs"));
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn system_trusted_and_name_checked() {
    let d = tempfile::tempdir().unwrap();
    let ca_key = KeyPair::generate().unwrap();
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let ca_cert = ca_params.self_signed(&ca_key).unwrap();
    let issuer = Issuer::new(ca_params, ca_key);
    let leaf_key = KeyPair::generate().unwrap();
    let leaf = CertificateParams::new(vec!["localhost".to_string()])
        .unwrap()
        .signed_by(&leaf_key, &issuer)
        .unwrap();
    let addr = server(
        leaf.der().clone(),
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der())),
    )
    .await;
    let mut o = opts(d.path());
    o.extra_roots = vec![ca_cert.der().clone()];
    // Trusted and the name matches: no question, nothing pinned.
    tls::connect("localhost", addr.port(), &o).await.unwrap();
    assert!(o.known.load().unwrap().is_empty());
    // Trusted CA, but the name does not match: asked like an unknown one.
    assert!(matches!(
        tls::connect("127.0.0.1", addr.port(), &o).await,
        Err(VsphereError::UnknownCertificate { .. })
    ));
}

#[tokio::test]
async fn nothing_listening() {
    let d = tempfile::tempdir().unwrap();
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    drop(l);
    assert!(matches!(
        tls::connect("127.0.0.1", port, &opts(d.path())).await,
        Err(VsphereError::Connect { .. })
    ));
}
