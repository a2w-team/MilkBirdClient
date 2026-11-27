//! Cross-platform windowing abstraction, multi-monitor geometry, persistent config,
//! and native macOS desktop pet shell.

use crate::bubble::{BubbleAction, BubbleLayout, BubbleState, ScreenRect};
use crate::model::LOGICAL_CANVAS_SIZE;
use crate::render::PetRenderer;
use crate::scheduler::{FrameDecision, FrameScheduler};
use crate::state::{BehaviorMode, PetController, PetEvent, PetState, ScreenEdge};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[cfg(target_os = "macos")]
pub mod macos;

/// Logical window width & height matching the `160x160` procedural canvas.
pub const PET_WINDOW_SIZE: f64 = LOGICAL_CANVAS_SIZE as f64;

/// Distance (in logical points) within which releasing a drag snaps MilkBird to a screen edge.
pub const EDGE_SNAP_THRESHOLD: f64 = 24.0;

/// User-facing actions triggered from the native right-click menu or menubar status item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuCommand {
    OpenMilkBirdTray,
    SetBehaviorMode(BehaviorMode),
    DemoFeed,
    DemoPat,
    DemoAlert,
    ToggleSleep,
    ToggleStealth,
    ResetPosition,
    Quit,
}

/// Startup CLI options for `milkbird-pet`.
#[derive(Debug, Clone, Default)]
pub struct LaunchOptions {
    pub initial_mode: Option<BehaviorMode>,
    pub initial_state: Option<PetState>,
    pub demo_bubble: Option<String>,
    pub duration_secs: Option<f64>,
    pub auto_suspend_fullscreen: Option<bool>,
    pub config_path: Option<PathBuf>,
}

/// Persisted desktop pet preferences stored in `${MILKBIRD_TRAY_DIR:-$HOME/.milkbird}/pet-config.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PetConfig {
    #[serde(default)]
    pub behavior_mode: BehaviorMode,
    #[serde(default)]
    pub pos_x: Option<f64>,
    #[serde(default)]
    pub pos_y: Option<f64>,
    #[serde(default)]
    pub sleeping: bool,
    #[serde(default = "default_true")]
    pub auto_suspend_fullscreen: bool,
}

const fn default_true() -> bool {
    true
}

impl Default for PetConfig {
    fn default() -> Self {
        Self {
            behavior_mode: BehaviorMode::FreeRoam,
            pos_x: None,
            pos_y: None,
            sleeping: false,
            auto_suspend_fullscreen: true,
        }
    }
}

impl PetConfig {
    /// Resolve default config path inside the MilkBird data directory (`~/.milkbird/pet-config.json`).
    pub fn default_path() -> PathBuf {
        if let Ok(dir) = std::env::var("MILKBIRD_TRAY_DIR") {
            let trimmed = dir.trim();
            if !trimmed.is_empty() {
                return PathBuf::from(trimmed).join("pet-config.json");
            }
        }
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home)
                .join(".milkbird")
                .join("pet-config.json");
        }
        PathBuf::from("pet-config.json")
    }

    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(raw) => serde_json::from_str(&raw).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let body = serde_json::to_string_pretty(self)
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, body)?;
        std::fs::rename(tmp, path)?;
        Ok(())
    }
}

/// Result of clamping and snapping a pet window origin against multi-monitor visible work areas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EdgeSnapResult {
    pub x: f64,
    pub y: f64,
    pub snapped_edge: Option<ScreenEdge>,
    pub screen: ScreenRect,
}

