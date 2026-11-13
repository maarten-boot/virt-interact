# virt-interact: project working rules

These rules were agreed in the session of 2026-11-13. See `architecture/Conversation-2026-11-13.md`.

## Documentation layout
- `architecture/Conversation-YYYY-MM-DD.md`: running conversation log, one file per day.
- `architecture/Plan.md`: implementation plan. Keep it up to date.
- `architecture/Spec.md`: specification with numbered items (SPEC-nnn). Keep it up to date.
- `architecture/TestCases.md`: test cases, each traced to a Spec item. Written as the work proceeds.
- `architecture/Crosscheck-YYYY-MM-DD-nn.md`: crosscheck reports. `nn` is two digits and restarts at 01 each day.
- `Changes.md` (repo root): changes between releases. Keep an "Unreleased" section on top.
- Commit docs on `main`.

## Input review
- If a user sentence contains errors or uncertainties, suggest corrections and confirm before proceeding.

## Crosschecks
- After significant changes, suggest that it is time for a crosscheck.
- When the user says "crosscheck" or "do a crosscheck":
  1. Inspect all code and match it against Spec.md and Plan.md.
  2. Inspect Spec.md and Plan.md and check them against the code.
  3. Verify that all examples match the code, Spec and Plan.
  4. Check TestCases.md against the code and the actual tests.
  5. Run the test suite and report which tests pass or fail.
  6. Add missing examples where there is reason to.
  7. Suggest other improvements.
  8. Save the report in `architecture/Crosscheck-YYYY-MM-DD-nn.md` and post a short summary in chat.
- For mismatches between code and docs, propose the fix and wait for the user's OK before changing either side.

## GUI / TUI colour and contrast
- Default to a white background with black text.
- Every foreground/background pair must reach a WCAG contrast ratio of at least 4.5:1 (AA).
- Let the user raise contrast interactively and through a config file, with presets AA (4.5:1), AAA (7:1) and Max (21:1, pure black/white) plus a custom minimum ratio.

## Privacy of test data
- Never commit real machine names, hostnames, IP or MAC addresses, UUIDs or image paths (for example, in fixtures, docs, logs or commit messages). Substitute random data, consistently within one capture. Use `scripts/anonymise_fixtures.py` and review the result; see `tests/fixtures/README.md`.
