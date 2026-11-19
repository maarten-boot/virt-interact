//! The main window (SPEC-015): toolbar, host/VM tree, detail panel, dialogs.
//!
//! State changes go through methods on [`App`] so tests can drive them
//! without a display (Plan 4.7). Drawing is in [`App::show`], which tests
//! run headless with `egui::Context::run_ui`.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

use eframe::egui::{self, Color32, RichText};
use vi_core::cache::{Cache, CachedInventory, format_age, format_utc, now_secs};
use vi_core::config::{Backend, Config, ConfigStore, HostConfig};
use vi_core::contrast::{ContrastLevel, MAX_RATIO, MIN_RATIO, Palette};
use vi_core::model::{Domain, DomainDetails, HostInventory};
use vi_ssh::Secret;

use crate::console::{self, ConsoleKey, ConsoleState};
use crate::prompts::{GuiPrompter, PromptKind, PromptRequest};
use crate::terminal;
use crate::theme;
use crate::view::{self, Filter, StateColor, StateFilter};
use crate::worker::{Event, HostConnector, HostStatus, Worker};

/// A message shown under the toolbar until dismissed.
#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub text: String,
    pub is_error: bool,
}

impl Message {
    pub fn warning(text: impl Into<String>) -> Message {
        Message {
            text: text.into(),
            is_error: false,
        }
    }
    pub fn error(text: impl Into<String>) -> Message {
        Message {
            text: text.into(),
            is_error: true,
        }
    }
    pub fn info(text: impl Into<String>) -> Message {
        Message::warning(text)
    }
}

/// The inventory shown for a host.
#[derive(Debug, Clone, PartialEq)]
pub struct Shown {
    pub inventory: HostInventory,
    pub taken_at: u64,
    /// From the cache, or the last good result after a failed refresh
    /// (SPEC-014): shown with a "cached" marker.
    pub cached: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HostEntry {
    pub status: HostStatus,
    pub shown: Option<Shown>,
}

impl Default for HostEntry {
    fn default() -> Self {
        HostEntry {
            status: HostStatus::Idle,
            shown: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Selection {
    #[default]
    None,
    Host(String),
    Domain(String, String),
}

impl Selection {
    pub fn host(&self) -> Option<&str> {
        match self {
            Selection::None => None,
            Selection::Host(h) | Selection::Domain(h, _) => Some(h),
        }
    }
}

/// Which contrast option the settings show.
#[derive(Debug, Clone, Copy, PartialEq)]
enum ContrastChoice {
    Aa,
    Aaa,
    Max,
    Custom,
}

/// The add/edit host form (Plan 4.3).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HostForm {
    /// Name of the host being edited; `None` when adding.
    pub original: Option<String>,
    pub backend: Backend,
    pub name: String,
    pub host: String,
    pub port: String,
    pub user: String,
    pub identity_file: String,
    pub libvirt_uri: String,
    pub virsh_prefix: String,
    pub error: Option<String>,
}

impl HostForm {
    pub fn new_host() -> HostForm {
        HostForm {
            libvirt_uri: HostConfig::new("", "").libvirt_uri,
            ..HostForm::default()
        }
    }

    pub fn edit(h: &HostConfig) -> HostForm {
        HostForm {
            original: Some(h.name.clone()),
            backend: h.backend,
            name: h.name.clone(),
            host: h.host.clone(),
            port: h.port.map(|p| p.to_string()).unwrap_or_default(),
            user: h.user.clone().unwrap_or_default(),
            identity_file: h
                .identity_file
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            libvirt_uri: h.libvirt_uri.clone(),
            virsh_prefix: h.virsh_prefix.clone().unwrap_or_default(),
            error: None,
        }
    }

    /// The host entry this form describes. Field checks that need the whole
    /// config (unique name, prefix characters) are done by `Config::validate`.
    pub fn to_host(&self, base: Option<&HostConfig>) -> Result<HostConfig, String> {
        let opt = |s: &str| {
            let t = s.trim();
            (!t.is_empty()).then(|| t.to_string())
        };
        let port = match self.port.trim() {
            "" => None,
            p => Some(
                p.parse::<u16>()
                    .ok()
                    .filter(|p| *p > 0)
                    .ok_or_else(|| format!("port \"{p}\" must be a number 1–65535"))?,
            ),
        };
        if self.name.trim().is_empty() {
            return Err("Name is required.".into());
        }
        if self.host.trim().is_empty() {
            return Err("Host is required.".into());
        }
        let vsphere = self.backend == Backend::Vsphere;
        if vsphere && self.user.trim().is_empty() {
            return Err("User is required for vSphere.".into());
        }
        let mut h = base.cloned().unwrap_or_else(|| HostConfig::new("", ""));
        h.backend = self.backend;
        h.name = self.name.trim().to_string();
        h.host = self.host.trim().to_string();
        h.port = port;
        h.user = opt(&self.user);
        // The SSH/libvirt fields do not apply to vSphere (SPEC-005); what
        // was typed in them is kept in the form but not saved.
        let default_uri = HostConfig::new("", "").libvirt_uri;
        h.identity_file = (!vsphere)
            .then(|| opt(&self.identity_file).map(PathBuf::from))
            .flatten();
        h.libvirt_uri = if vsphere {
            default_uri
        } else {
            opt(&self.libvirt_uri).unwrap_or(default_uri)
        };
        h.virsh_prefix = (!vsphere).then(|| opt(&self.virsh_prefix)).flatten();
        Ok(h)
    }
}

/// A prompt from the SSH layer being shown.
struct OpenPrompt {
    request: PromptRequest,
    passphrase: String,
}

pub struct App {
    pub store: ConfigStore,
    pub palette: Palette,
    pub messages: Vec<Message>,
    pub hosts: HashMap<String, HostEntry>,
    pub selection: Selection,
    pub filter: Filter,
    pub host_form: Option<HostForm>,
    pub confirm_remove: Option<String>,
    pub settings_open: bool,
    custom_ratio: f64,
    cache: Option<Cache>,
    worker: Option<Worker>,
    events: Option<Receiver<Event>>,
    prompt_rx: Option<Receiver<PromptRequest>>,
    prompts: VecDeque<PromptRequest>,
    open_prompt: Option<OpenPrompt>,
    /// `[ui] refresh_on_start` handled (first frame only).
    started: bool,
    /// Scroll the tree to the selection (after keyboard navigation).
    scroll_to_selection: bool,
    /// Where `virsh` was found locally (SPEC-020), if detection ran.
    local_virsh: Option<PathBuf>,
    /// The local entries currently shown (from `[local]`).
    local_hosts: Vec<HostConfig>,
    /// Consoles opening or open (SPEC-018); closed ones are removed.
    pub consoles: HashMap<ConsoleKey, ConsoleState>,
    /// Settings: the terminal command as text, and which terminal is used
    /// or why the text was not saved (SPEC-025).
    pub terminal_text: String,
    pub terminal_note: String,
}

/// A click in the graphics table.
enum ConsoleAction {
    Open(ConsoleKey),
    Close(ConsoleKey),
}

/// What a right-click menu item in the tree does (SPEC-024).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeAction {
    Refresh(String),
    Edit(String),
    Remove(String),
    StartSsh(String),
    CopyText(String),
    OpenConsole(ConsoleKey),
    CloseConsole(ConsoleKey),
}

/// One item of a right-click menu; `Err` holds why it is disabled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItem {
    pub label: String,
    pub action: Result<TreeAction, String>,
}

impl MenuItem {
    fn new(label: impl Into<String>, action: Result<TreeAction, String>) -> MenuItem {
        MenuItem {
            label: label.into(),
            action,
        }
    }
}

impl App {
    /// An app without background worker (tests, or no network).
    pub fn new(store: ConfigStore, messages: Vec<Message>, cache: Option<Cache>) -> App {
        let level = store.config.ui.contrast;
        let custom_ratio = match level {
            ContrastLevel::Custom(r) => r,
            _ => 10.0,
        };
        let mut app = App {
            palette: Palette::for_level(level),
            store,
            messages,
            hosts: HashMap::new(),
            selection: Selection::None,
            filter: Filter::default(),
            host_form: None,
            confirm_remove: None,
            settings_open: false,
            custom_ratio,
            cache,
            worker: None,
            events: None,
            prompt_rx: None,
            prompts: VecDeque::new(),
            open_prompt: None,
            started: false,
            scroll_to_selection: false,
            local_virsh: None,
            local_hosts: Vec::new(),
            consoles: HashMap::new(),
            terminal_text: String::new(),
            terminal_note: String::new(),
        };
        app.local_hosts = app.store.config.local_hosts(false);
        app.sync_hosts();
        app
    }

