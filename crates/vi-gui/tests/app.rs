//! App-level tests without a display (Plan 4.7): state changes through the
//! `App` methods, and every view drawn headless with `Context::run_ui`.

use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

use eframe::egui;
use vi_backend::{BackendError, BackendErrorKind};
use vi_core::cache::{Cache, CachedInventory};
use vi_core::config::{ConfigStore, HostConfig, SshSettings};
use vi_core::contrast::ContrastLevel;
use vi_core::model::{Domain, DomainState, HostInventory};
use vi_gui::app::{App, HostForm, Selection};
use vi_gui::worker::{Connector, HostStatus, Worker};
use vi_ssh::prompt::BoxFuture;
use vi_ssh::{CommandRunner, NoPrompt, Prompter, SshError};

fn fixture_inventory() -> HostInventory {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
    let mut domains = Vec::new();
    for (set, vm, state) in [
        ("libvirt-10.0.0", "vm-j2sunl", DomainState::Running),
        ("libvirt-10.0.0", "vm-rcqh1f", DomainState::Running),
        ("stopped", "vm-wn1018", DomainState::ShutOff),
    ] {
        let xml = std::fs::read_to_string(root.join(set).join(vm).join("dumpxml.xml")).unwrap();
        let mut d = Domain::new(vm, state);
        d.details = Some(vi_backend::domxml::parse(&xml).unwrap());
        domains.push(d);
    }
    let mut broken = Domain::new("broken", DomainState::Paused);
    broken.error = Some("XML parse error: test".into());
    domains.push(broken);
    HostInventory {
        library: Some("libvirt 10.0.0".into()),
        hypervisor: Some("QEMU 8.2.2".into()),
        domains,
        warnings: vec!["a warning".into()],
    }
}

/// Config with hosts `lab` and `edge` in a temp dir, plus a cache dir.
fn setup(dir: &Path) -> (ConfigStore, Cache) {
    let path = dir.join("config.toml");
    std::fs::write(
        &path,
        "[[hosts]]\nname = \"lab\"\nhost = \"lab.example.lan\"\n[[hosts]]\nname = \"edge\"\nhost = \"edge.example.lan\"\n",
    )
    .unwrap();
    let (store, warnings, err) = ConfigStore::open(path);
    assert!(warnings.is_empty() && err.is_none());
    (store, Cache::new(dir.join("cache")))
}

/// Every host refuses the connection.
struct Down;
impl Connector for Down {
    fn connect<'a>(
        &'a self,
        h: &'a HostConfig,
        _: &'a SshSettings,
        _: &'a dyn Prompter,
    ) -> BoxFuture<'a, Result<Arc<dyn CommandRunner>, BackendError>> {
        Box::pin(async move {
            Err(BackendError::Ssh(SshError::Connect {
                addr: format!("{}:22", h.host),
                source: std::io::Error::from(std::io::ErrorKind::ConnectionRefused),
            }))
        })
    }
}

fn frame(app: &mut App, ctx: &egui::Context) {
    let _ = ctx.run_ui(egui::RawInput::default(), |ui| app.show(ui));
}

// TC-006 (automated part): a contrast change is applied and saved.
#[test]
fn contrast_change_is_saved() {
    let dir = tempfile::tempdir().unwrap();
    let (store, cache) = setup(dir.path());
    let path = store.path.clone();
    let mut app = App::new(store, vec![], Some(cache));
    let ctx = egui::Context::default();
    app.set_contrast(&ctx, ContrastLevel::Aaa);
    assert!(ctx.global_style().visuals.disabled_alpha == 1.0);
    assert!(app.palette.worst_ratio() >= 7.0);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("contrast = \"AAA\""), "{text}");
    // Hosts survive the save.
    assert!(text.contains("edge.example.lan"));
}

