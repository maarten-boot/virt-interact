//! The vSphere backend (SPEC-021): the host's connection is a logged-in
//! [`VsphereSession`]; the inventory is one fixed, paged, read-only query.

use vi_core::model::HostInventory;
use vi_ssh::CommandRunner;
use vi_ssh::prompt::BoxFuture;
use vi_vsphere::VsphereSession;

use crate::{BackendError, HypervisorBackend};

pub struct VsphereBackend;

impl HypervisorBackend for VsphereBackend {
    fn name(&self) -> &'static str {
        "vsphere"
    }

    fn inventory<'a>(
        &'a self,
        runner: &'a dyn CommandRunner,
    ) -> BoxFuture<'a, Result<HostInventory, BackendError>> {
        Box::pin(async move {
            let session = runner
                .as_any()
                .and_then(|a| a.downcast_ref::<VsphereSession>())
                .ok_or_else(|| {
                    BackendError::Config("a vSphere host needs a vSphere session".into())
                })?;
            let pages = session.inventory_pages().await?;
            vi_vsphere::inventory::parse(&pages, session.about()).map_err(|message| {
                BackendError::Parse {
                    subcommand: "RetrievePropertiesEx".into(),
                    message,
                }
            })
        })
    }
    // No consoles (SPEC-021): the default `graphics` reports none.
}
