//! virt-interact desktop GUI (SPEC-003, SPEC-015).
//!
//! Phase 0: an empty window with the default white/black theme (SPEC-001).

use eframe::egui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(vi_core::APP_NAME)
            .with_inner_size([1024.0, 700.0]),
        ..Default::default()
    };
    eframe::run_native(
        vi_core::APP_NAME,
        options,
        Box::new(|cc| {
            // SPEC-001 default: white background, black text. The full
            // contrast-driven palette arrives in phase 1.
            let mut visuals = egui::Visuals::light();
            visuals.panel_fill = egui::Color32::WHITE;
            visuals.window_fill = egui::Color32::WHITE;
            visuals.override_text_color = Some(egui::Color32::BLACK);
            cc.egui_ctx.set_visuals(visuals);
            Ok(Box::new(App))
        }),
    )
}

struct App;

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui.heading(format!("{} {}", vi_core::APP_NAME, vi_core::VERSION));
        ui.label("Phase 0 skeleton. No hosts configured yet.");
    }
}
