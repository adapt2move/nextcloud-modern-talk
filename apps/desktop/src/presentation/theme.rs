use gpui::{Hsla, WindowAppearance, rgb};

pub use talk_core::preferences::{Appearance, Palette};
#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub dark: bool,
    pub disabled_fg: Hsla,
    pub hover: Hsla,
    pub row_hover: Hsla,
    pub segment: Hsla,
    pub brand_front: Hsla,
    pub chrome_end: Hsla,
    pub focus: Hsla,
    pub sidebar_secondary: Hsla,
    pub scrim: Hsla,
    pub canvas: Hsla,
    pub sidebar: Hsla,
    pub surface: Hsla,
    pub subtle: Hsla,
    pub text: Hsla,
    pub muted: Hsla,
    pub accent: Hsla,
    pub on_accent: Hsla,
    pub selected: Hsla,
    pub selected_text: Hsla,
    pub border: Hsla,
    pub input_border: Hsla,
    pub danger: Hsla,
    pub danger_bg: Hsla,
    pub warning: Hsla,
    pub warning_bg: Hsla,
    pub success: Hsla,
    pub stage: Hsla,
    pub stage_panel: Hsla,
    pub stage_text: Hsla,
    pub stage_muted: Hsla,
    pub stage_disabled: Hsla,
}
fn color(value: u32) -> Hsla {
    rgb(value).into()
}
impl Theme {
    fn chat_header_tints(self) -> [Hsla; 2] {
        [0.92, 0.86].map(|a| Hsla { a, ..self.surface })
    }

    /// Windows has no backdrop implementation. Its explicit product fallback
    /// is a white header, including when the surrounding application is dark.
    pub fn chat_header_theme(self, palette: Palette) -> Self {
        if cfg!(target_os = "windows") {
            Self::resolve(palette, Appearance::Light, WindowAppearance::Light)
        } else {
            self
        }
    }

    /// Readability tint painted after the supported native backdrop pass.
    pub fn chat_header_surface(self) -> gpui::Background {
        if cfg!(target_os = "windows") {
            return color(0xffffff).into();
        }
        let [top, bottom] = self.chat_header_tints();
        gpui::linear_gradient(
            180.,
            gpui::linear_color_stop(top, 0.),
            gpui::linear_color_stop(bottom, 1.),
        )
    }

    pub fn chrome(self) -> gpui::Background {
        gpui::linear_gradient(
            155.,
            gpui::linear_color_stop(self.canvas, 0.),
            gpui::linear_color_stop(self.chrome_end, 1.),
        )
    }
    pub fn avatar(self, seed: usize) -> (Hsla, Hsla) {
        let (bg, fg) = if self.dark {
            [
                (0x473a33, 0xf2d6c3),
                (0x30424a, 0xc3e1ee),
                (0x403a4c, 0xe1d5f3),
                (0x45402e, 0xe9ddba),
                (0x303f4b, 0xc8dff1),
                (0x403a4c, 0xe1d5f3),
                (0x41384b, 0xe3d0ef),
            ][seed % 7]
        } else {
            [
                (0xe9d7c8, 0x7e4f37),
                (0xd3e1e8, 0x43667a),
                (0xe1daed, 0x64557a),
                (0xe8dcc0, 0x6f5c2e),
                (0xd6e5f7, 0x355f98),
                (0xe1daed, 0x64557a),
                (0xe0d8e9, 0x635273),
            ][seed % 7]
        };
        (color(bg), color(fg))
    }

