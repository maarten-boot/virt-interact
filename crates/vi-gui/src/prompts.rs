//! The GUI's [`Prompter`]: questions from the SSH layer (unknown host key,
//! key passphrase) are sent to the GUI thread as [`PromptRequest`]s and
//! answered from a dialog (SPEC-006, SPEC-008, Plan 4.3).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::Sender;

use tokio::sync::oneshot;
use vi_ssh::prompt::BoxFuture;
use vi_ssh::{CertificatePrompt, HostKeyPrompt, PasswordPrompt, Prompter, Secret};

pub enum PromptKind {
    HostKey(HostKeyPrompt),
    Passphrase(PathBuf),
    /// vSphere (v1.3): a server certificate to accept (SPEC-023).
    Certificate(CertificatePrompt),
    /// vSphere (v1.3): the login password (SPEC-022).
    Password(PasswordPrompt),
}

/// One question waiting for the user. Dropping it without an answer counts
/// as "no" / "skip this key".
pub struct PromptRequest {
    pub kind: PromptKind,
    reply: oneshot::Sender<Reply>,
}

enum Reply {
    HostKey(bool),
    Passphrase(Option<Secret>),
    Certificate(bool),
    Password(Option<Secret>),
}

impl PromptRequest {
    pub fn answer_host_key(self, accept: bool) {
        let _ = self.reply.send(Reply::HostKey(accept));
    }

    pub fn answer_passphrase(self, passphrase: Option<Secret>) {
        let _ = self.reply.send(Reply::Passphrase(passphrase));
    }

    pub fn answer_certificate(self, accept: bool) {
        let _ = self.reply.send(Reply::Certificate(accept));
    }

    pub fn answer_password(self, password: Option<Secret>) {
        let _ = self.reply.send(Reply::Password(password));
    }
}

pub struct GuiPrompter {
    tx: Sender<PromptRequest>,
    notify: Arc<dyn Fn() + Send + Sync>,
}

impl GuiPrompter {
    /// `notify` wakes the GUI (egui `request_repaint`).
    pub fn new(tx: Sender<PromptRequest>, notify: Arc<dyn Fn() + Send + Sync>) -> GuiPrompter {
        GuiPrompter { tx, notify }
    }

    async fn ask(&self, kind: PromptKind) -> Option<Reply> {
        let (reply, rx) = oneshot::channel();
        self.tx.send(PromptRequest { kind, reply }).ok()?;
        (self.notify)();
        rx.await.ok()
    }
}

impl Prompter for GuiPrompter {
    fn confirm_host_key<'a>(&'a self, p: &'a HostKeyPrompt) -> BoxFuture<'a, bool> {
        Box::pin(async move {
            matches!(
                self.ask(PromptKind::HostKey(p.clone())).await,
                Some(Reply::HostKey(true))
            )
        })
    }

    fn passphrase<'a>(&'a self, key_path: &'a Path) -> BoxFuture<'a, Option<Secret>> {
        Box::pin(async move {
            match self
                .ask(PromptKind::Passphrase(key_path.to_path_buf()))
                .await
            {
                Some(Reply::Passphrase(s)) => s,
                _ => None,
            }
        })
    }

    fn confirm_certificate<'a>(&'a self, p: &'a CertificatePrompt) -> BoxFuture<'a, bool> {
        Box::pin(async move {
            matches!(
                self.ask(PromptKind::Certificate(p.clone())).await,
                Some(Reply::Certificate(true))
            )
        })
    }

    fn password<'a>(&'a self, p: &'a PasswordPrompt) -> BoxFuture<'a, Option<Secret>> {
        Box::pin(async move {
            match self.ask(PromptKind::Password(p.clone())).await {
                Some(Reply::Password(s)) => s,
                _ => None,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn prompt() -> HostKeyPrompt {
        HostKeyPrompt {
            name: "lab".into(),
            hostname: "lab.example.lan".into(),
            port: 22,
            key_type: "ssh-ed25519".into(),
            fingerprint: "SHA256:x".into(),
            other_types: vec![],
        }
    }

    // TC-085: answers from the GUI thread reach the SSH layer; a dropped
    // request means "no".
    #[test]
    fn answers_reach_the_ssh_layer() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (tx, rx) = mpsc::channel();
        let p = GuiPrompter::new(tx, Arc::new(|| {}));
        let gui = std::thread::spawn(move || {
            let r = rx.recv().unwrap();
            assert!(matches!(r.kind, PromptKind::HostKey(_)));
            r.answer_host_key(true);
            let r = rx.recv().unwrap();
            assert!(matches!(&r.kind, PromptKind::Passphrase(p) if p.ends_with("id")));
            r.answer_passphrase(Some(Secret::new("pw")));
            drop(rx.recv().unwrap()); // dismissed without answer
        });
        rt.block_on(async {
            assert!(p.confirm_host_key(&prompt()).await);
            let s = p.passphrase(Path::new("/k/id")).await.unwrap();
            assert_eq!(s.expose(), "pw");
            assert!(!p.confirm_host_key(&prompt()).await);
        });
        gui.join().unwrap();
    }

    // TC-134/TC-135 (GUI part): certificate and password answers reach the
    // vSphere layer; a dismissed password question means "cancel".
    #[test]
    fn vsphere_answers() {
        use vi_ssh::{CertificatePrompt, PasswordPrompt};
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (tx, rx) = mpsc::channel();
        let p = GuiPrompter::new(tx, Arc::new(|| {}));
        let gui = std::thread::spawn(move || {
            let r = rx.recv().unwrap();
            assert!(matches!(&r.kind, PromptKind::Certificate(c) if c.target == "esx:443"));
            r.answer_certificate(true);
            let r = rx.recv().unwrap();
            assert!(matches!(&r.kind, PromptKind::Password(w) if w.user == "reader"));
            r.answer_password(Some(Secret::new("pw")));
            drop(rx.recv().unwrap());
        });
        let cert = CertificatePrompt {
            name: "esx".into(),
            target: "esx:443".into(),
            fingerprint: "SHA256:00".into(),
            subject: "CN=esx".into(),
            issuer: "CN=esx".into(),
            not_before: "2026-01-01".into(),
            not_after: "2036-01-01".into(),
        };
        let pw = PasswordPrompt {
            name: "esx".into(),
            target: "esx:443".into(),
            user: "reader".into(),
            retry: false,
        };
        rt.block_on(async {
            assert!(p.confirm_certificate(&cert).await);
            assert_eq!(p.password(&pw).await.unwrap().expose(), "pw");
            assert!(p.password(&pw).await.is_none());
        });
        gui.join().unwrap();
    }
}