// TC-087: every view draws without a display: empty selection, host,
// each VM (running, stopped, broken), dialogs and settings.
#[test]
fn all_views_render_headless() {
    let dir = tempfile::tempdir().unwrap();
    let (store, cache) = setup(dir.path());
    cache
        .save(&CachedInventory::new(
            "lab",
            1_700_000_000,
            fixture_inventory(),
        ))
        .unwrap();
    let mut app = App::new(store, vec![], Some(cache));
    let ctx = egui::Context::default();
    app.apply_theme(&ctx);
    frame(&mut app, &ctx);
    app.selection = Selection::Host("lab".into());
    frame(&mut app, &ctx);
    let names: Vec<String> = app.hosts["lab"]
        .shown
        .as_ref()
        .unwrap()
        .inventory
        .domains
        .iter()
        .map(|d| d.name.clone())
        .collect();
    for n in names {
        app.selection = Selection::Domain("lab".into(), n);
        frame(&mut app, &ctx);
    }
    app.selection = Selection::Host("edge".into());
    frame(&mut app, &ctx);
    app.settings_open = true;
    app.host_form = Some(HostForm::new_host());
    app.confirm_remove = Some("edge".into());
    frame(&mut app, &ctx);
    app.filter.text = "j2s".into();
    frame(&mut app, &ctx);
}

// TC-071 + TC-014 behaviour: cached inventory is shown at start, marked
// cached; a failed refresh keeps it, still marked.
#[test]
fn cached_inventory_shown_when_host_unreachable() {
    let dir = tempfile::tempdir().unwrap();
    let (store, cache) = setup(dir.path());
    cache
        .save(&CachedInventory::new(
            "lab",
            1_700_000_000,
            fixture_inventory(),
        ))
        .unwrap();
    let (worker, events) = Worker::new(Arc::new(Down), Arc::new(NoPrompt), Arc::new(|| {}));
    let (_ptx, prx) = channel();
    let mut app = App::new(store, vec![], Some(cache)).with_worker(worker, events, prx);
    {
        let s = app.hosts["lab"].shown.as_ref().unwrap();
        assert!(s.cached);
        assert_eq!(s.taken_at, 1_700_000_000);
        assert!(app.hosts["edge"].shown.is_none());
    }
    app.refresh("lab");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        app.poll();
        if let HostStatus::Error(e) = &app.hosts["lab"].status {
            assert_eq!(e.kind, BackendErrorKind::Connection);
            break;
        }
        assert!(Instant::now() < deadline, "no error status");
        std::thread::sleep(Duration::from_millis(10));
    }
    let s = app.hosts["lab"].shown.as_ref().unwrap();
    assert!(s.cached);
    assert_eq!(s.inventory.domains.len(), 4);

    // Clearing the cache removes cached inventories from view (TC-072).
    app.clear_cache();
    assert!(app.hosts["lab"].shown.is_none());
}

// TC-029: add, edit (rename) and remove hosts through the form; invalid
// input keeps the form open with a message and changes nothing.
#[test]
fn host_form_add_edit_remove() {
    let dir = tempfile::tempdir().unwrap();
    let (store, cache) = setup(dir.path());
    let path = store.path.clone();
    let mut app = App::new(store, vec![], Some(cache));

    let mut f = HostForm::new_host();
    app.host_form = Some(f.clone());
    assert!(!app.submit_host_form());
    assert_eq!(
        app.host_form.as_ref().unwrap().error.as_deref(),
        Some("Name is required.")
    );
    f.name = "lab".into(); // duplicate
    f.host = "x".into();
    app.host_form = Some(f);
    assert!(!app.submit_host_form());
    assert!(
        app.host_form
            .as_ref()
            .unwrap()
            .error
            .as_deref()
            .unwrap()
            .contains("duplicate")
    );

    let f = app.host_form.as_mut().unwrap();
    f.name = "new".into();
    f.port = "70000".into();
    assert!(!app.submit_host_form());
    assert!(
        app.host_form
            .as_ref()
            .unwrap()
            .error
            .as_deref()
            .unwrap()
            .contains("port")
    );

    let f = app.host_form.as_mut().unwrap();
    f.port = "2222".into();
    f.virsh_prefix = "sudo; reboot".into();
    assert!(!app.submit_host_form());
    assert!(
        app.host_form
            .as_ref()
            .unwrap()
            .error
            .as_deref()
            .unwrap()
            .contains("virsh_prefix")
    );
    assert_eq!(app.store.config.hosts.len(), 2, "nothing changed");

    app.host_form.as_mut().unwrap().virsh_prefix = "sudo -n".into();
    assert!(app.submit_host_form());
    assert!(app.host_form.is_none());
    assert_eq!(app.selection, Selection::Host("new".into()));
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        text.contains("name = \"new\"") && text.contains("port = 2222"),
        "{text}"
    );
    assert!(app.hosts.contains_key("new"));

    let cfg = app
        .store
        .config
        .hosts
        .iter()
        .find(|h| h.name == "new")
        .unwrap()
        .clone();
    let mut f = HostForm::edit(&cfg);
    f.name = "renamed".into();
    f.port.clear();
    app.host_form = Some(f);
    assert!(app.submit_host_form());
    assert!(app.hosts.contains_key("renamed") && !app.hosts.contains_key("new"));
    let h = app
        .store
        .config
        .hosts
        .iter()
        .find(|h| h.name == "renamed")
        .unwrap();
    assert_eq!(h.port, None);
    assert_eq!(h.virsh_prefix.as_deref(), Some("sudo -n"));

    app.remove_host("renamed");
    assert!(!app.hosts.contains_key("renamed"));
    assert_eq!(app.selection, Selection::None);
    assert!(!std::fs::read_to_string(&path).unwrap().contains("renamed"));
}

