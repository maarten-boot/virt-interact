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

### SPEC-002: Target platforms (v1)
- Runs on Windows 10+ (x86_64), Linux (x86_64), and macOS 12+ (x86_64 and aarch64). CI builds all of them; macOS x86_64 is cross-compiled on an aarch64 runner (build only, no tests).
- Written in Rust (stable toolchain) and built from one code base.
- The client needs no libvirt installation. It only needs network access to the hosts.

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
  - an unknown argument prints usage and exits with code 2.
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
| `virsh_prefix` | no | none | Command prefix, e.g. `"sudo -n"` |

- Hosts can be added, edited and removed in the GUI. Changes are saved to the config file.
- Host names must be unique and not empty; `host` must not be empty.
- The GUI and the file stay consistent: a host added in the file appears after a reload.

## 4. SSH

### SPEC-006: Key-based authentication only (v1)
- Authentication methods, in this order:
  1. ssh-agent: `SSH_AUTH_SOCK` on Linux/macOS; on Windows, the OpenSSH agent named pipe, with Pageant as a fallback.
  2. The configured `identity_file`.
  3. Default keys: `~/.ssh/id_ed25519`, `id_ecdsa`, `id_rsa`.
- Password authentication and keyboard-interactive authentication are never used.
- If a key file is encrypted, the GUI asks for its passphrase. The passphrase is kept in memory for the session only and is never written to disk or to logs.
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
- The server host key is checked against `~/.ssh/known_hosts`. Both hashed and plain entries are supported.
- Host patterns (`*`, `?`, `!`), `[host]:port` for non-22 ports, `@revoked` (the key is refused) are supported. `@cert-authority` lines are ignored: host certificates are not supported and are refused. Unreadable lines are skipped with a warning.
- If the host is unknown, the app shows the key type and SHA256 fingerprint (and, if present, which other key types are recorded for the host). The user can accept (the key is added to `known_hosts`) or reject (the connection is aborted). The question is asked outside any network timeout: the handshake stops at the unknown key, and after acceptance the app reconnects.
- New entries are written hashed (`|1|...`, like OpenSSH `HashKnownHosts yes`); the file is created with mode 0600 on Unix. If the file cannot be written, the key is accepted for this connection only, with a warning.
- If the key does not match the stored key, the connection is refused with a prominent warning. There is no override in the GUI.

## 5. Inventory

### SPEC-009: Machine list (v1)
- For each host, the app lists every domain (virtual machine) defined in the configured libvirt URI, running or not.
- For each domain it shows: name, UUID, state (running, paused, shut off, crashed, pmsuspended, other), persistent or transient, and autostart.

### SPEC-010: Machine components (v1)
For each domain, the app shows:
- **General:** OS type, architecture, machine type, firmware (BIOS or UEFI), guest OS (from the libosinfo metadata `os id`, if present; an unknown ID is shown as is), description and title if set.
- **CPU:** vCPU count (current/max), CPU mode or model, topology (sockets/dies/cores/threads).
- **Memory:** maximum and current memory, hugepages if configured.
- **Disks:** device type (disk, cdrom, floppy, lun), target dev, bus, source (file, block, network or volume), driver format, read-only/shareable flags, capacity/allocation/physical size (when the size can be read).
  - A cdrom or floppy without a `<source>` is shown as "empty".
  - Allocation and physical size can be larger than capacity (seen with preallocated images). Values are shown as reported.
  - Disk targets are not assumed to be contiguous (e.g. `vda`, `vdc`).
- **Network interfaces:** type, source (network or bridge), MAC address, model.
- **Graphics:** type (vnc, spice), listen type and address, port/tlsPort (live values for running domains).
- **Other devices:** controllers, video, sound, hostdev, filesystem, channels. Shown as a summary list.
- **Raw XML:** the full domain XML, read-only, with copy to clipboard.

For running domains the live XML is used. For stopped domains the persistent definition is used.

Parsing is tolerant (see `tests/fixtures/`):
- XML with either `'` or `"` attribute quoting is accepted.
- `dominfo` is read as generic `key: value` lines, and unknown keys are ignored. libvirt 12 adds `Autostart Once`; security model and label lines are also present.

