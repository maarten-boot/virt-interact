# Plan: virt-interact

Status: draft v1, 2026-11-13. The SPEC references point to `Spec.md`.

## Technology choices
| Area | Choice | Why |
|------|--------|-----|
| Language | Rust stable, edition 2024 | One code base for Windows, Linux and macOS |
| GUI | `eframe`/`egui` 0.35 (0.36+ needs rustc 1.95; the local toolchain is 1.93; MSRV 1.92) | Simple, cross-platform, full control over colours (SPEC-001) |
| Async | `tokio` | Needed by `russh`; background work off the GUI thread |
| SSH | `russh` (+ its key/agent support) | Pure Rust; tunnels inside the same session (SPEC-018) |
| XML | `roxmltree` | Read-only parsing of `virsh dumpxml` output |
| Config | `serde` + `toml`, `directories` | SPEC-004 |
| Cache | `serde_json` | SPEC-014 |
| Logging | `tracing`, `tracing-subscriber`, `tracing-appender` | SPEC-016 |
| Errors | `thiserror` (library code), `anyhow` (application) | |

## Code layout (Cargo workspace)
```
crates/
  vi-core/     config, ssh-config parsing, contrast/theme maths, model types, cache
  vi-ssh/      russh session, auth, known_hosts, command runner, tunnels
  vi-backend/  HypervisorBackend trait + libvirt implementation (virsh + XML parsing)
  vi-gui/      eframe app (binary: virt-interact)
tests/fixtures/  captured virsh outputs (XML, list, domblkinfo) for parser tests
```

## Phases

### Phase 0: Skeleton
- [x] 0.1 Create the Cargo workspace with the crates above. `cargo build` works on Linux. (2026-11-13)
- [x] 0.2 Set up a CI matrix (GitHub Actions) on Windows, Linux and macOS: fmt, clippy, test, build, plus an MSRV job. (SPEC-002) Written; not yet run, because the repo is not pushed.
- [x] 0.3 Set up a Rust `.gitignore` and the `Changes.md` workflow.

### Phase 1: Config and theme (SPEC-001, 004, 005, 016)
- [ ] 1.1 Config model, load and save, platform paths, `--config` override, error with line number.
- [ ] 1.2 Contrast maths: WCAG relative luminance and ratio; palette generation for a given minimum ratio; presets; clamping.
- [ ] 1.3 egui theme built from the palette (white/black default).
- [ ] 1.4 Logging setup.

### Phase 2: SSH (SPEC-006, 007, 008)
- [ ] 2.1 `~/.ssh/config` subset parser and resolution of the effective host parameters.
- [ ] 2.2 known_hosts check (hashed and plain), plus adding a new key.
- [ ] 2.3 russh connect: agent (Unix socket; Windows named pipe and Pageant), key files, passphrase callback.
- [ ] 2.4 `CommandRunner` trait with a russh implementation and timeouts; a mock implementation for tests. (SPEC-017)
- [ ] 2.5 Spike early: test the Windows agent and Pageant support in russh. If it is missing, decide on a workaround.

### Phase 3: libvirt backend (SPEC-009, 010, 011, 013, 017)
- [ ] 3.1 Allowlisted virsh command builder with shell quoting. (SPEC-011)
- [ ] 3.2 Parse `virsh list --all` and `dominfo` into the domain list.
- [ ] 3.3 Parse `dumpxml` into the component model; add `domblkinfo` sizes.
- [ ] 3.4 Error classification (virsh missing, permission denied, and so on).
- [ ] 3.5 Collect fixtures from real hosts: running and stopped VMs, UEFI and BIOS, VNC and SPICE, socket listen.
  - Done (2026-11-13): 7 running (libvirt 12.0.0, VNC and SPICE, bridge) and 3 stopped (VNC, SPICE incl. `listen none`, NAT and bridge), anonymised.
  - Added: a second host with libvirt 10.0.0 (3 running VMs, including SPICE `listen none` on a running VM, NAT network).
  - Still missing: UEFI (no UEFI VM available at the moment), a socket listen on a running VM, a network/volume disk, hostdev, and a host with stopped VMs in `list --all`.

### Phase 4: GUI (SPEC-003, 012, 014, 015)
- [ ] 4.1 Main window: host/VM tree, detail panel, toolbar, filter.
- [ ] 4.2 Background refresh per host with status; refresh all runs in parallel.
- [ ] 4.3 Host add/edit/remove dialogs; dialogs for the host key prompt and the passphrase prompt.
- [ ] 4.4 Inventory cache: save, load and clear; "cached" marker.
- [ ] 4.5 Settings dialog with the contrast control.

### Phase 5: Release v1
- [ ] 5.1 Packaging: Windows (zip, later MSI), Linux (tar.gz, later AppImage/deb), macOS (.app; signing and notarization to be decided).
- [ ] 5.2 Crosscheck. Update `Changes.md` and tag `v1.0.0`.

### Phase 6: v1.1 remote viewing (SPEC-018)
- [ ] 6.1 Local listener and `direct-tcpip` / `direct-streamlocal` forwarding.
- [ ] 6.2 Viewer launch, configurable path, tunnel lifetime tied to the viewer process.
- [ ] 6.3 Check that `remote-viewer` is available on each platform and document how to install it.

### Later
- SPEC-019 embedded VNC viewer; other backends (SPEC-017); `ProxyJump` support.

## Risks
| Risk | Mitigation |
|------|-----------|
| russh agent support on Windows (named pipe / Pageant) | Spike in 2.5. Fallback: key files with a passphrase prompt. |
| virsh output and XML differ between libvirt versions | Fixtures from several versions; tolerant parser (unknown elements are ignored, not fatal). |
| `qemu:///system` permissions | Clear error message with a hint (SPEC-013); `virsh_prefix = "sudo -n"` option. |
| macOS signing and notarization | Postpone to 5.1; an unsigned build is fine for development. |
| No mature Rust SPICE client | v1.1 uses an external `remote-viewer`. |

## Done
- [x] Project setup: git repo, `architecture/` docs, `Changes.md`, `CLAUDE.md` with working rules (2026-11-13).
- [x] Scenario captured in Spec.md (2026-11-13).
