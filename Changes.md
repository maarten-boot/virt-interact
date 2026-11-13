# Changes

Changes between releases. Newest on top.

## Unreleased
- Project setup: git repo, `architecture/` docs (Spec, Plan, TestCases, conversation log), working rules in `CLAUDE.md`.
- Spec, Plan and TestCases for v1 (read-only inventory) and v1.1 (remote viewing).
- Phase 0: Cargo workspace (`vi-core`, `vi-ssh`, `vi-backend`, `vi-gui`), empty white/black GUI window, CI for Windows/Linux/macOS, README.
- Anonymised virsh fixtures (libvirt 12.0.0 and 10.0.0 live captures plus 3 stopped domain XMLs); Spec/TestCases updated with the parsing findings.
- Phase 1: TOML config with hosts, validation and line-numbered errors (an invalid file is never overwritten); `--config`, `--help`, `--version`; WCAG contrast maths and palette generation (AA/AAA/Max/custom); egui theme with a contrast control in the toolbar that saves to the config; file logging.
