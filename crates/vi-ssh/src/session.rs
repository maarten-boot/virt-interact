//! SSH session: connect, verify the host key, authenticate with keys only,
//! and run commands (SPEC-006, SPEC-008, SPEC-012).
//!
//! Connecting is done in two steps when the host key is unknown: the first
//! handshake stops at the unknown key, the user is asked (outside any
//! network timeout), and on acceptance the key is recorded and the
//! connection is made again.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use russh::client::{self, Handle};
use russh::keys::agent::client::AgentClient;
use russh::keys::{HashAlg, PrivateKeyWithHashAlg, PublicKey, PublicKeyOrCertificate};
use russh::{ChannelMsg, Disconnect};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::time::timeout;
use vi_core::config::SshSettings;

use crate::error::SshError;
use crate::known_hosts::{self, HostKeyStatus};
use crate::prompt::{BoxFuture, HostKeyPrompt, Prompter};
use crate::runner::{CommandOutput, CommandRunner};
use crate::sshconfig::ResolvedHost;

/// Options for [`SshSession::connect`].
#[derive(Debug, Clone)]
pub struct ConnectOptions {
    /// Applies separately to the TCP connect, the handshake and each
    /// authentication attempt; time waiting for the user is not counted.
    pub connect_timeout: Duration,
    pub command_timeout: Duration,
    /// The user's known_hosts file (read and written).
    pub known_hosts: PathBuf,
    /// System-wide known_hosts files (read only), checked after the user's.
    pub global_known_hosts: Vec<PathBuf>,
    /// Try ssh-agent keys first (SPEC-006).
    pub use_agent: bool,
    /// Write newly accepted host keys in hashed form.
    pub hash_known_hosts: bool,
}

impl ConnectOptions {
    /// Defaults: 10 s / 30 s, the platform's system known_hosts, agent on,
    /// hashed entries.
    pub fn new(known_hosts: PathBuf) -> ConnectOptions {
        ConnectOptions::from_settings(&SshSettings::default(), known_hosts)
    }

    /// Options with the timeouts from the config file (SPEC-012).
    pub fn from_settings(s: &SshSettings, known_hosts: PathBuf) -> ConnectOptions {
        ConnectOptions {
            connect_timeout: Duration::from_secs(s.connect_timeout_secs),
            command_timeout: Duration::from_secs(s.command_timeout_secs),
            known_hosts,
            global_known_hosts: default_global_known_hosts(),
            use_agent: true,
            hash_known_hosts: true,
        }
    }

    /// `~/.ssh/known_hosts`, if there is a home directory.
    pub fn default_known_hosts() -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join(".ssh").join("known_hosts"))
    }
}

/// The system-wide known_hosts files OpenSSH reads on this platform.
pub fn default_global_known_hosts() -> Vec<PathBuf> {
    if cfg!(windows) {
        std::env::var_os("PROGRAMDATA")
            .map(|p| vec![PathBuf::from(p).join("ssh").join("ssh_known_hosts")])
            .unwrap_or_default()
    } else {
        vec![
            PathBuf::from("/etc/ssh/ssh_known_hosts"),
            PathBuf::from("/etc/ssh/ssh_known_hosts2"),
        ]
    }
}

/// One authentication step, in the order tried (SPEC-006). There is no
/// password or keyboard-interactive step by design (TC-035).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthStep {
    Agent,
    KeyFile(PathBuf),
}

/// The authentication plan for a host: agent first (if enabled), then the
/// key files.
pub fn auth_plan(host: &ResolvedHost, use_agent: bool) -> Vec<AuthStep> {
    let mut plan = Vec::new();
    if use_agent {
        plan.push(AuthStep::Agent);
    }
    plan.extend(host.identity_files.iter().cloned().map(AuthStep::KeyFile));
    plan
}

/// The `.pub` file next to a key file.
fn pub_path(key_file: &Path) -> PathBuf {
    PathBuf::from(format!("{}.pub", key_file.display()))
}