    /// Attach a worker; prompts arrive on `prompt_rx`.
    pub fn with_worker(
        mut self,
        worker: Worker,
        events: Receiver<Event>,
        prompt_rx: Receiver<PromptRequest>,
    ) -> App {
        self.worker = Some(worker);
        self.events = Some(events);
        self.prompt_rx = Some(prompt_rx);
        self
    }

    /// The real setup: SSH connector, GUI prompter, repaint on events.
    pub fn with_ssh(self, ctx: &egui::Context) -> App {
        let (ptx, prx) = channel();
        let c1 = ctx.clone();
        let notify: Arc<dyn Fn() + Send + Sync> = Arc::new(move || c1.request_repaint());
        let prompter = Arc::new(GuiPrompter::new(ptx, notify.clone()));
        let (worker, events) = Worker::new(Arc::new(HostConnector::new()), prompter, notify);
        self.with_worker(worker, events, prx)
    }

    /// Look for `virsh` on `PATH` and show the local entries if `[local]`
    /// says so (SPEC-020). Not done in `new`, so tests stay independent of
    /// the machine they run on.
    pub fn with_local_detection(mut self) -> App {
        self.local_virsh = vi_backend::local::find_virsh();
        self.update_local_hosts();
        self
    }

    /// For tests: pretend `virsh` was (not) found.
    pub fn set_local_virsh(&mut self, path: Option<PathBuf>) {
        self.local_virsh = path;
        self.update_local_hosts();
    }

    fn update_local_hosts(&mut self) {
        self.local_hosts = self.store.config.local_hosts(self.local_virsh.is_some());
        self.sync_hosts();
    }

    /// Local entries first, then the configured hosts.
    pub fn all_hosts(&self) -> impl Iterator<Item = &HostConfig> {
        self.local_hosts
            .iter()
            .chain(self.store.config.hosts.iter())
    }

    /// Display name of a host (`localhost (system)` for local entries).
    pub fn display_name(&self, name: &str) -> String {
        self.host_config(name)
            .map_or_else(|| name.to_string(), HostConfig::display_name)
    }

    pub fn apply_theme(&self, ctx: &egui::Context) {
        theme::apply(ctx, &self.palette);
    }

    /// Make `hosts` match the config: add entries (from the cache if there
    /// is one), drop entries that are gone and delete their cache files.
    /// That covers hosts removed in the GUI or from the file (reload), and
    /// local entries that disappear (crosscheck 02, C3).
    fn sync_hosts(&mut self) {
        let names: Vec<String> = self.all_hosts().map(|h| h.name.clone()).collect();
        let cache = self.cache.as_ref();
        self.hosts.retain(|k, _| {
            let keep = names.contains(k);
            if !keep && let Some(c) = cache {
                let _ = c.remove(k);
            }
            keep
        });
        for n in names {
            if !self.hosts.contains_key(&n) {
                let shown = self.cache.as_ref().and_then(|c| c.load(&n)).map(|c| Shown {
                    inventory: c.inventory,
                    taken_at: c.taken_at,
                    cached: true,
                });
                self.hosts.insert(
                    n,
                    HostEntry {
                        status: HostStatus::Idle,
                        shown,
                    },
                );
            }
        }
        if let Some(h) = self.selection.host()
            && !self.hosts.contains_key(h)
        {
            self.selection = Selection::None;
        }
    }

    fn host_config(&self, name: &str) -> Option<&HostConfig> {
        self.all_hosts().find(|h| h.name == name)
    }

    // ---- actions -------------------------------------------------------

    /// Start a refresh of one host (SPEC-012).
    pub fn refresh(&mut self, name: &str) {
        let Some(h) = self.host_config(name).cloned() else {
            return;
        };
        let ssh = self.store.config.ssh.clone();
        match &self.worker {
            Some(w) => {
                let reuse = w.has_session(name);
                if w.refresh(h, ssh)
                    && let Some(e) = self.hosts.get_mut(name)
                {
                    // A kept connection is reused: no "connecting" step (C6).
                    e.status = if reuse {
                        HostStatus::Refreshing
                    } else {
                        HostStatus::Connecting
                    };
                }
            }
            None => self.messages.push(Message::error(
                "Refresh is not available (no background worker).",
            )),
        }
    }

    /// Open a console of a VM (SPEC-018). Errors arrive as messages.
    pub fn open_console(&mut self, key: ConsoleKey) {
        let Some(h) = self.host_config(&key.host).cloned() else {
            return;
        };
        let Some(w) = &self.worker else {
            self.messages.push(Message::error(
                "Consoles are not available (no background worker).",
            ));
            return;
        };
        let viewer = console::find_viewer(self.store.config.console.viewer.as_deref());
        if w.open_console(h, self.store.config.ssh.clone(), key.clone(), viewer) {
            self.consoles.insert(key, ConsoleState::Opening);
        }
    }

    // ---- right-click menus (SPEC-024) -----------------------------------

    /// The menu of a VM in the tree, in groups (separated when drawn).
    pub fn vm_menu_items(&self, host: &str, d: &Domain) -> Vec<Vec<MenuItem>> {
        let mut consoles = Vec::new();
        match &d.details {
            None => consoles.push(MenuItem::new(
                "Open console",
                Err("refresh the host first to read the VM's details".into()),
            )),
            Some(x) => {
                let devices: Vec<_> = x
                    .graphics
                    .iter()
                    .enumerate()
                    .filter(|(_, g)| vi_core::model::CONSOLE_KINDS.contains(&g.kind.as_str()))
                    .collect();
                if devices.is_empty() {
                    consoles.push(MenuItem::new(
                        "Open console",
                        Err("no VNC or SPICE display".into()),
                    ));
                }
                for (n, (index, g)) in devices.iter().enumerate() {
                    let kind = g.kind.to_uppercase();
                    let same = devices.iter().filter(|(_, o)| o.kind == g.kind).count();
                    let name = if same > 1 {
                        let nth = devices[..=n]
                            .iter()
                            .filter(|(_, o)| o.kind == g.kind)
                            .count();
                        format!("{kind} {nth}")
                    } else {
                        kind
                    };
                    let key = ConsoleKey {
                        host: host.to_string(),
                        domain: d.name.clone(),
                        index: *index,
                    };
                    let open = format!("Open console ({name})");
                    consoles.push(match self.consoles.get(&key) {
                        Some(ConsoleState::Open { .. }) => MenuItem::new(
                            format!("Close console ({name})"),
                            Ok(TreeAction::CloseConsole(key)),
                        ),
                        Some(ConsoleState::Opening) => MenuItem::new(open, Err("opening…".into())),
                        Some(ConsoleState::Closing) => MenuItem::new(open, Err("closing…".into())),
                        _ => MenuItem::new(
                            open,
                            view::console_availability(&d.state, g)
                                .map(|()| TreeAction::OpenConsole(key)),
                        ),
                    });
                }
            }
        }
        let copy = vec![
            MenuItem::new("Copy name", Ok(TreeAction::CopyText(d.name.clone()))),
            MenuItem::new(
                "Copy UUID",
                d.uuid
                    .clone()
                    .map(TreeAction::CopyText)
                    .ok_or_else(|| "UUID unknown".to_string()),
            ),
        ];
        vec![consoles, copy]
    }

