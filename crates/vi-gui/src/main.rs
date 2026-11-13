//! virt-interact: command line, config, logging, then the GUI (SPEC-003).

use eframe::egui;
use vi_core::cache::Cache;
use vi_core::config::ConfigStore;
use vi_core::paths::{self, CliArgs};
use vi_gui::app::{App, Message};

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
            let cache = paths::cache_dir().map(Cache::new);
            let app = App::new(store, messages, cache).with_ssh(&cc.egui_ctx);
            app.apply_theme(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    )
}