// TC-029a (Plan 4.6): reload picks up hosts added in the file; an invalid
// file is reported and the current config stays.
#[test]
fn reload_config() {
    let dir = tempfile::tempdir().unwrap();
    let (store, cache) = setup(dir.path());
    let path = store.path.clone();
    cache
        .save(&CachedInventory::new("edge", 1, fixture_inventory()))
        .unwrap();
    let mut app = App::new(store, vec![], Some(cache.clone()));
    let ctx = egui::Context::default();
    let mut text = std::fs::read_to_string(&path).unwrap();
    text.push_str("[[hosts]]\nname = \"third\"\nhost = \"t.example.lan\"\n");
    std::fs::write(&path, &text).unwrap();
    app.reload_config(&ctx);
    assert!(app.hosts.contains_key("third"));
    assert_eq!(app.store.config.hosts.len(), 3);

    // A host removed in the file loses its cache file on reload (C5; now
    // done by sync_hosts for every entry that goes, crosscheck 02 C3).
    assert!(cache.load("edge").is_some());
    let text = text.replace("name = \"edge\"\nhost = \"edge.example.lan\"\n", "");
    let text = text.replacen("[[hosts]]\n[[hosts]]", "[[hosts]]", 1);
    std::fs::write(&path, &text).unwrap();
    app.reload_config(&ctx);
    assert!(!app.hosts.contains_key("edge"), "{text}");
    assert!(cache.load("edge").is_none());

    std::fs::write(&path, "[[hosts]\nbroken").unwrap();
    app.reload_config(&ctx);
    assert_eq!(app.store.config.hosts.len(), 2);
    assert!(app.messages.last().unwrap().is_error);
}

// TC-029 (T2): when the config cannot be saved (the file was invalid at
// start, so the store is read-only), the change is reverted and reported.
#[test]
fn host_form_save_failure_reverts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[[hosts]\nbroken").unwrap();
    let (store, _, err) = ConfigStore::open(path.clone());
    assert!(err.is_some());
    let mut app = App::new(store, vec![], None);
    let mut f = HostForm::new_host();
    f.name = "a".into();
    f.host = "a.example.lan".into();
    app.host_form = Some(f);
    assert!(!app.submit_host_form());
    assert!(app.store.config.hosts.is_empty());
    assert!(app.messages.last().unwrap().is_error);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "[[hosts]\nbroken");
}

