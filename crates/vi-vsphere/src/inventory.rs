//! The inventory answer of a vSphere server, mapped onto the app's model
//! (SPEC-021), so the GUI needs no vSphere-specific screens.
//!
//! Objects or properties the user may not read are simply absent (a
//! `missingSet`, or not returned at all): fields stay unknown, and that is
//! not an error (a vCenter with a restricted view).

use std::collections::HashMap;

use roxmltree::Node;
use vi_core::model::{
    BlockSize, Cpu, Disk, DiskSource, Domain, DomainDetails, DomainState, Firmware, General,
    HostInventory, Interface, Memory, OtherDevice, Topology, VsphereInfo,
};

use crate::session::About;
use crate::soap::{self, MoRef, XSI};

/// One returned object: its properties as owned values.
#[derive(Debug, Clone, Default)]
struct Object {
    /// Simple values by property path (text).
    text: HashMap<String, String>,
    /// Object references by property path.
    refs: HashMap<String, MoRef>,
    /// `config.hardware.device` as parsed devices.
    devices: Vec<Device>,
    /// The `<objects>` element as returned, for the raw view.
    raw: String,
}

#[derive(Debug, Clone, Default)]
struct Device {
    /// `VirtualDisk`, `VirtualVmxnet3`, … (xsi:type).
    kind: String,
    key: Option<i64>,
    label: String,
    summary: String,
    controller_key: Option<i64>,
    unit: Option<i64>,
    backing_kind: String,
    file_name: Option<String>,
    device_name: Option<String>,
    portgroup_key: Option<String>,
    thin: Option<bool>,
    sharing: Option<String>,
    capacity_bytes: Option<u64>,
    mac: Option<String>,
    connected: Option<bool>,
}

/// Map the answer pages of the inventory query.
pub fn parse(pages: &[String], about: &About) -> Result<HostInventory, String> {
    let mut objects: HashMap<MoRef, Object> = HashMap::new();
    for page in pages {
        let doc = soap::parse_body(page)?;
        for o in doc
            .descendants()
            .filter(|n| n.is_element() && n.tag_name().name() == "objects")
        {
            let Some(obj) = child(o, "obj").map(moref) else {
                continue;
            };
            let mut entry = Object {
                raw: page[o.range()].to_string(),
                ..Object::default()
            };
            for p in o.children().filter(|n| n.tag_name().name() == "propSet") {
                let (Some(name), Some(val)) = (child_text(p, "name"), child(p, "val")) else {
                    continue;
                };
                if name == "config.hardware.device" {
                    entry.devices = val
                        .children()
                        .filter(|n| n.is_element())
                        .map(device)
                        .collect();
                } else if soap::ref_type(val).is_some() {
                    entry.refs.insert(name, moref(val));
                } else {
                    entry
                        .text
                        .insert(name, val.text().unwrap_or("").trim().to_string());
                }
            }
            objects.insert(obj, entry);
        }
    }
    Ok(map(&objects, about))
}

