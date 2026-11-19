//! A logged-in vSphere session (SPEC-021, SPEC-022), kept per host while
//! the app runs like an SSH session, and logged out when dropped.
//!
//! Connecting: TLS (an unknown certificate is shown to the user and pinned
//! when accepted, SPEC-023), `RetrieveServiceContent`, then `Login` with the
//! password the user types. A refused password is asked again (at most
//! three times in all); the app never retries on its own.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use vi_core::model::ConsoleTarget;
use vi_ssh::prompt::BoxFuture;
use vi_ssh::{
    CertificatePrompt, CommandOutput, CommandRunner, ConsoleStream, PasswordPrompt, Prompter,
    SshError,
};

use crate::error::VsphereError;
use crate::http::SoapHttp;
use crate::soap::{self, MoRef, Request};
use crate::tls::{self, TlsOptions};

/// How many times the password is asked in one connect.
pub const PASSWORD_ATTEMPTS: u32 = 3;
/// Objects per page of the inventory query.
pub const PAGE: u32 = 500;

/// Where and as whom to log in.
#[derive(Debug, Clone)]
pub struct Settings {
    /// Display name (app config), for the questions.
    pub name: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub tls: TlsOptions,
    /// Limit per request.
    pub timeout: Duration,
}

/// What the server is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct About {
    /// `HostAgent` (ESXi) or `VirtualCenter` (vCenter).
    pub api_type: String,
    pub version: String,
    pub full_name: String,
}

impl About {
    pub fn is_vcenter(&self) -> bool {
        self.api_type == "VirtualCenter"
    }
}

#[derive(Debug, Clone)]
struct Content {
    root: MoRef,
    property_collector: MoRef,
    session_manager: MoRef,
    about: About,
}

pub struct VsphereSession {
    http: Arc<SoapHttp>,
    content: Content,
    logged_in: Arc<AtomicBool>,
    runtime: Option<tokio::runtime::Handle>,
}

impl VsphereSession {
    pub async fn connect(
        s: &Settings,
        prompter: &dyn Prompter,
    ) -> Result<VsphereSession, VsphereError> {
        let http = Arc::new(SoapHttp::new(&s.host, s.port, s.tls.clone(), s.timeout));
        let target = http.target();
        // TLS first: an unknown certificate is asked once, then pinned.
        let mut asked = false;
        loop {
            match http.connect().await {
                Ok(()) => break,
                Err(VsphereError::UnknownCertificate { cert, .. }) if !asked => {
                    asked = true;
                    let p = CertificatePrompt {
                        name: s.name.clone(),
                        target: target.clone(),
                        fingerprint: cert.fingerprint.clone(),
                        subject: cert.subject.clone(),
                        issuer: cert.issuer.clone(),
                        not_before: cert.not_before.clone(),
                        not_after: cert.not_after.clone(),
                    };
                    if !prompter.confirm_certificate(&p).await {
                        return Err(VsphereError::CertificateRejected { target });
                    }
                    s.tls
                        .known
                        .add(&tls::target(&s.host, s.port), &cert.fingerprint)?;
                    tracing::info!(%target, fingerprint = %cert.fingerprint, "certificate accepted and pinned");
                }
                Err(e) => return Err(e),
            }
        }
        let content = service_content(&http).await?;
        let session = VsphereSession {
            http,
            content,
            logged_in: Arc::new(AtomicBool::new(false)),
            runtime: tokio::runtime::Handle::try_current().ok(),
        };
        session.login(s, prompter).await?;
        Ok(session)
    }

    async fn login(&self, s: &Settings, prompter: &dyn Prompter) -> Result<(), VsphereError> {
        let target = self.http.target();
        for attempt in 0..PASSWORD_ATTEMPTS {
            let p = PasswordPrompt {
                name: s.name.clone(),
                target: target.clone(),
                user: s.user.clone(),
                retry: attempt > 0,
            };
            let Some(pw) = prompter.password(&p).await else {
                return Err(VsphereError::LoginCancelled { target });
            };
            let req = Request::login(&self.content.session_manager, &s.user, &pw);
            drop(pw);
            match self.call(&req).await {
                Ok(_) => {
                    self.logged_in.store(true, Ordering::SeqCst);
                    tracing::info!(%target, user = %s.user, api = %self.content.about.api_type, "vSphere login");
                    return Ok(());
                }
                Err(VsphereError::Fault { kind, .. }) if kind == "InvalidLogin" => {
                    tracing::warn!(%target, user = %s.user, "vSphere login failed");
                }
                Err(e) => return Err(e),
            }
        }
        Err(VsphereError::LoginFailed {
            target,
            user: s.user.clone(),
        })
    }

    pub fn about(&self) -> &About {
        &self.content.about
    }

    pub fn target(&self) -> String {
        self.http.target()
    }

