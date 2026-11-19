//! TC-133: against vcsim (govmomi's vSphere simulator), in ESXi and in
//! vCenter mode. Runs only when `VI_VCSIM` names the vcsim binary (CI job
//! "vSphere simulator", Plan 8.6); otherwise each test says it skipped.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use vi_ssh::ScriptedPrompter;
use vi_vsphere::{KnownCerts, Settings, TlsOptions, VsphereError, VsphereSession};

pub struct Sim {
    child: Child,
    pub port: u16,
}

impl Drop for Sim {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Start vcsim (`esx` = standalone ESXi) with user `reader`, password `pw`.
pub fn start(esx: bool) -> Option<Sim> {
    start_with(esx, "pw")
}

/// [`start`] with another password.
pub fn start_with(esx: bool, password: &str) -> Option<Sim> {
    let bin = PathBuf::from(std::env::var_os("VI_VCSIM")?);
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let mut cmd = Command::new(bin);
    cmd.args([
        "-l",
        &format!("127.0.0.1:{port}"),
        "-username",
        "reader",
        "-password",
        password,
    ]);
    if esx {
        cmd.arg("-esx");
    }
    let child = cmd
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start vcsim");
    let sim = Sim { child, port };
    let end = Instant::now() + Duration::from_secs(20);
    while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(Instant::now() < end, "vcsim did not start");
        std::thread::sleep(Duration::from_millis(100));
    }
    Some(sim)
}

pub fn settings(dir: &Path, port: u16) -> Settings {
    Settings {
        name: "sim".into(),
        host: "127.0.0.1".into(),
        port,
        user: "reader".into(),
        tls: TlsOptions {
            known: KnownCerts::new(dir.join("known_certs")),
            extra_roots: Vec::new(),
            connect_timeout: Duration::from_secs(10),
        },
        timeout: Duration::from_secs(30),
    }
}

pub fn prompter(passwords: &[&str]) -> ScriptedPrompter {
    let mut p = ScriptedPrompter::new(false, None);
    p.accept_certificate = true;
    *p.passwords.get_mut().unwrap() = passwords.iter().map(|s| s.to_string()).collect();
    p
}

fn skip() {
    eprintln!("skipped: set VI_VCSIM to the vcsim binary to run this test");
}

#[tokio::test(flavor = "multi_thread")]
async fn esx_login_inventory_logout() {
    let Some(sim) = start(true) else {
        return skip();
    };
    let d = tempfile::tempdir().unwrap();
    let s = settings(d.path(), sim.port);
    // First a wrong password, then the right one: asked twice, with retry.
    let p = prompter(&["wrong", "pw"]);
    let session = VsphereSession::connect(&s, &p).await.unwrap();
    assert_eq!(p.certificate_prompts.lock().unwrap().len(), 1);
    let asked = p.password_prompts.lock().unwrap().clone();
    assert_eq!(asked.len(), 2);
    assert!(!asked[0].retry && asked[1].retry);
    assert_eq!(session.about().api_type, "HostAgent");
    assert!(!session.about().is_vcenter());
    let pages = session.inventory_pages().await.unwrap();
    let all = pages.concat();
    assert!(
        all.contains("type=\"VirtualMachine\""),
        "{}",
        &all[..all.len().min(500)]
    );
    let inv = vi_vsphere::inventory::parse(&pages, session.about()).unwrap();
    assert_eq!(inv.hypervisor.as_deref(), Some("ESXi"));
    assert!(!inv.domains.is_empty());
    if let Some(dir) = std::env::var_os("VI_DUMP") {
        std::fs::write(std::path::Path::new(&dir).join("esx-page0.xml"), &pages[0]).unwrap();
    }
    // Pinned: the next connect does not ask about the certificate.
    session.logout().await;
    let p2 = prompter(&["pw"]);
    let again = VsphereSession::connect(&s, &p2).await.unwrap();
    assert!(p2.certificate_prompts.lock().unwrap().is_empty());
    drop(again);
}