    /// The menu of a host in the tree, in groups.
    pub fn host_menu_items(&self, name: &str) -> Vec<Vec<MenuItem>> {
        let Some(cfg) = self.host_config(name) else {
            return Vec::new();
        };
        let busy = self.hosts.get(name).is_some_and(|e| e.status.busy());
        let n = name.to_string();
        let local = || Err("this machine is set up under [local] in the config file".to_string());
        let mut groups = vec![vec![
            MenuItem::new(
                "Refresh",
                if busy {
                    Err("being read".into())
                } else {
                    Ok(TreeAction::Refresh(n.clone()))
                },
            ),
            MenuItem::new(
                "Edit…",
                if cfg.local {
                    local()
                } else {
                    Ok(TreeAction::Edit(n.clone()))
                },
            ),
            MenuItem::new(
                "Remove…",
                if cfg.local {
                    local()
                } else {
                    Ok(TreeAction::Remove(n.clone()))
                },
            ),
        ]];
        // No SSH for this machine, nor for vSphere (the entry's user is an
        // API account, SPEC-024).
        if !cfg.local && cfg.backend == Backend::Libvirt {
            groups.push(vec![
                MenuItem::new("Start SSH", Ok(TreeAction::StartSsh(n))),
                MenuItem::new(
                    "Copy ssh command",
                    Ok(TreeAction::CopyText(terminal::command_line(
                        &terminal::ssh_args(cfg),
                    ))),
                ),
            ]);
        }
        groups
    }

    /// Carry out a menu choice.
    pub fn apply_tree_action(&mut self, ctx: &egui::Context, action: TreeAction) {
        match action {
            TreeAction::Refresh(n) => self.refresh(&n),
            TreeAction::Edit(n) => {
                if let Some(cfg) = self.host_config(&n).cloned() {
                    self.host_form = Some(HostForm::edit(&cfg));
                }
            }
            TreeAction::Remove(n) => self.confirm_remove = Some(n),
            TreeAction::StartSsh(n) => self.start_ssh(&n),
            TreeAction::CopyText(t) => ctx.copy_text(t),
            TreeAction::OpenConsole(k) => self.open_console(k),
            TreeAction::CloseConsole(k) => self.close_console(&k),
        }
    }

    /// Open a terminal with ssh to a host (SPEC-025).
    pub fn start_ssh(&mut self, name: &str) {
        let Some(cfg) = self.host_config(name).cloned() else {
            return;
        };
        match terminal::start_ssh(&cfg, self.store.config.terminal.command.as_deref()) {
            Ok(line) => self.messages.push(Message::info(format!(
                "Started SSH to {} in a terminal: {line}",
                cfg.display_name()
            ))),
            Err(e) => self.messages.push(Message::error(format!(
                "SSH to {}: {e}",
                cfg.display_name()
            ))),
        }
    }

    /// Draw a menu; returns the chosen action.
    fn show_menu(&self, ui: &mut egui::Ui, groups: &[Vec<MenuItem>]) -> Option<TreeAction> {
        let mut chosen = None;
        for (i, group) in groups.iter().enumerate() {
            if i > 0 {
                ui.separator();
            }
            for item in group {
                let mut r = self.button(ui, item.action.is_ok(), &item.label);
                if let Err(reason) = &item.action {
                    r = r.on_disabled_hover_text(reason);
                }
                if r.clicked() {
                    chosen = item.action.clone().ok();
                    ui.close();
                }
            }
        }
        chosen
    }

    /// Settings: show the configured terminal command and which is used.
    pub fn load_terminal_setting(&mut self) {
        self.terminal_text = self
            .store
            .config
            .terminal
            .command
            .as_deref()
            .map(terminal::command_line)
            .unwrap_or_default();
        self.terminal_note = self.terminal_in_use();
    }

    fn terminal_in_use(&self) -> String {
        match terminal::choose(self.store.config.terminal.command.as_deref()) {
            Ok(t) => format!("Used: {}", t.describe()),
            Err(e) => e,
        }
    }

    /// Settings: save the terminal text (empty means automatic).
    pub fn apply_terminal_setting(&mut self) {
        match terminal::split_words(&self.terminal_text) {
            Err(e) => self.terminal_note = format!("Not saved: {e}"),
            Ok(words) => {
                self.store.config.terminal.command = (!words.is_empty()).then_some(words);
                self.save_config();
                self.terminal_note = self.terminal_in_use();
            }
        }
    }

    /// Close a console: its viewer is stopped and the tunnel closed.
    pub fn close_console(&mut self, key: &ConsoleKey) {
        if let Some(w) = &self.worker {
            w.close_console(key);
        }
        // Shown until the worker reports Closed: the same console cannot be
        // opened again while its viewer is still stopping.
        if let Some(state) = self.consoles.get_mut(key) {
            *state = ConsoleState::Closing;
        }
    }

    pub fn refresh_all(&mut self) {
        let names: Vec<String> = self.all_hosts().map(|h| h.name.clone()).collect();
        for n in names {
            self.refresh(&n);
        }
    }

    /// Handle worker events and SSH prompts. Called every frame.
    pub fn poll(&mut self) {
        let mut events = Vec::new();
        if let Some(rx) = &self.events {
            events.extend(rx.try_iter());
        }
        for e in events {
            self.handle(e);
        }
        if let Some(rx) = &self.prompt_rx {
            self.prompts.extend(rx.try_iter());
        }
        if self.open_prompt.is_none()
            && let Some(request) = self.prompts.pop_front()
        {
            self.open_prompt = Some(OpenPrompt {
                request,
                passphrase: String::new(),
            });
        }
    }

    pub fn handle(&mut self, e: Event) {
        match e {
            Event::Status { host, status } => {
                let Some(entry) = self.hosts.get_mut(&host) else {
                    return; // host removed meanwhile
                };
                if let HostStatus::Error(_) = status
                    && let Some(s) = &mut entry.shown
                {
                    // Keep showing the last good inventory, marked (SPEC-014).
                    s.cached = true;
                }
                entry.status = status;
            }
            Event::Inventory {
                host,
                inventory,
                taken_at,
            } => {
                let Some(entry) = self.hosts.get_mut(&host) else {
                    return;
                };
                if let Some(c) = &self.cache {
                    let ci = CachedInventory::new(&host, taken_at, inventory.clone());
                    if let Err(e) = c.save(&ci) {
                        tracing::warn!(host, error = %e, "cannot save cache");
                    }
                }
                entry.shown = Some(Shown {
                    inventory,
                    taken_at,
                    cached: false,
                });
                // A selected domain that no longer exists: select the host.
                if let Selection::Domain(h, d) = &self.selection
                    && *h == host
                    && !entry
                        .shown
                        .as_ref()
                        .is_some_and(|s| s.inventory.domains.iter().any(|x| &x.name == d))
                {
                    self.selection = Selection::Host(host);
                }
            }
            Event::Console { key, state } => match state {
                ConsoleState::Closed { error } => {
                    self.consoles.remove(&key);
                    if let Some(e) = error {
                        self.messages.push(Message::error(format!(
                            "Console of {} on {}: {e}",
                            key.domain,
                            self.display_name(&key.host)
                        )));
                    }
                }
                state => {
                    self.consoles.insert(key, state);
                }
            },
        }
    }

    /// Change contrast: re-theme at once and save to the config (TC-006).
    pub fn set_contrast(&mut self, ctx: &egui::Context, level: ContrastLevel) {
        if self.store.config.ui.contrast == level {
            return;
        }
        self.store.config.ui.contrast = level;
        self.palette = Palette::for_level(level);
        self.apply_theme(ctx);
        tracing::info!("contrast set to {}", level.label());
        self.save_config();
    }

    fn save_config(&mut self) -> bool {
        match self.store.save() {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!("{e}");
                self.messages
                    .push(Message::error(format!("Not saved: {e}")));
                false
            }
        }
    }

    /// Apply the host form: validate, update the config, save. On error the
    /// form stays open with the message.
    pub fn submit_host_form(&mut self) -> bool {
        let Some(form) = self.host_form.clone() else {
            return false;
        };
        let base = form
            .original
            .as_deref()
            .and_then(|n| self.host_config(n))
            .cloned();
        let host = match form.to_host(base.as_ref()) {
            Ok(h) => h,
            Err(e) => {
                self.host_form.as_mut().expect("form").error = Some(e);
                return false;
            }
        };
        let mut config: Config = self.store.config.clone();
        match &form.original {
            Some(old) => {
                if let Some(slot) = config.hosts.iter_mut().find(|h| &h.name == old) {
                    *slot = host.clone();
                }
            }
            None => config.hosts.push(host.clone()),
        }
        if let Err(e) = config.validate(&self.store.path) {
            let msg = e.to_string();
            let msg = msg
                .strip_prefix(&format!("{}: ", self.store.path.display()))
                .unwrap_or(&msg)
                .to_string();
            self.host_form.as_mut().expect("form").error = Some(msg);
            return false;
        }
        let old_config = std::mem::replace(&mut self.store.config, config);
        if !self.save_config() {
            self.store.config = old_config;
            return false;
        }
        if let Some(old) = &form.original {
            // Connection settings may have changed: reconnect next time.
            if let Some(w) = &self.worker {
                w.forget(old);
            }
            if *old != host.name {
                if let Some(entry) = self.hosts.remove(old) {
                    self.hosts.insert(host.name.clone(), entry);
                }
                if let Some(c) = &self.cache {
                    let _ = c.remove(old);
                }
            }
        }
        self.sync_hosts();
        self.selection = Selection::Host(host.name);
        self.host_form = None;
        true
    }

