# Plan: virt-interact

Status: draft v1, 2026-11-13. The SPEC references point to `Spec.md`.

## Technology choices
| Area | Choice | Why |
|------|--------|-----|
| Language | Rust stable, edition 2024 | One code base for Windows, Linux and macOS |
| GUI | `eframe`/`egui` 0.35 (0.36+ needs rustc 1.95; the local toolchain is 1.93; MSRV 1.92) | Simple, cross-platform, full control over colours (SPEC-001) |
| Async | `tokio` | Needed by `russh`; background work off the GUI thread |
| SSH | `russh` 0.65 (+ its key/agent support), crypto backend `ring` | Pure Rust; tunnels inside the same session (SPEC-018); `ring` avoids cmake/NASM on Windows |
| XML | `roxmltree` | Read-only parsing of `virsh dumpxml` output |
| Config | `serde` + `toml`, `dirs` | SPEC-004 |
| Cache | `serde_json` (added in 4.4) | SPEC-014 |
| Logging | `tracing`, `tracing-subscriber`, `tracing-appender` | SPEC-016 |
| Errors | `thiserror` | |
| Parallel domain queries | `futures-util` (`buffered`) | SPEC-012 |

## Code layout (Cargo workspace)
```
crates/
  vi-core/     config, paths/CLI, contrast/palette maths, logging, model types, cache
  vi-ssh/      ~/.ssh/config parsing, russh session, auth, known_hosts, command runner, tunnels
  vi-backend/  HypervisorBackend trait + libvirt implementation (virsh + XML parsing)
  vi-gui/      library (app, worker, prompts, view, theme) + thin binary `virt-interact`; tests drive the app headless
tests/fixtures/  captured virsh outputs (XML, list, domblkinfo) for parser tests
```

## Phases

### Phase 0: Skeleton
- [x] 0.1 Create the Cargo workspace with the crates above. `cargo build` works on Linux. (2026-11-13)
- [x] 0.2 Set up a CI matrix (GitHub Actions) on Windows, Linux and macOS: fmt, clippy, test, build, plus an MSRV job. (SPEC-002) Written; not yet run, because the repo is not pushed.
- [x] 0.3 Set up a Rust `.gitignore` and the `Changes.md` workflow.
- [x] 0.4 First CI runs (2026-11-13): new clippy lint in Rust 1.100 fixed; Windows CRLF checkout broke TC-028, so `.gitattributes` forces LF and CI tests run with `--no-fail-fast`.

### Phase 1: Config and theme (SPEC-001, 004, 005, 016)
- [x] 1.1 Config model, load and save, platform paths, `--config` override, error with line number. (`vi-core/src/config.rs`, `paths.rs`)
- [x] 1.2 Contrast maths: WCAG relative luminance and ratio; palette generation for a given minimum ratio; presets; clamping. (`vi-core/src/contrast.rs`)
- [x] 1.3 egui theme built from the palette (white/black default), plus a contrast control in the toolbar that saves to the config. (`vi-gui/src/theme.rs`, `main.rs`)
- [x] 1.4 Logging setup. (`vi-core/src/logging.rs`)
- [x] 1.5 Crosscheck 2026-11-13-01 fixes: no fading of disabled widgets and an egui colour-pair test (TC-007), clamping for infinite/NaN contrast, port 0 rejected, `~` expansion in `identity_file`, example config with a test, macOS x86_64 CI build.

### Phase 2: SSH (SPEC-006, 007, 008)
- [x] 2.1 `~/.ssh/config` subset parser and resolution of the effective host parameters. (`vi-ssh/src/sshconfig.rs`)
- [x] 2.2 known_hosts check (hashed and plain), plus adding a new key. Own implementation: russh's version only splits on single spaces and has no markers or patterns. (`vi-ssh/src/known_hosts.rs`)
- [x] 2.3 russh connect: agent (Unix socket; Windows named pipe and Pageant), key files, passphrase callback. (`vi-ssh/src/session.rs`, `prompt.rs`, `secret.rs`, `error.rs`)
- [x] 2.4 `CommandRunner` trait with a russh implementation and timeouts; a mock implementation for tests. (SPEC-017) (`vi-ssh/src/runner.rs`)
- [~] 2.5 Spike: Windows agent and Pageant support in russh.
  - Source review: russh 0.65 has `AgentClient::connect_named_pipe` (OpenSSH agent, `\\.\pipe\openssh-ssh-agent`) and `connect_pageant`; both are used (pipe first). No workaround needed.
  - CI compiles the Windows code and all tests pass on windows-latest (run 43509629083). Still open: a manual run on Windows (TC-038) with `cargo run -p vi-ssh --example ssh-run`.
- [x] 2.6 Integration tests against an in-process russh server (`vi-ssh/tests/`), and the `ssh-run` example for manual tests on real hosts.
- [x] 2.8 Crosscheck 2026-11-13-02 fixes: auth errors keep the tried list (server hang-up), clear error for a host key change between attempts, `ConnectOptions::from_settings`, `AuthState`, agent keys matching configured files first, system-wide known_hosts.
- [ ] 2.7 Manual check against a real OpenSSH server and ssh-agent (TC-036/037 on a real host). Starting a local `sshd` from the assistant session was not permitted; to be run by the user.

