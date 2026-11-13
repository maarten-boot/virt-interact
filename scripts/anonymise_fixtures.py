#!/usr/bin/env python3
"""Anonymise a virsh capture before it goes into tests/fixtures/.

Input is a folder in the fixture layout, as written by
`cargo run -p vi-backend --example inventory -- --save DIR HOST`:

    version.txt, list-all.txt, <vm>/dominfo.txt, <vm>/dumpxml.xml,
    <vm>/domblkinfo-<target>.txt, <vm>/domdisplay.txt

Stopped-domain XMLs (a folder holding only <vm>/dumpxml.xml) work too.

Replaced with random or neutral data, consistently across all files:
VM names (also the 20-character prefix libvirt uses in run paths), UUIDs,
MAC addresses, IPv4 addresses other than 127.0.0.1 and 0.0.0.0, titles and
descriptions, file/block/dir source paths, network-disk host names and
host names in domdisplay URIs.

Not replaced: network and bridge names, and anything inside <metadata>
other than libosinfo. Pass words that must not appear with --leak; the
script fails if any original value or --leak word is left. Always review
the result by eye before committing (CLAUDE.md, "Privacy of test data").

usage: anonymise_fixtures.py SRC DST [--leak WORD ...]
"""

import argparse
import os
import re
import secrets
import string
import sys
import uuid

STATES = ["in shutdown", "shut off", "no state", "running", "idle", "paused", "crashed", "pmsuspended"]
rnd = secrets.SystemRandom()


class Mapper:
    def __init__(self):
        self.names, self.uuids, self.macs, self.ips, self.paths, self.hosts = {}, {}, {}, {}, {}, {}
        self.counters = {}

    def name(self, old):
        if old not in self.names:
            while True:
                n = "vm-" + "".join(rnd.choice(string.ascii_lowercase + string.digits) for _ in range(6))
                if n not in self.names.values():
                    break
            self.names[old] = n
        return self.names[old]

    def uuid(self, m):
        return self.uuids.setdefault(m.group(0).lower(), str(uuid.uuid4()))

    def mac(self, m):
        old = m.group(0).lower()
        if old not in self.macs:
            self.macs[old] = "52:54:00:%02x:%02x:%02x" % tuple(rnd.randrange(256) for _ in range(3))
        return self.macs[old]

    def ip(self, m):
        old = m.group(0)
        if old in ("127.0.0.1", "0.0.0.0"):
            return old
        if old not in self.ips:
            self.ips[old] = "192.0.2.%d" % (len(self.ips) + 10)  # TEST-NET-1
        return self.ips[old]

    def path(self, old, vm, kind):
        if old not in self.paths:
            n = self.counters.get((vm, kind), 0)
            self.counters[(vm, kind)] = n + 1
            if kind == "file":
                ext = os.path.splitext(old)[1] or ".img"
                self.paths[old] = f"/var/lib/libvirt/images/{vm}-disk{n}{ext}"
            elif kind == "dev":
                self.paths[old] = f"/dev/disk/by-id/{vm}-vol{n}"
            else:
                self.paths[old] = f"/srv/{vm}-share{n}"
        return self.paths[old]

    def host(self, old):
        if old in ("127.0.0.1", "0.0.0.0", "localhost") or re.fullmatch(r"192\.0\.2\.\d+", old):
            return old
        return self.hosts.setdefault(old, "host%d.example.invalid" % (len(self.hosts) + 1))


def scrub(text, vm, mp):
    text = re.sub(r"\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b", mp.uuid, text)
    text = re.sub(r"\b[0-9a-fA-F]{2}(?::[0-9a-fA-F]{2}){5}\b", mp.mac, text)
    text = re.sub(r"\b\d{1,3}(?:\.\d{1,3}){3}\b", mp.ip, text)
    text = re.sub(r"<title>.*?</title>", f"<title>{vm} title</title>", text, flags=re.S)
    text = re.sub(r"<description>.*?</description>", f"<description>{vm} description</description>", text, flags=re.S)
    # Disk/filesystem sources: <source file=..>, <source dev=..>, <source dir=..>
    def src(m):
        tag = m.group(0)
        for kind in ("file", "dev", "dir"):
            tag = re.sub(rf"""(\b{kind}=)(['"])(.*?)\2""",
                         lambda a: f"{a.group(1)}{a.group(2)}{mp.path(a.group(3), vm, kind)}{a.group(2)}"
                         if not a.group(3).startswith(("/dev/pts/", "/dev/urandom", "/dev/random")) else a.group(0),
                         tag)
        return tag
    text = re.sub(r"<source\b[^>]*>", src, text)
    # Network disk hosts
    text = re.sub(r"""(<host\b[^>]*\bname=)(['"])(.*?)\2""",
                  lambda m: f"{m.group(1)}{m.group(2)}{mp.host(m.group(3))}{m.group(2)}", text)
    # domdisplay URIs: vnc://HOST:N
    text = re.sub(r"\b(vnc|spice|rdp)://([^:/\s]+)",
                  lambda m: f"{m.group(1)}://{mp.host(m.group(2))}", text)
    # Names, longest first; libvirt also uses the first 20 characters in paths.
    for old in sorted(mp.names, key=len, reverse=True):
        text = text.replace(old, mp.names[old])
        if len(old) > 20:
            text = text.replace(old[:20], mp.names[old][:20])
    return text


