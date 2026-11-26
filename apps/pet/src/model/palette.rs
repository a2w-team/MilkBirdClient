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
    pub hair_cocoa: Rgba,
    pub hair_cocoa_hi: Rgba,
    pub hair_cream: Rgba,
    pub hair_cream_shade: Rgba,
    pub hair_tie_gold: Rgba,
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
            hair_base: Rgba::rgb(239, 64, 62),         // #EF403E seamless body-matched natural ahoge
            hair_highlight: Rgba::rgb(255, 118, 102),  // #FF7666 ahoge warm sheen
            hair_cocoa: Rgba::rgb(68, 38, 42),         // #44262A rich warm espresso-cocoa chibi hair
            hair_cocoa_hi: Rgba::rgb(112, 66, 70),     // #704246 silky hair highlight
            hair_cream: Rgba::rgb(248, 224, 188),      // #F8E0BC cozy milk-tea caramel blonde curls
            hair_cream_shade: Rgba::rgb(226, 186, 138), // #E2BA8A soft caramel curl shadow
            hair_tie_gold: Rgba::rgb(252, 202, 70),    // #FCCA46 cute golden hair-tie accent
            flame_core: Rgba::rgb(255, 154, 24),       // #FF9A18 warm fiery orange tip (ref 1)
            flame_mid: Rgba::rgb(255, 96, 12),         // #FF600C vivid mid orange flame (ref 1)
            flame_outer: Rgba::rgb(238, 38, 10),       // #EE260A vibrant flame-red base (ref 1)
            wing_base: Rgba::rgb(232, 58, 60),         // #E83A3C coral wing shoulder
            wing_tip: Rgba::rgb(255, 228, 206),        // #FFE4CE soft peach-milk feather tip
            wing_shade: Rgba::rgb(188, 34, 50),        // #BC2232 back-wing depth
            tail_base: Rgba::rgb(224, 52, 56),         // #E03438 coral tail
            tail_tip: Rgba::rgb(255, 174, 58),         // #FFAE3A golden-tangerine tail tip
            mouth_line: Rgba::rgb(78, 22, 30),         // #4E161E crisp frog-mouth line
            mouth_cavity: Rgba::rgb(96, 18, 28),       // #60121C deep Nailong open mouth
            mouth_tongue: Rgba::rgb(236, 86, 112),     // #EC5670 rosy frog tongue
            foot_base: Rgba::rgb(245, 166, 48),        // #F5A630 warm honey-apricot feet
            foot_shade: Rgba::rgb(214, 128, 22),       // #D68016 foot shadow
            cheek: Rgba::rgb(255, 174, 186),           // #FFAEBA soft strawberry-milk blush
            eye_sclera: Rgba::rgb(252, 253, 250),      // #FCFDFA clean soft white sclera (ref 2)
            eye_green_outer: Rgba::rgb(42, 92, 46),    // #2A5C2E deep Nailong moss-green iris rim (ref 2)
            eye_green_inner: Rgba::rgb(112, 188, 92),  // #70BC5C luminous Nailong olive-emerald lower glow (ref 2)
            eye_pupil: Rgba::rgb(20, 28, 22),          // #141C16 deep hollow glassy dark pupil (ref 2)
            eye_sparkle: Rgba::rgb(255, 255, 255),     // #FFFFFF crisp star/pinpoint catchlight
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
