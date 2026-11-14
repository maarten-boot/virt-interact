//! The libvirt backend: inventory from read-only virsh commands (SPEC-009..011).

use std::time::Duration;

use futures_util::stream::{self, StreamExt};
use tracing::{debug, warn};
use vi_core::config::HostConfig;
use vi_core::model::{DiskSource, Domain, HostInventory};
use vi_ssh::prompt::BoxFuture;
use vi_ssh::{CommandOutput, CommandRunner, SshError, SshErrorKind};

use crate::HypervisorBackend;
use crate::command::{Virsh, VirshCommand};
use crate::error::{BackendError, classify};
use crate::{domxml, parse};

/// Domains of one host queried at the same time, each on its own SSH
/// channel of the one connection (SPEC-012). OpenSSH allows 10 sessions per
/// connection by default (`MaxSessions`).
pub const DEFAULT_PARALLEL: usize = 4;

pub struct LibvirtBackend {
    virsh: Virsh,
    uri: String,
    command_timeout: Option<Duration>,
    parallel: usize,
}

/// Result of querying one domain.
struct DomainOutcome {
    /// `None` if the domain disappeared during the refresh.
    domain: Option<Domain>,
    warnings: Vec<String>,
}

impl LibvirtBackend {
    pub fn new(uri: &str, prefix: Option<&str>) -> Result<LibvirtBackend, BackendError> {
        Ok(LibvirtBackend {
            virsh: Virsh::new(uri, prefix)?,
            uri: uri.to_string(),
            command_timeout: None,
            parallel: DEFAULT_PARALLEL,
        })
    }

    pub fn from_host(host: &HostConfig) -> Result<LibvirtBackend, BackendError> {
        LibvirtBackend::new(&host.libvirt_uri, host.virsh_prefix.as_deref())
    }

    /// Limit each command to `d`, on top of any timeout of the runner itself
    /// (an `SshSession` has its own; the mock runner has none).
    pub fn with_command_timeout(mut self, d: Duration) -> LibvirtBackend {
        self.command_timeout = Some(d);
        self
    }

    /// How many domains are queried at the same time (at least 1).
    pub fn with_parallel(mut self, n: usize) -> LibvirtBackend {
        self.parallel = n.max(1);
        self
    }

    /// The command line for `cmd` (for logs and manual tests).
    pub fn command_line(&self, cmd: &VirshCommand) -> String {
        self.virsh.line(cmd)
    }

    /// Run one allowlisted command and return its raw output (any exit
    /// status). Used by the inventory and by the fixture capture
    /// (`inventory --save`). SSH errors, including timeouts, are `Err`.
    pub async fn run_command(
        &self,
        runner: &dyn CommandRunner,
        cmd: &VirshCommand,
    ) -> Result<CommandOutput, BackendError> {
        let line = self.virsh.line(cmd);
        debug!(command = %line, "virsh");
        let out = match self.command_timeout {
            Some(d) => tokio::time::timeout(d, runner.run(&line))
                .await
                .map_err(|_| SshError::Timeout {
                    what: format!("running \"{line}\""),
                    secs: d.as_secs(),
                })?,
            None => runner.run(&line).await,
        }?;
        Ok(out)
    }

    /// Run one command for the inventory. `Err` = fatal for the host (SSH
    /// connection problems). `Ok(Err(..))` = this command failed (non-zero
    /// exit status, or a timeout); the caller decides how bad that is.
    async fn run(
        &self,
        runner: &dyn CommandRunner,
        cmd: &VirshCommand,
    ) -> Result<Result<String, BackendError>, BackendError> {
        match self.run_command(runner, cmd).await {
            Ok(out) if out.success() => Ok(Ok(out.stdout_text())),
            Ok(out) => Ok(Err(classify(cmd.subcommand(), &self.uri, &out))),
            Err(e) if is_timeout(&e) => Ok(Err(e)),
            Err(e) => Err(e),
        }
    }

    /// Full inventory of the host (TC-047).
    pub async fn inventory(
        &self,
        runner: &dyn CommandRunner,
    ) -> Result<HostInventory, BackendError> {
        let mut inv = HostInventory::default();

        // Host-level commands: any failure, including a timeout, is a host
        // error (SPEC-013).
        let version = self.run(runner, &VirshCommand::Version).await??;
        (inv.library, inv.hypervisor) = parse::version(&version);
        let list = self.run(runner, &VirshCommand::ListAll).await??;
        let entries = parse::list(&list).map_err(|message| BackendError::Parse {
            subcommand: "list".into(),
            message,
        })?;

        // Up to `parallel` domains at once; results keep the list order.
        let outcomes: Vec<Result<DomainOutcome, BackendError>> = stream::iter(entries)
            .map(|entry| self.domain(runner, entry))
            .buffered(self.parallel)
            .collect()
            .await;
        for o in outcomes {
            let o = o?;
            inv.warnings.extend(o.warnings);
            inv.domains.extend(o.domain);
        }
        for d in &inv.domains {
            if let Some(e) = &d.error {
                inv.warnings.push(format!("domain \"{}\": {e}", d.name));
            }
        }
        Ok(inv)
    }

