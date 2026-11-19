//! TC-132: vSphere answers mapped onto the model (fixtures from vcsim).

use std::path::Path;

use vi_core::model::{DiskSource, DomainState, Firmware};
use vi_vsphere::About;
use vi_vsphere::inventory::parse;

fn fixture(name: &str) -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/vsphere")
        .join(name);
    // Normalise line endings: a Windows checkout may have CRLF.
    std::fs::read_to_string(p).unwrap().replace("\r\n", "\n")
}

fn about(api: &str) -> About {
    About {
        api_type: api.into(),
        version: "8.0".into(),
        full_name: format!("{api} simulator"),
    }
}

#[test]
fn esx() {
    let inv = parse(&[fixture("esx-inventory.xml")], &about("HostAgent")).unwrap();
    assert_eq!(inv.hypervisor.as_deref(), Some("ESXi"));
    assert_eq!(inv.library.as_deref(), Some("HostAgent simulator"));
    assert!(inv.warnings.is_empty(), "{:?}", inv.warnings);
    let names: Vec<_> = inv.domains.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["ha-host_VM0", "ha-host_VM1"]);
    let vm = &inv.domains[0];
    assert_eq!(vm.state, DomainState::Running);
    assert_eq!((vm.persistent, vm.autostart, vm.id), (None, None, None));
    let x = vm.details.as_ref().unwrap();
    let v = x.general.vsphere.as_ref().unwrap();
    assert_eq!(v.datacenter.as_deref(), Some("ha-datacenter"));
    assert_eq!(v.cluster, None);
    assert!(v.esxi_host.is_some());
    assert!(v.hardware_version.as_deref().unwrap().starts_with("vmx-"));
    assert!(!v.template);
    assert_eq!(x.general.firmware, Firmware::Bios);
    assert_eq!(x.general.domain_type.as_deref(), Some("vmware"));
    assert_eq!(vm.uuid, x.general.uuid);
    assert!(x.graphics.is_empty(), "no consoles for vSphere");
    let disk = x.disks.iter().find(|d| d.device == "disk").unwrap();
    assert!(
        matches!(&disk.source, DiskSource::File(f) if f.starts_with('[') && f.ends_with(".vmdk"))
    );
    assert_eq!(disk.format.as_deref(), Some("thin"));
    assert_eq!(disk.size.unwrap().capacity, Some(10 * 1024 * 1024 * 1024));
    assert!(disk.target.as_deref().unwrap().contains("unit 0"));
    let nic = &x.interfaces[0];
    assert_eq!(nic.kind, "port group");
    assert_eq!(nic.source.as_deref(), Some("VM Network"));
    assert!(nic.mac.as_deref().unwrap().starts_with("00:00:5e:00:53:"));
    assert_eq!(nic.model.as_deref(), Some("e1000"));
    assert!(x.other_devices.iter().any(|d| d.kind == "controller"));
    assert!(x.other_devices.iter().any(|d| d.kind == "video"));
    assert!(x.xml.starts_with("<objects>") && x.xml.ends_with("</objects>"));
}

#[test]
fn vcenter() {
    let inv = parse(&[fixture("vcenter-inventory.xml")], &about("VirtualCenter")).unwrap();
    assert_eq!(inv.hypervisor.as_deref(), Some("vCenter"));
    assert_eq!(inv.domains.len(), 4);
    let info = |name: &str| {
        inv.domains
            .iter()
            .find(|d| d.name == name)
            .unwrap()
            .details
            .as_ref()
            .unwrap()
            .general
            .vsphere
            .clone()
            .unwrap()
    };
    let c = info("DC0_C0_RP0_VM0");
    assert_eq!(c.datacenter.as_deref(), Some("DC0"));
    assert_eq!(c.cluster.as_deref(), Some("DC0_C0"));
    let h = info("DC0_H0_VM0");
    assert_eq!(h.cluster, None, "a standalone host is not a cluster");
    assert_eq!(h.datacenter.as_deref(), Some("DC0"));
    // A distributed port group by its name, not its key.
    let vm = inv
        .domains
        .iter()
        .find(|d| d.name == "DC0_C0_RP0_VM0")
        .unwrap();
    let nic = &vm.details.as_ref().unwrap().interfaces[0];
    assert_eq!(nic.kind, "distributed port group");
    assert!(
        !nic.source.as_deref().unwrap().starts_with("dvportgroup-"),
        "{:?}",
        nic.source
    );
}

// A restricted view (SPEC-021): a VM whose configuration may not be read,
// and a host that is not visible. Not an error: fields stay unknown.
#[test]
fn restricted_view() {
    let full = fixture("vcenter-inventory.xml");
    // Drop every config.* property of one VM, as a server does when the
    // user lacks the privilege, and the HostSystem objects entirely.
    let mut page = String::new();
    for part in full.split_inclusive("</objects>") {
        if part.contains("<obj type=\"HostSystem\">") {
            let start = part.find("<objects>").unwrap();
            page.push_str(&part[..start]);
            continue;
        }
        if part.contains(">DC0_H0_VM0</val>") {
            let mut kept = String::new();
            for p in part.split_inclusive("</propSet>") {
                if !p.contains("<name>config.") {
                    kept.push_str(p);
                }
            }
            page.push_str(&kept.replace(
                "</objects>",
                "<missingSet><path>config</path><fault><fault xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:type=\"NoPermission\"></fault></fault></missingSet></objects>",
            ));
            continue;
        }
        page.push_str(part);
    }
    let inv = parse(&[page], &about("VirtualCenter")).unwrap();
    assert_eq!(inv.domains.len(), 4);
    let vm = inv.domains.iter().find(|d| d.name == "DC0_H0_VM0").unwrap();
    assert_eq!(vm.details, None);
    assert_eq!(vm.uuid, None);
    assert_eq!(vm.state, DomainState::Running, "the state is still known");
    assert_eq!(vm.error, None, "not an error");
    assert!(
        inv.warnings.iter().any(|w| w.contains("DC0_H0_VM0")),
        "{:?}",
        inv.warnings
    );
    // Hosts not visible: no host name, the rest still there.
    let other = inv
        .domains
        .iter()
        .find(|d| d.name == "DC0_C0_RP0_VM0")
        .unwrap();
    let v = other
        .details
        .as_ref()
        .unwrap()
        .general
        .vsphere
        .as_ref()
        .unwrap();
    assert_eq!(v.esxi_host, None);
    assert_eq!(v.cluster, None);
    assert_eq!(v.datacenter.as_deref(), Some("DC0"));
}

#[test]
fn unreadable_answer() {
    assert!(parse(&["<not xml".into()], &about("HostAgent")).is_err());
}
