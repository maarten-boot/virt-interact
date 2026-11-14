# virt-interact

A desktop GUI (Windows, Linux, macOS) that connects to remote hosts over SSH
(key-based auth) and builds a read-only inventory of their virtual machines
(libvirt/virsh first). Remote VNC/SPICE viewing over the same SSH connection
follows in v1.1.

Status: phases 2–4 done (config, contrast/theme, logging, SSH layer,
libvirt inventory backend, GUI with host/VM tree, details, cache and host
dialogs). Open: manual checks on a real desktop, on Windows and macOS.
Packaging (phase 5) is next. See `architecture/Spec.md` and
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

**This machine** needs no login: if `virsh` is on your `PATH`, the tree
starts with "localhost (system)" (`qemu:///system`) and "localhost (session)"
(your own VMs, `qemu:///session`). They are read with the same read-only
virsh commands, run locally. Configure or hide them under `[local]`
(`enabled = "auto" | "always" | "never"`, `session = true | false`).

To refresh automatically at startup, set `refresh_on_start = true` under
`[ui]` (see `examples/config.example.toml`). In the tree, Up/Down/Home/End
move the selection and Enter refreshes the selected host.

Contrast is set under **Settings…**; the choice is saved to the config.
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
| `crates/vi-backend` | hypervisor backends (libvirt via virsh) |
| `crates/vi-gui` | the GUI as a library (app, background worker, SSH prompts, view logic, theme) plus the thin `virt-interact` binary |
| `examples/` | example config and ssh config (kept valid by unit tests) |
| `scripts/` | `anonymise_fixtures.py` for new virsh captures |
| `tests/fixtures` | anonymised virsh output for parser tests |
| `architecture/` | Spec, Plan, TestCases, conversation logs, crosschecks |
| `Changes.md` | changes between releases |
