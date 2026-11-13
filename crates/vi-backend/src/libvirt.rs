//! The libvirt backend: inventory from read-only virsh commands (SPEC-009..011).

use tracing::{debug, warn};
use vi_core::config::HostConfig;
use vi_core::model::{DiskSource, Domain, HostInventory};
use vi_ssh::prompt::BoxFuture;
use vi_ssh::{CommandOutput, CommandRunner};

use crate::HypervisorBackend;
use crate::command::{Virsh, VirshCommand};
use crate::error::{BackendError, classify};
use crate::{domxml, parse};

pub struct LibvirtBackend {
    virsh: Virsh,
    uri: String,
}

impl LibvirtBackend {
    pub fn new(uri: &str, prefix: Option<&str>) -> Result<LibvirtBackend, BackendError> {
        Ok(LibvirtBackend {
            virsh: Virsh::new(uri, prefix)?,
            uri: uri.to_string(),
        })
    }

    pub fn from_host(host: &HostConfig) -> Result<LibvirtBackend, BackendError> {
        LibvirtBackend::new(&host.libvirt_uri, host.virsh_prefix.as_deref())
    }

    /// The command line for `cmd` (for logs and manual tests).
    pub fn command_line(&self, cmd: &VirshCommand) -> String {
        self.virsh.line(cmd)
    }

    /// Run one allowlisted command. SSH errors are returned as `Err`; a
    /// non-zero exit status is returned as `Ok(Err(..))` so the caller can
    /// decide whether it is fatal for the host.
    async fn run(
        &self,
        runner: &dyn CommandRunner,
        cmd: &VirshCommand,
    ) -> Result<Result<String, BackendError>, BackendError> {
        let line = self.virsh.line(cmd);
        debug!(command = %line, "virsh");
        let out: CommandOutput = runner.run(&line).await?;
        if out.success() {
            Ok(Ok(out.stdout_text()))
        } else {
            Ok(Err(classify(cmd.subcommand(), &self.uri, &out)))
        }
    }

    /// Full inventory of the host (TC-047).
    pub async fn inventory(
        &self,
        runner: &dyn CommandRunner,
    ) -> Result<HostInventory, BackendError> {
        let mut inv = HostInventory::default();

        // Host-level commands: any failure is a host error (SPEC-013).
        let version = self.run(runner, &VirshCommand::Version).await??;
        (inv.library, inv.hypervisor) = parse::version(&version);
        let list = self.run(runner, &VirshCommand::ListAll).await??;
        let entries = parse::list(&list).map_err(|message| BackendError::Parse {
            subcommand: "list".into(),
            message,
        })?;

        for entry in entries {
            let name = entry.name.clone();
            match self.domain(runner, entry).await? {
                Some(d) => inv.domains.push(d),
                None => inv
                    .warnings
                    .push(format!("domain \"{name}\" disappeared during the refresh")),
            }
        }
        for d in &inv.domains {
            if let Some(e) = &d.error {
                inv.warnings.push(format!("domain \"{}\": {e}", d.name));
            }
        }
        Ok(inv)
    }

    /// One domain. `Ok(None)` if it no longer exists. Per-domain virsh
    /// failures go into `Domain::error`; only SSH errors and access
    /// problems abort the refresh.
    async fn domain(
        &self,
        runner: &dyn CommandRunner,
        entry: parse::ListEntry,
    ) -> Result<Option<Domain>, BackendError> {
        let mut d = Domain::new(&entry.name, entry.state.clone());
        d.id = entry.id;

        match self
            .run(runner, &VirshCommand::DomInfo(entry.name.clone()))
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
            Err(e) if gone(&e) => return Ok(None),
            Err(e) => return fatal_or(e, d),
        }

        let xml = match self
            .run(runner, &VirshCommand::DumpXml(entry.name.clone()))
            .await?
        {
            Ok(xml) => xml,
            Err(e) if gone(&e) => return Ok(None),
            Err(e) => return fatal_or(e, d),
        };
        let mut details = match domxml::parse(&xml) {
            Ok(det) => det,
            Err(e) => {
                warn!(domain = %d.name, error = %e, "XML parse error");
                d.error = Some(format!("XML parse error: {e}"));
                return Ok(Some(d));
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
                domain: entry.name.clone(),
                target,
            };
            match self.run(runner, &cmd).await? {
                Ok(text) => disk.size = Some(parse::domblkinfo(&text)),
                // Size unknown is not an error (TC-046).
                Err(e) => debug!(domain = %d.name, error = %e, "domblkinfo failed"),
            }
        }
        d.details = Some(details);
        Ok(Some(d))
    }
}

/// The domain was removed between `list` and a later command.
fn gone(e: &BackendError) -> bool {
    e.detail()
        .is_some_and(|s| s.contains("failed to get domain") || s.contains("Domain not found"))
}

/// Access problems abort the host refresh; anything else is recorded on the
/// domain, and the refresh continues.
fn fatal_or(e: BackendError, mut d: Domain) -> Result<Option<Domain>, BackendError> {
    match e {
        BackendError::CommandFailed { .. } => {
            d.error = Some(e.to_string());
            Ok(Some(d))
        }
        fatal => Err(fatal),
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