/// Find the visible monitor work area containing `(cx, cy)`, or the closest monitor if outside all.
pub fn find_target_screen(screens: &[ScreenRect], cx: f64, cy: f64) -> ScreenRect {
    if screens.is_empty() {
        return ScreenRect::new(0.0, 0.0, 1440.0, 900.0);
    }
    if let Some(hit) = screens.iter().copied().find(|s| s.contains_point(cx, cy)) {
        return hit;
    }
    screens
        .iter()
        .copied()
        .min_by(|a, b| {
            let da = dist_sq_to_rect(*a, cx, cy);
            let db = dist_sq_to_rect(*b, cx, cy);
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap_or(screens[0])
}

fn dist_sq_to_rect(r: ScreenRect, px: f64, py: f64) -> f64 {
    let clamped_x = px.clamp(r.x, r.right());
    let clamped_y = py.clamp(r.y, r.top());
    let dx = px - clamped_x;
    let dy = py - clamped_y;
    dx * dx + dy * dy
}

/// Default spawn origin: comfortably resting near the bottom-right corner of the primary screen.
pub fn default_spawn_origin(primary_screen: ScreenRect) -> (f64, f64) {
    let x = (primary_screen.right() - PET_WINDOW_SIZE - 48.0).max(primary_screen.x + 12.0);
    let y = primary_screen.y + 8.0;
    (x, y)
}

/// Clamp and snap the pet window origin to the nearest screen edge (left, right, or bottom).
pub fn snap_window_to_edges(
    origin_x: f64,
    origin_y: f64,
    screens: &[ScreenRect],
    allow_snap: bool,
) -> EdgeSnapResult {
    let cx = origin_x + PET_WINDOW_SIZE * 0.5;
    let cy = origin_y + PET_WINDOW_SIZE * 0.5;
    let screen = find_target_screen(screens, cx, cy);

    let min_x = screen.x - 10.0;
    let max_x = (screen.right() - PET_WINDOW_SIZE + 10.0).max(min_x);
    let min_y = screen.y;
    let max_y = (screen.top() - PET_WINDOW_SIZE).max(min_y);

    let mut x = origin_x.clamp(min_x, max_x);
    let mut y = origin_y.clamp(min_y, max_y);
    let mut snapped_edge = None;

    if allow_snap {
        let dist_left = (x - screen.x).abs();
        let dist_right = ((x + PET_WINDOW_SIZE) - screen.right()).abs();
        let dist_bottom = (y - screen.y).abs();

        if dist_left <= EDGE_SNAP_THRESHOLD && dist_left <= dist_right {
            x = screen.x - 16.0;
            snapped_edge = Some(ScreenEdge::Left);
        } else if dist_right <= EDGE_SNAP_THRESHOLD {
            x = screen.right() - PET_WINDOW_SIZE + 16.0;
            snapped_edge = Some(ScreenEdge::Right);
        } else if dist_bottom <= EDGE_SNAP_THRESHOLD {
            y = screen.y;
            snapped_edge = Some(ScreenEdge::Bottom);
        }
    }

    EdgeSnapResult {
        x,
        y,
        snapped_edge,
        screen,
    }
}

/// Step output produced by [`PetSession::step`] for the native platform layer to apply.
#[derive(Debug, Clone)]
pub struct StepOutput {
    pub rendered_new_frame: bool,
    pub window_origin_changed: bool,
    pub window_origin: (f64, f64),
    pub bubble: Option<BubbleLayout>,
    pub next_interval: Option<Duration>,
    pub is_suspended: bool,
    pub is_stealth: bool,
}

/// Platform-agnostic live session combining `PetController`, `PetRenderer`, `FrameScheduler`,
/// `BubbleState`, multi-monitor placement, and power-saving suspend reasons.
pub struct PetSession {
    controller: PetController,
    renderer: PetRenderer,
    scheduler: FrameScheduler,
    bubble: BubbleState,
    config: PetConfig,
    config_path: PathBuf,
    origin: (f64, f64),
    dragging: bool,
    stealth_hidden: bool,
    screen_locked: bool,
    system_sleeping: bool,
    fullscreen_active: bool,
    last_tick: Instant,
    combo_announced: u64,
}

impl PetSession {
    pub fn new(
        scale_factor: f32,
        screens: &[ScreenRect],
        options: &LaunchOptions,
    ) -> Self {
        let config_path = options
            .config_path
            .clone()
            .unwrap_or_else(PetConfig::default_path);
        let mut config = PetConfig::load(&config_path);
        if let Some(mode) = options.initial_mode {
            config.behavior_mode = mode;
        }
        if let Some(auto_fs) = options.auto_suspend_fullscreen {
            config.auto_suspend_fullscreen = auto_fs;
        }

        let primary = screens
            .first()
            .copied()
            .unwrap_or_else(|| ScreenRect::new(0.0, 0.0, 1440.0, 900.0));
        let raw_origin = match (config.pos_x, config.pos_y) {
            (Some(x), Some(y)) => (x, y),
            _ => default_spawn_origin(primary),
        };
        let snapped = snap_window_to_edges(raw_origin.0, raw_origin.1, screens, false);

        let mut controller = PetController::default();
        controller.handle_event(PetEvent::SetBehaviorMode(config.behavior_mode));
        if let Some(state) = options.initial_state {
            controller.handle_event(PetEvent::DemoState(state));
        } else if config.sleeping {
            controller.handle_event(PetEvent::ForceSleep);
        }

        let mut bubble = BubbleState::new();
        if let Some(ref text) = options.demo_bubble {
            bubble.show(text.clone(), 5.0, BubbleAction::Dismiss);
        } else if options.initial_state.is_none() && !config.sleeping {
            bubble.show("咕咕~ 小奶鸟已就位，右键可以切换模式哦！", 3.6, BubbleAction::Dismiss);
        }

        let mut renderer = PetRenderer::new(scale_factor);
        let (initial_pose, _) = controller.tick(0.0);
        renderer.render(&initial_pose);

        Self {
            controller,
            renderer,
            scheduler: FrameScheduler::new(),
            bubble,
            config,
            config_path,
            origin: (snapped.x, snapped.y),
            dragging: false,
            stealth_hidden: false,
            screen_locked: false,
            system_sleeping: false,
            fullscreen_active: false,
            last_tick: Instant::now(),
            combo_announced: 0,
        }
    }

    pub fn origin(&self) -> (f64, f64) {
        self.origin
    }

    pub fn window_frame(&self) -> ScreenRect {
        ScreenRect::new(self.origin.0, self.origin.1, PET_WINDOW_SIZE, PET_WINDOW_SIZE)
    }

    pub fn state(&self) -> PetState {
        self.controller.state()
    }

    pub fn mode(&self) -> BehaviorMode {
        self.controller.mode()
    }

    pub fn config(&self) -> &PetConfig {
        &self.config
    }

    pub fn renderer(&self) -> &PetRenderer {
        &self.renderer
    }

    pub fn scheduler(&self) -> &FrameScheduler {
        &self.scheduler
    }

    pub fn is_stealth(&self) -> bool {
        self.stealth_hidden
    }

    pub fn is_suspended(&self) -> bool {
        self.controller.state() == PetState::Suspended || self.effective_suspend()
    }

    fn effective_suspend(&self) -> bool {
        self.stealth_hidden
            || self.screen_locked
            || self.system_sleeping
            || (self.config.auto_suspend_fullscreen && self.fullscreen_active)
    }

    fn sync_suspend_state(&mut self) {
        let should_suspend = self.effective_suspend();
        self.controller
            .handle_event(PetEvent::SetSuspended(should_suspend));
        if should_suspend {
            self.bubble.dismiss();
        } else {
            self.scheduler.invalidate();
            self.last_tick = Instant::now();
        }
    }

    pub fn set_screen_locked(&mut self, locked: bool) {
        if self.screen_locked != locked {
            self.screen_locked = locked;
            self.sync_suspend_state();
        }
    }

    pub fn set_system_sleeping(&mut self, sleeping: bool) {
        if self.system_sleeping != sleeping {
            self.system_sleeping = sleeping;
            self.sync_suspend_state();
        }
    }

    pub fn set_fullscreen_active(&mut self, fullscreen: bool) {
        if self.fullscreen_active != fullscreen {
            self.fullscreen_active = fullscreen;
            self.sync_suspend_state();
        }
    }

    /// Recreate internal `Pixmap` when moving between `@1x` and `@2x` monitors.
    pub fn update_scale_factor(&mut self, scale_factor: f32) -> bool {
        if (self.renderer.scale() - scale_factor).abs() > 0.05 {
            self.renderer = PetRenderer::new(scale_factor);
            self.scheduler.invalidate();
            true
        } else {
            false
        }
    }

    /// O(1) alpha pixel hit test in logical `160x160` top-left coordinates.
    pub fn hit_test(&self, logical_x: f32, logical_y: f32) -> bool {
        self.renderer.hit_test(logical_x, logical_y)
    }

    /// Feed normalized pointer offset relative to pet center (`-1..1`).
    pub fn on_pointer_move(&mut self, rel_x: f32, rel_y: f32) {
        if self.is_suspended() {
            return;
        }
        self.controller
            .handle_event(PetEvent::PointerMove { rel_x, rel_y });
    }

    /// Feed gentle head stroke when moving cursor over the crown area.
    pub fn on_stroke_head(&mut self, intensity: f32) {
        if self.is_suspended() {
            return;
        }
        self.controller
            .handle_event(PetEvent::StrokeHead { intensity });
    }

    /// Handle click/tap in local top-left `160x160` coordinates.
    pub fn on_click(&mut self, local_x: f32, local_y: f32) {
        if self.is_suspended() {
            return;
        }
        let was_sleeping = self.controller.state() == PetState::Sleep;
        self.controller
            .handle_event(PetEvent::Click { local_x, local_y });
        let (pose_after, _) = self.controller.tick(0.0);
        if was_sleeping {
            self.config.sleeping = false;
            self.persist_config();
            self.bubble
                .show("唔哇醒啦～有什么需要帮忙的吗？", 3.0, BubbleAction::Dismiss);
        } else if matches!(pose_after.prop, crate::model::PropKind::Sparkles { .. }) {
            self.combo_announced += 1;
            self.bubble.show(
                "耶！触发三连彩蛋，转圈圈欢呼～",
                3.2,
                BubbleAction::Dismiss,
            );
        } else if local_y < 92.0 {
            self.bubble
                .show("蹭蹭~ 摸头好舒服呀！", 2.4, BubbleAction::Dismiss);
        }
    }

    pub fn on_drag_start(&mut self) {
        if self.is_suspended() {
            return;
        }
        self.dragging = true;
        self.bubble.dismiss();
        self.controller.handle_event(PetEvent::DragStart);
    }

    pub fn on_drag_move(&mut self, new_origin_x: f64, new_origin_y: f64, dx: f32, dy: f32) {
        if self.is_suspended() {
            return;
        }
        self.origin = (new_origin_x, new_origin_y);
        self.controller.handle_event(PetEvent::DragMove { dx, dy });
    }

    pub fn on_drag_end(&mut self, screens: &[ScreenRect]) -> EdgeSnapResult {
        self.dragging = false;
        let snapped = snap_window_to_edges(self.origin.0, self.origin.1, screens, true);
        self.origin = (snapped.x, snapped.y);
        self.controller.handle_event(PetEvent::DragEnd {
            near_edge: snapped.snapped_edge,
        });
        self.config.pos_x = Some(self.origin.0);
        self.config.pos_y = Some(self.origin.1);
        self.persist_config();

        if let Some(edge) = snapped.snapped_edge {
            let msg = match edge {
                ScreenEdge::Left | ScreenEdge::Right => "贴边站好啦～随时待命！",
                ScreenEdge::Bottom => "稳稳落地！",
            };
            self.bubble.show(msg, 2.2, BubbleAction::Dismiss);
        }
        snapped
    }

    pub fn on_bubble_click(&mut self) -> Option<BubbleAction> {
        self.bubble.dismiss()
    }

    /// Apply a native menu command; returns `true` if the application should quit.
    pub fn apply_menu_command(&mut self, cmd: MenuCommand, screens: &[ScreenRect]) -> bool {
        match cmd {
            MenuCommand::OpenMilkBirdTray => {
                self.bubble
                    .show("正在唤起奶鸟俱乐部托盘…", 2.4, BubbleAction::Dismiss);
                false
            }
            MenuCommand::SetBehaviorMode(mode) => {
                self.stealth_hidden = false;
                self.config.behavior_mode = mode;
                self.config.sleeping = false;
                self.sync_suspend_state();
                self.controller
                    .handle_event(PetEvent::SetBehaviorMode(mode));
                let msg = match mode {
                    BehaviorMode::FreeRoam => "切换为「自由散步」模式～",
                    BehaviorMode::StayPut => "切换为「原地乖坐」模式，安静陪你工作",
                    BehaviorMode::EdgePerch => {
                        let snapped = snap_window_to_edges(self.origin.0, self.origin.1, screens, true);
                        self.origin = (snapped.x, snapped.y);
                        self.config.pos_x = Some(self.origin.0);
                        self.config.pos_y = Some(self.origin.1);
                        "切换为「贴边挂机」模式，不挡屏幕！"
                    }
                };
                self.bubble.show(msg, 2.8, BubbleAction::Dismiss);
                self.persist_config();
                false
            }
            MenuCommand::DemoFeed => {
                self.stealth_hidden = false;
                self.config.sleeping = false;
                self.sync_suspend_state();
                self.controller.handle_event(PetEvent::Click {
                    local_x: 80.0,
                    local_y: 68.0,
                });
                self.bubble
                    .show("嚼嚼嚼～小鱼干真香！元气满满！", 3.2, BubbleAction::Dismiss);
                false
            }
            MenuCommand::DemoPat => {
                self.stealth_hidden = false;
                self.config.sleeping = false;
                self.sync_suspend_state();
                self.controller
                    .handle_event(PetEvent::StrokeHead { intensity: 1.4 });
                self.bubble
                    .show("眯眯眼～谢谢摸头呀！", 2.8, BubbleAction::Dismiss);
                false
            }
            MenuCommand::DemoAlert => {
                self.stealth_hidden = false;
                self.config.sleeping = false;
                self.sync_suspend_state();
                self.controller.handle_event(PetEvent::TrayStatus {
                    unread_count: 1,
                    active_demands: 1,
                    urgent_pulse: true,
                });
                self.bubble.show(
                    "叮咚！校园广场有新的匹配回应，点击打开托盘看看～",
                    4.8,
                    BubbleAction::OpenTray,
                );
                false
            }
            MenuCommand::ToggleSleep => {
                if self.controller.state() == PetState::Sleep {
                    self.config.sleeping = false;
                    self.controller.handle_event(PetEvent::WakeUp);
                    self.bubble
                        .show("伸个懒腰～小奶鸟醒来啦！", 2.8, BubbleAction::Dismiss);
                } else {
                    self.stealth_hidden = false;
                    self.config.sleeping = true;
                    self.sync_suspend_state();
                    self.controller.handle_event(PetEvent::ForceSleep);
                    self.bubble
                        .show("困啦，缩成团子打个盹 Zzz…", 2.4, BubbleAction::Dismiss);
                }
                self.persist_config();
                false
            }
            MenuCommand::ToggleStealth => {
                self.stealth_hidden = !self.stealth_hidden;
                self.sync_suspend_state();
                if !self.stealth_hidden {
                    self.bubble
                        .show("小奶鸟回来啦！", 2.2, BubbleAction::Dismiss);
                }
                false
            }
            MenuCommand::ResetPosition => {
                let primary = screens
                    .first()
                    .copied()
                    .unwrap_or_else(|| ScreenRect::new(0.0, 0.0, 1440.0, 900.0));
                self.origin = default_spawn_origin(primary);
                self.config.pos_x = Some(self.origin.0);
                self.config.pos_y = Some(self.origin.1);
                self.persist_config();
                self.bubble
                    .show("已回到默认角落位置～", 2.2, BubbleAction::Dismiss);
                false
            }
            MenuCommand::Quit => {
                self.persist_config();
                true
            }
        }
    }

    pub fn persist_config(&self) {
        let _ = self.config.save(&self.config_path);
    }

    /// Advance simulation, roaming motion, speech bubble, and adaptive dirty-frame rasterization.
    pub fn step(&mut self, screens: &[ScreenRect]) -> StepOutput {
        let now = Instant::now();
        let dt = now
            .saturating_duration_since(self.last_tick)
            .as_secs_f32()
            .clamp(0.001, 0.10);
        self.last_tick = now;
        self.step_with_dt(dt, screens)
    }

    /// Deterministic step entrypoint used by both the live run loop and headless unit tests.
    pub fn step_with_dt(&mut self, dt_secs: f32, screens: &[ScreenRect]) -> StepOutput {
        let prev_origin = self.origin;
        let (pose, motion) = self.controller.tick(dt_secs);
        self.bubble.tick(dt_secs);

        // Apply horizontal roaming motion inside the current monitor's visible work area.
        if !self.dragging && !self.is_suspended() && motion.dx.abs() > 0.001 {
            let cx = self.origin.0 + PET_WINDOW_SIZE * 0.5;
            let cy = self.origin.1 + PET_WINDOW_SIZE * 0.5;
            let screen = find_target_screen(screens, cx, cy);
            let min_x = screen.x + 8.0;
            let max_x = (screen.right() - PET_WINDOW_SIZE - 8.0).max(min_x);
            let mut next_x = self.origin.0 + motion.dx as f64;
            if next_x <= min_x {
                next_x = min_x;
                self.controller.reverse_walk_direction();
            } else if next_x >= max_x {
                next_x = max_x;
                self.controller.reverse_walk_direction();
            }
            self.origin.0 = next_x;
        }

        let tier = self.controller.suggested_fps_tier();
        let decision = self.scheduler.plan_frame(tier, &pose);

        let (rendered_new_frame, mut next_interval) = match decision {
            FrameDecision::Render {
                tier,
                next_interval,
            } => {
                let start = Instant::now();
                self.renderer.render(&pose);
                self.scheduler
                    .record_render_time_us(start.elapsed().as_micros() as u64);
                (true, power_aware_interval(tier, &pose, next_interval))
            }
            FrameDecision::Skip {
                tier,
                next_interval,
            } => (false, power_aware_interval(tier, &pose, next_interval)),
        };

        // While a bubble is fading in/out, ensure smooth updates even if pet itself is idle.
        if self.bubble.is_animating() && !self.is_suspended() {
            let bubble_interval = Duration::from_millis(33);
            next_interval = Some(match next_interval {
                Some(iv) => iv.min(bubble_interval),
                None => bubble_interval,
            });
        }

        let current_screen = find_target_screen(
            screens,
            self.origin.0 + PET_WINDOW_SIZE * 0.5,
            self.origin.1 + PET_WINDOW_SIZE * 0.5,
        );
        let bubble = if self.is_suspended() {
            None
        } else {
            self.bubble.layout(self.window_frame(), current_screen)
        };

        let window_origin_changed =
            (self.origin.0 - prev_origin.0).abs() > 0.25 || (self.origin.1 - prev_origin.1).abs() > 0.25;

        StepOutput {
            rendered_new_frame,
            window_origin_changed,
            window_origin: self.origin,
            bubble,
            next_interval,
            is_suspended: self.is_suspended(),
            is_stealth: self.stealth_hidden,
        }
    }
}

/// Power-aware desktop presentation interval:
/// - `ActiveSmooth`: full 60 Hz (`16.67 ms`) for silky dragging, hopping, petting & alerts.
/// - `IdleBreeze`: calm breathing paced at ~2.8 Hz (`360 ms`), or `75 ms` mid-blink so blinks stay crisp.
/// - `SleepLow`: cozy curled-up breathing & floating `Zzz` paced at ~0.75 Hz (`1350 ms`) for <= 0.2% CPU.
/// - `Suspended`: `None` (`0.0% CPU`, timer completely stopped).
fn power_aware_interval(
    tier: crate::scheduler::FpsTier,
    pose: &crate::model::PetPose,
    base: Option<Duration>,
) -> Option<Duration> {
    use crate::model::EyeExpression;
    use crate::scheduler::FpsTier;

    match tier {
        FpsTier::Suspended => None,
        FpsTier::ActiveSmooth => base,
        FpsTier::IdleBreeze => {
            if pose.eyes == EyeExpression::Blink {
                Some(Duration::from_millis(75))
            } else {
                Some(Duration::from_millis(420))
            }
        }
        FpsTier::SleepLow => Some(Duration::from_millis(1350)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multi_monitor_snapping_handles_left_right_bottom_edges() {
        let screens = [
            ScreenRect::new(0.0, 0.0, 1440.0, 900.0),
            ScreenRect::new(1440.0, 0.0, 1920.0, 1080.0),
        ];

        // Near right edge of secondary display.
        let right = snap_window_to_edges(1440.0 + 1920.0 - PET_WINDOW_SIZE - 10.0, 400.0, &screens, true);
        assert_eq!(right.snapped_edge, Some(ScreenEdge::Right));
        assert!((right.x - (1440.0 + 1920.0 - PET_WINDOW_SIZE + 16.0)).abs() < 1e-6);

        // Near left edge of primary display.
        let left = snap_window_to_edges(12.0, 300.0, &screens, true);
        assert_eq!(left.snapped_edge, Some(ScreenEdge::Left));
        assert!((left.x - (-16.0)).abs() < 1e-6);

        // Near bottom of primary display.
        let bottom = snap_window_to_edges(600.0, 15.0, &screens, true);
        assert_eq!(bottom.snapped_edge, Some(ScreenEdge::Bottom));
        assert!((bottom.y - 0.0).abs() < 1e-6);
    }

    #[test]
    fn config_round_trip_and_fullscreen_suspend_work() {
        let dir = std::env::temp_dir().join(format!("milkbird-pet-test-{}", std::process::id()));
        let cfg_path = dir.join("pet-config.json");
        let screens = [ScreenRect::new(0.0, 0.0, 1440.0, 900.0)];

        let opts = LaunchOptions {
            config_path: Some(cfg_path.clone()),
            ..LaunchOptions::default()
        };
        let mut session = PetSession::new(2.0, &screens, &opts);
        session.apply_menu_command(MenuCommand::SetBehaviorMode(BehaviorMode::StayPut), &screens);
        session.on_drag_start();
        session.on_drag_move(500.0, 220.0, 12.0, 4.0);
        session.on_drag_end(&screens);

        // Reload config and verify mode + position persisted.
        let reloaded = PetConfig::load(&cfg_path);
        assert_eq!(reloaded.behavior_mode, BehaviorMode::StayPut);
        assert_eq!(reloaded.pos_x, Some(500.0));
        assert_eq!(reloaded.pos_y, Some(220.0));

        // Fullscreen suspension must yield 0 Hz (next_interval == None).
        session.set_fullscreen_active(true);
        let out = session.step_with_dt(0.016, &screens);
        assert!(out.is_suspended);
        assert_eq!(out.next_interval, None);
        assert!(out.bubble.is_none());

        // Resume from fullscreen restores updates.
        session.set_fullscreen_active(false);
        let out_resume = session.step_with_dt(0.016, &screens);
        assert!(!out_resume.is_suspended);
        assert!(out_resume.next_interval.is_some());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
