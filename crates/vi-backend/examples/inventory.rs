//! Manual test tool for the libvirt backend (TC-048, Plan 3.6).
//!
//! ```text
//! cargo run -p vi-backend --example inventory -- [--no-agent] [--port N]
//!     [--user U] [--identity PATH] [--known-hosts PATH] [--uri URI]
//!     [--prefix "sudo -n"] [--xml DOMAIN] [--parallel N] [--save DIR] HOST
//! ```
//! Connects like `ssh-run`, runs only the allowlisted read-only virsh
//! commands and prints the inventory. `--xml DOMAIN` also prints that
//! domain's XML. `--parallel N` sets how many domains are queried at once
//! (default 4; compare with 1 to measure). `--save DIR` writes every virsh
//! output in the fixture layout of `tests/fixtures/` (plus `domdisplay` for
//! running domains). That output holds **real** names, addresses and paths:
//! anonymise it with `scripts/anonymise_fixtures.py` before committing.
//! Set `RUST_LOG=vi_backend=debug` to see each command.

use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use vi_backend::{LibvirtBackend, VirshCommand};
use vi_core::config::HostConfig;
use vi_core::model::{Domain, HostInventory, human_bytes};
use vi_ssh::prompt::BoxFuture;
use vi_ssh::{
    CommandOutput, CommandRunner, ConnectOptions, HostKeyPrompt, Prompter, Secret, SshConfig,
    SshError, SshSession, resolve,
};

/// Passes commands through and keeps every output (for `--save`).
struct Recorder<'a> {
    inner: &'a dyn CommandRunner,
    log: Mutex<HashMap<String, CommandOutput>>,
}

impl CommandRunner for Recorder<'_> {
    fn run<'a>(&'a self, command: &'a str) -> BoxFuture<'a, Result<CommandOutput, SshError>> {
        Box::pin(async move {
            let r = self.inner.run(command).await;
            if let Ok(o) = &r {
                self.log
                    .lock()
                    .expect("lock")
                    .insert(command.to_string(), o.clone());
            }
            r
        })
    }
}

/// What virsh printed: stdout on success, else stderr (as in the fixtures).
fn body(o: &CommandOutput) -> String {
    if o.success() {
        o.stdout_text()
    } else {
        o.stderr_text()
    }
}

/// Write the recorded outputs in the fixture layout.
async fn save(
    dir: &Path,
    backend: &LibvirtBackend,
    session: &SshSession,
    rec: &Recorder<'_>,
    inv: &HostInventory,
) -> std::io::Result<usize> {
    let log = rec.log.lock().expect("lock").clone();
    let mut n = 0;
    let mut write = |path: PathBuf, cmd: &VirshCommand| -> std::io::Result<()> {
        if let Some(o) = log.get(&backend.command_line(cmd)) {
            std::fs::create_dir_all(path.parent().expect("parent"))?;
            std::fs::write(&path, body(o))?;
            n += 1;
        }
        Ok(())
    };
    write(dir.join("version.txt"), &VirshCommand::Version)?;
    write(dir.join("list-all.txt"), &VirshCommand::ListAll)?;
    for d in &inv.domains {
        let vm = dir.join(&d.name);
        write(
            vm.join("dominfo.txt"),
            &VirshCommand::DomInfo(d.name.clone()),
        )?;
        write(
            vm.join("dumpxml.xml"),
            &VirshCommand::DumpXml(d.name.clone()),
        )?;
        for disk in d.details.iter().flat_map(|x| &x.disks) {
            if let Some(t) = &disk.target {
                write(
                    vm.join(format!("domblkinfo-{t}.txt")),
                    &VirshCommand::DomBlkInfo {
                        domain: d.name.clone(),
                        target: t.clone(),
                    },
                )?;
            }
        }
    }
    // domdisplay is not part of the inventory; fetch it for running domains.
    for d in inv.domains.iter().filter(|d| d.state.is_active()) {
        match backend
            .run_command(session, &VirshCommand::DomDisplay(d.name.clone()))
            .await
        {
            Ok(o) => {
                std::fs::create_dir_all(dir.join(&d.name))?;
                std::fs::write(dir.join(&d.name).join("domdisplay.txt"), body(&o))?;
                n += 1;
            }
            Err(e) => eprintln!("domdisplay {}: {e}", d.name),
        }
    }
    Ok(n)
}

struct Terminal;

