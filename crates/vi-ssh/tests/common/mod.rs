//! Shared test helpers: an in-process russh server on 127.0.0.1 and test
//! hosts/options. Keys: `tests/fixtures/ssh` (test-only).
#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use russh::keys::PublicKey;
use russh::server::{self, Auth, Msg, Session, VerifiedPublicKey};
use russh::{Channel, ChannelId};
use tokio::net::TcpListener;
use vi_ssh::{ConnectOptions, ResolvedHost};

pub const PASSPHRASE: &str = "test-passphrase-123";

pub fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/ssh")
        .join(name)
}

pub fn pubkey(name: &str) -> PublicKey {
    russh::keys::load_public_key(fixture(&format!("{name}.pub"))).unwrap()
}

#[derive(Clone)]
struct TestServer {
    allowed: Vec<PublicKey>,
}

impl server::Handler for TestServer {
    type Error = russh::Error;

    async fn auth_publickey(
        &mut self,
        _user: &str,
        key: VerifiedPublicKey<'_>,
    ) -> Result<Auth, Self::Error> {
        if self
            .allowed
            .iter()
            .any(|k| k.key_data() == key.public_key.key_data())
        {
            Ok(Auth::Accept)
        } else {
            Ok(Auth::reject())
        }
    }

    async fn channel_open_session(
        &mut self,
        _channel: Channel<Msg>,
        reply: server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }

    /// `direct-tcpip`: connect to the target from the "host" (this process)
    /// and copy bytes both ways; refuse if the connect fails.
    async fn channel_open_direct_tcpip(
        &mut self,
        channel: Channel<Msg>,
        host_to_connect: &str,
        port_to_connect: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        let addr = format!("{host_to_connect}:{port_to_connect}");
        match tokio::net::TcpStream::connect(addr).await {
            Ok(mut sock) => {
                reply.accept().await;
                tokio::spawn(async move {
                    let mut ch = channel.into_stream();
                    let _ = tokio::io::copy_bidirectional(&mut sock, &mut ch).await;
                });
            }
            Err(_) => reply.reject(russh::ChannelOpenFailure::ConnectFailed).await,
        }
        Ok(())
    }

    /// `direct-streamlocal`: the same for a unix socket (unix only).
    async fn channel_open_direct_streamlocal(
        &mut self,
        channel: Channel<Msg>,
        socket_path: &str,
        reply: server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        #[cfg(unix)]
        match tokio::net::UnixStream::connect(socket_path).await {
            Ok(mut sock) => {
                reply.accept().await;
                tokio::spawn(async move {
                    let mut ch = channel.into_stream();
                    let _ = tokio::io::copy_bidirectional(&mut sock, &mut ch).await;
                });
            }
            Err(_) => reply.reject(russh::ChannelOpenFailure::ConnectFailed).await,
        }
        #[cfg(not(unix))]
        {
            let _ = (channel, socket_path);
            reply
                .reject(russh::ChannelOpenFailure::AdministrativelyProhibited)
                .await;
        }
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)?;
        let cmd = String::from_utf8_lossy(data).to_string();
        let status = match cmd.as_str() {
            "echo hello" => {
                session.data(channel, b"hello\n".to_vec())?;
                0
            }
            "fail" => {
                session.extended_data(channel, 1, b"boom\n".to_vec())?;
                3
            }
            "hang" => return Ok(()),
            _ => {
                session.extended_data(channel, 1, format!("unknown: {cmd}\n").into_bytes())?;
                127
            }
        };
        session.exit_status_request(channel, status)?;
        session.eof(channel)?;
        session.close(channel)?;
        Ok(())
    }
}

/// Start a server accepting `allowed` client keys; returns its port.
pub async fn start_server(allowed: Vec<PublicKey>) -> u16 {
    start_server_with("server_ed25519", allowed, 10).await
}

/// Start a server with the given host key file and auth attempt limit.
pub async fn start_server_with(
    host_key: &str,
    allowed: Vec<PublicKey>,
    max_auth_attempts: usize,
) -> u16 {
    let key = russh::keys::load_secret_key(fixture(host_key), None).unwrap();
    let config = Arc::new(server::Config {
        keys: vec![key],
        auth_rejection_time: Duration::from_millis(10),
        auth_rejection_time_initial: Some(Duration::ZERO),
        max_auth_attempts,
        ..Default::default()
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            let cfg = config.clone();
            let h = TestServer {
                allowed: allowed.clone(),
            };
            tokio::spawn(async move {
                if let Ok(s) = server::run_stream(cfg, sock, h).await {
                    let _ = s.await;
                }
            });
        }
    });
    port
}

/// A TCP proxy that sends connection N to `targets[min(N, last)]`.
/// Used to present a different host key on the second connection.
pub async fn start_switching_proxy(targets: Vec<u16>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let mut n = 0usize;
        while let Ok((mut inbound, _)) = listener.accept().await {
            let target = targets[n.min(targets.len() - 1)];
            n += 1;
            tokio::spawn(async move {
                if let Ok(mut out) = tokio::net::TcpStream::connect(("127.0.0.1", target)).await {
                    let _ = tokio::io::copy_bidirectional(&mut inbound, &mut out).await;
                }
            });
        }
    });
    port
}

pub fn host(port: u16, keys: &[&str]) -> ResolvedHost {
    ResolvedHost {
        name: "test".into(),
        alias: "127.0.0.1".into(),
        hostname: "127.0.0.1".into(),
        port,
        user: "tester".into(),
        identity_files: keys.iter().map(|k| fixture(k)).collect(),
        identity_explicit: true,
        warnings: vec![],
    }
}

pub fn opts(dir: &tempfile::TempDir) -> ConnectOptions {
    let mut o = ConnectOptions::new(dir.path().join("known_hosts"));
    o.use_agent = false; // deterministic: no agent from the environment
    o.connect_timeout = Duration::from_secs(5);
    o.command_timeout = Duration::from_secs(5);
    o
}
