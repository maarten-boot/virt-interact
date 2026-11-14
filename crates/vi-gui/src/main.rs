//! virt-interact: command line, config, logging, then the GUI (SPEC-003).

// Windows release builds are GUI programs: no console window behind the
// app. `--help`, `--version` and start-up errors are then shown in a small
// window (`startup::exit_with`); debug builds keep the console.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use vi_core::cache::Cache;
use vi_core::config::ConfigStore;
use vi_core::paths;
use vi_gui::app::{App, Message};
use vi_gui::startup::{self, Start};

fn main() -> eframe::Result {
    let config_path = match startup::start(std::env::args().skip(1)) {
        Start::Run(p) => p,
        Start::Exit { text, code } => startup::exit_with(&text, code),
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
        viewport: startup::viewport().with_inner_size([1024.0, 700.0]),
        ..Default::default()
    };
    eframe::run_native(
        vi_core::APP_NAME,
        options,
        Box::new(move |cc| {
            let cache = paths::cache_dir().map(Cache::new);
            let app = App::new(store, messages, cache)
                .with_local_detection()
                .with_ssh(&cc.egui_ctx);
            app.apply_theme(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    )
}