impl Prompter for Terminal {
    fn confirm_host_key<'a>(&'a self, p: &'a HostKeyPrompt) -> BoxFuture<'a, bool> {
        Box::pin(async move {
            eprintln!(
                "The authenticity of host '{}' (port {}) can't be established.\n{} key fingerprint is {}.",
                p.hostname, p.port, p.key_type, p.fingerprint
            );
            eprint!("Accept and add to known_hosts (yes/no)? ");
            let _ = std::io::stderr().flush();
            let mut line = String::new();
            let _ = std::io::stdin().lock().read_line(&mut line);
            line.trim().eq_ignore_ascii_case("yes")
        })
    }

    fn passphrase<'a>(&'a self, key_path: &'a Path) -> BoxFuture<'a, Option<Secret>> {
        Box::pin(async move {
            rpassword::prompt_password(format!("Passphrase for {}: ", key_path.display()))
                .ok()
                .filter(|s| !s.is_empty())
                .map(Secret::new)
        })
    }
}

fn usage() -> ! {
    eprintln!(
        "usage: inventory [--no-agent] [--port N] [--user U] [--identity PATH] [--known-hosts PATH] [--uri URI] [--prefix PREFIX] [--xml DOMAIN] [--parallel N] [--save DIR] HOST"
    );
    std::process::exit(2)
}

fn opt<T>(v: Option<T>) -> String
where
    T: std::fmt::Display,
{
    v.map_or_else(|| "-".to_string(), |v| v.to_string())
}

fn print_domain(d: &Domain) {
    println!(
        "\n== {}  [{}]  id {}  uuid {}  persistent {}  autostart {}",
        d.name,
        d.state.label(),
        opt(d.id),
        d.uuid.as_deref().unwrap_or("-"),
        opt(d.persistent),
        opt(d.autostart)
    );
    if let Some(e) = &d.error {
        println!("   ERROR: {e}");
    }
    let Some(x) = &d.details else { return };
    let g = &x.general;
    println!(
        "   general: {} {} {} machine {} firmware {} guest {}",
        g.domain_type.as_deref().unwrap_or("-"),
        g.os_type.as_deref().unwrap_or("-"),
        g.arch.as_deref().unwrap_or("-"),
        g.machine.as_deref().unwrap_or("-"),
        g.firmware.label(),
        g.guest_os().unwrap_or("-")
    );
    if let Some(t) = &g.title {
        println!("   title: {t}");
    }
    let c = &x.cpu;
    print!(
        "   cpu: {}/{} vCPU, mode {}",
        opt(c.current),
        opt(c.max),
        c.mode.as_deref().unwrap_or("-")
    );
    if let Some(m) = &c.model {
        print!(", model {m}");
    }
    if let Some(t) = &c.topology {
        print!(
            ", topology {}s/{}d/{}c/{}t",
            opt(t.sockets),
            opt(t.dies),
            opt(t.cores),
            opt(t.threads)
        );
    }
    println!();
    let kib = |v: Option<u64>| v.map_or_else(|| "-".into(), |k| human_bytes(k * 1024));
    println!(
        "   memory: max {}, current {}{}",
        kib(x.memory.max_kib),
        kib(x.memory.current_kib),
        if x.memory.hugepages {
            ", hugepages"
        } else {
            ""
        }
    );
    for k in &x.disks {
        let size = k.size.map_or_else(
            || "size unknown".to_string(),
            |s| {
                let b = |v: Option<u64>| v.map_or_else(|| "-".into(), human_bytes);
                format!(
                    "capacity {}, allocation {}, physical {}",
                    b(s.capacity),
                    b(s.allocation),
                    b(s.physical)
                )
            },
        );
        println!(
            "   disk {} {} bus {} {} format {}{}{} ({size})",
            k.target.as_deref().unwrap_or("-"),
            k.device,
            k.bus.as_deref().unwrap_or("-"),
            k.source.describe(),
            k.format.as_deref().unwrap_or("-"),
            if k.readonly { " ro" } else { "" },
            if k.shareable { " shared" } else { "" },
        );
    }
    for n in &x.interfaces {
        println!(
            "   nic {} {} mac {} model {} dev {}",
            n.kind,
            n.source.as_deref().unwrap_or("-"),
            n.mac.as_deref().unwrap_or("-"),
            n.model.as_deref().unwrap_or("-"),
            n.target_dev.as_deref().unwrap_or("-")
        );
    }
    for gr in &x.graphics {
        println!(
            "   graphics {} listen {} port {} tls {}{}",
            gr.kind,
            gr.listen.describe(),
            opt(gr.port),
            opt(gr.tls_port),
            if gr.autoport { " autoport" } else { "" }
        );
    }
    println!("   other devices: {}", x.other_devices.len());
    for o in &x.other_devices {
        println!("     {} {}", o.kind, o.summary);
    }
}