/// Order agent keys so that those matching a configured key file (by its
/// `.pub`) come first; otherwise keep the agent's order. This keeps a big
/// agent from using up the server's `MaxAuthTries` before the right key.
pub fn order_agent_keys(keys: Vec<PublicKey>, preferred: &[PublicKey]) -> Vec<PublicKey> {
    let (mut first, rest): (Vec<_>, Vec<_>) = keys
        .into_iter()
        .partition(|k| preferred.iter().any(|p| p.key_data() == k.key_data()));
    first.extend(rest);
    first
}

/// Combine the user's and the system-wide known_hosts results, as OpenSSH
/// does: revoked anywhere refuses; the first file that knows the host
/// (match or mismatch) decides, the user's file first. Returns the status
/// and the index of the file it came from (0 = user, 1.. = system).
pub fn combine_status(results: &[HostKeyStatus]) -> (HostKeyStatus, usize) {
    if let Some((i, s)) = results
        .iter()
        .enumerate()
        .find(|(_, s)| matches!(s, HostKeyStatus::Revoked { .. }))
    {
        return (s.clone(), i);
    }
    for (i, s) in results.iter().enumerate() {
        if matches!(s, HostKeyStatus::Known | HostKeyStatus::Mismatch { .. }) {
            return (s.clone(), i);
        }
    }
    let mut other = Vec::new();
    for s in results {
        if let HostKeyStatus::Unknown { other_types } = s {
            for t in other_types {
                if !other.contains(t) {
                    other.push(t.clone());
                }
            }
        }
    }
    (HostKeyStatus::Unknown { other_types: other }, 0)
}

struct Checker {
    known_hosts: PathBuf,
    global_known_hosts: Vec<PathBuf>,
    hostname: String,
    port: u16,
    accepted: Option<PublicKey>,
}

impl client::Handler for Checker {
    type Error = SshError;

    async fn check_server_key(
        &mut self,
        presented: &PublicKeyOrCertificate,
    ) -> Result<bool, SshError> {
        let key = match presented {
            PublicKeyOrCertificate::PublicKey { key, .. } => key,
            PublicKeyOrCertificate::Certificate(_) => return Err(SshError::HostCertificate),
        };
        let host = known_hosts::host_field(&self.hostname, self.port);
        if let Some(a) = &self.accepted {
            if a.key_data() == key.key_data() {
                return Ok(true);
            }
            // The user accepted one key, and now another one is presented.
            return Err(SshError::HostKeyChangedDuringConnect {
                host,
                fingerprint: known_hosts::fingerprint(key),
            });
        }
        let mut files = vec![self.known_hosts.clone()];
        files.extend(self.global_known_hosts.iter().cloned());
        let mut results = Vec::new();
        for (i, f) in files.iter().enumerate() {
            match known_hosts::check_file(f, &self.hostname, self.port, key) {
                Ok((status, warnings)) => {
                    for w in warnings {
                        tracing::warn!("{}: {w}", f.display());
                    }
                    results.push(status);
                }
                // The user's file must be readable; system files are optional.
                Err(source) if i == 0 => {
                    return Err(SshError::KnownHosts {
                        path: f.clone(),
                        source,
                    });
                }
                Err(e) => {
                    tracing::debug!("{}: {e}", f.display());
                    results.push(HostKeyStatus::Unknown {
                        other_types: Vec::new(),
                    });
                }
            }
        }
        let (status, from) = combine_status(&results);
        match status {
            HostKeyStatus::Known => Ok(true),
            HostKeyStatus::Unknown { other_types } => Err(SshError::UnknownHostKey {
                key: Box::new(key.clone()),
                other_types,
            }),
            HostKeyStatus::Mismatch { line } => Err(SshError::HostKeyMismatch {
                host,
                line,
                fingerprint: known_hosts::fingerprint(key),
                known_hosts: files[from].clone(),
            }),
            HostKeyStatus::Revoked { line } => Err(SshError::HostKeyRevoked { host, line }),
        }
    }
}

/// State of one authentication run.
struct AuthState {
    user: String,
    rsa_hash: Option<HashAlg>,
    /// Human-readable list of what was tried and why it failed.
    tried: Vec<String>,
    /// Public keys already offered (agent or file).
    tried_keys: Vec<PublicKey>,
    /// Public keys of the configured key files (from their `.pub`).
    preferred: Vec<PublicKey>,
    /// Set when the server closed the connection during authentication.
    closed: bool,
}

