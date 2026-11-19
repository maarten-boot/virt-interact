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

## 4. Tree menus, SSH in a terminal, release pipeline (phase 7, v1.2)
**User:** Add that to the release pipeline. No draft left; files uploaded. Can we have right-click menus on the VM names in the left pane, with "open console", and on a host "start ssh to host"?

**Claude:** Asked: terminal (auto-detect and configurable; **user:** also configurable in the preferences), menu items (all offered: VM close console/copy name/copy UUID; host refresh/edit/remove/copy ssh command), release (1.2 before vSphere, vSphere becomes 1.3), pipeline (publish after checking the files). Spec: SPEC-024 (menus), SPEC-025 (SSH in a terminal), SPEC-002 release step; vSphere SPEC-021–023 and phase 8 moved to v1.3.

Done:
- 7.1 Menus: items as data (`vm_menu_items`, `host_menu_items`) drawn as egui context menus; disabled items show the reason on hover; right-click does not change the selection.
- 7.2 `vi-gui::terminal`: ssh arguments (only the host entry's values, `--` before the host), terminal choice (configured, else Linux list / Terminal.app via osascript / Windows Terminal or a new console), Settings field "Terminal (Start SSH)" with Apply and a one-line note of the terminal in use; `[terminal] command` in the config and example.
- 7.3 `scripts/publish_release.sh`: exactly six files and matching checksums, then publish as latest; tried against v1.0.0/v1.1.0 (publish stubbed) and a missing file (refused).
- Under Xvfb: both menus shown; "Start SSH" gave a configured stand-in terminal `-e ssh -l ops -- <host>`; Settings row shown.
- 152 tests pass; fmt and clippy clean. Open: TC-144 (real terminals), TC-123 (next tag).

**User:** Pushed.

**Claude:** CI run 44522791048 (`006c2d8`): all 4 jobs green; 152 tests on Linux, macOS and MSRV, 140 on Windows. TC-010 updated. Its archives are for TC-144 (Start SSH with real terminals).

**User:** Start SSH works on Linux.

**Claude:** TC-144: Linux pass; Windows open (macOS: no Mac).

**User:** Start SSH works on Windows.

**Claude:** TC-144: Windows pass too. Crosscheck of phase 7 and release 1.2.0 preparation follow (Plan 7.4).

## 5. Release 1.2.0 prepared
**Claude:** Crosscheck 2026-11-19-02: Spec, Plan and code match for phase 7; one open observation (O1: whether Windows Terminal is detected through its app execution alias). Version 1.2.0; `## 1.2.0 (2026-11-19)` in `Changes.md` (Unreleased kept), README status, Plan 7.4. 152 tests pass; `--version` 1.2.0. The tag `v1.2.0` is the first release published by the new check (TC-123).

## 6. Release 1.2.0 published by the pipeline
**User:** Pushed the commit and the tag.

**Claude:** CI run 44531403477 (`c984e3f`) passed on all 4 jobs. Release run 44531444697: packaged all platforms, checked the six files and three checksums, and published v1.2.0 as latest by itself (TC-123 pass; Plan 7.4 done). Releases now 3 (v1.0.0, v1.1.0, v1.2.0); CI artifacts: none older than 14 days, so no cleanup yet. The v1.1.0 release title ends with "." (from the hand-made release); can be fixed on GitHub.

## 7. Phase 8 started: 8.1 vSphere host config
**User:** Continue.

**Claude:** Started phase 8 (vSphere, 1.3). Design for the connection: the worker keeps one "connection" per host; for vSphere it will be an API session (implemented in 8.3), so the worker's flow stays the same. 8.1 done: `Backend::Vsphere` (`backend = "vsphere"`), port 443 by default, validation (user required; `identity_file`, `virsh_prefix`, non-default `libvirt_uri` refused; `password` refused as an unknown key), default `libvirt_uri` no longer written; host editor with a backend choice (rows stay, SSH/libvirt rows disabled for vSphere); no SSH menu items for vSphere hosts; reading a vSphere host says "not finished yet" until 8.3/8.4. 154 tests pass. Tools for the next steps: `go` is installed, so `vcsim` (govmomi's simulator) can be built for tests.

