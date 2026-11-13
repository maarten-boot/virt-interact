//! Background refresh (SPEC-003, SPEC-012): every refresh runs on a tokio
//! runtime, never on the GUI thread. Results come back as [`Event`]s.
//!
//! - One SSH connection per host is kept open and reused, so a refresh does
//!   not ask for a passphrase again. If a reused connection turns out to be
//!   dead, the worker reconnects once.
//! - Hosts run in parallel; a slow host does not delay the others.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use vi_backend::{BackendError, BackendErrorKind};
use vi_core::cache::now_secs;
use vi_core::config::{HostConfig, SshSettings};
use vi_core::model::HostInventory;
use vi_ssh::prompt::BoxFuture;
use vi_ssh::{CommandRunner, ConnectOptions, Prompter, SshConfig, SshSession, resolve};

/// What the GUI shows for a host (SPEC-012).
#[derive(Debug, Clone, PartialEq)]
pub enum HostStatus {
    Idle,
    Connecting,
    Refreshing,
    /// Last refresh succeeded at this time (Unix seconds).
    Ok(u64),
    Error(HostError),
}

impl HostStatus {
    pub fn busy(&self) -> bool {
        matches!(self, HostStatus::Connecting | HostStatus::Refreshing)
    }

    pub fn label(&self) -> &'static str {
        match self {
            HostStatus::Idle => "idle",
            HostStatus::Connecting => "connecting…",
            HostStatus::Refreshing => "refreshing…",
            HostStatus::Ok(_) => "ok",
            HostStatus::Error(_) => "error",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HostError {
    pub kind: BackendErrorKind,
    /// Plain-language message (SPEC-013).
    pub message: String,
    /// Underlying detail (stderr), if any.
    pub detail: Option<String>,
    pub at: u64,
}

impl HostError {
    fn from(e: &BackendError) -> HostError {
        HostError {
            kind: e.kind(),
            message: e.to_string(),
            detail: e.detail().map(str::to_string),
            at: now_secs(),
        }
    }
}

pub enum Event {
    Status {
        host: String,
        status: HostStatus,
    },
    Inventory {
        host: String,
        inventory: HostInventory,
        taken_at: u64,
    },
}

/// Opens a connection to a host. The real one uses SSH; tests use mocks.
pub trait Connector: Send + Sync {
    fn connect<'a>(
        &'a self,
        host: &'a HostConfig,
        ssh: &'a SshSettings,
        prompter: &'a dyn Prompter,
    ) -> BoxFuture<'a, Result<Arc<dyn CommandRunner>, BackendError>>;
}

/// Connects over SSH with the user's ssh config and known_hosts.
pub struct SshConnector {
    pub known_hosts: PathBuf,
}

impl SshConnector {
    pub fn new() -> SshConnector {
        SshConnector {
            known_hosts: ConnectOptions::default_known_hosts()
                .unwrap_or_else(|| PathBuf::from("known_hosts")),
        }
    }
}

impl Default for SshConnector {
    fn default() -> Self {
        SshConnector::new()
    }
}

impl Connector for SshConnector {
    fn connect<'a>(
        &'a self,
        host: &'a HostConfig,
        ssh: &'a SshSettings,
        prompter: &'a dyn Prompter,
    ) -> BoxFuture<'a, Result<Arc<dyn CommandRunner>, BackendError>> {
        Box::pin(async move {
            let cfg = SshConfig::load_default().unwrap_or_default();
            let home = dirs::home_dir().unwrap_or_default();
            let user = vi_ssh::sshconfig::local_user().unwrap_or_else(|| "root".into());
            let resolved = resolve(host, &cfg, &user, &home);
            let opts = ConnectOptions::from_settings(ssh, self.known_hosts.clone());
            let session = SshSession::connect(resolved, &opts, prompter).await?;
            Ok(Arc::new(session) as Arc<dyn CommandRunner>)
        })
    }
}

type Sessions = Arc<Mutex<HashMap<String, Arc<dyn CommandRunner>>>>;

pub struct Worker {
    rt: tokio::runtime::Runtime,
    events: Sender<Event>,
    connector: Arc<dyn Connector>,
    prompter: Arc<dyn Prompter>,
    notify: Arc<dyn Fn() + Send + Sync>,
    sessions: Sessions,
    running: Arc<Mutex<HashSet<String>>>,
    /// Extra per-command limit in the backend (tests; an SSH session has
    /// its own command timeout).
    pub command_timeout: Option<Duration>,
}

impl Worker {
    /// `notify` wakes the GUI after each event.
    pub fn new(
        connector: Arc<dyn Connector>,
        prompter: Arc<dyn Prompter>,
        notify: Arc<dyn Fn() + Send + Sync>,
    ) -> (Worker, Receiver<Event>) {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("vi-worker")
            .build()
            .expect("tokio runtime");
        let (events, rx) = channel();
        (
            Worker {
                rt,
                events,
                connector,
                prompter,
                notify,
                sessions: Arc::default(),
                running: Arc::default(),
                command_timeout: None,
            },
            rx,
        )
    }

    pub fn is_running(&self, host: &str) -> bool {
        self.running.lock().expect("lock").contains(host)
    }

