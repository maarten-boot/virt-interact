//! Map a contrast [`Palette`] onto egui visuals (SPEC-001).
//!
//! Every colour egui draws comes from the palette, so every text/background
//! pair meets the palette's minimum ratio (TC-007). egui normally fades
//! disabled widgets to 50% opacity, and fades text at the edges of scroll
//! areas; both would break that, so both are turned off here ([`apply`]).
//! Use [`disabled_text`] to show something as disabled.

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

    // No fading of disabled widgets: it would drop text below the minimum.
    v.disabled_alpha = 1.0;

    v.selection.bg_fill = c(p.selection_bg);
    v.selection.stroke = Stroke::new(1.0, text);

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = c(p.window_bg);
    w.noninteractive.weak_bg_fill = c(p.panel_bg);
    w.noninteractive.bg_stroke = border;
    w.noninteractive.fg_stroke = Stroke::new(1.0, text);
    // Feedback is colour only (SPEC-015, crosscheck 2026-11-14-02 H1): every
    // state has the same border width and expansion, so hovering or
    // pressing never changes a widget's size or moves anything. egui sizes
    // a button as padding + expansion - stroke (inner margin, whole pixels)
    // + stroke + (-expansion); an unselected tree row drops its frame when
    // inactive and keeps only the inner margin. Both stay equal to the
    // padding only when expansion == stroke width, a whole number.
    for (wv, bg, border_col) in [
        (&mut w.inactive, p.widget_bg, c(p.border)),
        (&mut w.hovered, p.widget_hover_bg, text),
        (&mut w.active, p.widget_hover_bg, text),
        (&mut w.open, p.widget_bg, c(p.border)),
    ] {
        wv.bg_fill = c(bg);
        wv.weak_bg_fill = c(bg);
        wv.bg_stroke = Stroke::new(WIDGET_STROKE, border_col);
        wv.fg_stroke = Stroke::new(1.0, text);
        wv.expansion = WIDGET_STROKE;
    }
    v
}

/// Border width of interactive widgets in every state (see [`visuals`]).
pub const WIDGET_STROKE: f32 = 1.0;

/// Apply the palette to every egui style (light and dark): visuals from
/// [`visuals`], and no fading at scroll-area edges.
pub fn apply(ctx: &egui::Context, p: &Palette) {
    let v = visuals(p);
    ctx.all_styles_mut(|s| {
        s.visuals = v.clone();
        // Faded text at a scroll edge would drop below the minimum ratio.
        s.spacing.scroll.fade.strength = 0.0;
    });
}

