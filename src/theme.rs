use crate::model::{AppearanceSettings, Settings};
use gpui::{Hsla, Rgba, WindowAppearance, hsla, rgb};

#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: Hsla,
    pub panel: Hsla,
    pub card: Hsla,
    /// Large bordered containers such as quadrants and day cards.
    /// Explicit window effects use a much lighter fill than `subtle`, keeping
    /// the window's single backdrop readable while the panel edges stay visible.
    pub surface: Hsla,
    pub subtle: Hsla,
    pub hover: Hsla,
    pub text: Hsla,
    pub muted: Hsla,
    pub border: Hsla,
    pub accent: Hsla,
    pub accent_text: Hsla,
    pub selected: Hsla,
    pub quadrant: [Hsla; 4],
    pub dark: bool,
}

impl Palette {
    pub fn is_dark(settings: &Settings, appearance: WindowAppearance) -> bool {
        match settings.theme_mode() {
            "light" => false,
            "system" => matches!(
                appearance,
                WindowAppearance::Dark | WindowAppearance::VibrantDark
            ),
            _ => true,
        }
    }

    pub fn new(settings: &Settings, appearance: WindowAppearance) -> Self {
        let dark = Self::is_dark(settings, appearance);
        let color = |v| Hsla::from(rgb(v));
        // Keep legacy named palettes byte-for-byte until the first appearance edit.
        let legacy = if settings.appearance.is_none() {
            match settings.theme.as_str() {
                "ocean" => Some((0x0c1219, 0x090f16, 0x16212c, 0xe2eaf0, 0x94a5b5, 0x7db8d9)),
                "forest" => Some((0x0e1511, 0x0b110d, 0x19231c, 0xe4ebe5, 0x9bab9f, 0xa3c1a3)),
                "terminal" => Some((0x0a100c, 0x080d09, 0x151e18, 0xdce7df, 0x91a596, 0x8fc9a0)),
                _ => None,
            }
        } else {
            None
        };
        let (bg, panel, card, foreground, muted, accent, accent_text) =
            if let Some((bg, panel, card, text, muted, accent)) = legacy {
                (
                    color(bg),
                    color(panel),
                    color(card),
                    color(text),
                    color(muted),
                    color(accent),
                    color(0x101a23),
                )
            } else {
                custom_colors(&settings.appearance_values(), dark)
            };
        let explicit_effects = settings.window_effects.is_some();
        let (alpha, panel_alpha, translucent) = match &settings.window_effects {
            // A single background tint makes both endpoints exact and keeps the
            // sidebar and main pane equally transparent, without fading text.
            Some(effects) => (1. - effects.transparency, 0., effects.transparency > 0.),
            None => (
                glass_tint_alpha(settings.glass, settings.opacity, dark),
                if settings.glass { 0.12 } else { 1. },
                settings.glass,
            ),
        };
        // With explicit effects, content sits one lightness point below the
        // configured shell, matching the terminal-style backdrop: the window
        // paints one backdrop and bordered panels add only a faint surface fill.
        let bg = if explicit_effects && dark {
            Hsla {
                l: (bg.l - 0.01).max(0.),
                ..bg
            }
        } else {
            bg
        };
        let subtle = foreground.opacity(if dark { 0.025 } else { 0.018 });
        Self {
            bg: bg.opacity(alpha),
            panel: panel.opacity(panel_alpha),
            card: if translucent {
                color(0xffffff).opacity(if dark { 0.025 } else { 0.20 })
            } else {
                card
            },
            surface: if explicit_effects {
                // Keep large panels legible without adding the old 2.5% wash.
                foreground.opacity(if dark { 0.015 } else { 0.012 })
            } else {
                subtle
            },
            subtle,
            hover: foreground.opacity(if dark { 0.085 } else { 0.065 }),
            text: foreground,
            muted,
            border: foreground.opacity(if dark { 0.08 } else { 0.12 }),
            accent,
            accent_text,
            selected: foreground.opacity(if dark { 0.10 } else { 0.07 }),
            quadrant: if dark {
                [
                    color(0xd8908c),
                    color(0x8eadd2),
                    color(0xc6ad7a),
                    color(0x8fbaa4),
                ]
            } else {
                [
                    color(0xa55650),
                    color(0x5379a4),
                    color(0x93743d),
                    color(0x4e8068),
                ]
            },
            dark,
        }
    }
}

