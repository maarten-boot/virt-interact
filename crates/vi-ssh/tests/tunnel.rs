//! TC-102a: console tunnels through a real SSH session against the in-process
//! russh server (SPEC-018): `direct-tcpip` to a TCP "console", and on unix
//! `direct-streamlocal` to a socket; a refused target is reported.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{host, opts, pubkey, start_server};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use vi_core::model::ConsoleTarget;
use vi_ssh::{CommandRunner, ScriptedPrompter, SshSession, Tunnel};

async fn session() -> Arc<dyn CommandRunner> {
    let port = start_server(vec![pubkey("client_ed25519")]).await;
    // The tempdir only holds known_hosts; leaking it keeps it for the test.
    let dir = Box::leak(Box::new(tempfile::tempdir().unwrap()));
    let s = SshSession::connect(
        host(port, &["client_ed25519"]),
        &opts(dir),
        &ScriptedPrompter::new(true, None),
    )
    .await
    .unwrap();
    Arc::new(s)
}

/// Echo every byte back; stands in for a VNC/SPICE server.
async fn echo<S: AsyncRead + AsyncWrite + Unpin>(mut s: S) {
    let mut buf = [0u8; 1024];
    while let Ok(n) = s.read(&mut buf).await {
        if n == 0 || s.write_all(&buf[..n]).await.is_err() {
            break;
        }
    }
}

async fn round_trip(t: &Tunnel, msg: &[u8]) {
    let mut c = TcpStream::connect(t.local_addr()).await.unwrap();
    c.write_all(msg).await.unwrap();
    let mut buf = vec![0u8; msg.len()];
    tokio::time::timeout(Duration::from_secs(5), c.read_exact(&mut buf))
        .await
        .expect("echo through the tunnel")
        .unwrap();
    assert_eq!(buf, msg);
}

#[tokio::test]
async fn tcp_console_through_ssh() {
    let console = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = console.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((s, _)) = console.accept().await {
            tokio::spawn(echo(s));
        }
    });
    let t = Tunnel::start(
        session().await,
        ConsoleTarget::Tcp {
            host: "127.0.0.1".into(),
            port,
        },
    )
    .await
    .unwrap();
    // Two connections, as a SPICE viewer opens several channels.
    round_trip(&t, b"RFB 003.008\n").await;
    round_trip(&t, &[7u8; 5000]).await;
    assert_eq!(t.total(), 2);
    assert_eq!(t.last_error(), None);
}

#[cfg(unix)]
#[tokio::test]
async fn socket_console_through_ssh() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("console.sock");
    let console = tokio::net::UnixListener::bind(&path).unwrap();
    tokio::spawn(async move {
        while let Ok((s, _)) = console.accept().await {
            tokio::spawn(echo(s));
        }
    });
    let t = Tunnel::start(
        session().await,
        ConsoleTarget::Socket(path.to_string_lossy().into_owned()),
    )
    .await
    .unwrap();
    round_trip(&t, b"spice").await;
}

#[tokio::test]
async fn refused_target_is_reported() {
    // A port with nothing listening: bind, note the port, close.
    let port = TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let t = Tunnel::start(
        session().await,
        ConsoleTarget::Tcp {
            host: "127.0.0.1".into(),
            port,
        },
    )
    .await
    .unwrap();
    let mut c = TcpStream::connect(t.local_addr()).await.unwrap();
    let mut buf = [0u8; 1];
    let n = tokio::time::timeout(Duration::from_secs(5), c.read(&mut buf))
        .await
        .expect("local connection closed")
        .unwrap_or(0);
    assert_eq!(n, 0);
    for _ in 0..200 {
        if t.last_error().is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let e = t.last_error().expect("error recorded");
    assert!(e.contains(&format!("127.0.0.1:{port}")), "{e}");
}
