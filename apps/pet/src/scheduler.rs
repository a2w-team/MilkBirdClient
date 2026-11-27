//! Adaptive frame-rate scheduler (0 / 6 / 15 / 60 Hz) with dirty-frame skipping.
//!
//! Ensures zero wasted CPU cycles when suspended or visually unchanged, ultra-low timer
//! wakeups when sleeping/idle, and instant 60 Hz promotion during interactive animations.

use crate::model::PetPose;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Adaptive pacing tiers ordered from lowest to highest power draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FpsTier {
    /// Fullscreen app / screen lock / stealth: 0 Hz, wait indefinitely until wake event.
    Suspended,
    /// Sleeping curled dumpling: 6 Hz (166 ms coalesced wakeup).
    SleepLow,
    /// Calm idle breathing: 15 Hz (66 ms).
    IdleBreeze,
    /// Active drag, hop, pat, or match celebration: 60 Hz (16.67 ms).
    ActiveSmooth,
}

impl FpsTier {
    /// Target frame rate in Hz (`0` when suspended).
    pub const fn target_hz(self) -> u32 {
        match self {
            Self::Suspended => 0,
            Self::SleepLow => 6,
            Self::IdleBreeze => 15,
            Self::ActiveSmooth => 60,
        }
    }

    /// Suggested sleep interval before the next tick (`None` means wait indefinitely for event).
    pub const fn interval(self) -> Option<Duration> {
        match self {
            Self::Suspended => None,
            Self::SleepLow => Some(Duration::from_millis(166)),
            Self::IdleBreeze => Some(Duration::from_millis(66)),
            Self::ActiveSmooth => Some(Duration::from_micros(16_667)),
        }
    }
}

/// Outcome of deciding whether to rasterize and commit a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameDecision {
    /// Rasterize and present this frame, then sleep for `next_interval`.
    Render {
        tier: FpsTier,
        next_interval: Option<Duration>,
    },
    /// Pose fingerprint matches the last committed frame — skip rasterization completely!
    Skip {
        tier: FpsTier,
        next_interval: Option<Duration>,
    },
}

/// Live efficiency counters for benchmarking and diagnostics.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct SchedulerStats {
    pub rendered_frames: u64,
    pub skipped_frames: u64,
    pub last_render_us: u64,
    pub total_render_us: u64,
}

impl SchedulerStats {
    pub fn avg_render_us(&self) -> f64 {
        if self.rendered_frames == 0 {
            0.0
        } else {
            self.total_render_us as f64 / self.rendered_frames as f64
        }
    }

    pub fn skip_ratio(&self) -> f64 {
        let total = self.rendered_frames + self.skipped_frames;
        if total == 0 {
            0.0
        } else {
            self.skipped_frames as f64 / total as f64
        }
    }
}

/// Dirty-frame deduplicator and adaptive pacing scheduler.
#[derive(Debug, Clone)]
pub struct FrameScheduler {
    last_fingerprint: Option<u64>,
    current_tier: FpsTier,
    stats: SchedulerStats,
}

impl Default for FrameScheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameScheduler {
    pub const fn new() -> Self {
        Self {
            last_fingerprint: None,
            current_tier: FpsTier::IdleBreeze,
            stats: SchedulerStats {
                rendered_frames: 0,
                skipped_frames: 0,
                last_render_us: 0,
                total_render_us: 0,
            },
        }
    }

    pub const fn tier(&self) -> FpsTier {
        self.current_tier
    }

    pub const fn stats(&self) -> SchedulerStats {
        self.stats
    }

    /// Force the next frame to re-render (e.g., after display scale factor or theme change).
    pub fn invalidate(&mut self) {
        self.last_fingerprint = None;
    }

    /// Decide whether a new pose needs rasterization and update the active pacing tier.
    pub fn plan_frame(&mut self, tier: FpsTier, pose: &PetPose) -> FrameDecision {
        self.current_tier = tier;
        let next_interval = tier.interval();

        if tier == FpsTier::Suspended {
            self.stats.skipped_frames += 1;
            return FrameDecision::Skip {
                tier,
                next_interval: None,
            };
        }

        let fp = pose.fingerprint();
        if self.last_fingerprint == Some(fp) {
            self.stats.skipped_frames += 1;
            FrameDecision::Skip {
                tier,
                next_interval,
            }
        } else {
            self.last_fingerprint = Some(fp);
            self.stats.rendered_frames += 1;
            FrameDecision::Render {
                tier,
                next_interval,
            }
        }
    }

    /// Record elapsed rasterization time in microseconds for diagnostics.
    pub fn record_render_time_us(&mut self, elapsed_us: u64) {
        self.stats.last_render_us = elapsed_us;
        self.stats.total_render_us += elapsed_us;
    }
}
