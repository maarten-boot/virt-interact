//! Inventory model: hosts, domains and their components (SPEC-009, SPEC-010).
//!
//! Backend-neutral (SPEC-017): the libvirt backend fills it, the GUI shows
//! it, the cache (SPEC-014) stores it. Values are kept as reported; missing
//! values are `None` ("unknown" in the GUI), never an error.

use serde::{Deserialize, Serialize};

/// Inventory of one host.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct HostInventory {
    /// e.g. `"libvirt 12.0.0"`.
    pub library: Option<String>,
    /// e.g. `"QEMU 10.2.1"`.
    pub hypervisor: Option<String>,
    pub domains: Vec<Domain>,
    /// Non-fatal problems, e.g. a domain that disappeared during the refresh.
    pub warnings: Vec<String>,
}

/// One virtual machine (SPEC-009).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Domain {
    pub name: String,
    pub uuid: Option<String>,
    /// Runtime ID; only for running domains.
    pub id: Option<u32>,
    pub state: DomainState,
    pub persistent: Option<bool>,
    pub autostart: Option<bool>,
    /// Components; `None` if they could not be read (see `error`).
    pub details: Option<DomainDetails>,
    /// Why the details are missing, e.g. an XML parse error.
    pub error: Option<String>,
}

impl Domain {
    pub fn new(name: impl Into<String>, state: DomainState) -> Domain {
        Domain {
            name: name.into(),
            uuid: None,
            id: None,
            state,
            persistent: None,
            autostart: None,
            details: None,
            error: None,
        }
    }
}

/// Domain state as reported by the hypervisor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DomainState {
    Running,
    Paused,
    ShutOff,
    Crashed,
    PmSuspended,
    /// Anything else (`idle`, `in shutdown`, `no state`, ...), as reported.
    Other(String),
}

impl DomainState {
    /// Parse a virsh state string (`list`, `dominfo`, `domstate`).
    pub fn from_virsh(s: &str) -> DomainState {
        match s.trim() {
            "running" => DomainState::Running,
            "paused" => DomainState::Paused,
            "shut off" => DomainState::ShutOff,
            "crashed" => DomainState::Crashed,
            "pmsuspended" => DomainState::PmSuspended,
            other => DomainState::Other(other.to_string()),
        }
    }

    pub fn label(&self) -> &str {
        match self {
            DomainState::Running => "running",
            DomainState::Paused => "paused",
            DomainState::ShutOff => "shut off",
            DomainState::Crashed => "crashed",
            DomainState::PmSuspended => "pmsuspended",
            DomainState::Other(s) => s,
        }
    }

    pub fn is_active(&self) -> bool {
        !matches!(self, DomainState::ShutOff | DomainState::Crashed)
    }
}

/// Components of a domain (SPEC-010).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct DomainDetails {
    pub general: General,
    pub cpu: Cpu,
    pub memory: Memory,
    pub disks: Vec<Disk>,
    pub interfaces: Vec<Interface>,
    pub graphics: Vec<Graphics>,
    pub other_devices: Vec<OtherDevice>,
    /// The XML the details were read from (live for running domains).
    pub xml: String,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct General {
    /// Hypervisor type from `<domain type=..>`, e.g. `kvm`.
    pub domain_type: Option<String>,
    /// e.g. `hvm`.
    pub os_type: Option<String>,
    pub arch: Option<String>,
    pub machine: Option<String>,
    pub firmware: Firmware,
    /// libosinfo ID as in the XML, e.g. `http://ubuntu.com/ubuntu/24.04`.
    pub guest_os_id: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
}

