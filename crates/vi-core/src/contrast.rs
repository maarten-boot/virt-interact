//! WCAG contrast maths, contrast levels and palette generation (SPEC-001).
//!
//! The palette is "dark on light": all backgrounds are light, all foreground
//! colours are dark. [`Palette::for_min_ratio`] adjusts the base colours so
//! that *every* foreground/background pair reaches the requested minimum
//! ratio: backgrounds are lightened toward white and foregrounds darkened
//! toward black as far as needed. At 21:1 (Max) this yields pure black on
//! pure white.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Lowest allowed minimum ratio (WCAG AA for normal text).
pub const MIN_RATIO: f64 = 4.5;
/// Highest possible ratio (black on white).
pub const MAX_RATIO: f64 = 21.0;
/// Tolerance for floating point comparisons of ratios.
const EPS: f64 = 1e-9;

/// An sRGB colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub const WHITE: Rgb = Rgb(0xFF, 0xFF, 0xFF);
    pub const BLACK: Rgb = Rgb(0x00, 0x00, 0x00);

    /// WCAG 2.x relative luminance (0.0 = black, 1.0 = white).
    pub fn relative_luminance(self) -> f64 {
        fn lin(c: u8) -> f64 {
            let c = f64::from(c) / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        }
        0.2126 * lin(self.0) + 0.7152 * lin(self.1) + 0.0722 * lin(self.2)
    }

    /// Linear mix in sRGB space: `t = 0` gives `self`, `t = 1` gives `other`.
    fn mix(self, other: Rgb, t: f64) -> Rgb {
        let m = |a: u8, b: u8| {
            (f64::from(a) + (f64::from(b) - f64::from(a)) * t)
                .round()
                .clamp(0.0, 255.0) as u8
        };
        Rgb(m(self.0, other.0), m(self.1, other.1), m(self.2, other.2))
    }
}

impl fmt::Display for Rgb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02X}{:02X}{:02X}", self.0, self.1, self.2)
    }
}

/// WCAG contrast ratio between two colours (1.0 ..= 21.0, order independent).
pub fn contrast_ratio(a: Rgb, b: Rgb) -> f64 {
    let (la, lb) = (a.relative_luminance(), b.relative_luminance());
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// Requested minimum contrast: a preset or a custom ratio.
///
/// In the config file: `contrast = "AA" | "AAA" | "Max" | <number>`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum ContrastLevel {
    /// 4.5:1
    #[default]
    Aa,
    /// 7:1
    Aaa,
    /// 21:1, pure black on white
    Max,
    /// Custom minimum ratio, 4.5 ..= 21.
    Custom(f64),
}

impl ContrastLevel {
    /// The minimum ratio this level requires.
    pub fn min_ratio(self) -> f64 {
        match self {
            ContrastLevel::Aa => 4.5,
            ContrastLevel::Aaa => 7.0,
            ContrastLevel::Max => MAX_RATIO,
            ContrastLevel::Custom(r) => r.clamp(MIN_RATIO, MAX_RATIO),
        }
    }

    /// Build a custom level, clamping to 4.5 ..= 21.
    /// Returns the level and a warning if the value had to be clamped (TC-004).
    pub fn custom(ratio: f64) -> (ContrastLevel, Option<String>) {
        if ratio.is_nan() {
            (
                ContrastLevel::Custom(MIN_RATIO),
                Some(format!("contrast is not a number; using {MIN_RATIO}")),
            )
        } else if ratio < MIN_RATIO {
            (
                ContrastLevel::Custom(MIN_RATIO),
                Some(format!(
                    "contrast {ratio} is below the minimum {MIN_RATIO}; using {MIN_RATIO}"
                )),
            )
        } else if ratio > MAX_RATIO {
            (
                ContrastLevel::Custom(MAX_RATIO),
                Some(format!(
                    "contrast {ratio} is above the maximum {MAX_RATIO}; using {MAX_RATIO}"
                )),
            )
        } else {
            (ContrastLevel::Custom(ratio), None)
        }
    }

    /// Short label for the GUI.
    pub fn label(self) -> String {
        match self {
            ContrastLevel::Aa => "AA (4.5:1)".into(),
            ContrastLevel::Aaa => "AAA (7:1)".into(),
            ContrastLevel::Max => "Max (21:1)".into(),
            ContrastLevel::Custom(r) => format!("Custom ({r:.1}:1)"),
        }
    }
}

impl Serialize for ContrastLevel {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            ContrastLevel::Aa => s.serialize_str("AA"),
            ContrastLevel::Aaa => s.serialize_str("AAA"),
            ContrastLevel::Max => s.serialize_str("Max"),
            ContrastLevel::Custom(r) => s.serialize_f64(*r),
        }
    }
}

/// Raw form as it appears in the config file, before clamping.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum RawContrast {
    Name(String),
    Int(i64),
    Float(f64),
}