fn map(objects: &HashMap<MoRef, Object>, about: &About) -> HostInventory {
    let name_of = |r: &MoRef| objects.get(r).and_then(|o| o.text.get("name")).cloned();
    let portgroups: HashMap<&str, &str> = objects
        .iter()
        .filter(|(r, _)| r.kind == "DistributedVirtualPortgroup")
        .filter_map(|(_, o)| Some((o.text.get("key")?.as_str(), o.text.get("name")?.as_str())))
        .collect();
    // Walk up the parents to the first object of `kind`.
    let up = |start: Option<&MoRef>, kind: &str| -> Option<String> {
        let mut cur = start.cloned();
        for _ in 0..64 {
            let r = cur?;
            if r.kind == kind {
                return name_of(&r);
            }
            let o = objects.get(&r)?;
            cur = o
                .refs
                .get("parent")
                .or_else(|| o.refs.get("parentFolder"))
                .cloned();
        }
        None
    };
    let mut vms: Vec<(&MoRef, &Object)> = objects
        .iter()
        .filter(|(r, _)| r.kind == "VirtualMachine")
        .collect();
    vms.sort_by(|a, b| {
        let n = |o: &Object, r: &MoRef| o.text.get("name").cloned().unwrap_or(r.value.clone());
        n(a.1, a.0).cmp(&n(b.1, b.0))
    });
    let mut warnings = Vec::new();
    let domains = vms
        .into_iter()
        .map(|(r, o)| {
            let t = |k: &str| o.text.get(k).filter(|v| !v.is_empty()).cloned();
            let num = |k: &str| t(k).and_then(|v| v.parse::<u64>().ok());
            let name = t("name").unwrap_or_else(|| r.value.clone());
            let state = match t("runtime.powerState").as_deref() {
                Some("poweredOn") => DomainState::Running,
                Some("poweredOff") => DomainState::ShutOff,
                Some("suspended") => DomainState::Other("suspended".into()),
                Some(other) => DomainState::Other(other.to_string()),
                None => DomainState::Other("unknown".into()),
            };
            let host = o.refs.get("runtime.host");
            let cluster = host
                .and_then(|h| objects.get(h))
                .and_then(|h| h.refs.get("parent"))
                .filter(|p| p.kind == "ClusterComputeResource")
                .and_then(name_of);
            let vm_parent = o.refs.get("parentVApp").or_else(|| o.refs.get("parent"));
            let datacenter = up(vm_parent, "Datacenter").or_else(|| up(host, "Datacenter"));
            let tools = match (
                t("guest.toolsRunningStatus"),
                t("guest.toolsVersionStatus2"),
            ) {
                (None, None) => None,
                (a, b) => Some(
                    [a.map(|s| tools_text(&s)), b.map(|s| tools_text(&s))]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>()
                        .join(", "),
                ),
            };
            let template = t("config.template").as_deref() == Some("true");
            let cpus = num("config.hardware.numCPU").map(|v| v as u32);
            let per_socket = num("config.hardware.numCoresPerSocket").map(|v| v as u32);
            let has_config = o.text.keys().any(|k| k.starts_with("config."));
            if !has_config {
                warnings.push(format!(
                    "{name}: configuration not readable (no permission, or the VM is inaccessible)"
                ));
            }
            let controllers: HashMap<i64, &Device> =
                o.devices.iter().filter_map(|d| Some((d.key?, d))).collect();
            let mut disks = Vec::new();
            let mut interfaces = Vec::new();
            let mut other = Vec::new();
            for d in &o.devices {
                if d.kind == "VirtualDisk" || d.kind == "VirtualCdrom" || d.kind == "VirtualFloppy"
                {
                    disks.push(disk(d, &controllers));
                } else if d.mac.is_some() || d.kind.contains("Ethernet") || is_nic(&d.kind) {
                    interfaces.push(interface(d, &portgroups));
                } else {
                    other.push(OtherDevice {
                        kind: other_kind(&d.kind),
                        summary: if d.summary.is_empty() || d.summary == d.label {
                            d.label.clone()
                        } else {
                            format!("{}: {}", d.label, d.summary)
                        },
                    });
                }
            }
            let details = has_config.then(|| DomainDetails {
                general: General {
                    uuid: t("config.uuid"),
                    domain_type: Some("vmware".into()),
                    os_type: None,
                    arch: None,
                    machine: None,
                    firmware: if t("config.firmware").as_deref() == Some("efi") {
                        Firmware::Uefi
                    } else {
                        Firmware::Bios
                    },
                    guest_os_id: t("config.guestId"),
                    title: None,
                    description: t("config.annotation"),
                    vsphere: Some(VsphereInfo {
                        datacenter,
                        cluster,
                        esxi_host: host.and_then(name_of),
                        hardware_version: t("config.version"),
                        tools,
                        guest_full_name: t("config.guestFullName"),
                        template,
                    }),
                },
                cpu: Cpu {
                    current: cpus,
                    max: cpus,
                    mode: None,
                    model: None,
                    topology: match (cpus, per_socket) {
                        (Some(c), Some(p)) if p > 0 => Some(Topology {
                            sockets: Some(c.div_ceil(p)),
                            dies: None,
                            cores: Some(p),
                            threads: None,
                        }),
                        _ => None,
                    },
                },
                memory: Memory {
                    max_kib: num("config.hardware.memoryMB").map(|m| m * 1024),
                    current_kib: num("config.hardware.memoryMB").map(|m| m * 1024),
                    hugepages: false,
                },
                disks,
                interfaces,
                graphics: Vec::new(),
                other_devices: other,
                xml: o.raw.clone(),
            });
            Domain {
                name,
                uuid: t("config.uuid"),
                id: None,
                state,
                // Not vSphere concepts (shown as "n/a").
                persistent: None,
                autostart: None,
                details,
                error: None,
            }
        })
        .collect();
    HostInventory {
        library: Some(about.full_name.clone()).filter(|s| !s.is_empty()),
        hypervisor: Some(
            if about.is_vcenter() {
                "vCenter"
            } else {
                "ESXi"
            }
            .to_string(),
        ),
        domains,
        warnings,
    }
}

