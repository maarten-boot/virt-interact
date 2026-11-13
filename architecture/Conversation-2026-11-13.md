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
