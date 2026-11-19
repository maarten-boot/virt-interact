//! TC-133 (backend part): `for_host` gives the vSphere backend, which reads
//! the inventory through a logged-in session. Runs when `VI_VCSIM` names
//! the vcsim binary (Plan 8.6).

use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use vi_core::config::{Backend, HostConfig};
use vi_ssh::{CommandRunner, ScriptedPrompter};
use vi_vsphere::{KnownCerts, Settings, TlsOptions, VsphereSession};

#[tokio::test(flavor = "multi_thread")]
async fn vsphere_backend_reads_vcsim() {
    let Some(bin) = std::env::var_os("VI_VCSIM") else {
        eprintln!("skipped: set VI_VCSIM to the vcsim binary to run this test");
        return;
    };
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut child = Command::new(bin)
        .args([
            "-esx",
            "-l",
            &format!("127.0.0.1:{port}"),
            "-username",
            "reader",
            "-password",
            "pw",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let end = Instant::now() + Duration::from_secs(20);
    while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(Instant::now() < end, "vcsim did not start");
        std::thread::sleep(Duration::from_millis(100));
    }
    let d = tempfile::tempdir().unwrap();
    let mut host = HostConfig::new("esx", "127.0.0.1");
    host.backend = Backend::Vsphere;
    host.port = Some(port);
    host.user = Some("reader".into());
    let s = Settings {
        name: host.name.clone(),
        host: host.host.clone(),
        port,
        user: "reader".into(),
        tls: TlsOptions {
            known: KnownCerts::new(d.path().join("known_certs")),
            extra_roots: Vec::new(),
            connect_timeout: Duration::from_secs(10),
        },
        timeout: Duration::from_secs(30),
    };
    let mut p = ScriptedPrompter::new(false, None);
    p.accept_certificate = true;
    p.passwords.get_mut().unwrap().push("pw".into());
    let session: Arc<dyn CommandRunner> = Arc::new(VsphereSession::connect(&s, &p).await.unwrap());
    let backend = vi_backend::for_host(&host).unwrap();
    assert_eq!(backend.name(), "vsphere");
    let inv = backend.inventory(&*session).await.unwrap();
    assert_eq!(inv.hypervisor.as_deref(), Some("ESXi"));
    assert!(!inv.domains.is_empty());
    // No consoles; the session runs no commands.
    assert!(
        backend
            .graphics(&*session, &inv.domains[0].name)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(session.run("virsh list").await.is_err());
    // A plain shell runner is refused by the vSphere backend.
    let mock = vi_ssh::MockRunner::new();
    assert!(backend.inventory(&mock).await.is_err());
    drop(session);
    let _ = child.kill();
    let _ = child.wait();
}