/// Builds an opaque palette before native-material opacity is applied.
fn custom_colors(
    values: &AppearanceSettings,
    dark: bool,
) -> (Hsla, Hsla, Hsla, Hsla, Hsla, Hsla, Hsla) {
    let hue = values.hue / 360.;
    let saturation = values.saturation;
    let surface = |lightness| hsla(hue, saturation, lightness, 1.);
    let (bg, panel, card) = if dark {
        let lightness = values.dark_lightness;
        // Reduce elevation at the bright end so even saturated yellow surfaces
        // still have room for readable text and selection overlays.
        let elevation = (0.075 * (1. - lightness / 0.22)).max(0.);
        (
            surface(lightness),
            surface((lightness - 0.01).max(0.)),
            surface(lightness + elevation),
        )
    } else {
        (surface(0.967), surface(0.934), surface(0.994))
    };
    // Include selection overlays in the contrast budget. Actual material and
    // wallpaper contrast remains controlled by AppKit and the user.
    let overlay = hsla(
        0.,
        0.,
        if dark { 1. } else { 0. },
        if dark { 0.10 } else { 0.07 },
    );
    let selected = |surface| Hsla::from(Rgba::from(surface).blend(overlay.into()));
    let backgrounds = [
        bg,
        selected(bg),
        panel,
        selected(panel),
        card,
        selected(card),
    ];
    let text = readable_color(
        hsla(hue, saturation * 0.18, if dark { 0.92 } else { 0.18 }, 1.),
        &backgrounds,
        dark,
    );
    let muted = readable_color(
        hsla(hue, saturation * 0.30, if dark { 0.63 } else { 0.42 }, 1.),
        &backgrounds,
        dark,
    );
    let accent = readable_color(parse_accent(&values.accent_hex), &backgrounds, dark);
    let black = hsla(0., 0., 0., 1.);
    let white = hsla(0., 0., 1., 1.);
    let accent_text = if contrast(black, accent) > contrast(white, accent) {
        black
    } else {
        white
    };
    (bg, panel, card, text, muted, accent, accent_text)
}

fn parse_accent(hex: &str) -> Hsla {
    // Settings validation requires six digits. Fall back safely while callers
    // are constructing a preview with an incomplete input value.
    hex.strip_prefix('#')
        .filter(|digits| digits.len() == 6 && digits.bytes().all(|b| b.is_ascii_hexdigit()))
        .and_then(|digits| u32::from_str_radix(digits, 16).ok())
        .map(|value| Hsla::from(rgb(value)))
        .unwrap_or_else(|| rgb(0x6aa9ef).into())
}

fn luminance(color: Hsla) -> f32 {
    let color = Rgba::from(color);
    let linear = |channel: f32| {
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color.r) + 0.7152 * linear(color.g) + 0.0722 * linear(color.b)
}

