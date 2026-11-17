# virt-interact

A desktop GUI (Windows, Linux, macOS) that connects to remote hosts over SSH
(key-based auth) and builds a read-only inventory of their virtual machines
(libvirt/virsh first), and opens their VNC/SPICE consoles over the same SSH
connection (v1.1, in development).

Status: version 1.0.0 is released (GitHub releases page): the read-only
inventory (see `Changes.md` for what it does and its known limitations).
In development for 1.1: VNC/SPICE consoles over SSH (see "Unreleased" in
`Changes.md`). See `architecture/Spec.md` and `architecture/Plan.md`.

## Install

Download the archive for your platform from the GitHub releases page and
compare it with its `.sha256` file:

- **Linux** (`…-linux-x86_64.tar.gz`): unpack and run `./virt-interact`.
  Needs glibc 2.35 or newer (Ubuntu 22.04, Debian 12, Fedora 36 or later)
  and a desktop with X11 or Wayland and OpenGL. For a menu entry with the
  icon, copy `virt-interact` to `~/.local/bin/`, `virt-interact.desktop` to
  `~/.local/share/applications/` and `virt-interact.png` to
  `~/.local/share/icons/hicolor/512x512/apps/`.
- **Windows** (`…-windows-x86_64.zip`): unpack and run `virt-interact.exe`.
  It is a GUI program without a console: `--help`, `--version` and
  start-up errors are shown in a small window.
- **macOS** (`…-macos-universal.zip`, Apple Silicon and Intel, macOS 12+):
  unpack and move `virt-interact.app` to Applications. The app is not
  notarized: the first start is blocked; open it once via System Settings →
  Privacy & Security → "Open Anyway" (or run
  `xattr -d com.apple.quarantine /Applications/virt-interact.app`).

Each archive also holds `LICENSE`, `THIRD_PARTY_LICENSES.html` (the
licences of the libraries built into the program), `Changes.md` and the
example configs.

## Build

Requires Rust 1.92 or newer.

```sh
cargo build --release
./target/release/virt-interact
```

Linux build dependencies (Debian/Ubuntu):

```sh
sudo apt-get install libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev libgl1-mesa-dev
```

## Usage

```text
Usage: virt-interact [OPTIONS]

Options:
  --config <PATH>   Use this config file instead of the default location
  -h, --help        Show this help
  -V, --version     Show the version
```

The config file is TOML. Default location:

| Platform | Path |
|----------|------|
| Linux | `~/.config/virt-interact/config.toml` (`$XDG_CONFIG_HOME` respected) |
| Windows | `%APPDATA%\virt-interact\config\config.toml` |
| macOS | `~/Library/Application Support/virt-interact/config.toml` |

A missing file means defaults; it is created on the first save. A complete,
commented example is in [`examples/config.example.toml`](examples/config.example.toml);
the supported part of `~/.ssh/config` is shown in
[`examples/ssh_config.example`](examples/ssh_config.example).
A minimal one:

```toml
[ui]
contrast = "AA"      # "AA" | "AAA" | "Max" | a number between 4.5 and 21

[[hosts]]
name = "lab"
host = "lab.example.lan"
```

In the GUI, add hosts with **Add host…** (or in the config file, then
**Reload config**), and press **Refresh all**. The app does not connect on its
own at startup; it shows the last inventory from its cache, marked "cached".
Unknown host keys and key passphrases are asked in a dialog. The SSH
connection of each host stays open while the app runs, so later refreshes
do not ask again.

**This machine** needs no login: if `virsh` is found (on your `PATH`, or
in `/opt/homebrew/bin` or `/usr/local/bin`, as on macOS), the tree
starts with "localhost (system)" (`qemu:///system`) and "localhost (session)"
(your own VMs, `qemu:///session`). They are read with the same read-only
virsh commands, run locally. Configure or hide them under `[local]`
(`enabled = "auto" | "always" | "never"`, `session = true | false`).

To refresh automatically at startup, set `refresh_on_start = true` under
`[ui]` (see `examples/config.example.toml`). In the tree, Up/Down/Home/End
move the selection and Enter refreshes the selected host.

