//! Questions the connection layers ask the user: accept an unknown host key
//! (SPEC-008) and the passphrase of an encrypted key (SPEC-006); for
//! vSphere (v1.3) accept a server certificate (SPEC-023) and the login
//! password (SPEC-022).
//! The GUI implements [`Prompter`]; tests use [`ScriptedPrompter`].

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Mutex;

use crate::secret::Secret;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Details shown when a host key is not yet known.
#[derive(Debug, Clone, PartialEq)]
pub struct HostKeyPrompt {
    /// Display name of the host (app config).
    pub name: String,
    pub hostname: String,
    pub port: u16,
    pub key_type: String,
    /// `SHA256:...`
    pub fingerprint: String,
    /// Other key types already recorded for this host (normally empty).
    pub other_types: Vec<String>,
}

/// Details shown for a server certificate that is not trusted yet.
#[derive(Debug, Clone, PartialEq)]
pub struct CertificatePrompt {
    /// Display name of the host (app config).
    pub name: String,
    /// `host:port`
    pub target: String,
    /// `SHA256:` and hex.
    pub fingerprint: String,
    pub subject: String,
    pub issuer: String,
    pub not_before: String,
    pub not_after: String,
}

/// The vSphere login question.
#[derive(Debug, Clone, PartialEq)]
pub struct PasswordPrompt {
    /// Display name of the host (app config).
    pub name: String,
    /// `host:port`
    pub target: String,
    pub user: String,
    /// The previous password was refused.
    pub retry: bool,
}

pub trait Prompter: Send + Sync {
    /// Ask whether to trust and record an unknown host key.
    fn confirm_host_key<'a>(&'a self, prompt: &'a HostKeyPrompt) -> BoxFuture<'a, bool>;
    /// Ask for the passphrase of `key_path`; `None` skips the key.
    fn passphrase<'a>(&'a self, key_path: &'a Path) -> BoxFuture<'a, Option<Secret>>;
    /// Ask whether to trust and pin a server certificate (default: no).
    fn confirm_certificate<'a>(&'a self, _prompt: &'a CertificatePrompt) -> BoxFuture<'a, bool> {
        Box::pin(async { false })
    }
    /// Ask for a login password; `None` cancels (default: cancel).
    fn password<'a>(&'a self, _prompt: &'a PasswordPrompt) -> BoxFuture<'a, Option<Secret>> {
        Box::pin(async { None })
    }
}

/// Rejects unknown host keys and never gives a passphrase.
pub struct NoPrompt;

impl Prompter for NoPrompt {
    fn confirm_host_key<'a>(&'a self, _: &'a HostKeyPrompt) -> BoxFuture<'a, bool> {
        Box::pin(async { false })
    }
    fn passphrase<'a>(&'a self, _: &'a Path) -> BoxFuture<'a, Option<Secret>> {
        Box::pin(async { None })
    }
}

/// Fixed answers, recording what was asked. For tests and scripted use.
pub struct ScriptedPrompter {
    pub accept_host_key: bool,
    pub passphrase: Option<String>,
    pub host_key_prompts: Mutex<Vec<HostKeyPrompt>>,
    pub passphrase_prompts: Mutex<Vec<PathBuf>>,
    /// vSphere: accept certificates; passwords given in turn (then none).
    pub accept_certificate: bool,
    pub passwords: Mutex<Vec<String>>,
    pub certificate_prompts: Mutex<Vec<CertificatePrompt>>,
    pub password_prompts: Mutex<Vec<PasswordPrompt>>,
}

impl ScriptedPrompter {
    pub fn new(accept_host_key: bool, passphrase: Option<&str>) -> ScriptedPrompter {
        ScriptedPrompter {
            accept_host_key,
            passphrase: passphrase.map(str::to_string),
            host_key_prompts: Mutex::new(Vec::new()),
            passphrase_prompts: Mutex::new(Vec::new()),
            accept_certificate: false,
            passwords: Mutex::new(Vec::new()),
            certificate_prompts: Mutex::new(Vec::new()),
            password_prompts: Mutex::new(Vec::new()),
        }
    }
}

impl Prompter for ScriptedPrompter {
    fn confirm_host_key<'a>(&'a self, prompt: &'a HostKeyPrompt) -> BoxFuture<'a, bool> {
        self.host_key_prompts
            .lock()
            .expect("lock")
            .push(prompt.clone());
        let a = self.accept_host_key;
        Box::pin(async move { a })
    }
    fn passphrase<'a>(&'a self, key_path: &'a Path) -> BoxFuture<'a, Option<Secret>> {
        self.passphrase_prompts
            .lock()
            .expect("lock")
            .push(key_path.to_path_buf());
        let p = self.passphrase.clone().map(Secret::new);
        Box::pin(async move { p })
    }
    fn confirm_certificate<'a>(&'a self, prompt: &'a CertificatePrompt) -> BoxFuture<'a, bool> {
        self.certificate_prompts
            .lock()
            .expect("lock")
            .push(prompt.clone());
        let a = self.accept_certificate;
        Box::pin(async move { a })
    }
    fn password<'a>(&'a self, prompt: &'a PasswordPrompt) -> BoxFuture<'a, Option<Secret>> {
        self.password_prompts
            .lock()
            .expect("lock")
            .push(prompt.clone());
        let mut list = self.passwords.lock().expect("lock");
        let p = (!list.is_empty()).then(|| Secret::new(list.remove(0)));
        Box::pin(async move { p })
    }
}