fn contrast(a: Hsla, b: Hsla) -> f32 {
    let a = luminance(a);
    let b = luminance(b);
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

fn readable_color(mut color: Hsla, backgrounds: &[Hsla], dark: bool) -> Hsla {
    // A small margin above WCAG AA absorbs RGB rounding at render time.
    const MIN_CONTRAST: f32 = 4.6;
    let readable = |color: Hsla| {
        backgrounds
            .iter()
            .all(|background| contrast(color, *background) >= MIN_CONTRAST)
    };
    if readable(color) {
        return color;
    }
    let mut failing = color.l;
    let mut passing = if dark { 1. } else { 0. };
    for _ in 0..16 {
        color.l = (failing + passing) / 2.;
        if readable(color) {
            passing = color.l;
        } else {
            failing = color.l;
        }
    }
    color.l = passing;
    color
}

// AppKit supplies the actual frosted material. This is only the colored overlay;
// even maximum tint must leave that material visible rather than paint over it.
fn glass_tint_alpha(glass: bool, strength: f32, dark: bool) -> f32 {
    if !glass {
        return 1.;
    }
    let strength = strength.clamp(0., 1.);
    if dark {
        0.30 + strength * 0.50
    } else {
        0.08 + strength * 0.28
    }
}

pub fn tr<'a>(settings: &Settings, zh: &'a str, en: &'a str) -> &'a str {
    if settings.language == "en" { en } else { zh }
}

pub fn quadrant_name(q: usize, english: bool) -> &'static str {
    if english {
        [
            "Urgent & Important",
            "Important, Not Urgent",
            "Urgent, Not Important",
            "Neither",
        ][q]
    } else {
        ["紧急且重要", "重要不紧急", "紧急不重要", "不紧急不重要"][q]
    }
}

pub fn quadrant_description(q: usize, english: bool) -> &'static str {
    if english {
        [
            "Do now",
            "Schedule",
            "Delegate / quick",
            "Consider dropping",
        ][q]
    } else {
        ["立即做", "计划做", "委派 / 快速做", "考虑删减"][q]
    }
}

#[cfg(test)]
mod tests {
    use super::{Palette, contrast, glass_tint_alpha, luminance};
    use crate::model::{AppearanceSettings, Settings, WindowEffectsSettings};
    use gpui::{Hsla, Rgba, WindowAppearance, rgb};

    #[test]
    fn legacy_palettes_stay_exact_until_customized() {
        for (theme, expected) in [
            (
                "ocean",
                [0x0c1219, 0x090f16, 0x16212c, 0xe2eaf0, 0x94a5b5, 0x7db8d9],
            ),
            (
                "forest",
                [0x0e1511, 0x0b110d, 0x19231c, 0xe4ebe5, 0x9bab9f, 0xa3c1a3],
            ),
            (
                "terminal",
                [0x0a100c, 0x080d09, 0x151e18, 0xdce7df, 0x91a596, 0x8fc9a0],
            ),
        ] {
            let mut settings = Settings {
                theme: theme.into(),
                glass: false,
                ..Default::default()
            };
            let old = Palette::new(&settings, WindowAppearance::Light);
            assert!(old.dark);
            for (actual, hex) in [old.bg, old.panel, old.card, old.text, old.muted, old.accent]
                .into_iter()
                .zip(expected)
            {
                assert_eq!(actual, Hsla::from(rgb(hex)), "{theme}");
            }
            assert_eq!(old.accent_text, Hsla::from(rgb(0x101a23)));
            settings.customize_appearance();
            let converted = Palette::new(&settings, WindowAppearance::Light);
            for (before, after) in [
                (old.bg, converted.bg),
                (old.panel, converted.panel),
                (old.card, converted.card),
            ] {
                let before = Rgba::from(before);
                let after = Rgba::from(after);
                for (a, b) in [
                    (before.r, after.r),
                    (before.g, after.g),
                    (before.b, after.b),
                ] {
                    assert!(
                        (a - b).abs() < 0.025,
                        "{theme}: legacy conversion changed surface excessively"
                    );
                }
            }
            assert_eq!(converted.accent, old.accent);
            assert_eq!(converted.quadrant, old.quadrant);
        }
    }