    pub fn remove_host(&mut self, name: &str) {
        let before = self.store.config.clone();
        self.store.config.hosts.retain(|h| h.name != name);
        if !self.save_config() {
            self.store.config = before;
            return;
        }
        if let Some(w) = &self.worker {
            w.forget(name);
        }
        self.sync_hosts();
    }

    /// Re-read the config file (Plan 4.6). An invalid file is reported and
    /// the current config stays.
    pub fn reload_config(&mut self, ctx: &egui::Context) {
        let (store, warnings, error) = ConfigStore::open(self.store.path.clone());
        for w in warnings {
            self.messages
                .push(Message::warning(format!("Warning: {w}")));
        }
        if let Some(e) = error {
            self.messages
                .push(Message::error(format!("Config not reloaded: {e}")));
            return;
        }
        self.store = store;
        if let Some(w) = &self.worker {
            w.forget_all();
        }
        self.palette = Palette::for_level(self.store.config.ui.contrast);
        if let ContrastLevel::Custom(r) = self.store.config.ui.contrast {
            self.custom_ratio = r;
        }
        self.apply_theme(ctx);
        self.local_hosts = self.store.config.local_hosts(self.local_virsh.is_some());
        self.sync_hosts();
        self.messages.push(Message::info(format!(
            "Config reloaded: {} host(s).",
            self.store.config.hosts.len()
        )));
    }

    /// Delete the cache files and stop showing cached inventories.
    pub fn clear_cache(&mut self) {
        let Some(c) = &self.cache else { return };
        match c.clear() {
            Ok(n) => {
                for e in self.hosts.values_mut() {
                    if e.shown.as_ref().is_some_and(|s| s.cached) {
                        e.shown = None;
                    }
                }
                self.messages
                    .push(Message::info(format!("Cache cleared ({n} file(s)).")));
            }
            Err(e) => self
                .messages
                .push(Message::error(format!("Cannot clear the cache: {e}"))),
        }
    }

    pub fn domain(&self, host: &str, name: &str) -> Option<(&Domain, bool)> {
        let s = self.hosts.get(host)?.shown.as_ref()?;
        let d = s.inventory.domains.iter().find(|d| d.name == name)?;
        Some((d, s.cached))
    }

    fn state_color(&self, c: StateColor) -> Color32 {
        theme::c(match c {
            StateColor::Running => self.palette.state_running,
            StateColor::Paused => self.palette.state_paused,
            StateColor::Stopped => self.palette.state_stopped,
            StateColor::Error => self.palette.state_error,
        })
    }

    fn status_text(&self, s: &HostStatus) -> RichText {
        let p = &self.palette;
        match s {
            HostStatus::Ok(at) => RichText::new(format!("ok, {}", format_age(*at, now_secs())))
                .color(theme::c(p.state_running)),
            HostStatus::Error(_) => RichText::new("error").color(theme::c(p.state_error)),
            other => RichText::new(other.label()).color(theme::c(p.weak_text)),
        }
    }

    // ---- keyboard navigation (SPEC-015) ----------------------------------

    /// Tree rows in display order: each host, then its VMs that match the
    /// filter if the host is expanded (`is_open`).
    pub fn visible_items(&self, is_open: impl Fn(&str) -> bool) -> Vec<Selection> {
        let mut out = Vec::new();
        for h in self.all_hosts() {
            out.push(Selection::Host(h.name.clone()));
            if !is_open(&h.name) {
                continue;
            }
            if let Some(s) = self.hosts.get(&h.name).and_then(|e| e.shown.as_ref()) {
                for d in s
                    .inventory
                    .domains
                    .iter()
                    .filter(|d| self.filter.matches(d))
                {
                    out.push(Selection::Domain(h.name.clone(), d.name.clone()));
                }
            }
        }
        out
    }

    /// Move the selection `delta` rows (Up = -1, Down = +1), clamped. With
    /// nothing selected, Down selects the first row and Up the last.
    pub fn move_selection(&mut self, delta: i32, is_open: impl Fn(&str) -> bool) {
        let items = self.visible_items(is_open);
        if items.is_empty() {
            return;
        }
        let last = items.len() as i32 - 1;
        let next = match items.iter().position(|i| *i == self.selection) {
            Some(p) => (p as i32).saturating_add(delta).clamp(0, last),
            None if delta > 0 => 0,
            None => last,
        };
        self.selection = items[next as usize].clone();
        self.scroll_to_selection = true;
    }

    fn modal_open(&self) -> bool {
        self.host_form.is_some() || self.confirm_remove.is_some() || self.open_prompt.is_some()
    }

