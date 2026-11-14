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
