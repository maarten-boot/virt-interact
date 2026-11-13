# Conversation log: 2026-11-13

## 1. Project setup rules
**User:** Gave the working rules for the project: a daily conversation log, Plan.md, Spec.md and test cases kept as we go; crosscheck suggestions after significant changes; review of input sentences before proceeding; files in an `architecture/` folder; a new git repo; Changes.md for release changes; white background / black text with at least 4.5:1 contrast for any GUI/TUI, adjustable interactively and through a config file; and a detailed crosscheck procedure.

**Claude:** Found an empty directory with no git repo. Raised these points for confirmation:
1. Use `architecture/` (not the global default `docs/planning/`) and commit docs on `main`.
2. Name the test case file `architecture/TestCases.md`, with each test traced to a Spec item.
3. Put `Changes.md` in the repo root.
4. Treat the `;;`-wrapped contrast line as an active rule, with presets AA 4.5:1, AAA 7:1, Max 21:1 and a custom ratio.
5. Name crosscheck files `architecture/Crosscheck-YYYY-MM-DD-nn.md`, with `nn` restarting at 01 each day.

**User:** Yes to all, go ahead.

**Claude:** Ran `git init` (branch `main`). Created `CLAUDE.md` (project rules), `architecture/` with this log, `Plan.md`, `Spec.md` and `TestCases.md`, plus `Changes.md` and `.gitignore`. Made the initial commit. Waiting for the scenario.

## 2. Scenario
**User:** A Rust app for Windows, Linux and macOS with a GUI. The user defines remote hosts and logs in over SSH without a password. The app builds an inventory of the virtual machines on each host (virsh first, maybe other platforms later): every machine, running or stopped, and for each machine its components (disk, CPU, memory, etc.). After that, remote viewing of running machines with VNC or SPICE, tunnelled over the same SSH connection. Asked for Claude's opinion.

**Claude:** Suggested wording fixes. Gave an assessment and raised open design questions: SSH implementation, libvirt URI and permissions, how the inventory is gathered, GUI toolkit, viewer (embedded or external), scope (read-only or with actions), config storage, and host key checking. Waiting for confirmation before writing the Spec and Plan.

## 3. Design decisions
**User:** Go with the recommendations; v1 is read-only.

**Decisions:** `russh` for SSH (key-based only, agent plus key files, `~/.ssh/config` subset, known_hosts checking); `qemu:///system` by default, configurable per host, with an optional `virsh_prefix`; inventory built from `virsh list/dominfo/dumpxml/domblkinfo` over SSH, with no client-side libvirt; `egui`/`eframe` for the GUI; TOML config in the platform config directory. v1 = read-only inventory, enforced by a command allowlist. v1.1 = VNC/SPICE through an SSH tunnel with an external `remote-viewer`. Later: embedded VNC viewer and other backends.

**Assumption (Claude):** The local inventory cache (question 3) was not answered explicitly. It is included in v1 as SPEC-014 and can be dropped if not wanted.

**Claude:** Wrote Spec.md (SPEC-001 to SPEC-019), Plan.md (phases 0–6, crates, risks) and TestCases.md (TC-001 to TC-103, traced to the Spec). Switched `.gitignore` to Rust.

## 4. Phase 0
**User:** Start phase 0. The user will collect sample XML files.

**Claude:** Created the Cargo workspace (4 crates), an empty eframe window with the white/black theme, a CI workflow (3 OSes plus an MSRV job), a README and a fixtures guide.
- The local rustc is 1.93 (distro package, no rustup). eframe 0.36+ needs 1.95, so eframe is pinned to 0.35 and MSRV is set to 1.92.
- Build, fmt, clippy (`-D warnings`) and tests pass locally. The GUI could not be launched in this session (no display), so TC-011 is for the user to check.
- The user's XML files in `tmp/` were left out of the commit.

## 5. Sample XML and fixtures
**User:** Provided 3 domain XMLs in `tmp/` (stopped VMs) and later a tar archive: a full capture from one host (libvirt 12.0.0, QEMU 10.2.1) with 7 running VMs (version, list, dominfo, dumpxml, domdisplay, domblkinfo).

**Claude:** Analysed both sets (details in the chat). Proposed Spec additions: guest OS from libosinfo, empty cdrom, `listen type="none"` not reachable remotely, the VNC display-number format of `domdisplay`, and tolerant parsing of dominfo. Proposed anonymising the fixtures before committing, because the repo may be public and the samples contain internal hostnames, a person's name, MAC addresses and UUIDs. Waiting for the user's OK.

