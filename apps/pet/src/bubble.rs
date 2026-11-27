//! Lightweight speech bubble state machine, CJK text measurement, and edge-avoidance layout.
//!
//! Designed so the native macOS shell can render crisp system-font (`PingFang SC`) frosted-glass
//! bubbles without bundling any font files or running a WebView, while keeping all geometry and
//! timing logic 100% pure-Rust and unit-testable.

use serde::{Deserialize, Serialize};

/// Logical rectangle in screen coordinates (origin at bottom-left, matching macOS AppKit).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ScreenRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl ScreenRect {
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn right(self) -> f64 {
        self.x + self.width
    }

    pub fn top(self) -> f64 {
        self.y + self.height
    }

    pub fn center_x(self) -> f64 {
        self.x + self.width * 0.5
    }

    pub fn center_y(self) -> f64 {
        self.y + self.height * 0.5
    }

    pub fn contains_point(self, px: f64, py: f64) -> bool {
        px >= self.x && px <= self.right() && py >= self.y && py <= self.top()
    }
}

/// Optional click action attached to a speech bubble (used by M3 tray bridge for room jump).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum BubbleAction {
    #[default]
    Dismiss,
    OpenTray,
    OpenRoom(String),
}

/// An active speech bubble shown near MilkBird.
#[derive(Debug, Clone, PartialEq)]
pub struct BubbleMessage {
    pub text: String,
    pub action: BubbleAction,
    pub elapsed_secs: f32,
    pub duration_secs: f32,
}

impl BubbleMessage {
    /// Smooth fade-in (`0.18 s`) and fade-out (`0.35 s`) alpha in `0.0..=1.0`.
    pub fn opacity(&self) -> f32 {
        let fade_in = 0.18_f32;
        let fade_out = 0.35_f32;
        let remaining = (self.duration_secs - self.elapsed_secs).max(0.0);
        if remaining <= 0.0 {
            return 0.0;
        }
        let alpha_in = if self.elapsed_secs < fade_in {
            (self.elapsed_secs / fade_in).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let alpha_out = if remaining < fade_out {
            (remaining / fade_out).clamp(0.0, 1.0)
        } else {
            1.0
        };
        alpha_in.min(alpha_out)
    }

    /// True while opacity is actively animating (requires smooth updates).
    pub fn is_fading(&self) -> bool {
        let remaining = self.duration_secs - self.elapsed_secs;
        self.elapsed_secs < 0.22 || remaining < 0.40
    }
}

/// Computed placement and appearance snapshot for the native bubble panel.
#[derive(Debug, Clone, PartialEq)]
pub struct BubbleLayout {
    pub frame: ScreenRect,
    pub placed_below: bool,
    pub opacity: f32,
    pub text: String,
}

/// Pure-Rust controller managing speech-bubble lifecycle.
#[derive(Debug, Clone, Default)]
pub struct BubbleState {
    active: Option<BubbleMessage>,
    revision: u64,
}

impl BubbleState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn active(&self) -> Option<&BubbleMessage> {
        self.active.as_ref()
    }

    pub fn is_visible(&self) -> bool {
        self.active.is_some()
    }

    pub fn is_animating(&self) -> bool {
        self.active.as_ref().is_some_and(BubbleMessage::is_fading)
    }

    /// Show or replace the active bubble for `duration_secs` (clamped to `1.2..=15.0`).
    pub fn show(&mut self, text: impl Into<String>, duration_secs: f32, action: BubbleAction) {
        let trimmed = text.into().trim().to_string();
        if trimmed.is_empty() {
            return;
        }
        self.active = Some(BubbleMessage {
            text: trimmed,
            action,
            elapsed_secs: 0.0,
            duration_secs: duration_secs.clamp(1.2, 15.0),
        });
        self.revision = self.revision.wrapping_add(1);
    }

    /// Dismiss the active bubble immediately and return its click action if any.
    pub fn dismiss(&mut self) -> Option<BubbleAction> {
        let msg = self.active.take()?;
        self.revision = self.revision.wrapping_add(1);
        Some(msg.action)
    }

    /// Advance elapsed time; returns `true` if visibility/opacity changed meaningfully.
    pub fn tick(&mut self, dt_secs: f32) -> bool {
        let Some(msg) = self.active.as_mut() else {
            return false;
        };
        let dt = dt_secs.clamp(0.0, 0.25);
        let prev_q = (msg.opacity() * 40.0).round() as i32;
        msg.elapsed_secs += dt;
        if msg.elapsed_secs >= msg.duration_secs {
            self.active = None;
            self.revision = self.revision.wrapping_add(1);
            return true;
        }
        let next_q = (msg.opacity() * 40.0).round() as i32;
        prev_q != next_q
    }

    /// Compute native bubble frame positioned cleanly near the pet and kept strictly inside `screen`.
    pub fn layout(&self, pet_frame: ScreenRect, screen: ScreenRect) -> Option<BubbleLayout> {
        let msg = self.active.as_ref()?;
        let opacity = msg.opacity();
        if opacity <= 0.005 {
            return None;
        }
        let (width, height) = measure_bubble_size(&msg.text);
        let (frame, placed_below) = place_bubble_frame(width, height, pet_frame, screen);
        Some(BubbleLayout {
            frame,
            placed_below,
            opacity,
            text: msg.text.clone(),
        })
    }
}

