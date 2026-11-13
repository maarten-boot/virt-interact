# Changes

Changes between releases. Newest on top.

## Unreleased
- Project setup: git repo, `architecture/` docs (Spec, Plan, TestCases, conversation log), working rules in `CLAUDE.md`.
- Spec, Plan and TestCases for v1 (read-only inventory) and v1.1 (remote viewing).
- Phase 0: Cargo workspace (`vi-core`, `vi-ssh`, `vi-backend`, `vi-gui`), empty white/black GUI window, CI for Windows/Linux/macOS, README.
- Anonymised virsh fixtures (libvirt 12.0.0 and 10.0.0 live captures plus 3 stopped domain XMLs); Spec/TestCases updated with the parsing findings.
- Phase 1: TOML config with hosts, validation and line-numbered errors (an invalid file is never overwritten); `--config`, `--help`, `--version`; WCAG contrast maths and palette generation (AA/AAA/Max/custom); egui theme with a contrast control in the toolbar that saves to the config; file logging.
- Crosscheck 01 fixes: disabled widgets no longer fade below the contrast minimum (egui colour-pair test added); infinite/NaN contrast values clamped; port 0 rejected; `~` expanded in `identity_file`; `examples/config.example.toml`; README usage section; CI also builds macOS x86_64.
- Phase 2: SSH layer (`vi-ssh`): `~/.ssh/config` subset, own known_hosts check/learn (hashed, patterns, `@revoked`), key-only auth (agent incl. Windows pipe/Pageant, key files, passphrase prompt), command runner with timeouts and a mock; integration tests against an in-process SSH server; `ssh-run` example for manual tests.
- Crosscheck 02 fixes: auth errors keep the list of keys tried (also when the server hangs up); clear error when the host key changes between attempts; config timeouts used by the SSH layer; agent keys matching configured key files are tried first; system-wide known_hosts files are read; `examples/ssh_config.example`.
- Phase 3: libvirt backend (`vi-backend`): allowlisted read-only virsh commands (`LC_ALL=C`, `virsh -r`, POSIX quoting, validated `virsh_prefix`); parsers for `version`, `list --all`, `dominfo`, `domblkinfo` and the domain XML (general, CPU, memory, disks, NICs, graphics, other devices); error classification with hints; per-domain errors do not stop the host; inventory model in `vi-core`; `inventory` example for manual tests.
- CI fixes: `clippy::question_mark` (Rust 1.100); `.gitattributes` keeps LF line endings on all platforms; README example test tolerates CRLF; CI tests use `--no-fail-fast`.