    /// Up/Down move through the tree, Home/End jump, Enter refreshes the
    /// selected host. Ignored while typing or while a dialog is open.
    fn keyboard(&mut self, ctx: &egui::Context) {
        if self.modal_open() || ctx.egui_wants_keyboard_input() {
            return;
        }
        let (up, down, home, end, enter) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::ArrowDown),
                i.key_pressed(egui::Key::Home),
                i.key_pressed(egui::Key::End),
                i.key_pressed(egui::Key::Enter),
            )
        });
        let is_open = |h: &str| {
            egui::collapsing_header::CollapsingState::load(ctx, tree_id(h))
                .is_none_or(|s| s.is_open())
        };
        if up {
            self.move_selection(-1, is_open);
        } else if down {
            self.move_selection(1, is_open);
        } else if home {
            self.move_selection(-i32::MAX, is_open);
        } else if end {
            self.move_selection(i32::MAX, is_open);
        } else if enter
            && let Some(h) = self.selection.host().map(str::to_string)
            && !self.hosts.get(&h).is_some_and(|e| e.status.busy())
        {
            self.refresh(&h);
        }
    }

    /// A button; when disabled its text uses the palette's disabled colour,
    /// because fading is off (SPEC-001, C2).
    fn button(&self, ui: &mut egui::Ui, enabled: bool, text: &str) -> egui::Response {
        let label = RichText::new(text).color(button_color(&self.palette, enabled));
        ui.add_enabled(enabled, egui::Button::new(label))
    }

    // ---- drawing -------------------------------------------------------

    /// Draw the whole window into `ui`.
    pub fn show(&mut self, ui: &mut egui::Ui) {
        self.poll();
        let ctx = ui.ctx().clone();
        if !self.started {
            self.started = true;
            if self.store.config.ui.refresh_on_start && self.worker.is_some() {
                self.refresh_all();
            }
        }
        self.keyboard(&ctx);
        // Keep "5 min ago" texts current while idle (C4).
        if self
            .hosts
            .values()
            .any(|e| e.shown.is_some() || matches!(e.status, HostStatus::Ok(_)))
        {
            ctx.request_repaint_after(std::time::Duration::from_secs(30));
        }
        egui::Panel::top("toolbar").show(ui, |ui| {
            self.toolbar(ui);
            self.messages_bar(ui);
        });
        // The tree's width changes only when the user drags the separator:
        // wide rows scroll instead of widening the panel (SPEC-015,
        // crosscheck 2026-11-14-02 H2). The width is kept between runs
        // with the rest of egui's memory (eframe persistence).
        egui::Panel::left(TREE_PANEL)
            .resizable(true)
            .default_size(300.0)
            .show(ui, |ui| {
                egui::ScrollArea::both()
                    .id_salt("tree-scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.tree(ui));
            });
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::both()
                .auto_shrink([false, false])
                .show(ui, |ui| self.details(ui));
        });
        self.host_form_window(&ctx);
        self.remove_dialog(&ctx);
        self.settings_window(&ctx);
        self.prompt_dialog(&ctx);
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.strong(format!("{} {}", vi_core::APP_NAME, vi_core::VERSION));
            ui.separator();
            let sel = self.selection.host().map(str::to_string);
            let busy = sel
                .as_deref()
                .and_then(|h| self.hosts.get(h))
                .is_some_and(|e| e.status.busy());
            if self
                .button(ui, sel.is_some() && !busy, "Refresh selected")
                .on_hover_text("Refresh the inventory of the selected host")
                .clicked()
                && let Some(h) = &sel
            {
                self.refresh(h);
            }
            if self
                .button(ui, self.all_hosts().next().is_some(), "Refresh all")
                .clicked()
            {
                self.refresh_all();
            }
            if ui.button("Add host…").clicked() {
                self.host_form = Some(HostForm::new_host());
            }
            if ui.button("Settings…").clicked() {
                self.settings_open = true;
                self.load_terminal_setting();
            }
            if ui
                .button("Reload config")
                .on_hover_text("Read the config file again")
                .clicked()
            {
                let ctx = ui.ctx().clone();
                self.reload_config(&ctx);
            }
            ui.separator();
            ui.label("Filter:");
            ui.add(
                egui::TextEdit::singleline(&mut self.filter.text)
                    .hint_text("VM name")
                    .desired_width(140.0),
            );
            egui::ComboBox::from_id_salt("state-filter")
                .selected_text(self.filter.state.label())
                .show_ui(ui, |ui| {
                    for f in StateFilter::ALL {
                        ui.selectable_value(&mut self.filter.state, f, f.label());
                    }
                });
            if self.filter.is_active() && ui.button("Clear filter").clicked() {
                self.filter = Filter::default();
            }
        });
    }

    fn messages_bar(&mut self, ui: &mut egui::Ui) {
        let mut dismiss = None;
        for (i, m) in self.messages.iter().enumerate() {
            ui.horizontal_wrapped(|ui| {
                let color = if m.is_error {
                    theme::c(self.palette.state_error)
                } else {
                    theme::c(self.palette.text)
                };
                ui.colored_label(color, &m.text);
                if ui.small_button("Dismiss").clicked() {
                    dismiss = Some(i);
                }
            });
        }
        if let Some(i) = dismiss {
            self.messages.remove(i);
        }
    }

    fn tree(&mut self, ui: &mut egui::Ui) {
        if self.all_hosts().next().is_none() {
            ui.label("No hosts configured.");
            if ui.button("Add host…").clicked() {
                self.host_form = Some(HostForm::new_host());
            }
            return;
        }
        let hosts: Vec<String> = self.all_hosts().map(|h| h.name.clone()).collect();
        let mut new_sel = None;
        let mut action = None;
        for name in hosts {
            let Some(entry) = self.hosts.get(&name) else {
                continue;
            };
            let id = tree_id(&name);
            let label = self.display_name(&name);
            let status = self.status_text(&entry.status);
            let selected = self.selection == Selection::Host(name.clone());
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
                .show_header(ui, |ui| {
                    let r = ui.selectable_label(selected, RichText::new(&label).strong());
                    if selected && self.scroll_to_selection {
                        r.scroll_to_me(None);
                    }
                    if r.clicked() {
                        new_sel = Some(Selection::Host(name.clone()));
                    }
                    r.context_menu(|ui| {
                        if let Some(a) = self.show_menu(ui, &self.host_menu_items(&name)) {
                            action = Some(a);
                        }
                    });
                    ui.label(status);
                    if entry.shown.as_ref().is_some_and(|s| s.cached) {
                        ui.label(RichText::new("(cached)").color(theme::c(self.palette.weak_text)));
                    }
                })
                .body(|ui| match &entry.shown {
                    None => {
                        ui.label(
                            RichText::new("not loaded yet; use Refresh")
                                .color(theme::c(self.palette.weak_text)),
                        );
                    }
                    Some(s) => {
                        let mut any = false;
                        for d in s
                            .inventory
                            .domains
                            .iter()
                            .filter(|d| self.filter.matches(d))
                        {
                            any = true;
                            let sel =
                                self.selection == Selection::Domain(name.clone(), d.name.clone());
                            let (badge, col) = view::state_badge(&d.state);
                            ui.horizontal(|ui| {
                                let r = ui.selectable_label(sel, &d.name);
                                if sel && self.scroll_to_selection {
                                    r.scroll_to_me(None);
                                }
                                if r.clicked() {
                                    new_sel = Some(Selection::Domain(name.clone(), d.name.clone()));
                                }
                                r.context_menu(|ui| {
                                    let items = self.vm_menu_items(&name, d);
                                    if let Some(a) = self.show_menu(ui, &items) {
                                        action = Some(a);
                                    }
                                });
                                ui.label(RichText::new(badge).color(self.state_color(col)));
                                if d.error.is_some() {
                                    ui.label(
                                        RichText::new("(error)")
                                            .color(theme::c(self.palette.state_error)),
                                    );
                                }
                            });
                        }
                        if !any {
                            let t = if s.inventory.domains.is_empty() {
                                "no VMs"
                            } else {
                                "no VM matches the filter"
                            };
                            ui.label(RichText::new(t).color(theme::c(self.palette.weak_text)));
                        }
                    }
                });
        }
        self.scroll_to_selection = false;
        if let Some(s) = new_sel {
            self.selection = s;
        }
        if let Some(a) = action {
            self.apply_tree_action(&ui.ctx().clone(), a);
        }
    }

    fn details(&mut self, ui: &mut egui::Ui) {
        match self.selection.clone() {
            Selection::None => {
                ui.heading("Select a host or a VM");
                ui.label("Hosts are listed on the left. Use Refresh to read their VMs (over SSH, or directly for localhost).");
            }
            Selection::Host(h) => self.host_details(ui, &h),
            Selection::Domain(h, d) => self.domain_details(ui, &h, &d),
        }
    }

    fn host_details(&mut self, ui: &mut egui::Ui, name: &str) {
        let (Some(cfg), Some(entry)) = (
            self.host_config(name).cloned(),
            self.hosts.get(name).cloned(),
        ) else {
            return;
        };
        ui.heading(cfg.display_name());
        ui.horizontal(|ui| {
            if self
                .button(ui, !entry.status.busy(), "Refresh")
                .on_hover_text("Shortcut: Enter in the tree")
                .clicked()
            {
                self.refresh(name);
            }
            // Local entries come from [local] in the config file (SPEC-020).
            if !cfg.local {
                if ui.button("Edit…").clicked() {
                    self.host_form = Some(HostForm::edit(&cfg));
                }
                if ui.button("Remove…").clicked() {
                    self.confirm_remove = Some(name.to_string());
                }
            }
        });
        if cfg.local {
            ui.label(
                RichText::new(
                    "This machine, read without SSH. Set it up under [local] in the config file \
                     (enabled = \"auto\" | \"always\" | \"never\").",
                )
                .color(theme::c(self.palette.weak_text)),
            );
        }
        ui.add_space(6.0);
        let none = "default";
        egui::Grid::new("host-grid")
            .num_columns(2)
            .striped(true)
            .show(ui, |ui| {
                if cfg.local {
                    row(ui, "Connection", "local (no SSH)");
                    row(ui, "libvirt URI", cfg.libvirt_uri.clone());
                    row(
                        ui,
                        "virsh prefix",
                        cfg.virsh_prefix.clone().unwrap_or_else(|| "none".into()),
                    );
                    row(ui, "Status", self.status_text(&entry.status));
                    if let Some(s) = &entry.shown {
                        inventory_rows(ui, s);
                    }
                    return;
                }
                row(ui, "Host", cfg.host.clone());
                row(
                    ui,
                    "Port",
                    cfg.port.map_or_else(|| none.into(), |p| p.to_string()),
                );
                row(ui, "User", cfg.user.clone().unwrap_or_else(|| none.into()));
                row(
                    ui,
                    "Identity file",
                    cfg.identity_file
                        .as_ref()
                        .map_or_else(|| none.into(), |p| p.display().to_string()),
                );
                row(ui, "libvirt URI", cfg.libvirt_uri.clone());
                row(
                    ui,
                    "virsh prefix",
                    cfg.virsh_prefix.clone().unwrap_or_else(|| "none".into()),
                );
                row(ui, "Status", self.status_text(&entry.status));
                if let Some(s) = &entry.shown {
                    inventory_rows(ui, s);
                }
            });
        if let HostStatus::Error(e) = &entry.status {
            ui.add_space(6.0);
            ui.label(RichText::new(&e.message).color(theme::c(self.palette.state_error)));
            if let Some(d) = &e.detail {
                ui.collapsing("Details", |ui| {
                    ui.monospace(d);
                });
            }
        }
        if let Some(s) = &entry.shown
            && !s.inventory.warnings.is_empty()
        {
            ui.add_space(6.0);
            ui.collapsing(format!("Warnings ({})", s.inventory.warnings.len()), |ui| {
                for w in &s.inventory.warnings {
                    ui.label(w);
                }
            });
        }
    }

    fn domain_details(&mut self, ui: &mut egui::Ui, host: &str, name: &str) {
        let Some((d, cached)) = self.domain(host, name).map(|(d, c)| (d.clone(), c)) else {
            ui.label("This VM is no longer in the inventory.");
            return;
        };
        let (badge, col) = view::state_badge(&d.state);
        ui.horizontal(|ui| {
            ui.heading(&d.name);
            ui.label(RichText::new(badge).color(self.state_color(col)).strong());
            if cached {
                ui.label(RichText::new("(cached)").color(theme::c(self.palette.weak_text)));
            }
        });
        ui.label(format!("on {}", self.display_name(host)));
        if let Some(e) = &d.error {
            ui.label(RichText::new(e).color(theme::c(self.palette.state_error)));
        }
        egui::Grid::new("dom-grid")
            .num_columns(2)
            .striped(true)
            .show(ui, |ui| {
                row(
                    ui,
                    "UUID",
                    d.uuid.clone().unwrap_or_else(|| "unknown".into()),
                );
                row(ui, "ID", view::opt_text(&d.id));
                row(ui, "Persistent", view::yes_no(d.persistent));
                row(ui, "Autostart", view::yes_no(d.autostart));
            });
        let Some(x) = &d.details else { return };
        match self.components(ui, x, host, &d) {
            Some(ConsoleAction::Open(k)) => self.open_console(k),
            Some(ConsoleAction::Close(k)) => self.close_console(&k),
            None => {}
        }
    }

    /// The console cell of one graphics device (SPEC-018).
    fn console_cell(
        &self,
        ui: &mut egui::Ui,
        key: ConsoleKey,
        d: &Domain,
        gr: &vi_core::model::Graphics,
    ) -> Option<ConsoleAction> {
        let weak = theme::c(self.palette.weak_text);
        match self.consoles.get(&key) {
            Some(ConsoleState::Opening) => {
                ui.label(RichText::new("opening…").color(weak));
                None
            }
            Some(ConsoleState::Closing) => {
                ui.label(RichText::new("closing…").color(weak));
                None
            }
            Some(ConsoleState::Open { url }) => self
                .button(ui, true, "Close console")
                .on_hover_text(format!("The viewer is connected to {url}"))
                .clicked()
                .then_some(ConsoleAction::Close(key)),
            _ => match view::console_availability(&d.state, gr) {
                Ok(()) => self
                    .button(ui, true, "Open console")
                    .on_hover_text("Open in the external viewer, through the host's connection")
                    .clicked()
                    .then_some(ConsoleAction::Open(key)),
                Err(reason) => {
                    ui.horizontal(|ui| {
                        self.button(ui, false, "Open console");
                        ui.label(RichText::new(reason).color(weak));
                    });
                    None
                }
            },
        }
    }

    fn components(
        &self,
        ui: &mut egui::Ui,
        x: &DomainDetails,
        host: &str,
        d: &Domain,
    ) -> Option<ConsoleAction> {
        let mut action = None;
        let g = &x.general;
        egui::CollapsingHeader::new("General")
            .default_open(true)
            .show(ui, |ui| {
                egui::Grid::new("general")
                    .num_columns(2)
                    .striped(true)
                    .show(ui, |ui| {
                        row(
                            ui,
                            "Guest OS",
                            g.guest_os().unwrap_or("unknown").to_string(),
                        );
                        row(ui, "Firmware", g.firmware.label().to_string());
                        row(ui, "Hypervisor type", view::opt_text(&g.domain_type));
                        row(ui, "OS type", view::opt_text(&g.os_type));
                        row(ui, "Architecture", view::opt_text(&g.arch));
                        row(ui, "Machine", view::opt_text(&g.machine));
                        if let Some(t) = &g.title {
                            row(ui, "Title", t.clone());
                        }
                        if let Some(t) = &g.description {
                            row(ui, "Description", t.clone());
                        }
                    });
            });
        egui::CollapsingHeader::new("CPU and memory")
            .default_open(true)
            .show(ui, |ui| {
                egui::Grid::new("cpu")
                    .num_columns(2)
                    .striped(true)
                    .show(ui, |ui| {
                        let c = &x.cpu;
                        row(
                            ui,
                            "vCPUs",
                            format!(
                                "{} (max {})",
                                view::opt_text(&c.current),
                                view::opt_text(&c.max)
                            ),
                        );
                        row(ui, "CPU mode", view::opt_text(&c.mode));
                        if let Some(m) = &c.model {
                            row(ui, "CPU model", m.clone());
                        }
                        if let Some(t) = &c.topology {
                            row(
                                ui,
                                "Topology",
                                format!(
                                    "{} sockets, {} dies, {} cores, {} threads",
                                    view::opt_text(&t.sockets),
                                    view::opt_text(&t.dies),
                                    view::opt_text(&t.cores),
                                    view::opt_text(&t.threads)
                                ),
                            );
                        }
                        row(ui, "Memory (max)", view::kib_text(x.memory.max_kib));
                        row(ui, "Memory (current)", view::kib_text(x.memory.current_kib));
                        row(
                            ui,
                            "Hugepages",
                            if x.memory.hugepages { "yes" } else { "no" },
                        );
                    });
            });
        egui::CollapsingHeader::new(format!("Disks ({})", x.disks.len()))
            .default_open(true)
            .show(ui, |ui| {
                egui::Grid::new("disks")
                    .num_columns(6)
                    .striped(true)
                    .show(ui, |ui| {
                        for h in ["Target", "Device", "Bus", "Source", "Format", "Size"] {
                            ui.strong(h);
                        }
                        ui.end_row();
                        for k in &x.disks {
                            ui.label(k.target.clone().unwrap_or_else(|| "?".into()));
                            let mut dev = k.device.clone();
                            if k.readonly {
                                dev.push_str(", read-only");
                            }
                            if k.shareable {
                                dev.push_str(", shared");
                            }
                            ui.label(dev);
                            ui.label(k.bus.clone().unwrap_or_else(|| "?".into()));
                            ui.label(k.source.describe());
                            ui.label(k.format.clone().unwrap_or_else(|| "-".into()));
                            ui.label(view::disk_size_text(k));
                            ui.end_row();
                        }
                    });
            });
        egui::CollapsingHeader::new(format!("Network interfaces ({})", x.interfaces.len()))
            .default_open(true)
            .show(ui, |ui| {
                egui::Grid::new("nics")
                    .num_columns(5)
                    .striped(true)
                    .show(ui, |ui| {
                        for h in ["Type", "Source", "MAC", "Model", "Host device"] {
                            ui.strong(h);
                        }
                        ui.end_row();
                        for n in &x.interfaces {
                            ui.label(&n.kind);
                            ui.label(n.source.clone().unwrap_or_else(|| "-".into()));
                            ui.label(n.mac.clone().unwrap_or_else(|| "-".into()));
                            ui.label(n.model.clone().unwrap_or_else(|| "-".into()));
                            ui.label(n.target_dev.clone().unwrap_or_else(|| "-".into()));
                            ui.end_row();
                        }
                    });
            });
        egui::CollapsingHeader::new(format!("Graphics ({})", x.graphics.len()))
            .default_open(true)
            .show(ui, |ui| {
                egui::Grid::new("gfx")
                    .num_columns(5)
                    .striped(true)
                    .show(ui, |ui| {
                        for h in ["Type", "Listen", "Port", "TLS port", "Console"] {
                            ui.strong(h);
                        }
                        ui.end_row();
                        for (index, gr) in x.graphics.iter().enumerate() {
                            ui.label(&gr.kind);
                            ui.label(gr.listen.describe());
                            ui.label(gr.port.map_or_else(
                                || {
                                    if gr.autoport {
                                        "auto".into()
                                    } else {
                                        "-".into()
                                    }
                                },
                                |p| p.to_string(),
                            ));
                            ui.label(gr.tls_port.map_or_else(|| "-".into(), |p| p.to_string()));
                            let key = ConsoleKey {
                                host: host.to_string(),
                                domain: d.name.clone(),
                                index,
                            };
                            if let Some(a) = self.console_cell(ui, key, d, gr) {
                                action = Some(a);
                            }
                            ui.end_row();
                        }
                    });
            });
        let grouped = view::group_devices(&x.other_devices);
        egui::CollapsingHeader::new(format!("Other devices ({})", x.other_devices.len()))
            .default_open(false)
            .show(ui, |ui| {
                egui::Grid::new("other")
                    .num_columns(3)
                    .striped(true)
                    .show(ui, |ui| {
                        for (dev, n) in grouped {
                            ui.label(&dev.kind);
                            ui.label(&dev.summary);
                            ui.label(if n > 1 {
                                format!("×{n}")
                            } else {
                                String::new()
                            });
                            ui.end_row();
                        }
                    });
            });
        egui::CollapsingHeader::new("Raw XML")
            .default_open(false)
            .show(ui, |ui| {
                if ui.button("Copy to clipboard").clicked() {
                    ui.ctx().copy_text(x.xml.clone());
                }
                let mut text = x.xml.as_str();
                ui.add(
                    egui::TextEdit::multiline(&mut text)
                        .code_editor()
                        .desired_width(f32::INFINITY),
                );
            });
        action
    }

    fn contrast_control(&mut self, ui: &mut egui::Ui) {
        let level = self.store.config.ui.contrast;
        let current = match level {
            ContrastLevel::Aa => ContrastChoice::Aa,
            ContrastLevel::Aaa => ContrastChoice::Aaa,
            ContrastLevel::Max => ContrastChoice::Max,
            ContrastLevel::Custom(_) => ContrastChoice::Custom,
        };
        let mut choice = current;
        ui.label("Contrast:");
        egui::ComboBox::from_id_salt("contrast")
            .selected_text(level.label())
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut choice, ContrastChoice::Aa, "AA (4.5:1)");
                ui.selectable_value(&mut choice, ContrastChoice::Aaa, "AAA (7:1)");
                ui.selectable_value(&mut choice, ContrastChoice::Max, "Max (21:1)");
                ui.selectable_value(&mut choice, ContrastChoice::Custom, "Custom…");
            });
        if choice == ContrastChoice::Custom {
            let resp = ui.add(
                egui::DragValue::new(&mut self.custom_ratio)
                    .range(MIN_RATIO..=MAX_RATIO)
                    .speed(0.1)
                    .fixed_decimals(1)
                    .suffix(":1"),
            );
            if choice != current || resp.changed() {
                let (lvl, _) = ContrastLevel::custom(self.custom_ratio);
                self.set_contrast(ui.ctx(), lvl);
            }
        } else if choice != current {
            let lvl = match choice {
                ContrastChoice::Aa => ContrastLevel::Aa,
                ContrastChoice::Aaa => ContrastLevel::Aaa,
                _ => ContrastLevel::Max,
            };
            self.set_contrast(ui.ctx(), lvl);
        }
        ui.label(format!("(lowest pair {:.1}:1)", self.palette.worst_ratio()));
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }
        let mut open = true;
        egui::Window::new("Settings")
            .open(&mut open)
            .resizable(false)
            .default_width(560.0)
            .max_width(560.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| self.contrast_control(ui));
                ui.add_space(8.0);
                egui::Grid::new("settings").num_columns(2).show(ui, |ui| {
                    ui.label("Config file");
                    ui.add(egui::Label::new(self.store.path.display().to_string()).wrap());
                    ui.end_row();
                    ui.label("SSH timeouts");
                    ui.label(format!(
                        "connect {} s, command {} s (edit in the config file)",
                        self.store.config.ssh.connect_timeout_secs,
                        self.store.config.ssh.command_timeout_secs
                    ));
                    ui.end_row();
                    ui.label("Local libvirt");
                    ui.add(
                        egui::Label::new(match &self.local_virsh {
                            Some(p) => format!(
                                "virsh found at {}; [local] enabled = {:?}",
                                p.display(),
                                self.store.config.local.enabled
                            ),
                            None => format!(
                                "virsh not found on PATH; [local] enabled = {:?}",
                                self.store.config.local.enabled
                            ),
                        })
                        .wrap(),
                    );
                    ui.end_row();
                    ui.label("Terminal (Start SSH)");
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            let r = ui.add(
                                egui::TextEdit::singleline(&mut self.terminal_text)
                                    .desired_width(330.0)
                                    .hint_text("automatic"),
                            );
                            let enter =
                                r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                            if self
                                .button(ui, true, "Apply")
                                .on_hover_text(
                                    "Program and arguments, e.g. kitty -e; the ssh command is added. \
                                     Empty: detected automatically.",
                                )
                                .clicked()
                                || enter
                            {
                                self.apply_terminal_setting();
                            }
                        });
                        // One line, so a new note never changes the layout.
                        ui.add(
                            egui::Label::new(
                                RichText::new(&self.terminal_note)
                                    .color(theme::c(self.palette.weak_text)),
                            )
                            .truncate(),
                        );
                    });
                    ui.end_row();
                    ui.label("Cache");
                    ui.add(
                        egui::Label::new(
                            self.cache
                                .as_ref()
                                .map_or_else(|| "none".into(), |c| c.dir().display().to_string()),
                        )
                        .wrap(),
                    );
                    ui.end_row();
                });
                if self.cache.is_some() && ui.button("Clear cache").clicked() {
                    self.clear_cache();
                }
            });
        self.settings_open = open;
    }

    fn host_form_window(&mut self, ctx: &egui::Context) {
        let Some(form) = &mut self.host_form else {
            return;
        };
        let title = if form.original.is_some() {
            "Edit host"
        } else {
            "Add host"
        };
        let mut save = false;
        let mut cancel = false;
        let err_color = theme::c(self.palette.state_error);
        egui::Modal::new(egui::Id::new("host-form"))
            .backdrop_color(Color32::TRANSPARENT)
            .show(ctx, |ui| {
                ui.set_width(460.0);
                ui.heading(title);
                egui::Grid::new("host-form-grid")
                    .num_columns(2)
                    .show(ui, |ui| {
                        ui.label("Backend");
                        egui::ComboBox::from_id_salt("host-form-backend")
                            .width(280.0)
                            .selected_text(backend_label(form.backend))
                            .show_ui(ui, |ui| {
                                for b in [Backend::Libvirt, Backend::Vsphere] {
                                    ui.selectable_value(&mut form.backend, b, backend_label(b));
                                }
                            });
                        ui.end_row();
                        let vsphere = form.backend == Backend::Vsphere;
                        // Every row stays in place for both backends; rows
                        // that do not apply are disabled (nothing moves).
                        let mut field =
                            |label: &str, value: &mut String, hint: &str, enabled: bool| {
                                ui.label(label);
                                ui.add_enabled(
                                    enabled,
                                    egui::TextEdit::singleline(value)
                                        .hint_text(hint)
                                        .desired_width(280.0),
                                );
                                ui.end_row();
                            };
                        field("Name *", &mut form.name, "display name, unique", true);
                        if vsphere {
                            field(
                                "Host *",
                                &mut form.host,
                                "ESXi or vCenter host name or IP",
                                true,
                            );
                            field("Port", &mut form.port, "443", true);
                            field("User *", &mut form.user, "e.g. reader@vsphere.local", true);
                        } else {
                            field(
                                "Host *",
                                &mut form.host,
                                "hostname, IP or ~/.ssh/config alias",
                                true,
                            );
                            field("Port", &mut form.port, "from ssh config, else 22", true);
                            field(
                                "User",
                                &mut form.user,
                                "from ssh config, else local user",
                                true,
                            );
                        }
                        let ssh_hint =
                            |h: &'static str| if vsphere { "not used for vSphere" } else { h };
                        field(
                            "Identity file",
                            &mut form.identity_file,
                            ssh_hint("agent / ssh config / default keys"),
                            !vsphere,
                        );
                        field(
                            "libvirt URI",
                            &mut form.libvirt_uri,
                            ssh_hint("qemu:///system"),
                            !vsphere,
                        );
                        field(
                            "virsh prefix",
                            &mut form.virsh_prefix,
                            ssh_hint("e.g. sudo -n"),
                            !vsphere,
                        );
                    });
                if let Some(e) = &form.error {
                    ui.label(RichText::new(e).color(err_color));
                }
                ui.horizontal(|ui| {
                    save = ui.button("Save").clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });
        if cancel {
            self.host_form = None;
        } else if save {
            self.submit_host_form();
        }
    }

    fn remove_dialog(&mut self, ctx: &egui::Context) {
        let Some(name) = self.confirm_remove.clone() else {
            return;
        };
        let mut answer = None;
        egui::Modal::new(egui::Id::new("remove-host"))
            .backdrop_color(Color32::TRANSPARENT)
            .show(ctx, |ui| {
                ui.label(format!(
                    "Remove host \"{name}\" from the config? Its cached inventory is deleted too."
                ));
                ui.horizontal(|ui| {
                    if ui.button("Remove").clicked() {
                        answer = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        answer = Some(false);
                    }
                });
            });
        if let Some(yes) = answer {
            self.confirm_remove = None;
            if yes {
                self.remove_host(&name);
            }
        }
    }

    fn prompt_dialog(&mut self, ctx: &egui::Context) {
        let Some(p) = &mut self.open_prompt else {
            return;
        };
        enum Answer {
            HostKey(bool),
            Passphrase(bool),
            Certificate(bool),
            Password(bool),
        }
        let mut answer = None;
        let weak = theme::c(self.palette.weak_text);
        egui::Modal::new(egui::Id::new("ssh-prompt"))
            .backdrop_color(Color32::TRANSPARENT)
            .show(ctx, |ui| {
                ui.set_width(460.0);
                match &p.request.kind {
                    PromptKind::HostKey(k) => {
                        ui.heading("Unknown host key");
                        ui.label(format!(
                            "The authenticity of host \"{}\" ({} port {}) can't be established.",
                            k.name, k.hostname, k.port
                        ));
                        ui.label(format!("{} key fingerprint:", k.key_type));
                        ui.monospace(&k.fingerprint);
                        if !k.other_types.is_empty() {
                            ui.label(format!(
                                "known_hosts already has keys of other types for this host: {}",
                                k.other_types.join(", ")
                            ));
                        }
                        ui.label(
                            RichText::new(
                                "Compare the fingerprint with the host before accepting.",
                            )
                            .color(weak),
                        );
                        ui.horizontal(|ui| {
                            if ui.button("Accept and remember").clicked() {
                                answer = Some(Answer::HostKey(true));
                            }
                            if ui.button("Reject").clicked() {
                                answer = Some(Answer::HostKey(false));
                            }
                        });
                    }
                    PromptKind::Passphrase(path) => {
                        ui.heading("Key passphrase");
                        ui.label(format!("Passphrase for {}", path.display()));
                        let r = ui.add(
                            egui::TextEdit::singleline(&mut p.passphrase)
                                .password(true)
                                .desired_width(280.0),
                        );
                        r.request_focus();
                        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        ui.horizontal(|ui| {
                            if ui.button("OK").clicked() || enter {
                                answer = Some(Answer::Passphrase(true));
                            }
                            if ui.button("Skip this key").clicked() {
                                answer = Some(Answer::Passphrase(false));
                            }
                        });
                    }
                    PromptKind::Certificate(c) => {
                        ui.heading("Unknown server certificate");
                        ui.label(format!(
                            "The certificate of \"{}\" ({}) is not trusted by this system.",
                            c.name, c.target
                        ));
                        egui::Grid::new("cert-grid").num_columns(2).show(ui, |ui| {
                            for (k, v) in [
                                ("Subject", &c.subject),
                                ("Issuer", &c.issuer),
                                ("Valid from", &c.not_before),
                                ("Valid until", &c.not_after),
                            ] {
                                ui.label(k);
                                ui.add(egui::Label::new(v.as_str()).wrap());
                                ui.end_row();
                            }
                        });
                        ui.label("SHA-256 fingerprint:");
                        ui.add(egui::Label::new(RichText::new(&c.fingerprint).monospace()).wrap());
                        ui.label(
                            RichText::new(
                                "Compare the fingerprint with the server (ESXi: Host Client → \
                                 Manage → Security → Certificates) before accepting.",
                            )
                            .color(weak),
                        );
                        ui.horizontal(|ui| {
                            if ui.button("Accept and remember").clicked() {
                                answer = Some(Answer::Certificate(true));
                            }
                            if ui.button("Reject").clicked() {
                                answer = Some(Answer::Certificate(false));
                            }
                        });
                    }
                    PromptKind::Password(w) => {
                        ui.heading("vSphere login");
                        if w.retry {
                            ui.label(
                                RichText::new("Login failed. Check the user and password.")
                                    .color(theme::c(self.palette.state_error)),
                            );
                        }
                        ui.label(format!(
                            "Password of {} on \"{}\" ({})",
                            w.user, w.name, w.target
                        ));
                        let r = ui.add(
                            egui::TextEdit::singleline(&mut p.passphrase)
                                .password(true)
                                .desired_width(280.0),
                        );
                        r.request_focus();
                        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        ui.label(RichText::new("Used once to log in; never stored.").color(weak));
                        ui.horizontal(|ui| {
                            if ui.button("Log in").clicked() || enter {
                                answer = Some(Answer::Password(true));
                            }
                            if ui.button("Cancel").clicked() {
                                answer = Some(Answer::Password(false));
                            }
                        });
                    }
                }
            });
        if let Some(a) = answer {
            let OpenPrompt {
                request,
                mut passphrase,
            } = self.open_prompt.take().expect("prompt");
            // Always move the buffer into a Secret, which wipes it on drop,
            // also when the key is skipped (C3).
            let secret = Secret::new(std::mem::take(&mut passphrase));
            match a {
                Answer::HostKey(yes) => request.answer_host_key(yes),
                Answer::Passphrase(true) if !secret.expose().is_empty() => {
                    request.answer_passphrase(Some(secret))
                }
                Answer::Passphrase(_) => {
                    drop(secret);
                    request.answer_passphrase(None)
                }
                Answer::Certificate(yes) => request.answer_certificate(yes),
                Answer::Password(true) if !secret.expose().is_empty() => {
                    request.answer_password(Some(secret))
                }
                Answer::Password(_) => {
                    drop(secret);
                    request.answer_password(None)
                }
            }
        }
    }
}

/// Id of the left panel with the tree (its width is in egui's memory).
pub const TREE_PANEL: &str = "tree";

/// Persistent id of a host's node in the tree (also read by the keyboard
/// navigation to know whether the host is expanded).
fn backend_label(b: Backend) -> &'static str {
    match b {
        Backend::Libvirt => "libvirt (virsh over SSH)",
        Backend::Vsphere => "VMware vSphere (ESXi or vCenter, HTTPS)",
    }
}

