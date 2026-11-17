# Conversation 2026-11-17

## 1. 1.0.0 published
**User:** Published.

**Claude:** Checked: release v1.0.0 is public (published 2026-11-17) with the 3 archives and their `.sha256` files and the 1.0.0 notes; the downloaded Linux archive matches its checksum. CI run 43960743658 (`b87b57e`) passed. Plan 5.2 marked done; README status updated. The `## 1.0.0 (2026-11-16)` heading keeps the tag date. Next: phase 6 (v1.1 consoles).

## 2. Phase 6.1: console tunnels
**User:** Start phase 6.

**Claude:** Phase 6.1 (SPEC-018), no GUI change yet:
- `vi-core`: `ConsoleTarget` (TCP host:port or unix socket) and `Graphics::console_target()`: a wildcard or default listen is reached as `127.0.0.1` on the host; unsupported kinds, no port, TLS-only, unknown socket or network address give a reason.
- `vi-ssh`: `CommandRunner::open_stream` (default: "cannot forward"); `SshSession` opens `direct-tcpip` or `direct-streamlocal` channels. `vi_ssh::Tunnel`: a listener on `127.0.0.1:0`, forwards every connection until closed or dropped, counts connections and keeps the last error.
- Tests: TC-100 (target part), TC-101 (tunnel unit tests), new TC-102a (through a real SSH session to the in-process test server, which now accepts forwarding channels: TCP, unix socket, refused target), TC-104 (part). 134 tests pass; fmt and clippy clean.
- Spec SPEC-018 describes the target rules and the tunnel behaviour, including that any local process can reach the port while it is open.

## 3. Phase 6.2: console button and viewer
**User:** CI for 6.1 pushed; start 6.2.

**Claude:** CI run 44057271940 (`8f6cec3`) passed: 134 tests (Windows 126). Phase 6.2 (SPEC-018):
- Backend: `HypervisorBackend::graphics(runner, domain)` reads the live XML again before opening (default: no consoles, so TC-090 holds). `LocalRunner::open_stream` connects directly (TCP or unix socket): local VMs need no SSH but use the same listener.
- Config `[console] viewer`; `vi-gui::console`: viewer search (PATH, Homebrew dirs, `Program Files\VirtViewer*\bin` newest first), install hint, URL.
- Worker: `open_console`/`close_console`, `Event::Console` (Opening, Open, Closed with error). Uses the kept connection or connects without changing the host status. The viewer runs with the tunnel URL; the tunnel closes when it exits or on Close; forgetting a host or reloading the config closes its consoles.
- GUI: "Open console"/"Close console" in the graphics table (column "Console"), disabled with the reason when not possible; errors as messages.
- Found and fixed during testing: (1) the tunnel port could still accept briefly after "closed" → `Tunnel::shutdown().await`; (2) closing killed a wrapper script but not its child → the viewer gets its own process group on unix and the group gets SIGTERM (then SIGKILL after 2 s), via `rustix` (already in the tree; the workspace forbids `unsafe`); the test fails without the fix; (3) a test race (helper pid file) fixed.
- End-to-end under Xvfb with a fake `virsh` (fixture VM, VNC port of a fake server) and a fake viewer: the viewer read `RFB 003.008` through the tunnel; Close and closing the window stop the viewer and its helper. No real `remote-viewer`/libvirt on the dev machine: TC-103c (real host, all platforms) is open.
- `domdisplay` fallback dropped from the Spec: the live XML of a running domain always has the port.
- 144 tests pass (3 runs); fmt and clippy clean. Spec, TestCases (TC-100, 103, 103a–c, 104, 105), Plan 6.2, README (Consoles), example config, Changes (Unreleased) updated.

## 4. CI fix for Windows
**User:** Pushed 6.2.

**Claude:** CI run 44066898742 (`fe7ca44`): Linux, macOS and MSRV pass with 144 tests; Windows failed in clippy: the `ViewerGroup` field is only read on unix (`dead_code`). Fixed with `#[cfg_attr(not(unix), allow(dead_code))]` on the field (no Windows target on the dev machine; the other unix-only paths were reviewed).

