//! GUI-independent display logic: filter, state labels, grouping and text
//! for the detail panel (SPEC-010, SPEC-015). Kept free of egui so it can be
//! unit tested.

use vi_core::model::{Disk, DiskSource, Domain, DomainState, Graphics, OtherDevice, human_bytes};

/// State filter in the toolbar (SPEC-015).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StateFilter {
    #[default]
    All,
    Running,
    /// Shut off.
    Stopped,
    /// Paused, crashed, suspended and anything else.
    Other,
}

impl StateFilter {
    pub const ALL: [StateFilter; 4] = [
        StateFilter::All,
        StateFilter::Running,
        StateFilter::Stopped,
        StateFilter::Other,
    ];

    pub fn label(self) -> &'static str {
        match self {
            StateFilter::All => "all states",
            StateFilter::Running => "running",
            StateFilter::Stopped => "shut off",
            StateFilter::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filter {
    /// Case-insensitive substring of the VM name.
    pub text: String,
    pub state: StateFilter,
}

impl Filter {
    pub fn is_active(&self) -> bool {
        !self.text.trim().is_empty() || self.state != StateFilter::All
    }

    /// TC-080.
    pub fn matches(&self, d: &Domain) -> bool {
        let text = self.text.trim().to_lowercase();
        if !text.is_empty() && !d.name.to_lowercase().contains(&text) {
            return false;
        }
        match self.state {
            StateFilter::All => true,
            StateFilter::Running => d.state == DomainState::Running,
            StateFilter::Stopped => d.state == DomainState::ShutOff,
            StateFilter::Other => !matches!(d.state, DomainState::Running | DomainState::ShutOff),
        }
    }
}

/// Which palette colour a state uses. The text label is always shown too,
/// so the state never depends on colour alone (TC-082).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateColor {
    Running,
    Paused,
    Stopped,
    Error,
}

/// Text label and colour for a domain state.
pub fn state_badge(state: &DomainState) -> (String, StateColor) {
    let color = match state {
        DomainState::Running => StateColor::Running,
        DomainState::ShutOff => StateColor::Stopped,
        DomainState::Crashed => StateColor::Error,
        DomainState::Paused | DomainState::PmSuspended | DomainState::Other(_) => {
            StateColor::Paused
        }
    };
    (format!("[{}]", state.label()), color)
}

/// Group identical "other devices" with a count, keeping the first-seen order:
/// 15 × `controller pci pcie-root-port` becomes one row.
pub fn group_devices(devs: &[OtherDevice]) -> Vec<(&OtherDevice, usize)> {
    let mut out: Vec<(&OtherDevice, usize)> = Vec::new();
    for d in devs {
        match out.iter_mut().find(|(x, _)| *x == d) {
            Some((_, n)) => *n += 1,
            None => out.push((d, 1)),
        }
    }
    out
}

/// Size column for a disk.
pub fn disk_size_text(d: &Disk) -> String {
    if d.source == DiskSource::Empty {
        return "empty drive".into();
    }
    let Some(s) = d.size else {
        return "size unknown".into();
    };
    let b = |v: Option<u64>| v.map_or_else(|| "?".into(), human_bytes);
    let mut t = b(s.capacity);
    if s.allocation.is_some() || s.physical.is_some() {
        t.push_str(&format!(
            " (allocated {}, on disk {})",
            b(s.allocation),
            b(s.physical)
        ));
    }
    t
}

/// Memory in KiB as text.
pub fn kib_text(v: Option<u64>) -> String {
    v.map_or_else(|| "unknown".into(), |k| human_bytes(k.saturating_mul(1024)))
}

pub fn opt_text<T: std::fmt::Display>(v: &Option<T>) -> String {
    v.as_ref()
        .map_or_else(|| "unknown".into(), |v| v.to_string())
}

/// A vSphere template (SPEC-021).
pub fn is_template(d: &Domain) -> bool {
    d.details
        .as_ref()
        .and_then(|x| x.general.vsphere.as_ref())
        .is_some_and(|v| v.template)
}

pub fn yes_no(v: Option<bool>) -> &'static str {
    match v {
        Some(true) => "yes",
        Some(false) => "no",
        None => "unknown",
    }
}

