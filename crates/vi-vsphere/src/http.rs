//! HTTP/1.1 POSTs to `/sdk` over one kept TLS connection (SPEC-021).
//!
//! The session cookie (`vmware_soap_session`) is taken from the first
//! response that sets it and sent with every later request. A connection
//! that the server closed is opened again once per request.

use std::sync::Mutex;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full, Limited};
use hyper::client::conn::http1::SendRequest;
use hyper_util::rt::TokioIo;

use crate::error::VsphereError;
use crate::tls::{self, TlsOptions};

/// Larger answers are refused (a vCenter inventory is a few MB).
pub const MAX_RESPONSE: usize = 256 * 1024 * 1024;

pub struct SoapHttp {
    host: String,
    port: u16,
    opts: TlsOptions,
    timeout: Duration,
    conn: tokio::sync::Mutex<Option<SendRequest<Full<Bytes>>>>,
    cookie: Mutex<Option<String>>,
}

/// One HTTP answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub body: String,
}

impl SoapHttp {
    pub fn new(host: &str, port: u16, opts: TlsOptions, timeout: Duration) -> SoapHttp {
        SoapHttp {
            host: host.to_string(),
            port,
            opts,
            timeout,
            conn: tokio::sync::Mutex::new(None),
            cookie: Mutex::new(None),
        }
    }

    pub fn target(&self) -> String {
        tls::target(&self.host, self.port)
    }

    /// The session cookie, once the server has set one.
    pub fn cookie(&self) -> Option<String> {
        self.cookie.lock().expect("lock").clone()
    }

    /// Open the TLS connection now (certificate check, SPEC-023).
    pub async fn connect(&self) -> Result<(), VsphereError> {
        let mut c = self.conn.lock().await;
        *c = Some(self.open().await?);
        Ok(())
    }

    async fn open(&self) -> Result<SendRequest<Full<Bytes>>, VsphereError> {
        let stream = tls::connect(&self.host, self.port, &self.opts).await?;
        let (sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
            .await
            .map_err(|e| self.http_err(e.to_string()))?;
        let target = self.target();
        tokio::spawn(async move {
            if let Err(e) = conn.await {
                tracing::debug!(%target, "vSphere connection ended: {e}");
            }
        });
        Ok(sender)
    }

    fn http_err(&self, reason: String) -> VsphereError {
        VsphereError::Http {
            target: self.target(),
            reason,
        }
    }

    /// POST a SOAP envelope; any HTTP status is returned (SOAP faults come
    /// with 500).
    pub async fn post(&self, body: &str, soap_action: &str) -> Result<Response, VsphereError> {
        let secs = self.timeout.as_secs().max(1);
        let t = self.target();
        tokio::time::timeout(self.timeout, self.post_inner(body, soap_action))
            .await
            .map_err(|_| VsphereError::Timeout { target: t, secs })?
    }

    async fn post_inner(&self, body: &str, soap_action: &str) -> Result<Response, VsphereError> {
        let mut guard = self.conn.lock().await;
        let mut retried = false;
        loop {
            if guard.as_ref().is_none_or(|s| s.is_closed()) {
                *guard = Some(self.open().await?);
            }
            let sender = guard.as_mut().expect("connection");
            let mut req = hyper::Request::post("/sdk")
                .header(hyper::header::HOST, self.target())
                .header(hyper::header::CONTENT_TYPE, "text/xml; charset=utf-8")
                .header("SOAPAction", soap_action);
            if let Some(c) = self.cookie() {
                req = req.header(hyper::header::COOKIE, c);
            }
            let req = req
                .body(Full::new(Bytes::from(body.to_string())))
                .map_err(|e| self.http_err(e.to_string()))?;
            let sent = match sender.ready().await {
                Ok(()) => sender.send_request(req).await,
                Err(e) => Err(e),
            };
            let resp = match sent {
                Ok(r) => r,
                // The server closed a kept connection: open it again once.
                Err(e)
                    if !retried
                        && (e.is_closed() || e.is_canceled() || e.is_incomplete_message()) =>
                {
                    tracing::debug!(target = %self.target(), "reconnecting: {e}");
                    *guard = None;
                    retried = true;
                    continue;
                }
                Err(e) => return Err(self.http_err(e.to_string())),
            };
            for v in resp.headers().get_all(hyper::header::SET_COOKIE) {
                if let Some(c) = v.to_str().ok().and_then(session_cookie) {
                    *self.cookie.lock().expect("lock") = Some(c);
                }
            }
            let status = resp.status().as_u16();
            let bytes = Limited::new(resp.into_body(), MAX_RESPONSE)
                .collect()
                .await
                .map_err(|e| self.http_err(format!("reading the answer: {e}")))?
                .to_bytes();
            return Ok(Response {
                status,
                body: String::from_utf8_lossy(&bytes).into_owned(),
            });
        }
    }

    /// Forget the connection and the session cookie.
    pub async fn close(&self) {
        *self.conn.lock().await = None;
        *self.cookie.lock().expect("lock") = None;
    }
}

/// `vmware_soap_session="…"` from a `Set-Cookie` value (name and value
/// only, attributes dropped).
pub fn session_cookie(set_cookie: &str) -> Option<String> {
    let first = set_cookie.split(';').next()?.trim();
    first
        .starts_with("vmware_soap_session=")
        .then(|| first.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookies() {
        assert_eq!(
            session_cookie("vmware_soap_session=\"abc\"; Path=/; HttpOnly; Secure;").as_deref(),
            Some("vmware_soap_session=\"abc\"")
        );
        assert_eq!(session_cookie("other=1; Path=/"), None);
    }
}