    #[test]
    fn system_mode_tracks_both_native_appearances_and_explicit_modes_override() {
        for (appearance, expected_dark) in [
            (WindowAppearance::Light, false),
            (WindowAppearance::VibrantLight, false),
            (WindowAppearance::Dark, true),
            (WindowAppearance::VibrantDark, true),
        ] {
            for custom in [None, Some(AppearanceSettings::default())] {
                let mut settings = Settings {
                    appearance: custom,
                    ..Default::default()
                };
                assert_eq!(Palette::new(&settings, appearance).dark, expected_dark);
                settings.theme = "light".into();
                assert!(!Palette::new(&settings, appearance).dark);
                settings.theme = "dark".into();
                assert!(Palette::new(&settings, appearance).dark);
            }
        }
    }

    #[test]
    fn dark_lightness_is_monotonic_for_every_surface() {
        for hue in [0., 60., 120., 180., 240., 300., 360.] {
            for saturation in [0., 0.1, 0.4, 1.] {
                let mut previous = [-1.; 3];
                for step in 0..=20 {
                    let settings = Settings {
                        theme: "dark".into(),
                        glass: false,
                        appearance: Some(AppearanceSettings {
                            hue,
                            saturation,
                            dark_lightness: step as f32 / 100.,
                            ..Default::default()
                        }),
                        ..Default::default()
                    };
                    let p = Palette::new(&settings, WindowAppearance::Light);
                    for (index, surface) in [p.bg, p.panel, p.card].into_iter().enumerate() {
                        let value = luminance(surface);
                        assert!(
                            value >= previous[index],
                            "{hue}/{saturation}/{step}/{index}"
                        );
                        if index != 1 && step > 0 {
                            assert!(value > previous[index]);
                        }
                        previous[index] = value;
                    }
                }
            }
        }
    }