fn print_inventory(inv: &HostInventory) {
    println!(
        "library {}, hypervisor {}, {} domains",
        inv.library.as_deref().unwrap_or("?"),
        inv.hypervisor.as_deref().unwrap_or("?"),
        inv.domains.len()
    );
    for w in &inv.warnings {
        println!("warning: {w}");
    }
    for d in &inv.domains {
        print_domain(d);
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "vi_ssh=info,vi_backend=info".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    let mut args = std::env::args().skip(1);
    let mut cfg = HostConfig::new("cli", "");
    let mut use_agent = true;
    let mut known_hosts: Option<PathBuf> = None;
    let mut xml_of: Option<String> = None;
    let mut parallel = vi_backend::libvirt::DEFAULT_PARALLEL;
    let mut save_dir: Option<PathBuf> = None;
    let mut host = None;
    while let Some(a) = args.next() {
        let mut val = || args.next().unwrap_or_else(|| usage());
        match a.as_str() {
            "--no-agent" => use_agent = false,
            "--port" => cfg.port = Some(val().parse().unwrap_or_else(|_| usage())),
            "--user" => cfg.user = Some(val()),
            "--identity" => cfg.identity_file = Some(PathBuf::from(val())),
            "--known-hosts" => known_hosts = Some(PathBuf::from(val())),
            "--uri" => cfg.libvirt_uri = val(),
            "--prefix" => cfg.virsh_prefix = Some(val()),
            "--xml" => xml_of = Some(val()),
            "--parallel" => parallel = val().parse().unwrap_or_else(|_| usage()),
            "--save" => save_dir = Some(PathBuf::from(val())),
            "-h" | "--help" => usage(),
            s if s.starts_with('-') || host.is_some() => usage(),
            _ => host = Some(a),
        }
    }
    cfg.host = host.unwrap_or_else(|| usage());
    cfg.name = cfg.host.clone();

    if let Some(d) = &save_dir
        && d.read_dir().is_ok_and(|mut r| r.next().is_some())
    {
        eprintln!("error: {} exists and is not empty", d.display());
        std::process::exit(2);
    }
    let backend = match LibvirtBackend::from_host(&cfg) {
        Ok(b) => b.with_parallel(parallel),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(2);
        }
    };

    let ssh = SshConfig::load_default().unwrap_or_default();
    let home = dirs::home_dir().unwrap_or_default();
    let user = vi_ssh::sshconfig::local_user().unwrap_or_else(|| "root".into());
    let resolved = resolve(&cfg, &ssh, &user, &home);
    eprintln!(
        "connecting to {}@{}:{}",
        resolved.user, resolved.hostname, resolved.port
    );
    let kh = known_hosts
        .or_else(ConnectOptions::default_known_hosts)
        .unwrap_or_else(|| PathBuf::from("known_hosts"));
    let mut opts = ConnectOptions::new(kh);
    opts.use_agent = use_agent;

    let session = match SshSession::connect(resolved, &opts, &Terminal).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error ({:?}): {e}", e.kind());
            std::process::exit(1);
        }
    };
    let started = std::time::Instant::now();
    let rec = Recorder {
        inner: &session,
        log: Mutex::new(HashMap::new()),
    };
    let result = backend.inventory(&rec).await;
    let elapsed = started.elapsed();
    let code = match result {
        Ok(inv) => {
            print_inventory(&inv);
            if let Some(dir) = &save_dir {
                match save(dir, &backend, &session, &rec, &inv).await {
                    Ok(n) => eprintln!(
                        "saved {n} files to {}.\nThey contain REAL names, addresses and paths: run scripts/anonymise_fixtures.py before committing.",
                        dir.display()
                    ),
                    Err(e) => eprintln!("error saving to {}: {e}", dir.display()),
                }
            }
            if let Some(name) = &xml_of {
                match inv.domains.iter().find(|d| &d.name == name) {
                    Some(Domain {
                        details: Some(x), ..
                    }) => println!("\n== XML of {name}\n{}", x.xml),
                    _ => eprintln!("no XML for domain {name:?}"),
                }
            }
            eprintln!(
                "done in {:.1}s ({parallel} domains at a time)",
                elapsed.as_secs_f64()
            );
            0
        }
        Err(e) => {
            eprintln!("error ({:?}): {e}", e.kind());
            if let Some(d) = e.detail() {
                eprintln!("detail: {d}");
            }
            1
        }
    };
    session.close().await;
    std::process::exit(code);
}
