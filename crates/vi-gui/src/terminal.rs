//! "Start SSH" (SPEC-025): the system OpenSSH client in a terminal window.
//!
//! The ssh command line is built from the app's host entry (only the values
//! set there; the user's own `~/.ssh/config` does the rest). The terminal is
//! `[terminal] command` when set, else detected per platform.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use vi_backend::command::shell_quote;
use vi_core::config::HostConfig;

/// Linux terminals in the order tried, each with the arguments that come
/// before the command to run.
pub const LINUX_TERMINALS: [(&str, &[&str]); 7] = [
    ("x-terminal-emulator", &["-e"]),
    ("gnome-terminal", &["--"]),
    ("konsole", &["-e"]),
    ("xfce4-terminal", &["-x"]),
    ("kitty", &[]),
    ("alacritty", &["-e"]),
    ("xterm", &["-e"]),
];

/// The arguments of `ssh` for a host: only the values set in the entry,
/// `--` before the host.
pub fn ssh_args(host: &HostConfig) -> Vec<String> {
    let mut v = vec!["ssh".to_string()];
    if let Some(p) = host.port {
        v.extend(["-p".into(), p.to_string()]);
    }
    if let Some(u) = &host.user {
        v.extend(["-l".into(), u.clone()]);
    }
    if let Some(i) = host.identity_path() {
        v.extend(["-i".into(), i.to_string_lossy().into_owned()]);
    }
    v.extend(["--".into(), host.host.clone()]);
    v
}

/// The command line as text for a POSIX shell: words that need it quoted.
pub fn command_line(args: &[String]) -> String {
    args.iter()
        .map(|a| quote_if_needed(a))
        .collect::<Vec<_>>()
        .join(" ")
}

fn quote_if_needed(a: &str) -> String {
    let plain = !a.is_empty()
        && a.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:@=+,%~".contains(c));
    if plain { a.to_string() } else { shell_quote(a) }
}

/// Split the Settings text into words: spaces separate, `'…'` and `"…"`
/// group (a backslash escapes `"` and `\` inside double quotes and any
/// character outside quotes).
pub fn split_words(text: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            '\'' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(c) => cur.push(c),
                        None => return Err("unclosed ' quote".into()),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(c @ ('"' | '\\')) => cur.push(c),
                            Some(c) => {
                                cur.push('\\');
                                cur.push(c);
                            }
                            None => return Err("unclosed \" quote".into()),
                        },
                        Some(c) => cur.push(c),
                        None => return Err("unclosed \" quote".into()),
                    }
                }
            }
            '\\' => {
                in_word = true;
                match chars.next() {
                    Some(c) => cur.push(c),
                    None => return Err("a backslash at the end".into()),
                }
            }
            c => {
                in_word = true;
                cur.push(c);
            }
        }
    }
    if in_word {
        words.push(cur);
    }
    Ok(words)
}

/// How the terminal is started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Terminal {
    /// `[terminal] command`: program and arguments, ssh appended.
    Configured(Vec<String>),
    /// A terminal found on the search path (Linux), with its arguments.
    Found { program: PathBuf, args: Vec<String> },
    /// macOS Terminal.app through `osascript`.
    MacTerminal,
    /// Windows Terminal (`wt.exe`).
    WindowsTerminal(PathBuf),
    /// Windows without Windows Terminal: `ssh.exe` in a new console window.
    WindowsConsole,
}

/// The platform a terminal is chosen for (a parameter, so tests can check
/// every platform anywhere).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Linux,
    MacOs,
    Windows,
}

impl Platform {
    pub fn current() -> Platform {
        if cfg!(windows) {
            Platform::Windows
        } else if cfg!(target_os = "macos") {
            Platform::MacOs
        } else {
            Platform::Linux
        }
    }
}

/// The terminal for this machine: configured, else detected.
pub fn choose(configured: Option<&[String]>) -> Result<Terminal, String> {
    choose_in(
        configured,
        Platform::current(),
        &vi_backend::local::search_path(),
    )
}

