# Test cases: virt-interact

Each test case is traced to a Spec item.
- **Type:** U = unit test; I = integration test (mock runner, or the in-process SSH server in `crates/vi-ssh/tests/`); H = real-host test (manual with the `ssh-run` example, or automatic when `VIRT_INTERACT_TEST_HOST` is set); M = manual; CI = GitHub Actions run. Combinations such as `U/I` or `I/H` mean the case is covered at several levels.
- **Status:** `planned`, `implemented` (automated test exists), `pass` or `fail` (as of the last crosscheck).

## SPEC-001: Colour and contrast
| ID | Type | Description | Test location | Status |
|----|------|-------------|---------------|--------|
| TC-001 | U | The default palette is a white background (#FFFFFF) with black text (#000000). | `vi-core contrast::tests::default_palette_is_white_on_black` | pass (2026-11-13) |
| TC-002 | U | The WCAG contrast function gives known values: black/white = 21.0, #777/#FFF ≈ 4.48. | `vi-core contrast::tests::known_ratios` | pass (2026-11-13) |
| TC-003 | U | For each preset (AA, AAA, Max) and for custom ratios 5.0 and 10.0, every fg/bg pair in the palette is ≥ the minimum. | `vi-core contrast::tests::every_pair_meets_minimum_for_all_levels, max_is_pure_black_and_white` | pass (2026-11-13) |
| TC-004 | U | Config `contrast = 3.0` is clamped to 4.5 and logs a warning; `inf` → 21, `-inf` → 4.5, NaN → 4.5, each with a warning. | `vi-core contrast::tests::custom_below_minimum_is_clamped_with_warning; config::tests::low_contrast_in_file_is_clamped_with_warning` | pass (2026-11-13) |
| TC-005 | U | Config values `"AA"`, `"AAA"`, `"Max"` and `7.5` are parsed; an invalid string is an error. | `vi-core contrast::tests::raw_contrast_values; config::tests::contrast_values_in_file` | pass (2026-11-13) |
| TC-007 | U | Every fg/bg colour pair in the egui `Visuals` built from the palette meets the minimum for each level; `disabled_alpha` is 1.0 (no fading). | `vi-gui theme::tests::every_egui_pair_meets_minimum, default_visuals_white_and_black` | pass (2026-11-13) |
| TC-006 | M | Changing contrast in the toolbar takes effect at once and is saved to the config file. | — | planned (needs an interactive session) |

## SPEC-002 / SPEC-003: Platforms and GUI
| ID | Type | Description | Test location | Status |
|----|------|-------------|---------------|--------|
| TC-010 | CI | Build and tests pass on windows-latest, ubuntu-latest and macos-latest (aarch64), plus an x86_64-apple-darwin build and the MSRV job. | `.github/workflows/ci.yml` | pass: run 43509629083 (2026-11-13), all 4 jobs green; all 59 tests pass on Windows too |
| TC-011 | M | The GUI starts on all three platforms with an empty config. | — | Linux: pass under Xvfb (2026-11-13); Windows/macOS: open |

## SPEC-004 / SPEC-005: Config and hosts
| ID | Type | Description | Test location | Status |
|----|------|-------------|---------------|--------|
| TC-020 | U | A config with 2 hosts survives a round trip (load, save, load gives the same result). | `vi-core config::tests::round_trip_two_hosts, round_trip_custom_contrast` | pass (2026-11-13) |
| TC-021 | U | A missing file gives defaults; it is not created until the first save; the first save creates the missing directory and the file. | `vi-core config::tests::missing_file_gives_defaults_and_is_not_created` | pass (2026-11-13) |
| TC-022 | U | Invalid TOML gives an error with the line number, and the file is not modified. | `vi-core config::tests::invalid_toml_reports_line_and_is_not_overwritten, unknown_key_is_an_error_with_line` | pass (2026-11-13) |
| TC-023 | U | Defaults are applied: port 22, backend libvirt, URI `qemu:///system`. | `vi-core config::tests::host_defaults` | pass (2026-11-13) |
| TC-024 | U | A duplicate host `name` and port 0 are rejected. | `vi-core config::tests::duplicate_host_name_rejected, port_zero_rejected` | pass (2026-11-13) |
| TC-025 | U | The `--config` path override is used. | `vi-core paths::tests::config_override` | pass (2026-11-13) |
| TC-026 | U | `-h`/`--help` and `-V`/`--version` are recognised; `--config` without a value and unknown arguments are errors. | `vi-core paths::tests::bad_arguments` | pass (2026-11-13) |
| TC-027 | U | The default path is `<config_dir>/virt-interact/config.toml` (Windows: `.../virt-interact/config/config.toml`). | `vi-core paths::tests::default_path_ends_with_app_and_file` | pass (2026-11-13) |
| TC-028 | U | `examples/config.example.toml` and every TOML block in `README.md` parse; the example matches the documented values. | `vi-core config::tests::example_config_parses, readme_config_examples_parse` | pass (2026-11-13) |
| TC-028a | U | `examples/ssh_config.example` parses without warnings and resolves as its comments say. | `vi-ssh sshconfig::tests::example_ssh_config` | pass (2026-11-13) |

## SPEC-006 / SPEC-007 / SPEC-008: SSH
| ID | Type | Description | Test location | Status |
|----|------|-------------|---------------|--------|
| TC-030 | U | ssh config: `Host` wildcards (`*`, `?`, `!`), case-insensitive; first value wins; global options; quoting and `Key=Value`. | `vi-ssh sshconfig::tests::host_wildcards_and_first_value_wins, quoting_and_equals, missing_file_is_empty` | pass (2026-11-13) |
| TC-031 | U | Precedence: app config > ssh config > defaults; `IdentityFile` tokens (`%d`, `%h`, ...) and default keys when none given. | `vi-ssh sshconfig::tests::precedence_app_over_sshconfig_over_defaults, identity_tokens` | pass (2026-11-13) |
| TC-031a | U | `identity_file` with a leading `~` is expanded to the home directory; `~user`, absolute and relative paths are unchanged. | `vi-core config::tests::identity_file_tilde_expansion` | pass (2026-11-13) |
| TC-032 | U | `ProxyJump`, `Include` and `Match` give a warning, not an error. | `vi-ssh sshconfig::tests::unsupported_keywords_warn` | pass (2026-11-13) |
| TC-033 | U | known_hosts: plain entry, hashed entry (`\|1\|`, incl. lines hashed by OpenSSH `ssh-keygen -H`), `[host]:port`, patterns/negation, `@revoked`, bad lines skipped; mismatch refused without prompting; unknown host reported. | `vi-ssh known_hosts::tests::plain_entries, hashed_entries, patterns_markers_and_bad_lines, known_wins_over_mismatch_of_same_type, fingerprint_format`; `tests/session.rs::mismatched_host_key_is_refused` | pass (2026-11-13) |
| TC-033b | U | User and system known_hosts are combined like OpenSSH: revoked anywhere refuses; the first file that knows the host decides. | `vi-ssh session::tests::combine_user_and_system_known_hosts` | pass (2026-11-13) |
| TC-033c | I | A host known only in a system-wide known_hosts file connects without a prompt; the user's file is not written. | `tests/session.rs::system_known_hosts_is_used` | pass (2026-11-13) |
| TC-034 | U/I | An accepted new host key is added to known_hosts (hashed, mode 0600, newline handling) and matches afterwards; a rejected key aborts and records nothing. | `vi-ssh known_hosts::tests::learn_appends_and_then_matches`; `tests/session.rs::unknown_host_is_learned_and_exec_works, rejected_host_key` | pass (2026-11-13) |
| TC-034a | I | After the user accepts a key, a reconnect presenting a different key is refused with "changed between attempts". | `tests/session.rs::host_key_changed_between_attempts` | pass (2026-11-13) |
| TC-035 | U | The auth plan is agent first, then key files; it has no password or keyboard-interactive step. | `vi-ssh session::tests::auth_plan_is_agent_then_key_files_only` | pass (2026-11-13) |
| TC-035a | U | Agent keys matching a configured key file (`.pub`) are offered first; otherwise the agent order is kept. | `vi-ssh session::tests::agent_keys_matching_config_come_first` | pass (2026-11-13) |
| TC-036 | I/H | Connect and run a command: in-process test server (CI) and, manually, a real host with agent auth (`cargo run -p vi-ssh --example ssh-run -- HOST virsh version`). | `tests/session.rs::unknown_host_is_learned_and_exec_works`; real host: `ssh-run` | in-process: pass (2026-11-13); real host: open |
| TC-037 | I/H | Encrypted key file with passphrase works; a wrong passphrase gives an auth failure saying so. | `tests/session.rs::encrypted_key_with_passphrase`; real host: `ssh-run` | in-process: pass (2026-11-13); real host: open |
| TC-038 | M | Windows: agent auth through the OpenSSH agent pipe and through Pageant. | `ssh-run` example on Windows | open |
| TC-039 | U/I | Passphrases never appear in Debug/Display output or in log output at trace level. | `vi-ssh secret::tests::never_printed`; `tests/logging.rs::passphrase_not_logged` | pass (2026-11-13) |
| TC-039a | I | Auth failure lists every key tried with the reason; RSA keys authenticate (rsa-sha2). | `tests/session.rs::auth_failure_lists_tried_keys, rsa_key_works` | pass (2026-11-13) |
| TC-039b | I | When the server closes the connection after too many attempts, the auth error still lists the keys tried, plus a note. | `tests/session.rs::server_closing_during_auth_keeps_tried_list` | pass (2026-11-13) |

## SPEC-009 / SPEC-010: Inventory
| ID | Type | Description | Test location | Status |
|----|------|-------------|---------------|--------|
| TC-040 | U | Parse `virsh list --all` fixture output: running and shut-off domains, names with spaces. | `vi-backend tests/fixtures.rs::list_fixtures`; `parse::tests::list_with_spaces_and_states, list_edge_cases` | pass (2026-11-13) |
| TC-040a | U | virsh state strings map to the model (`shut off`, `in shutdown` → other); sizes are shown in binary units. | `vi-core model::tests::states_from_virsh, human_byte_units` | pass (2026-11-13) |
| TC-041 | U | Parse `dominfo` fixture: state, persistent, autostart, UUID. Unknown keys (`Autostart Once`, `Security *`) are ignored without an error; works for both libvirt 10.0.0 and 12.0.0 fixtures. | `vi-backend tests/fixtures.rs::dominfo_fixtures_both_versions`; `parse::tests::dominfo_values` | pass (2026-11-13) |
| TC-042 | U | Parse the dumpxml fixture of a BIOS VM with 3 disks, an empty cdrom, 1 NAT NIC and VNC graphics; memory units are converted to KiB. | `vi-backend tests/fixtures.rs::bios_vm_components`; `domxml::tests::memory_units` | pass (2026-11-13) |
| TC-043 | U | Parse the XML of a UEFI VM (`firmware="efi"` or a pflash loader) with SPICE socket listen, a network listen with live address, and a CPU topology. Synthetic until a UEFI capture exists. | `vi-backend domxml::tests::uefi_spice_socket_topology` (synthetic XML: no UEFI capture yet) | pass (2026-11-13) |
| TC-044 | U | Parse a network disk (rbd/iscsi) and a volume disk source. | `vi-backend domxml::tests::network_volume_block_disks` (synthetic XML) | pass (2026-11-13) |
| TC-045 | U | Unknown XML elements are ignored without an error. | `vi-backend domxml::tests::unknown_elements_ignored_bad_xml_rejected` | pass (2026-11-13) |
| TC-045a | U | Other devices get a one-line summary: hostdev PCI address or USB vendor:product, filesystem source → target, channel name, video model. | `vi-backend domxml::tests::other_device_summaries` | pass (2026-11-13) |
| TC-046 | U | Merge `domblkinfo` sizes into the disks; a missing size gives "unknown", not an error; allocation > capacity is kept as reported. | `vi-backend tests/fixtures.rs::full_inventory_libvirt_12, running_and_stopped_only_allowlisted_commands`; `parse::tests::blkinfo_missing_values` | pass (2026-11-13) |
| TC-046a | U | Guest OS from libosinfo (`ubuntu/24.04`, `win/10`, `fedora/unknown`); an empty cdrom is shown as "empty". | `vi-backend tests/fixtures.rs::guest_os_title_and_listen_none`; `vi-core model::tests::guest_os_short_names` | pass (2026-11-13) |
| TC-046b | U | All fixtures in `tests/fixtures/` (live `'` and stopped `"` quoting) parse without errors. | `vi-backend tests/fixtures.rs::all_fixture_xml_parses` | pass (2026-11-13) |
| TC-047 | I | The mock runner gives a full host inventory from fixtures, including both running and stopped VMs. | `vi-backend tests/fixtures.rs::full_inventory_libvirt_12, running_and_stopped_only_allowlisted_commands` | pass (2026-11-13) |
| TC-048 | H | The inventory of the test host matches `virsh list --all` run on that host; with the read-only connection (`-r`) dominfo, dumpxml and domblkinfo all work. | `cargo run -p vi-backend --example inventory -- <host>` | planned (manual, user) |

## SPEC-011: Read-only guarantee
| ID | Type | Description | Test location | Status |
|----|------|-------------|---------------|--------|
| TC-050 | U | The command builder only produces allowlisted virsh subcommands; there is no API for any other command. | `vi-backend command::tests::only_allowlisted_subcommands` | pass (2026-11-13) |
| TC-051 | U | Domain names containing `'`, `;`, `$()` and spaces are quoted safely. | `vi-backend command::tests::hostile_names_are_quoted, quoting_round_trips_through_sh` (sh check on unix) | pass (2026-11-13) |
| TC-052 | I | During a full refresh, the mock runner records only allowlisted commands. | `vi-backend tests/fixtures.rs::running_and_stopped_only_allowlisted_commands` | pass (2026-11-13) |
| TC-053 | U | `virsh_prefix` is prepended exactly as configured, and is validated: only letters, digits, spaces and `_ - . / = : @ +` (so no `;`, `&`, `\|`, `$`, quotes, redirections); checked at config load and in the command builder. | `vi-backend command::tests::prefix_prepended_exactly_and_validated`; `vi-core config::tests::virsh_prefix_validated` | pass (2026-11-13) |
| TC-054 | U | The configured backend is built from a host entry (`libvirt` by default); an invalid `virsh_prefix` gives a config error. | `vi-backend tests::backend_from_host_config` | pass (2026-11-13) |

## SPEC-012 / SPEC-013: Refresh and errors
| ID | Type | Description | Test location | Status |
|----|------|-------------|---------------|--------|
| TC-060 | I | Of 3 hosts, one hangs (mock delay). The other two finish; the hanging one times out. | _tbd_ | planned |
| TC-061 | U/I | Error classification from the stderr of: virsh not found (also behind `sudo`), not executable (exit 126, no libvirt-group hint), permission denied (socket, polkit), sudo needs a password, hypervisor not running; messages carry hints. Through the backend: host-level failures abort, a vanished domain is a warning, an XML error or other per-domain failure is recorded on that domain. | `vi-backend error::tests::classify_stderr, messages_have_hints`; `tests/fixtures.rs::error_handling` | pass (2026-11-13) |
| TC-061a | I | SSH errors are classified: connection refused → connection, command timeout → timeout (session stays usable), host key, authentication; exit status and stderr are returned. | `tests/session.rs::connection_refused, exit_status_stderr_and_timeout` | pass (2026-11-13) |
| TC-062 | I | Status transitions: idle → connecting → refreshing → ok/error. | _tbd_ | planned |
| TC-063 | U | `ConnectOptions::from_settings` takes the `[ssh]` timeouts from the config; defaults are 10 s / 30 s. | `vi-ssh session::tests::options_from_config_settings` | pass (2026-11-13) |
| TC-064 | I | A hanging `domblkinfo` makes only that disk's size unknown (with a warning); a hanging `dumpxml` is shown on that domain; the other domains are complete; a hanging `virsh version` fails the host with a timeout. | `vi-backend tests/fixtures.rs::timeouts_stay_per_domain` | pass (2026-11-13) |
| TC-065 | I | Domains of one host are queried in parallel (8 domains with a 1 s delay: about 2 s with 4 at a time, 8 s with 1); results keep the list order. | `vi-backend tests/fixtures.rs::domains_are_queried_in_parallel` | pass (2026-11-13) |

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
| TC-083 | U | The log file is created in the platform log directory at the configured level. | `vi-core logging::tests::writes_log_file_at_level, other_crates_only_warn` | pass (2026-11-13) |
| TC-084 | U | An invalid `logging.level` is a config error. | `vi-core config::tests::invalid_log_level_rejected` | pass (2026-11-13) |

## SPEC-017: Backend abstraction
| ID | Type | Description | Test location | Status |
|----|------|-------------|---------------|--------|
| TC-090 | U | The GUI depends only on the `HypervisorBackend` trait. A dummy backend compiles and is listed. | _tbd_ | planned |
| TC-091 | U | `MockRunner` returns canned output, gives exit 127 for unknown commands, and records every command. | `vi-ssh runner::tests::mock_runner_records_and_answers` | pass (2026-11-13) |

## SPEC-018: Remote viewing (v1.1)
| ID | Type | Description | Test location | Status |
|----|------|-------------|---------------|--------|
| TC-100 | U | Builds the viewer URL from graphics info (vnc and spice, TCP and socket). Fallback: the VNC display number from `domdisplay` (`:1` = 5901); the SPICE port is used as is. | _tbd_ | planned |
| TC-101 | U | The local listener binds to 127.0.0.1 only. | _tbd_ | planned |
| TC-102 | H | A tunnel to the test VM's VNC port gives an RFB handshake. | _tbd_ | planned |
| TC-103 | M | remote-viewer opens the console; the tunnel closes when the viewer exits. | — | planned |
| TC-104 | U | Graphics with `listen type="none"` give "not reachable remotely", and no tunnel is offered; the `domdisplay` error "No graphical display found" is not reported as a host error (fixture libvirt-10.0.0). | _tbd_ | planned |
