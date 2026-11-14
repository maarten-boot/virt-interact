# Conversation log: 2026-11-14

## 1. Crosscheck 01 (after phase 4)
**User:** Do a crosscheck. (Phase 4 had been pushed meanwhile.)

**Claude:** CI run 43589887347 for phase 4 passed on all four jobs (107 tests; 106 on Windows). Saved `architecture/Crosscheck-2026-11-14-01.md`; fmt and clippy are clean, and 107 tests pass locally. Main findings:
- C1: a refresh that is still connecting can store its SSH connection after the host was edited, removed or the config reloaded, so a later refresh reuses old settings.
- C2: disabled buttons look exactly like enabled ones, because fading is off for contrast; the palette's disabled colour is not used.

Smaller findings:
- C3: a skipped passphrase buffer is not wiped.
- C4: status ages do not update while the window is idle.
- C5: cache files of hosts removed via the file are left behind.
- C6: a status flicker.
- S2, S3: README and Plan layout wording.
- T2: no test for the save-failure path.

Everything waits for the user's OK.

**User:** Apply all fixes, including improvements 1–3.

**Claude:** Applied C1–C6, S2–S3, T2 and improvements 1–3 (details in Crosscheck-2026-11-14-01 §8):
- C1: a stale-connection race closed with per-host generations.
- C2: disabled buttons now visibly disabled (seen under Xvfb).
- C3: a skipped passphrase is wiped.
- C4: status ages refresh while idle.
- C5: cache files of removed hosts are cleaned on reload.
- Improvement 1: UUID from the XML.
- Improvement 2: `[ui] refresh_on_start`.
- Improvement 3: keyboard navigation. Its test found an integer overflow on Home/End, which was fixed.

113 tests pass; fmt and clippy are clean.

## 2. CI for the crosscheck fixes
**User:** Pushed.

**Claude:** Run 43599218266: all four jobs pass. 113 tests on Linux, macOS and MSRV 1.92; 112 on Windows (the unix-only `sh` quoting test is skipped). TC-010 updated. Open: the manual GUI check against a real host (Plan 4.9) and TC-048 with `--prefix "sudo -n"`; then phase 5.

## 3. Local host without login
**User:** Noted that the local host does not need a login, and asked whether it can always be shown when QEMU is installed.

**Claude:** Suggested a rewording and a correction: what matters is libvirt (`virsh`), not QEMU itself. Proposed:
- a local entry that runs the same read-only virsh commands through the local shell, shown when `virsh` is on `PATH`;
- a `[local]` config section (`enabled = auto|always|never`, `libvirt_uri`, `virsh_prefix`).

Asked whether `qemu:///session` should be included too.

**User:** Go ahead; include `qemu:///session` too.

**Claude:** Implemented SPEC-020:
- `[local]` config (with `session = true`); names starting with `@` are reserved.
- `vi-backend/src/local.rs`: `LocalRunner` (`sh -c`, time limit) and `virsh` detection.
- `HostConnector`: local entries run locally, others over SSH.
- GUI: "localhost (system)" and "localhost (session)" first in the tree. They cannot be edited; their panel shows the connection as local; Settings shows where `virsh` was found.
- Tests: TC-110 to TC-115, including an end-to-end inventory through a fake `virsh` that checks every call has `LC_ALL=C` and `-r`.
- Checked under Xvfb with that fake `virsh` on `PATH`: detected, refreshed and details shown (TC-116).

This machine has no libvirt, so the check against a real local libvirt is for the user.
