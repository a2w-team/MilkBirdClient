//! Color palette for the original "奶鸟 (MilkBird)" character.
//!
//! Takes loose creative inspiration from fiery campus red-bird energy while remaining a
//! 100% original designer-toy character: lively coral-vermilion plumage, pure creamy
//! milk-foam belly fluff (no logos), mango-gold flame tuft, warm peach-tipped wings,
//! starry espresso-jewel eyes, and honey-apricot beak & feet.

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
    pub crest_tip: Rgba,
    pub crest_base: Rgba,
    pub crest_cream: Rgba,
    pub wing_base: Rgba,
    pub wing_tip: Rgba,
    pub wing_shade: Rgba,
    pub tail_base: Rgba,
    pub tail_tip: Rgba,
    pub beak_top: Rgba,
    pub beak_base: Rgba,
    pub beak_inner: Rgba,
    pub foot_base: Rgba,
    pub foot_shade: Rgba,
    pub cheek: Rgba,
    pub eye_ink: Rgba,
    pub eye_sheen: Rgba,
    pub eye_sparkle: Rgba,
    pub badge_alert: Rgba,
    pub badge_ok: Rgba,
    pub heart: Rgba,
    pub sleep_zzz: Rgba,
    pub sparkle: Rgba,
    pub shadow: Rgba,
}

impl Palette {
    /// Original MilkBird ("奶鸟") palette: coral-flame plumage + creamy milk-foam fluff + mango-gold crest.
    pub const fn milkbird_original() -> Self {
        Self {
            body_highlight: Rgba::rgb(255, 102, 92),   // #FF665C sunny coral sheen
            body_base: Rgba::rgb(239, 64, 62),         // #EF403E lively coral-red plumage
            body_shade: Rgba::rgb(200, 38, 52),        // #C82634 warm berry shadow
            belly_highlight: Rgba::rgb(255, 254, 250), // #FFFEFA pure milk foam
            belly_base: Rgba::rgb(255, 249, 238),      // #FFF9EE warm creamy milk
            belly_shade: Rgba::rgb(242, 230, 214),     // #F2E6D6 soft oat-milk shadow
            outline: Rgba::rgb(102, 34, 42),           // #66222A soft warm berry-cocoa contour
            crest_tip: Rgba::rgb(255, 190, 64),        // #FFBE40 bright mango-gold tip
            crest_base: Rgba::rgb(255, 108, 54),       // #FF6C36 warm tangerine flame
            crest_cream: Rgba::rgb(255, 244, 224),     // #FFF4E0 milk-foam secondary sprout
            wing_base: Rgba::rgb(232, 58, 60),         // #E83A3C coral wing shoulder
            wing_tip: Rgba::rgb(255, 228, 206),        // #FFE4CE soft peach-milk feather tip
            wing_shade: Rgba::rgb(190, 36, 52),        // #BE2434 back-wing depth
            tail_base: Rgba::rgb(224, 52, 56),         // #E03438 coral tail
            tail_tip: Rgba::rgb(255, 174, 58),         // #FFAE3A golden-tangerine tail tip
            beak_top: Rgba::rgb(253, 214, 96),         // #FDD660 bright honey-gold highlight
            beak_base: Rgba::rgb(245, 162, 28),        // #F5A21C warm apricot beak
            beak_inner: Rgba::rgb(244, 114, 140),      // #F4728C rosy open mouth
            foot_base: Rgba::rgb(245, 166, 48),        // #F5A630 warm honey-apricot feet
            foot_shade: Rgba::rgb(214, 128, 22),       // #D68016 foot shadow
            cheek: Rgba::rgb(255, 174, 186),           // #FFAEBA soft strawberry-milk blush
            eye_ink: Rgba::rgb(32, 24, 34),            // #201822 deep starry espresso-plum
            eye_sheen: Rgba::rgb(255, 186, 92),        // #FFBA5C subtle warm amber iris crescent
            eye_sparkle: Rgba::rgb(255, 255, 255),     // #FFFFFF crisp twin catchlights
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