// TC-088: keyboard navigation follows the tree order, respects the filter
// and collapsed hosts, and clamps at both ends.
#[test]
fn keyboard_navigation_order() {
    let dir = tempfile::tempdir().unwrap();
    let (store, cache) = setup(dir.path());
    cache
        .save(&CachedInventory::new("lab", 1, fixture_inventory()))
        .unwrap();
    let mut app = App::new(store, vec![], Some(cache));
    let all_open = |_: &str| true;
    let items = app.visible_items(all_open);
    let names: Vec<String> = items
        .iter()
        .map(|s| match s {
            Selection::Host(h) => format!("H:{h}"),
            Selection::Domain(_, d) => d.clone(),
            Selection::None => unreachable!(),
        })
        .collect();
    assert_eq!(
        names,
        [
            "H:lab",
            "vm-j2sunl",
            "vm-rcqh1f",
            "vm-wn1018",
            "broken",
            "H:edge"
        ]
    );
    app.move_selection(1, all_open);
    assert_eq!(app.selection, Selection::Host("lab".into()));
    app.move_selection(1, all_open);
    assert_eq!(
        app.selection,
        Selection::Domain("lab".into(), "vm-j2sunl".into())
    );
    app.move_selection(i32::MAX, all_open);
    assert_eq!(app.selection, Selection::Host("edge".into()));
    app.move_selection(1, all_open);
    assert_eq!(app.selection, Selection::Host("edge".into()), "clamped");
    app.move_selection(-1, all_open);
    assert_eq!(
        app.selection,
        Selection::Domain("lab".into(), "broken".into())
    );
    // Collapsed host: its VMs are skipped.
    app.selection = Selection::Host("edge".into());
    app.move_selection(-1, |h| h != "lab");
    assert_eq!(app.selection, Selection::Host("lab".into()));
    // Filter.
    app.filter.text = "rcq".into();
    assert_eq!(app.visible_items(all_open).len(), 3);
    // Up with nothing selected goes to the last row.
    app.selection = Selection::None;
    app.move_selection(-1, all_open);
    assert_eq!(app.selection, Selection::Host("edge".into()));
}

// TC-007a (C2): disabled buttons use the palette's disabled colour, which is
// distinct from the normal text and meets the minimum on every background.
#[test]
fn disabled_buttons_use_disabled_colour() {
    use vi_core::contrast::{Palette, Rgb, contrast_ratio};
    for level in [ContrastLevel::Aa, ContrastLevel::Aaa] {
        let p = Palette::for_level(level);
        let off = vi_gui::app::button_color(&p, false);
        let on = vi_gui::app::button_color(&p, true);
        assert_ne!(off, on, "{level:?}");
        let rgb = Rgb(off.r(), off.g(), off.b());
        assert_eq!(rgb, p.disabled_text);
        for bg in p.backgrounds() {
            assert!(contrast_ratio(rgb, bg) >= p.min_ratio - 1e-9);
        }
    }
}

// TC-089: with `refresh_on_start`, the first frame starts a refresh of
// every host; without it, nothing connects.
#[test]
fn refresh_on_start() {
    for on in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let (mut store, cache) = setup(dir.path());
        store.config.ui.refresh_on_start = on;
        let (worker, events) = Worker::new(Arc::new(Down), Arc::new(NoPrompt), Arc::new(|| {}));
        let (_ptx, prx) = channel();
        let mut app = App::new(store, vec![], Some(cache)).with_worker(worker, events, prx);
        let ctx = egui::Context::default();
        frame(&mut app, &ctx);
        let idle = app.hosts.values().all(|e| e.status == HostStatus::Idle);
        assert_eq!(idle, !on, "refresh_on_start = {on}");
    }
}