    /// Start a refresh of `host` unless one is already running. Returns
    /// whether a refresh was started.
    pub fn refresh(&self, host: HostConfig, ssh: SshSettings) -> bool {
        if !self.running.lock().expect("lock").insert(host.name.clone()) {
            return false;
        }
        let ctx = TaskCtx {
            events: self.events.clone(),
            connector: self.connector.clone(),
            prompter: self.prompter.clone(),
            notify: self.notify.clone(),
            sessions: self.sessions.clone(),
            command_timeout: self.command_timeout,
        };
        let running = self.running.clone();
        self.rt.spawn(async move {
            let name = host.name.clone();
            ctx.refresh(host, ssh).await;
            running.lock().expect("lock").remove(&name);
            (ctx.notify)();
        });
        true
    }

    /// Drop the kept connection of a host (host edited or removed).
    pub fn forget(&self, host: &str) {
        self.sessions.lock().expect("lock").remove(host);
    }

    pub fn forget_all(&self) {
        self.sessions.lock().expect("lock").clear();
    }
}

struct TaskCtx {
    events: Sender<Event>,
    connector: Arc<dyn Connector>,
    prompter: Arc<dyn Prompter>,
    notify: Arc<dyn Fn() + Send + Sync>,
    sessions: Sessions,
    command_timeout: Option<Duration>,
}

impl TaskCtx {
    fn send(&self, e: Event) {
        let _ = self.events.send(e);
        (self.notify)();
    }

    fn status(&self, host: &str, status: HostStatus) {
        self.send(Event::Status {
            host: host.to_string(),
            status,
        });
    }

    async fn refresh(&self, host: HostConfig, ssh: SshSettings) {
        let name = host.name.clone();
        // Only the backend trait is used here (SPEC-017, TC-090).
        let backend = match vi_backend::for_host_with(&host, self.command_timeout) {
            Ok(b) => b,
            Err(e) => return self.status(&name, HostStatus::Error(HostError::from(&e))),
        };
        let existing = self.sessions.lock().expect("lock").get(&name).cloned();
        let reused = existing.is_some();
        let runner = match existing {
            Some(r) => r,
            None => match self.connect(&host, &ssh).await {
                Ok(r) => r,
                Err(e) => return self.status(&name, HostStatus::Error(HostError::from(&e))),
            },
        };
        self.status(&name, HostStatus::Refreshing);
        let mut result = backend.inventory(&*runner).await;
        if reused && result.as_ref().is_err_and(connection_lost) {
            // The kept connection died (host rebooted, network change):
            // reconnect once.
            tracing::info!(host = %name, "connection lost; reconnecting");
            self.sessions.lock().expect("lock").remove(&name);
            match self.connect(&host, &ssh).await {
                Ok(r) => {
                    self.status(&name, HostStatus::Refreshing);
                    result = backend.inventory(&*r).await;
                }
                Err(e) => result = Err(e),
            }
        }
        match result {
            Ok(inventory) => {
                let taken_at = now_secs();
                self.send(Event::Inventory {
                    host: name.clone(),
                    inventory,
                    taken_at,
                });
                self.status(&name, HostStatus::Ok(taken_at));
            }
            Err(e) => {
                if connection_lost(&e) {
                    self.sessions.lock().expect("lock").remove(&name);
                }
                tracing::warn!(host = %name, error = %e, "refresh failed");
                self.status(&name, HostStatus::Error(HostError::from(&e)));
            }
        }
    }

    async fn connect(
        &self,
        host: &HostConfig,
        ssh: &SshSettings,
    ) -> Result<Arc<dyn CommandRunner>, BackendError> {
        self.status(&host.name, HostStatus::Connecting);
        let r = self
            .connector
            .connect(host, ssh, self.prompter.as_ref())
            .await?;
        self.sessions
            .lock()
            .expect("lock")
            .insert(host.name.clone(), r.clone());
        Ok(r)
    }
}

