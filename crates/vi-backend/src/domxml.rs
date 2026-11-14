//! Domain XML (`virsh dumpxml`) into the component model (SPEC-010).
//!
//! Tolerant: unknown elements and attributes are ignored, both `'` and `"`
//! quoting work, missing values become `None`. Only malformed XML is an error.

use roxmltree::{Document, Node};
use vi_core::model::{
    Cpu, Disk, DiskSource, DomainDetails, Firmware, General, Graphics, Interface, Listen, Memory,
    OtherDevice, Topology,
};

const LIBOSINFO_NS: &str = "http://libosinfo.org/xmlns/libvirt/domain/1.0";

/// Parse a domain XML. Disk sizes are filled in later from `domblkinfo`.
pub fn parse(xml: &str) -> Result<DomainDetails, String> {
    let doc = Document::parse(xml).map_err(|e| e.to_string())?;
    let root = doc.root_element();
    if root.tag_name().name() != "domain" {
        return Err(format!(
            "root element is <{}>, expected <domain>",
            root.tag_name().name()
        ));
    }
    let mut d = DomainDetails {
        general: general(root),
        cpu: cpu(root),
        memory: memory(root),
        xml: xml.to_string(),
        ..DomainDetails::default()
    };
    if let Some(devices) = child(root, "devices") {
        for dev in devices.children().filter(Node::is_element) {
            match dev.tag_name().name() {
                "disk" => d.disks.push(disk(dev)),
                "interface" => d.interfaces.push(interface(dev)),
                "graphics" => d.graphics.push(graphics(dev)),
                "emulator" => {}
                kind => d.other_devices.push(OtherDevice {
                    kind: kind.to_string(),
                    summary: summary(dev),
                }),
            }
        }
    }
    Ok(d)
}

fn child<'a, 'i>(n: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    n.children().find(|c| {
        c.is_element() && c.tag_name().name() == name && c.tag_name().namespace().is_none()
    })
}

fn attr(n: Option<Node>, name: &str) -> Option<String> {
    n?.attribute(name).map(str::to_string)
}

fn text(n: Option<Node>) -> Option<String> {
    let t = n?.text()?.trim();
    (!t.is_empty()).then(|| t.to_string())
}

fn general(root: Node) -> General {
    let os = child(root, "os");
    let os_type = os.and_then(|o| child(o, "type"));
    let loader = os.and_then(|o| child(o, "loader"));
    let uefi = attr(os, "firmware").as_deref() == Some("efi")
        || attr(loader, "type").as_deref() == Some("pflash")
        || text(loader).is_some_and(|p| {
            let p = p.to_ascii_lowercase();
            p.contains("ovmf") || p.contains("edk2") || p.contains("aavmf")
        });
    let guest_os_id = child(root, "metadata").and_then(|m| {
        m.descendants()
            .find(|n| {
                n.is_element()
                    && n.tag_name().name() == "os"
                    && n.tag_name().namespace() == Some(LIBOSINFO_NS)
            })
            .and_then(|n| n.attribute("id").map(str::to_string))
    });
    General {
        uuid: text(child(root, "uuid")),
        domain_type: root.attribute("type").map(str::to_string),
        os_type: text(os_type),
        arch: attr(os_type, "arch"),
        machine: attr(os_type, "machine"),
        firmware: if uefi { Firmware::Uefi } else { Firmware::Bios },
        guest_os_id,
        title: text(child(root, "title")),
        description: text(child(root, "description")),
    }
}

fn cpu(root: Node) -> Cpu {
    let vcpu = child(root, "vcpu");
    let max = text(vcpu).and_then(|v| v.parse().ok());
    let current = attr(vcpu, "current").and_then(|v| v.parse().ok()).or(max);
    let c = child(root, "cpu");
    let topology = c.and_then(|c| child(c, "topology")).map(|t| {
        let n = |a| t.attribute(a).and_then(|v| v.parse().ok());
        Topology {
            sockets: n("sockets"),
            dies: n("dies"),
            cores: n("cores"),
            threads: n("threads"),
        }
    });
    Cpu {
        current,
        max,
        mode: attr(c, "mode"),
        model: text(c.and_then(|c| child(c, "model"))),
        topology,
    }
}

