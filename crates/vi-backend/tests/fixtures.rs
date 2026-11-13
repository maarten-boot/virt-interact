//! Inventory from the captured fixtures in `tests/fixtures/` through the
//! mock runner (TC-040..047, TC-052).

use std::fs;
use std::path::{Path, PathBuf};

use vi_backend::{
    BackendErrorKind, HypervisorBackend, LibvirtBackend, VirshCommand, domxml, parse,
};
use vi_core::model::{DiskSource, DomainState, Firmware, HostInventory, Listen};
use vi_ssh::{CommandOutput, MockRunner};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn read(p: &Path) -> String {
    fs::read_to_string(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn backend() -> LibvirtBackend {
    LibvirtBackend::new("qemu:///system", None).unwrap()
}

/// Answers for one capture folder (`version.txt`, `list-all.txt`, `<vm>/...`).
fn add_capture(mut m: MockRunner, b: &LibvirtBackend, dir: &Path) -> MockRunner {
    m = m
        .with(
            &b.command_line(&VirshCommand::Version),
            CommandOutput::ok(read(&dir.join("version.txt"))),
        )
        .with(
            &b.command_line(&VirshCommand::ListAll),
            CommandOutput::ok(read(&dir.join("list-all.txt"))),
        );
    for vm in vm_dirs(dir) {
        m = add_vm(m, b, &vm, None);
    }
    m
}

/// Answers for one VM folder; `dominfo` may be supplied (stopped set).
fn add_vm(mut m: MockRunner, b: &LibvirtBackend, vm: &Path, dominfo: Option<String>) -> MockRunner {
    let name = vm.file_name().unwrap().to_string_lossy().to_string();
    let info = dominfo.unwrap_or_else(|| read(&vm.join("dominfo.txt")));
    m = m
        .with(
            &b.command_line(&VirshCommand::DomInfo(name.clone())),
            CommandOutput::ok(info),
        )
        .with(
            &b.command_line(&VirshCommand::DumpXml(name.clone())),
            CommandOutput::ok(read(&vm.join("dumpxml.xml"))),
        );
    for f in fs::read_dir(vm).unwrap() {
        let f = f.unwrap().path();
        let fname = f.file_name().unwrap().to_string_lossy().to_string();
        if let Some(target) = fname
            .strip_prefix("domblkinfo-")
            .and_then(|t| t.strip_suffix(".txt"))
        {
            m = m.with(
                &b.command_line(&VirshCommand::DomBlkInfo {
                    domain: name.clone(),
                    target: target.into(),
                }),
                CommandOutput::ok(read(&f)),
            );
        }
    }
    m
}

fn vm_dirs(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect();
    v.sort();
    v
}

async fn capture_inventory(set: &str) -> (HostInventory, MockRunner) {
    let b = backend();
    let m = add_capture(MockRunner::new(), &b, &fixtures().join(set));
    let inv = b.inventory(&m).await.expect("inventory");
    (inv, m)
}

// TC-040
#[test]
fn list_fixtures() {
    let l = parse::list(&read(&fixtures().join("libvirt-12.0.0/list-all.txt"))).unwrap();
    assert_eq!(l.len(), 7);
    assert!(l.iter().all(|e| e.state == DomainState::Running));
    assert_eq!(l[5].id, Some(7));
    assert_eq!(l[5].name, "vm-mnzm29");
    let l = parse::list(&read(&fixtures().join("libvirt-10.0.0/list-all.txt"))).unwrap();
    assert_eq!(
        l.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
        ["vm-j2sunl", "vm-wtu5zf", "vm-rcqh1f"]
    );
}

// TC-041
#[test]
fn dominfo_fixtures_both_versions() {
    let d = parse::dominfo(&read(
        &fixtures().join("libvirt-12.0.0/vm-aoufj6/dominfo.txt"),
    ));
    assert_eq!(d.id, Some(1));
    assert_eq!(
        d.uuid.as_deref(),
        Some("ebfcb5aa-d9d1-422f-b4c4-2805b3a442b5")
    );
    assert_eq!(d.state, Some(DomainState::Running));
    assert_eq!((d.persistent, d.autostart), (Some(true), Some(true)));
    let d = parse::dominfo(&read(
        &fixtures().join("libvirt-10.0.0/vm-rcqh1f/dominfo.txt"),
    ));
    assert_eq!((d.id, d.autostart), (Some(3), Some(false)));
}

// TC-046b: every captured XML parses.
#[test]
fn all_fixture_xml_parses() {
    let mut n = 0;
    for set in ["libvirt-12.0.0", "libvirt-10.0.0", "stopped"] {
        for vm in vm_dirs(&fixtures().join(set)) {
            let p = vm.join("dumpxml.xml");
            let d = domxml::parse(&read(&p)).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
            assert!(!d.disks.is_empty(), "{}", p.display());
            assert!(d.memory.max_kib.is_some());
            n += 1;
        }
    }
    assert_eq!(n, 13);
}

// TC-042: BIOS VM, 3 disks + empty cdrom, NAT NIC, VNC (live XML, `'` quoting).
#[test]
fn bios_vm_components() {
    let d = domxml::parse(&read(
        &fixtures().join("libvirt-10.0.0/vm-j2sunl/dumpxml.xml"),
    ))
    .unwrap();
    assert_eq!(d.general.firmware, Firmware::Bios);
    assert_eq!(d.general.domain_type.as_deref(), Some("kvm"));
    assert_eq!(d.general.machine.as_deref(), Some("pc-q35-noble"));
    assert_eq!(d.general.guest_os(), Some("ubuntu/25.10"));
    assert_eq!((d.cpu.current, d.cpu.max), (Some(4), Some(4)));
    assert_eq!(d.cpu.mode.as_deref(), Some("host-passthrough"));
    assert_eq!(d.memory.max_kib, Some(16_777_216));
    assert!(!d.memory.hugepages);
    let targets: Vec<_> = d
        .disks
        .iter()
        .map(|x| x.target.as_deref().unwrap())
        .collect();
    assert_eq!(targets, ["vda", "vdb", "vdc", "sda"]);
    assert_eq!(d.disks[0].format.as_deref(), Some("qcow2"));
    assert!(matches!(&d.disks[0].source, DiskSource::File(p) if p.ends_with("disk0.qcow2")));
    assert_eq!(d.disks[3].device, "cdrom");
    assert_eq!(d.disks[3].source, DiskSource::Empty);
    assert!(d.disks[3].readonly);
    let nic = &d.interfaces[0];
    assert_eq!(nic.kind, "network");
    assert_eq!(nic.source.as_deref(), Some("default"));
    assert_eq!(nic.mac.as_deref(), Some("52:54:00:f3:9d:c4"));
    assert_eq!(nic.model.as_deref(), Some("virtio"));
    assert_eq!(nic.target_dev.as_deref(), Some("vnet0"));
    let g = &d.graphics[0];
    assert_eq!(g.kind, "vnc");
    assert_eq!(g.port, Some(5900));
    assert_eq!(g.listen, Listen::Address(Some("127.0.0.1".into())));
    assert!(
        d.other_devices
            .iter()
            .any(|o| o.kind == "controller" && o.summary == "usb qemu-xhci")
    );
    assert!(
        d.other_devices
            .iter()
            .any(|o| o.kind == "channel" && o.summary.contains("guest_agent"))
    );
}

// TC-046a and SPEC-018 cases from the captures.
#[test]
fn guest_os_title_and_listen_none() {
    let p = |s: &str| domxml::parse(&read(&fixtures().join(s))).unwrap();
    let win = p("libvirt-10.0.0/vm-rcqh1f/dumpxml.xml");
    assert_eq!(win.general.guest_os(), Some("win/10"));
    assert_eq!(win.graphics[0].listen, Listen::None);
    assert!(!win.graphics[0].reachable());
    let fed = p("libvirt-10.0.0/vm-wtu5zf/dumpxml.xml");
    assert_eq!(fed.general.guest_os(), Some("fedora/unknown"));
    assert_eq!(fed.general.title.as_deref(), Some("vm-wtu5zf title"));
    assert_eq!(fed.graphics[0].kind, "spice");
    // Stopped VNC autoport: port -1 → unknown; listen address without address.
    let st = p("stopped/vm-w59any/dumpxml.xml");
    assert_eq!(st.graphics[0].port, None);
    assert!(st.graphics[0].autoport);
    assert_eq!(st.graphics[0].listen, Listen::Address(None));
    assert_eq!(st.interfaces[0].target_dev, None);
}

// TC-047 (running VMs) + TC-046 (sizes merged, allocation > capacity kept).
#[tokio::test]
async fn full_inventory_libvirt_12() {
    let (inv, _) = capture_inventory("libvirt-12.0.0").await;
    assert_eq!(inv.library.as_deref(), Some("libvirt 12.0.0"));
    assert_eq!(inv.hypervisor.as_deref(), Some("QEMU 10.2.1"));
    assert_eq!(inv.domains.len(), 7);
    assert!(inv.warnings.is_empty(), "{:?}", inv.warnings);
    let a = inv.domains.iter().find(|d| d.name == "vm-aoufj6").unwrap();
    assert_eq!(
        a.uuid.as_deref(),
        Some("ebfcb5aa-d9d1-422f-b4c4-2805b3a442b5")
    );
    let det = a.details.as_ref().unwrap();
    let sda = det
        .disks
        .iter()
        .find(|d| d.target.as_deref() == Some("sda"))
        .unwrap();
    let size = sda.size.unwrap();
    assert_eq!(size.capacity, Some(68_719_476_736));
    assert_eq!(size.allocation, Some(68_731_621_376));
    assert!(size.allocation > size.capacity);
}

// TC-047 + TC-052: running and stopped VMs; only allowlisted commands sent.
#[tokio::test]
async fn running_and_stopped_only_allowlisted_commands() {
    let b = backend();
    let dir = fixtures().join("libvirt-10.0.0");
    let mut list = read(&dir.join("list-all.txt"));
    let mut m = add_capture(MockRunner::new(), &b, &dir);
    // The stopped set has only XML: add list rows and dominfo in virsh format.
    for vm in vm_dirs(&fixtures().join("stopped")) {
        let name = vm.file_name().unwrap().to_string_lossy().to_string();
        list.push_str(&format!(" -    {name:<11} shut off\n"));
        let info = format!(
            "Id:             -\nName:           {name}\nUUID:           00000000-0000-4000-8000-000000000000\nOS Type:        hvm\nState:          shut off\nCPU(s):         4\nMax memory:     16777216 KiB\nUsed memory:    16777216 KiB\nPersistent:     yes\nAutostart:      disable\nManaged save:   no\n"
        );
        m = add_vm(m, &b, &vm, Some(info));
    }
    let m = m.with(
        &b.command_line(&VirshCommand::ListAll),
        CommandOutput::ok(list),
    );

    let inv = HypervisorBackend::inventory(&b, &m)
        .await
        .expect("inventory");
    assert_eq!(inv.domains.len(), 6);
    let stopped: Vec<_> = inv
        .domains
        .iter()
        .filter(|d| d.state == DomainState::ShutOff)
        .collect();
    assert_eq!(stopped.len(), 3);
    for d in &stopped {
        assert_eq!(d.id, None);
        let det = d.details.as_ref().unwrap();
        // No domblkinfo captured for stopped VMs: size unknown, not an error.
        assert!(det.disks.iter().all(|x| x.size.is_none()));
    }
    assert!(inv.warnings.is_empty(), "{:?}", inv.warnings);

    const ALLOWED: [&str; 7] = [
        "version",
        "list",
        "dominfo",
        "dumpxml",
        "domblkinfo",
        "domstate",
        "domdisplay",
    ];
    for c in m.commands() {
        let rest = c
            .strip_prefix("LC_ALL=C virsh -r -c 'qemu:///system' ")
            .unwrap_or_else(|| panic!("unexpected command {c}"));
        let sub = rest.split(' ').next().unwrap();
        assert!(ALLOWED.contains(&sub), "{c}");
    }
    // Empty cdroms are not queried.
    assert!(
        !m.commands()
            .iter()
            .any(|c| c.contains("domblkinfo") && c.ends_with("'sda'") && c.contains("vm-j2sunl"))
    );
}

// TC-061 through the backend: host-level failures abort with a classified
// error; a vanished domain is a warning; other per-domain failures are
// recorded on the domain.
#[tokio::test]
async fn error_handling() {
    let b = backend();
    let version = b.command_line(&VirshCommand::Version);
    let m = MockRunner::new().with(
        &version,
        CommandOutput::failed(127, "sh: 1: virsh: not found\n"),
    );
    let e = b.inventory(&m).await.unwrap_err();
    assert_eq!(e.kind(), BackendErrorKind::VirshNotFound);

    let m = MockRunner::new().with(
        &version,
        CommandOutput::failed(1, "error: failed to connect to the hypervisor\nerror: Failed to connect socket to '/var/run/libvirt/libvirt-sock-ro': Permission denied\n"),
    );
    assert_eq!(
        b.inventory(&m).await.unwrap_err().kind(),
        BackendErrorKind::PermissionDenied
    );

    let list = " Id   Name   State\n------------------\n 1    gone   running\n 2    bad    running\n 3    odd    running\n";
    let m = MockRunner::new()
        .with(
            &version,
            CommandOutput::ok("Using library: libvirt 10.0.0\n"),
        )
        .with(
            &b.command_line(&VirshCommand::ListAll),
            CommandOutput::ok(list),
        )
        .with(
            &b.command_line(&VirshCommand::DomInfo("gone".into())),
            CommandOutput::failed(1, "error: failed to get domain 'gone'\n"),
        )
        .with(
            &b.command_line(&VirshCommand::DomInfo("bad".into())),
            CommandOutput::ok("State: running\n"),
        )
        .with(
            &b.command_line(&VirshCommand::DumpXml("bad".into())),
            CommandOutput::ok("<domain><name>bad</nam>"),
        )
        .with(
            &b.command_line(&VirshCommand::DomInfo("odd".into())),
            CommandOutput::ok("State: running\n"),
        )
        .with(
            &b.command_line(&VirshCommand::DumpXml("odd".into())),
            CommandOutput::failed(1, "error: internal error: something odd\n"),
        );
    let inv = b.inventory(&m).await.unwrap();
    assert_eq!(inv.domains.len(), 2);
    let bad = &inv.domains[0];
    assert!(bad.error.as_deref().unwrap().contains("XML parse error"));
    assert!(
        inv.domains[1]
            .error
            .as_deref()
            .unwrap()
            .contains("something odd")
    );
    assert_eq!(inv.warnings.len(), 3, "{:?}", inv.warnings);
    assert!(inv.warnings[0].contains("\"gone\" disappeared"));
    assert!(inv.warnings[1].contains("\"bad\""));
}
