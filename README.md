# virt-interact

A desktop GUI (Windows, Linux, macOS) that connects to remote hosts over SSH
(key-based auth) and builds a read-only inventory of their virtual machines
(libvirt/virsh first). Remote VNC/SPICE viewing over the same SSH connection
follows in v1.1.

Status: phase 0 (skeleton). See `architecture/Spec.md` and `architecture/Plan.md`.

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

## Layout

| Path | Content |
|------|---------|
| `crates/vi-core` | config, theme/contrast, model types, cache |
| `crates/vi-ssh` | SSH sessions, auth, known_hosts, command runner, tunnels |
| `crates/vi-backend` | hypervisor backends (libvirt via virsh) |
| `crates/vi-gui` | the `virt-interact` binary |
| `tests/fixtures` | captured virsh output for parser tests |
| `architecture/` | Spec, Plan, TestCases, conversation logs, crosschecks |
| `Changes.md` | changes between releases |
