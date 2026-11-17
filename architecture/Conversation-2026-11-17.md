# Conversation 2026-11-17

## 1. 1.0.0 published
**User:** Published.

**Claude:** Checked: release v1.0.0 is public (published 2026-11-17) with the 3 archives and their `.sha256` files and the 1.0.0 notes; the downloaded Linux archive matches its checksum. CI run 43960743658 (`b87b57e`) passed. Plan 5.2 marked done; README status updated. The `## 1.0.0 (2026-11-16)` heading keeps the tag date. Next: phase 6 (v1.1 consoles).

## 2. Phase 6.1: console tunnels
**User:** Start phase 6.

**Claude:** Phase 6.1 (SPEC-018), no GUI change yet:
- `vi-core`: `ConsoleTarget` (TCP host:port or unix socket) and `Graphics::console_target()`: a wildcard or default listen is reached as `127.0.0.1` on the host; unsupported kinds, no port, TLS-only, unknown socket or network address give a reason.
- `vi-ssh`: `CommandRunner::open_stream` (default: "cannot forward"); `SshSession` opens `direct-tcpip` or `direct-streamlocal` channels. `vi_ssh::Tunnel`: a listener on `127.0.0.1:0`, forwards every connection until closed or dropped, counts connections and keeps the last error.
- Tests: TC-100 (target part), TC-101 (tunnel unit tests), new TC-102a (through a real SSH session to the in-process test server, which now accepts forwarding channels: TCP, unix socket, refused target), TC-104 (part). 134 tests pass; fmt and clippy clean.
- Spec SPEC-018 describes the target rules and the tunnel behaviour, including that any local process can reach the port while it is open.
