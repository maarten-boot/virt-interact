# Conversation 2026-11-18

## 1. VMware vSphere backend (planning)
**User:** Could we add reading VMware (ESXi) resources when we have a login?

**Claude:** Two routes: (A) SSH into the ESXi shell with a key (`vim-cmd`, `.vmx` files; fits the current design but SSH is off by default, the output is fragile, no vCenter), or (B) the vSphere API over HTTPS with user and password (official, also vCenter; needs a password, certificate pinning, an HTTPS client). Checked (web, 2026-11-18): the REST Automation API (`/api`) is vCenter-only, so the SOAP API (`/sdk`) is the one route for both; ESXi 7.0 removed the built-in VNC server, so no consoles for vSphere VMs; the free ESXi 8.0U3e API is read-only, which is enough.

**User:** B, username and password; standalone ESXi and vCenter. Decisions: password asked and kept in memory only; release 1.2 after 1.1; flat tree with ESXi host as a field; manual tests on the user's ESXi host and a vCenter with a restricted view (many resources hidden).

**Claude:** Drafted SPEC-021 (vSphere backend, read-only method allowlist, mapping, permission gaps tolerated, no consoles), SPEC-022 (password login, session kept in memory, Logout), SPEC-023 (certificate trust with `known_certs`), updates to SPEC-005/006/017, Plan phase 7 (7.1–7.6), TestCases TC-130–136 (planned).