/// Estimate bubble `(width, height)` in logical points for mixed Chinese/ASCII text (`12.5 pt` font).
pub fn measure_bubble_size(text: &str) -> (f64, f64) {
    const MIN_W: f64 = 104.0;
    const MAX_W: f64 = 236.0;
    const PAD_X: f64 = 24.0;
    const PAD_Y: f64 = 16.0;
    const LINE_H: f64 = 18.0;

    let max_content_w = MAX_W - PAD_X;
    let mut lines: usize = 0;
    let mut widest_line: f64 = 0.0;

    for raw_line in text.lines() {
        let mut cur_w = 0.0_f64;
        let mut line_count_for_para = 1usize;
        for ch in raw_line.chars() {
            let adv = char_advance_pt(ch);
            if cur_w + adv > max_content_w && cur_w > 0.0 {
                widest_line = widest_line.max(cur_w);
                cur_w = adv;
                line_count_for_para += 1;
            } else {
                cur_w += adv;
            }
        }
        widest_line = widest_line.max(cur_w);
        lines += line_count_for_para;
    }

    let lines = lines.clamp(1, 4);
    let width = (widest_line + PAD_X).clamp(MIN_W, MAX_W).ceil();
    let height = (lines as f64 * LINE_H + PAD_Y).ceil();
    (width, height)
}

fn char_advance_pt(ch: char) -> f64 {
    if ch.is_ascii() {
        if ch.is_ascii_uppercase() {
            8.2
        } else if ch == ' ' || ch == 'i' || ch == 'l' || ch == '!' || ch == '.' {
            4.5
        } else {
            7.2
        }
    } else {
        // Full-width CJK / emoji glyph at ~12.5pt PingFang SC.
        13.2
    }
}

/// Position a bubble of `(bubble_w, bubble_h)` above MilkBird's crown, flipping below when close to
/// the top edge and clamping horizontally inside `screen` with safe padding.
pub fn place_bubble_frame(
    bubble_w: f64,
    bubble_h: f64,
    pet_frame: ScreenRect,
    screen: ScreenRect,
) -> (ScreenRect, bool) {
    const MARGIN: f64 = 8.0;
    // Pet sits inside a 160x160 logical canvas; crown peaks near top - 22pt, feet near bottom + 14pt.
    let above_y = pet_frame.top() - 16.0;
    let below_y = pet_frame.y - bubble_h + 10.0;

    let (mut y, placed_below) = if above_y + bubble_h <= screen.top() - MARGIN {
        (above_y, false)
    } else if below_y >= screen.y + MARGIN {
        (below_y, true)
    } else {
        // Extreme cramped screen: clamp strictly inside screen vertical bounds.
        let clamped_y = above_y.clamp(
            screen.y + MARGIN,
            (screen.top() - MARGIN - bubble_h).max(screen.y + MARGIN),
        );
        (clamped_y, false)
    };

    let min_x = screen.x + MARGIN;
    let max_x = (screen.right() - MARGIN - bubble_w).max(min_x);
    let ideal_x = pet_frame.center_x() - bubble_w * 0.5;
    let x = ideal_x.clamp(min_x, max_x);

    let max_y = (screen.top() - MARGIN - bubble_h).max(screen.y + MARGIN);
    y = y.clamp(screen.y + MARGIN, max_y);

    (ScreenRect::new(x, y, bubble_w, bubble_h), placed_below)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bubble_avoids_top_and_horizontal_screen_edges() {
        let screen = ScreenRect::new(0.0, 0.0, 1440.0, 900.0);
        // Pet near top-right corner of screen.
        let pet = ScreenRect::new(1310.0, 760.0, 160.0, 160.0);
        let mut state = BubbleState::new();
        state.show("叮咚！奶鸟俱乐部收到一条新的匹配回复～", 4.0, BubbleAction::OpenTray);
        state.tick(0.2);

        let layout = state.layout(pet, screen).expect("bubble visible");
        assert!(layout.placed_below, "should flip below when near top edge");
        assert!(layout.frame.right() <= screen.right() - 8.0 + 1e-6);
        assert!(layout.frame.x >= screen.x + 8.0 - 1e-6);
        assert!(layout.frame.y >= screen.y + 8.0 - 1e-6);
        assert!(layout.frame.top() <= screen.top() - 8.0 + 1e-6);
    }

    #[test]
    fn bubble_fades_in_and_expires_cleanly() {
        let mut state = BubbleState::new();
        state.show("咕咕~ 摸头好舒服", 2.0, BubbleAction::Dismiss);
        assert!(state.is_visible());

        state.tick(0.09);
        let mid_in = state.active().unwrap().opacity();
        assert!(mid_in > 0.3 && mid_in < 0.7, "fade-in opacity={mid_in}");

        state.tick(0.25);
        assert!((state.active().unwrap().opacity() - 1.0).abs() < 1e-4);

        for _ in 0..10 {
            state.tick(0.2);
        }
        assert!(!state.is_visible());
    }
}
