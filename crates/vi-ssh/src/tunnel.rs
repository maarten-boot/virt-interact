//! Console tunnels (SPEC-018): a listener on `127.0.0.1` with a random free
//! port; every connection to it is forwarded to a console on the host over
//! the host's connection ([`CommandRunner::open_stream`]: `direct-tcpip` or
//! `direct-streamlocal` for SSH).
//!
//! A viewer may open several connections (SPICE uses one per channel), so
//! the listener accepts until the tunnel is closed. Closing (or dropping)
//! the tunnel stops the listener and ends every forwarded connection.
//!
//! The port is only reachable from this machine, but by any local process
//! while the tunnel is open; a console password, if set, still applies.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tokio::io::copy_bidirectional;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use vi_core::model::ConsoleTarget;

use crate::runner::CommandRunner;

/// What a tunnel has done so far, shared with its tasks.
#[derive(Default)]
struct Stats {
    /// Connections being forwarded now.
    active: AtomicUsize,
    /// Connections accepted since the start.
    total: AtomicUsize,
    /// The last forwarding error, for the GUI.
    last_error: Mutex<Option<String>>,
}

/// An open console tunnel. Dropping it closes it.
pub struct Tunnel {
    addr: SocketAddr,
    target: ConsoleTarget,
    stop: watch::Sender<bool>,
    stats: Arc<Stats>,
}

impl Tunnel {
    /// Listen on `127.0.0.1:0` and forward each connection to `target`
    /// through `runner`. Must be called inside a tokio runtime.
    pub async fn start(
        runner: Arc<dyn CommandRunner>,
        target: ConsoleTarget,
    ) -> std::io::Result<Tunnel> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let addr = listener.local_addr()?;
        let (stop, stopped) = watch::channel(false);
        let stats = Arc::new(Stats::default());
        tracing::info!(%addr, %target, "console tunnel open");
        tokio::spawn(accept_loop(
            listener,
            runner,
            target.clone(),
            stopped,
            stats.clone(),
        ));
        Ok(Tunnel {
            addr,
            target,
            stop,
            stats,
        })
    }

    /// The local address a viewer connects to.
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn target(&self) -> &ConsoleTarget {
        &self.target
    }

    /// Connections being forwarded now.
    pub fn active(&self) -> usize {
        self.stats.active.load(Ordering::SeqCst)
    }

    /// Connections accepted since the start.
    pub fn total(&self) -> usize {
        self.stats.total.load(Ordering::SeqCst)
    }

    /// The last error while opening or forwarding a connection.
    pub fn last_error(&self) -> Option<String> {
        self.stats.last_error.lock().expect("lock").clone()
    }

    /// Stop listening and end every forwarded connection.
    pub fn close(self) {}
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
        tracing::info!(addr = %self.addr, target = %self.target, "console tunnel closed");
    }
}

async fn accept_loop(
    listener: TcpListener,
    runner: Arc<dyn CommandRunner>,
    target: ConsoleTarget,
    mut stopped: watch::Receiver<bool>,
    stats: Arc<Stats>,
) {
    loop {
        tokio::select! {
            _ = stopped.changed() => break,
            accepted = listener.accept() => match accepted {
                Ok((sock, peer)) => {
                    stats.total.fetch_add(1, Ordering::SeqCst);
                    tokio::spawn(forward(
                        sock,
                        peer,
                        runner.clone(),
                        target.clone(),
                        stopped.clone(),
                        stats.clone(),
                    ));
                }
                Err(e) => {
                    tracing::warn!(%target, "console tunnel: accept failed: {e}");
                    *stats.last_error.lock().expect("lock") = Some(e.to_string());
                    break;
                }
            },
        }
    }
}