// TC-114: local entries (SPEC-020) appear first when `virsh` is found
// ("auto"), always with "always", never with "never"; they are not
// editable and render headless.
#[test]
fn local_entries() {
    use vi_core::config::{LOCAL_SESSION, LOCAL_SYSTEM, LocalMode};
    let dir = tempfile::tempdir().unwrap();
    let (store, cache) = setup(dir.path());
    let path = store.path.clone();
    let mut app = App::new(store, vec![], Some(cache.clone()));
    assert!(
        !app.hosts.contains_key(LOCAL_SYSTEM),
        "auto, virsh not detected"
    );

    app.set_local_virsh(Some("/usr/bin/virsh".into()));
    let first: Vec<_> = app.all_hosts().map(|h| h.name.clone()).collect();
    assert_eq!(first, [LOCAL_SYSTEM, LOCAL_SESSION, "lab", "edge"]);
    assert_eq!(app.display_name(LOCAL_SYSTEM), "localhost (system)");
    assert_eq!(app.display_name(LOCAL_SESSION), "localhost (session)");
    assert!(app.hosts.contains_key(LOCAL_SESSION));

    let ctx = egui::Context::default();
    for name in [LOCAL_SYSTEM, LOCAL_SESSION] {
        app.selection = Selection::Host(name.into());
        frame(&mut app, &ctx);
    }
    // Saving the config never writes the local entries.
    app.set_contrast(&ctx, ContrastLevel::Aaa);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains("@local"), "{text}");

    // "never" hides them even with virsh; "always" shows them without.
    // An entry that disappears loses its cache file (crosscheck 02, C3).
    cache
        .save(&CachedInventory::new(LOCAL_SESSION, 1, fixture_inventory()))
        .unwrap();
    app.store.config.local.enabled = LocalMode::Never;
    app.set_local_virsh(Some("/usr/bin/virsh".into()));
    assert!(!app.hosts.contains_key(LOCAL_SYSTEM));
    assert!(cache.load(LOCAL_SESSION).is_none());
    assert_eq!(
        app.selection,
        Selection::None,
        "selection of a vanished entry cleared"
    );
    app.store.config.local.enabled = LocalMode::Always;
    app.store.config.local.session = false;
    app.set_local_virsh(None);
    assert!(app.hosts.contains_key(LOCAL_SYSTEM));
    assert!(!app.hosts.contains_key(LOCAL_SESSION));
}

/// Width of the tree panel after one frame with `input`.
fn tree_width(app: &mut App, ctx: &egui::Context, input: egui::RawInput) -> f32 {
    let _ = ctx.run_ui(input, |ui| app.show(ui));
    egui::containers::panel::PanelState::load(ctx, egui::Id::new(vi_gui::app::TREE_PANEL))
        .expect("tree panel state")
        .size()
        .x
}

// TC-007b (H1/H2): moving the pointer over every row of the tree, pressing
// on rows, a status text that changes width, and a VM name wider than the
// panel all leave the tree panel's width (the separator) where it was.
#[test]
fn tree_width_is_stable() {
    let dir = tempfile::tempdir().unwrap();
    let (store, cache) = setup(dir.path());
    let mut inv = fixture_inventory();
    inv.domains.push(Domain::new(
        "a-very-long-virtual-machine-name-that-is-wider-than-the-tree-panel",
        DomainState::Running,
    ));
    cache.save(&CachedInventory::new("lab", 1, inv)).unwrap();
    let mut app = App::new(store, vec![], Some(cache));
    let ctx = egui::Context::default();
    app.apply_theme(&ctx);
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1024.0, 700.0));
    let input = |events: Vec<egui::Event>| egui::RawInput {
        screen_rect: Some(screen),
        events,
        ..Default::default()
    };
    let w0 = tree_width(&mut app, &ctx, input(vec![]));
    let w0 = tree_width(&mut app, &ctx, input(vec![])).max(w0);
    assert!(w0 < 400.0, "the long name must not widen the panel: {w0}");

    let mut y = 40.0;
    while y < 400.0 {
        for x in [20.0, 60.0, 120.0, 200.0] {
            let p = egui::pos2(x, y);
            let w = tree_width(&mut app, &ctx, input(vec![egui::Event::PointerMoved(p)]));
            assert_eq!(w, w0, "hover at {p:?}");
        }
        let p = egui::pos2(40.0, y);
        let press = |pressed| egui::Event::PointerButton {
            pos: p,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        let w = tree_width(
            &mut app,
            &ctx,
            input(vec![egui::Event::PointerMoved(p), press(true)]),
        );
        assert_eq!(w, w0, "press at {p:?}");
        let w = tree_width(&mut app, &ctx, input(vec![press(false)]));
        assert_eq!(w, w0, "release at {p:?}");
        y += 9.0;
    }

    for status in [
        HostStatus::Connecting,
        HostStatus::Refreshing,
        HostStatus::Ok(1),
        HostStatus::Idle,
    ] {
        app.hosts.get_mut("lab").unwrap().status = status.clone();
        app.hosts.get_mut("edge").unwrap().status = status.clone();
        let w = tree_width(&mut app, &ctx, input(vec![]));
        assert_eq!(w, w0, "status {status:?}");
    }
}