/// Text in the palette's disabled colour, which meets the minimum ratio.
pub fn disabled_text(p: &Palette, text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text).color(c(p.disabled_text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vi_core::contrast::{ContrastLevel, contrast_ratio};

    fn rgb(c: Color32) -> Rgb {
        assert_eq!(c.a(), 255, "colour {c:?} is not opaque");
        Rgb(c.r(), c.g(), c.b())
    }

    type Named = Vec<(String, Color32)>;

    /// (name, foreground) and (name, background) colours egui uses.
    fn colours(v: &egui::Visuals) -> (Named, Named) {
        let mut fg = vec![
            (
                "override_text_color".to_string(),
                v.override_text_color.unwrap(),
            ),
            ("weak_text_color".into(), v.weak_text_color.unwrap()),
            ("hyperlink_color".into(), v.hyperlink_color),
            ("warn_fg_color".into(), v.warn_fg_color),
            ("error_fg_color".into(), v.error_fg_color),
            ("window_stroke".into(), v.window_stroke.color),
            ("text_cursor".into(), v.text_cursor.stroke.color),
            ("selection.stroke".into(), v.selection.stroke.color),
        ];
        let mut bg = vec![
            ("window_fill".to_string(), v.window_fill),
            ("panel_fill".into(), v.panel_fill),
            ("extreme_bg_color".into(), v.extreme_bg_color),
            ("text_edit_bg_color".into(), v.text_edit_bg_color.unwrap()),
            ("faint_bg_color".into(), v.faint_bg_color),
            ("code_bg_color".into(), v.code_bg_color),
            ("selection.bg_fill".into(), v.selection.bg_fill),
        ];
        let w = &v.widgets;
        for (name, wv) in [
            ("noninteractive", &w.noninteractive),
            ("inactive", &w.inactive),
            ("hovered", &w.hovered),
            ("active", &w.active),
            ("open", &w.open),
        ] {
            fg.push((format!("{name}.fg_stroke"), wv.fg_stroke.color));
            fg.push((format!("{name}.bg_stroke"), wv.bg_stroke.color));
            bg.push((format!("{name}.bg_fill"), wv.bg_fill));
            bg.push((format!("{name}.weak_bg_fill"), wv.weak_bg_fill));
        }
        (fg, bg)
    }

    // TC-007
    #[test]
    fn every_egui_pair_meets_minimum() {
        for lvl in [
            ContrastLevel::Aa,
            ContrastLevel::Aaa,
            ContrastLevel::Max,
            ContrastLevel::Custom(5.0),
            ContrastLevel::Custom(12.0),
        ] {
            let p = Palette::for_level(lvl);
            let v = visuals(&p);
            assert_eq!(v.disabled_alpha, 1.0, "disabled widgets must not fade");
            let (fg, bg) = colours(&v);
            for (fname, f) in &fg {
                for (bname, b) in &bg {
                    let r = contrast_ratio(rgb(*f), rgb(*b));
                    assert!(
                        r + 1e-9 >= lvl.min_ratio(),
                        "{lvl:?}: {fname} on {bname} = {r:.2}"
                    );
                }
            }
            // Disabled text helper uses a palette colour too.
            for (_, b) in &bg {
                assert!(contrast_ratio(p.disabled_text, rgb(*b)) + 1e-9 >= lvl.min_ratio());
            }
        }
    }

    // TC-007b: hover/press feedback is colour only; size never changes.
    #[test]
    fn widget_states_have_equal_geometry() {
        for lvl in [ContrastLevel::Aa, ContrastLevel::Aaa, ContrastLevel::Max] {
            let v = visuals(&Palette::for_level(lvl));
            let w = &v.widgets;
            for (name, wv) in [
                ("inactive", &w.inactive),
                ("hovered", &w.hovered),
                ("active", &w.active),
                ("open", &w.open),
            ] {
                assert_eq!(wv.bg_stroke.width, WIDGET_STROKE, "{name} border width");
                assert_eq!(wv.expansion, WIDGET_STROKE, "{name} expansion");
            }
            assert_eq!(WIDGET_STROKE.fract(), 0.0, "whole pixels only");
        }
    }

    /// Rect of a button and a selectable label, with the pointer away from
    /// or on them (two frames each: egui styles by last frame's state).
    fn rects(pointer: Option<egui::Pos2>, pressed: bool) -> Vec<egui::Rect> {
        let ctx = egui::Context::default();
        apply(&ctx, &Palette::for_level(ContrastLevel::Aa));
        let mut out = Vec::new();
        for _ in 0..3 {
            let mut input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400.0, 300.0),
                )),
                ..Default::default()
            };
            if let Some(p) = pointer {
                input.events.push(egui::Event::PointerMoved(p));
                if pressed {
                    input.events.push(egui::Event::PointerButton {
                        pos: p,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: Default::default(),
                    });
                }
            }
            out.clear();
            let _ = ctx.run_ui(input, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    out.push(ui.button("Refresh").rect);
                    out.push(ui.selectable_label(false, "some-vm-name").rect);
                    out.push(ui.selectable_label(true, "selected-vm").rect);
                    out.push(ui.label("after").rect);
                });
            });
        }
        out
    }

    // TC-007b: hovering or pressing a button or a tree row keeps every
    // widget's rect (its own and the next one's) where it was.
    #[test]
    fn hover_and_press_do_not_move_anything() {
        let rest = rects(None, false);
        for (i, r) in rest.iter().take(3).enumerate() {
            for pressed in [false, true] {
                let now = rects(Some(r.center()), pressed);
                assert_eq!(now, rest, "widget {i}, pressed {pressed}");
            }
        }
    }

    // TC-001 at the egui level
    #[test]
    fn default_visuals_white_and_black() {
        let v = visuals(&Palette::default_palette());
        assert_eq!(v.panel_fill, Color32::WHITE);
        assert_eq!(v.override_text_color, Some(Color32::BLACK));
    }

    // TC-007: no fading anywhere (disabled widgets, scroll edges).
    #[test]
    fn apply_turns_off_all_fading() {
        let ctx = egui::Context::default();
        apply(&ctx, &Palette::for_level(ContrastLevel::Aa));
        ctx.all_styles_mut(|s| {
            assert_eq!(s.spacing.scroll.fade.strength, 0.0);
            assert_eq!(s.visuals.disabled_alpha, 1.0);
            assert_eq!(s.visuals.panel_fill, Color32::WHITE);
        });
    }
}
