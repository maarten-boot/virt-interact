# Test cases: virt-interact

Each test case is traced to a Spec item.
- **Type:** U = unit test, I = integration test with the mock runner, H = real-host test (only runs when `VIRT_INTERACT_TEST_HOST` is set), M = manual.
- **Status:** `planned`, `implemented` (automated test exists), `pass` or `fail` (as of the last crosscheck).

## SPEC-001: Colour and contrast
| ID | Type | Description | Test location | Status |
|----|------|-------------|---------------|--------|
| TC-001 | U | The default palette is a white background (#FFFFFF) with black text (#000000). | _tbd_ | planned |
| TC-002 | U | The WCAG contrast function gives known values: black/white = 21.0, #777/#FFF ≈ 4.48. | _tbd_ | planned |
| TC-003 | U | For each preset (AA, AAA, Max) and for custom ratios 5.0 and 10.0, every fg/bg pair in the palette is ≥ the minimum. | _tbd_ | planned |
| TC-004 | U | Config `contrast = 3.0` is clamped to 4.5 and logs a warning. | _tbd_ | planned |
| TC-005 | U | Config values `"AA"`, `"AAA"`, `"Max"` and `7.5` are parsed; an invalid string is an error. | _tbd_ | planned |
| TC-006 | M | Changing contrast in the settings takes effect at once and is saved to the config file. | — | planned |

## SPEC-002 / SPEC-003: Platforms and GUI
| ID | Type | Description | Test location | Status |
|----|------|-------------|---------------|--------|
| TC-010 | CI | Build and tests pass on windows-latest, ubuntu-latest and macos-latest. | `.github/workflows/ci.yml` | implemented (not yet run) |
| TC-011 | M | The GUI starts on all three platforms with an empty config. | — | planned |

## SPEC-004 / SPEC-005: Config and hosts
| ID | Type | Description | Test location | Status |
|----|------|-------------|---------------|--------|
| TC-020 | U | A config with 2 hosts survives a round trip (load, save, load gives the same result). | _tbd_ | planned |
| TC-021 | U | A missing file gives defaults; it is not created until the first save. | _tbd_ | planned |
| TC-022 | U | Invalid TOML gives an error with the line number, and the file is not modified. | _tbd_ | planned |
| TC-023 | U | Defaults are applied: port 22, backend libvirt, URI `qemu:///system`. | _tbd_ | planned |
| TC-024 | U | A duplicate host `name` is rejected. | _tbd_ | planned |
| TC-025 | U | The `--config` path override is used. | _tbd_ | planned |

## SPEC-006 / SPEC-007 / SPEC-008: SSH
| ID | Type | Description | Test location | Status |
|----|------|-------------|---------------|--------|
| TC-030 | U | ssh config: `Host` wildcard matching; the first value found wins for each keyword. | _tbd_ | planned |
| TC-031 | U | Precedence: app config > ssh config > defaults. | _tbd_ | planned |
| TC-032 | U | A `ProxyJump` keyword gives a warning, not an error. | _tbd_ | planned |
| TC-033 | U | known_hosts: matches a plain entry, matches a hashed entry (`\|1\|`), detects a mismatch, reports an unknown host. | _tbd_ | planned |
| TC-034 | U | An accepted new host key is added to known_hosts in the correct format. | _tbd_ | planned |
| TC-035 | U | The auth method list contains no password or keyboard-interactive method. | _tbd_ | planned |
| TC-036 | H | Connects with agent auth to the test host and runs `virsh version`. | _tbd_ | planned |
| TC-037 | H | Connects with an encrypted key file and a passphrase. | _tbd_ | planned |
| TC-038 | M | Windows: agent auth through the OpenSSH agent and through Pageant. | — | planned |
| TC-039 | U | Passphrases do not appear in log output (log capture test). | _tbd_ | planned |

## SPEC-009 / SPEC-010: Inventory
| ID | Type | Description | Test location | Status |
|----|------|-------------|---------------|--------|
| TC-040 | U | Parse `virsh list --all` fixture output: running and shut-off domains, names with spaces. | _tbd_ | planned |
| TC-041 | U | Parse `dominfo` fixture: state, persistent, autostart, UUID. Unknown keys (`Autostart Once`, `Security *`) are ignored without an error; works for both libvirt 10.0.0 and 12.0.0 fixtures. | _tbd_ | planned |
| TC-042 | U | Parse the dumpxml fixture of a BIOS VM with 2 disks, a cdrom, 1 NIC and VNC graphics. | _tbd_ | planned |
| TC-043 | U | Parse the dumpxml fixture of a UEFI VM with SPICE graphics, socket listen and a CPU topology. | _tbd_ | planned |
| TC-044 | U | Parse a network disk (rbd/iscsi) and a volume disk source. | _tbd_ | planned |
| TC-045 | U | Unknown XML elements are ignored without an error. | _tbd_ | planned |
| TC-046 | U | Merge `domblkinfo` sizes into the disks; a missing size gives "unknown", not an error; allocation > capacity is kept as reported. | _tbd_ | planned |
| TC-046a | U | Guest OS from libosinfo (`ubuntu/24.04`, `win/10`, `fedora/unknown`); an empty cdrom is shown as "empty". | _tbd_ | planned |
| TC-046b | U | All fixtures in `tests/fixtures/` (live `'` and stopped `"` quoting) parse without errors. | _tbd_ | planned |
| TC-047 | I | The mock runner gives a full host inventory from fixtures, including both running and stopped VMs. | _tbd_ | planned |
| TC-048 | H | The inventory of the test host matches `virsh list --all` run on that host. | _tbd_ | planned |

## SPEC-011: Read-only guarantee
| ID | Type | Description | Test location | Status |
|----|------|-------------|---------------|--------|
| TC-050 | U | The command builder only produces allowlisted virsh subcommands; there is no API for any other command. | _tbd_ | planned |
| TC-051 | U | Domain names containing `'`, `;`, `$()` and spaces are quoted safely. | _tbd_ | planned |
| TC-052 | I | During a full refresh, the mock runner records only allowlisted commands. | _tbd_ | planned |
| TC-053 | U | `virsh_prefix` is prepended exactly as configured, and is validated (no `;`, `&`, `\|`). | _tbd_ | planned |

## SPEC-012 / SPEC-013: Refresh and errors
| ID | Type | Description | Test location | Status |
|----|------|-------------|---------------|--------|
| TC-060 | I | Of 3 hosts, one hangs (mock delay). The other two finish; the hanging one times out. | _tbd_ | planned |
| TC-061 | U | Error classification from the stderr of: virsh not found, permission denied, connection refused. | _tbd_ | planned |
| TC-062 | I | Status transitions: idle → connecting → refreshing → ok/error. | _tbd_ | planned |

## SPEC-014: Cache
| ID | Type | Description | Test location | Status |
|----|------|-------------|---------------|--------|
| TC-070 | U | The cache round-trips through JSON and keeps its timestamp. | _tbd_ | planned |
| TC-071 | I | When a host is unreachable, the cached inventory is shown with the cached flag set. | _tbd_ | planned |
| TC-072 | U | Clearing the cache removes the files. | _tbd_ | planned |

## SPEC-015 / SPEC-016: GUI and logging
| ID | Type | Description | Test location | Status |
|----|------|-------------|---------------|--------|
| TC-080 | U | The filter by name substring and state gives the expected VM set. | _tbd_ | planned |
| TC-081 | M | The tree, detail panel, raw XML copy and toolbar actions work. | — | planned |
| TC-082 | U | The state indicator has a text or icon label, not only colour. | _tbd_ | planned |
| TC-083 | U | The log file is created in the platform log directory at the configured level. | _tbd_ | planned |

## SPEC-017: Backend abstraction
| ID | Type | Description | Test location | Status |
|----|------|-------------|---------------|--------|
| TC-090 | U | The GUI depends only on the `HypervisorBackend` trait. A dummy backend compiles and is listed. | _tbd_ | planned |

## SPEC-018: Remote viewing (v1.1)
| ID | Type | Description | Test location | Status |
|----|------|-------------|---------------|--------|
| TC-100 | U | Builds the viewer URL from graphics info (vnc and spice, TCP and socket). Fallback: the VNC display number from `domdisplay` (`:1` = 5901); the SPICE port is used as is. | _tbd_ | planned |
| TC-101 | U | The local listener binds to 127.0.0.1 only. | _tbd_ | planned |
| TC-102 | H | A tunnel to the test VM's VNC port gives an RFB handshake. | _tbd_ | planned |
| TC-103 | M | remote-viewer opens the console; the tunnel closes when the viewer exits. | — | planned |
| TC-104 | U | Graphics with `listen type="none"` give "not reachable remotely", and no tunnel is offered; the `domdisplay` error "No graphical display found" is not reported as a host error (fixture libvirt-10.0.0). | _tbd_ | planned |