/// Can the console of `g` be opened (SPEC-018)? `Err` is the reason shown
/// next to the disabled button. The worker checks again with live data.
pub fn console_availability(state: &DomainState, g: &Graphics) -> Result<(), String> {
    if !state.is_active() {
        return Err("VM not running".into());
    }
    g.console_target().map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use vi_core::model::BlockSize;

    fn dom(name: &str, state: DomainState) -> Domain {
        Domain::new(name, state)
    }

    // TC-080
    #[test]
    fn filter_by_name_and_state() {
        let ds = [
            dom("Web-01", DomainState::Running),
            dom("web-02", DomainState::ShutOff),
            dom("db", DomainState::Paused),
            dom("old", DomainState::Crashed),
        ];
        let names = |f: &Filter| -> Vec<&str> {
            ds.iter()
                .filter(|d| f.matches(d))
                .map(|d| d.name.as_str())
                .collect()
        };
        let mut f = Filter::default();
        assert!(!f.is_active());
        assert_eq!(names(&f).len(), 4);
        f.text = " WEB ".into();
        assert!(f.is_active());
        assert_eq!(names(&f), ["Web-01", "web-02"]);
        f.state = StateFilter::Stopped;
        assert_eq!(names(&f), ["web-02"]);
        f.text.clear();
        f.state = StateFilter::Other;
        assert_eq!(names(&f), ["db", "old"]);
        f.state = StateFilter::Running;
        assert_eq!(names(&f), ["Web-01"]);
    }

    // TC-082
    #[test]
    fn state_badges_have_text() {
        for s in [
            DomainState::Running,
            DomainState::Paused,
            DomainState::ShutOff,
            DomainState::Crashed,
            DomainState::PmSuspended,
            DomainState::Other("in shutdown".into()),
        ] {
            let (text, _) = state_badge(&s);
            assert!(text.len() > 2 && text.contains(s.label()), "{text}");
        }
        assert_eq!(state_badge(&DomainState::Crashed).1, StateColor::Error);
        assert_eq!(state_badge(&DomainState::ShutOff).1, StateColor::Stopped);
    }

    // TC-086: identical devices grouped, order kept; disk size wording.
    #[test]
    fn grouping_and_disk_text() {
        let d = |k: &str, s: &str| OtherDevice {
            kind: k.into(),
            summary: s.into(),
        };
        let devs = vec![
            d("controller", "usb qemu-xhci"),
            d("controller", "pci pcie-root-port"),
            d("controller", "pci pcie-root-port"),
            d("video", "virtio"),
            d("controller", "pci pcie-root-port"),
        ];
        let g = group_devices(&devs);
        assert_eq!(g.len(), 3);
        assert_eq!((g[1].0.summary.as_str(), g[1].1), ("pci pcie-root-port", 3));
        assert_eq!(g[2].0.kind, "video");

        let mut disk = Disk {
            device: "cdrom".into(),
            target: Some("sda".into()),
            bus: None,
            source: DiskSource::Empty,
            format: None,
            readonly: true,
            shareable: false,
            size: None,
        };
        assert_eq!(disk_size_text(&disk), "empty drive");
        disk.source = DiskSource::File("/x".into());
        assert_eq!(disk_size_text(&disk), "size unknown");
        disk.size = Some(BlockSize {
            capacity: Some(64 << 30),
            allocation: Some(1 << 30),
            physical: None,
        });
        assert_eq!(
            disk_size_text(&disk),
            "64.0 GiB (allocated 1.0 GiB, on disk ?)"
        );
        assert_eq!(kib_text(Some(1024)), "1.0 MiB");
        assert_eq!(kib_text(None), "unknown");
    }

    // TC-104: the console button is offered only for a running VM with a
    // reachable VNC/SPICE console; otherwise the reason is shown.
    #[test]
    fn console_button_availability() {
        use vi_core::model::Listen;
        let g = |listen: Listen, port: Option<u16>| Graphics {
            kind: "vnc".into(),
            listen,
            port,
            tls_port: None,
            autoport: true,
        };
        let ok = g(Listen::Address(Some("127.0.0.1".into())), Some(5900));
        assert_eq!(console_availability(&DomainState::Running, &ok), Ok(()));
        assert_eq!(console_availability(&DomainState::Paused, &ok), Ok(()));
        assert_eq!(
            console_availability(&DomainState::ShutOff, &ok),
            Err("VM not running".into())
        );
        let none = g(Listen::None, None);
        assert!(
            console_availability(&DomainState::Running, &none)
                .unwrap_err()
                .contains("not reachable")
        );
    }
}
