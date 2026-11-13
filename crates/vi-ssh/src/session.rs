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

use crate::error::SshError;
use crate::known_hosts::{self, HostKeyStatus};
use crate::prompt::{BoxFuture, HostKeyPrompt, Prompter};
use crate::runner::{CommandOutput, CommandRunner};
use crate::sshconfig::ResolvedHost;

/// Options for [`SshSession::connect`].
#[derive(Debug, Clone)]
pub struct ConnectOptions {
    pub connect_timeout: Duration,
    pub command_timeout: Duration,
    /// Path of the known_hosts file.
    pub known_hosts: PathBuf,
    /// Try ssh-agent keys first (SPEC-006).
    pub use_agent: bool,
    /// Write newly accepted host keys in hashed form.
    pub hash_known_hosts: bool,
}

impl ConnectOptions {
    /// Defaults: 10 s / 30 s, `~/.ssh/known_hosts`, agent on, hashed.
    pub fn new(known_hosts: PathBuf) -> ConnectOptions {
        ConnectOptions {
            connect_timeout: Duration::from_secs(10),
            command_timeout: Duration::from_secs(30),
            known_hosts,
            use_agent: true,
            hash_known_hosts: true,
        }
    }

    /// `~/.ssh/known_hosts`, if there is a home directory.
    pub fn default_known_hosts() -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join(".ssh").join("known_hosts"))
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