## 8. Phase 8.2: HTTPS and certificate trust
**Claude:** New crate `vi-vsphere` (licences checked: all on the accepted list; no aws-lc, no OpenSSL linking; rcgen/x509-parser only for tests). `tls.rs`: rustls with ring, TLS 1.2/1.3; a pinned fingerprint in `known_certs` decides alone (mismatch → "certificate CHANGED", refused, names the file); without a pin the system roots decide with the host name checked; a rejected certificate stops the handshake and comes back with subject, issuer, validity and SHA-256 for the user. `known_certs.rs`: `host:port SHA256:<hex>` lines, 0600 on Unix, replace per target. `http.rs`: hyper HTTP/1.1 POST `/sdk` over one kept connection, reconnect once when the server closed it, session cookie kept, 256 MB answer limit. Prompter gets `confirm_certificate` and `password` (default: no/cancel) with GUI dialogs (certificate details and fingerprint; password field, "never stored", retry note). Tests with a local TLS server (rcgen certificates): unknown → pinned → changed; CA-trusted with matching name, name mismatch asked; nothing listening; cookie round trip. 161 tests pass.

## 9. Phase 8.3: SOAP layer and session
**User:** Pushed 8.2.

**Claude:** Built vcsim (govmomi `3e19bc8`) in the scratchpad (`go install` refuses the module's replace directives; built from a clone). `soap.rs`: the allowlist as types (one constructor per allowed method; `post` takes only a `Request`; the fixed inventory query with traversal specs from the root folder: VMs, hosts, compute resources/clusters, datacenters, folders, vApps, networks, DV port groups); request bodies (and the HTTP body buffer) wiped on drop; SOAP faults parsed (`InvalidLogin`, `NotAuthenticated`). `session.rs`: `VsphereSession` (certificate question → pin, service content, login with up to three password questions, paged inventory, logout on drop) implements `CommandRunner` (runs nothing, forwards nothing) so the worker keeps it like an SSH session; `CommandRunner::as_any` lets the backend reach it. `BackendError::Vsphere` with kinds; an expired session counts as a lost connection (log in again once). `known_certs` next to the config file. Tests: TC-130; TC-133 against vcsim in ESXi and vCenter mode (local; CI job in 8.6). 166 tests pass. Spec SPEC-022 made precise (what is wiped; logout on quit is best effort; three password questions).

## 10. Phase 8.4: inventory mapping
**Claude:** CI run 44592417182 (`df45273`, 8.2) passed on all 4 jobs (161 tests; Windows 149): the system certificate stores load on every platform. `vi-vsphere::inventory` maps the paged answer: VMs sorted by name; state; `VsphereInfo` (datacenter by walking the parents, cluster from the host's parent when it is a cluster, ESXi host, hardware version, Tools, guest name, template); CPU/topology, memory; disks (datastore path, thin/thick, capacity, controller and unit), CD-ROM/floppy; NICs (port group, distributed port group by name, MAC, model, disconnected); other devices grouped by kind; the raw `<objects>` element as the raw view; a VM without readable configuration gives a warning, not an error. Bug found with vcsim and fixed: roxmltree's `attribute("type")` also matches `xsi:type`, so every value was read as an object reference; `soap::ref_type` takes only the plain attribute (unit-tested). `VsphereBackend` (vi-backend) reads through the session; `for_host` returns it. Fixtures from vcsim with MACs replaced by the documentation range. 172 tests pass (also with vcsim).

## 11. Phase 8.5: vSphere in the GUI
**User:** Pushed 8.3/8.4.

**Claude:** vSphere host page (backend, host, port, user, server name); VM page with datacenter, cluster, ESXi host, hardware version, VMware Tools, notes; ID/persistent/autostart "n/a"; CPU and memory without libvirt-only rows (mode, dies, threads, current memory, hugepages); the Graphics section says why there are no consoles; "Raw properties (XML)"; "(template)" in tree and heading; the menu's console item says "not available for vSphere". README section (example host entry, Read-only role, password never stored, certificates). End to end in the real app under Xvfb against vcsim (vCenter mode): certificate dialog (accepted → `known_certs`), a wrong password shows "Login failed" and asks again, the right one gives the inventory (4 VMs), VM page correct, cached inventory after a restart. Found and fixed: "→" in the certificate hint had no glyph in the font (boxes). New TC-137. 173 tests pass.

## 12. Phase 8.6 (part): CI job with vcsim
**Claude:** `vsphere-sim` job in `ci.yml`: builds vcsim from govmomi `3e19bc8` (Go 1.25), runs the vi-vsphere and vi-backend tests with `VI_VCSIM`, fails if any simulator test was skipped (`pipefail` set, since GitHub's bash has none by default). Checked locally both ways. Open in 8.6: TC-136 (the user's ESXi host and restricted vCenter), crosscheck.

**User:** Pushed 8.5/8.6.

**Claude:** CI run 44595228560 (`59ee007`) passed; run 44596936667 (`b3fd7e9`) passed on all 5 jobs: 173 tests (Windows 161) and the new `vsphere-sim` job (52 tests against vcsim, none skipped). TC-010 and TC-133 updated. Next: TC-136 by the user with that run's archives.

## 13. Old app, newer config file; Makefile
**User:** What do we do with older toml files? `config.toml:17:2: unknown field 'console', expected one of ui, ssh, logging, local, hosts. Using defaults; changes will not be saved.`

**Claude:** Not an old file: an old program (1.0.x, whose list has no `console`) reading a file saved by 1.1 or later, which writes `[console]`/`[terminal]` even when empty. 1.0.0 behaved safely (defaults, no save). Proposed: (1) write sections only when they hold a setting; (2) future versions accept unknown top-level sections with a warning and keep them on save (toml_edit). Waiting for the user's choice.

**User:** Always rebuild the binary at `target/release` in the repo; or add a top-level Makefile for `make run` during testing.

**Claude:** Added `Makefile` (release, run [ARGS=…], dev, test, test-vcsim VCSIM=…, lint, fmt, check) and a README note; the release binary is rebuilt in the repo's `target/release` after code changes (saved as a working preference).

## 14. TC-136: ESXi
**User:** ESXi works. The vCenter allows only its web SSO login page (SAML2) with a supported browser, so it cannot be tested.

**Claude:** TC-136: ESXi pass. That page is vCenter's standard login page for every vSphere Client login; the app logs in through the API (`/sdk`) with the same user and password, so it may still work: to be tried in the app (user written in full, e.g. `user@domain`). If the password is refused, the vCenter likely uses identity federation (external identity provider), whose accounts have no API password: then a known limitation for 1.3, and token-based login a separate later feature. The vCenter's host name is not recorded in the repo.

## 15. Config files between versions
**User:** Do both for 1.3.

**Claude:** No toml_edit needed (the app already rewrites the whole file on save): `Config` keeps unknown top-level entries in a `#[serde(flatten)]` table (one warning each; written back unchanged); the known sections and host entries stay strict (typos are errors with their line); `[console]`/`[terminal]` are skipped when they hold no setting. New TC-028b (also checks that a default file has only the five 1.0 sections). Spec SPEC-004, Changes.

## 16. TC-136: vCenter login
**User:** vCenter login works.

**Claude:** TC-136: vCenter login pass (the API login takes the same user and password as the SSO web page). Still to confirm for the restricted view: the VMs the user may see are listed and the host shows no error.

**User:** The vCenter shows my VMs, status ok, no warnings.

**Claude:** TC-136 pass (ESXi and restricted vCenter). Crosscheck 2026-11-19-03 of phase 8: C1 Spec says the refresh timeout covers the whole vSphere read, the code applies the command timeout per request; C2 Spec lists fewer CD-ROM/disk cases than the code shows; D1 `known_certs` location wording; D2 unwritable `known_certs` not specified; T1 no test for "expired session → log in again once"; T2 TC-135 (password never logged or saved) still without a test. Fixes proposed; waiting for the user's OK.
