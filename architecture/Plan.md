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
  vi-backend/  HypervisorBackend trait + libvirt implementation (virsh + XML parsing), local runner (SPEC-020)
  vi-gui/      library (app, worker, prompts, view, theme) + thin binary `virt-interact`; tests drive the app headless
tests/fixtures/  captured virsh outputs (XML, list, domblkinfo) for parser tests
scripts/         package.sh, release_notes.sh, make_icon.py, anonymise_fixtures.py
packaging/       app icon (PNG/ICO/ICNS), macOS Info.plist, Linux .desktop
about.toml       accepted third-party licences (cargo-about; about.hbs is the notices template)
.github/workflows/  ci.yml (every push: tests, clippy, archives), release.yml (tag: draft release)
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
- [x] 4.12 Crosscheck 2026-11-14-02 fixes: feedback by colour only (same border width in every state) and a tree that scrolls instead of widening, so nothing moves on hover (user report); tree width and window size kept between runs (eframe persistence); `virsh` search path for macOS; local labels from the URI; cache files of vanished entries deleted. Checked under Xvfb. (2026-11-14)
- [ ] 4.9 Manual check on a real desktop against a real host (TC-081, TC-006 on Windows/macOS): refresh, host key and passphrase dialogs, clipboard.

### Phase 5: Release v1
- [x] 5.1 Packaging (SPEC-002 distribution): `scripts/package.sh` builds and checks the archive per platform: Linux tar.gz (built on Ubuntu 22.04), Windows zip (GUI subsystem, no console), macOS universal `.app` in a zip (ad-hoc signed, not notarized). Release profile with thin LTO and stripped symbols. CI packages on every push (TC-120); `.github/workflows/release.yml` tests and packages all platforms for a `v<version>` tag and creates a **draft** GitHub release with the `Changes.md` section (`scripts/release_notes.sh`). Later: MSI, AppImage/deb, notarization. (2026-11-14)
- [x] 5.1a Crosscheck 2026-11-14-03 fixes: MIT `LICENSE`; `THIRD_PARTY_LICENSES.html` in every archive (cargo-about, accepted list in `about.toml`); Windows shows `--help`/`--version`/start-up errors in a small window; CI Linux on ubuntu-22.04 like the release; app icon (window, Windows exe, macOS bundle, Linux `.desktop`). (2026-11-14)
- [x] 5.2 Release 1.0.0 (tagged 2026-11-16, published 2026-11-17):
  - [x] version 1.0.0 in `Cargo.toml`; `## 1.0.0` in `Changes.md` with user-facing release notes (features, downloads, known limitations), the development history below it (2026-11-16);
  - [x] release crosscheck (2026-11-16-01; R1/R2 applied);
  - [x] the user runs `release.yml` via "Run workflow" (optional), pushes tag `v1.0.0`, tries the draft release's archives on each platform (TC-121) and publishes it.

### Phase 6: v1.1 remote viewing (SPEC-018)
- [x] 6.0 New app icon from the user's logo (a blue "V" on a green rounded square): master `packaging/icon/logo.png`, all sizes made by `scripts/make_icon.py` (2026-11-16).
- [x] 6.1 Local listener and `direct-tcpip` / `direct-streamlocal` forwarding: `ConsoleTarget` from the live graphics (vi-core), `CommandRunner::open_stream` (SSH: direct-tcpip/streamlocal), `vi_ssh::Tunnel`; tests TC-100 (target), TC-101, TC-102a, TC-104 (part) (2026-11-17).
- [x] 6.2 Viewer launch, configurable path, tunnel lifetime tied to the viewer process: "Open console"/"Close console" per graphics device; live graphics read again on open (`HypervisorBackend::graphics`); local VMs connect directly; `[console] viewer` and the viewer search; the viewer's process group is stopped on close (unix); tests TC-100, TC-103/103a/103b, TC-104, TC-105; end-to-end check under Xvfb with stand-ins (2026-11-17). TC-103c: Linux pass (2026-11-18), Windows pass (2026-11-19); macOS open (no Mac).
- [x] 6.3 Check that `remote-viewer` is available on each platform and document how to install it: Linux `virt-viewer` package; macOS MacPorts (no Homebrew formula; `/opt/local/bin` added to the search path); Windows winget `RedHat.VirtViewer` or the MSI (virt-viewer 11.0). README table; `examples/console-check.rs`; `viewer-check.yml` (by hand). Workflow run 44167663646 passed on all three platforms (TC-106).
- [x] 6.4 Release 1.1.0 (tagged and published 2026-11-19):
  - [x] version 1.1.0 in `Cargo.toml`; `## 1.1.0` in `Changes.md` with user-facing release notes, `## Unreleased` kept on top; README status (2026-11-19);
  - [x] release crosscheck (2026-11-19-01);
  - [x] the user pushed tag `v1.1.0` and published the release; the six files were attached by hand (the published release first had none), all checksums verified (2026-11-19). TC-121 for 1.1.0 open.