### Phase 3: libvirt backend (SPEC-009, 010, 011, 013, 017)
- [x] 3.1 Allowlisted virsh command builder with shell quoting, `LC_ALL=C` and a read-only connection (`-r`). (SPEC-011; `vi-backend/src/command.rs`)
- [x] 3.2 Parse `virsh version`, `list --all` and `dominfo` into the domain list. (`parse.rs`; model in `vi-core/src/model.rs`)
- [x] 3.3 Parse `dumpxml` into the component model (roxmltree); add `domblkinfo` sizes. (`domxml.rs`, `libvirt.rs`)
- [x] 3.4 Error classification (virsh missing, permission denied, sudo password, hypervisor down) and error scope per host/domain. (`error.rs`)
  - Commands of one host run one after another over the one SSH session. Running them in parallel channels is possible later if a refresh is too slow.
- [ ] 3.5 Collect fixtures from real hosts: running and stopped VMs, UEFI and BIOS, VNC and SPICE, socket listen.
  - Done (2026-11-13): 7 running (libvirt 12.0.0, VNC and SPICE, bridge) and 3 stopped (VNC, SPICE incl. `listen none`, NAT and bridge), anonymised.
  - Added: a second host with libvirt 10.0.0 (3 running VMs, including SPICE `listen none` on a running VM, NAT network).
  - New captures: `inventory --save DIR <host>` (into the git-ignored `tmp/`), then `scripts/anonymise_fixtures.py DIR tests/fixtures/<set> --leak <hostname>`; review by eye before committing.
  - Still missing: UEFI (no UEFI VM available at the moment), a socket listen on a running VM, a network/volume disk, hostdev, and a host with stopped VMs in `list --all`. Until then these cases are tested with small synthetic XML in `domxml.rs`.
- [x] 3.7 Crosscheck 03 fixes: per-domain timeouts, exit status 126, network listen address, up to 4 domains in parallel per host, `inventory --save` and `scripts/anonymise_fixtures.py` for new captures. (2026-11-13)
- [~] 3.6 Real-host check (TC-048), by the user. Done without prefix (2026-11-13): all VMs and disk sizes correct. Open: a run with `--prefix "sudo -n"`. Command: `cargo run -p vi-backend --example inventory -- <host>`. Confirms that the read-only connection allows `domblkinfo`, and that the output matches `virsh list --all` on the host.

### Phase 4: GUI (SPEC-003, 005, 012, 014, 015)
- [x] 4.1 Main window: host/VM tree, detail panel, toolbar, filter. (`vi-gui/src/app.rs`, `view.rs`)
- [x] 4.2 Background refresh per host with status; refresh all runs in parallel; SSH connection kept per host. (`worker.rs`)
- [x] 4.3 Host add/edit/remove dialogs; dialogs for the host key prompt and the passphrase prompt (`prompts.rs`). Passphrases are not kept; the open connection makes re-asking unnecessary.
- [x] 4.4 Inventory cache: save, load and clear; "cached" marker. (`vi-core/src/cache.rs`)
- [x] 4.5 Settings window with the contrast control.
- [x] 4.6 "Reload config" action.
- [x] 4.7 `vi-gui` is now a library plus a thin binary; `tests/app.rs` drives the app and renders every view headless (`Context::run_ui`).
- [x] 4.8 Smoke test under Xvfb (2026-11-13). Found and fixed: egui fades text at scroll-area edges (contrast); dialog fields too narrow; settings paths not wrapped; form error wording.
- [x] 4.10 Crosscheck 2026-11-14-01 fixes: stale-connection race (generations), disabled button colour, passphrase wiped on skip, ages repaint, cache cleanup on reload; UUID from XML; `refresh_on_start`; keyboard navigation. (2026-11-14)
- [x] 4.11 Local host without login (SPEC-020): `[local]` config, `LocalRunner` (`sh -c`), `virsh` detection, "localhost (system)" and "localhost (session)" entries. Checked under Xvfb with a fake `virsh`. (2026-11-14)
- [ ] 4.9 Manual check on a real desktop against a real host (TC-081, TC-006 on Windows/macOS): refresh, host key and passphrase dialogs, clipboard.

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
| russh agent support on Windows (named pipe / Pageant) | Source review (2.5): russh supports both, and both are used. Compiles and tests pass in CI (run 43509629083). Open: a manual run (TC-038). Fallback: key files with a passphrase prompt. |
| virsh output and XML differ between libvirt versions | Fixtures from several versions; tolerant parser (unknown elements are ignored, not fatal). |
| `LC_ALL=C` through `sudo -n` is not yet checked on a real host (`virsh -r` with `domblkinfo` is confirmed, TC-048) | TC-048 with `--prefix`. If `domblkinfo` is refused on a read-only connection, sizes show as unknown (no failure); if sudo drops `LC_ALL`, non-English output would give parse problems. |
| `qemu:///system` permissions | Clear error message with a hint (SPEC-013); `virsh_prefix = "sudo -n"` option. |
| macOS signing and notarization | Postpone to 5.1; an unsigned build is fine for development. |
| No mature Rust SPICE client | v1.1 uses an external `remote-viewer`. |

## Done
- [x] Project setup: git repo, `architecture/` docs, `Changes.md`, `CLAUDE.md` with working rules (2026-11-13).
- [x] Scenario captured in Spec.md (2026-11-13).