impl AuthState {
    fn already_tried(&self, k: &PublicKey) -> bool {
        self.tried_keys.iter().any(|t| t.key_data() == k.key_data())
    }
}

/// Outcome of one authentication step.
enum Attempt {
    Success,
    Failed,
}

/// An authenticated SSH connection.
pub struct SshSession {
    handle: Handle<Checker>,
    host: ResolvedHost,
    command_timeout: Duration,
}

fn secs(d: Duration) -> u64 {
    d.as_secs().max(1)
}

async fn with_timeout<T>(
    d: Duration,
    what: &str,
    f: impl std::future::Future<Output = T>,
) -> Result<T, SshError> {
    timeout(d, f).await.map_err(|_| SshError::Timeout {
        what: what.to_string(),
        secs: secs(d),
    })
}

impl SshSession {
    /// Connect, verify the host key (asking `prompter` if it is unknown) and
    /// authenticate.
    pub async fn connect(
        host: ResolvedHost,
        opts: &ConnectOptions,
        prompter: &dyn Prompter,
    ) -> Result<SshSession, SshError> {
        for w in &host.warnings {
            tracing::warn!(host = %host.name, "{w}");
        }
        let mut accepted: Option<PublicKey> = None;
        let handle = loop {
            match Self::handshake(&host, opts, accepted.clone()).await {
                Err(SshError::UnknownHostKey { key, other_types }) if accepted.is_none() => {
                    let prompt = HostKeyPrompt {
                        name: host.name.clone(),
                        hostname: host.hostname.clone(),
                        port: host.port,
                        key_type: known_hosts::key_type(&key),
                        fingerprint: known_hosts::fingerprint(&key),
                        other_types,
                    };
                    if !prompter.confirm_host_key(&prompt).await {
                        return Err(SshError::HostKeyRejected {
                            host: known_hosts::host_field(&host.hostname, host.port),
                            fingerprint: prompt.fingerprint,
                        });
                    }
                    match known_hosts::learn(
                        &opts.known_hosts,
                        &host.hostname,
                        host.port,
                        &key,
                        opts.hash_known_hosts,
                    ) {
                        Ok(()) => tracing::info!(
                            host = %host.name,
                            fingerprint = %prompt.fingerprint,
                            "host key added to {}",
                            opts.known_hosts.display()
                        ),
                        Err(e) => tracing::warn!(
                            host = %host.name,
                            "could not record host key in {}: {e}; accepted for this connection only",
                            opts.known_hosts.display()
                        ),
                    }
                    accepted = Some(*key);
                }
                other => break other?,
            }
        };
        let mut session = SshSession {
            handle,
            command_timeout: opts.command_timeout,
            host,
        };
        session.authenticate(opts, prompter).await?;
        Ok(session)
    }

    async fn handshake(
        host: &ResolvedHost,
        opts: &ConnectOptions,
        accepted: Option<PublicKey>,
    ) -> Result<Handle<Checker>, SshError> {
        let addr = format!("{}:{}", host.hostname, host.port);
        tracing::debug!(host = %host.name, %addr, "connecting");
        let stream = with_timeout(
            opts.connect_timeout,
            &format!("connecting to {addr}"),
            tokio::net::TcpStream::connect((host.hostname.as_str(), host.port)),
        )
        .await?
        .map_err(|source| SshError::Connect {
            addr: addr.clone(),
            source,
        })?;
        let _ = stream.set_nodelay(true);
        let config = Arc::new(client::Config {
            keepalive_interval: Some(Duration::from_secs(30)),
            ..Default::default()
        });
        let checker = Checker {
            known_hosts: opts.known_hosts.clone(),
            global_known_hosts: opts.global_known_hosts.clone(),
            hostname: host.hostname.clone(),
            port: host.port,
            accepted,
        };
        with_timeout(
            opts.connect_timeout,
            &format!("handshaking with {addr}"),
            client::connect_stream(config, stream, checker),
        )
        .await?
    }

