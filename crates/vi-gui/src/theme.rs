//! Map a contrast [`Palette`] onto egui visuals (SPEC-001).
//!
//! Every colour egui draws comes from the palette, so every text/background
//! pair meets the palette's minimum ratio (TC-007). egui normally fades
//! disabled widgets to 50% opacity, which would break that; fading is turned
//! off here. Use [`disabled_text`] to show something as disabled.

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

/// Text in the palette's disabled colour, which meets the minimum ratio.
#[allow(dead_code)] // used from phase 4 on
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

    // TC-001 at the egui level
    #[test]
    fn default_visuals_white_and_black() {
        let v = visuals(&Palette::default_palette());
        assert_eq!(v.panel_fill, Color32::WHITE);
        assert_eq!(v.override_text_color, Some(Color32::BLACK));
    }
}