### SPEC-011: Read-only guarantee (v1)
- The app runs only these remote commands, from a fixed allowlist: `virsh version`, `virsh list`, `virsh dominfo`, `virsh dumpxml`, `virsh domblkinfo`, `virsh domstate`, `virsh domdisplay`, plus the optional `virsh_prefix`.
- No other remote command can be built or sent. This is enforced in code, in one place, and covered by tests.
- Arguments (domain names, disk targets) are passed with safe shell quoting.

### SPEC-012: Refresh and concurrency (v1)
- The user can refresh the inventory of one host or of all hosts.
- Hosts are queried in parallel. A slow or failing host does not delay the others.
- Each host shows its status: idle, connecting, refreshing, ok (with a timestamp), or error.
- The connect timeout and the command timeout can be configured (defaults: 10 s and 30 s).

### SPEC-013: Error reporting (v1)
These errors are shown per host, in plain language, with the underlying detail available. The SSH layer groups them as connection, timeout, host key, authentication and other:
- DNS or connection failure, or a timeout;
- authentication failure (no usable key);
- host key mismatch;
- `virsh` not found;
- permission denied on the libvirt URI (with a hint about the `libvirt` group or `virsh_prefix`);
- XML parse error (with the domain name).

### SPEC-014: Inventory cache (v1)
- The last successful inventory of each host is saved as JSON in the platform cache directory.
- At startup, and when a host is unreachable, the cached inventory is shown, clearly marked as "cached" with its timestamp.
- The user can clear the cache.

## 6. GUI

### SPEC-015: Main window layout (v1)
- **Left:** a tree of hosts, with the VMs of each host under it. Each VM shows a state indicator. The indicator uses an icon or text as well as colour, so it does not rely on colour alone.
- **Right:** the details of the selected item. For a host: connection info and status. For a VM: the sections from SPEC-010.
- **Toolbar:** refresh selected, refresh all, add host, settings (including contrast, SPEC-001).
- **Filter:** by VM name (substring) and by state.

### SPEC-016: Logging (v1)
- Logs go to a file in the platform data directory (`<data_local_dir>/virt-interact/logs/virt-interact.log.YYYY-MM-DD`, daily rotation). The level is configurable (default `info`). It applies to virt-interact's own code; other libraries log at `warn`. The `RUST_LOG` environment variable overrides both.
- Warnings and errors are also written to stderr.
- An invalid level in the config is a config error (SPEC-004).
- Logs never contain passphrases or private key material.

## 7. Architecture

### SPEC-017: Backend abstraction (v1 design, later implementations)
- Inventory access goes through a backend trait (for example, `HypervisorBackend`). v1 has one implementation: `libvirt` (virsh over SSH).
- Remote command execution goes through a trait, so backends can be tested without a real host.
- Other backends (for example Proxmox or Hyper-V) can be added later without changing the GUI.

## 8. Remote viewing

### SPEC-018: Remote console through SSH tunnel (v1.1)
- For a running domain with VNC or SPICE graphics, the user can open a console.
- The app reads the live graphics listen address and port, or the unix socket, from the live XML.
  - `virsh domdisplay` is only a fallback. For VNC it reports a display number (`vnc://127.0.0.1:1` means port 5901); for SPICE it reports the real port (`spice://127.0.0.1:5907`).
  - If the graphics have `listen type="none"` (for example, SPICE with local GL), the console is marked "not reachable remotely" and the console action is disabled. In that case `virsh domdisplay` fails with `error: No graphical display found`; this is not treated as a host error.
- It opens a local listener on `127.0.0.1` on a random free port and forwards connections over the existing SSH session:
  - `direct-tcpip` for a TCP listen address;
  - `direct-streamlocal` for a unix socket listen.
- It launches an external viewer (default `remote-viewer`; the path is configurable) with `vnc://127.0.0.1:<port>` or `spice://127.0.0.1:<port>`.
- The tunnel closes when the viewer exits or the user disconnects.
- If the console has a password, the viewer asks for it. The app does not read or store console passwords.

### SPEC-019: Embedded VNC viewer (later)
- A VNC viewer built into the app, as an alternative to the external viewer.
