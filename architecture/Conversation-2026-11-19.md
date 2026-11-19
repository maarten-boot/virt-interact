# Conversation 2026-11-19

## 1. Status and TC-103c on Windows
**User:** Status?

**Claude:** In sync with GitHub. CI run 44340475976 (`07795b4`, docs only) shows failure: the macOS job never got a runner (cancelled after 15 minutes, no steps); the other three jobs passed and the same code passed on macOS in run 44324750113. One release (v1.0.0) and 71 CI artifacts, none older than 14 days: no cleanup needed.

**User:** Console works on Windows.

**Claude:** TC-103c: Windows pass (Linux passed on 2026-11-18); macOS stays open (no Mac). Plan 6.2 updated.

## 2. Release 1.1.0 prepared
**User:** Pushed; prepare the 1.1.0 release.

**Claude:** Version 1.1.0 (`Cargo.toml`, `Cargo.lock`); `## 1.1.0 (2026-11-19)` in `Changes.md` with user-facing notes (consoles, other changes, downloads, known limitations: no real console test on macOS, TLS-only unsupported, external viewer, vSphere planned for 1.2), `## Unreleased` with "Nothing yet." on top; README status; Plan 6.4. Release crosscheck 2026-11-19-01: every note matches Spec and code; R1 (Spec header v1.2) and R2 (TC-121 for 1.1.0) applied; R3 date stays if tagged today. 145 tests pass; `--version` says 1.1.0; `release_notes.sh 1.1.0` prints the section.

## 3. Release 1.1.0 published
**User:** Pushed the commit and the tag; how do I publish? Published.

**Claude:** CI run 44509699334 and Release run 44509974755 passed on all jobs; the Release run created a draft with the six files (11:07). The published release then had **no files** and a title ending in "." (likely a second, hand-made release). **User:** no draft left; uploaded the files by hand. Verified: six files, all three checksums OK. Plan 6.4 done; TC-121 for 1.1.0 open. Releases: 2 (v1.0.0, v1.1.0); 80 CI artifacts, none older than 14 days: no cleanup needed.

Open question from the user, "we should add that to the release pipeline": meaning to be confirmed (publish automatically after checking the files, or something else).

## 4. Tree menus, SSH in a terminal, release pipeline (phase 7, v1.2)
**User:** Add that to the release pipeline. No draft left; files uploaded. Can we have right-click menus on the VM names in the left pane, with "open console", and on a host "start ssh to host"?

**Claude:** Asked: terminal (auto-detect and configurable; **user:** also configurable in the preferences), menu items (all offered: VM close console/copy name/copy UUID; host refresh/edit/remove/copy ssh command), release (1.2 before vSphere, vSphere becomes 1.3), pipeline (publish after checking the files). Spec: SPEC-024 (menus), SPEC-025 (SSH in a terminal), SPEC-002 release step; vSphere SPEC-021–023 and phase 8 moved to v1.3.

Done:
- 7.1 Menus: items as data (`vm_menu_items`, `host_menu_items`) drawn as egui context menus; disabled items show the reason on hover; right-click does not change the selection.
- 7.2 `vi-gui::terminal`: ssh arguments (only the host entry's values, `--` before the host), terminal choice (configured, else Linux list / Terminal.app via osascript / Windows Terminal or a new console), Settings field "Terminal (Start SSH)" with Apply and a one-line note of the terminal in use; `[terminal] command` in the config and example.
- 7.3 `scripts/publish_release.sh`: exactly six files and matching checksums, then publish as latest; tried against v1.0.0/v1.1.0 (publish stubbed) and a missing file (refused).
- Under Xvfb: both menus shown; "Start SSH" gave a configured stand-in terminal `-e ssh -l ops -- <host>`; Settings row shown.
- 152 tests pass; fmt and clippy clean. Open: TC-144 (real terminals), TC-123 (next tag).