impl RawContrast {
    /// Convert to a level. Unknown names are an error; out-of-range numbers
    /// are clamped with a warning.
    pub fn resolve(&self) -> Result<(ContrastLevel, Option<String>), String> {
        match self {
            RawContrast::Name(n) => match n.to_ascii_lowercase().as_str() {
                "aa" => Ok((ContrastLevel::Aa, None)),
                "aaa" => Ok((ContrastLevel::Aaa, None)),
                "max" => Ok((ContrastLevel::Max, None)),
                _ => Err(format!(
                    "invalid contrast \"{n}\": expected \"AA\", \"AAA\", \"Max\" or a number >= {MIN_RATIO}"
                )),
            },
            RawContrast::Int(i) => Ok(ContrastLevel::custom(*i as f64)),
            RawContrast::Float(f) => Ok(ContrastLevel::custom(*f)),
        }
    }
}

impl<'de> Deserialize<'de> for ContrastLevel {
    /// Deserialising directly clamps silently; the config loader uses
    /// [`RawContrast`] instead so it can report warnings.
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = RawContrast::deserialize(d)?;
        raw.resolve()
            .map(|(lvl, _)| lvl)
            .map_err(serde::de::Error::custom)
    }
}

/// Colour roles used by the GUI. All foregrounds are checked against all
/// backgrounds.
#[derive(Debug, Clone, PartialEq)]
pub struct Palette {
    // backgrounds (light)
    pub window_bg: Rgb,
    pub panel_bg: Rgb,
    pub widget_bg: Rgb,
    pub widget_hover_bg: Rgb,
    pub selection_bg: Rgb,
    // foregrounds (dark)
    pub text: Rgb,
    pub weak_text: Rgb,
    pub disabled_text: Rgb,
    pub hyperlink: Rgb,
    pub border: Rgb,
    pub state_running: Rgb,
    pub state_paused: Rgb,
    pub state_stopped: Rgb,
    pub state_error: Rgb,
    /// The minimum ratio this palette was generated for.
    pub min_ratio: f64,
}

impl Palette {
    /// Base design colours. White background and black text (SPEC-001).
    fn base() -> Palette {
        Palette {
            window_bg: Rgb::WHITE,
            panel_bg: Rgb(0xF7, 0xF7, 0xF7),
            widget_bg: Rgb(0xEE, 0xEE, 0xEE),
            widget_hover_bg: Rgb(0xE2, 0xE2, 0xE2),
            selection_bg: Rgb(0xCC, 0xE0, 0xFF),
            text: Rgb::BLACK,
            weak_text: Rgb(0x55, 0x55, 0x55),
            disabled_text: Rgb(0x6B, 0x6B, 0x6B),
            hyperlink: Rgb(0x06, 0x45, 0xAD),
            border: Rgb(0x60, 0x60, 0x60),
            state_running: Rgb(0x1B, 0x6E, 0x20),
            state_paused: Rgb(0x8A, 0x53, 0x00),
            state_stopped: Rgb(0x4A, 0x4A, 0x4A),
            state_error: Rgb(0xB0, 0x00, 0x20),
            min_ratio: MIN_RATIO,
        }
    }

    /// Palette for a contrast level.
    pub fn for_level(level: ContrastLevel) -> Palette {
        Palette::for_min_ratio(level.min_ratio())
    }

    /// Palette where every fg/bg pair reaches at least `min` (clamped to
    /// 4.5 ..= 21).
    pub fn for_min_ratio(min: f64) -> Palette {
        let min = min.clamp(MIN_RATIO, MAX_RATIO);
        let mut p = Palette::base();
        p.min_ratio = min;

        // 1. Lighten each background until black text on it reaches `min`.
        for bg in p.backgrounds_mut() {
            *bg = adjust(*bg, Rgb::WHITE, |c| {
                contrast_ratio(Rgb::BLACK, c) + EPS >= min
            });
        }
        // 2. Darken each foreground until it reaches `min` on the darkest background.
        let darkest = p
            .backgrounds()
            .into_iter()
            .min_by(|a, b| a.relative_luminance().total_cmp(&b.relative_luminance()))
            .unwrap_or(Rgb::WHITE);
        for fg in p.foregrounds_mut() {
            *fg = adjust(*fg, Rgb::BLACK, |c| contrast_ratio(c, darkest) + EPS >= min);
        }
        p
    }

    /// The default palette: white/black at AA.
    pub fn default_palette() -> Palette {
        Palette::for_level(ContrastLevel::Aa)
    }

    pub fn backgrounds(&self) -> Vec<Rgb> {
        vec![
            self.window_bg,
            self.panel_bg,
            self.widget_bg,
            self.widget_hover_bg,
            self.selection_bg,
        ]
    }

    pub fn foregrounds(&self) -> Vec<Rgb> {
        vec![
            self.text,
            self.weak_text,
            self.disabled_text,
            self.hyperlink,
            self.border,
            self.state_running,
            self.state_paused,
            self.state_stopped,
            self.state_error,
        ]
    }

    fn backgrounds_mut(&mut self) -> [&mut Rgb; 5] {
        [
            &mut self.window_bg,
            &mut self.panel_bg,
            &mut self.widget_bg,
            &mut self.widget_hover_bg,
            &mut self.selection_bg,
        ]
    }