// TC-103 (app part): console events change what the graphics table shows
// (every state renders headless); a failure becomes an error message that
// names the VM; without a worker, opening says so.
#[test]
fn console_states_in_the_app() {
    use vi_gui::console::{ConsoleKey, ConsoleState};
    use vi_gui::worker::Event;
    let dir = tempfile::tempdir().unwrap();
    let (store, cache) = setup(dir.path());
    cache
        .save(&CachedInventory::new(
            "lab",
            1_700_000_000,
            fixture_inventory(),
        ))
        .unwrap();
    let mut app = App::new(store, vec![], Some(cache));
    let ctx = egui::Context::default();
    app.apply_theme(&ctx);
    app.selection = Selection::Domain("lab".into(), "vm-j2sunl".into());
    let key = ConsoleKey {
        host: "lab".into(),
        domain: "vm-j2sunl".into(),
        index: 0,
    };
    frame(&mut app, &ctx);

    app.open_console(key.clone());
    assert!(app.messages.last().unwrap().text.contains("not available"));
    app.messages.clear();

    for state in [
        ConsoleState::Opening,
        ConsoleState::Open {
            url: "vnc://127.0.0.1:40000".into(),
        },
    ] {
        app.handle(Event::Console {
            key: key.clone(),
            state: state.clone(),
        });
        assert_eq!(app.consoles.get(&key), Some(&state));
        frame(&mut app, &ctx);
    }
    // Close: "closing…" until the worker reports Closed.
    app.close_console(&key);
    assert_eq!(app.consoles.get(&key), Some(&ConsoleState::Closing));
    frame(&mut app, &ctx);
    app.handle(Event::Console {
        key: key.clone(),
        state: ConsoleState::Closed {
            error: Some("the viewer exited with exit status: 1".into()),
        },
    });
    assert!(app.consoles.is_empty());
    let m = &app.messages.last().unwrap().text;
    assert!(
        m.contains("vm-j2sunl") && m.contains("exit status: 1"),
        "{m}"
    );
    frame(&mut app, &ctx);

    // A clean close leaves no message.
    app.messages.clear();
    app.handle(Event::Console {
        key: key.clone(),
        state: ConsoleState::Closed { error: None },
    });
    assert!(app.messages.is_empty());
}

// TC-140: the VM menu: one console item per VNC/SPICE device with the same
// rules as the Graphics table, Close while open, copy name/UUID; drawn
// headless with the menu open.
#[test]
fn vm_menu() {
    use vi_gui::app::TreeAction;
    use vi_gui::console::{ConsoleKey, ConsoleState};
    let dir = tempfile::tempdir().unwrap();
    let (store, cache) = setup(dir.path());
    let mut inv = fixture_inventory();
    inv.domains[0].uuid = Some("00000000-0000-4000-8000-000000000001".into());
    cache
        .save(&CachedInventory::new("lab", 1_700_000_000, inv.clone()))
        .unwrap();
    let mut app = App::new(store, vec![], Some(cache));
    let labels = |groups: &[Vec<vi_gui::app::MenuItem>]| -> Vec<String> {
        groups.iter().flatten().map(|i| i.label.clone()).collect()
    };

    // Running VM with VNC on 127.0.0.1:5900.
    let vm = &inv.domains[0];
    let key = ConsoleKey {
        host: "lab".into(),
        domain: vm.name.clone(),
        index: 0,
    };
    let g = app.vm_menu_items("lab", vm);
    assert_eq!(labels(&g), ["Open console (VNC)", "Copy name", "Copy UUID"]);
    assert_eq!(g[0][0].action, Ok(TreeAction::OpenConsole(key.clone())));
    assert_eq!(g[1][0].action, Ok(TreeAction::CopyText(vm.name.clone())));
    assert_eq!(
        g[1][1].action,
        Ok(TreeAction::CopyText(
            "00000000-0000-4000-8000-000000000001".into()
        ))
    );
    app.consoles.insert(
        key.clone(),
        ConsoleState::Open {
            url: "vnc://127.0.0.1:40000".into(),
        },
    );
    let g = app.vm_menu_items("lab", vm);
    assert_eq!(g[0][0].label, "Close console (VNC)");
    assert_eq!(g[0][0].action, Ok(TreeAction::CloseConsole(key.clone())));
    app.consoles.insert(key.clone(), ConsoleState::Closing);
    assert_eq!(
        app.vm_menu_items("lab", vm)[0][0].action,
        Err("closing…".into())
    );

    // Stopped VM: disabled with the reason; no details: refresh first;
    // unknown UUID: disabled.
    let stopped = &inv.domains[2];
    let g = app.vm_menu_items("lab", stopped);
    assert_eq!(g[0][0].label, "Open console (SPICE)");
    assert_eq!(g[0][0].action, Err("VM not running".into()));
    assert_eq!(g[1][1].action, Err("UUID unknown".into()));
    let bare = Domain::new("bare", DomainState::Running);
    let g = app.vm_menu_items("lab", &bare);
    assert!(matches!(&g[0][0].action, Err(e) if e.contains("refresh the host")));

    // Two VNC displays are numbered.
    let mut two = inv.domains[0].clone();
    let x = two.details.as_mut().unwrap();
    let extra = x.graphics[0].clone();
    x.graphics.push(extra);
    assert_eq!(
        labels(&app.vm_menu_items("lab", &two))[..2],
        ["Open console (VNC 1)", "Open console (VNC 2)"]
    );
}