/// [`choose`] for a given platform and search path.
pub fn choose_in(
    configured: Option<&[String]>,
    platform: Platform,
    path: &OsStr,
) -> Result<Terminal, String> {
    if let Some(c) = configured.filter(|c| !c.is_empty()) {
        return Ok(Terminal::Configured(c.to_vec()));
    }
    match platform {
        Platform::MacOs => Ok(Terminal::MacTerminal),
        Platform::Windows => {
            Ok(find_in("wt.exe", path).map_or(Terminal::WindowsConsole, Terminal::WindowsTerminal))
        }
        Platform::Linux => LINUX_TERMINALS
            .iter()
            .find_map(|(name, args)| {
                find_in(name, path).map(|program| Terminal::Found {
                    program,
                    args: args.iter().map(|a| a.to_string()).collect(),
                })
            })
            .ok_or_else(|| {
                "no terminal found (tried x-terminal-emulator, gnome-terminal, konsole, \
                 xfce4-terminal, kitty, alacritty, xterm); set [terminal] command, \
                 e.g. [\"xterm\", \"-e\"], or Terminal in Settings"
                    .to_string()
            }),
    }
}

fn find_in(name: &str, path: &OsStr) -> Option<PathBuf> {
    std::env::split_paths(path)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

impl Terminal {
    /// For Settings: which terminal is used.
    pub fn describe(&self) -> String {
        match self {
            Terminal::Configured(c) => format!("{} (configured)", command_line(c)),
            Terminal::Found { program, .. } => format!("{} (detected)", program.display()),
            Terminal::MacTerminal => "Terminal.app (detected)".into(),
            Terminal::WindowsTerminal(p) => format!("Windows Terminal, {} (detected)", p.display()),
            Terminal::WindowsConsole => "a new console window (no Windows Terminal found)".into(),
        }
    }

    /// The process that opens a terminal running `ssh`.
    pub fn command(&self, ssh: &[String]) -> Command {
        let (program, args): (OsString, Vec<String>) = match self {
            Terminal::Configured(c) => (
                c[0].clone().into(),
                c[1..].iter().chain(ssh).cloned().collect(),
            ),
            Terminal::Found { program, args } => (
                program.clone().into(),
                args.iter().chain(ssh).cloned().collect(),
            ),
            Terminal::MacTerminal => (
                "osascript".into(),
                vec![
                    "-e".into(),
                    "tell application \"Terminal\"".into(),
                    "-e".into(),
                    "activate".into(),
                    "-e".into(),
                    format!("do script \"{}\"", applescript_escape(&command_line(ssh))),
                    "-e".into(),
                    "end tell".into(),
                ],
            ),
            Terminal::WindowsTerminal(wt) => (wt.clone().into(), ssh.to_vec()),
            Terminal::WindowsConsole => (
                Path::new(&ssh[0]).with_extension("exe").into_os_string(),
                ssh[1..].to_vec(),
            ),
        };
        let mut cmd = Command::new(program);
        cmd.args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // Not in the app's process group, so it outlives the app and a
        // signal to the app does not reach it.
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        #[cfg(windows)]
        if matches!(self, Terminal::WindowsConsole) {
            use std::os::windows::process::CommandExt;
            const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
            cmd.creation_flags(CREATE_NEW_CONSOLE);
        }
        cmd
    }
}

/// Escape text for an AppleScript string literal.
pub fn applescript_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Open a terminal with `ssh` to `host`. The terminal runs on its own;
/// a thread waits for the launcher so no zombie is left.
pub fn start_ssh(host: &HostConfig, configured: Option<&[String]>) -> Result<String, String> {
    let terminal = choose(configured)?;
    let ssh = ssh_args(host);
    let mut child = terminal
        .command(&ssh)
        .spawn()
        .map_err(|e| format!("cannot start the terminal ({}): {e}", terminal.describe()))?;
    tracing::info!(host = %host.name, terminal = %terminal.describe(), "ssh terminal started");
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(command_line(&ssh))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host() -> HostConfig {
        HostConfig::new("lab", "lab.example.lan")
    }

    fn args(cmd: &Command) -> (String, Vec<String>) {
        (
            cmd.get_program().to_string_lossy().into_owned(),
            cmd.get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect(),
        )
    }

    // TC-142: only the values set, `--` before the host; quoting.
    #[test]
    fn ssh_command() {
        assert_eq!(ssh_args(&host()), ["ssh", "--", "lab.example.lan"]);
        let mut h = host();
        h.port = Some(2222);
        h.user = Some("ops".into());
        h.identity_file = Some(PathBuf::from("/keys/my key"));
        assert_eq!(
            ssh_args(&h),
            [
                "ssh",
                "-p",
                "2222",
                "-l",
                "ops",
                "-i",
                "/keys/my key",
                "--",
                "lab.example.lan"
            ]
        );
        assert_eq!(
            command_line(&ssh_args(&h)),
            "ssh -p 2222 -l ops -i '/keys/my key' -- lab.example.lan"
        );
        assert_eq!(command_line(&["a'b".into(), String::new()]), "'a'\\''b' ''");
    }

    // TC-143: Settings text ↔ words.
    #[test]
    fn words() {
        assert_eq!(split_words("kitty -e").unwrap(), ["kitty", "-e"]);
        assert_eq!(split_words("  ").unwrap(), Vec::<String>::new());
        assert_eq!(
            split_words(r#"'/My Apps/term' --title "a \"b\" \\c" x\ y ''"#).unwrap(),
            ["/My Apps/term", "--title", r#"a "b" \c"#, "x y", ""]
        );
        assert!(split_words("'open").is_err());
        assert!(split_words("\"open").is_err());
        assert!(split_words("end\\").is_err());
        let w: Vec<String> = ["/My Apps/term", "-e", "it's"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(split_words(&command_line(&w)).unwrap(), w);
    }

    // TC-143: configured first; Linux detection order; macOS; Windows.
    #[test]
    fn terminal_choice() {
        let d = tempfile::tempdir().unwrap();
        let (a, b) = (d.path().join("a"), d.path().join("b"));
        for (dir, name) in [(&a, "xterm"), (&b, "konsole"), (&b, "wt.exe")] {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(dir.join(name), b"").unwrap();
        }
        let path = std::env::join_paths([&a, &b]).unwrap();
        let conf = vec!["kitty".to_string(), "-e".to_string()];
        assert_eq!(
            choose_in(Some(&conf), Platform::Linux, &path).unwrap(),
            Terminal::Configured(conf.clone())
        );
        // An empty configured list means automatic.
        assert_eq!(
            choose_in(Some(&[]), Platform::Linux, &path).unwrap(),
            Terminal::Found {
                program: b.join("konsole"),
                args: vec!["-e".into()]
            },
            "konsole comes before xterm in the list"
        );
        let empty = std::env::join_paths([d.path().join("none")]).unwrap();
        let e = choose_in(None, Platform::Linux, &empty).unwrap_err();
        assert!(e.contains("[terminal] command"), "{e}");
        assert_eq!(
            choose_in(None, Platform::MacOs, &empty).unwrap(),
            Terminal::MacTerminal
        );
        assert_eq!(
            choose_in(None, Platform::Windows, &path).unwrap(),
            Terminal::WindowsTerminal(b.join("wt.exe"))
        );
        assert_eq!(
            choose_in(None, Platform::Windows, &empty).unwrap(),
            Terminal::WindowsConsole
        );
    }

    // TC-143: the process for each kind of terminal.
    #[test]
    fn terminal_commands() {
        let ssh = ssh_args(&host());
        let (p, a) = args(&Terminal::Configured(vec!["kitty".into(), "-e".into()]).command(&ssh));
        assert_eq!(
            (p.as_str(), a),
            (
                "kitty",
                vec![
                    "-e".to_string(),
                    "ssh".into(),
                    "--".into(),
                    "lab.example.lan".into()
                ]
            )
        );
        let (p, a) = args(
            &Terminal::Found {
                program: "/usr/bin/gnome-terminal".into(),
                args: vec!["--".into()],
            }
            .command(&ssh),
        );
        assert_eq!(p, "/usr/bin/gnome-terminal");
        assert_eq!(a, ["--", "ssh", "--", "lab.example.lan"]);
        let (p, a) = args(&Terminal::WindowsTerminal("wt.exe".into()).command(&ssh));
        assert_eq!((p.as_str(), a.len()), ("wt.exe", 3));
        let (p, a) = args(&Terminal::WindowsConsole.command(&ssh));
        assert_eq!(p, "ssh.exe");
        assert_eq!(a, ["--", "lab.example.lan"]);
        // macOS: quoted for the shell, then for AppleScript.
        let mut h = host();
        h.identity_file = Some(PathBuf::from(r#"/k/a "b"\c"#));
        let (p, a) = args(&Terminal::MacTerminal.command(&ssh_args(&h)));
        assert_eq!(p, "osascript");
        assert_eq!(
            a[5],
            r#"do script "ssh -i '/k/a \"b\"\\c' -- lab.example.lan""#
        );
        assert_eq!(applescript_escape(r#"a"b\"#), r#"a\"b\\"#);
    }
}