async fn forward(
    mut sock: TcpStream,
    peer: SocketAddr,
    runner: Arc<dyn CommandRunner>,
    target: ConsoleTarget,
    mut stopped: watch::Receiver<bool>,
    stats: Arc<Stats>,
) {
    let opened = tokio::select! {
        _ = stopped.changed() => return,
        r = runner.open_stream(&target, peer) => r,
    };
    let mut remote = match opened {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(%target, "console tunnel: {e}");
            *stats.last_error.lock().expect("lock") = Some(e.to_string());
            return;
        }
    };
    stats.active.fetch_add(1, Ordering::SeqCst);
    tokio::select! {
        _ = stopped.changed() => {}
        r = copy_bidirectional(&mut sock, &mut remote) => match r {
            Ok((up, down)) => tracing::debug!(%target, up, down, "console connection ended"),
            Err(e) => tracing::debug!(%target, "console connection ended: {e}"),
        },
    }
    stats.active.fetch_sub(1, Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::io::{AsyncReadExt, AsyncWriteExt, duplex};

    use super::*;
    use crate::error::SshError;
    use crate::prompt::BoxFuture;
    use crate::runner::{CommandOutput, ConsoleStream};

    /// Answers every stream with an in-memory echo, or refuses.
    struct EchoRunner {
        refuse: bool,
    }

    impl CommandRunner for EchoRunner {
        fn run<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<CommandOutput, SshError>> {
            Box::pin(async { Ok(CommandOutput::ok("")) })
        }

        fn open_stream<'a>(
            &'a self,
            target: &'a ConsoleTarget,
            _: SocketAddr,
        ) -> BoxFuture<'a, Result<Box<dyn ConsoleStream>, SshError>> {
            Box::pin(async move {
                if self.refuse {
                    return Err(SshError::Forward {
                        target: target.to_string(),
                        reason: "refused".into(),
                    });
                }
                let (a, mut b) = duplex(1024);
                tokio::spawn(async move {
                    let mut buf = [0u8; 256];
                    while let Ok(n) = b.read(&mut buf).await {
                        if n == 0 || b.write_all(&buf[..n]).await.is_err() {
                            break;
                        }
                    }
                });
                Ok(Box::new(a) as Box<dyn ConsoleStream>)
            })
        }
    }

    fn target() -> ConsoleTarget {
        ConsoleTarget::Tcp {
            host: "127.0.0.1".into(),
            port: 5900,
        }
    }

    async fn wait_for(f: impl Fn() -> bool) {
        for _ in 0..200 {
            if f() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("condition not reached");
    }

    // TC-101: two connections forwarded at once; closing ends both and
    // stops the listener.
    #[tokio::test]
    async fn forwards_and_closes() {
        let t = Tunnel::start(Arc::new(EchoRunner { refuse: false }), target())
            .await
            .unwrap();
        assert!(t.local_addr().ip().is_loopback());
        let mut c1 = TcpStream::connect(t.local_addr()).await.unwrap();
        let mut c2 = TcpStream::connect(t.local_addr()).await.unwrap();
        for (c, msg) in [(&mut c1, b"one"), (&mut c2, b"two")] {
            c.write_all(msg).await.unwrap();
            let mut buf = [0u8; 3];
            c.read_exact(&mut buf).await.unwrap();
            assert_eq!(&buf, msg);
        }
        assert_eq!((t.active(), t.total()), (2, 2));
        let addr = t.local_addr();
        t.close();
        let mut buf = [0u8; 1];
        let n = tokio::time::timeout(Duration::from_secs(2), c1.read(&mut buf))
            .await
            .expect("connection ended by close")
            .unwrap_or(0);
        assert_eq!(n, 0);
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(TcpStream::connect(addr).await.is_err(), "listener stopped");
    }

    // TC-101: a refused stream closes the local connection and is reported.
    #[tokio::test]
    async fn refused_stream_is_reported() {
        let t = Tunnel::start(Arc::new(EchoRunner { refuse: true }), target())
            .await
            .unwrap();
        let mut c = TcpStream::connect(t.local_addr()).await.unwrap();
        let mut buf = [0u8; 1];
        assert_eq!(c.read(&mut buf).await.unwrap_or(0), 0);
        wait_for(|| t.last_error().is_some()).await;
        let e = t.last_error().unwrap();
        assert!(e.contains("127.0.0.1:5900") && e.contains("refused"), "{e}");
        assert_eq!(t.active(), 0);
    }

    // TC-101: a runner without forwarding (e.g. a mock) refuses clearly.
    #[tokio::test]
    async fn default_runner_cannot_forward() {
        let r = crate::runner::MockRunner::new();
        let peer: SocketAddr = "127.0.0.1:1".parse().unwrap();
        let err = match r.open_stream(&target(), peer).await {
            Err(e) => e.to_string(),
            Ok(_) => panic!("mock forwarded"),
        };
        assert!(err.contains("cannot forward"), "{err}");
    }
}
