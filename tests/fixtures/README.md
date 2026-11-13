# Fixtures

Captured output of read-only virsh commands, used by parser tests (Plan 3.5).
Suggested layout, one folder per host/libvirt version:

```
<libvirt-version>/
  version.txt          virsh version
  list-all.txt         virsh list --all
  <vm>/dominfo.txt     virsh dominfo <vm>
  <vm>/dumpxml.xml     virsh dumpxml <vm>   (running VMs: live XML)
  <vm>/domblkinfo-<target>.txt   virsh domblkinfo <vm> <target>
```

Wanted: running and stopped VMs, BIOS and UEFI, VNC and SPICE, a socket listen,
a network or volume disk. Remove or anonymise anything sensitive (hostnames,
paths, MAC addresses) before committing.

## Anonymisation (mandatory)

Never commit real VM names, hostnames, IP or MAC addresses, UUIDs or image paths.
Replace them with random data, consistently across all files of one capture
(the same VM keeps the same random name, UUID and MAC in `list-all`, `dominfo`,
`dumpxml` and security labels). Loopback `127.0.0.1` may stay.

## Current sets

| Folder | Source | Content |
|--------|--------|---------|
| `libvirt-12.0.0/` | live capture, QEMU 10.2.1 | 7 running VMs: 5 VNC, 2 SPICE, bridge NICs, `'` quoting, domblkinfo incl. allocation > capacity |
| `libvirt-10.0.0/` | live capture, QEMU 8.2.2 | 3 running VMs: VNC, SPICE, SPICE `listen none` (`domdisplay` gives "error: No graphical display found"); NAT network with `portid`/`virbr0`; `<title>`; no `Autostart Once` in dominfo; allocation > capacity |
| `ssh/` | generated with `ssh-keygen` | **test-only** SSH keys and an OpenSSH-hashed known_hosts; see [`ssh/README.md`](ssh/README.md) |
| `stopped/` | 3 persistent definitions | VNC autoport, SPICE `listen none` + GL, SPICE autoport; NAT and bridge; empty cdroms; `"` quoting |
