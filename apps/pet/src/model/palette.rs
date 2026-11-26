//! Color palette for the original "奶鸟 (MilkBird)" IP character.
//!
//! Core IP visual anchors:
//! 1. Living orange-gold little flame atop customizable hairstyles.
//! 2. Vacant-cute hollow green eyes (inspired by Nailong's emerald iris + deep hollow pupil).
//! 3. Expressive wide frog mouth (deadpan chill line, round "o", or big joyful grin).
//!
//! Paired with lively coral-red plumage and a clean creamy milk-foam tummy (zero logos).

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

/// Complete original MilkBird palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Palette {
    pub body_highlight: Rgba,
    pub body_base: Rgba,
    pub body_shade: Rgba,
    pub belly_highlight: Rgba,
    pub belly_base: Rgba,
    pub belly_shade: Rgba,
    pub outline: Rgba,
    pub hair_base: Rgba,
    pub hair_highlight: Rgba,
    pub flame_core: Rgba,
    pub flame_mid: Rgba,
    pub flame_outer: Rgba,
    pub wing_base: Rgba,
    pub wing_tip: Rgba,
    pub wing_shade: Rgba,
    pub tail_base: Rgba,
    pub tail_tip: Rgba,
    pub mouth_line: Rgba,
    pub mouth_cavity: Rgba,
    pub mouth_tongue: Rgba,
    pub foot_base: Rgba,
    pub foot_shade: Rgba,
    pub cheek: Rgba,
    pub eye_sclera: Rgba,
    pub eye_green_outer: Rgba,
    pub eye_green_inner: Rgba,
    pub eye_pupil: Rgba,
    pub eye_sparkle: Rgba,
    pub badge_alert: Rgba,
    pub badge_ok: Rgba,
    pub heart: Rgba,
    pub sleep_zzz: Rgba,
    pub sparkle: Rgba,
    pub shadow: Rgba,
}

impl Palette {
    /// Original MilkBird ("奶鸟") palette with IP anchors: living flame + hollow green eyes + frog mouth.
    pub const fn milkbird_original() -> Self {
        Self {
            body_highlight: Rgba::rgb(255, 102, 92),   // #FF665C sunny coral sheen
            body_base: Rgba::rgb(239, 64, 62),         // #EF403E lively coral-red plumage
            body_shade: Rgba::rgb(198, 36, 50),        // #C62432 warm berry shadow
            belly_highlight: Rgba::rgb(255, 254, 250), // #FFFEFA pure milk foam
            belly_base: Rgba::rgb(255, 249, 238),      // #FFF9EE warm creamy milk
            belly_shade: Rgba::rgb(242, 230, 214),     // #F2E6D6 soft oat-milk shadow
            outline: Rgba::rgb(96, 30, 38),            // #601E26 crisp warm berry-cocoa contour
            hair_base: Rgba::rgb(244, 72, 58),         // #F4483A soft coral hair tuft
            hair_highlight: Rgba::rgb(255, 136, 114),  // #FF8872 hair highlight
            flame_core: Rgba::rgb(255, 243, 128),      // #FFF380 bright warm flame heart
            flame_mid: Rgba::rgb(255, 152, 34),        // #FF9822 glowing tangerine flame
            flame_outer: Rgba::rgb(246, 66, 20),       // #F64214 fiery vermilion flame outer
            wing_base: Rgba::rgb(232, 58, 60),         // #E83A3C coral wing shoulder
            wing_tip: Rgba::rgb(255, 228, 206),        // #FFE4CE soft peach-milk feather tip
            wing_shade: Rgba::rgb(188, 34, 50),        // #BC2232 back-wing depth
            tail_base: Rgba::rgb(224, 52, 56),         // #E03438 coral tail
            tail_tip: Rgba::rgb(255, 174, 58),         // #FFAE3A golden-tangerine tail tip
            mouth_line: Rgba::rgb(78, 22, 30),         // #4E161E crisp frog-mouth line
            mouth_cavity: Rgba::rgb(112, 20, 34),      // #701422 deep cheerful open mouth
            mouth_tongue: Rgba::rgb(251, 113, 133),    // #FB7185 rosy frog tongue
            foot_base: Rgba::rgb(245, 166, 48),        // #F5A630 warm honey-apricot feet
            foot_shade: Rgba::rgb(214, 128, 22),       // #D68016 foot shadow
            cheek: Rgba::rgb(255, 174, 186),           // #FFAEBA soft strawberry-milk blush
            eye_sclera: Rgba::rgb(255, 255, 255),      // #FFFFFF clean white sclera
            eye_green_outer: Rgba::rgb(22, 128, 62),   // #16803E rich emerald outer iris rim
            eye_green_inner: Rgba::rgb(74, 222, 118),  // #4ADE76 vivid Nailong jade-green glow
            eye_pupil: Rgba::rgb(15, 26, 20),          // #0F1A14 deep hollow void pupil
            eye_sparkle: Rgba::rgb(255, 255, 255),     // #FFFFFF crisp star/dot highlight
            badge_alert: Rgba::rgb(37, 99, 235),       // #2563EB crisp cobalt unread pill
            badge_ok: Rgba::rgb(16, 185, 129),         // #10B981 emerald active watch
            heart: Rgba::rgb(255, 118, 152),           // #FF7698 floating hearts
            sleep_zzz: Rgba::rgb(125, 186, 255),       // #7DBAFF dreamy sky Zzz
            sparkle: Rgba::rgb(253, 204, 58),          // #FDCC3A twinkling gold stars
            shadow: Rgba::new(56, 24, 28, 44),         // soft ground contact shadow
        }
    }

    pub const fn milkbird_day() -> Self {
        Self::milkbird_original()
    }
}

impl Default for Palette {
    fn default() -> Self {
        Self::milkbird_original()
    }
}
