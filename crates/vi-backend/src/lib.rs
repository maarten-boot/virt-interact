//! Hypervisor backends for virt-interact (SPEC-017). v1 ships the libvirt
//! backend: allowlisted, read-only virsh commands over SSH (SPEC-009..011).

pub mod command;
pub mod domxml;
pub mod error;
pub mod libvirt;
pub mod local;
pub mod parse;

use std::time::Duration;

use vi_core::config::{Backend, HostConfig};
use vi_core::model::{Graphics, HostInventory};
use vi_ssh::CommandRunner;
use vi_ssh::prompt::BoxFuture;

pub use command::{Virsh, VirshCommand};
pub use error::{BackendError, BackendErrorKind};
pub use libvirt::LibvirtBackend;

/// A source of inventory (SPEC-017). Remote commands go through a
/// [`CommandRunner`], so backends can be tested without a host.
pub trait HypervisorBackend: Send + Sync {
    fn name(&self) -> &'static str;

    fn inventory<'a>(
        &'a self,
        runner: &'a dyn CommandRunner,
    ) -> BoxFuture<'a, Result<HostInventory, BackendError>>;

    /// The live graphics devices of one domain, read just before a console
    /// is opened (SPEC-018), so the port is current. A backend without
    /// consoles reports none.
    fn graphics<'a>(
        &'a self,
        _runner: &'a dyn CommandRunner,
        _domain: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Graphics>, BackendError>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

/// The backend configured for `host`.
pub fn for_host(host: &HostConfig) -> Result<Box<dyn HypervisorBackend>, BackendError> {
    for_host_with(host, None)
}

/// Like [`for_host`], with an extra per-command time limit enforced by the
/// backend (on top of any limit of the runner).
pub fn for_host_with(
    host: &HostConfig,
    command_timeout: Option<Duration>,
) -> Result<Box<dyn HypervisorBackend>, BackendError> {
    match host.backend {
        Backend::Libvirt => {
            let mut b = LibvirtBackend::from_host(host)?;
            if let Some(t) = command_timeout {
                b = b.with_command_timeout(t);
            }
            Ok(Box::new(b))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // TC-054
    #[test]
    fn backend_from_host_config() {
        let mut h = HostConfig::new("a", "h");
        assert_eq!(for_host(&h).unwrap().name(), "libvirt");
        h.virsh_prefix = Some("sudo; x".into());
        assert_eq!(
            for_host(&h).err().map(|e| e.kind()),
            Some(BackendErrorKind::Config)
        );
    }

    /// A second backend needs nothing but the trait (TC-090).
    struct Dummy;

    impl HypervisorBackend for Dummy {
        fn name(&self) -> &'static str {
            "dummy"
        }
        fn inventory<'a>(
            &'a self,
            _: &'a dyn CommandRunner,
        ) -> BoxFuture<'a, Result<HostInventory, BackendError>> {
            Box::pin(async {
                Ok(HostInventory {
                    domains: vec![vi_core::model::Domain::new(
                        "d",
                        vi_core::model::DomainState::Running,
                    )],
                    ..HostInventory::default()
                })
            })
        }
    }

    // TC-090
    #[tokio::test]
    async fn dummy_backend_through_the_trait() {
        let backends: Vec<Box<dyn HypervisorBackend>> = vec![
            Box::new(Dummy),
            for_host(&HostConfig::new("a", "h")).unwrap(),
        ];
        let names: Vec<_> = backends.iter().map(|b| b.name()).collect();
        assert_eq!(names, ["dummy", "libvirt"]);
        let inv = backends[0]
            .inventory(&vi_ssh::MockRunner::new())
            .await
            .unwrap();
        assert_eq!(inv.domains[0].name, "d");
    }
}
