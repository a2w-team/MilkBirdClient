//! Performance-first native desktop pet core for MilkBird Club.
//!
//! Architecture layers:
//! - [`model`]: Original procedural vector rig ("小奶鸟 / MilkBird") with 11 layers and 7 expressions.
//! - [`physics`]: Unconditionally stable sub-stepped second-order damped springs.
//! - [`state`]: 8-state behavioral FSM and interaction controller.
//! - [`scheduler`]: Adaptive frame-rate scheduler (0 / 6 / 15 / 60 Hz) with dirty-frame skipping.
//! - [`render`]: Pre-allocated `tiny-skia` vector renderer with O(1) alpha hit-testing.
//! - [`bubble`]: Lightweight speech-bubble state, CJK sizing, and automatic screen-edge avoidance.
//! - [`bridge`]: Non-blocking `tray.sock` v1 Unix Socket bridge with smart backoff and offline fallback.
//! - [`platform`]: Cross-platform windowing abstraction, config persistence, and native macOS AppKit shell.

pub mod bridge;
pub mod bubble;
pub mod model;
pub mod physics;
pub mod platform;
pub mod render;
pub mod scheduler;
pub mod state;