/// Memory value in KiB; `unit` defaults to KiB (libvirt).
fn kib(n: Option<Node>) -> Option<u64> {
    let v: u64 = text(n)?.parse().ok()?;
    let factor_bytes: u64 = match attr(n, "unit").as_deref().unwrap_or("KiB") {
        "b" | "bytes" => 1,
        "KB" => 1_000,
        "k" | "KiB" => 1 << 10,
        "MB" => 1_000_000,
        "M" | "MiB" => 1 << 20,
        "GB" => 1_000_000_000,
        "G" | "GiB" => 1 << 30,
        "TB" => 1_000_000_000_000,
        "T" | "TiB" => 1 << 40,
        _ => return None,
    };
    Some(v.checked_mul(factor_bytes)? / 1024)
}

fn memory(root: Node) -> Memory {
    Memory {
        max_kib: kib(child(root, "memory")),
        current_kib: kib(child(root, "currentMemory")),
        hugepages: child(root, "memoryBacking")
            .and_then(|m| child(m, "hugepages"))
            .is_some(),
    }
}

fn disk(n: Node) -> Disk {
    let kind = n.attribute("type").unwrap_or("file");
    let src = child(n, "source");
    let source = match (kind, src) {
        (_, None) => DiskSource::Empty,
        ("file", s) => attr(s, "file").map_or(DiskSource::Empty, DiskSource::File),
        ("block", s) => attr(s, "dev").map_or(DiskSource::Empty, DiskSource::Block),
        ("dir", s) => attr(s, "dir").map_or(DiskSource::Empty, DiskSource::Dir),
        ("network", Some(s)) => DiskSource::Network {
            protocol: attr(Some(s), "protocol"),
            name: attr(Some(s), "name"),
            hosts: s
                .children()
                .filter(|h| h.is_element() && h.tag_name().name() == "host")
                .filter_map(|h| {
                    let name = h.attribute("name")?;
                    Some(match h.attribute("port") {
                        Some(p) => format!("{name}:{p}"),
                        None => name.to_string(),
                    })
                })
                .collect(),
        },
        ("volume", Some(s)) => DiskSource::Volume {
            pool: attr(Some(s), "pool"),
            volume: attr(Some(s), "volume"),
        },
        (other, _) => DiskSource::Other(other.to_string()),
    };
    let target = child(n, "target");
    Disk {
        device: n.attribute("device").unwrap_or("disk").to_string(),
        target: attr(target, "dev"),
        bus: attr(target, "bus"),
        source,
        format: attr(child(n, "driver"), "type"),
        readonly: child(n, "readonly").is_some(),
        shareable: child(n, "shareable").is_some(),
        size: None,
    }
}

fn interface(n: Node) -> Interface {
    let src = child(n, "source");
    Interface {
        kind: n.attribute("type").unwrap_or("unknown").to_string(),
        source: attr(src, "network")
            .or_else(|| attr(src, "bridge"))
            .or_else(|| attr(src, "dev")),
        mac: attr(child(n, "mac"), "address"),
        model: attr(child(n, "model"), "type"),
        target_dev: attr(child(n, "target"), "dev"),
    }
}

fn port(v: Option<String>) -> Option<u16> {
    // -1 (autoport on a stopped domain) and junk become None.
    v?.parse().ok()
}

fn graphics(n: Node) -> Graphics {
    let l = child(n, "listen");
    let listen = match attr(l, "type").as_deref() {
        Some("none") => Listen::None,
        Some("socket") => Listen::Socket(attr(l, "socket").or_else(|| attr(Some(n), "socket"))),
        Some("network") => Listen::Network {
            network: attr(l, "network"),
            address: attr(l, "address"),
        },
        Some(_) => Listen::Address(attr(l, "address").or_else(|| attr(Some(n), "listen"))),
        None => match (n.attribute("listen"), n.attribute("socket")) {
            (_, Some(s)) => Listen::Socket(Some(s.to_string())),
            (a, None) => Listen::Address(a.map(str::to_string)),
        },
    };
    Graphics {
        kind: n.attribute("type").unwrap_or("unknown").to_string(),
        listen,
        port: port(attr(Some(n), "port")),
        tls_port: port(attr(Some(n), "tlsPort")),
        autoport: n.attribute("autoport") == Some("yes"),
    }
}

