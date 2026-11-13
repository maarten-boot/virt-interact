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
