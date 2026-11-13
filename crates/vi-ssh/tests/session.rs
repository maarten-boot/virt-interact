//! Integration tests for `SshSession` against an in-process russh server on
//! 127.0.0.1 (no real host needed; runs in CI on all platforms).

mod common;

use std::time::Duration;

use common::{
    PASSPHRASE, fixture, host, opts, pubkey, start_server, start_server_with, start_switching_proxy,
};
use tokio::net::TcpListener;
use vi_ssh::known_hosts;
use vi_ssh::{CommandRunner, NoPrompt, ScriptedPrompter, SshError, SshErrorKind, SshSession};

// TC-034 / TC-036 (local): unknown host → prompt → learned → exec works;
// the second connection does not prompt.
#[tokio::test]
async fn unknown_host_is_learned_and_exec_works() {
    let port = start_server(vec![pubkey("client_ed25519")]).await;
    let dir = tempfile::tempdir().unwrap();
    let o = opts(&dir);
    let p = ScriptedPrompter::new(true, None);

    let s = SshSession::connect(host(port, &["client_ed25519"]), &o, &p)
        .await
        .unwrap();
    let prompts = p.host_key_prompts.lock().unwrap().clone();
    assert_eq!(prompts.len(), 1);
    assert_eq!(prompts[0].key_type, "ssh-ed25519");
    assert_eq!(
        prompts[0].fingerprint,
        known_hosts::fingerprint(&pubkey("server_ed25519"))
    );

    let out = s.exec("echo hello").await.unwrap();
    assert_eq!(out.stdout_text(), "hello\n");
    assert_eq!(out.exit_status, Some(0));
    s.close().await;

    let kh = std::fs::read_to_string(dir.path().join("known_hosts")).unwrap();
    assert!(kh.starts_with("|1|"), "hashed entry expected: {kh}");

    // Second time: known, no prompt needed.
    let s = SshSession::connect(host(port, &["client_ed25519"]), &o, &NoPrompt)
        .await
        .unwrap();
    assert!(s.exec("echo hello").await.unwrap().success());
}

// SPEC-008: rejecting an unknown key aborts and records nothing.
#[tokio::test]
async fn rejected_host_key() {
    let port = start_server(vec![pubkey("client_ed25519")]).await;
    let dir = tempfile::tempdir().unwrap();
    let err = SshSession::connect(host(port, &["client_ed25519"]), &opts(&dir), &NoPrompt)
        .await
        .err()
        .unwrap();
    assert!(matches!(err, SshError::HostKeyRejected { .. }), "{err}");
    assert_eq!(err.kind(), SshErrorKind::HostKey);
    assert!(!dir.path().join("known_hosts").exists());
}

// SPEC-008 / TC-033: a changed key is refused without asking.
#[tokio::test]
async fn mismatched_host_key_is_refused() {
    let port = start_server(vec![pubkey("client_ed25519")]).await;
    let dir = tempfile::tempdir().unwrap();
    let o = opts(&dir);
    known_hosts::learn(
        &o.known_hosts,
        "127.0.0.1",
        port,
        &pubkey("other_ed25519"),
        false,
    )
    .unwrap();
    let p = ScriptedPrompter::new(true, None);
    let err = SshSession::connect(host(port, &["client_ed25519"]), &o, &p)
        .await
        .err()
        .unwrap();
    match &err {
        SshError::HostKeyMismatch { line, .. } => assert_eq!(*line, 1),
        e => panic!("expected mismatch, got {e}"),
    }
    assert!(err.to_string().contains("CHANGED"));
    assert!(p.host_key_prompts.lock().unwrap().is_empty());
}

// TC-037 (local): encrypted key with passphrase; wrong passphrase fails.
#[tokio::test]
async fn encrypted_key_with_passphrase() {
    let port = start_server(vec![pubkey("client_enc_ed25519")]).await;
    let dir = tempfile::tempdir().unwrap();
    let o = opts(&dir);
    let p = ScriptedPrompter::new(true, Some(PASSPHRASE));
    let s = SshSession::connect(host(port, &["client_enc_ed25519"]), &o, &p)
        .await
        .unwrap();
    assert!(s.exec("echo hello").await.unwrap().success());
    assert_eq!(p.passphrase_prompts.lock().unwrap().len(), 1);

    let wrong = ScriptedPrompter::new(true, Some("nope"));
    let err = SshSession::connect(host(port, &["client_enc_ed25519"]), &o, &wrong)
        .await
        .err()
        .unwrap();
    match err {
        SshError::AuthFailed { tried, .. } => {
            assert!(tried[0].contains("wrong passphrase"), "{tried:?}")
        }
        e => panic!("expected auth failure, got {e}"),
    }
}

// SPEC-006/013: no accepted key → AuthFailed listing what was tried.
#[tokio::test]
async fn auth_failure_lists_tried_keys() {
    let port = start_server(vec![pubkey("client_rsa")]).await;
    let dir = tempfile::tempdir().unwrap();
    let mut h = host(port, &["client_ed25519", "does_not_exist"]);
    h.identity_files.push(fixture("other_ed25519"));
    let err = SshSession::connect(h, &opts(&dir), &ScriptedPrompter::new(true, None))
        .await
        .err()
        .unwrap();
    assert_eq!(err.kind(), SshErrorKind::Authentication);
    match err {
        SshError::AuthFailed { tried, user, .. } => {
            assert_eq!(user, "tester");
            assert_eq!(tried.len(), 3, "{tried:?}");
            assert!(tried[1].contains("not found"));
        }
        e => panic!("expected auth failure, got {e}"),
    }
}