def split_row(row):
    """' 1    my vm    shut off' -> ('1', 'my vm', 'shut off')"""
    i, rest = row.strip().split(None, 1)
    rest = rest.strip()
    for s in STATES:
        if rest.endswith(" " + s):
            return i, rest[: -len(s)].strip(), s
    n, s = rest.rsplit(None, 1)
    return i, n.strip(), s


def list_table(rows):
    """Rebuild `virsh list --all` with virsh's column layout."""
    w = max([4] + [len(n) for _, n, _ in rows]) + 3
    out = [" " + "Id".ljust(5) + "Name".ljust(w) + "State", "-" * (6 + w + 5 + 4)]
    out += [" " + i.ljust(5) + n.ljust(w) + s for i, n, s in rows]
    return "\n".join(out) + "\n\n"


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("src")
    ap.add_argument("dst")
    ap.add_argument("--leak", action="append", default=[], help="word that must not remain (repeatable)")
    a = ap.parse_args()
    if os.path.exists(a.dst):
        sys.exit(f"{a.dst} already exists")

    mp = Mapper()
    vms = sorted(d for d in os.listdir(a.src) if os.path.isdir(os.path.join(a.src, d)))
    for d in vms:
        mp.name(d)
    # Names from XML too (folder names should match, but be safe).
    for d in vms:
        x = os.path.join(a.src, d, "dumpxml.xml")
        if os.path.exists(x):
            m = re.search(r"<name>([^<]+)</name>", open(x, encoding="utf-8").read())
            if m and m.group(1) != d:
                mp.names[m.group(1)] = mp.names[d]

    os.makedirs(a.dst)
    for f in ("version.txt",):
        p = os.path.join(a.src, f)
        if os.path.exists(p):
            open(os.path.join(a.dst, f), "w", encoding="utf-8").write(open(p, encoding="utf-8").read())
    lst = os.path.join(a.src, "list-all.txt")
    if os.path.exists(lst):
        lines = open(lst, encoding="utf-8").read().splitlines()
        sep = next(i for i, l in enumerate(lines) if l.strip() and set(l.strip()) == {"-"})
        rows = [split_row(l) for l in lines[sep + 1:] if l.strip()]
        rows = [(i, mp.name(n), s) for i, n, s in rows]
        open(os.path.join(a.dst, "list-all.txt"), "w", encoding="utf-8").write(list_table(rows))

    for d in vms:
        new = mp.names[d]
        os.makedirs(os.path.join(a.dst, new))
        for f in sorted(os.listdir(os.path.join(a.src, d))):
            t = open(os.path.join(a.src, d, f), encoding="utf-8").read()
            open(os.path.join(a.dst, new, f), "w", encoding="utf-8").write(scrub(t, new, mp))

    # Leak check: no original value may remain anywhere.
    leaks = (list(mp.names) + list(mp.uuids) + list(mp.macs) + list(mp.ips)
             + list(mp.paths) + list(mp.hosts) + a.leak)
    bad = []
    for root, _, files in os.walk(a.dst):
        for f in files:
            t = open(os.path.join(root, f), encoding="utf-8").read().lower()
            bad += [(os.path.join(root, f), w) for w in leaks if w and w.lower() in t]
    print(f"vms {len(mp.names)}, uuids {len(mp.uuids)}, macs {len(mp.macs)}, ips {len(mp.ips)}, "
          f"paths {len(mp.paths)}, hosts {len(mp.hosts)}")
    if bad:
        for f, w in bad:
            print(f"LEAK: {w!r} in {f}")
        sys.exit(1)
    print("leak check: none found. Review the files by eye before committing.")


if __name__ == "__main__":
    main()
