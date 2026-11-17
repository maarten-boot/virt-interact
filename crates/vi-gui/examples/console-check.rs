//! Check the console viewer on this machine (Plan 6.3, TC-106).
//!
//!   console-check [--viewer PATH] --find
//!       Find the viewer like the app does, print its path and run
//!       `<viewer> --version`. Fails if it is not found.
//!   console-check [--viewer PATH] [--seconds N] [vnc|spice] HOST:PORT
//!       Open a tunnel (directly, no SSH; the same code as the app) to a
//!       console at HOST:PORT on this machine and start the viewer with the
//!       tunnel URL. Passes if after N seconds (default 8) the viewer still
//!       runs with a connection through the tunnel; then it is stopped.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use vi_backend::local::LocalRunner;
use vi_core::model::ConsoleTarget;
use vi_gui::console::{find_viewer, viewer_url};
use vi_ssh::Tunnel;

const USAGE: &str = "usage: console-check [--viewer PATH] --find\n       console-check [--viewer PATH] [--seconds N] [vnc|spice] HOST:PORT";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("FAIL: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut viewer: Option<String> = None;
    let mut seconds = 8u64;
    let mut find = false;
    let mut rest = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--viewer" => viewer = Some(args.next().ok_or(USAGE)?),
            "--seconds" => {
                seconds = args.next().and_then(|s| s.parse().ok()).ok_or(USAGE)?;
            }
            "--find" => find = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            _ => rest.push(a),
        }
    }
    let path = find_viewer(viewer.as_deref())?;
    println!("viewer: {}", path.display());
    if find {
        return version(&path);
    }
    let (kind, addr) = match rest.as_slice() {
        [addr] => ("vnc".to_string(), addr.clone()),
        [kind, addr] if kind == "vnc" || kind == "spice" => (kind.clone(), addr.clone()),
        _ => return Err(USAGE.into()),
    };
    let (host, port) = addr.rsplit_once(':').ok_or(USAGE)?;
    let port: u16 = port.parse().map_err(|_| USAGE)?;
    let target = ConsoleTarget::Tcp {
        host: host.to_string(),
        port,
    };
    let rt = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
    rt.block_on(console(path, &kind, target, Duration::from_secs(seconds)))
}

fn version(path: &PathBuf) -> Result<(), String> {
    let out = std::process::Command::new(path)
        .arg("--version")
        .output()
        .map_err(|e| format!("cannot run {}: {e}", path.display()))?;
    println!("--version: {}", out.status);
    for line in String::from_utf8_lossy(&out.stdout)
        .lines()
        .chain(String::from_utf8_lossy(&out.stderr).lines())
        .filter(|l| !l.trim().is_empty())
        .take(5)
    {
        println!("  {line}");
    }
    Ok(())
}

async fn console(
    path: PathBuf,
    kind: &str,
    target: ConsoleTarget,
    wait: Duration,
) -> Result<(), String> {
    let runner = Arc::new(LocalRunner::new(Duration::from_secs(10)));
    let tunnel = Tunnel::start(runner, target.clone())
        .await
        .map_err(|e| e.to_string())?;
    let url = viewer_url(kind, tunnel.local_addr());
    println!("console: {target} via {url}");
    let mut child = tokio::process::Command::new(&path)
        .arg(&url)
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("cannot start {}: {e}", path.display()))?;
    let exited = tokio::time::timeout(wait, child.wait()).await;
    let (total, active, err) = (tunnel.total(), tunnel.active(), tunnel.last_error());
    println!("tunnel: {total} connection(s), {active} open, last error: {err:?}");
    match exited {
        Ok(status) => Err(format!(
            "the viewer exited within {} s: {:?}",
            wait.as_secs(),
            status.map(|s| s.to_string())
        )),
        Err(_) => {
            let _ = child.kill().await;
            tunnel.shutdown().await;
            if active == 0 {
                return Err("the viewer runs but has no connection through the tunnel".into());
            }
            println!("PASS: the viewer is connected through the tunnel");
            Ok(())
        }
    }
}