// RSA keys authenticate with rsa-sha2 signatures.
#[tokio::test]
async fn rsa_key_works() {
    let port = start_server(vec![pubkey("client_rsa")]).await;
    let dir = tempfile::tempdir().unwrap();
    let s = SshSession::connect(
        host(port, &["client_rsa"]),
        &opts(&dir),
        &ScriptedPrompter::new(true, None),
    )
    .await
    .unwrap();
    assert!(s.exec("echo hello").await.unwrap().success());
}

// SPEC-012/013: exit status, stderr, command timeout; via the CommandRunner trait.
#[tokio::test]
async fn exit_status_stderr_and_timeout() {
    let port = start_server(vec![pubkey("client_ed25519")]).await;
    let dir = tempfile::tempdir().unwrap();
    let mut o = opts(&dir);
    o.command_timeout = Duration::from_millis(500);
    let s = SshSession::connect(
        host(port, &["client_ed25519"]),
        &o,
        &ScriptedPrompter::new(true, None),
    )
    .await
    .unwrap();
    let runner: &dyn CommandRunner = &s;

    let out = runner.run("fail").await.unwrap();
    assert_eq!(out.exit_status, Some(3));
    assert_eq!(out.stderr_text(), "boom\n");
    assert!(!out.success());

    let err = runner.run("hang").await.err().unwrap();
    assert_eq!(err.kind(), SshErrorKind::Timeout, "{err}");

    // The session is still usable after a timed-out command.
    assert_eq!(
        runner.run("echo hello").await.unwrap().stdout_text(),
        "hello\n"
    );
}

// SPEC-013: connection refused is a connection error.
#[tokio::test]
async fn connection_refused() {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    drop(l);
    let dir = tempfile::tempdir().unwrap();
    let err = SshSession::connect(host(port, &["client_ed25519"]), &opts(&dir), &NoPrompt)
        .await
        .err()
        .unwrap();
    assert_eq!(err.kind(), SshErrorKind::Connection, "{err}");
}

// C1 / TC-039b: the server hangs up after too many attempts → AuthFailed
// with the keys tried and a note, not a generic protocol error.
#[tokio::test]
async fn server_closing_during_auth_keeps_tried_list() {
    let port = start_server_with("server_ed25519", vec![pubkey("client_rsa")], 1).await;
    let dir = tempfile::tempdir().unwrap();
    let h = host(port, &["client_ed25519", "other_ed25519", "client_rsa"]);
    let err = SshSession::connect(h, &opts(&dir), &ScriptedPrompter::new(true, None))
        .await
        .err()
        .unwrap();
    match err {
        SshError::AuthFailed { tried, .. } => {
            assert!(tried[0].contains("client_ed25519"), "{tried:?}");
            assert!(
                tried.last().unwrap().contains("closed the connection"),
                "{tried:?}"
            );
        }
        e => panic!("expected AuthFailed, got {e}"),
    }
}

// C2 / TC-034a: the user accepts the key of the first connection; the
// reconnect presents another key → clear "changed between attempts" error.
#[tokio::test]
async fn host_key_changed_between_attempts() {
    let a = start_server_with("server_ed25519", vec![pubkey("client_ed25519")], 10).await;
    let b = start_server_with("other_ed25519", vec![pubkey("client_ed25519")], 10).await;
    let proxy = start_switching_proxy(vec![a, b]).await;
    let dir = tempfile::tempdir().unwrap();
    let p = ScriptedPrompter::new(true, None);
    let err = SshSession::connect(host(proxy, &["client_ed25519"]), &opts(&dir), &p)
        .await
        .err()
        .unwrap();
    assert!(
        matches!(err, SshError::HostKeyChangedDuringConnect { .. }),
        "{err}"
    );
    assert!(err.to_string().contains("changed between"));
    assert_eq!(err.kind(), SshErrorKind::HostKey);
    assert_eq!(p.host_key_prompts.lock().unwrap().len(), 1);
}

// Suggestion 4 / TC-033c: a host known only in a system-wide known_hosts file
// connects without a prompt; nothing is written to the user's file.
#[tokio::test]
async fn system_known_hosts_is_used() {
    let port = start_server(vec![pubkey("client_ed25519")]).await;
    let dir = tempfile::tempdir().unwrap();
    let mut o = opts(&dir);
    let global = dir.path().join("ssh_known_hosts");
    known_hosts::learn(&global, "127.0.0.1", port, &pubkey("server_ed25519"), false).unwrap();
    o.global_known_hosts = vec![dir.path().join("missing_global"), global];
    let p = ScriptedPrompter::new(false, None);
    let s = SshSession::connect(host(port, &["client_ed25519"]), &o, &p)
        .await
        .unwrap();
    assert!(s.exec("echo hello").await.unwrap().success());
    assert!(p.host_key_prompts.lock().unwrap().is_empty());
    assert!(!o.known_hosts.exists());
}
