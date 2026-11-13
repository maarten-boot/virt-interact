//! Hypervisor backends for virt-interact (SPEC-017). v1 ships the libvirt
//! backend: allowlisted, read-only virsh commands over SSH (SPEC-009..011).

pub mod command;
pub mod domxml;
pub mod error;
pub mod libvirt;
pub mod parse;

use vi_core::config::{Backend, HostConfig};
use vi_core::model::HostInventory;
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
}

/// The backend configured for `host`.
pub fn for_host(host: &HostConfig) -> Result<Box<dyn HypervisorBackend>, BackendError> {
    match host.backend {
        Backend::Libvirt => Ok(Box::new(LibvirtBackend::from_host(host)?)),
    }
}