    async fn authenticate(
        &mut self,
        opts: &ConnectOptions,
        prompter: &dyn Prompter,
    ) -> Result<(), SshError> {
        let preferred = if self.host.identity_explicit {
            self.host
                .identity_files
                .iter()
                .filter_map(|f| russh::keys::load_public_key(pub_path(f)).ok())
                .collect()
        } else {
            Vec::new()
        };
        let mut st = AuthState {
            user: self.host.user.clone(),
            rsa_hash: self
                .handle
                .best_supported_rsa_hash()
                .await
                .ok()
                .flatten()
                .flatten(),
            tried: Vec::new(),
            tried_keys: Vec::new(),
            preferred,
            closed: false,
        };

        for step in auth_plan(&self.host, opts.use_agent) {
            let r = match step {
                AuthStep::Agent => self.try_agent(&mut st, opts).await?,
                AuthStep::KeyFile(path) => {
                    self.try_key_file(&mut st, &path, opts, prompter).await?
                }
            };
            if let Attempt::Success = r {
                tracing::info!(host = %self.host.name, user = %st.user, "authenticated");
                return Ok(());
            }
            if st.closed {
                st.tried.push(
                    "server closed the connection (too many authentication attempts?)".into(),
                );
                break;
            }
        }
        Err(SshError::AuthFailed {
            user: st.user,
            host: self.host.hostname.clone(),
            tried: st.tried,
        })
    }

    /// Record a failed attempt; notice when the server hung up.
    fn record_failure(&self, st: &mut AuthState, label: String, err: Option<String>) {
        match err {
            Some(e) => st.tried.push(format!("{label}: {e}")),
            None => st.tried.push(label),
        }
        if self.handle.is_closed() {
            st.closed = true;
        }
    }

