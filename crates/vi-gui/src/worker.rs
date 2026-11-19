//! Background refresh (SPEC-003, SPEC-012): every refresh runs on a tokio
//! runtime, never on the GUI thread. Results come back as [`Event`]s.
//!
//! - One SSH connection per host is kept open and reused, so a refresh does
//!   not ask for a passphrase again. If a reused connection turns out to be
//!   dead, the worker reconnects once.
//! - Hosts run in parallel; a slow host does not delay the others.
//! - Consoles (SPEC-018): a tunnel over the host's connection and an
//!   external viewer; the tunnel lives as long as the viewer.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::oneshot;
use vi_backend::{BackendError, BackendErrorKind};
use vi_core::cache::now_secs;
use vi_core::config::{HostConfig, SshSettings};
use vi_core::model::HostInventory;
use vi_ssh::prompt::BoxFuture;
use vi_ssh::{CommandRunner, ConnectOptions, Prompter, SshConfig, SshSession, Tunnel, resolve};

use crate::console::{ConsoleKey, ConsoleState, viewer_url};

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
    /// A console changed state (SPEC-018).
    Console {
        key: ConsoleKey,
        state: ConsoleState,
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

/// The real connector: local entries (SPEC-020) run on this machine,
/// every other host over SSH with the user's ssh config and known_hosts.
pub struct HostConnector {
    pub known_hosts: PathBuf,
    /// Pinned vSphere certificates (SPEC-023), next to the config file.
    pub known_certs: PathBuf,
}

impl HostConnector {
    pub fn new() -> HostConnector {
        let config_dir = vi_core::paths::default_config_path()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .unwrap_or_default();
        HostConnector {
            known_hosts: ConnectOptions::default_known_hosts()
                .unwrap_or_else(|| PathBuf::from("known_hosts")),
            known_certs: config_dir.join("known_certs"),
        }
    }
}

impl Default for HostConnector {
    fn default() -> Self {
        HostConnector::new()
    }
}

impl Connector for HostConnector {
    fn connect<'a>(
        &'a self,
        host: &'a HostConfig,
        ssh: &'a SshSettings,
        prompter: &'a dyn Prompter,
    ) -> BoxFuture<'a, Result<Arc<dyn CommandRunner>, BackendError>> {
        Box::pin(async move {
            if host.backend == vi_core::config::Backend::Vsphere {
                // SPEC-021/022: an API session, kept like an SSH session.
                let s = vi_vsphere::Settings {
                    name: host.display_name(),
                    host: host.host.clone(),
                    port: host.port_or_default(),
                    user: host.user.clone().unwrap_or_default(),
                    tls: vi_vsphere::TlsOptions {
                        known: vi_vsphere::KnownCerts::new(self.known_certs.clone()),
                        extra_roots: Vec::new(),
                        connect_timeout: Duration::from_secs(ssh.connect_timeout_secs.max(1)),
                    },
                    timeout: Duration::from_secs(ssh.command_timeout_secs.max(1)),
                };
                let session = vi_vsphere::VsphereSession::connect(&s, prompter)
                    .await
                    .map_err(BackendError::from)?;
                return Ok(Arc::new(session) as Arc<dyn CommandRunner>);
            }
            if host.local {
                let r = vi_backend::local::LocalRunner::new(Duration::from_secs(
                    ssh.command_timeout_secs.max(1),
                ));
                return Ok(Arc::new(r) as Arc<dyn CommandRunner>);
            }
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

/// Kept connections, with a generation per host. `forget` raises the
/// generation, so a refresh that was already running when the host was
/// edited, removed or the config reloaded cannot store its (old) connection
/// or report its (old) result (crosscheck 2026-11-14-01, C1).
#[derive(Default)]
struct SessionTable {
    map: HashMap<String, Arc<dyn CommandRunner>>,
    generation: HashMap<String, u64>,
    epoch: u64,
}

/// Identifies the settings a refresh started with.
type Token = (u64, u64);

impl SessionTable {
    fn token(&self, host: &str) -> Token {
        (self.generation.get(host).copied().unwrap_or(0), self.epoch)
    }

    fn forget(&mut self, host: &str) {
        self.map.remove(host);
        *self.generation.entry(host.to_string()).or_default() += 1;
    }

    fn forget_all(&mut self) {
        self.map.clear();
        self.epoch += 1;
    }

    /// Store `r` only if nothing was forgotten since `token` was taken.
    fn store(&mut self, host: &str, token: Token, r: Arc<dyn CommandRunner>) -> bool {
        if self.token(host) != token {
            return false;
        }
        self.map.insert(host.to_string(), r);
        true
    }
}

type Sessions = Arc<Mutex<SessionTable>>;

/// Consoles whose task is running; sending on (or dropping) the sender
/// closes one. Closing takes the sender out but keeps the key until the
/// task has ended, so the same console cannot be opened again while the
/// old viewer is still stopping (and the old task cannot remove the entry
/// of a new one).
type Consoles = Arc<Mutex<HashMap<ConsoleKey, Option<oneshot::Sender<()>>>>>;

pub struct Worker {
    rt: tokio::runtime::Runtime,
    events: Sender<Event>,
    connector: Arc<dyn Connector>,
    prompter: Arc<dyn Prompter>,
    notify: Arc<dyn Fn() + Send + Sync>,
    sessions: Sessions,
    running: Arc<Mutex<HashSet<String>>>,
    consoles: Consoles,
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
                consoles: Arc::default(),
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
        let ctx = self.ctx();
        let running = self.running.clone();
        self.rt.spawn(async move {
            let name = host.name.clone();
            ctx.refresh(host, ssh).await;
            running.lock().expect("lock").remove(&name);
            (ctx.notify)();
        });
        true
    }

    /// Is a connection to `host` kept open?
    pub fn has_session(&self, host: &str) -> bool {
        self.sessions.lock().expect("lock").map.contains_key(host)
    }

    /// Drop the kept connection of a host (host edited or removed). A
    /// refresh still running for it is discarded when it finishes; its
    /// consoles are closed.
    pub fn forget(&self, host: &str) {
        self.sessions.lock().expect("lock").forget(host);
        for (_, close) in self
            .consoles
            .lock()
            .expect("lock")
            .iter_mut()
            .filter(|(k, _)| k.host == host)
        {
            close.take();
        }
    }

    /// Drop all connections and close all consoles (config reloaded).
    pub fn forget_all(&self) {
        self.sessions.lock().expect("lock").forget_all();
        for close in self.consoles.lock().expect("lock").values_mut() {
            close.take();
        }
    }

    fn ctx(&self) -> TaskCtx {
        TaskCtx {
            events: self.events.clone(),
            connector: self.connector.clone(),
            prompter: self.prompter.clone(),
            notify: self.notify.clone(),
            sessions: self.sessions.clone(),
            command_timeout: self.command_timeout,
        }
    }

    /// Is this console opening, open or still stopping?
    pub fn console_open(&self, key: &ConsoleKey) -> bool {
        self.consoles.lock().expect("lock").contains_key(key)
    }

    /// Open a console (SPEC-018): read the live graphics, open a tunnel and
    /// run `viewer` (or report why it cannot be found) until it exits or
    /// [`Worker::close_console`]. Returns false if it is already open.
    pub fn open_console(
        &self,
        host: HostConfig,
        ssh: SshSettings,
        key: ConsoleKey,
        viewer: Result<PathBuf, String>,
    ) -> bool {
        let (close, closed) = oneshot::channel();
        {
            let mut c = self.consoles.lock().expect("lock");
            if c.contains_key(&key) {
                return false;
            }
            c.insert(key.clone(), Some(close));
        }
        let ctx = self.ctx();
        let consoles = self.consoles.clone();
        self.rt.spawn(async move {
            ctx.send(Event::Console {
                key: key.clone(),
                state: ConsoleState::Opening,
            });
            let error = ctx.console(&host, &ssh, &key, viewer, closed).await.err();
            if let Some(e) = &error {
                tracing::warn!(host = %key.host, domain = %key.domain, "console: {e}");
            }
            consoles.lock().expect("lock").remove(&key);
            ctx.send(Event::Console {
                key,
                state: ConsoleState::Closed { error },
            });
        });
        true
    }

    /// Close a console: the viewer is stopped and the tunnel closed.
    pub fn close_console(&self, key: &ConsoleKey) {
        if let Some(tx) = self
            .consoles
            .lock()
            .expect("lock")
            .get_mut(key)
            .and_then(Option::take)
        {
            let _ = tx.send(());
        }
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
        let (token, existing) = {
            let t = self.sessions.lock().expect("lock");
            (t.token(&name), t.map.get(&name).cloned())
        };
        let reused = existing.is_some();
        let runner = match existing {
            Some(r) => r,
            None => match self.connect(&host, &ssh, token).await {
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
            self.sessions.lock().expect("lock").map.remove(&name);
            match self.connect(&host, &ssh, token).await {
                Ok(r) => {
                    self.status(&name, HostStatus::Refreshing);
                    result = backend.inventory(&*r).await;
                }
                Err(e) => result = Err(e),
            }
        }
        if self.sessions.lock().expect("lock").token(&name) != token {
            // Host edited/removed or config reloaded meanwhile: this result
            // belongs to the old settings.
            tracing::info!(host = %name, "discarding refresh started with old settings");
            return self.status(&name, HostStatus::Idle);
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
                    self.sessions.lock().expect("lock").map.remove(&name);
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
        token: Token,
    ) -> Result<Arc<dyn CommandRunner>, BackendError> {
        self.status(&host.name, HostStatus::Connecting);
        self.connect_quiet(host, ssh, token).await
    }

    /// Connect without changing the host's status (for a console: the
    /// inventory status stays as it is).
    async fn connect_quiet(
        &self,
        host: &HostConfig,
        ssh: &SshSettings,
        token: Token,
    ) -> Result<Arc<dyn CommandRunner>, BackendError> {
        let r = self
            .connector
            .connect(host, ssh, self.prompter.as_ref())
            .await?;
        // Not kept if the host was forgotten while connecting (C1); it is
        // still used for this refresh, whose result is then discarded.
        self.sessions
            .lock()
            .expect("lock")
            .store(&host.name, token, r.clone());
        Ok(r)
    }
}

impl TaskCtx {
    /// The kept connection of `host`, or a new one (kept for later).
    async fn runner(
        &self,
        host: &HostConfig,
        ssh: &SshSettings,
    ) -> Result<(Arc<dyn CommandRunner>, bool), BackendError> {
        let (token, existing) = {
            let t = self.sessions.lock().expect("lock");
            (t.token(&host.name), t.map.get(&host.name).cloned())
        };
        match existing {
            Some(r) => Ok((r, true)),
            None => Ok((self.connect_quiet(host, ssh, token).await?, false)),
        }
    }

    /// One console from start to end (SPEC-018). `Err` is shown to the user.
    async fn console(
        &self,
        host: &HostConfig,
        ssh: &SshSettings,
        key: &ConsoleKey,
        viewer: Result<PathBuf, String>,
        mut closed: oneshot::Receiver<()>,
    ) -> Result<(), String> {
        let viewer = viewer?;
        let backend =
            vi_backend::for_host_with(host, self.command_timeout).map_err(|e| e.to_string())?;
        let (mut runner, reused) = self.runner(host, ssh).await.map_err(|e| e.to_string())?;
        let mut graphics = backend.graphics(&*runner, &key.domain).await;
        if reused && graphics.as_ref().is_err_and(connection_lost) {
            tracing::info!(host = %host.name, "connection lost; reconnecting for the console");
            self.sessions.lock().expect("lock").map.remove(&host.name);
            runner = self.runner(host, ssh).await.map_err(|e| e.to_string())?.0;
            graphics = backend.graphics(&*runner, &key.domain).await;
        }
        let graphics = graphics.map_err(|e| e.to_string())?;
        let g = graphics
            .get(key.index)
            .ok_or("this graphics device is no longer in the domain")?;
        let target = g.console_target()?;
        let tunnel = Tunnel::start(runner, target)
            .await
            .map_err(|e| format!("cannot open a local port for the console: {e}"))?;
        let url = viewer_url(&g.kind, tunnel.local_addr());
        let mut cmd = tokio::process::Command::new(&viewer);
        cmd.arg(&url)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        // Its own process group, so helpers of a wrapper script stop too.
        #[cfg(unix)]
        cmd.process_group(0);
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("cannot start {}: {e}", viewer.display()))?;
        let group = ViewerGroup(child.id());
        tracing::info!(host = %key.host, domain = %key.domain, %url, viewer = %viewer.display(), "console open");
        let stderr = child.stderr.take().map(|mut s| {
            tokio::spawn(async move {
                use tokio::io::AsyncReadExt;
                let mut buf = Vec::new();
                let _ = s.read_to_end(&mut buf).await;
                buf
            })
        });
        self.send(Event::Console {
            key: key.clone(),
            state: ConsoleState::Open { url },
        });
        let status = tokio::select! {
            s = child.wait() => Some(s),
            _ = &mut closed => None,
        };
        let tunnel_error = tunnel.last_error();
        let Some(status) = status else {
            // Closed by the user (or the host was forgotten): ask the viewer
            // and its helpers to stop, then force them.
            group.signal(Signal::Term);
            if tokio::time::timeout(Duration::from_secs(2), child.wait())
                .await
                .is_err()
            {
                group.signal(Signal::Kill);
                let _ = child.kill().await;
            } else {
                // The viewer has stopped; give helpers the same time.
                group.wait_or_kill(Duration::from_secs(2)).await;
            }
            tunnel.shutdown().await;
            return Ok(());
        };
        tunnel.shutdown().await;
        let status = status.map_err(|e| format!("viewer: {e}"))?;
        if status.success() {
            return Ok(());
        }
        let mut msg = format!("the viewer exited with {status}");
        if let Some(h) = stderr
            && let Ok(buf) = h.await
        {
            let text = String::from_utf8_lossy(&buf);
            if let Some(last) = text.lines().rev().find(|l| !l.trim().is_empty()) {
                msg.push_str(&format!(": {}", last.trim()));
            }
        }
        if let Some(e) = tunnel_error {
            msg.push_str(&format!(" (tunnel: {e})"));
        }
        Err(msg)
    }
}

/// The viewer's process group (unix). Dropping it sends SIGTERM to the
/// group: on close, when the viewer has ended (leftover helpers), and when
/// the app quits (the worker's runtime drops the task). Elsewhere a no-op;
/// the viewer itself is also killed on drop (`kill_on_drop`).
struct ViewerGroup(#[cfg_attr(not(unix), allow(dead_code))] Option<u32>);

#[derive(Clone, Copy)]
enum Signal {
    Term,
    Kill,
}

impl ViewerGroup {
    /// The group was created for the viewer (process_group(0)), so its id
    /// is the viewer's pid.
    #[cfg(unix)]
    fn pid(&self) -> Option<rustix::process::Pid> {
        self.0
            .and_then(|p| i32::try_from(p).ok())
            .and_then(rustix::process::Pid::from_raw)
    }

    /// Signal the whole group (unix; an empty group is fine).
    fn signal(&self, sig: Signal) {
        #[cfg(unix)]
        if let Some(pid) = self.pid() {
            let sig = match sig {
                Signal::Term => rustix::process::Signal::TERM,
                Signal::Kill => rustix::process::Signal::KILL,
            };
            let _ = rustix::process::kill_process_group(pid, sig);
        }
        #[cfg(not(unix))]
        let _ = sig;
    }

    /// Wait until the group is empty, at most `limit`, then SIGKILL it.
    async fn wait_or_kill(&self, limit: Duration) {
        #[cfg(unix)]
        {
            let Some(pid) = self.pid() else { return };
            let end = tokio::time::Instant::now() + limit;
            while tokio::time::Instant::now() < end {
                if rustix::process::test_kill_process_group(pid).is_err() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            self.signal(Signal::Kill);
        }
        #[cfg(not(unix))]
        let _ = limit;
    }
}

impl Drop for ViewerGroup {
    fn drop(&mut self) {
        // An already empty group is fine.
        self.signal(Signal::Term);
    }
}

/// Failures after which the kept connection is not reused, so the next
/// try connects (SSH) or logs in (vSphere) again once.
fn connection_lost(e: &BackendError) -> bool {
    match e {
        BackendError::Ssh(_) => matches!(
            e.kind(),
            BackendErrorKind::Connection | BackendErrorKind::Other
        ),
        // Connection gone or session expired (NotAuthenticated): log in
        // again, which asks for the password (SPEC-022).
        BackendError::Vsphere(_) => e.kind() == BackendErrorKind::Connection,
        _ => false,
    }
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
                if host.name.starts_with("slow") {
                    tokio::time::sleep(Duration::from_millis(300)).await;
                }
                let delay = host
                    .name
                    .starts_with("hang")
                    .then_some(Duration::from_secs(600));
                if host.name.starts_with("gfx") {
                    // vm1 has a VNC console on 127.0.0.1:5999.
                    let b = LibvirtBackend::new("qemu:///system", None).unwrap();
                    let xml = "<domain><name>vm1</name><devices>\
                        <graphics type='vnc' port='5999' listen='127.0.0.1'/>\
                        </devices></domain>";
                    return Ok(Arc::new(healthy(None).with(
                        &b.command_line(&VirshCommand::DumpXml("vm1".into())),
                        CommandOutput::ok(xml),
                    )) as Arc<dyn CommandRunner>);
                }
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
                Event::Console { .. } => {}
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

    fn wait_idle(w: &Worker, host: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while w.is_running(host) {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    // TC-066a: a host forgotten while connecting does not keep the old
    // connection, and the old result is not reported.
    #[test]
    fn forget_during_connect_discards_old_connection() {
        let (w, rx, c) = worker();
        assert!(w.refresh(host("slow1"), SshSettings::default()));
        std::thread::sleep(Duration::from_millis(50));
        w.forget("slow1");
        wait_idle(&w, "slow1");
        let statuses: Vec<HostStatus> = rx
            .try_iter()
            .map(|e| match e {
                Event::Status { status, .. } => status,
                Event::Inventory { .. } | Event::Console { .. } => panic!("unexpected event"),
            })
            .collect();
        assert_eq!(statuses.last(), Some(&HostStatus::Idle), "{statuses:?}");
        assert!(!w.has_session("slow1"));

        // Same with forget_all (config reload).
        w.refresh(host("slow2"), SshSettings::default());
        std::thread::sleep(Duration::from_millis(50));
        w.forget_all();
        wait_idle(&w, "slow2");
        assert!(!w.has_session("slow2"));

        // The next refresh connects again and keeps that connection.
        w.refresh(host("slow1"), SshSettings::default());
        let s = until_done(&rx, &["slow1"]);
        assert!(matches!(s["slow1"].last(), Some(HostStatus::Ok(_))));
        assert!(w.has_session("slow1"));
        assert_eq!(c.connects.load(Ordering::SeqCst), 3);
    }

    // TC-115: the real connector runs local entries on this machine, with
    // no SSH (no prompts, no known_hosts).
    #[cfg(unix)]
    #[test]
    fn local_entries_do_not_use_ssh() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut cfg = vi_core::config::Config::default();
        cfg.local.enabled = vi_core::config::LocalMode::Always;
        let local = cfg.local_hosts(false).remove(0);
        let c = HostConnector {
            known_hosts: PathBuf::from("/nonexistent/known_hosts"),
            known_certs: PathBuf::from("/nonexistent/known_certs"),
        };
        rt.block_on(async {
            let r = c
                .connect(&local, &SshSettings::default(), &NoPrompt)
                .await
                .expect("local runner");
            let o = r.run("printf local").await.unwrap();
            assert_eq!(o.stdout_text(), "local");
        });
    }

    /// Console events of one key until it is closed; returns all states.
    fn console_states(rx: &Receiver<Event>) -> Vec<ConsoleState> {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut v = Vec::new();
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if let Event::Console { state, .. } = rx.recv_timeout(left).expect("console event") {
                let done = matches!(state, ConsoleState::Closed { .. });
                v.push(state);
                if done {
                    return v;
                }
            }
        }
    }

    fn key(host: &str) -> ConsoleKey {
        ConsoleKey {
            host: host.into(),
            domain: "vm1".into(),
            index: 0,
        }
    }

    /// A shell script standing in for remote-viewer.
    #[cfg(unix)]
    fn script(dir: &std::path::Path, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join("viewer.sh");
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    // TC-103: open → the viewer gets the tunnel URL → it exits → closed
    // without error, and the tunnel port is closed with it.
    #[cfg(unix)]
    #[test]
    fn console_lives_as_long_as_the_viewer() {
        let (w, rx, _) = worker();
        let d = tempfile::tempdir().unwrap();
        let out = d.path().join("url");
        let v = script(
            d.path(),
            &format!("echo \"$1\" > '{}'; sleep 0.3", out.display()),
        );
        assert!(w.open_console(host("gfx1"), SshSettings::default(), key("gfx1"), Ok(v)));
        assert!(w.console_open(&key("gfx1")));
        let states = console_states(&rx);
        assert_eq!(states[0], ConsoleState::Opening);
        let ConsoleState::Open { url } = &states[1] else {
            panic!("{states:?}")
        };
        assert!(url.starts_with("vnc://127.0.0.1:"), "{url}");
        assert_eq!(states[2], ConsoleState::Closed { error: None });
        assert_eq!(std::fs::read_to_string(&out).unwrap().trim(), url);
        assert!(!w.console_open(&key("gfx1")));
        let port = url.rsplit(':').next().unwrap();
        assert!(std::net::TcpStream::connect(format!("127.0.0.1:{port}")).is_err());
        // The host status was not touched (no "connecting" for a console).
        assert!(rx.try_iter().all(|e| !matches!(e, Event::Status { .. })));
    }

    /// Wait for the Open event of a console.
    #[cfg(unix)]
    fn wait_open(rx: &Receiver<Event>) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if let Event::Console {
                state: ConsoleState::Open { .. },
                ..
            } = rx.recv_timeout(left).expect("console event")
            {
                return;
            }
        }
    }

    /// The pid in `pidfile` once the wrapper script has written it.
    #[cfg(unix)]
    fn helper_pid(pidfile: &std::path::Path) -> String {
        let started = (0..500).any(|_| {
            std::thread::sleep(Duration::from_millis(10));
            std::fs::read_to_string(pidfile).is_ok_and(|s| s.ends_with('\n'))
        });
        assert!(started, "helper not started");
        std::fs::read_to_string(pidfile).unwrap().trim().to_string()
    }

    #[cfg(unix)]
    fn gone(pid: &str) -> bool {
        (0..250).any(|_| {
            std::thread::sleep(Duration::from_millis(20));
            !std::process::Command::new("kill")
                .args(["-0", pid])
                .status()
                .unwrap()
                .success()
        })
    }

    // TC-103: Close stops a viewer that would run for a long time, and the
    // helpers of a wrapper script, also when they ignore SIGTERM (SIGKILL to
    // the group after 2 s); forget (host removed) does the same.
    #[cfg(unix)]
    #[test]
    fn close_console_stops_the_viewer() {
        let (w, rx, _) = worker();
        let d = tempfile::tempdir().unwrap();
        // A wrapper script and its helper, both ignoring SIGTERM.
        let pidfile = d.path().join("helper.pid");
        let v = script(
            d.path(),
            &format!(
                "trap '' TERM; sleep 30 & echo $! > '{}'; wait",
                pidfile.display()
            ),
        );
        let start = Instant::now();
        w.open_console(
            host("gfx2"),
            SshSettings::default(),
            key("gfx2"),
            Ok(v.clone()),
        );
        wait_open(&rx);
        assert!(
            !w.open_console(
                host("gfx2"),
                SshSettings::default(),
                key("gfx2"),
                Ok(v.clone())
            ),
            "already open"
        );
        let helper = helper_pid(&pidfile);
        w.close_console(&key("gfx2"));
        assert_eq!(
            console_states(&rx).last(),
            Some(&ConsoleState::Closed { error: None })
        );
        assert!(gone(&helper), "helper {helper} of the viewer still runs");
        assert!(start.elapsed() < Duration::from_secs(10));

        // Only the helper ignores SIGTERM: the viewer stops at once, the
        // helper is killed after 2 s. Here the host is forgotten.
        std::fs::remove_file(&pidfile).unwrap();
        let v = script(
            d.path(),
            &format!(
                "(trap '' TERM; exec sleep 30) & echo $! > '{}'; wait",
                pidfile.display()
            ),
        );
        w.open_console(host("gfx2"), SshSettings::default(), key("gfx2"), Ok(v));
        wait_open(&rx);
        let helper = helper_pid(&pidfile);
        w.forget("gfx2");
        assert!(matches!(
            console_states(&rx).last(),
            Some(ConsoleState::Closed { .. })
        ));
        assert!(gone(&helper), "helper {helper} still runs after forget");
    }

    // C1 of crosscheck 2026-11-17-01: a console cannot be opened again while
    // its viewer is still stopping; once Closed it opens normally and is not
    // closed by the end of the old task.
    #[cfg(unix)]
    #[test]
    fn reopen_waits_until_the_old_viewer_stopped() {
        let (w, rx, _) = worker();
        let d = tempfile::tempdir().unwrap();
        // Stops 0.5 s after SIGTERM.
        let v = script(d.path(), "trap 'sleep 0.5; exit 0' TERM; sleep 30 & wait");
        let open = |w: &Worker| {
            w.open_console(
                host("gfx4"),
                SshSettings::default(),
                key("gfx4"),
                Ok(v.clone()),
            )
        };
        assert!(open(&w));
        wait_open(&rx);
        w.close_console(&key("gfx4"));
        assert!(!open(&w), "reopened while the old viewer is stopping");
        assert!(w.console_open(&key("gfx4")));
        assert!(matches!(
            console_states(&rx).last(),
            Some(ConsoleState::Closed { .. })
        ));
        assert!(!w.console_open(&key("gfx4")));
        assert!(open(&w));
        wait_open(&rx);
        // Still open a while later (the old task did not close it).
        std::thread::sleep(Duration::from_millis(300));
        assert!(w.console_open(&key("gfx4")));
        assert!(rx.try_iter().all(|e| !matches!(
            e,
            Event::Console {
                state: ConsoleState::Closed { .. },
                ..
            }
        )));
        w.close_console(&key("gfx4"));
        console_states(&rx);
    }

    // TC-103: failures become a Closed error: no viewer, the viewer fails
    // (its last stderr line is shown), no console in the live XML, VM gone.
    #[test]
    fn console_errors_are_reported() {
        let (w, rx, _) = worker();
        let err = |rx: &Receiver<Event>| match console_states(rx).pop() {
            Some(ConsoleState::Closed { error: Some(e) }) => e,
            s => panic!("{s:?}"),
        };
        w.open_console(
            host("gfx3"),
            SshSettings::default(),
            key("gfx3"),
            Err("viewer not found".into()),
        );
        assert_eq!(err(&rx), "viewer not found");

        #[cfg(unix)]
        {
            let d = tempfile::tempdir().unwrap();
            let v = script(d.path(), "echo 'cannot connect to display' >&2; exit 3");
            w.open_console(host("gfx3"), SshSettings::default(), key("gfx3"), Ok(v));
            let e = err(&rx);
            assert!(
                e.contains("exit status: 3") && e.contains("cannot connect to display"),
                "{e}"
            );
        }

        let exe = PathBuf::from("/nonexistent/viewer");
        // Host without graphics in vm1's XML.
        w.open_console(
            host("plain"),
            SshSettings::default(),
            key("plain"),
            Ok(exe.clone()),
        );
        assert!(err(&rx).contains("no longer in the domain"));
        // Unknown domain: dumpxml fails.
        let mut k = key("gfx3");
        k.domain = "gone".into();
        w.open_console(host("gfx3"), SshSettings::default(), k, Ok(exe));
        assert!(!err(&rx).is_empty());
    }

    // Crosscheck 2026-11-19-03 T1: which errors count as a lost connection
    // (then the worker connects or logs in again once, SPEC-022).
    #[test]
    fn lost_connections() {
        use vi_vsphere::{CertInfo, VsphereError as V};
        let t = || "vc:443".to_string();
        let fault = |kind: &str| {
            BackendError::Vsphere(V::Fault {
                target: t(),
                kind: kind.into(),
                message: "m".into(),
            })
        };
        // vSphere: expired session, connection gone.
        assert!(connection_lost(&fault("NotAuthenticated")));
        assert!(connection_lost(&BackendError::Vsphere(V::Connect {
            target: t(),
            reason: "reset".into()
        })));
        assert!(connection_lost(&BackendError::Vsphere(V::Http {
            target: t(),
            reason: "closed".into()
        })));
        // Not lost: a refused login, other faults, certificate problems,
        // timeouts, unreadable answers.
        for e in [
            fault("InvalidLogin"),
            fault("NoPermission"),
            BackendError::Vsphere(V::LoginFailed {
                target: t(),
                user: "u".into(),
            }),
            BackendError::Vsphere(V::UnknownCertificate {
                target: t(),
                cert: Box::new(CertInfo::from_der(b"x")),
            }),
            BackendError::Vsphere(V::Timeout {
                target: t(),
                secs: 1,
            }),
            BackendError::Vsphere(V::Parse {
                target: t(),
                reason: "r".into(),
            }),
        ] {
            assert!(!connection_lost(&e), "{e}");
        }
        // SSH: a protocol failure is lost; libvirt errors are not.
        assert!(connection_lost(&BackendError::Ssh(
            vi_ssh::SshError::Protocol("eof".into())
        )));
        assert!(!connection_lost(&BackendError::Config("x".into())));
    }
}