fn tree_id(host: &str) -> egui::Id {
    egui::Id::new(("host-tree", host))
}

/// Text colour of a button: normal, or the palette's disabled colour.
pub fn button_color(p: &Palette, enabled: bool) -> Color32 {
    theme::c(if enabled { p.text } else { p.disabled_text })
}

/// Grid rows about a host's inventory: versions, VM count, live/cached.
fn inventory_rows(ui: &mut egui::Ui, s: &Shown) {
    let inv = &s.inventory;
    row(
        ui,
        "libvirt",
        inv.library.clone().unwrap_or_else(|| "unknown".into()),
    );
    row(
        ui,
        "Hypervisor",
        inv.hypervisor.clone().unwrap_or_else(|| "unknown".into()),
    );
    row(ui, "VMs", inv.domains.len().to_string());
    row(
        ui,
        "Inventory",
        format!(
            "{}, taken {} ({})",
            if s.cached { "cached" } else { "live" },
            format_utc(s.taken_at),
            format_age(s.taken_at, now_secs())
        ),
    );
}

/// One label/value row of a two-column grid.
fn row(ui: &mut egui::Ui, k: &str, v: impl Into<egui::WidgetText>) {
    ui.label(k);
    ui.label(v);
    ui.end_row();
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.show(ui);
    }
}
