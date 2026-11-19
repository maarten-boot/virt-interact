# vSphere fixtures

Answers of the app's inventory query (`RetrievePropertiesEx`, see
`crates/vi-vsphere/src/soap.rs`) from vcsim, govmomi's vSphere simulator
(commit `3e19bc8`), used by `crates/vi-vsphere/tests/inventory.rs` (TC-132):

- `esx-inventory.xml`: `vcsim -esx` (a standalone ESXi host).
- `vcenter-inventory.xml`: `vcsim` (a vCenter: one datacenter, a cluster of
  three hosts, a standalone host, distributed port groups).

All names are the simulator's. MAC addresses were replaced with the
documentation range `00:00:5e:00:53:xx`. To refresh them, run the vcsim tests
with `VI_VCSIM=<vcsim binary> VI_DUMP=<dir>` and anonymise the dumped pages the
same way. Answers from real servers must have host names, VM names, paths,
UUIDs and MAC addresses replaced before they are committed.
