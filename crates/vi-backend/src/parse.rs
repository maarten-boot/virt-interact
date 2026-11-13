//! Parsers for the text output of virsh (`LC_ALL=C`): `version`,
//! `list --all`, `dominfo`, `domblkinfo` (SPEC-009, SPEC-010).
//! Unknown lines and keys are ignored.

use vi_core::model::{BlockSize, DomainState};

/// `virsh version`: (`"libvirt 12.0.0"`, `"QEMU 10.2.1"`).
pub fn version(text: &str) -> (Option<String>, Option<String>) {
    let mut library = None;
    let mut hypervisor = None;
    for (k, v) in key_values(text) {
        match k {
            "Using library" => library = Some(v.to_string()),
            "Running hypervisor" => hypervisor = Some(v.to_string()),
            _ => {}
        }
    }
    (library, hypervisor)
}

/// One row of `virsh list --all`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListEntry {
    pub id: Option<u32>,
    pub name: String,
    pub state: DomainState,
}

/// Multi-word states first, so "shut off" is not read as name "... shut" + "off".
const STATES: [&str; 8] = [
    "in shutdown",
    "shut off",
    "no state",
    "running",
    "idle",
    "paused",
    "crashed",
    "pmsuspended",
];

/// `virsh list --all` (TC-040). Rows are `Id Name State`; the Id is `-` for
/// inactive domains. Names may contain spaces; the state is matched at the
/// end of the row. Errors if the header separator is missing.
pub fn list(text: &str) -> Result<Vec<ListEntry>, String> {
    let mut lines = text.lines();
    if !lines.any(|l| {
        let t = l.trim();
        !t.is_empty() && t.chars().all(|c| c == '-')
    }) {
        return Err("no table header found".into());
    }
    let mut out = Vec::new();
    for line in lines {
        let row = line.trim();
        if row.is_empty() {
            continue;
        }
        let (id, rest) = row
            .split_once(char::is_whitespace)
            .ok_or_else(|| format!("unexpected row: {row:?}"))?;
        let id = if id == "-" {
            None
        } else {
            Some(id.parse().map_err(|_| format!("bad id in row: {row:?}"))?)
        };
        let rest = rest.trim();
        let (name, state) = match STATES.iter().find(|s| {
            rest.strip_suffix(*s)
                .is_some_and(|n| n.ends_with(char::is_whitespace))
        }) {
            Some(s) => (rest[..rest.len() - s.len()].trim_end(), *s),
            None => rest
                .rsplit_once(char::is_whitespace)
                .map(|(n, s)| (n.trim_end(), s))
                .ok_or_else(|| format!("unexpected row: {row:?}"))?,
        };
        out.push(ListEntry {
            id,
            name: name.to_string(),
            state: DomainState::from_virsh(state),
        });
    }
    Ok(out)
}

/// Fields of `virsh dominfo` used by the inventory (TC-041).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DomInfo {
    pub id: Option<u32>,
    pub name: Option<String>,
    pub uuid: Option<String>,
    pub state: Option<DomainState>,
    pub persistent: Option<bool>,
    pub autostart: Option<bool>,
}

/// `virsh dominfo`: generic `key: value` lines; unknown keys are ignored.
pub fn dominfo(text: &str) -> DomInfo {
    let mut d = DomInfo::default();
    for (k, v) in key_values(text) {
        match k {
            "Id" => d.id = v.parse().ok(),
            "Name" => d.name = Some(v.to_string()),
            "UUID" => d.uuid = Some(v.to_string()),
            "State" => d.state = Some(DomainState::from_virsh(v)),
            "Persistent" => d.persistent = yes_no(v),
            "Autostart" => d.autostart = yes_no(v),
            _ => {}
        }
    }
    d
}

/// `virsh domblkinfo <dom> <target>` in bytes (TC-046).
pub fn domblkinfo(text: &str) -> BlockSize {
    let mut b = BlockSize::default();
    for (k, v) in key_values(text) {
        match k {
            "Capacity" => b.capacity = v.parse().ok(),
            "Allocation" => b.allocation = v.parse().ok(),
            "Physical" => b.physical = v.parse().ok(),
            _ => {}
        }
    }
    b
}

fn yes_no(v: &str) -> Option<bool> {
    match v {
        "yes" | "enable" => Some(true),
        "no" | "disable" => Some(false),
        _ => None,
    }
}

/// `Key: value` lines, split at the first `:` (keys have no colons;
/// values keep theirs).
fn key_values(text: &str) -> impl Iterator<Item = (&str, &str)> {
    text.lines().filter_map(|l| {
        let (k, v) = l.split_once(':')?;
        Some((k.trim(), v.trim()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_with_spaces_and_states() {
        let text = " Id   Name            State\n\
                    ---------------------------------\n \
                    1    web 01          running\n \
                    -    old box         shut off\n \
                    12   x               paused\n \
                    -    weird           in shutdown\n\n";
        let l = list(text).unwrap();
        assert_eq!(l.len(), 4);
        assert_eq!(
            l[0],
            ListEntry {
                id: Some(1),
                name: "web 01".into(),
                state: DomainState::Running
            }
        );
        assert_eq!(l[1].id, None);
        assert_eq!(l[1].name, "old box");
        assert_eq!(l[1].state, DomainState::ShutOff);
        assert_eq!(l[2].state, DomainState::Paused);
        assert_eq!(l[3].state, DomainState::Other("in shutdown".into()));
    }

    #[test]
    fn list_edge_cases() {
        assert_eq!(
            list(" Id   Name   State\n--------------\n\n").unwrap(),
            vec![]
        );
        assert!(list("error: something\n").is_err());
        // A name ending in a state word is still split at the last state.
        let l = list("--\n -   shut off shut off\n").unwrap();
        assert_eq!(l[0].name, "shut off");
    }

    #[test]
    fn dominfo_values() {
        let d = dominfo(
            "Id:             -\nName:           a: b\nState:          shut off\nPersistent:     no\nAutostart:      enable\nAutostart Once: disable\n",
        );
        assert_eq!(d.id, None);
        assert_eq!(d.name.as_deref(), Some("a: b"));
        assert_eq!(d.state, Some(DomainState::ShutOff));
        assert_eq!(d.persistent, Some(false));
        assert_eq!(d.autostart, Some(true));
    }

    #[test]
    fn blkinfo_missing_values() {
        let b = domblkinfo("Capacity:       100\nPhysical:       x\n");
        assert_eq!(b.capacity, Some(100));
        assert_eq!(b.allocation, None);
        assert_eq!(b.physical, None);
    }
}
