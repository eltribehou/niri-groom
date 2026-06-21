//! Color themes. A `Theme` carries a handful of base colors; the few extra
//! shades the drawing needs (separators, subtle borders, selection tints) are
//! derived from those, so a single definition works for both dark and light
//! palettes.

pub type Rgb = (f64, f64, f64);

/// Linear blend: `k=0` → `a`, `k=1` → `b`.
pub fn mix(a: Rgb, b: Rgb, k: f64) -> Rgb {
    (
        a.0 + (b.0 - a.0) * k,
        a.1 + (b.1 - a.1) * k,
        a.2 + (b.2 - a.2) * k,
    )
}

fn rgb(hex: u32) -> Rgb {
    (
        ((hex >> 16) & 0xff) as f64 / 255.0,
        ((hex >> 8) & 0xff) as f64 / 255.0,
        (hex & 0xff) as f64 / 255.0,
    )
}

#[derive(Clone)]
pub struct Theme {
    pub name: &'static str,
    pub bg: Rgb,
    pub surface: Rgb,
    pub window: Rgb,
    pub text: Rgb,
    pub subtext: Rgb,
    pub accent: Rgb,
    pub urgent: Rgb,
}

impl Theme {
    /// Background of a selected workspace card (faint accent tint).
    pub fn selected_card(&self) -> Rgb {
        mix(self.surface, self.accent, 0.16)
    }

    /// Background of the niri-focused window (a touch off the base).
    pub fn focused_window(&self) -> Rgb {
        mix(self.window, self.text, 0.07)
    }

    /// Background of the selected window — tinted toward the accent, but mild
    /// enough that `text` stays readable on top (the border does the shouting).
    pub fn selected_window(&self) -> Rgb {
        mix(self.window, self.accent, 0.30)
    }
}

/// All bundled themes, in picker order. The first is the default.
pub fn all() -> Vec<Theme> {
    vec![
        Theme {
            name: "catppuccin-mocha",
            bg: rgb(0x181825),
            surface: rgb(0x1e1e2e),
            window: rgb(0x313244),
            text: rgb(0xcdd6f4),
            subtext: rgb(0xa6adc8),
            accent: rgb(0x89b4fa),
            urgent: rgb(0xf38ba8),
        },
        Theme {
            name: "catppuccin-macchiato",
            bg: rgb(0x1e2030),
            surface: rgb(0x24273a),
            window: rgb(0x363a4f),
            text: rgb(0xcad3f5),
            subtext: rgb(0xa5adcb),
            accent: rgb(0x8aadf4),
            urgent: rgb(0xed8796),
        },
        Theme {
            name: "catppuccin-latte",
            bg: rgb(0xeff1f5),
            surface: rgb(0xe6e9ef),
            window: rgb(0xccd0da),
            text: rgb(0x4c4f69),
            subtext: rgb(0x6c6f85),
            accent: rgb(0x1e66f5),
            urgent: rgb(0xd20f39),
        },
        Theme {
            name: "gruvbox-material",
            bg: rgb(0x1d2021),
            surface: rgb(0x282828),
            window: rgb(0x3c3836),
            text: rgb(0xd4be98),
            subtext: rgb(0xa89984),
            accent: rgb(0x7daea3),
            urgent: rgb(0xea6962),
        },
        Theme {
            name: "gruvbox-light",
            bg: rgb(0xfbf1c7),
            surface: rgb(0xf2e5bc),
            window: rgb(0xebdbb2),
            text: rgb(0x3c3836),
            subtext: rgb(0x7c6f64),
            accent: rgb(0x458588),
            urgent: rgb(0xcc241d),
        },
        Theme {
            name: "tokyo-night",
            bg: rgb(0x16161e),
            surface: rgb(0x1a1b26),
            window: rgb(0x292e42),
            text: rgb(0xc0caf5),
            subtext: rgb(0xa9b1d6),
            accent: rgb(0x7aa2f7),
            urgent: rgb(0xf7768e),
        },
        Theme {
            name: "nord",
            bg: rgb(0x2e3440),
            surface: rgb(0x3b4252),
            window: rgb(0x434c5e),
            text: rgb(0xeceff4),
            subtext: rgb(0xabb2c0),
            accent: rgb(0x88c0d0),
            urgent: rgb(0xbf616a),
        },
        Theme {
            name: "dracula",
            bg: rgb(0x21222c),
            surface: rgb(0x282a36),
            window: rgb(0x44475a),
            text: rgb(0xf8f8f2),
            subtext: rgb(0xa0a4c0),
            accent: rgb(0xbd93f9),
            urgent: rgb(0xff5555),
        },
        Theme {
            name: "rose-pine",
            bg: rgb(0x191724),
            surface: rgb(0x1f1d2e),
            window: rgb(0x26233a),
            text: rgb(0xe0def4),
            subtext: rgb(0x908caa),
            accent: rgb(0xc4a7e7),
            urgent: rgb(0xeb6f92),
        },
    ]
}

/// Index of the theme named `name`, if bundled.
pub fn index_of(name: &str) -> Option<usize> {
    all().iter().position(|t| t.name == name)
}

pub const DEFAULT: &str = "catppuccin-mocha";