    /// One domain. Per-domain failures (a failing command, a timeout, bad
    /// XML) go into `Domain::error`; only SSH connection errors and access
    /// problems abort the refresh.
    async fn domain(
        &self,
        runner: &dyn CommandRunner,
        entry: parse::ListEntry,
    ) -> Result<DomainOutcome, BackendError> {
        let name = entry.name.clone();
        let mut d = Domain::new(&entry.name, entry.state.clone());
        d.id = entry.id;
        let mut warnings = Vec::new();
        let gone = || DomainOutcome {
            domain: None,
            warnings: vec![format!("domain \"{name}\" disappeared during the refresh")],
        };

        match self
            .run(runner, &VirshCommand::DomInfo(name.clone()))
            .await?
        {
            Ok(text) => {
                let info = parse::dominfo(&text);
                d.uuid = info.uuid;
                d.id = info.id.or(d.id);
                if let Some(s) = info.state {
                    d.state = s;
                }
                d.persistent = info.persistent;
                d.autostart = info.autostart;
            }
            Err(e) if is_gone(&e) => return Ok(gone()),
            Err(e) => return fatal_or(e, d),
        }

        let xml = match self
            .run(runner, &VirshCommand::DumpXml(name.clone()))
            .await?
        {
            Ok(xml) => xml,
            Err(e) if is_gone(&e) => return Ok(gone()),
            Err(e) => return fatal_or(e, d),
        };
        let mut details = match domxml::parse(&xml) {
            Ok(det) => det,
            Err(e) => {
                warn!(domain = %d.name, error = %e, "XML parse error");
                d.error = Some(format!("XML parse error: {e}"));
                return Ok(DomainOutcome {
                    domain: Some(d),
                    warnings,
                });
            }
        };

        for disk in &mut details.disks {
            let Some(target) = disk.target.clone() else {
                continue;
            };
            if disk.source == DiskSource::Empty {
                continue;
            }
            let cmd = VirshCommand::DomBlkInfo {
                domain: name.clone(),
                target: target.clone(),
            };
            match self.run(runner, &cmd).await? {
                Ok(text) => disk.size = Some(parse::domblkinfo(&text)),
                // Size unknown is not an error (TC-046); a hang (e.g. an
                // unreachable network disk) is worth a warning (TC-064).
                Err(e) if is_timeout(&e) => warnings.push(format!(
                    "domain \"{name}\", disk {target}: size unknown ({e})"
                )),
                Err(e) => debug!(domain = %name, error = %e, "domblkinfo failed"),
            }
        }
        if d.uuid.is_none() {
            d.uuid = details.general.uuid.clone();
        }
        d.details = Some(details);
        Ok(DomainOutcome {
            domain: Some(d),
            warnings,
        })
    }
}

fn is_timeout(e: &BackendError) -> bool {
    matches!(e, BackendError::Ssh(s) if s.kind() == SshErrorKind::Timeout)
}

/// The domain was removed between `list` and a later command.
fn is_gone(e: &BackendError) -> bool {
    e.detail()
        .is_some_and(|s| s.contains("failed to get domain") || s.contains("Domain not found"))
}

/// Access problems abort the host refresh; a failing command or a timeout
/// is recorded on the domain, and the refresh continues.
fn fatal_or(e: BackendError, mut d: Domain) -> Result<DomainOutcome, BackendError> {
    if matches!(e, BackendError::CommandFailed { .. }) || is_timeout(&e) {
        d.error = Some(e.to_string());
        Ok(DomainOutcome {
            domain: Some(d),
            warnings: Vec::new(),
        })
    } else {
        Err(e)
    }
}

impl HypervisorBackend for LibvirtBackend {
    fn name(&self) -> &'static str {
        "libvirt"
    }

    fn inventory<'a>(
        &'a self,
        runner: &'a dyn CommandRunner,
    ) -> BoxFuture<'a, Result<HostInventory, BackendError>> {
        Box::pin(LibvirtBackend::inventory(self, runner))
    }
}