    async fn try_agent(
        &mut self,
        st: &mut AuthState,
        opts: &ConnectOptions,
    ) -> Result<Attempt, SshError> {
        #[cfg(unix)]
        {
            match AgentClient::connect_env().await {
                Ok(agent) => return self.agent_keys(st, agent, opts).await,
                Err(e) => tracing::debug!("no ssh-agent: {e}"),
            }
        }
        #[cfg(windows)]
        {
            match AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent").await {
                Ok(agent) => return self.agent_keys(st, agent, opts).await,
                Err(e) => tracing::debug!("no OpenSSH agent pipe: {e}"),
            }
            match AgentClient::connect_pageant().await {
                Ok(agent) => return self.agent_keys(st, agent, opts).await,
                Err(e) => tracing::debug!("no Pageant: {e}"),
            }
        }
        let _ = (st, opts);
        Ok(Attempt::Failed)
    }

    async fn agent_keys<S>(
        &mut self,
        st: &mut AuthState,
        mut agent: AgentClient<S>,
        opts: &ConnectOptions,
    ) -> Result<Attempt, SshError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let ids = match agent.request_identities().await {
            Ok(ids) => ids,
            Err(e) => {
                tracing::warn!("ssh-agent: cannot list keys: {e}");
                return Ok(Attempt::Failed);
            }
        };
        let labelled: Vec<(PublicKey, String)> = ids
            .iter()
            .map(|id| (id.public_key().into_owned(), id.comment().to_string()))
            .collect();
        let keys = order_agent_keys(
            labelled.iter().map(|(k, _)| k.clone()).collect(),
            &st.preferred,
        );
        for key in keys {
            let comment = labelled
                .iter()
                .find(|(k, _)| k.key_data() == key.key_data())
                .map(|(_, c)| c.as_str())
                .unwrap_or_default();
            let label = format!("agent key {comment} ({})", known_hosts::fingerprint(&key));
            let hash = if key.algorithm().is_rsa() {
                st.rsa_hash
            } else {
                None
            };
            let r = with_timeout(
                opts.connect_timeout,
                "authenticating",
                self.handle
                    .authenticate_publickey_with(&st.user, key.clone(), hash, &mut agent),
            )
            .await?;
            st.tried_keys.push(key);
            match r {
                Ok(res) if res.success() => return Ok(Attempt::Success),
                Ok(_) => self.record_failure(st, label, None),
                Err(e) => self.record_failure(st, label, Some(e.to_string())),
            }
            if st.closed {
                break;
            }
        }
        Ok(Attempt::Failed)
    }

    async fn try_key_file(
        &mut self,
        st: &mut AuthState,
        path: &Path,
        opts: &ConnectOptions,
        prompter: &dyn Prompter,
    ) -> Result<Attempt, SshError> {
        if !path.exists() {
            if self.host.identity_explicit {
                st.tried.push(format!("{} (not found)", path.display()));
            }
            return Ok(Attempt::Failed);
        }
        // Skip a key the agent already offered, without asking for its passphrase.
        if let Ok(pk) = russh::keys::load_public_key(pub_path(path))
            && st.already_tried(&pk)
        {
            return Ok(Attempt::Failed);
        }
        let key = match russh::keys::load_secret_key(path, None) {
            Ok(k) => k,
            Err(russh::keys::Error::KeyIsEncrypted) => match prompter.passphrase(path).await {
                Some(pass) => match russh::keys::load_secret_key(path, Some(pass.expose())) {
                    Ok(k) => k,
                    Err(_) => {
                        st.tried
                            .push(format!("{} (wrong passphrase)", path.display()));
                        return Ok(Attempt::Failed);
                    }
                },
                None => {
                    st.tried.push(format!(
                        "{} (encrypted, no passphrase given)",
                        path.display()
                    ));
                    return Ok(Attempt::Failed);
                }
            },
            Err(e) => {
                st.tried
                    .push(format!("{} (cannot read: {e})", path.display()));
                return Ok(Attempt::Failed);
            }
        };
        let public = key.public_key().clone();
        if st.already_tried(&public) {
            return Ok(Attempt::Failed);
        }
        let hash = if key.algorithm().is_rsa() {
            st.rsa_hash
        } else {
            None
        };
        let r = with_timeout(
            opts.connect_timeout,
            "authenticating",
            self.handle
                .authenticate_publickey(&st.user, PrivateKeyWithHashAlg::new(Arc::new(key), hash)),
        )
        .await?;
        st.tried_keys.push(public);
        let label = path.display().to_string();
        match r {
            Ok(res) if res.success() => Ok(Attempt::Success),
            Ok(_) => {
                self.record_failure(st, label, None);
                Ok(Attempt::Failed)
            }
            // C1: a protocol error (e.g. the server hung up) is recorded,
            // not turned into a generic error.
            Err(e) => {
                self.record_failure(st, label, Some(e.to_string()));
                Ok(Attempt::Failed)
            }
        }
    }

    /// The host this session is connected to.
    pub fn host(&self) -> &ResolvedHost {
        &self.host
    }

    /// Run a command line and collect its output (SPEC-012 command timeout).
    pub async fn exec(&self, command: &str) -> Result<CommandOutput, SshError> {
        let what = format!("running \"{command}\" on {}", self.host.name);
        with_timeout(self.command_timeout, &what, self.exec_inner(command)).await?
    }

    async fn exec_inner(&self, command: &str) -> Result<CommandOutput, SshError> {
        tracing::debug!(host = %self.host.name, %command, "exec");
        let mut ch = self.handle.channel_open_session().await?;
        ch.exec(true, command).await?;
        let mut out = CommandOutput {
            exit_status: None,
            stdout: Vec::new(),
            stderr: Vec::new(),
        };
        while let Some(msg) = ch.wait().await {
            match msg {
                ChannelMsg::Data { data } => out.stdout.extend_from_slice(&data),
                ChannelMsg::ExtendedData { data, ext: 1 } => out.stderr.extend_from_slice(&data),
                ChannelMsg::ExitStatus { exit_status } => out.exit_status = Some(exit_status),
                ChannelMsg::Close => break,
                _ => {}
            }
        }
        tracing::debug!(host = %self.host.name, %command, status = ?out.exit_status, "exec done");
        Ok(out)
    }

    /// Close the connection.
    pub async fn close(self) {
        let _ = self
            .handle
            .disconnect(Disconnect::ByApplication, "", "en")
            .await;
    }
}