/// A one-line description of any other device.
fn summary(n: Node) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut add = |s: Option<String>| {
        if let Some(s) = s.filter(|s| !s.is_empty())
            && !parts.contains(&s)
        {
            parts.push(s);
        }
    };
    add(attr(Some(n), "type"));
    add(attr(Some(n), "model"));
    let model = child(n, "model");
    add(attr(model, "type").or_else(|| attr(model, "name")));
    match n.tag_name().name() {
        "channel" => add(attr(child(n, "target"), "name")),
        "input" => add(attr(Some(n), "bus")),
        "filesystem" => {
            let s = child(n, "source");
            let src = attr(s, "dir").or_else(|| attr(s, "name"));
            let dst = attr(child(n, "target"), "dir");
            if src.is_some() || dst.is_some() {
                add(Some(format!(
                    "{} -> {}",
                    src.unwrap_or_else(|| "?".into()),
                    dst.unwrap_or_else(|| "?".into())
                )));
            }
        }
        "hostdev" => {
            let s = child(n, "source");
            let addr = s.and_then(|s| child(s, "address"));
            if let Some(a) = addr {
                let f = |k| a.attribute(k).unwrap_or("?");
                add(Some(
                    if a.attribute("bus").is_some() && a.attribute("slot").is_some() {
                        format!(
                            "{}:{}:{}.{}",
                            f("domain"),
                            f("bus"),
                            f("slot"),
                            f("function")
                        )
                    } else {
                        format!("bus {} device {}", f("bus"), f("device"))
                    },
                ));
            }
            let id = |k| attr(s.and_then(|s| child(s, k)), "id");
            if let (Some(v), Some(p)) = (id("vendor"), id("product")) {
                add(Some(format!("{v}:{p}")));
            }
        }
        _ => {}
    }
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    // TC-043 (synthetic: no UEFI capture yet)
    #[test]
    fn uefi_spice_socket_topology() {
        let x = r#"<domain type="kvm"><name>u</name>
          <memory unit="GiB">4</memory><currentMemory unit="MiB">2048</currentMemory>
          <memoryBacking><hugepages/></memoryBacking>
          <vcpu current="2">8</vcpu>
          <os firmware="efi"><type arch="aarch64" machine="virt">hvm</type></os>
          <cpu mode="custom"><model>Skylake</model><topology sockets="1" dies="1" cores="4" threads="2"/></cpu>
          <devices>
            <graphics type="spice"><listen type="socket" socket="/run/x.sock"/></graphics>
            <graphics type="vnc" port="5900" tlsPort="5901" listen="0.0.0.0"/>
            <graphics type="vnc" port="5902"><listen type="network" network="lan" address="192.0.2.10"/></graphics>
          </devices></domain>"#;
        let d = parse(x).unwrap();
        assert_eq!(d.general.firmware, Firmware::Uefi);
        assert_eq!(d.general.arch.as_deref(), Some("aarch64"));
        assert_eq!(d.memory.max_kib, Some(4 * 1024 * 1024));
        assert_eq!(d.memory.current_kib, Some(2048 * 1024));
        assert!(d.memory.hugepages);
        assert_eq!((d.cpu.current, d.cpu.max), (Some(2), Some(8)));
        assert_eq!(d.cpu.model.as_deref(), Some("Skylake"));
        let t = d.cpu.topology.unwrap();
        assert_eq!(
            (t.sockets, t.dies, t.cores, t.threads),
            (Some(1), Some(1), Some(4), Some(2))
        );
        assert_eq!(
            d.graphics[0].listen,
            Listen::Socket(Some("/run/x.sock".into()))
        );
        assert!(d.graphics[0].reachable());
        assert_eq!(
            d.graphics[1].listen,
            Listen::Address(Some("0.0.0.0".into()))
        );
        assert_eq!(
            d.graphics[2].listen,
            Listen::Network {
                network: Some("lan".into()),
                address: Some("192.0.2.10".into())
            }
        );
        assert_eq!(d.graphics[2].listen.describe(), "network lan (192.0.2.10)");
        assert_eq!(
            (d.graphics[1].port, d.graphics[1].tls_port),
            (Some(5900), Some(5901))
        );

        let pflash = r#"<domain><os><type>hvm</type><loader readonly="yes" type="pflash">/usr/share/OVMF/OVMF_CODE.fd</loader></os></domain>"#;
        assert_eq!(parse(pflash).unwrap().general.firmware, Firmware::Uefi);
    }

    // TC-044 (synthetic)
    #[test]
    fn network_volume_block_disks() {
        let x = r#"<domain><devices>
          <disk type="network" device="disk"><source protocol="rbd" name="pool/img">
            <host name="mon1" port="6789"/><host name="mon2"/></source><target dev="vda" bus="virtio"/></disk>
          <disk type="volume" device="disk"><source pool="default" volume="v.qcow2"/><target dev="vdb"/><shareable/></disk>
          <disk type="block" device="lun"><source dev="/dev/sdz"/><target dev="sdb" bus="scsi"/></disk>
          <disk type="nvme" device="disk"><source type="pci"/><target dev="vdc"/></disk>
          <disk type="file" device="floppy"><target dev="fda"/></disk>
        </devices></domain>"#;
        let d = parse(x).unwrap();
        assert_eq!(
            d.disks[0].source,
            DiskSource::Network {
                protocol: Some("rbd".into()),
                name: Some("pool/img".into()),
                hosts: vec!["mon1:6789".into(), "mon2".into()]
            }
        );
        assert_eq!(
            d.disks[0].source.describe(),
            "rbd://mon1:6789,mon2/pool/img"
        );
        assert_eq!(d.disks[1].source.describe(), "default/v.qcow2");
        assert!(d.disks[1].shareable);
        assert_eq!(d.disks[2].source, DiskSource::Block("/dev/sdz".into()));
        assert_eq!(d.disks[2].device, "lun");
        assert_eq!(d.disks[3].source, DiskSource::Other("nvme".into()));
        assert_eq!(d.disks[4].source, DiskSource::Empty);
    }

    // TC-045
    #[test]
    fn unknown_elements_ignored_bad_xml_rejected() {
        let x = r#"<domain type="kvm" future="1"><frobnicate a="b"><x/></frobnicate>
          <devices><teleporter model="t1000"/></devices></domain>"#;
        let d = parse(x).unwrap();
        assert_eq!(d.other_devices[0].kind, "teleporter");
        assert_eq!(d.other_devices[0].summary, "t1000");
        assert!(parse("<domain><name>x</domain>").is_err());
        assert!(parse("<network/>").is_err());
        assert!(parse("").is_err());
    }

    #[test]
    fn other_device_summaries() {
        let x = r#"<domain><devices>
          <hostdev mode="subsystem" type="pci"><source><address domain="0x0000" bus="0x03" slot="0x00" function="0x0"/></source></hostdev>
          <hostdev mode="subsystem" type="usb"><source><vendor id="0x1234"/><product id="0xbeef"/></source></hostdev>
          <filesystem type="mount"><source dir="/srv/share"/><target dir="share"/></filesystem>
          <channel type="unix"><target type="virtio" name="org.qemu.guest_agent.0"/></channel>
          <video><model type="virtio" heads="1"/></video>
        </devices></domain>"#;
        let s: Vec<String> = parse(x)
            .unwrap()
            .other_devices
            .into_iter()
            .map(|o| format!("{}: {}", o.kind, o.summary))
            .collect();
        assert_eq!(
            s,
            [
                "hostdev: pci 0x0000:0x03:0x00.0x0",
                "hostdev: usb 0x1234:0xbeef",
                "filesystem: mount /srv/share -> share",
                "channel: unix org.qemu.guest_agent.0",
                "video: virtio",
            ]
        );
    }

    #[test]
    fn memory_units() {
        let m = |u: &str, v: &str| {
            parse(&format!(
                r#"<domain><memory unit="{u}">{v}</memory></domain>"#
            ))
            .unwrap()
            .memory
            .max_kib
        };
        assert_eq!(m("KiB", "1024"), Some(1024));
        assert_eq!(m("bytes", "2048"), Some(2));
        assert_eq!(m("G", "1"), Some(1024 * 1024));
        assert_eq!(m("parsec", "1"), None);
        assert_eq!(
            parse("<domain><memory>8</memory></domain>")
                .unwrap()
                .memory
                .max_kib,
            Some(8)
        );
    }
}