    pub fn resolve(palette: Palette, appearance: Appearance, system: WindowAppearance) -> Self {
        let dark = match appearance {
            Appearance::Light => false,
            Appearance::Dark => true,
            Appearance::System => matches!(
                system,
                WindowAppearance::Dark | WindowAppearance::VibrantDark
            ),
        };
        let sage = palette == Palette::Sage;
        let values = match (dark, sage) {
            (false, false) => [
                0xe3edfb, 0xe3edfb, 0xffffff, 0xf6f8fc, 0x26354b, 0x596b85, 0x356fc1, 0xffffff,
                0xcbdbf3, 0x315e98, 0xe0e7f0, 0x70819c, 0x9e3e32, 0xfff0ec, 0x795300, 0xfff4d6,
                0x286343, 0x1c293b, 0x2c3d55, 0xeaf0f8, 0xc0ccdd, 0xd7e5f5, 0x356fc1, 0x4a5b73,
            ],
            (false, true) => [
                0xe9e7bf, 0xe9e7bf, 0xffffff, 0xf6f7f4, 0x252e2e, 0x566358, 0x354f41, 0xffffff,
                0xd3d5b0, 0x354f41, 0xe0e5dd, 0x66776a, 0x9e3e32, 0xfff0ec, 0x795300, 0xfff4d6,
                0x286343, 0x202725, 0x303d35, 0xe9eeea, 0xc5d2c9, 0xc6d6df, 0x354f41, 0x4c584e,
            ],
            (true, false) => [
                0x172131, 0x172131, 0x1c2633, 0x243140, 0xeaf0f8, 0xb1c1d6, 0x3a75c8, 0xffffff,
                0x294366, 0xeaf0f8, 0x394b60, 0x7b90a9, 0xffb8a9, 0x51302c, 0xffdc95, 0x4a3c23,
                0xa3d6b5, 0x1c293b, 0x2c3d55, 0xeaf0f8, 0xc0ccdd, 0x131b26, 0x9fc7ff, 0xb1c1d6,
            ],
            (true, true) => [
                0x1a211f, 0x1a211f, 0x1f2724, 0x28332d, 0xe9eeea, 0xb3c1b8, 0x4f7a63, 0xffffff,
                0x2b3a32, 0xe9eeea, 0x3c4c42, 0x7e9487, 0xffb8a9, 0x51302c, 0xffdc95, 0x4a3c23,
                0xa3d6b5, 0x202725, 0x303d35, 0xe9eeea, 0xc5d2c9, 0x151b19, 0xb7d7bf, 0xb3c1b8,
            ],
        };
        let (disabled, hover, segment) = match (dark, sage) {
            (false, false) => (0x8797ad, 0xeaf0f8, 0xffffff),
            (false, true) => (0x879586, 0xeef1eb, 0xffffff),
            (true, false) => (0x5d6d81, 0x293646, 0x2e3b4d),
            (true, true) => (0x5c6f62, 0x2d3a32, 0x33413a),
        };
        Self {
            dark,
            disabled_fg: color(disabled),
            hover: color(hover),
            row_hover: Hsla {
                a: if !dark && sage { 0.40 } else { 0.45 },
                ..color(values[8])
            },
            segment: color(segment),
            brand_front: color(if sage { 0x718969 } else { 0x6c99d6 }),
            canvas: color(values[0]),
            sidebar: color(values[1]),
            surface: color(values[2]),
            subtle: color(values[3]),
            text: color(values[4]),
            muted: color(values[5]),
            accent: color(values[6]),
            on_accent: color(values[7]),
            selected: color(values[8]),
            selected_text: color(values[9]),
            border: color(values[10]),
            input_border: color(values[11]),
            danger: color(values[12]),
            danger_bg: color(values[13]),
            warning: color(values[14]),
            warning_bg: color(values[15]),
            success: color(values[16]),
            stage: color(values[17]),
            stage_panel: color(values[18]),
            stage_text: color(values[19]),
            stage_muted: color(values[20]),
            stage_disabled: color(if sage { 0x78887c } else { 0x758493 }),
            chrome_end: color(values[21]),
            focus: color(values[22]),
            sidebar_secondary: color(values[23]),
            scrim: gpui::rgba(if dark { 0x00000073 } else { 0x2b384b26 }).into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn contrast(a: Hsla, b: Hsla) -> f32 {
        let luminance = |h: Hsla| {
            let c = gpui::Rgba::from(h);
            let linear = |v: f32| {
                if v <= 0.04045 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * linear(c.r) + 0.7152 * linear(c.g) + 0.0722 * linear(c.b)
        };
        let a = luminance(a);
        let b = luminance(b);
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }
    #[test]
    fn translucent_header_keeps_text_and_focus_readable_over_extreme_content() {
        for palette in [Palette::Blue, Palette::Sage] {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::resolve(palette, appearance, WindowAppearance::Light);
                for tint in t.chat_header_tints().map(gpui::Rgba::from) {
                    assert!((0.5..1.).contains(&tint.a));
                    for underlying in [0., 1.] {
                        let mixed = gpui::Rgba {
                            r: tint.r * tint.a + underlying * (1. - tint.a),
                            g: tint.g * tint.a + underlying * (1. - tint.a),
                            b: tint.b * tint.a + underlying * (1. - tint.a),
                            a: 1.,
                        };
                        for text in [t.text, t.sidebar_secondary] {
                            assert!(contrast(text, mixed.into()) >= 4.5);
                        }
                        assert!(contrast(t.focus, mixed.into()) >= 3.);
                    }
                }
            }
        }
    }
    #[test]
    fn row_hover_is_a_distinct_intermediate_lightness_step_across_chrome() {
        let lightness = |c: gpui::Rgba| {
            let linear = |v: f32| {
                if v <= 0.04045 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            let y = 0.2126 * linear(c.r) + 0.7152 * linear(c.g) + 0.0722 * linear(c.b);
            if y > 0.008856 {
                116. * y.cbrt() - 16.
            } else {
                903.3 * y
            }
        };
        for palette in [Palette::Blue, Palette::Sage] {
            for mode in [Appearance::Light, Appearance::Dark] {
                let t = Theme::resolve(palette, mode, WindowAppearance::Light);
                let hover = gpui::Rgba::from(t.row_hover);
                for background in [t.canvas, t.chrome_end] {
                    let bg = gpui::Rgba::from(background);
                    let mixed = gpui::Rgba {
                        r: bg.r + hover.a * (hover.r - bg.r),
                        g: bg.g + hover.a * (hover.g - bg.g),
                        b: bg.b + hover.a * (hover.b - bg.b),
                        a: 1.,
                    };
                    let step = (lightness(mixed) - lightness(bg)).abs();
                    let selection = (lightness(gpui::Rgba::from(t.selected)) - lightness(bg)).abs();
                    assert!(
                        (0.39..0.53).contains(&(step / selection)),
                        "hover must remain visibly intermediate for {palette:?}/{mode:?}"
                    );
                }
            }
        }
    }
    #[test]
    fn semantic_text_and_control_roles_meet_design_contrast_in_all_four_themes() {
        for palette in [Palette::Blue, Palette::Sage] {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::resolve(palette, appearance, WindowAppearance::Light);
                for (name, fg, bg) in [
                    ("body", t.text, t.surface),
                    ("secondary", t.muted, t.surface),
                    ("sidebar", t.sidebar_secondary, t.sidebar),
                    ("chrome end", t.sidebar_secondary, t.chrome_end),
                    ("selection", t.sidebar_secondary, t.selected),
                    ("avatar", t.selected_text, t.selected),
                    ("primary", t.on_accent, t.accent),
                    ("selected segment", t.text, t.segment),
                    ("warning", t.warning, t.warning_bg),
                    ("danger", t.danger, t.danger_bg),
                    ("stage", t.stage_muted, t.stage_panel),
                ] {
                    assert!(
                        contrast(fg, bg) >= 4.5,
                        "{palette:?}/{appearance:?}/{name}: {}",
                        contrast(fg, bg)
                    );
                }
                assert!(contrast(t.input_border, t.surface) >= 3.0);
                for background in [
                    t.surface,
                    t.subtle,
                    t.canvas,
                    t.chrome_end,
                    t.selected,
                    t.segment,
                ] {
                    assert!(
                        contrast(t.focus, background) >= 3.0,
                        "focus {palette:?}/{appearance:?}: {}",
                        contrast(t.focus, background)
                    );
                }
                for track in [t.subtle, t.canvas, t.chrome_end] {
                    assert!(
                        contrast(t.input_border, track) >= 3.0,
                        "selected marker {palette:?}/{appearance:?}: {}",
                        contrast(t.input_border, track)
                    );
                    assert!(
                        contrast(t.disabled_fg, track) < contrast(t.muted, track) * 0.7,
                        "disabled must be visibly weaker"
                    );
                }
                assert!((2.0..3.6).contains(&contrast(t.disabled_fg, t.surface)));
                if t.dark {
                    // Raised selection must increase lightness; its accessible
                    // marker, not that small luminance difference, carries state.
                    assert!(contrast(t.segment, gpui::black()) > contrast(t.subtle, gpui::black()));
                }
                for seed in 0..7 {
                    let (bg, fg) = t.avatar(seed);
                    assert!(contrast(fg, bg) >= 4.5);
                }
            }
        }
    }
}
