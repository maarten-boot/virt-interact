# Conversation 2026-11-16

## 1. Phase 5.2: release crosscheck
**User:** Continue (phase 5.2, release 1.0.0).

**Claude:** CI run 43647045859 (`7b50a3e`) passed on all 4 jobs and packaged all three platforms; the `::error::` lines in its log are the install-action script being echoed, not a failure. Ran the release crosscheck `Crosscheck-2026-11-16-01.md`: fmt, clippy, 127 tests, `--version` 1.0.0, `release_notes.sh 1.0.0`, the tag check, and the release notes against Spec and code. Updated TC-010/TC-120 with run 43647045859. Committed the version bump, release notes and crosscheck.

Findings waiting for the user's OK:
- R1: `Changes.md` lost its "Unreleased" section (the project CLAUDE.md asks for one).
- R2: the release date in `## 1.0.0 (2026-11-14)` should be the tag date.

## 2. R1 and R2
**User:** Yes, apply R1 and R2.

**Claude:** Added `## Unreleased` on top of `Changes.md` and dated the 1.0.0 release 2026-11-16. Checked `release_notes.sh 1.0.0`. Committed.

## 3. Tag v1.0.0
**User:** Pushed `main`, then the tag `v1.0.0`.

**Claude:** CI run 43837860224 (`9fdc0da`) passed on all 4 jobs (127 tests; Windows 120) and built archives named 1.0.0. Release run 43839695459 passed: it packaged all three platforms and created the draft release (not yet published). The local token cannot read draft releases; the release job's log shows the `gh release create --draft` call and the draft's URL. TC-120 updated. Open: TC-121 (the archives on each platform), then the user publishes.
