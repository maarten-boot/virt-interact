//! Manual test tool for the SSH layer (TC-036, TC-037, TC-038).
//!
//! ```text
//! cargo run -p vi-ssh --example ssh-run -- [--no-agent] [--port N] [--user U]
//!     [--identity PATH] [--known-hosts PATH] HOST [COMMAND...]
//! ```
//! HOST may be an `~/.ssh/config` alias. COMMAND defaults to `virsh version`.
//! Unknown host keys and key passphrases are asked on the terminal.
//! Set `RUST_LOG=vi_ssh=debug` to see each step.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use vi_core::config::HostConfig;
use vi_ssh::prompt::BoxFuture;
use vi_ssh::{ConnectOptions, HostKeyPrompt, Prompter, Secret, SshConfig, SshSession, resolve};

struct Terminal;

impl Prompter for Terminal {
    fn confirm_host_key<'a>(&'a self, p: &'a HostKeyPrompt) -> BoxFuture<'a, bool> {
        Box::pin(async move {
            eprintln!(
                "The authenticity of host '{}' (port {}) can't be established.\n{} key fingerprint is {}.",
                p.hostname, p.port, p.key_type, p.fingerprint
            );
            if !p.other_types.is_empty() {
                eprintln!(
                    "Note: known_hosts has keys of other types for this host: {}",
                    p.other_types.join(", ")
                );
            }
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
        "usage: ssh-run [--no-agent] [--port N] [--user U] [--identity PATH] [--known-hosts PATH] HOST [COMMAND...]"
    );
    std::process::exit(2)
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "vi_ssh=info".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    let mut args = std::env::args().skip(1);
    let mut cfg = HostConfig::new("cli", "");
    let mut use_agent = true;
    let mut known_hosts: Option<PathBuf> = None;
    let mut rest = Vec::new();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--no-agent" => use_agent = false,
            "--port" => cfg.port = args.next().and_then(|p| p.parse().ok()).or_else(|| usage()),
            "--user" => cfg.user = Some(args.next().unwrap_or_else(|| usage())),
            "--identity" => {
                cfg.identity_file = Some(PathBuf::from(args.next().unwrap_or_else(|| usage())))
            }
            "--known-hosts" => {
                known_hosts = Some(PathBuf::from(args.next().unwrap_or_else(|| usage())))
            }
            "-h" | "--help" => usage(),
            _ => {
                rest.push(a);
                rest.extend(args.by_ref());
            }
        }
    }
    if rest.is_empty() {
        usage();
    }
    cfg.host = rest.remove(0);
    cfg.name = cfg.host.clone();
    let command = if rest.is_empty() {
        "virsh version".to_string()
    } else {
        rest.join(" ")
    };

    let ssh = SshConfig::load_default().unwrap_or_default();
    let home = dirs::home_dir().unwrap_or_default();
    let user = vi_ssh::sshconfig::local_user().unwrap_or_else(|| "root".into());
    let resolved = resolve(&cfg, &ssh, &user, &home);
    eprintln!(
        "connecting to {}@{}:{} (keys: {:?})",
        resolved.user, resolved.hostname, resolved.port, resolved.identity_files
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
    match session.exec(&command).await {
        Ok(out) => {
            print!("{}", out.stdout_text());
            eprint!("{}", out.stderr_text());
            eprintln!("[exit status: {:?}]", out.exit_status);
            session.close().await;
            std::process::exit(out.exit_status.map_or(255, |c| c as i32));
        }
        Err(e) => {
            eprintln!("error ({:?}): {e}", e.kind());
            std::process::exit(1);
        }
    }
}
