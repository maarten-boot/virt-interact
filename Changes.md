# Changes

Changes between releases. Newest on top.

## Unreleased

Nothing yet.

## 1.1.0 (2026-11-19)

Adds **VNC and SPICE consoles**: open a running VM's display from the app,
through the host's SSH connection.

**Consoles**
- "Open console" in the Graphics table of a running VM opens its VNC or
  SPICE console in `remote-viewer` (virt-viewer), through the host's SSH
  connection; nothing is opened on the host's network. The app listens on
  a random port on `127.0.0.1` only.
- The tunnel lives as long as the viewer: it closes when the viewer exits,
  or with "Close console". Editing or removing the host, reloading the
  config and quitting the app close its consoles too.
- Consoles of VMs on this machine are reached directly, without SSH.
- When a console cannot be opened, the button is disabled with the reason
  (VM not running, not reachable remotely, TLS-only, …); errors name the
  VM and host.
- A console password, if set, is asked by the viewer; the app never sees it.
- The viewer is found on your `PATH`, in Homebrew's and MacPorts' folders,
  or on Windows in `Program Files\VirtViewer*`; set `[console] viewer` for
  another one. Install it with your Linux package manager (`virt-viewer`),
  MacPorts on macOS (`sudo port install virt-viewer`), or on Windows with
  `winget install RedHat.VirtViewer` or the MSI (see the README).

**Other changes**
- `virsh` for "This machine" is now also found in `/opt/local/bin` (MacPorts).
- New app icon: a blue "V" on a green rounded square (window, Windows
  program, macOS app, Linux menu entry).

**Downloads**
- As 1.0.0: Linux x86_64 (tar.gz, glibc 2.35 or newer), Windows x86_64
  (zip), macOS 12+ for Apple Silicon and Intel (app in a zip); each with a
  `.sha256` file, `LICENSE` (MIT) and the licences of all bundled libraries.

**Known limitations**
- Consoles were tried with a real host and `remote-viewer` on Linux and
  Windows; on macOS only the viewer install and search were checked (in
  CI), not a real console.
- TLS-only consoles (no plain port) are not supported yet; the viewer is
  external (no built-in viewer).
- libvirt (`virsh`) is the only backend; VMware ESXi/vCenter is planned
  for 1.2.
- As in 1.0.0: the macOS app is not notarized, the Windows program is not
  signed, and there are no installers yet.

## 1.0.0 (2026-11-16)

First release: a desktop app for Windows, Linux and macOS that shows a
**read-only inventory of libvirt virtual machines** on your hosts, over SSH.

**Inventory**
- Every VM per host, running or stopped, with its state as text and colour.
- Per VM: general info (UUID, firmware BIOS/UEFI, guest OS, architecture,
  autostart, persistent), CPU and
  memory, disks with their size, network interfaces, graphics, and the other
  devices grouped; the raw XML can be viewed and copied.
- Only read-only commands run on a host (`virsh -r`, a fixed allowlist);
  nothing is ever changed.
- Up to 4 VMs per host are read in parallel. A VM that fails or times out
  is marked; the rest of the host still shows.
- The last inventory of each host is cached and shown, marked "cached", when
  the app starts or a host is unreachable.

**Hosts and SSH**
- Key-based SSH only: ssh-agent (also the Windows OpenSSH agent and
  Pageant) and key files, with a passphrase dialog. Passwords are never used.
- `~/.ssh/config` (`Host` patterns, `HostName`, `Port`, `User`,
  `IdentityFile`) and `known_hosts` are respected; unknown host keys are asked in a dialog, and a
  changed key is refused.
- Hosts are added, edited and removed in the app or in the TOML config;
  `virsh_prefix` (e.g. `sudo -n`) for users outside the `libvirt` group.
- This machine is shown without login when `virsh` is installed:
  "localhost (system)" and "localhost (session)".

**Interface**
- White background, black text; every colour pair meets WCAG AA (4.5:1),
  with AAA, Max (pure black/white) and a custom minimum in Settings or the
  config file.
- Filter by name and state; keyboard navigation in the tree.
- Hovering and pressing change only colours; nothing moves. The tree width
  and window size are remembered.

**Downloads**
- Linux x86_64 (tar.gz, glibc 2.35 or newer, with a `.desktop` file),
  Windows x86_64 (zip), macOS 12+ for Apple Silicon and Intel (app in a zip).
- Each archive has a `.sha256` file and holds `LICENSE` (MIT) and the
  licences of all bundled libraries.

**Known limitations**
- Read-only inventory only. VNC/SPICE consoles over SSH follow in 1.1.
- libvirt (`virsh`) is the only backend.
- The macOS app is not notarized: allow it once in System Settings →
  Privacy & Security → "Open Anyway". The Windows program is not signed, so
  SmartScreen may warn on first start.
- No installers yet (MSI, AppImage/deb).

## Development before 1.0.0

