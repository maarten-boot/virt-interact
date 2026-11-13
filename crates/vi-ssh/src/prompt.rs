//! Questions the SSH layer asks the user: accept an unknown host key
//! (SPEC-008) and the passphrase of an encrypted key (SPEC-006).
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

pub trait Prompter: Send + Sync {
    /// Ask whether to trust and record an unknown host key.
    fn confirm_host_key<'a>(&'a self, prompt: &'a HostKeyPrompt) -> BoxFuture<'a, bool>;
    /// Ask for the passphrase of `key_path`; `None` skips the key.
    fn passphrase<'a>(&'a self, key_path: &'a Path) -> BoxFuture<'a, Option<Secret>>;
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
}

impl ScriptedPrompter {
    pub fn new(accept_host_key: bool, passphrase: Option<&str>) -> ScriptedPrompter {
        ScriptedPrompter {
            accept_host_key,
            passphrase: passphrase.map(str::to_string),
            host_key_prompts: Mutex::new(Vec::new()),
            passphrase_prompts: Mutex::new(Vec::new()),
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
}