struct Checker {
    known_hosts: PathBuf,
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
        if let Some(a) = &self.accepted
            && a.key_data() == key.key_data()
        {
            return Ok(true);
        }
        let (status, warnings) =
            known_hosts::check_file(&self.known_hosts, &self.hostname, self.port, key).map_err(
                |source| SshError::KnownHosts {
                    path: self.known_hosts.clone(),
                    source,
                },
            )?;
        for w in warnings {
            tracing::warn!("{}: {w}", self.known_hosts.display());
        }
        let host = known_hosts::host_field(&self.hostname, self.port);
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
                known_hosts: self.known_hosts.clone(),
            }),
            HostKeyStatus::Revoked { line } => Err(SshError::HostKeyRevoked { host, line }),
        }
    }
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
                        Ok(()) => {
                            tracing::info!(host = %host.name, fingerprint = %prompt.fingerprint, "host key added to {}", opts.known_hosts.display())
                        }
                        Err(e) => {
                            tracing::warn!(host = %host.name, "could not record host key in {}: {e}; accepted for this session only", opts.known_hosts.display())
                        }
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
        let user = self.host.user.clone();
        let mut tried: Vec<String> = Vec::new();
        let mut tried_keys: Vec<PublicKey> = Vec::new();
        let rsa_hash = self
            .handle
            .best_supported_rsa_hash()
            .await
            .ok()
            .flatten()
            .flatten();

        for step in auth_plan(&self.host, opts.use_agent) {
            let ok = match step {
                AuthStep::Agent => {
                    self.try_agent(&user, rsa_hash, opts, &mut tried, &mut tried_keys)
                        .await?
                }
                AuthStep::KeyFile(path) => {
                    self.try_key_file(
                        &user,
                        &path,
                        rsa_hash,
                        opts,
                        prompter,
                        &mut tried,
                        &mut tried_keys,
                    )
                    .await?
                }
            };
            if ok {
                tracing::info!(host = %self.host.name, %user, "authenticated");
                return Ok(());
            }
        }
        Err(SshError::AuthFailed {
            user,
            host: self.host.hostname.clone(),
            tried,
        })
    }

    async fn try_agent(
        &mut self,
        user: &str,
        rsa_hash: Option<HashAlg>,
        opts: &ConnectOptions,
        tried: &mut Vec<String>,
        tried_keys: &mut Vec<PublicKey>,
    ) -> Result<bool, SshError> {
        #[cfg(unix)]
        {
            match AgentClient::connect_env().await {
                Ok(agent) => {
                    return self
                        .agent_keys(agent, user, rsa_hash, opts, tried, tried_keys)
                        .await;
                }
                Err(e) => tracing::debug!("no ssh-agent: {e}"),
            }
        }
        #[cfg(windows)]
        {
            match AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent").await {
                Ok(agent) => {
                    return self
                        .agent_keys(agent, user, rsa_hash, opts, tried, tried_keys)
                        .await;
                }
                Err(e) => tracing::debug!("no OpenSSH agent pipe: {e}"),
            }
            match AgentClient::connect_pageant().await {
                Ok(agent) => {
                    return self
                        .agent_keys(agent, user, rsa_hash, opts, tried, tried_keys)
                        .await;
                }
                Err(e) => tracing::debug!("no Pageant: {e}"),
            }
        }
        let _ = (user, rsa_hash, opts, tried, tried_keys);
        Ok(false)
    }

    async fn agent_keys<S>(
        &mut self,
        mut agent: AgentClient<S>,
        user: &str,
        rsa_hash: Option<HashAlg>,
        opts: &ConnectOptions,
        tried: &mut Vec<String>,
        tried_keys: &mut Vec<PublicKey>,
    ) -> Result<bool, SshError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let ids = match agent.request_identities().await {
            Ok(ids) => ids,
            Err(e) => {
                tracing::warn!("ssh-agent: cannot list keys: {e}");
                return Ok(false);
            }
        };
        for id in ids {
            let key = id.public_key().into_owned();
            let label = format!(
                "agent key {} ({})",
                id.comment(),
                known_hosts::fingerprint(&key)
            );
            let hash = if key.algorithm().is_rsa() {
                rsa_hash
            } else {
                None
            };
            let r = with_timeout(
                opts.connect_timeout,
                "authenticating",
                self.handle
                    .authenticate_publickey_with(user, key.clone(), hash, &mut agent),
            )
            .await?;
            tried_keys.push(key);
            match r {
                Ok(res) if res.success() => return Ok(true),
                Ok(_) => tried.push(label),
                Err(e) => tried.push(format!("{label}: {e}")),
            }
        }
        Ok(false)
    }

    #[allow(clippy::too_many_arguments)]
    async fn try_key_file(
        &mut self,
        user: &str,
        path: &Path,
        rsa_hash: Option<HashAlg>,
        opts: &ConnectOptions,
        prompter: &dyn Prompter,
        tried: &mut Vec<String>,
        tried_keys: &mut Vec<PublicKey>,
    ) -> Result<bool, SshError> {
        if !path.exists() {
            if self.host.identity_explicit {
                tried.push(format!("{} (not found)", path.display()));
            }
            return Ok(false);
        }
        // Skip a key the agent already offered, without asking for its passphrase.
        let pub_path = PathBuf::from(format!("{}.pub", path.display()));
        if let Ok(pk) = russh::keys::load_public_key(&pub_path)
            && tried_keys.iter().any(|k| k.key_data() == pk.key_data())
        {
            return Ok(false);
        }
        let key = match russh::keys::load_secret_key(path, None) {
            Ok(k) => k,
            Err(russh::keys::Error::KeyIsEncrypted) => match prompter.passphrase(path).await {
                Some(pass) => match russh::keys::load_secret_key(path, Some(pass.expose())) {
                    Ok(k) => k,
                    Err(_) => {
                        tried.push(format!("{} (wrong passphrase)", path.display()));
                        return Ok(false);
                    }
                },
                None => {
                    tried.push(format!(
                        "{} (encrypted, no passphrase given)",
                        path.display()
                    ));
                    return Ok(false);
                }
            },
            Err(e) => {
                tried.push(format!("{} (cannot read: {e})", path.display()));
                return Ok(false);
            }
        };
        let public = key.public_key().clone();
        if tried_keys.iter().any(|k| k.key_data() == public.key_data()) {
            return Ok(false);
        }
        let hash = if key.algorithm().is_rsa() {
            rsa_hash
        } else {
            None
        };
        let r = with_timeout(
            opts.connect_timeout,
            "authenticating",
            self.handle
                .authenticate_publickey(user, PrivateKeyWithHashAlg::new(Arc::new(key), hash)),
        )
        .await??;
        tried_keys.push(public);
        if r.success() {
            Ok(true)
        } else {
            tried.push(path.display().to_string());
            Ok(false)
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
}
