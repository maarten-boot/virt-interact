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
    let mut app = App::new(store, vec![], Some(cache));
    let ctx = egui::Context::default();
    let mut text = std::fs::read_to_string(&path).unwrap();
    text.push_str("[[hosts]]\nname = \"third\"\nhost = \"t.example.lan\"\n");
    std::fs::write(&path, &text).unwrap();
    app.reload_config(&ctx);
    assert!(app.hosts.contains_key("third"));
    assert_eq!(app.store.config.hosts.len(), 3);

    std::fs::write(&path, "[[hosts]\nbroken").unwrap();
    app.reload_config(&ctx);
    assert_eq!(app.store.config.hosts.len(), 3);
    assert!(app.messages.last().unwrap().is_error);
}