// TC-141: the host menu; Edit/Remove disabled and no SSH items for the
// local entries; Copy ssh command; Refresh disabled while busy.
#[test]
fn host_menu() {
    use vi_gui::app::TreeAction;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        "[local]\nenabled = \"always\"\nsession = false\n[[hosts]]\nname = \"lab\"\nhost = \"lab.example.lan\"\nuser = \"ops\"\nport = 2222\n",
    )
    .unwrap();
    let (store, _, err) = ConfigStore::open(path);
    assert!(err.is_none());
    let mut app = App::new(store, vec![], None);
    let ctx = egui::Context::default();
    let g = app.host_menu_items("lab");
    let labels: Vec<_> = g.iter().flatten().map(|i| i.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "Refresh",
            "Edit…",
            "Remove…",
            "Start SSH",
            "Copy ssh command"
        ]
    );
    assert_eq!(
        g[1][1].action,
        Ok(TreeAction::CopyText(
            "ssh -p 2222 -l ops -- lab.example.lan".into()
        ))
    );
    // Edit opens the host form; Remove asks.
    app.apply_tree_action(&ctx, TreeAction::Edit("lab".into()));
    assert!(app.host_form.is_some());
    app.host_form = None;
    app.apply_tree_action(&ctx, TreeAction::Remove("lab".into()));
    assert_eq!(app.confirm_remove.as_deref(), Some("lab"));
    app.confirm_remove = None;

    let local = app
        .all_hosts()
        .find(|h| h.local)
        .map(|h| h.name.clone())
        .expect("a local entry");
    let g = app.host_menu_items(&local);
    assert_eq!(g.len(), 1, "no SSH items for this machine");
    assert!(g[0][0].action.is_ok());
    assert!(g[0][1].action.is_err() && g[0][2].action.is_err());

    app.hosts.get_mut("lab").unwrap().status = HostStatus::Refreshing;
    assert_eq!(
        app.host_menu_items("lab")[0][0].action,
        Err("being read".into())
    );
}

// TC-143 (Settings part): the terminal text is saved as a list; bad
// quoting is not saved and says why; empty means automatic.
#[test]
fn terminal_setting() {
    let dir = tempfile::tempdir().unwrap();
    let (store, cache) = setup(dir.path());
    let mut app = App::new(store, vec![], Some(cache));
    let ctx = egui::Context::default();
    app.apply_theme(&ctx);
    app.settings_open = true;
    app.load_terminal_setting();
    assert_eq!(app.terminal_text, "");
    app.terminal_text = "'/My Apps/term' -e".into();
    app.apply_terminal_setting();
    assert_eq!(
        app.store.config.terminal.command,
        Some(vec!["/My Apps/term".to_string(), "-e".to_string()])
    );
    assert!(
        app.terminal_note.contains("configured"),
        "{}",
        app.terminal_note
    );
    let saved = std::fs::read_to_string(&app.store.path).unwrap();
    assert!(saved.contains("[terminal]"), "{saved}");
    frame(&mut app, &ctx);
    app.terminal_text = "'open".into();
    app.apply_terminal_setting();
    assert!(app.terminal_note.starts_with("Not saved"));
    assert!(app.store.config.terminal.command.is_some());
    app.terminal_text = String::new();
    app.apply_terminal_setting();
    assert_eq!(app.store.config.terminal.command, None);
    frame(&mut app, &ctx);
}

