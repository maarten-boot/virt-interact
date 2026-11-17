# Spec: virt-interact

Status: draft v1, 2026-11-13.

Items are numbered `SPEC-nnn` so that Plan.md and TestCases.md can refer to them. Each item is tagged with the release it belongs to: **v1** (read-only inventory), **v1.1** (remote viewing), or **later**.

## 1. Purpose
virt-interact is a desktop GUI application that connects to remote hosts over SSH with key-based authentication. It builds a read-only inventory of the virtual machines defined on each host: every machine, running or stopped, and the components of each one. A later release adds remote viewing (VNC/SPICE) of running machines, tunnelled through the same SSH connection.

## 2. Platform and technology

### SPEC-001: UI colour and contrast (v1)
- The default theme is a white background with black text.
- Every foreground/background pair reaches a WCAG contrast ratio of at least 4.5:1. This includes state colours (running, stopped, error), selection highlights, disabled text and links.
- The user can raise the minimum contrast:
  - interactively, from a control in the GUI, with immediate effect;
  - through the config file (SPEC-004), with `[ui] contrast = "AA" | "AAA" | "Max" | <number>`.
- Presets: AA = 4.5:1, AAA = 7:1, Max = 21:1 (pure black on white). A custom number sets the minimum ratio and must be at least 4.5. Lower values are clamped to 4.5, and values above 21 are clamped to 21; in both cases a warning is logged and shown.
- The palette is generated for the chosen minimum: backgrounds are lightened toward white and foregrounds darkened toward black until every pair reaches it. The toolbar shows the lowest ratio of all pairs in the current palette.
- A change made interactively is saved back to the config file.
- Disabled widgets are not shown by fading (that would lower the contrast). They use the palette's disabled text colour, which meets the minimum.
- Nothing else fades either: text at the edge of a scrolled area is drawn at full contrast (egui's scroll-edge fade is off). Dialogs do not dim the window behind them.

### SPEC-002: Target platforms (v1)
- Runs on Windows 10+ (x86_64), Linux (x86_64), and macOS 12+ (x86_64 and aarch64). CI builds all of them; macOS x86_64 is cross-compiled on an aarch64 runner (build only, no tests).
- Written in Rust (stable toolchain) and built from one code base.
- The client needs no libvirt installation. It only needs network access to the hosts.
- **Distribution (v1):** one archive per platform, built by CI from a version tag, with the README, `Changes.md`, the example configs and a SHA-256 checksum file:
  - Linux: `virt-interact-<ver>-linux-x86_64.tar.gz`, built on Ubuntu 22.04 (glibc 2.35 or newer needed);
  - Windows: `virt-interact-<ver>-windows-x86_64.zip`; the release binary is a GUI program (no console window);
  - macOS: `virt-interact-<ver>-macos-universal.zip` with `virt-interact.app` (arm64 + x86_64, macOS 12+), ad-hoc signed but not notarized: the first start needs "Open Anyway" in System Settings → Privacy & Security.
  - Installers (MSI, AppImage/deb) and notarization are later work.
- **Licence:** virt-interact is MIT licensed. Every archive holds `LICENSE` and `THIRD_PARTY_LICENSES.html` with the licence texts of all bundled crates (all platforms). Only licences listed in `about.toml` are accepted; packaging fails on any other, so a new licence is reviewed before it is shipped. For a crate under "A OR B" the first accepted licence applies; no GPL-only code is included.
- **Icon:** the app has its own icon, the project logo (a blue "V" on a green rounded square; master `packaging/icon/logo.png`): the window icon (all platforms), embedded in the Windows `.exe`, in the macOS app bundle, and with a `.desktop` file in the Linux archive (the window's app id is `virt-interact`).

### SPEC-003: GUI toolkit (v1)
- The GUI uses `egui`/`eframe`.
- SSH and inventory work runs in the background and never blocks the GUI (see SPEC-012).

## 3. Configuration

### SPEC-004: Config file (v1)
- The config file is TOML, in the platform config directory:
  - Linux: `$XDG_CONFIG_HOME/virt-interact/config.toml` (default `~/.config/virt-interact/config.toml`)
  - Windows: `%APPDATA%\virt-interact\config\config.toml`
  - macOS: `~/Library/Application Support/virt-interact/config.toml`
- Command-line options:
  - `--config <path>` (or `--config=<path>`) overrides the location;
  - `-h`/`--help` prints usage; `-V`/`--version` prints the version;
  - an unknown argument prints usage and exits with code 2;
  - Windows release builds have no console: this output, and start-up errors such as an unknown config directory, are shown in a small window with an OK button (same exit codes).
- `[ui]` keys: `contrast` (SPEC-001) and `refresh_on_start` (bool, default `false`; SPEC-012).
- `[local]` keys: `enabled`, `libvirt_uri`, `virsh_prefix`, `session` (SPEC-020).
- If the file is missing, the app starts with defaults and creates the file the first time it saves.
- An invalid file gives a clear error message with the line number. Unknown keys count as invalid, so typing mistakes are caught. The app does not overwrite an invalid file: it starts with defaults and does not save changes until the file is fixed.

### SPEC-005: Host definitions (v1)
Each host has these fields:

| Field | Required | Default | Meaning |
|-------|----------|---------|---------|
| `name` | yes | — | Display name. Must be unique. |
| `host` | yes | — | Hostname, IP address, or `~/.ssh/config` alias |
| `port` | no | from ssh config, else 22 | SSH port, 1–65535 |
| `user` | no | from ssh config, else the local user name | Remote user |
| `identity_file` | no | from ssh config / agent / default keys | Private key path. A leading `~` is expanded to the local home directory (`~user` is not). |
| `backend` | no | `"libvirt"` | Virtualisation backend (SPEC-017) |
| `libvirt_uri` | no | `"qemu:///system"` | URI passed to `virsh -c` |
| `virsh_prefix` | no | none | Command prefix, e.g. `"sudo -n"`; plain characters only (SPEC-011) |

- Hosts can be added, edited and removed in the GUI. Changes are saved to the config file.
- Host names must be unique and not empty; `host` must not be empty. Names starting with `@` are reserved for the local entries (SPEC-020).
- The GUI and the file stay consistent: a host added in the file appears after a reload.

### SPEC-020: Local host without login (v1)
- The local machine's libvirt can be shown without SSH: no login, no host key, no passphrase.
- Two entries, shown first in the tree, labelled from their URI ("localhost (system)" for `qemu:///system`, "localhost (session)" for `qemu:///session`, otherwise "localhost (<uri>)"):
  - "localhost (system)": `[local] libvirt_uri`, default `qemu:///system`, with the optional `[local] virsh_prefix`;
  - "localhost (session)": `qemu:///session`, the user's own VMs (as used by GNOME Boxes, and by libvirt on macOS). Shown when `[local] session = true` (the default) and the system URI is not already the session URI. Reading it may start the user's session libvirt daemon, as any `virsh -c qemu:///session` does.
- `[local] enabled`:
  - `"auto"` (default): shown when `virsh` is found on the search path: `PATH`, then `/opt/homebrew/bin`, `/usr/local/bin`, `/usr/bin`, `/bin` (an app started from the macOS Finder or Dock gets a minimal `PATH` without Homebrew). The same search path is used to run `virsh`, so detection and execution agree. Settings shows where `virsh` was found;
  - `"always"`: always shown (an error explains a missing `virsh`);
  - `"never"`: hidden.
- The same allowlisted, read-only command lines as for remote hosts (SPEC-011: `LC_ALL=C`, `virsh -r`, quoting) run through the local POSIX shell (`sh -c`), with the `[ssh] command_timeout_secs` limit. Platforms without a POSIX shell (Windows) do not show local entries in `"auto"` mode.
- Local entries are not stored in `[[hosts]]` and cannot be edited or removed in the GUI. They are configured only through `[local]`. Refresh, status, cache, filter and keyboard work as for other hosts.
- v1.1 (SPEC-018): the console of a local VM is reached directly, without SSH: the same local listener connects straight to the TCP port or unix socket.

## 4. SSH

### SPEC-006: Key-based authentication only (v1)
- Authentication methods, in this order:
  1. ssh-agent: `SSH_AUTH_SOCK` on Linux/macOS; on Windows, the OpenSSH agent named pipe, with Pageant as a fallback.
  2. Configured key files: the app's `identity_file` first, then the ssh config `IdentityFile`s.
  3. Only if no key file is configured (app or ssh config): the default keys `~/.ssh/id_ed25519`, `id_ecdsa`, `id_rsa` (those that exist).
- Agent keys that match a configured key file (recognised by its `.pub`) are offered first, so an agent with many keys does not use up the server's `MaxAuthTries` before the right key.
- If the server closes the connection during authentication (e.g. too many attempts), the error still lists the keys tried, with a note that the server hung up.
- Password authentication and keyboard-interactive authentication are never used.
- If a key file is encrypted, the GUI asks for its passphrase. The SSH layer uses it for one connection attempt and then wipes it; the GUI does not keep it. Instead the SSH connection of each host stays open while the app runs, so a later refresh does not ask again; the connection is dropped when the host is edited or removed, when the config is reloaded, or when it breaks (the app then reconnects once). It is never written to disk or to logs.
- SSH is implemented with `russh` (crypto backend `ring`, so no C toolchain/cmake is needed on Windows). There is no dependency on an external `ssh` binary.
- A key that the agent already offered is not tried again from its file, so its passphrase is not asked for needlessly (the `.pub` next to the key file is used to recognise it).
- RSA keys use `rsa-sha2-512`/`-256` signatures when the server supports them.
- Each authentication failure reports which keys were tried and why each failed (not found, wrong passphrase, rejected).

### SPEC-007: `~/.ssh/config` support (v1)
- For a host's `host` value, the app applies `Host` pattern matching (`*`, `?`, `!negation`, case-insensitive) and these keywords: `HostName`, `User`, `Port`, `IdentityFile`. Options before the first `Host` apply to all hosts. As in OpenSSH, the first value found wins; `IdentityFile` values accumulate.
- `IdentityFile` supports `~` and the tokens `%d` (home), `%u` (local user), `%r` (remote user), `%h` (host name), `%n` (original host), `%%`.
- If no `IdentityFile` comes from the app config or ssh config, the default keys are tried (SPEC-006).
- Values in the config file (SPEC-005) take precedence over `~/.ssh/config`.
- `ProxyJump`, `ProxyCommand`, `Include` and `Match` blocks are not supported in v1. If they apply to a host, a warning is shown (the options inside `Match` blocks are ignored). Other keywords are ignored silently.

### SPEC-008: Host key verification (v1)
- The server host key is checked against `~/.ssh/known_hosts` and then the system-wide files (read only): `/etc/ssh/ssh_known_hosts` and `/etc/ssh/ssh_known_hosts2` on Linux/macOS, `%PROGRAMDATA%\ssh\ssh_known_hosts` on Windows. Both hashed and plain entries are supported.
- As in OpenSSH: `@revoked` in any file refuses the key; otherwise the first file that knows the host (match or mismatch), the user's file first, decides. New keys are only written to the user's file.
- The name looked up and written is the resolved host name (`HostName` from the ssh config, else the configured `host`), as `[name]:port` for ports other than 22 (like OpenSSH without `HostKeyAlias`).
- Host patterns (`*`, `?`, `!`), `[host]:port` for non-22 ports, `@revoked` (the key is refused) are supported. `@cert-authority` lines are ignored: host certificates are not supported and are refused. Unreadable lines are skipped with a warning.
- If the host is unknown, the app shows the key type and SHA256 fingerprint (and, if present, which other key types are recorded for the host). The user can accept (the key is added to `known_hosts`) or reject (the connection is aborted). The question is asked outside any network timeout: the handshake stops at the unknown key, and after acceptance the app reconnects.
- New entries are written hashed (`|1|...`, like OpenSSH `HashKnownHosts yes`); the file is created with mode 0600 on Unix. If the file cannot be written, the key is accepted for this connection only, with a warning. If the reconnect after acceptance presents a different key, the connection is refused ("host key changed between attempts").
- If the key does not match the stored key, the connection is refused with a prominent warning. There is no override in the GUI.

## 5. Inventory

### SPEC-009: Machine list (v1)
- For each host, the app lists every domain (virtual machine) defined in the configured libvirt URI, running or not.
- For each domain it shows: name, UUID, state (running, paused, shut off, crashed, pmsuspended, other), persistent or transient, and autostart.

### SPEC-010: Machine components (v1)
For each domain, the app shows:
- **General:** UUID (from `dominfo`, else from the XML), hypervisor type (e.g. kvm), OS type, architecture, machine type, firmware (UEFI if `<os firmware="efi">`, a `pflash` loader or an OVMF/EDK2/AAVMF loader path; otherwise BIOS), guest OS (from the libosinfo metadata `os id`, if present, shown without scheme and vendor host: `http://ubuntu.com/ubuntu/24.04` → `ubuntu/24.04`; an ID in another form is shown as is), description and title if set.
- **CPU:** vCPU count (current/max), CPU mode or model, topology (sockets/dies/cores/threads).
- **Memory:** maximum and current memory, hugepages if configured.
- **Disks:** device type (disk, cdrom, floppy, lun), target dev, bus, source (file, block, dir, network or volume; other types are shown by name), driver format, read-only/shareable flags, capacity/allocation/physical size (when the size can be read).
  - A cdrom or floppy without a `<source>` is shown as "empty".
  - Allocation and physical size can be larger than capacity (seen with preallocated images). Values are shown as reported.
  - Disk targets are not assumed to be contiguous (e.g. `vda`, `vdc`).
- **Network interfaces:** type, source (network or bridge), MAC address, model.
- **Graphics:** type (vnc, spice), listen type and address, port/tlsPort (live values for running domains).
- **Other devices:** all other device elements (controllers, video, sound, hostdev, filesystem, channels, input, serial, rng, …), each with a one-line summary (type and model; PCI/USB address or vendor:product for hostdev; source → target for filesystem; name for channels).
- **Raw XML:** the full domain XML, read-only, with copy to clipboard.

For running domains the live XML is used. For stopped domains the persistent definition is used.

Parsing is tolerant (see `tests/fixtures/`):
- XML with either `'` or `"` attribute quoting is accepted.
- `dominfo` is read as generic `key: value` lines, and unknown keys are ignored. libvirt 12 adds `Autostart Once`; security model and label lines are also present.

### SPEC-011: Read-only guarantee (v1)
- The app runs only these remote commands, from a fixed allowlist: `virsh version`, `virsh list`, `virsh dominfo`, `virsh dumpxml`, `virsh domblkinfo`, `virsh domstate`, `virsh domdisplay`, plus the optional `virsh_prefix`.
- No other remote command can be built or sent. This is enforced in code, in one place, and covered by tests.
- Every command has the form `LC_ALL=C [virsh_prefix ]virsh -r -c '<uri>' <subcommand> <args>`:
  - `LC_ALL=C` keeps virsh output in English, so it can be parsed whatever the remote locale is.
  - `-r` opens a **read-only** libvirt connection, so libvirt itself refuses any change: a second guard behind the allowlist.
- Arguments (URI, domain names, disk targets) are passed with POSIX single-quote shell quoting. The remote login shell must be POSIX-compatible (sh, bash, dash, zsh, ksh).
- `virsh_prefix` may contain only letters, digits, spaces and `_ - . / = : @ +`. It is checked when the config is loaded.

### SPEC-012: Refresh and concurrency (v1)
- The user can refresh the inventory of one host or of all hosts.
- Hosts are queried in parallel. A slow or failing host does not delay the others.
- Within one host, up to 4 domains are queried at the same time, each on its own channel of the one SSH connection. Results keep the order of `virsh list`.
- Each host shows its status: idle, connecting, refreshing, ok (with a timestamp), or error.
- By default there is no automatic refresh at startup (it could ask for passphrases or host keys before the user does anything); the cached inventory is shown instead (SPEC-014). With `[ui] refresh_on_start = true` all hosts are refreshed once when the app starts.
- A refresh of a host that is already refreshing is ignored.
- The connect timeout and the command timeout can be configured in `[ssh]` (`connect_timeout_secs`, `command_timeout_secs`; defaults 10 s and 30 s).
- The connect timeout applies separately to the TCP connect, the SSH handshake and each authentication attempt. Time spent waiting for the user (host key question, passphrase) is not counted. The command timeout applies to each remote command; after a timeout the connection stays usable.

### SPEC-013: Error reporting (v1)
These errors are shown per host, in plain language, with the underlying detail available. The SSH layer groups them as connection, timeout, host key, authentication and other:
- DNS or connection failure, or a timeout;
- authentication failure (no usable key);
- host key mismatch;
- `virsh` not found;
- permission denied on the libvirt URI (with a hint about the `libvirt` group or `virsh_prefix`);
- XML parse error (with the domain name).

Scope of an error:
- SSH connection errors, `virsh` not found, permission denied and "cannot connect to the hypervisor" (from any command) stop the refresh of that host. A timeout of `virsh version` or `virsh list` also stops it.
- A domain that disappears between `list` and a later command is left out, with a warning.
- An XML parse error or another failing command for one domain is shown on that domain (with the detail); the other domains are still listed.
- A command timeout for one domain (`dominfo`, `dumpxml`) is shown on that domain; the others are still listed.
- A failing `domblkinfo` only makes that disk's size "unknown"; if it timed out, a warning names the disk (e.g. a hanging network disk).

### SPEC-014: Inventory cache (v1)
- The last successful inventory of each host is saved as JSON in the platform cache directory (`<cache_dir>/virt-interact/inventory-<name>-<hash>.json`, one file per host). A damaged file, or one from another format version, is ignored.
- After a failed refresh the last inventory stays visible, marked "cached".
- At startup, and when a host is unreachable, the cached inventory is shown, clearly marked as "cached" with its timestamp.
- The user can clear the cache.
- The cache file of an entry that goes away (a host removed in the GUI or from the file, or a local entry that is no longer shown) is deleted.

## 6. GUI

### SPEC-015: Main window layout (v1)
- **Left:** a tree of hosts, with the VMs of each host under it. Each VM shows a state indicator. The indicator uses an icon or text as well as colour, so it does not rely on colour alone.
- **Right:** the details of the selected item. For a host: connection info and status. For a VM: the sections from SPEC-010.
- **Toolbar:** refresh selected, refresh all, add host, settings, reload config, and the filter.
- **Settings window:** contrast (SPEC-001), the config and cache locations, the SSH timeouts (read-only; edited in the config file), clear cache.
- **Host details:** connection settings, status with the error message and its detail, libvirt and hypervisor version, number of VMs, whether the inventory is live or cached and when it was taken, warnings; Refresh, Edit and Remove (with confirmation).
- **VM details:** identical other devices are grouped with a count; an empty cdrom shows "empty drive".
- **Filter:** by VM name (substring) and by state.
- **Keyboard:** when no text field has focus and no dialog is open, Up/Down move the selection through the tree (only through expanded hosts, respecting the filter), Home/End jump to the first/last row, and Enter refreshes the selected host.
- Disabled buttons show their text in the palette's disabled colour (SPEC-001).
- **No movement from feedback:** hover, press and selection feedback changes only colours (border and background), never the size or position of anything. The tree panel's width changes only when the user drags the separator: rows wider than the panel scroll horizontally instead of widening it, so a longer status text or VM name moves nothing either.
- The tree width, the window size and position, and which hosts are expanded are kept between runs (egui's own state file `app.ron`: Linux `$XDG_DATA_HOME/virt-interact/`, default `~/.local/share/virt-interact/`; macOS `~/Library/Application Support/virt-interact/`; Windows `%APPDATA%\virt-interact\data\`). It holds no host data; deleting it only resets the layout.

### SPEC-016: Logging (v1)
- Logs go to a file in the platform data directory (`<data_local_dir>/virt-interact/logs/virt-interact.log.YYYY-MM-DD`, daily rotation). The level is configurable (default `info`). It applies to virt-interact's own code; other libraries log at `warn`. The `RUST_LOG` environment variable overrides both.
- Warnings and errors are also written to stderr.
- An invalid level in the config is a config error (SPEC-004).
- Logs never contain passphrases or private key material.

## 7. Architecture

### SPEC-017: Backend abstraction (v1 design, later implementations)
- Inventory access goes through a backend trait (for example, `HypervisorBackend`). v1 has one implementation: `libvirt` (virsh over SSH, or locally for SPEC-020).
- Command execution goes through a trait (`CommandRunner`): an SSH session, a local shell runner (SPEC-020), or a mock for tests.
- Other backends (for example Proxmox or Hyper-V) can be added later without changing the GUI.

## 8. Remote viewing

### SPEC-018: Remote console through SSH tunnel (v1.1)
- For a running (or paused) domain with VNC or SPICE graphics, the user can open a console: the graphics table in the VM details has an "Open console" button per device. When it cannot be opened, the button is disabled and the reason is shown next to it (VM not running, not reachable remotely, …).
- On "Open console", the app reads the domain's live XML again (`virsh dumpxml`, allowlisted, read-only), so the port is current even when the shown inventory is cached. It uses the kept connection of the host, or connects (prompts as for a refresh); the host's inventory status does not change.
  - `virsh domdisplay` is not used: the live XML of a running domain always has the port (also with autoport). For VNC `domdisplay` would report a display number (`vnc://127.0.0.1:1` means port 5901), for SPICE the real port.
  - If the graphics have `listen type="none"` (for example, SPICE with local GL), the console is marked "not reachable remotely" and the console action is disabled. (`virsh domdisplay` would fail with `error: No graphical display found`.)
- The forward target follows from the live graphics (`Graphics::console_target`):
  - a TCP listen address as is; a wildcard (`0.0.0.0`, `::`) or the default listen as `127.0.0.1` on the host; a network listen by its resolved address;
  - a unix socket by its path;
  - refused, with the reason shown: kinds other than VNC and SPICE, no port (domain not running), TLS-only (no plain port; not supported yet), an unknown socket path or network address.
- It opens a local listener on `127.0.0.1` on a random free port and forwards connections over the existing SSH session (`vi_ssh::Tunnel`):
  - `direct-tcpip` for a TCP listen address;
  - `direct-streamlocal` for a unix socket listen.
  - Several connections at once are forwarded (SPICE opens one per channel). Closing the tunnel stops the listener and ends every connection. A connection the host refuses is closed locally and its error kept for the GUI.
  - The port is reachable only from this machine, but by any local process while the tunnel is open; a console password still applies.
- It launches an external viewer with `vnc://127.0.0.1:<port>` or `spice://127.0.0.1:<port>` as its only argument.
  - The viewer is `[console] viewer` in the config (a path, or a program name), else `remote-viewer` (virt-viewer). A name is searched on `PATH`, then `/opt/homebrew/bin`, `/usr/local/bin`, `/usr/bin`, `/bin`, and on Windows in `Program Files\VirtViewer*\bin` (newest version first; `.exe` added).
  - Not found: an error with the install hint for the platform (Linux: the `virt-viewer` package; macOS: `brew install virt-viewer`; Windows: the virt-viewer MSI) and the config key.
- The tunnel lives as long as the viewer:
  - When the viewer exits, the tunnel closes. A viewer that fails (non-zero exit) is reported with its last stderr line and any tunnel error.
  - "Close console" (the same button while open) stops the viewer and closes the tunnel. On Linux/macOS the viewer runs in its own process group; closing sends it SIGTERM, then SIGKILL after 2 s, so the helpers of a wrapper script stop too. The same happens when the app is closed, or the host is edited or removed, or the config is reloaded.
  - Errors appear as a message naming the VM and host. One console per graphics device at a time.
- If the console has a password, the viewer asks for it. The app does not read or store console passwords.

### SPEC-019: Embedded VNC viewer (later)
- A VNC viewer built into the app, as an alternative to the external viewer.
