//! Map a contrast [`Palette`] onto egui visuals (SPEC-001).

use eframe::egui::{self, Color32, Stroke};
use vi_core::contrast::{Palette, Rgb};

pub fn c(rgb: Rgb) -> Color32 {
    Color32::from_rgb(rgb.0, rgb.1, rgb.2)
}

/// Light visuals where every text/background pair comes from `p`.
pub fn visuals(p: &Palette) -> egui::Visuals {
    let mut v = egui::Visuals::light();
    let text = c(p.text);
    let border = Stroke::new(1.0, c(p.border));

    v.override_text_color = Some(text);
    v.weak_text_color = Some(c(p.weak_text));
    v.hyperlink_color = c(p.hyperlink);
    v.warn_fg_color = c(p.state_paused);
    v.error_fg_color = c(p.state_error);

    v.window_fill = c(p.window_bg);
    v.panel_fill = c(p.window_bg);
    v.extreme_bg_color = c(p.window_bg);
    v.text_edit_bg_color = Some(c(p.window_bg));
    v.faint_bg_color = c(p.panel_bg);
    v.code_bg_color = c(p.widget_bg);
    v.window_stroke = border;
    v.text_cursor.stroke = Stroke::new(2.0, text);

    v.selection.bg_fill = c(p.selection_bg);
    v.selection.stroke = Stroke::new(1.0, text);

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = c(p.window_bg);
    w.noninteractive.weak_bg_fill = c(p.panel_bg);
    w.noninteractive.bg_stroke = border;
    w.noninteractive.fg_stroke = Stroke::new(1.0, text);
    for (wv, bg, stroke_w) in [
        (&mut w.inactive, p.widget_bg, 1.0),
        (&mut w.hovered, p.widget_hover_bg, 1.5),
        (&mut w.active, p.widget_hover_bg, 2.0),
        (&mut w.open, p.widget_bg, 1.0),
    ] {
        wv.bg_fill = c(bg);
        wv.weak_bg_fill = c(bg);
        wv.bg_stroke = Stroke::new(stroke_w, if stroke_w > 1.0 { text } else { c(p.border) });
        wv.fg_stroke = Stroke::new(1.0, text);
    }
    v
}
