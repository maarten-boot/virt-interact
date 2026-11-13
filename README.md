# virt-interact

A desktop GUI (Windows, Linux, macOS) that connects to remote hosts over SSH
(key-based auth) and builds a read-only inventory of their virtual machines
(libvirt/virsh first). Remote VNC/SPICE viewing over the same SSH connection
follows in v1.1.

Status: phase 1 done (config, contrast/theme, logging). SSH, inventory and the
host/VM views follow in phases 2–4. See `architecture/Spec.md` and
`architecture/Plan.md`; changes per release are in `Changes.md`.

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
commented example is in [`examples/config.example.toml`](examples/config.example.toml).
A minimal one:

```toml
[ui]
contrast = "AA"      # "AA" | "AAA" | "Max" | a number between 4.5 and 21

[[hosts]]
name = "lab"
host = "lab.example.lan"
```

Contrast can also be changed in the toolbar; the choice is saved to the config.
Logs go to `<local data dir>/virt-interact/logs/` (on Linux
`~/.local/share/virt-interact/logs/`); set `RUST_LOG` to override the level.

## Layout

| Path | Content |
|------|---------|
| `crates/vi-core` | config, paths/CLI, contrast/palette maths, logging, model types, cache |
| `crates/vi-ssh` | `~/.ssh/config` parsing, SSH sessions, auth, known_hosts, command runner, tunnels |
| `crates/vi-backend` | hypervisor backends (libvirt via virsh) |
| `crates/vi-gui` | the `virt-interact` binary: GUI and theme |
| `examples/` | example config (kept valid by a unit test) |
| `tests/fixtures` | anonymised virsh output for parser tests |
| `architecture/` | Spec, Plan, TestCases, conversation logs, crosschecks |
| `Changes.md` | changes between releases |