**User:** Never use machine names or their IP or MAC addresses in any commit; substitute random data.

**Claude:** Anonymised everything with a script (kept in the session scratchpad, not in the repo). VM names became random `vm-xxxxxx`. Image paths, channel paths, MAC addresses and UUIDs (including security labels) were replaced with random values, applied consistently across files. The only IP was loopback `127.0.0.1`, which was kept. The script's leak check found nothing, and the repo history contains no real names. The mapping was not saved. Added the rule to `tests/fixtures/README.md` and `CLAUDE.md`. Applied the proposed Spec, TestCases and Plan updates.

**User:** Added another capture directory in `tmp/` (a UEFI VM may not be available at the moment).

**Claude:** Second host: libvirt 10.0.0 / QEMU 8.2.2, 3 running VMs. New cases: SPICE `listen none` on a running VM (`domdisplay` reports "No graphical display found"), a NAT network with `portid`/`virbr0`, a `<title>`, and dominfo without `Autostart Once`. Extended the anonymiser to cover `<title>`/`<description>` and to rebuild the `list --all` table in virsh's column layout. No leaks found. Committed as `tests/fixtures/libvirt-10.0.0/`. UEFI stays on the missing list; it is not blocking.

## 6. Phase 1
**User:** Start phase 1.

**Claude:** Implemented in `vi-core`: `contrast` (WCAG ratio, levels, palette generation), `config` (TOML, defaults, validation, atomic save, read-only store when the file is invalid), `paths` (platform directories, `--config`/`--help`/`--version`) and `logging` (daily file, own crates at the configured level, others at `warn`). In `vi-gui`: a theme generated from the palette, a contrast control in the toolbar that saves to the config, and dismissable warning/error messages. There are 22 unit tests, and they pass. A smoke test under Xvfb showed the white/black window, a clamped contrast warning, a config error shown in the GUI without overwriting the file, and the log file being created. Two issues found and fixed during the smoke test: long messages were not wrapping, and the log contained library noise.

Spec clarifications made while implementing: unknown config keys are errors; contrast values above 21 are clamped; the log level applies to our own crates only (`RUST_LOG` overrides); the toolbar shows the lowest contrast ratio.

## 7. Crosscheck 01
**User:** Do a crosscheck.

**Claude:** Saved `architecture/Crosscheck-2026-11-13-01.md`. 23 tests pass, and fmt and clippy are clean. Main finding: egui's `disabled_alpha = 0.5` would make disabled widgets fall below 4.5:1 (latent SPEC-001 violation), and the theme mapping has no test. Other findings: the Plan and code disagree on where ssh-config parsing lives; the Spec has no entry for `--help`/`--version`, log level validation, or `~` expansion in `identity_file`; the README is out of date; macOS x86_64 is not built in CI. Added `examples/config.example.toml` plus a test. All other fixes are waiting for the user's OK.

**User:** Apply all fixes.

**Claude:** Applied C1–C5, S1–S7, E2 and E3 (details in Crosscheck-2026-11-13-01 §8). New tests: TC-007 (egui colour pairs), TC-026, TC-027, TC-028 (example file plus the README TOML blocks), TC-031a (`~` expansion, implemented ahead of phase 2) and TC-084. CI now cross-builds macOS x86_64. 28 tests pass.

## 8. Phase 2
**User:** Start phase 2.

**Claude:** Implemented `vi-ssh`:
- `sshconfig`: subset parser and `resolve`.
- `known_hosts`: own implementation, because russh's version is too limited (single-space split, no markers or patterns).
- `session`: russh 0.65 with the `ring` backend (no cmake/NASM on Windows). The unknown host key is asked about in a two-step connect, outside any network timeout. Auth is agent first, then key files, then a passphrase prompt; a key the agent already offered is skipped.
- `runner`: `CommandRunner` and `MockRunner`.
- `secret` (redacted, wiped on drop), `prompt` and `error` (with kinds for SPEC-013).

Windows: russh has both the OpenSSH agent pipe and Pageant clients, so the spike needs no workaround; only a manual run on Windows remains. Added test-only SSH keys in `tests/fixtures/ssh/` and integration tests against an in-process russh server (unknown/learned/rejected/mismatched host key, encrypted key, auth failure list, RSA, exit status and stderr, command timeout, connection refused). The passphrase log test runs in its own test binary because of tracing's callsite interest cache. Added the `ssh-run` example for manual tests. 52 tests pass, and clippy and fmt are clean.

Starting a local `sshd` and `ssh-agent` for a check against real OpenSSH was blocked by the permission system; the user can run that check instead (Plan 2.7).