impl CommandRunner for SshSession {
    fn run<'a>(&'a self, command: &'a str) -> BoxFuture<'a, Result<CommandOutput, SshError>> {
        Box::pin(self.exec(command))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use russh::keys::parse_public_key_base64;

    // server_ed25519 and other_ed25519 from tests/fixtures/ssh
    const ED_A: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIPppDA0rOSaOEHE++x1T7TWMnNGwBKOxH5dp0HYliXdT";
    const ED_B: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIB66dIvD+ro2JT6TNDRKOARojl+A52Vp4TDh8NwJp9p3";

    fn host(files: Vec<&str>) -> ResolvedHost {
        ResolvedHost {
            name: "n".into(),
            alias: "a".into(),
            hostname: "h".into(),
            port: 22,
            user: "u".into(),
            identity_files: files.into_iter().map(PathBuf::from).collect(),
            identity_explicit: true,
            warnings: vec![],
        }
    }

    // TC-035 (+ SPEC-006 order)
    #[test]
    fn auth_plan_is_agent_then_key_files_only() {
        let p = auth_plan(&host(vec!["/k1", "/k2"]), true);
        assert_eq!(
            p,
            vec![
                AuthStep::Agent,
                AuthStep::KeyFile("/k1".into()),
                AuthStep::KeyFile("/k2".into())
            ]
        );
        assert_eq!(auth_plan(&host(vec![]), false), vec![]);
        // AuthStep has exactly two variants: no password, no keyboard-interactive.
        for s in p {
            match s {
                AuthStep::Agent | AuthStep::KeyFile(_) => {}
            }
        }
    }

    // TC-035a
    #[test]
    fn agent_keys_matching_config_come_first() {
        let a = parse_public_key_base64(ED_A).unwrap();
        let b = parse_public_key_base64(ED_B).unwrap();
        let ordered = order_agent_keys(vec![a.clone(), b.clone()], std::slice::from_ref(&b));
        assert_eq!(ordered, vec![b.clone(), a.clone()]);
        let ordered = order_agent_keys(vec![a.clone(), b.clone()], &[]);
        assert_eq!(ordered, vec![a, b]);
    }

    // TC-033b
    #[test]
    fn combine_user_and_system_known_hosts() {
        let unknown = HostKeyStatus::Unknown {
            other_types: vec![],
        };
        let c = |v: Vec<HostKeyStatus>| combine_status(&v);
        assert_eq!(
            c(vec![HostKeyStatus::Known, unknown.clone()]),
            (HostKeyStatus::Known, 0)
        );
        assert_eq!(
            c(vec![unknown.clone(), HostKeyStatus::Known]),
            (HostKeyStatus::Known, 1)
        );
        // The user's file decides when it knows the host.
        assert_eq!(
            c(vec![
                HostKeyStatus::Known,
                HostKeyStatus::Mismatch { line: 3 }
            ]),
            (HostKeyStatus::Known, 0)
        );
        assert_eq!(
            c(vec![unknown.clone(), HostKeyStatus::Mismatch { line: 3 }]),
            (HostKeyStatus::Mismatch { line: 3 }, 1)
        );
        // Revoked anywhere refuses.
        assert_eq!(
            c(vec![
                HostKeyStatus::Known,
                HostKeyStatus::Revoked { line: 1 }
            ]),
            (HostKeyStatus::Revoked { line: 1 }, 1)
        );
        let rsa = HostKeyStatus::Unknown {
            other_types: vec!["ssh-rsa".into()],
        };
        assert_eq!(c(vec![rsa.clone(), unknown]), (rsa, 0));
    }

    // TC-063
    #[test]
    fn options_from_config_settings() {
        let s = SshSettings {
            connect_timeout_secs: 3,
            command_timeout_secs: 45,
        };
        let o = ConnectOptions::from_settings(&s, PathBuf::from("/kh"));
        assert_eq!(o.connect_timeout, Duration::from_secs(3));
        assert_eq!(o.command_timeout, Duration::from_secs(45));
        assert_eq!(o.known_hosts, PathBuf::from("/kh"));
        assert!(o.use_agent && o.hash_known_hosts);
        let d = ConnectOptions::new(PathBuf::from("/kh"));
        assert_eq!(d.connect_timeout, Duration::from_secs(10));
        assert_eq!(d.command_timeout, Duration::from_secs(30));
        assert_eq!(d.global_known_hosts, default_global_known_hosts());
    }
}
