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