impl General {
    /// Short guest OS name: the libosinfo ID without scheme and vendor
    /// host (`ubuntu/24.04`); an ID in another form is returned as is.
    pub fn guest_os(&self) -> Option<&str> {
        let id = self.guest_os_id.as_deref()?;
        let short = id
            .strip_prefix("http://")
            .or_else(|| id.strip_prefix("https://"))
            .and_then(|rest| rest.split_once('/').map(|(_, path)| path))
            .filter(|p| !p.is_empty());
        Some(short.unwrap_or(id))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Firmware {
    #[default]
    Bios,
    Uefi,
}

impl Firmware {
    pub fn label(self) -> &'static str {
        match self {
            Firmware::Bios => "BIOS",
            Firmware::Uefi => "UEFI",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Cpu {
    /// vCPUs currently online (`<vcpu current=..>`, else the maximum).
    pub current: Option<u32>,
    pub max: Option<u32>,
    /// e.g. `host-passthrough`, `host-model`, `custom`.
    pub mode: Option<String>,
    pub model: Option<String>,
    pub topology: Option<Topology>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Topology {
    pub sockets: Option<u32>,
    pub dies: Option<u32>,
    pub cores: Option<u32>,
    pub threads: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Memory {
    pub max_kib: Option<u64>,
    pub current_kib: Option<u64>,
    pub hugepages: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Disk {
    /// `disk`, `cdrom`, `floppy`, `lun`.
    pub device: String,
    pub target: Option<String>,
    pub bus: Option<String>,
    pub source: DiskSource,
    /// Driver format, e.g. `qcow2`, `raw`.
    pub format: Option<String>,
    pub readonly: bool,
    pub shareable: bool,
    /// From `domblkinfo`; `None` = unknown.
    pub size: Option<BlockSize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DiskSource {
    /// No `<source>`: an empty cdrom/floppy drive.
    Empty,
    File(String),
    Block(String),
    Network {
        protocol: Option<String>,
        name: Option<String>,
        /// `host` or `host:port` entries.
        hosts: Vec<String>,
    },
    Volume {
        pool: Option<String>,
        volume: Option<String>,
    },
    Dir(String),
    /// A disk type this version does not know, e.g. `nvme`.
    Other(String),
}

impl DiskSource {
    pub fn describe(&self) -> String {
        match self {
            DiskSource::Empty => "empty".into(),
            DiskSource::File(p) | DiskSource::Block(p) | DiskSource::Dir(p) => p.clone(),
            DiskSource::Network {
                protocol,
                name,
                hosts,
            } => {
                let mut s = format!(
                    "{}://{}",
                    protocol.as_deref().unwrap_or("network"),
                    hosts.join(",")
                );
                if let Some(n) = name {
                    s.push('/');
                    s.push_str(n);
                }
                s
            }
            DiskSource::Volume { pool, volume } => format!(
                "{}/{}",
                pool.as_deref().unwrap_or("?"),
                volume.as_deref().unwrap_or("?")
            ),
            DiskSource::Other(t) => format!("({t})"),
        }
    }
}

/// `virsh domblkinfo` values in bytes, as reported (allocation and physical
/// may exceed capacity).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct BlockSize {
    pub capacity: Option<u64>,
    pub allocation: Option<u64>,
    pub physical: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Interface {
    /// `network`, `bridge`, `direct`, `user`, ...
    pub kind: String,
    /// Network name, bridge name or host device.
    pub source: Option<String>,
    pub mac: Option<String>,
    pub model: Option<String>,
    /// Host-side device of a running domain, e.g. `vnet0`.
    pub target_dev: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Graphics {
    /// `vnc`, `spice`, ...
    pub kind: String,
    pub listen: Listen,
    /// Live port; `None` if not set or `-1` (autoport on a stopped domain).
    pub port: Option<u16>,
    pub tls_port: Option<u16>,
    pub autoport: bool,
}

impl Graphics {
    /// Can a remote viewer reach this console through a tunnel (SPEC-018)?
    pub fn reachable(&self) -> bool {
        !matches!(self.listen, Listen::None)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Listen {
    /// TCP; address if known.
    Address(Option<String>),
    /// A libvirt network; `address` is the resolved address (live XML only).
    Network {
        network: Option<String>,
        address: Option<String>,
    },
    /// Unix socket path (if known).
    Socket(Option<String>),
    /// `listen type="none"`: not reachable remotely.
    None,
}

impl Listen {
    pub fn describe(&self) -> String {
        match self {
            Listen::Address(a) => a.clone().unwrap_or_else(|| "address (default)".into()),
            Listen::Network { network, address } => format!(
                "network {}{}",
                network.as_deref().unwrap_or("?"),
                address
                    .as_deref()
                    .map(|a| format!(" ({a})"))
                    .unwrap_or_default()
            ),
            Listen::Socket(p) => format!("socket {}", p.as_deref().unwrap_or("(auto)")),
            Listen::None => "none (not reachable remotely)".into(),
        }
    }
}

/// Any other device: controller, video, sound, hostdev, filesystem, channel...
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OtherDevice {
    /// XML element name, e.g. `controller`.
    pub kind: String,
    /// Short description, e.g. `pci pcie-root-port`.
    pub summary: String,
}

/// Bytes in binary units: `512 B`, `1.5 KiB`, `64.0 GiB`.
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut v = bytes as f64 / 1024.0;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    format!("{v:.1} {}", UNITS[i])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_from_virsh() {
        assert_eq!(DomainState::from_virsh("running"), DomainState::Running);
        assert_eq!(DomainState::from_virsh("shut off"), DomainState::ShutOff);
        assert_eq!(
            DomainState::from_virsh("in shutdown"),
            DomainState::Other("in shutdown".into())
        );
        assert!(!DomainState::ShutOff.is_active());
        assert!(DomainState::Paused.is_active());
    }

    #[test]
    fn guest_os_short_names() {
        let g = |id: &str| General {
            guest_os_id: Some(id.into()),
            ..General::default()
        };
        assert_eq!(
            g("http://ubuntu.com/ubuntu/24.04").guest_os(),
            Some("ubuntu/24.04")
        );
        assert_eq!(g("http://microsoft.com/win/10").guest_os(), Some("win/10"));
        assert_eq!(g("odd-id").guest_os(), Some("odd-id"));
        assert_eq!(General::default().guest_os(), None);
    }

    #[test]
    fn human_byte_units() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1536), "1.5 KiB");
        assert_eq!(human_bytes(68_719_476_736), "64.0 GiB");
    }
}