**Consoles** (1.1, in development): select a running VM; its Graphics table
has **Open console** for each VNC or SPICE display. The console opens in
`remote-viewer` (from virt-viewer) through the host's SSH connection: the
app listens on a random port on `127.0.0.1` and forwards it, so the
display does not need to be reachable on the host's network. Closing the
viewer closes the tunnel; **Close console** stops both. Install the viewer
with your package manager (`virt-viewer`), `brew install virt-viewer` on
macOS, or the virt-viewer MSI on Windows; another viewer can be set with
`[console] viewer = "/path/to/viewer"`. A console password, if set, is
asked by the viewer.

Contrast is set under **Settings…**; the choice is saved to the config.
Hovering or pressing changes only colours; nothing moves. Drag the line
between the tree and the details to set the tree's width; it is kept, with
the window size, between runs (egui's `app.ron`, see SPEC-015).
Logs go to `<local data dir>/virt-interact/logs/` (on Linux
`~/.local/share/virt-interact/logs/`); set `RUST_LOG` to override the level.

## Testing SSH against a real host

The SSH layer can be tried without the GUI:

```sh
cargo run -p vi-ssh --example ssh-run -- [--no-agent] [--port N] [--user U] \
    [--identity PATH] [--known-hosts PATH] HOST [COMMAND...]
```

`HOST` may be an `~/.ssh/config` alias; `COMMAND` defaults to `virsh version`.
Use `RUST_LOG=vi_ssh=debug` to see each step. `tests/fixtures/ssh/` holds
**test-only** keys for the automated tests; never use them elsewhere.

## Testing the inventory against a real host

The libvirt backend can be tried without the GUI. It connects like `ssh-run`,
runs only the allowlisted read-only virsh commands and prints the inventory:

```sh
cargo run -p vi-backend --example inventory -- [--no-agent] [--port N] [--user U] \
    [--identity PATH] [--known-hosts PATH] [--uri URI] [--prefix "sudo -n"] \
    [--xml DOMAIN] [--parallel N] [--save DIR] HOST
```

Every command runs as `LC_ALL=C virsh -r -c '<uri>' ...` (read-only connection);
the remote user needs read access to libvirt, e.g. membership of the `libvirt`
group. Use `RUST_LOG=vi_backend=debug` to see each command. `--parallel N`
sets how many VMs are queried at once (default 4). `--save DIR` writes the
raw virsh output in the fixture layout. It contains real names and
addresses: anonymise it with `scripts/anonymise_fixtures.py` before
committing (see `tests/fixtures/README.md`).

## Layout

| Path | Content |
|------|---------|
| `crates/vi-core` | config, paths/CLI, contrast/palette maths, logging, model types, cache |
| `crates/vi-ssh` | `~/.ssh/config` parsing, SSH sessions, auth, known_hosts, command runner, tunnels |
| `crates/vi-backend` | hypervisor backends (libvirt via virsh), local runner for this machine |
| `crates/vi-gui` | the GUI as a library (app, background worker, SSH prompts, view logic, theme) plus the thin `virt-interact` binary |
| `examples/` | example config and ssh config (kept valid by unit tests) |
| `scripts/` | `package.sh` (release archive per platform), `release_notes.sh`, `make_icon.py` (app icons from the logo), `anonymise_fixtures.py` (new virsh captures) |
| `packaging/` | logo and app icons (PNG, ICO, ICNS), macOS `Info.plist`, Linux `.desktop` file |
| `about.toml`, `about.hbs` | accepted third-party licences and the notices template (`cargo-about`, needed by `package.sh`) |
| `tests/fixtures` | anonymised virsh output for parser tests |
| `architecture/` | Spec, Plan, TestCases, conversation logs, crosschecks |
| `Changes.md` | changes between releases |

## Licence

MIT; see [`LICENSE`](LICENSE). The release archives list the licences of
all bundled libraries in `THIRD_PARTY_LICENSES.html`, made with
[`cargo-about`](https://github.com/EmbarkStudios/cargo-about)
(`cargo install --locked --features cli cargo-about`); `about.toml` lists
the accepted licences, and packaging fails when a new dependency brings
another one.
