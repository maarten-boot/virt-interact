//! TC-039 in its own test binary: `tracing` caches per-callsite interest
//! process-wide, so a thread-local subscriber only sees everything when no
//! other test runs in the same process.

mod common;

use std::io::Write;
use std::sync::{Arc, Mutex};

use common::{PASSPHRASE, host, opts, pubkey, start_server};
use vi_ssh::{ScriptedPrompter, SshSession};

#[derive(Clone)]
struct Capture(Arc<Mutex<Vec<u8>>>);
impl Write for Capture {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

// TC-039: the passphrase never appears in log output, even at trace level.
#[tokio::test]
async fn passphrase_not_logged() {
    let buf = Arc::new(Mutex::new(Vec::new()));
    let w = Capture(buf.clone());
    let sub = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(move || w.clone())
        .finish();
    let _g = tracing::subscriber::set_default(sub);

    let port = start_server(vec![pubkey("client_enc_ed25519")]).await;
    let dir = tempfile::tempdir().unwrap();
    let p = ScriptedPrompter::new(true, Some(PASSPHRASE));
    let s = SshSession::connect(host(port, &["client_enc_ed25519"]), &opts(&dir), &p)
        .await
        .unwrap();
    s.exec("echo hello").await.unwrap();

    let log = String::from_utf8_lossy(&buf.lock().unwrap()).to_string();
    assert!(
        log.contains("authenticated"),
        "log capture did not work: {log}"
    );
    assert!(!log.contains(PASSPHRASE));
}