    fn foregrounds_mut(&mut self) -> [&mut Rgb; 9] {
        [
            &mut self.text,
            &mut self.weak_text,
            &mut self.disabled_text,
            &mut self.hyperlink,
            &mut self.border,
            &mut self.state_running,
            &mut self.state_paused,
            &mut self.state_stopped,
            &mut self.state_error,
        ]
    }

    /// The lowest ratio over all fg/bg pairs.
    pub fn worst_ratio(&self) -> f64 {
        let mut worst = MAX_RATIO;
        for fg in self.foregrounds() {
            for bg in self.backgrounds() {
                worst = worst.min(contrast_ratio(fg, bg));
            }
        }
        worst
    }
}

/// Mix `c` toward `target` in small steps until `ok` holds. `target` itself
/// is returned if nothing earlier satisfies `ok`.
fn adjust(c: Rgb, target: Rgb, ok: impl Fn(Rgb) -> bool) -> Rgb {
    if ok(c) {
        return c;
    }
    for step in 1..=100 {
        let m = c.mix(target, f64::from(step) / 100.0);
        if ok(m) {
            return m;
        }
    }
    target
}

#[cfg(test)]
mod tests {
    use super::*;

    // TC-001
    #[test]
    fn default_palette_is_white_on_black() {
        let p = Palette::default_palette();
        assert_eq!(p.window_bg, Rgb::WHITE);
        assert_eq!(p.text, Rgb::BLACK);
    }

    // TC-002
    #[test]
    fn known_ratios() {
        assert!((contrast_ratio(Rgb::BLACK, Rgb::WHITE) - 21.0).abs() < 1e-9);
        assert!((contrast_ratio(Rgb::WHITE, Rgb::BLACK) - 21.0).abs() < 1e-9);
        let r = contrast_ratio(Rgb(0x77, 0x77, 0x77), Rgb::WHITE);
        assert!((r - 4.48).abs() < 0.01, "got {r}");
        assert!((contrast_ratio(Rgb::WHITE, Rgb::WHITE) - 1.0).abs() < 1e-9);
    }

    // TC-003
    #[test]
    fn every_pair_meets_minimum_for_all_levels() {
        let levels = [
            ContrastLevel::Aa,
            ContrastLevel::Aaa,
            ContrastLevel::Max,
            ContrastLevel::Custom(5.0),
            ContrastLevel::Custom(10.0),
            ContrastLevel::Custom(15.5),
        ];
        for lvl in levels {
            let p = Palette::for_level(lvl);
            let min = lvl.min_ratio();
            for fg in p.foregrounds() {
                for bg in p.backgrounds() {
                    let r = contrast_ratio(fg, bg);
                    assert!(r + EPS >= min, "{lvl:?}: {fg} on {bg} = {r:.2} < {min}");
                }
            }
            assert!(p.worst_ratio() + EPS >= min);
        }
    }

    // TC-003: Max is pure black on pure white
    #[test]
    fn max_is_pure_black_and_white() {
        let p = Palette::for_level(ContrastLevel::Max);
        assert!(p.backgrounds().iter().all(|c| *c == Rgb::WHITE));
        assert!(p.foregrounds().iter().all(|c| *c == Rgb::BLACK));
    }

    // TC-004
    #[test]
    fn custom_below_minimum_is_clamped_with_warning() {
        let (lvl, warn) = ContrastLevel::custom(3.0);
        assert_eq!(lvl, ContrastLevel::Custom(4.5));
        assert!(warn.unwrap().contains("below the minimum"));
        let (lvl, warn) = ContrastLevel::custom(30.0);
        assert_eq!(lvl, ContrastLevel::Custom(21.0));
        assert!(warn.is_some());
        let (lvl, warn) = ContrastLevel::custom(7.5);
        assert_eq!(lvl, ContrastLevel::Custom(7.5));
        assert!(warn.is_none());
        // C3: infinities and NaN
        let (lvl, warn) = ContrastLevel::custom(f64::INFINITY);
        assert_eq!(lvl, ContrastLevel::Custom(21.0));
        assert!(warn.unwrap().contains("above the maximum"));
        let (lvl, warn) = ContrastLevel::custom(f64::NEG_INFINITY);
        assert_eq!(lvl, ContrastLevel::Custom(4.5));
        assert!(warn.unwrap().contains("below the minimum"));
        let (lvl, warn) = ContrastLevel::custom(f64::NAN);
        assert_eq!(lvl, ContrastLevel::Custom(4.5));
        assert!(warn.unwrap().contains("not a number"));
    }

    // TC-005
    #[test]
    fn raw_contrast_values() {
        let ok = |r: RawContrast| r.resolve().unwrap().0;
        assert_eq!(ok(RawContrast::Name("AA".into())), ContrastLevel::Aa);
        assert_eq!(ok(RawContrast::Name("aaa".into())), ContrastLevel::Aaa);
        assert_eq!(ok(RawContrast::Name("Max".into())), ContrastLevel::Max);
        assert_eq!(ok(RawContrast::Float(7.5)), ContrastLevel::Custom(7.5));
        assert_eq!(ok(RawContrast::Int(8)), ContrastLevel::Custom(8.0));
        assert!(RawContrast::Name("high".into()).resolve().is_err());
    }
}