fn tools_text(s: &str) -> String {
    match s {
        "guestToolsRunning" => "running".into(),
        "guestToolsNotRunning" => "not running".into(),
        "guestToolsExecutingScripts" => "starting".into(),
        "guestToolsCurrent" => "current".into(),
        "guestToolsNeedUpgrade" => "upgrade available".into(),
        "guestToolsNotInstalled" => "not installed".into(),
        "guestToolsUnmanaged" => "unmanaged (open-vm-tools)".into(),
        "guestToolsSupportedNew" | "guestToolsSupportedOld" => "supported".into(),
        "guestToolsTooOld" | "guestToolsBlacklisted" => "too old".into(),
        "guestToolsTooNew" => "too new".into(),
        other => other.trim_start_matches("guestTools").to_string(),
    }
}

fn is_nic(kind: &str) -> bool {
    matches!(
        kind,
        "VirtualE1000"
            | "VirtualE1000e"
            | "VirtualVmxnet"
            | "VirtualVmxnet2"
            | "VirtualVmxnet3"
            | "VirtualVmxnet3Vrdma"
            | "VirtualPCNet32"
            | "VirtualSriovEthernetCard"
    )
}

fn short_kind(kind: &str) -> String {
    kind.strip_prefix("Virtual")
        .unwrap_or(kind)
        .to_ascii_lowercase()
}

fn other_kind(kind: &str) -> String {
    let k = kind.strip_prefix("Virtual").unwrap_or(kind);
    if k.ends_with("Controller") {
        return "controller".into();
    }
    match k {
        "MachineVideoCard" => "video".into(),
        "Keyboard" | "PointingDevice" => "input".into(),
        "SerialPort" => "serial".into(),
        "ParallelPort" => "parallel".into(),
        "SoundCard" | "HdAudioCard" | "Ensoniq1371" | "SoundBlaster16" => "sound".into(),
        "MachineVMCIDevice" => "vmci".into(),
        "PCIPassthrough" => "hostdev".into(),
        "TPM" => "tpm".into(),
        "WDT" => "watchdog".into(),
        "PrecisionClock" => "clock".into(),
        _ => short_kind(kind),
    }
}

fn controller_bus(kind: &str) -> String {
    match kind {
        "VirtualIDEController" => "ide",
        "VirtualAHCIController" => "sata",
        "VirtualNVMEController" => "nvme",
        "ParaVirtualSCSIController" => "pvscsi",
        "VirtualLsiLogicController"
        | "VirtualLsiLogicSASController"
        | "VirtualBusLogicController" => "scsi",
        "VirtualUSBController" | "VirtualUSBXHCIController" => "usb",
        other => return short_kind(other),
    }
    .into()
}