    /// Send one allowed request; a SOAP fault becomes [`VsphereError::Fault`].
    pub async fn call(&self, req: &Request) -> Result<String, VsphereError> {
        call(&self.http, req).await
    }

    /// The whole inventory query, page by page: the answer bodies.
    pub async fn inventory_pages(&self) -> Result<Vec<String>, VsphereError> {
        self.inventory_pages_of(PAGE).await
    }

    /// [`Self::inventory_pages`] with `page` objects per page.
    pub async fn inventory_pages_of(&self, page: u32) -> Result<Vec<String>, VsphereError> {
        let c = &self.content;
        let mut pages = vec![
            self.call(&Request::inventory(&c.property_collector, &c.root, page))
                .await?,
        ];
        while let Some(token) = page_token(pages.last().expect("page"), &self.target())? {
            pages.push(
                self.call(&Request::continue_inventory(&c.property_collector, &token))
                    .await?,
            );
        }
        Ok(pages)
    }

    /// Log out now (also done on drop).
    pub async fn logout(&self) {
        if self.logged_in.swap(false, Ordering::SeqCst) {
            let _ = self
                .call(&Request::logout(&self.content.session_manager))
                .await;
            tracing::info!(target = %self.target(), "vSphere logout");
        }
    }
}

impl Drop for VsphereSession {
    fn drop(&mut self) {
        // Host edited/removed, config reloaded, app closing (SPEC-022).
        if !self.logged_in.swap(false, Ordering::SeqCst) {
            return;
        }
        let Some(rt) = &self.runtime else { return };
        let http = self.http.clone();
        let sm = self.content.session_manager.clone();
        rt.spawn(async move {
            let _ = call(&http, &Request::logout(&sm)).await;
            tracing::info!(target = %http.target(), "vSphere logout");
        });
    }
}

/// The session is the host's kept "connection" in the worker; it runs no
/// commands and forwards nothing (no consoles for vSphere, SPEC-021).
impl CommandRunner for VsphereSession {
    fn run<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<CommandOutput, SshError>> {
        Box::pin(async {
            Err(SshError::Protocol(
                "a vSphere session runs no commands".into(),
            ))
        })
    }

    fn open_stream<'a>(
        &'a self,
        target: &'a ConsoleTarget,
        _: SocketAddr,
    ) -> BoxFuture<'a, Result<Box<dyn ConsoleStream>, SshError>> {
        Box::pin(async move {
            Err(SshError::Forward {
                target: target.to_string(),
                reason: "consoles are not available for vSphere".into(),
            })
        })
    }

    fn as_any(&self) -> Option<&(dyn std::any::Any + Send + Sync)> {
        Some(self)
    }
}

async fn call(http: &SoapHttp, req: &Request) -> Result<String, VsphereError> {
    let target = http.target();
    let r = http.post(req).await?;
    let doc = soap::parse_body(&r.body).map_err(|reason| VsphereError::Parse {
        target: target.clone(),
        reason,
    });
    if let Ok(d) = &doc
        && let Some(f) = soap::fault(d)
    {
        return Err(VsphereError::Fault {
            target,
            kind: f.kind,
            message: f.message,
        });
    }
    if r.status != 200 {
        return Err(VsphereError::Http {
            target,
            reason: format!("status {}", r.status),
        });
    }
    doc?;
    Ok(r.body)
}

async fn service_content(http: &SoapHttp) -> Result<Content, VsphereError> {
    let body = call(http, &Request::service_content()).await?;
    let target = http.target();
    let doc = soap::parse_body(&body).map_err(|reason| VsphereError::Parse {
        target: target.clone(),
        reason,
    })?;
    let el = |name: &str| {
        doc.descendants()
            .find(|n| n.is_element() && n.tag_name().name() == name)
    };
    let moref = |name: &str| -> Result<MoRef, VsphereError> {
        let n = el(name).ok_or_else(|| VsphereError::Parse {
            target: target.clone(),
            reason: format!("no {name} in the service content"),
        })?;
        Ok(MoRef::new(
            soap::ref_type(n).unwrap_or_default(),
            n.text().unwrap_or_default().trim(),
        ))
    };
    let text = |name: &str| {
        el(name)
            .and_then(|n| n.text())
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    Ok(Content {
        root: moref("rootFolder")?,
        property_collector: moref("propertyCollector")?,
        session_manager: moref("sessionManager")?,
        about: About {
            api_type: text("apiType"),
            version: text("version"),
            full_name: text("fullName"),
        },
    })
}

/// The continuation token of a page, if there are more.
fn page_token(body: &str, target: &str) -> Result<Option<String>, VsphereError> {
    let doc = soap::parse_body(body).map_err(|reason| VsphereError::Parse {
        target: target.to_string(),
        reason,
    })?;
    Ok(doc
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "token")
        .and_then(|n| n.text())
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty()))
}
