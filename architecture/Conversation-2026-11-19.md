# Conversation 2026-11-19

## 1. Status and TC-103c on Windows
**User:** Status?

**Claude:** In sync with GitHub. CI run 44340475976 (`07795b4`, docs only) shows failure: the macOS job never got a runner (cancelled after 15 minutes, no steps); the other three jobs passed and the same code passed on macOS in run 44324750113. One release (v1.0.0) and 71 CI artifacts, none older than 14 days: no cleanup needed.

**User:** Console works on Windows.

**Claude:** TC-103c: Windows pass (Linux passed on 2026-11-18); macOS stays open (no Mac). Plan 6.2 updated.

## 2. Release 1.1.0 prepared
**User:** Pushed; prepare the 1.1.0 release.

**Claude:** Version 1.1.0 (`Cargo.toml`, `Cargo.lock`); `## 1.1.0 (2026-11-19)` in `Changes.md` with user-facing notes (consoles, other changes, downloads, known limitations: no real console test on macOS, TLS-only unsupported, external viewer, vSphere planned for 1.2), `## Unreleased` with "Nothing yet." on top; README status; Plan 6.4. Release crosscheck 2026-11-19-01: every note matches Spec and code; R1 (Spec header v1.2) and R2 (TC-121 for 1.1.0) applied; R3 date stays if tagged today. 145 tests pass; `--version` says 1.1.0; `release_notes.sh 1.1.0` prints the section.
