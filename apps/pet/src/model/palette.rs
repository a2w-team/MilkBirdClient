//! Color palette for the MilkBird ("小红鸟 / 奶鸟") character.
//!
//! Inspired by the iconic HKUST / HKUST-GZ Red Bird sundial mascot plushie:
//! vivid plush crimson body, bright flame-orange sundial crest, snowy milky-white round tummy
//! with a minimalist gold-and-ink club emblem, electric sky-blue eye irises, berry-red plush
//! beak & 3-lobed wings, and warm caramel-tan feet.

use serde::{Deserialize, Serialize};

/// Non-premultiplied RGBA8 color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub fn with_alpha(self, alpha: f32) -> Self {
        let a = (alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
        Self { a, ..self }
    }

    pub fn to_skia(self) -> tiny_skia::Color {
        tiny_skia::Color::from_rgba8(self.r, self.g, self.b, self.a)
    }
}

/// Complete character palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Palette {
    pub body_highlight: Rgba,
    pub body_base: Rgba,
    pub body_shade: Rgba,
    pub belly_highlight: Rgba,
    pub belly_base: Rgba,
    pub belly_shade: Rgba,
    pub outline: Rgba,
    pub crest_tip: Rgba,
    pub crest_base: Rgba,
    pub wing_base: Rgba,
    pub wing_lobe: Rgba,
    pub wing_shade: Rgba,
    pub tail_base: Rgba,
    pub tail_accent: Rgba,
    pub beak_top: Rgba,
    pub beak_base: Rgba,
    pub beak_inner: Rgba,
    pub foot_base: Rgba,
    pub foot_shade: Rgba,
    pub cheek: Rgba,
    pub eye_sclera: Rgba,
    pub eye_iris: Rgba,
    pub eye_iris_deep: Rgba,
    pub eye_ink: Rgba,
    pub eye_sparkle: Rgba,
    pub brow_accent: Rgba,
    pub emblem_ink: Rgba,
    pub emblem_gold: Rgba,
    pub badge_alert: Rgba,
    pub badge_ok: Rgba,
    pub heart: Rgba,
    pub sleep_zzz: Rgba,
    pub sparkle: Rgba,
    pub shadow: Rgba,
}

impl Palette {
    /// Signature HKUST-GZ Red Bird ("小红鸟 / 奶鸟") plush palette.
    pub const fn milkbird_red() -> Self {
        Self {
            body_highlight: Rgba::rgb(248, 44, 62),    // #F82C3E plush crimson sheen
            body_base: Rgba::rgb(225, 22, 42),         // #E1162A vivid red-bird scarlet
            body_shade: Rgba::rgb(176, 14, 34),        // #B00E22 deep velvet shadow
            belly_highlight: Rgba::rgb(255, 255, 255), // #FFFFFF snowy white tummy
            belly_base: Rgba::rgb(252, 250, 247),      // #FCFAF7 soft milky plush
            belly_shade: Rgba::rgb(235, 230, 224),     // #EBE6E0 warm tummy shadow
            outline: Rgba::rgb(104, 15, 26),           // #680F1A deep burgundy plush contour
            crest_tip: Rgba::rgb(255, 138, 28),        // #FF8A1C bright flame-orange tip
            crest_base: Rgba::rgb(244, 63, 18),        // #F43F12 fiery sundial vermilion
            wing_base: Rgba::rgb(228, 24, 46),         // #E4182E crimson plush wing
            wing_lobe: Rgba::rgb(198, 22, 54),         // #C61636 berry-crimson scalloped pads
            wing_shade: Rgba::rgb(162, 14, 40),        // #A20E28 wing depth
            tail_base: Rgba::rgb(218, 26, 44),         // #DA1A2C red tail feather
            tail_accent: Rgba::rgb(249, 102, 24),      // #F96618 flame accent tail
            beak_top: Rgba::rgb(248, 84, 114),         // #F85472 soft berry-pink highlight
            beak_base: Rgba::rgb(214, 34, 66),         // #D62242 plush berry-red beak
            beak_inner: Rgba::rgb(255, 158, 174),      // #FF9EAE rosy open mouth
            foot_base: Rgba::rgb(198, 140, 70),        // #C68C46 warm caramel-tan plush feet
            foot_shade: Rgba::rgb(160, 108, 46),       // #A06C2E caramel shadow
            cheek: Rgba::rgb(255, 138, 160),           // #FF8AA0 warm rosy glow
            eye_sclera: Rgba::rgb(255, 255, 255),      // #FFFFFF outer white ring
            eye_iris: Rgba::rgb(56, 189, 248),         // #38BDF8 iconic electric sky-blue iris
            eye_iris_deep: Rgba::rgb(14, 145, 218),    // #0E91DA deeper cyan iris rim
            eye_ink: Rgba::rgb(22, 20, 26),            // #16141A deep glossy black pupil
            eye_sparkle: Rgba::rgb(255, 255, 255),     // #FFFFFF bright catchlight
            brow_accent: Rgba::rgb(152, 16, 36),       // #981024 subtle embroidered red brow
            emblem_ink: Rgba::rgb(34, 36, 42),         // #22242A crisp dark emblem lines
            emblem_gold: Rgba::rgb(217, 152, 34),      // #D99822 sundial gold accent
            badge_alert: Rgba::rgb(37, 99, 235),       // #2563EB vibrant cobalt pill (contrasts red!)
            badge_ok: Rgba::rgb(16, 185, 129),         // #10B981 emerald active watch
            heart: Rgba::rgb(255, 112, 148),           // #FF7094 floating petted hearts
            sleep_zzz: Rgba::rgb(96, 165, 250),        // #60A5FA crisp sky-blue Zzz
            sparkle: Rgba::rgb(251, 191, 36),          // #FBBF24 golden celebration stars
            shadow: Rgba::new(52, 16, 20, 48),         // soft warm ground shadow
        }
    }

    /// Alias kept for API stability (`milkbird_day` returns the signature Red Bird palette).
    pub const fn milkbird_day() -> Self {
        Self::milkbird_red()
    }
}

impl Default for Palette {
    fn default() -> Self {
        Self::milkbird_red()
    }
}
