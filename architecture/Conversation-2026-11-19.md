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