/// SSH-level failures after which the connection is not reused.
fn connection_lost(e: &BackendError) -> bool {
    matches!(
        e.kind(),
        BackendErrorKind::Connection | BackendErrorKind::Other
    ) && matches!(e, BackendError::Ssh(_))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;
    use vi_backend::{LibvirtBackend, VirshCommand};
    use vi_ssh::{CommandOutput, MockRunner, NoPrompt, SshError};

    const LIST: &str = " Id   Name   State\n------------------\n 1    vm1    running\n";

    fn host(name: &str) -> HostConfig {
        HostConfig::new(name, format!("{name}.example.lan"))
    }

    fn healthy(delay: Option<Duration>) -> MockRunner {
        let b = LibvirtBackend::new("qemu:///system", None).unwrap();
        let line = |c: VirshCommand| b.command_line(&c);
        let m = MockRunner::new()
            .with(
                &line(VirshCommand::Version),
                CommandOutput::ok("Using library: libvirt 12.0.0\n"),
            )
            .with(&line(VirshCommand::ListAll), CommandOutput::ok(LIST))
            .with(
                &line(VirshCommand::DomInfo("vm1".into())),
                CommandOutput::ok("State: running\n"),
            )
            .with(
                &line(VirshCommand::DumpXml("vm1".into())),
                CommandOutput::ok("<domain><name>vm1</name></domain>"),
            );
        match delay {
            Some(d) => m.with_delay(&line(VirshCommand::Version), d),
            None => m,
        }
    }

    /// Hosts named `hang*` never answer `virsh version`; `down*` refuse the
    /// connection; anything else answers at once.
    struct MockConnector {
        connects: AtomicUsize,
    }

    impl Connector for MockConnector {
        fn connect<'a>(
            &'a self,
            host: &'a HostConfig,
            _: &'a SshSettings,
            _: &'a dyn Prompter,
        ) -> BoxFuture<'a, Result<Arc<dyn CommandRunner>, BackendError>> {
            self.connects.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                if host.name.starts_with("down") {
                    return Err(BackendError::Ssh(SshError::Connect {
                        addr: format!("{}:22", host.host),
                        source: std::io::Error::from(std::io::ErrorKind::ConnectionRefused),
                    }));
                }
                let delay = host
                    .name
                    .starts_with("hang")
                    .then_some(Duration::from_secs(600));
                Ok(Arc::new(healthy(delay)) as Arc<dyn CommandRunner>)
            })
        }
    }

    fn worker() -> (Worker, Receiver<Event>, Arc<MockConnector>) {
        let c = Arc::new(MockConnector {
            connects: AtomicUsize::new(0),
        });
        let (mut w, rx) = Worker::new(c.clone(), Arc::new(NoPrompt), Arc::new(|| {}));
        w.command_timeout = Some(Duration::from_millis(500));
        (w, rx, c)
    }

    /// Collect events until every host in `hosts` has a final status.
    fn until_done(rx: &Receiver<Event>, hosts: &[&str]) -> HashMap<String, Vec<HostStatus>> {
        let mut seen: HashMap<String, Vec<HostStatus>> = HashMap::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        while hosts.iter().any(|h| {
            !seen
                .get(*h)
                .and_then(|v| v.last())
                .is_some_and(|s| matches!(s, HostStatus::Ok(_) | HostStatus::Error(_)))
        }) {
            let left = deadline.saturating_duration_since(Instant::now());
            match rx.recv_timeout(left).expect("event before deadline") {
                Event::Status { host, status } => seen.entry(host).or_default().push(status),
                Event::Inventory {
                    host, inventory, ..
                } => {
                    assert_eq!(inventory.domains.len(), 1, "{host}");
                }
            }
        }
        seen
    }

    // TC-062: idle → connecting → refreshing → ok / error.
    #[test]
    fn status_transitions() {
        let (w, rx, _) = worker();
        assert!(w.refresh(host("a"), SshSettings::default()));
        assert!(w.refresh(host("down1"), SshSettings::default()));
        let s = until_done(&rx, &["a", "down1"]);
        let a = &s["a"];
        assert_eq!(a[0], HostStatus::Connecting);
        assert_eq!(a[1], HostStatus::Refreshing);
        assert!(matches!(a[2], HostStatus::Ok(_)));
        let d = &s["down1"];
        assert_eq!(d[0], HostStatus::Connecting);
        match &d[1] {
            HostStatus::Error(e) => assert_eq!(e.kind, BackendErrorKind::Connection),
            other => panic!("{other:?}"),
        }
    }

    // TC-060: one host hangs; the others finish; the hanging one times out.
    #[test]
    fn hanging_host_does_not_block_others() {
        let (w, rx, _) = worker();
        let start = Instant::now();
        for h in ["hang1", "b", "c"] {
            w.refresh(host(h), SshSettings::default());
        }
        let s = until_done(&rx, &["hang1", "b", "c"]);
        assert!(matches!(s["b"].last(), Some(HostStatus::Ok(_))));
        assert!(matches!(s["c"].last(), Some(HostStatus::Ok(_))));
        match s["hang1"].last() {
            Some(HostStatus::Error(e)) => assert_eq!(e.kind, BackendErrorKind::Timeout),
            other => panic!("{other:?}"),
        }
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    // TC-066: the connection is kept and reused; a second refresh of a busy
    // host is not started.
    #[test]
    fn connection_is_reused() {
        let (w, rx, c) = worker();
        assert!(w.refresh(host("a"), SshSettings::default()));
        let _ = until_done(&rx, &["a"]);
        while w.is_running("a") {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(w.refresh(host("a"), SshSettings::default()));
        let s = until_done(&rx, &["a"]);
        assert_eq!(s["a"][0], HostStatus::Refreshing, "no new connect");
        assert_eq!(c.connects.load(Ordering::SeqCst), 1);
        w.forget("a");
        while w.is_running("a") {
            std::thread::sleep(Duration::from_millis(5));
        }
        w.refresh(host("a"), SshSettings::default());
        let _ = until_done(&rx, &["a"]);
        assert_eq!(c.connects.load(Ordering::SeqCst), 2);

        w.refresh(host("hang2"), SshSettings::default());
        assert!(!w.refresh(host("hang2"), SshSettings::default()));
    }
}