### Phase 7: v1.2 tree menus and SSH in a terminal (SPEC-024, 025, SPEC-002 release)
Decided 2026-11-19: menus on VMs (consoles, close, copy name/UUID) and hosts (refresh, edit, remove, start SSH, copy ssh command); terminal detected automatically and configurable in the config file and in Settings; the release pipeline publishes by itself after checking the files; vSphere moves to 1.3.
- [x] 7.1 Right-click menus on VMs and hosts (SPEC-024): menu items as data (`vm_menu_items`, `host_menu_items`), drawn with egui context menus; tests TC-140, TC-141; checked under Xvfb (2026-11-19).
- [x] 7.2 SSH command and terminal launch per platform (`vi-gui::terminal`); `[terminal] command`; Settings field with the terminal in use; tests TC-142, TC-143; under Xvfb a configured stand-in terminal got `-e ssh -l ops -- <host>` (2026-11-19). Open: TC-144 (real terminals).
- [x] 7.3 Release pipeline: `scripts/publish_release.sh` checks the six files and checksums, then publishes as latest; tried against v1.0.0 and v1.1.0 (publish step stubbed) and with a missing file (refused) (2026-11-19). Open: TC-123 on the next tag.
- [ ] 7.4 Crosscheck; release 1.2.0.

### Phase 8: v1.3 VMware vSphere (SPEC-021, 022, 023)
Decided 2026-11-18: password asked per app session and kept only in memory; flat tree with datacenter/cluster/ESXi host as VM fields; after release 1.1 (2026-11-19: moved to 1.3, after the tree menus). Tests: vcsim (govmomi's vSphere simulator, ESXi and vCenter mode) in CI, recorded SOAP responses as fixtures; manual check on the user's ESXi host and a vCenter with a restricted view.
- [ ] 8.1 Config: `backend = "vsphere"`, field rules (SPEC-005); host editor shows only the fields of the chosen backend.
- [ ] 8.2 HTTPS client: rustls (ring) on tokio; certificate check and `known_certs` with the accept dialog (SPEC-023).
- [ ] 8.3 SOAP layer with the method allowlist; Login/Logout, session kept per host and ended on forget/reload/quit; password dialog (SPEC-022).
- [ ] 8.4 Inventory: PropertyCollector traversal (paged), mapping to the model, permission gaps tolerated (SPEC-021); fixtures from vcsim and from real ESXi/vCenter responses (anonymised).
- [ ] 8.5 GUI: vSphere fields (datacenter, cluster, ESXi host, Tools, hardware version), "n/a" for libvirt-only fields, console "not available for vSphere"; README (Read-only role, certificates).
- [ ] 8.6 CI job with vcsim (ESXi and vCenter mode); manual tests on real systems; crosscheck.

### Later
- SPEC-019 embedded VNC viewer; other backends (SPEC-017); `ProxyJump` support; vSphere password in the OS keychain (optional).

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