## 5. Phase 6.3: remote-viewer per platform
**User:** Start phase 6.3.

**Claude:** Checked the install routes (web, 2026-11-17):
- Linux: the `virt-viewer` package (Debian/Ubuntu, Fedora).
- macOS: Homebrew has **no** `virt-viewer` formula (`formulae.brew.sh` 404) — the README/hint from 6.2 ("brew install virt-viewer") was wrong. The `remoteviewer` cask is version 0.5.7 (SPICE only, old). MacPorts has `virt-viewer` 11.0 in `/opt/local/bin`, which was not on the search path: added to `EXTRA_DIRS` (also for `virsh`).
- Windows: virt-viewer 11.0 (2021-11-18, still the latest) as MSI from gitlab.com/virt-viewer (linked from virt-manager.org), also winget `RedHat.VirtViewer` 11.0.256 (manifest with SHA256).
Added `examples/console-check.rs` (find the viewer and run `--version`; or open a console of this machine through the tunnel and pass if the viewer stays connected; tried with stand-ins: pass and both fail cases) and `.github/workflows/viewer-check.yml` (by hand: Linux apt + TigerVNC real console; macOS MacPorts via setup-macports pinned to the v1.1.6 commit; Windows MSI with checksum, found through the Program Files search). Install hints, README (table), Spec, example config, TestCases (TC-106 new, TC-112), Plan 6.3, Changes updated. The workflow has to be run by the user.

## 6. Viewer check run
**User:** Status? Start the viewer check.

**Claude:** CI run 44070390690 (`560d5fd`) passed on all four jobs (144 tests; Windows 133). Starting the "Viewer check" workflow from here failed (HTTP 403: the token cannot dispatch workflows); the user started it. Run 44167663646 passed on all three platforms:
- Linux (ubuntu-22.04): `/usr/bin/remote-viewer` 7.0 found; a real VNC console (TigerVNC `Xvnc`) opened through the tunnel, still connected after 8 s.
- macOS: MacPorts installed `virt-viewer` (about 15 minutes); found at `/opt/local/bin/remote-viewer`, version 11.0.
- Windows: MSI (checksum matched) installed to `Program Files\VirtViewer v11.0-256\bin`, found by the Program Files search (not on `PATH`), version 11.0-256.
TC-106 pass; Plan 6.3 done. Still open: TC-103c (real host and viewer on all platforms), TC-121 on macOS.

## 7. Crosscheck of phase 6
**User:** Pushed; start the crosscheck.

**Claude:** `architecture/Crosscheck-2026-11-17-01.md`: fmt, clippy clean, 144 tests pass. SPEC-018 matches the code except: C1 a reopen race (closing removes the console from the worker's table before its task ends; a quick reopen gets its handle dropped by the old task), C2 SIGKILL after 2 s reaches only the viewer, not its process group, C3 Spec wording on app exit; doc fixes D1 (TC-102 stale), D2 (reconnect once not in the Spec), D3 (TC-105 wording), D4 (Windows dirs searched twice). Fixes proposed; waiting for the user's OK.

## 8. Crosscheck fixes
**User:** Apply C1–C3 and D1–D4.

**Claude:** Applied (details in the crosscheck's Resolution):
- C1: a console's key stays in the worker's table until its task has ended; reopening is refused meanwhile and the app shows "closing…". New test `reopen_waits_until_the_old_viewer_stopped`.
- C2: the viewer's process group gets SIGKILL after 2 s (also when only helpers are left). `close_console_stops_the_viewer` now uses a wrapper and helper that ignore SIGTERM.
- Both tests were checked against the old code (each fails without its fix).
- C3, D2: SPEC-018; D1: TC-102 covered; D3: TC-105; D4: Windows `VirtViewer*\bin` folders searched once.
- 145 tests pass (3 runs); fmt and clippy clean.