// TC-131 (form part): a vSphere host from the form needs a user; the SSH
// and libvirt fields are not saved for it; no SSH items in its menu.
#[test]
fn vsphere_host_form() {
    use vi_core::config::Backend;
    let mut f = HostForm::new_host();
    f.backend = Backend::Vsphere;
    f.name = "esx".into();
    f.host = "esx.example.lan".into();
    f.identity_file = "~/.ssh/id_ed25519".into();
    f.virsh_prefix = "sudo -n".into();
    assert_eq!(
        f.to_host(None).unwrap_err(),
        "User is required for vSphere."
    );
    f.user = "reader".into();
    let h = f.to_host(None).unwrap();
    assert_eq!(h.backend, Backend::Vsphere);
    assert_eq!(h.identity_file, None);
    assert_eq!(h.virsh_prefix, None);
    assert_eq!(h.port_or_default(), 443);
    assert_eq!(HostForm::edit(&h).backend, Backend::Vsphere);

    let dir = tempfile::tempdir().unwrap();
    let (store, cache) = setup(dir.path());
    let mut app = App::new(store, vec![], Some(cache));
    app.host_form = Some(f);
    app.submit_host_form();
    assert!(app.host_form.is_none(), "saved");
    let g = app.host_menu_items("esx");
    assert_eq!(g.len(), 1, "no SSH items for vSphere");
    // The form draws with the vSphere fields disabled.
    let ctx = egui::Context::default();
    app.apply_theme(&ctx);
    let esx = app.all_hosts().find(|h| h.name == "esx").unwrap().clone();
    app.host_form = Some(HostForm::edit(&esx));
    frame(&mut app, &ctx);
}

// TC-137: a vSphere host in the GUI (SPEC-021): host and VM pages draw with
// the vSphere rows; libvirt-only fields say n/a; no console, with the
// reason in the menu; templates are marked.
#[test]
fn vsphere_host_in_the_gui() {
    use vi_gui::app::TreeAction;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        "[[hosts]]\nname = \"vc\"\nhost = \"vc.example.lan\"\nbackend = \"vsphere\"\nuser = \"reader\"\n",
    )
    .unwrap();
    let (store, _, err) = ConfigStore::open(path);
    assert!(err.is_none());
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/vsphere");
    let page = std::fs::read_to_string(root.join("vcenter-inventory.xml")).unwrap();
    let about = vi_vsphere::About {
        api_type: "VirtualCenter".into(),
        version: "8.0".into(),
        full_name: "vCenter simulator".into(),
    };
    let mut inv = vi_vsphere::inventory::parse(&[page], &about).unwrap();
    // Make one a template.
    inv.domains[1]
        .details
        .as_mut()
        .unwrap()
        .general
        .vsphere
        .as_mut()
        .unwrap()
        .template = true;
    let cache = Cache::new(dir.path().join("cache"));
    cache
        .save(&CachedInventory::new("vc", 1_700_000_000, inv.clone()))
        .unwrap();
    let mut app = App::new(store, vec![], Some(cache));
    assert!(app.is_vsphere("vc"));
    let ctx = egui::Context::default();
    app.apply_theme(&ctx);
    app.selection = Selection::Host("vc".into());
    frame(&mut app, &ctx);
    for d in &inv.domains {
        app.selection = Selection::Domain("vc".into(), d.name.clone());
        frame(&mut app, &ctx);
        let g = app.vm_menu_items("vc", d);
        assert_eq!(g[0][0].label, "Open console");
        assert_eq!(g[0][0].action, Err("not available for vSphere".into()));
        assert!(matches!(g[1][0].action, Ok(TreeAction::CopyText(_))));
    }
    assert!(vi_gui::view::is_template(&inv.domains[1]));
    assert!(!vi_gui::view::is_template(&inv.domains[0]));
    // No SSH items for the vSphere host.
    assert_eq!(app.host_menu_items("vc").len(), 1);
}