The steps that led to 1.0.0, in order (details in `architecture/`):

- Project setup: git repo, `architecture/` docs (Spec, Plan, TestCases, conversation log), working rules in `CLAUDE.md`.
- Spec, Plan and TestCases for v1 (read-only inventory) and v1.1 (remote viewing).
- Phase 0: Cargo workspace (`vi-core`, `vi-ssh`, `vi-backend`, `vi-gui`), empty white/black GUI window, CI for Windows/Linux/macOS, README.
- Anonymised virsh fixtures (libvirt 12.0.0 and 10.0.0 live captures plus 3 stopped domain XMLs); Spec/TestCases updated with the parsing findings.
- Phase 1: TOML config with hosts, validation and line-numbered errors (an invalid file is never overwritten); `--config`, `--help`, `--version`; WCAG contrast maths and palette generation (AA/AAA/Max/custom); egui theme with a contrast control in the toolbar that saves to the config; file logging.
- Crosscheck 01 fixes: disabled widgets no longer fade below the contrast minimum (egui colour-pair test added); infinite/NaN contrast values clamped; port 0 rejected; `~` expanded in `identity_file`; `examples/config.example.toml`; README usage section; CI also builds macOS x86_64.
- Phase 2: SSH layer (`vi-ssh`): `~/.ssh/config` subset, own known_hosts check/learn (hashed, patterns, `@revoked`), key-only auth (agent incl. Windows pipe/Pageant, key files, passphrase prompt), command runner with timeouts and a mock; integration tests against an in-process SSH server; `ssh-run` example for manual tests.
- Crosscheck 02 fixes: auth errors keep the list of keys tried (also when the server hangs up); clear error when the host key changes between attempts; config timeouts used by the SSH layer; agent keys matching configured key files are tried first; system-wide known_hosts files are read; `examples/ssh_config.example`.
- Phase 3: libvirt backend (`vi-backend`): allowlisted read-only virsh commands (`LC_ALL=C`, `virsh -r`, POSIX quoting, validated `virsh_prefix`); parsers for `version`, `list --all`, `dominfo`, `domblkinfo` and the domain XML (general, CPU, memory, disks, NICs, graphics, other devices); error classification with hints; per-domain errors do not stop the host; inventory model in `vi-core`; `inventory` example for manual tests.
- MIT licence (`LICENSE`, in every archive) and the licences of all bundled libraries (`THIRD_PARTY_LICENSES.html`).
- App icon: window, Windows `.exe`, macOS app, and a `.desktop` file for Linux.
- Windows: `--help`, `--version` and start-up errors are shown in a small window (the release build has no console).
- Phase 5.1 packaging: release archives for Linux (tar.gz), Windows (zip, no console window) and macOS (universal `.app`, ad-hoc signed), each with a SHA-256 file; CI builds them on every push; a version tag creates a draft GitHub release. Release builds use thin LTO and are stripped.
- Crosscheck 2026-11-14-02 fixes: hovering or pressing no longer moves anything (the separator used to shift by a pixel): feedback is colour only and the tree scrolls instead of widening; the tree width and window size are kept between runs; `virsh` from Homebrew is found when the app is started from the macOS Finder or Dock; local entries are labelled from their URI; cache files of entries that disappear are deleted.
- Local host without login (SPEC-020): "localhost (system)" and "localhost (session)" are shown when `virsh` is on `PATH`, and read with the same read-only commands through the local shell; `[local]` config section.
- Crosscheck 2026-11-14-01 fixes: a host edited/removed during a refresh no longer keeps the old SSH connection or shows the old result; disabled buttons are visibly disabled (disabled colour, still ≥ the contrast minimum); a skipped passphrase is wiped; status ages update while idle; cache files of hosts removed from the file are deleted on reload; UUID taken from the XML when `dominfo` has none; `[ui] refresh_on_start`; keyboard navigation in the tree.
- Phase 4: GUI with host/VM tree (state shown as text and colour), filter by name and state, host details (status, errors with detail, live/cached inventory) and VM details (general, CPU/memory, disks, NICs, graphics, grouped other devices, raw XML with copy); background refresh with one kept SSH connection per host; dialogs for adding/editing/removing hosts, unknown host keys and key passphrases; inventory cache shown at startup and after a failed refresh; settings window (contrast, paths, clear cache); reload config. No fading anywhere (scroll edges included), so every text stays at the contrast minimum.
- Crosscheck 03 fixes: a command timeout for one VM or disk no longer fails the whole host; up to 4 VMs per host are queried in parallel; exit status 126 no longer gives the libvirt-group hint; network listen keeps the live address; `inventory --save` and `scripts/anonymise_fixtures.py` for capturing new fixtures.
- CI fixes: `clippy::question_mark` (Rust 1.100); `.gitattributes` keeps LF line endings on all platforms; README example test tolerates CRLF; CI tests use `--no-fail-fast`.