fn disk(d: &Device, controllers: &HashMap<i64, &Device>) -> Disk {
    let device = match d.kind.as_str() {
        "VirtualCdrom" => "cdrom",
        "VirtualFloppy" => "floppy",
        _ => "disk",
    };
    let file = d.file_name.clone().filter(|f| !f.is_empty());
    let source = match (device, d.backing_kind.as_str(), file) {
        (_, k, Some(f)) if !k.contains("RawDiskMapping") => DiskSource::File(f),
        (_, k, _) if k.contains("RawDiskMapping") => {
            DiskSource::Block(d.device_name.clone().unwrap_or_else(|| "raw device".into()))
        }
        (_, k, _) if k.contains("Remote") => DiskSource::Other("client device".into()),
        (_, k, _) if k.contains("Atapi") || k.contains("Passthrough") || k.contains("Device") => {
            DiskSource::Other("host device".into())
        }
        ("disk", _, None) => DiskSource::Other(d.backing_kind.clone()),
        _ => DiskSource::Empty,
    };
    let controller = d.controller_key.and_then(|k| controllers.get(&k));
    let bus = controller.map(|c| controller_bus(&c.kind));
    let target = match (controller, d.unit) {
        (Some(c), Some(u)) => Some(format!("{} unit {u}", c.label)),
        _ => Some(d.label.clone()).filter(|l| !l.is_empty()),
    };
    Disk {
        device: device.into(),
        target,
        bus,
        source,
        format: match d.thin {
            Some(true) => Some("thin".into()),
            Some(false) if device == "disk" => Some("thick".into()),
            _ => None,
        },
        readonly: false,
        shareable: d.sharing.as_deref() == Some("sharingMultiWriter"),
        size: d.capacity_bytes.map(|c| BlockSize {
            capacity: Some(c),
            allocation: None,
            physical: None,
        }),
    }
}

fn interface(d: &Device, portgroups: &HashMap<&str, &str>) -> Interface {
    let b = d.backing_kind.as_str();
    let (kind, source) = if b.contains("DistributedVirtualPort") {
        (
            "distributed port group",
            d.portgroup_key.as_deref().map(|k| {
                portgroups
                    .get(k)
                    .map_or_else(|| k.to_string(), |n| n.to_string())
            }),
        )
    } else if b.contains("Opaque") {
        ("opaque network", d.device_name.clone())
    } else {
        ("port group", d.device_name.clone())
    };
    let kind = if d.connected == Some(false) {
        format!("{kind} (disconnected)")
    } else {
        kind.to_string()
    };
    Interface {
        kind,
        source,
        mac: d.mac.clone(),
        model: Some(short_kind(&d.kind)),
        target_dev: None,
    }
}

fn device(n: Node) -> Device {
    let xsi_type = |n: Node| {
        n.attribute((XSI, "type"))
            .map(soap::strip_prefix)
            .unwrap_or("")
            .to_string()
    };
    let int = |n: Option<Node>, name: &str| child_text_opt(n, name).and_then(|v| v.parse().ok());
    let info = child(n, "deviceInfo");
    let backing = child(n, "backing");
    let connectable = child(n, "connectable");
    Device {
        kind: xsi_type(n),
        key: int(Some(n), "key"),
        label: child_text_opt(info, "label")
            .unwrap_or_default()
            .trim()
            .to_string(),
        summary: child_text_opt(info, "summary")
            .unwrap_or_default()
            .trim()
            .to_string(),
        controller_key: int(Some(n), "controllerKey"),
        unit: int(Some(n), "unitNumber"),
        backing_kind: backing.map(xsi_type).unwrap_or_default(),
        file_name: child_text_opt(backing, "fileName"),
        device_name: child_text_opt(backing, "deviceName"),
        portgroup_key: backing
            .and_then(|b| child(b, "port"))
            .and_then(|p| child_text(p, "portgroupKey")),
        thin: child_text_opt(backing, "thinProvisioned").map(|v| v == "true"),
        sharing: child_text_opt(backing, "sharing"),
        capacity_bytes: child_text_opt(Some(n), "capacityInBytes")
            .and_then(|v| v.parse().ok())
            .or_else(|| {
                child_text_opt(Some(n), "capacityInKB")
                    .and_then(|v| v.parse::<u64>().ok())
                    .map(|k| k * 1024)
            }),
        mac: child_text_opt(Some(n), "macAddress"),
        connected: child_text_opt(connectable, "connected").map(|v| v == "true"),
    }
}

fn child<'a, 'i>(n: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    n.children()
        .find(|c| c.is_element() && c.tag_name().name() == name)
}

fn child_text(n: Node, name: &str) -> Option<String> {
    child(n, name).map(|c| c.text().unwrap_or("").trim().to_string())
}

fn child_text_opt(n: Option<Node>, name: &str) -> Option<String> {
    n.and_then(|n| child_text(n, name))
}

fn moref(n: Node) -> MoRef {
    MoRef::new(
        soap::ref_type(n).unwrap_or_default(),
        n.text().unwrap_or_default().trim(),
    )
}