    #[test]
    fn custom_colors_are_readable_at_extremes_and_for_every_accent_swatch() {
        for dark in [false, true] {
            for hue in [0., 60., 120., 180., 240., 300., 360.] {
                for saturation in [0., 0.1, 0.4, 1.] {
                    for lightness in [0., 0.05, 0.20] {
                        for hex in [
                            "#6AA9EF", "#9B8AFB", "#DB8ABE", "#E08E8E", "#DAAA63", "#83B59C",
                            "#7DB8D9", "#A3C1A3", "#8FC9A0", "#000000", "#FFFFFF", "#FFFF00",
                            "#0000FF", "#FF0000",
                        ] {
                            let settings = Settings {
                                theme: if dark { "dark" } else { "light" }.into(),
                                glass: false,
                                appearance: Some(AppearanceSettings {
                                    hue,
                                    saturation,
                                    dark_lightness: lightness,
                                    accent_hex: hex.into(),
                                }),
                                ..Default::default()
                            };
                            let p = Palette::new(&settings, WindowAppearance::Light);
                            for surface in [p.bg, p.panel, p.card] {
                                for overlay in [p.subtle, p.hover, p.selected] {
                                    let selected =
                                        Hsla::from(Rgba::from(surface).blend(overlay.into()));
                                    for foreground in [p.text, p.muted, p.accent] {
                                        assert!(
                                            contrast(foreground, surface) >= 4.5,
                                            "surface: dark={dark}, hue={hue}, s={saturation}, l={lightness}, accent={hex}"
                                        );
                                        assert!(
                                            contrast(foreground, selected) >= 4.5,
                                            "overlay: dark={dark}, hue={hue}, s={saturation}, l={lightness}, accent={hex}"
                                        );
                                    }
                                }
                            }
                            assert!(
                                contrast(p.accent, p.accent_text) >= 4.5,
                                "accent button {hex}"
                            );
                            for color in [
                                p.bg,
                                p.panel,
                                p.card,
                                p.text,
                                p.muted,
                                p.accent,
                                p.accent_text,
                            ] {
                                assert!(
                                    [color.h, color.s, color.l, color.a]
                                        .into_iter()
                                        .all(|channel| channel.is_finite()
                                            && (0.0..=1.0).contains(&channel))
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn glass_tint_leaves_native_material_visible_in_every_theme() {
        for theme in ["system", "light", "dark", "ocean", "forest", "terminal"] {
            for appearance in [WindowAppearance::Light, WindowAppearance::Dark] {
                let mut settings = Settings {
                    theme: theme.into(),
                    glass: true,
                    opacity: 1.,
                    ..Default::default()
                };
                let glass = Palette::new(&settings, appearance);
                let combined_alpha = glass.bg.a + glass.panel.a * (1. - glass.bg.a);
                assert!(
                    combined_alpha < 0.85,
                    "{theme}: opaque layers hide the native material"
                );
                assert!(glass.card.a <= 0.20);
                settings.glass = false;
                let solid = Palette::new(&settings, appearance);
                assert_eq!(solid.bg.a, 1.);
                assert_eq!(solid.panel.a, 1.);
            }
        }
    }

    #[test]
    fn explicit_transparency_has_exact_endpoints_and_leaves_text_opaque() {
        for appearance in [WindowAppearance::Light, WindowAppearance::Dark] {
            for transparency in [0., 0.25, 0.5, 1.] {
                let mut settings = Settings {
                    window_effects: Some(crate::model::WindowEffectsSettings {
                        transparency,
                        blur_radius: 0.,
                    }),
                    ..Default::default()
                };
                let clear = Palette::new(&settings, appearance);
                assert_eq!(clear.bg.a, 1. - transparency);
                assert_eq!(clear.panel.a, 0.);
                assert!(clear.surface.a > 0.);
                assert!(clear.surface.a < clear.subtle.a);
                assert_eq!(clear.text.a, 1.);
                assert_eq!(clear.muted.a, 1.);
                assert_eq!(clear.accent.a, 1.);
                settings.window_effects.as_mut().unwrap().blur_radius = 64.;
                let blurred = Palette::new(&settings, appearance);
                assert_eq!(clear.bg, blurred.bg);
                assert_eq!(clear.panel, blurred.panel);
                assert_eq!(clear.surface, blurred.surface);
                assert_eq!(clear.text, blurred.text);
            }
        }
    }

    #[test]
    fn explicit_dark_content_sits_one_point_below_the_configured_shell() {
        let appearance = Some(AppearanceSettings {
            hue: 220.,
            saturation: 0.5,
            dark_lightness: 0.05,
            accent_hex: "#08B03A".into(),
        });
        let settings = Settings {
            theme: "dark".into(),
            appearance: appearance.clone(),
            window_effects: Some(WindowEffectsSettings {
                transparency: 0.1,
                blur_radius: 10.,
            }),
            ..Default::default()
        };
        let explicit = Palette::new(&settings, WindowAppearance::Dark);
        assert!((explicit.bg.l - 0.04).abs() < 1e-6);
        assert!((explicit.bg.a - 0.9).abs() < 1e-6);
        assert!((explicit.surface.a - 0.015).abs() < 1e-6);
        assert!((explicit.panel.l - 0.04).abs() < 1e-6);

        // Without explicit effects the configured lightness and panel wash stay as before.
        let settings = Settings {
            appearance,
            glass: false,
            ..Default::default()
        };
        let opaque = Palette::new(&settings, WindowAppearance::Dark);
        assert!((opaque.bg.l - 0.05).abs() < 1e-6);
        assert_eq!(opaque.bg.a, 1.);
        assert_eq!(opaque.surface, opaque.subtle);
    }

    #[test]
    fn tint_strength_is_monotonic_and_clamped() {
        for dark in [true, false] {
            let values = [0., 0.25, 0.5, 0.85, 1.].map(|v| glass_tint_alpha(true, v, dark));
            assert!(values.windows(2).all(|v| v[0] < v[1]));
            assert_eq!(glass_tint_alpha(true, -1., dark), values[0]);
            assert_eq!(glass_tint_alpha(true, 2., dark), values[4]);
            assert_eq!(glass_tint_alpha(false, 0., dark), 1.);
        }
    }
}