#[tokio::test(flavor = "multi_thread")]
async fn login_failed_cancelled_rejected() {
    let Some(sim) = start(true) else {
        return skip();
    };
    let d = tempfile::tempdir().unwrap();
    let s = settings(d.path(), sim.port);
    // Three wrong passwords: failed, no fourth question.
    let p = prompter(&["a", "b", "c", "pw"]);
    match VsphereSession::connect(&s, &p).await {
        Err(VsphereError::LoginFailed { user, .. }) => assert_eq!(user, "reader"),
        other => panic!("{:?}", other.err()),
    }
    assert_eq!(p.password_prompts.lock().unwrap().len(), 3);
    // No password given: cancelled.
    let p = prompter(&[]);
    assert!(matches!(
        VsphereSession::connect(&s, &p).await,
        Err(VsphereError::LoginCancelled { .. })
    ));
    // Certificate rejected (fresh known_certs): nothing pinned.
    let d2 = tempfile::tempdir().unwrap();
    let s2 = settings(d2.path(), sim.port);
    let mut p = prompter(&["pw"]);
    p.accept_certificate = false;
    assert!(matches!(
        VsphereSession::connect(&s2, &p).await,
        Err(VsphereError::CertificateRejected { .. })
    ));
    assert!(s2.tls.known.load().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn vcenter_inventory_in_pages() {
    let Some(sim) = start(false) else {
        return skip();
    };
    let d = tempfile::tempdir().unwrap();
    let s = settings(d.path(), sim.port);
    let session = VsphereSession::connect(&s, &prompter(&["pw"]))
        .await
        .unwrap();
    assert!(session.about().is_vcenter());
    let pages = session.inventory_pages().await.unwrap();
    assert!(!pages.is_empty());
    let all = pages.concat();
    for kind in [
        "VirtualMachine",
        "HostSystem",
        "ClusterComputeResource",
        "Datacenter",
    ] {
        assert!(all.contains(&format!("type=\"{kind}\"")), "no {kind}");
    }
    // Small pages: the continuation gives the same objects.
    let small = session.inventory_pages_of(3).await.unwrap();
    assert!(small.len() > 2, "{} pages", small.len());
    let count = |pages: &[String]| {
        pages
            .iter()
            .map(|p| p.matches("<objects>").count())
            .sum::<usize>()
    };
    assert_eq!(count(&small), count(&pages));
}

// TC-132 (vcsim part): the vCenter inventory mapped onto the model.
#[tokio::test(flavor = "multi_thread")]
async fn vcenter_mapping() {
    let Some(sim) = start(false) else {
        return skip();
    };
    let d = tempfile::tempdir().unwrap();
    let s = settings(d.path(), sim.port);
    let session = VsphereSession::connect(&s, &prompter(&["pw"]))
        .await
        .unwrap();
    let pages = session.inventory_pages().await.unwrap();
    let inv = vi_vsphere::inventory::parse(&pages, session.about()).unwrap();
    // VI_DUMP=<dir>: keep the answer and the mapping for a look.
    if let Some(dir) = std::env::var_os("VI_DUMP") {
        let dir = std::path::PathBuf::from(dir);
        for (i, p) in pages.iter().enumerate() {
            std::fs::write(dir.join(format!("page{i}.xml")), p).unwrap();
        }
        std::fs::write(dir.join("inventory.txt"), format!("{inv:#?}")).unwrap();
    }
    assert_eq!(inv.hypervisor.as_deref(), Some("vCenter"));
    assert!(inv.domains.len() >= 4, "{}", inv.domains.len());
    for vm in &inv.domains {
        let x = vm.details.as_ref().expect("details");
        let v = x.general.vsphere.as_ref().unwrap();
        assert_eq!(v.datacenter.as_deref(), Some("DC0"), "{}", vm.name);
        assert!(v.esxi_host.is_some(), "{}", vm.name);
        assert!(x.cpu.current.is_some() && x.memory.max_kib.is_some());
        assert!(
            !x.disks.is_empty() && !x.interfaces.is_empty(),
            "{}",
            vm.name
        );
        assert!(x.interfaces.iter().all(|i| i.mac.is_some()));
        assert!(x.xml.starts_with("<objects>"));
        assert!(vm.uuid.is_some());
    }
    // Cluster VMs know their cluster; standalone-host VMs have none.
    assert!(inv.domains.iter().any(|d| {
        d.details
            .as_ref()
            .unwrap()
            .general
            .vsphere
            .as_ref()
            .unwrap()
            .cluster
            .as_deref()
            == Some("DC0_C0")
    }));
    assert!(inv.domains.iter().any(|d| {
        d.details
            .as_ref()
            .unwrap()
            .general
            .vsphere
            .as_ref()
            .unwrap()
            .cluster
            .is_none()
    }));
}

// TC-135: the password is in no log line (all levels), not in the
// inventory answers (they become the cache and the raw view) and not in
// known_certs; it is not in any error text either.
#[test]
fn password_never_logged_or_stored() {
    const PW: &str = "Pw-7f3c9-NotInLogs";
    let Some(sim) = start_with(true, PW) else {
        return skip();
    };
    let d = tempfile::tempdir().unwrap();
    let s = settings(d.path(), sim.port);
    let buf = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let writer = {
        let buf = buf.clone();
        move || LogBuf(buf.clone())
    };
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(writer)
        .with_ansi(false)
        .finish();
    // The process-wide logger, not a thread-local one: tracing caches per
    // log statement whether anyone listens, and other tests in this binary
    // run in parallel without a logger, so a thread-local capture can miss
    // lines (it did on CI). Setting the global logger resets that cache.
    // Their lines may end up here too; the passwords below are this test's
    // own. Only this test sets a logger.
    tracing::subscriber::set_global_default(subscriber).expect("only this test sets a logger");
    let pages = {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            // A wrong password first (its error text is checked too).
            let p = prompter(&["Wrong-a1b2", PW]);
            let session = VsphereSession::connect(&s, &p).await.unwrap();
            let pages = session.inventory_pages().await.unwrap();
            session.logout().await;
            drop(session);
            let failed = match VsphereSession::connect(
                &s,
                &prompter(&["Wrong-a1b2", "Wrong-a1b2", "Wrong-a1b2"]),
            )
            .await
            {
                Err(e) => e,
                Ok(_) => panic!("wrong passwords accepted"),
            };
            assert!(!failed.to_string().contains("Wrong-a1b2"), "{failed}");
            pages
        })
    };
    let log = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
    let target = format!("target=127.0.0.1:{}", sim.port);
    assert!(
        log.lines()
            .any(|l| l.contains("vSphere login failed") && l.contains(&target))
            && log
                .lines()
                .any(|l| l.contains("vSphere login ") && l.contains(&target)),
        "the capture works: {log}"
    );
    for secret in [PW, "Wrong-a1b2"] {
        assert!(!log.contains(secret), "password in the log");
        assert!(!pages.concat().contains(secret), "password in an answer");
        let inv = vi_vsphere::inventory::parse(
            &pages,
            &vi_vsphere::About {
                api_type: "HostAgent".into(),
                version: String::new(),
                full_name: String::new(),
            },
        )
        .unwrap();
        assert!(!format!("{inv:?}").contains(secret));
        let certs = std::fs::read_to_string(s.tls.known.path()).unwrap();
        assert!(!certs.contains(secret));
    }
}

struct LogBuf(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for LogBuf {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
