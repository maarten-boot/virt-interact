//! virt-interact desktop GUI (SPEC-003, SPEC-015).
//!
//! Phase 1: loads the config, sets up logging and applies the contrast theme
//! (SPEC-001). The contrast control is in the toolbar; changes are saved to
//! the config file.

mod theme;

use eframe::egui;
use vi_core::config::ConfigStore;
use vi_core::contrast::{ContrastLevel, MAX_RATIO, MIN_RATIO, Palette};
use vi_core::paths::{self, CliArgs};

fn main() -> eframe::Result {
    let args = match CliArgs::parse(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}\n\n{}", paths::USAGE);
            std::process::exit(2);
        }
    };
    if args.show_help {
        println!("{}", paths::USAGE);
        return Ok(());
    }
    if args.show_version {
        println!("{} {}", vi_core::APP_NAME, vi_core::VERSION);
        return Ok(());
    }
    let Some(config_path) = args.config_path() else {
        eprintln!("cannot determine the config directory; use --config <PATH>");
        std::process::exit(2);
    };

    let (store, warnings, load_error) = ConfigStore::open(config_path);

    let _log_guard = paths::log_dir().and_then(|dir| {
        vi_core::logging::init(&dir, &store.config.logging.level)
            .map_err(|e| eprintln!("logging disabled: {e}"))
            .ok()
    });
    tracing::info!(config = %store.path.display(), "starting {} {}", vi_core::APP_NAME, vi_core::VERSION);

    let mut messages: Vec<Message> = Vec::new();
    for w in warnings {
        tracing::warn!("{w}");
        messages.push(Message::warning(format!("Warning: {w}")));
    }
    if let Some(e) = load_error {
        tracing::error!("{e}");
        messages.push(Message::error(format!(
            "Config error: {e}. Using defaults; changes will not be saved."
        )));
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(vi_core::APP_NAME)
            .with_inner_size([1024.0, 700.0]),
        ..Default::default()
    };
    eframe::run_native(
        vi_core::APP_NAME,
        options,
        Box::new(move |cc| {
            let app = App::new(store, messages);
            app.apply_theme(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    )
}

/// A message shown at the top of the window until dismissed.
struct Message {
    text: String,
    is_error: bool,
}

impl Message {
    fn warning(text: String) -> Message {
        Message {
            text,
            is_error: false,
        }
    }
    fn error(text: String) -> Message {
        Message {
            text,
            is_error: true,
        }
    }
}

/// Which contrast option the toolbar shows.
#[derive(Debug, Clone, Copy, PartialEq)]
enum ContrastChoice {
    Aa,
    Aaa,
    Max,
    Custom,
}

struct App {
    store: ConfigStore,
    palette: Palette,
    messages: Vec<Message>,
    /// Ratio edited in the toolbar while "Custom" is selected.
    custom_ratio: f64,
}

impl App {
    fn new(store: ConfigStore, messages: Vec<Message>) -> App {
        let level = store.config.ui.contrast;
        let custom_ratio = match level {
            ContrastLevel::Custom(r) => r,
            _ => 10.0,
        };
        App {
            palette: Palette::for_level(level),
            store,
            messages,
            custom_ratio,
        }
    }

    fn apply_theme(&self, ctx: &egui::Context) {
        ctx.set_visuals(theme::visuals(&self.palette));
    }

    /// Change contrast: re-theme at once and save to the config (TC-006).
    fn set_contrast(&mut self, ctx: &egui::Context, level: ContrastLevel) {
        if self.store.config.ui.contrast == level {
            return;
        }
        self.store.config.ui.contrast = level;
        self.palette = Palette::for_level(level);
        self.apply_theme(ctx);
        tracing::info!("contrast set to {}", level.label());
        if let Err(e) = self.store.save() {
            tracing::warn!("{e}");
            self.messages
                .push(Message::error(format!("Not saved: {e}")));
        }
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
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.strong(format!("{} {}", vi_core::APP_NAME, vi_core::VERSION));
                ui.separator();
                self.contrast_control(ui);
            });
        });
        egui::CentralPanel::default().show(ui, |ui| {
            let mut dismiss = None;
            for (i, m) in self.messages.iter().enumerate() {
                ui.horizontal_wrapped(|ui| {
                    let color = if m.is_error {
                        theme::c(self.palette.state_error)
                    } else {
                        theme::c(self.palette.state_paused)
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
            ui.label(format!("Config file: {}", self.store.path.display()));
            ui.label(format!(
                "Hosts configured: {}",
                self.store.config.hosts.len()
            ));
            ui.weak("Host list and inventory arrive in phase 4.");
        });
    }
}
