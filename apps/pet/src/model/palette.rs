//! Color palette for the original MilkBird ("小奶鸟") character.
//!
//! Harmonized with `apps/tray/ui/style.css` (`#2563EB` club blue, `#059669` ok emerald,
//! `#F4F5F7` light surfaces) with a delicate warm cocoa contour so the cream-white bird
//! reads cleanly over both bright documents and dark wallpapers.

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
    pub belly_glow: Rgba,
    pub outline: Rgba,
    pub wing_base: Rgba,
    pub wing_tip: Rgba,
    pub wing_shade: Rgba,
    pub tail_base: Rgba,
    pub tail_accent: Rgba,
    pub beak_top: Rgba,
    pub beak_base: Rgba,
    pub beak_inner: Rgba,
    pub foot_base: Rgba,
    pub foot_shade: Rgba,
    pub cheek: Rgba,
    pub eye_ink: Rgba,
    pub eye_sparkle: Rgba,
    pub scarf_base: Rgba,
    pub scarf_highlight: Rgba,
    pub scarf_shade: Rgba,
    pub badge_idle: Rgba,
    pub badge_alert: Rgba,
    pub badge_ok: Rgba,
    pub heart: Rgba,
    pub sleep_zzz: Rgba,
    pub sparkle: Rgba,
    pub shadow: Rgba,
}

impl Palette {
    /// Signature MilkBird daytime palette (creamy milk-foam + oat wings + club cobalt scarf).
    pub const fn milkbird_day() -> Self {
        Self {
            body_highlight: Rgba::rgb(255, 255, 253), // #FFFFFD
            body_base: Rgba::rgb(255, 251, 245),      // #FFFBF5
            body_shade: Rgba::rgb(241, 232, 220),     // #F1E8DC
            belly_glow: Rgba::rgb(255, 255, 255),     // #FFFFFF
            outline: Rgba::rgb(90, 72, 60),           // #5A483C warm cocoa contour
            wing_base: Rgba::rgb(233, 220, 205),      // #E9DCCD warm oat-latte
            wing_tip: Rgba::rgb(248, 242, 234),       // #F8F2EA
            wing_shade: Rgba::rgb(212, 195, 177),     // #D4C3B1
            tail_base: Rgba::rgb(228, 213, 196),      // #E4D5C4
            tail_accent: Rgba::rgb(203, 184, 165),    // #CBB8A5
            beak_top: Rgba::rgb(252, 211, 77),        // #FCD34D
            beak_base: Rgba::rgb(245, 158, 11),       // #F59E0B
            beak_inner: Rgba::rgb(244, 114, 140),     // #F4728C
            foot_base: Rgba::rgb(245, 158, 11),       // #F59E0B
            foot_shade: Rgba::rgb(217, 119, 6),       // #D97706
            cheek: Rgba::rgb(253, 164, 175),          // #FDA4AF soft peach blush
            eye_ink: Rgba::rgb(35, 26, 22),           // #231A16 espresso ink
            eye_sparkle: Rgba::rgb(255, 255, 255),    // #FFFFFF
            scarf_base: Rgba::rgb(37, 99, 235),       // #2563EB club cobalt blue
            scarf_highlight: Rgba::rgb(96, 165, 250), // #60A5FA
            scarf_shade: Rgba::rgb(29, 78, 216),      // #1D4ED8
            badge_idle: Rgba::rgb(254, 243, 199),     // #FEF3C7 warm cream-gold pin
            badge_alert: Rgba::rgb(244, 63, 94),      // #F43F5E coral unread pill
            badge_ok: Rgba::rgb(16, 185, 129),        // #10B981 emerald pulse
            heart: Rgba::rgb(251, 113, 133),          // #FB7185
            sleep_zzz: Rgba::rgb(96, 165, 250),       // #60A5FA crisp sky blue for light & dark desktops
            sparkle: Rgba::rgb(251, 191, 36),         // #FBBF24
            shadow: Rgba::new(42, 32, 26, 46),        // soft warm ground shadow
        }
    }
}

impl Default for Palette {
    fn default() -> Self {
        Self::milkbird_day()
    }
}
