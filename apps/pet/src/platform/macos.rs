//! Zero-WebView native macOS AppKit + QuartzCore + CoreGraphics desktop pet shell.
//!
//! Architecture highlights:
//! - `NSPanel` (`Borderless | NonactivatingPanel`, `isOpaque = false`, `clearColor`, `NSFloatingWindowLevel`)
//!   floats cleanly above normal windows without stealing keyboard focus or showing a Dock icon.
//! - Direct `CALayer.contents = CGImage` backed by the pre-allocated `tiny-skia` premultiplied RGBA8
//!   buffer — zero WebView, zero GPU WebGL context, zero per-frame image decoding.
//! - Double-guaranteed pixel-level click-through: macOS transparent window hit-testing + custom
//!   `NSView::hitTest:` querying `PetRenderer::hit_test` (`O(1)` alpha lookup).
//! - Adaptive single-shot coalesced `NSTimer` (`60 / 15 / 6 Hz`), completely invalidated (`0.0% CPU`)
//!   when suspended (fullscreen app, screen lock, macOS sleep, or stealth mode).
//! - Native frosted-glass `NSVisualEffectView` + `PingFang SC` (`NSTextField`) speech bubble panel
//!   with automatic screen-edge avoidance.

use crate::bubble::{BubbleAction, BubbleLayout, ScreenRect};
use crate::platform::{LaunchOptions, MenuCommand, PET_WINDOW_SIZE, PetSession};
use crate::state::BehaviorMode;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, NSObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSColor,
    NSControlStateValueOff, NSControlStateValueOn, NSEvent, NSFloatingWindowLevel, NSFont,
    NSMenu, NSMenuItem, NSPanel, NSScreen, NSStatusBar, NSStatusItem, NSTextAlignment, NSTextField,
    NSTrackingArea, NSTrackingAreaOptions, NSVariableStatusItemLength, NSView,
    NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView,
    NSWindowCollectionBehavior, NSWindowStyleMask, NSWorkspace,
    NSWorkspaceActiveSpaceDidChangeNotification, NSWorkspaceDidActivateApplicationNotification,
    NSWorkspaceDidWakeNotification, NSWorkspaceScreensDidWakeNotification,
    NSWorkspaceWillSleepNotification,
};
use objc2_core_foundation::{CFData, CFRetained};
use objc2_core_graphics::{
    CGBitmapInfo, CGColorRenderingIntent, CGColorSpace, CGDataProvider, CGImage, CGImageAlphaInfo,
    CGWindowListCopyWindowInfo, CGWindowListOption, kCGNullWindowID,
};
use objc2_foundation::{
    NSArray, NSDictionary, NSDistributedNotificationCenter, NSNotification, NSNumber, NSPoint,
    NSRect, NSSize, NSString, NSTimer,
};
use objc2_quartz_core::{CALayer, CATransaction};
use std::cell::{Cell, RefCell};
use std::process::Command;
use std::rc::Rc;
use std::time::{Duration, Instant};

const DRAG_THRESHOLD_PT: f64 = 3.5;
const POINTER_THROTTLE_MS: u128 = 75;
const MILK_BIRD_TRAY_BUNDLE_ID: &str = "cn.fordece.milkbird.tray";

// Menu item tags for clean target/action dispatch.
const TAG_OPEN_TRAY: isize = 100;
const TAG_MODE_FREE: isize = 101;
const TAG_MODE_STAY: isize = 102;
const TAG_MODE_EDGE: isize = 103;
const TAG_DEMO_FEED: isize = 104;
const TAG_DEMO_PAT: isize = 105;
const TAG_DEMO_ALERT: isize = 106;
const TAG_TOGGLE_SLEEP: isize = 107;
const TAG_TOGGLE_STEALTH: isize = 108;
const TAG_RESET_POS: isize = 109;
const TAG_QUIT: isize = 110;

/// Shared main-thread state accessed by `PetView`, `BubbleView`, and `PetAppDelegate`.
struct SharedRuntime {
    session: PetSession,
    color_space: CFRetained<CGColorSpace>,
    pet_panel: Option<Retained<NSPanel>>,
    pet_layer: Option<Retained<CALayer>>,
    bubble_panel: Option<Retained<NSPanel>>,
    bubble_label: Option<Retained<NSTextField>>,
    bubble_effect: Option<Retained<NSVisualEffectView>>,
    status_item: Option<Retained<NSStatusItem>>,
    frame_timer: Option<Retained<NSTimer>>,
    duration_timer: Option<Retained<NSTimer>>,
    delegate: Option<Retained<PetAppDelegate>>,
    last_bubble_text: String,
    last_bubble_visible: bool,
    // Mouse drag & stroke tracking.
    mouse_down_screen: Option<NSPoint>,
    mouse_down_origin: Option<(f64, f64)>,
    last_drag_screen: Option<NSPoint>,
    is_dragging: bool,
    last_pointer_sample: Instant,
    last_crown_x: Option<f32>,
    crown_stroke_accum: f32,
}

impl SharedRuntime {
    fn new(scale: f32, screens: &[ScreenRect], options: &LaunchOptions) -> Self {
        let color_space =
            CGColorSpace::new_device_rgb().expect("macOS device RGB colorspace available");
        Self {
            session: PetSession::new(scale, screens, options),
            color_space,
            pet_panel: None,
            pet_layer: None,
            bubble_panel: None,
            bubble_label: None,
            bubble_effect: None,
            status_item: None,
            frame_timer: None,
            duration_timer: None,
            delegate: None,
            last_bubble_text: String::new(),
            last_bubble_visible: false,
            mouse_down_screen: None,
            mouse_down_origin: None,
            last_drag_screen: None,
            is_dragging: false,
            last_pointer_sample: Instant::now(),
            last_crown_x: None,
            crown_stroke_accum: 0.0,
        }
    }

    fn upload_current_frame(&self) {
        let Some(ref layer) = self.pet_layer else {
            return;
        };
        let (w, h) = self.session.renderer().pixel_size();
        let bytes = self.session.renderer().pixels();
        let cf_data = CFData::from_bytes(bytes);
        let Some(provider) = CGDataProvider::with_cf_data(Some(&cf_data)) else {
            return;
        };
        let bitmap_info = CGBitmapInfo(CGImageAlphaInfo::PremultipliedLast.0);
        let cg_image = unsafe {
            CGImage::new(
                w as usize,
                h as usize,
                8,
                32,
                (w * 4) as usize,
                Some(&self.color_space),
                bitmap_info,
                Some(&provider),
                std::ptr::null(),
                true,
                CGColorRenderingIntent::RenderingIntentDefault,
            )
        };
        if let Some(img) = cg_image {
            CATransaction::begin();
            CATransaction::setDisableActions(true);
            unsafe {
                let raw_obj: &AnyObject = &*(CFRetained::as_ptr(&img).as_ptr().cast::<AnyObject>());
                layer.setContents(Some(raw_obj));
                layer.setContentsScale(self.session.renderer().scale() as f64);
            }
            CATransaction::commit();
        }
    }

    fn apply_window_origin(&self, origin: (f64, f64)) {
        if let Some(ref panel) = self.pet_panel {
            panel.setFrameOrigin(NSPoint::new(origin.0, origin.1));
        }
    }

    fn sync_visibility(&self) {
        if let Some(ref panel) = self.pet_panel {
            if self.session.is_stealth() {
                panel.orderOut(None);
            } else {
                panel.orderFrontRegardless();
            }
        }
    }

    fn sync_bubble(&mut self, layout: Option<BubbleLayout>) {
        let (Some(panel), Some(label), Some(effect)) = (
            self.bubble_panel.as_ref(),
            self.bubble_label.as_ref(),
            self.bubble_effect.as_ref(),
        ) else {
            return;
        };

        let Some(layout) = layout else {
            if self.last_bubble_visible {
                panel.orderOut(None);
                self.last_bubble_visible = false;
            }
            return;
        };

        if self.last_bubble_text != layout.text {
            let ns_text = NSString::from_str(&layout.text);
            label.setStringValue(&ns_text);
            self.last_bubble_text = layout.text;
        }

        let frame = NSRect::new(
            NSPoint::new(layout.frame.x, layout.frame.y),
            NSSize::new(layout.frame.width, layout.frame.height),
        );
        panel.setFrame_display(frame, true);
        effect.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(layout.frame.width, layout.frame.height),
        ));
        label.setFrame(NSRect::new(
            NSPoint::new(11.0, 6.0),
            NSSize::new((layout.frame.width - 22.0).max(20.0), (layout.frame.height - 12.0).max(16.0)),
        ));
        panel.setAlphaValue(layout.opacity as f64);

        if !self.last_bubble_visible {
            panel.orderFrontRegardless();
            self.last_bubble_visible = true;
        }
    }

    fn stop_frame_timer(&mut self) {
        if let Some(timer) = self.frame_timer.take() {
            timer.invalidate();
        }
    }

    fn schedule_next_tick(&mut self, interval: Option<Duration>) {
        self.stop_frame_timer();
        let Some(delay) = interval else {
            // Suspended (0.0% CPU): leave timer unscheduled until a wake event arrives.
            return;
        };
        let Some(ref delegate) = self.delegate else {
            return;
        };
        let secs = delay.as_secs_f64().max(0.008);
        let timer = unsafe {
            NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                secs,
                delegate,
                sel!(onFrameTimer:),
                None,
                false,
            )
        };
        // Allow macOS timer coalescing in low-FPS Idle/Sleep tiers to reduce CPU wakeups.
        let tolerance = if secs >= 0.25 {
            secs * 0.25
        } else if secs >= 0.05 {
            secs * 0.20
        } else {
            0.001
        };
        timer.setTolerance(tolerance);
        self.frame_timer = Some(timer);
    }

    fn refresh_status_menu(&self, mtm: MainThreadMarker) {
        let (Some(item), Some(delegate)) = (self.status_item.as_ref(), self.delegate.as_ref()) else {
            return;
        };
        let menu = build_context_menu(mtm, delegate, &self.session);
        item.setMenu(Some(&menu));
    }
}

/// Query all active macOS monitors' visible work areas (excluding menu bar & Dock).
fn visible_screens(mtm: MainThreadMarker) -> Vec<ScreenRect> {
    let screens = NSScreen::screens(mtm);
    let mut out = Vec::with_capacity(screens.len());
    for i in 0..screens.len() {
        let screen = screens.objectAtIndex(i);
        let vis = screen.visibleFrame();
        out.push(ScreenRect::new(
            vis.origin.x,
            vis.origin.y,
            vis.size.width,
            vis.size.height,
        ));
    }
    if out.is_empty() {
        out.push(ScreenRect::new(0.0, 0.0, 1440.0, 900.0));
    }
    out
}

/// Query all active macOS monitors' full display frames (for fullscreen app comparison).
fn full_screen_bounds(mtm: MainThreadMarker) -> Vec<(f64, f64)> {
    let screens = NSScreen::screens(mtm);
    let mut out = Vec::with_capacity(screens.len());
    for i in 0..screens.len() {
        let frame = screens.objectAtIndex(i).frame();
        out.push((frame.size.width, frame.size.height));
    }
    out
}

/// Event-driven fullscreen detection (runs ONLY on active-app / space change, 0% steady-state CPU).
fn detect_frontmost_fullscreen(mtm: MainThreadMarker) -> bool {
    let workspace = NSWorkspace::sharedWorkspace();
    let Some(front_app) = workspace.frontmostApplication() else {
        return false;
    };
    let my_pid = std::process::id() as i32;
    let front_pid = front_app.processIdentifier();
    if front_pid <= 0 || front_pid == my_pid {
        return false;
    }
    if let Some(bundle_id) = front_app.bundleIdentifier()
        && bundle_id.to_string() == "com.apple.finder"
    {
        return false;
    }

    let screen_sizes = full_screen_bounds(mtm);
    if screen_sizes.is_empty() {
        return false;
    }

    let options = CGWindowListOption::OptionOnScreenOnly
        | CGWindowListOption::ExcludeDesktopElements;
    let Some(cf_arr) = CGWindowListCopyWindowInfo(options, kCGNullWindowID) else {
        return false;
    };

    // CoreFoundation CFArray of CFDictionary is toll-free bridged to NSArray<NSDictionary>.
    let ns_arr: &NSArray<NSDictionary<NSString, AnyObject>> =
        unsafe { &*(CFRetained::as_ptr(&cf_arr).as_ptr().cast()) };

    let key_pid = NSString::from_str("kCGWindowOwnerPID");
    let key_layer = NSString::from_str("kCGWindowLayer");
    let key_bounds = NSString::from_str("kCGWindowBounds");
    let key_w = NSString::from_str("Width");
    let key_h = NSString::from_str("Height");

    for i in 0..ns_arr.len() {
        let dict = ns_arr.objectAtIndex(i);
        let pid_match = dict
            .objectForKey(&key_pid)
            .and_then(|obj| obj.downcast::<NSNumber>().ok())
            .is_some_and(|num| num.intValue() == front_pid);
        if !pid_match {
            continue;
        }

        let is_normal_layer = dict
            .objectForKey(&key_layer)
            .and_then(|obj| obj.downcast::<NSNumber>().ok())
            .is_some_and(|num| num.intValue() == 0);
        if !is_normal_layer {
            continue;
        }

        let Some(bounds_obj) = dict.objectForKey(&key_bounds) else {
            continue;
        };
        let Ok(bounds) = bounds_obj.downcast::<NSDictionary>() else {
            continue;
        };
        // Read Width and Height numbers safely via raw objc dictionary lookup.
        let w = unsafe {
            let val: Option<Retained<AnyObject>> = msg_send![&*bounds, objectForKey: &*key_w];
            val.and_then(|v| v.downcast::<NSNumber>().ok())
                .map(|n| n.doubleValue())
                .unwrap_or(0.0)
        };
        let h = unsafe {
            let val: Option<Retained<AnyObject>> = msg_send![&*bounds, objectForKey: &*key_h];
            val.and_then(|v| v.downcast::<NSNumber>().ok())
                .map(|n| n.doubleValue())
                .unwrap_or(0.0)
        };

        for &(sw, sh) in &screen_sizes {
            if w >= sw - 2.0 && h >= sh - 2.0 {
                return true;
            }
        }
    }

    false
}

/// Wake or launch the MilkBird Tray app using macOS `open -b` (zero edits to `apps/tray/`).
fn activate_milkbird_tray() {
    let _ = Command::new("open")
        .args(["-b", MILK_BIRD_TRAY_BUNDLE_ID])
        .spawn();
}

fn menu_command_from_tag(tag: isize) -> Option<MenuCommand> {
    match tag {
        TAG_OPEN_TRAY => Some(MenuCommand::OpenMilkBirdTray),
        TAG_MODE_FREE => Some(MenuCommand::SetBehaviorMode(BehaviorMode::FreeRoam)),
        TAG_MODE_STAY => Some(MenuCommand::SetBehaviorMode(BehaviorMode::StayPut)),
        TAG_MODE_EDGE => Some(MenuCommand::SetBehaviorMode(BehaviorMode::EdgePerch)),
        TAG_DEMO_FEED => Some(MenuCommand::DemoFeed),
        TAG_DEMO_PAT => Some(MenuCommand::DemoPat),
        TAG_DEMO_ALERT => Some(MenuCommand::DemoAlert),
        TAG_TOGGLE_SLEEP => Some(MenuCommand::ToggleSleep),
        TAG_TOGGLE_STEALTH => Some(MenuCommand::ToggleStealth),
        TAG_RESET_POS => Some(MenuCommand::ResetPosition),
        TAG_QUIT => Some(MenuCommand::Quit),
        _ => None,
    }
}

fn add_menu_item(
    mtm: MainThreadMarker,
    menu: &NSMenu,
    title: &str,
    tag: isize,
    target: &PetAppDelegate,
    checked: bool,
) {
    let ns_title = NSString::from_str(title);
    let empty = NSString::from_str("");
    let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            mtm.alloc(),
            &ns_title,
            Some(sel!(onMenuAction:)),
            &empty,
        )
    };
    item.setTag(tag);
    unsafe {
        item.setTarget(Some(target));
    }
    item.setState(if checked {
        NSControlStateValueOn
    } else {
        NSControlStateValueOff
    });
    menu.addItem(&item);
}

fn build_context_menu(
    mtm: MainThreadMarker,
    delegate: &PetAppDelegate,
    session: &PetSession,
) -> Retained<NSMenu> {
    let menu_title = NSString::from_str("小奶鸟");
    let menu = NSMenu::initWithTitle(mtm.alloc(), &menu_title);
    menu.setAutoenablesItems(false);

    add_menu_item(mtm, &menu, "打开奶鸟俱乐部托盘", TAG_OPEN_TRAY, delegate, false);
    menu.addItem(&NSMenuItem::separatorItem(mtm));

    let mode = session.mode();
    add_menu_item(
        mtm,
        &menu,
        "自由散步",
        TAG_MODE_FREE,
        delegate,
        mode == BehaviorMode::FreeRoam,
    );
    add_menu_item(
        mtm,
        &menu,
        "原地乖坐",
        TAG_MODE_STAY,
        delegate,
        mode == BehaviorMode::StayPut,
    );
    add_menu_item(
        mtm,
        &menu,
        "贴边挂机",
        TAG_MODE_EDGE,
        delegate,
        mode == BehaviorMode::EdgePerch,
    );
    menu.addItem(&NSMenuItem::separatorItem(mtm));

    add_menu_item(mtm, &menu, "投喂小鱼干", TAG_DEMO_FEED, delegate, false);
    add_menu_item(mtm, &menu, "摸摸小奶鸟", TAG_DEMO_PAT, delegate, false);
    add_menu_item(mtm, &menu, "测试通知气泡", TAG_DEMO_ALERT, delegate, false);
    menu.addItem(&NSMenuItem::separatorItem(mtm));

    let sleep_label = if session.state() == crate::state::PetState::Sleep {
        "唤醒小奶鸟"
    } else {
        "休眠打盹"
    };
    add_menu_item(mtm, &menu, sleep_label, TAG_TOGGLE_SLEEP, delegate, false);

    let stealth_label = if session.is_stealth() {
        "恢复显示小奶鸟"
    } else {
        "暂时隐身（0% CPU）"
    };
    add_menu_item(mtm, &menu, stealth_label, TAG_TOGGLE_STEALTH, delegate, false);
    add_menu_item(mtm, &menu, "重置到默认角落", TAG_RESET_POS, delegate, false);
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    add_menu_item(mtm, &menu, "退出桌宠", TAG_QUIT, delegate, false);

    menu
}

/// Perform one runtime tick, upload dirty frame if changed, sync bubble, and arm next coalesced timer.
fn run_tick_and_schedule(rt_rc: &Rc<RefCell<SharedRuntime>>, mtm: MainThreadMarker) {
    let screens = visible_screens(mtm);
    let mut rt = rt_rc.borrow_mut();

    // Sync Retina backing scale factor if moved across monitors.
    if let Some(ref panel) = rt.pet_panel {
        let scale = panel.backingScaleFactor() as f32;
        if rt.session.update_scale_factor(scale) {
            rt.upload_current_frame();
        }
    }

    let step = rt.session.step(&screens);
    if step.window_origin_changed {
        rt.apply_window_origin(step.window_origin);
    }
    if step.rendered_new_frame {
        rt.upload_current_frame();
    }
    rt.sync_bubble(step.bubble);
    rt.schedule_next_tick(step.next_interval);
}

// -----------------------------------------------------------------------------
// Objective-C Delegate (Timers, Menus, Workspace/Screen-Lock Notifications)
// -----------------------------------------------------------------------------

struct PetAppDelegateIvars {
    runtime: Rc<RefCell<SharedRuntime>>,
    check_fullscreen: Cell<bool>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = PetAppDelegateIvars]
    struct PetAppDelegate;

    impl PetAppDelegate {
        #[unsafe(method(onFrameTimer:))]
        fn on_frame_timer(&self, _timer: &NSTimer) {
            let mtm = MainThreadMarker::from(self);
            run_tick_and_schedule(&self.ivars().runtime, mtm);
        }

        #[unsafe(method(onDurationExpired:))]
        fn on_duration_expired(&self, _timer: &NSTimer) {
            let mtm = MainThreadMarker::from(self);
            {
                let rt = self.ivars().runtime.borrow();
                let stats = rt.session.scheduler().stats();
                println!(
                    "milkbird-pet summary: state={:?}, rendered={}, skipped={} ({:.1}%), avg_render_us={:.1}",
                    rt.session.state(),
                    stats.rendered_frames,
                    stats.skipped_frames,
                    stats.skip_ratio() * 100.0,
                    stats.avg_render_us()
                );
            }
            let app = NSApplication::sharedApplication(mtm);
            app.terminate(None);
        }

        #[unsafe(method(onMenuAction:))]
        fn on_menu_action(&self, sender: &NSMenuItem) {
            let mtm = MainThreadMarker::from(self);
            let Some(cmd) = menu_command_from_tag(sender.tag()) else {
                return;
            };
            if cmd == MenuCommand::OpenMilkBirdTray {
                activate_milkbird_tray();
            }
            let screens = visible_screens(mtm);
            let should_quit = {
                let mut rt = self.ivars().runtime.borrow_mut();
                let quit = rt.session.apply_menu_command(cmd, &screens);
                let origin = rt.session.origin();
                rt.apply_window_origin(origin);
                rt.sync_visibility();
                rt.refresh_status_menu(mtm);
                quit
            };
            if should_quit {
                let app = NSApplication::sharedApplication(mtm);
                app.terminate(None);
            } else {
                run_tick_and_schedule(&self.ivars().runtime, mtm);
            }
        }

        #[unsafe(method(onWorkspaceWillSleep:))]
        fn on_workspace_will_sleep(&self, _note: &NSNotification) {
            let mut rt = self.ivars().runtime.borrow_mut();
            rt.session.set_system_sleeping(true);
            rt.stop_frame_timer();
            rt.sync_bubble(None);
        }

        #[unsafe(method(onWorkspaceDidWake:))]
        fn on_workspace_did_wake(&self, _note: &NSNotification) {
            let mtm = MainThreadMarker::from(self);
            {
                let mut rt = self.ivars().runtime.borrow_mut();
                rt.session.set_system_sleeping(false);
            }
            run_tick_and_schedule(&self.ivars().runtime, mtm);
        }

        #[unsafe(method(onScreenLocked:))]
        fn on_screen_locked(&self, _note: &NSNotification) {
            let mut rt = self.ivars().runtime.borrow_mut();
            rt.session.set_screen_locked(true);
            rt.stop_frame_timer();
            rt.sync_bubble(None);
        }

        #[unsafe(method(onScreenUnlocked:))]
        fn on_screen_unlocked(&self, _note: &NSNotification) {
            let mtm = MainThreadMarker::from(self);
            {
                let mut rt = self.ivars().runtime.borrow_mut();
                rt.session.set_screen_locked(false);
            }
            run_tick_and_schedule(&self.ivars().runtime, mtm);
        }

        #[unsafe(method(onActiveAppOrSpaceChanged:))]
        fn on_active_app_or_space_changed(&self, _note: &NSNotification) {
            if !self.ivars().check_fullscreen.get() {
                return;
            }
            let mtm = MainThreadMarker::from(self);
            let fullscreen = detect_frontmost_fullscreen(mtm);
            let was_suspended = {
                let mut rt = self.ivars().runtime.borrow_mut();
                let prev = rt.session.is_suspended();
                rt.session.set_fullscreen_active(fullscreen);
                if rt.session.is_suspended() {
                    rt.stop_frame_timer();
                    rt.sync_bubble(None);
                }
                prev
            };
            if was_suspended && !fullscreen {
                run_tick_and_schedule(&self.ivars().runtime, mtm);
            }
        }
    }
);

impl PetAppDelegate {
    fn new(
        mtm: MainThreadMarker,
        runtime: Rc<RefCell<SharedRuntime>>,
        check_fullscreen: bool,
    ) -> Retained<Self> {
        let this = mtm.alloc::<Self>().set_ivars(PetAppDelegateIvars {
            runtime,
            check_fullscreen: Cell::new(check_fullscreen),
        });
        unsafe { msg_send![super(this), init] }
    }
}

// -----------------------------------------------------------------------------
// Custom Interactive PetView (Alpha Click-Through, Drag, Pat, Stroke, Menu)
// -----------------------------------------------------------------------------

struct PetViewIvars {
    runtime: Rc<RefCell<SharedRuntime>>,
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[ivars = PetViewIvars]
    struct PetView;

    impl PetView {
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> Bool {
            Bool::YES
        }

        #[unsafe(method(hitTest:))]
        fn hit_test_override(&self, point: NSPoint) -> *mut NSView {
            // Convert AppKit bottom-left coordinates into tiny-skia top-left 160x160 coordinates.
            let local_x = point.x as f32;
            let local_y = (PET_WINDOW_SIZE - point.y) as f32;
            let hit = self.ivars().runtime.borrow().session.hit_test(local_x, local_y);
            if hit {
                let this: &NSView = self;
                (this as *const NSView).cast_mut()
            } else {
                std::ptr::null_mut()
            }
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, _event: &NSEvent) {
            let screen_pt = NSEvent::mouseLocation();
            let mut rt = self.ivars().runtime.borrow_mut();
            rt.mouse_down_screen = Some(screen_pt);
            rt.last_drag_screen = Some(screen_pt);
            rt.mouse_down_origin = Some(rt.session.origin());
            rt.is_dragging = false;
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, _event: &NSEvent) {
            let mtm = MainThreadMarker::from(self);
            let cur = NSEvent::mouseLocation();
            let mut should_tick = false;
            {
                let mut rt = self.ivars().runtime.borrow_mut();
                let (Some(start), Some(orig)) = (rt.mouse_down_screen, rt.mouse_down_origin) else {
                    return;
                };
                let total_dx = cur.x - start.x;
                let total_dy = cur.y - start.y;
                if !rt.is_dragging
                    && (total_dx * total_dx + total_dy * total_dy).sqrt() >= DRAG_THRESHOLD_PT
                {
                    rt.is_dragging = true;
                    rt.session.on_drag_start();
                    rt.sync_bubble(None);
                }
                if rt.is_dragging {
                    let prev = rt.last_drag_screen.unwrap_or(start);
                    let step_dx = (cur.x - prev.x) as f32;
                    let step_dy = (prev.y - cur.y) as f32;
                    let new_origin = (orig.0 + total_dx, orig.1 + total_dy);
                    rt.last_drag_screen = Some(cur);
                    rt.session.on_drag_move(new_origin.0, new_origin.1, step_dx, step_dy);
                    rt.apply_window_origin(new_origin);
                    should_tick = true;
                }
            }
            if should_tick {
                run_tick_and_schedule(&self.ivars().runtime, mtm);
            }
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            let mtm = MainThreadMarker::from(self);
            let screens = visible_screens(mtm);
            let was_dragging = {
                let mut rt = self.ivars().runtime.borrow_mut();
                let dragging = rt.is_dragging;
                rt.is_dragging = false;
                rt.mouse_down_screen = None;
                rt.mouse_down_origin = None;
                rt.last_drag_screen = None;
                if dragging {
                    let snapped = rt.session.on_drag_end(&screens);
                    rt.apply_window_origin((snapped.x, snapped.y));
                } else {
                    let pt = self.convertPoint_fromView(event.locationInWindow(), None);
                    let local_x = pt.x as f32;
                    let local_y = (PET_WINDOW_SIZE - pt.y) as f32;
                    rt.session.on_click(local_x, local_y);
                    rt.refresh_status_menu(mtm);
                }
                dragging
            };
            let _ = was_dragging;
            run_tick_and_schedule(&self.ivars().runtime, mtm);
        }

        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &NSEvent) {
            let mtm = MainThreadMarker::from(self);
            let pt = self.convertPoint_fromView(event.locationInWindow(), None);
            let local_x = pt.x as f32;
            let local_y = (PET_WINDOW_SIZE - pt.y) as f32;

            let mut need_tick = false;
            {
                let mut rt = self.ivars().runtime.borrow_mut();
                if rt.is_dragging || rt.session.is_suspended() {
                    return;
                }
                if rt.last_pointer_sample.elapsed().as_millis() < POINTER_THROTTLE_MS {
                    return;
                }
                rt.last_pointer_sample = Instant::now();

                let rel_x = ((local_x - 80.0) / 72.0).clamp(-1.0, 1.0);
                let rel_y = ((local_y - 78.0) / 72.0).clamp(-1.0, 1.0);
                rt.session.on_pointer_move(rel_x, rel_y);

                // Gentle back-and-forth rub across MilkBird's crown triggers head-petting!
                if (44.0..=116.0).contains(&local_x) && (34.0..=78.0).contains(&local_y) {
                    if let Some(prev_x) = rt.last_crown_x {
                        let travel = (local_x - prev_x).abs();
                        if travel >= 3.0 {
                            rt.crown_stroke_accum += travel;
                            if rt.crown_stroke_accum >= 28.0 {
                                rt.crown_stroke_accum = 0.0;
                                rt.session.on_stroke_head(1.0);
                                need_tick = true;
                            }
                        }
                    }
                    rt.last_crown_x = Some(local_x);
                } else {
                    rt.last_crown_x = None;
                    rt.crown_stroke_accum = 0.0;
                }
            }
            if need_tick {
                run_tick_and_schedule(&self.ivars().runtime, mtm);
            }
        }

        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, event: &NSEvent) {
            let mtm = MainThreadMarker::from(self);
            let menu = {
                let rt = self.ivars().runtime.borrow();
                let Some(ref delegate) = rt.delegate else {
                    return;
                };
                build_context_menu(mtm, delegate, &rt.session)
            };
            NSMenu::popUpContextMenu_withEvent_forView(&menu, event, self);
        }
    }
);

impl PetView {
    fn new(mtm: MainThreadMarker, frame: NSRect, runtime: Rc<RefCell<SharedRuntime>>) -> Retained<Self> {
        let this = mtm
            .alloc::<Self>()
            .set_ivars(PetViewIvars { runtime });
        let view: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };
        view.setWantsLayer(true);

        // Lightweight tracking area for gaze follow + gentle crown petting.
        let opts = NSTrackingAreaOptions::MouseMoved
            | NSTrackingAreaOptions::ActiveAlways
            | NSTrackingAreaOptions::InVisibleRect;
        let tracking = unsafe {
            NSTrackingArea::initWithRect_options_owner_userInfo(
                mtm.alloc(),
                frame,
                opts,
                Some(&view),
                None,
            )
        };
        view.addTrackingArea(&tracking);
        view
    }
}

// -----------------------------------------------------------------------------
// Clickable Bubble Overlay View (Dismiss / Jump to Tray on Click)
// -----------------------------------------------------------------------------

struct BubbleClickViewIvars {
    runtime: Rc<RefCell<SharedRuntime>>,
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[ivars = BubbleClickViewIvars]
    struct BubbleClickView;

    impl BubbleClickView {
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> Bool {
            Bool::YES
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, _event: &NSEvent) {
            let action = {
                let mut rt = self.ivars().runtime.borrow_mut();
                let act = rt.session.on_bubble_click();
                rt.sync_bubble(None);
                act
            };
            if let Some(BubbleAction::OpenTray | BubbleAction::OpenRoom(_)) = action {
                activate_milkbird_tray();
            }
        }
    }
);

impl BubbleClickView {
    fn new(mtm: MainThreadMarker, frame: NSRect, runtime: Rc<RefCell<SharedRuntime>>) -> Retained<Self> {
        let this = mtm
            .alloc::<Self>()
            .set_ivars(BubbleClickViewIvars { runtime });
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }
}

// -----------------------------------------------------------------------------
// Entry Point
// -----------------------------------------------------------------------------

/// Launch the native macOS AppKit run loop for MilkBird Desktop Pet.
pub fn run(options: LaunchOptions) -> Result<(), String> {
    let mtm = MainThreadMarker::new()
        .ok_or_else(|| "milkbird-pet 必须在 macOS 主线程运行".to_string())?;

    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    let screens = visible_screens(mtm);
    let initial_scale = NSScreen::mainScreen(mtm)
        .map(|s| s.backingScaleFactor() as f32)
        .unwrap_or(2.0);

    let runtime = Rc::new(RefCell::new(SharedRuntime::new(
        initial_scale,
        &screens,
        &options,
    )));
    let check_fullscreen = runtime.borrow().session.config().auto_suspend_fullscreen;
    let delegate = PetAppDelegate::new(mtm, Rc::clone(&runtime), check_fullscreen);

    // 1. Create transparent, borderless, non-activating always-on-top NSPanel for MilkBird.
    let origin = runtime.borrow().session.origin();
    let pet_rect = NSRect::new(
        NSPoint::new(origin.0, origin.1),
        NSSize::new(PET_WINDOW_SIZE, PET_WINDOW_SIZE),
    );
    let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
    let pet_panel = NSPanel::initWithContentRect_styleMask_backing_defer(
        mtm.alloc(),
        pet_rect,
        style,
        NSBackingStoreType::Buffered,
        false,
    );
    pet_panel.setOpaque(false);
    pet_panel.setBackgroundColor(Some(&NSColor::clearColor()));
    pet_panel.setHasShadow(false);
    pet_panel.setLevel(NSFloatingWindowLevel);
    pet_panel.setHidesOnDeactivate(false);
    pet_panel.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::Stationary
            | NSWindowCollectionBehavior::IgnoresCycle,
    );
    pet_panel.setIgnoresMouseEvents(false);

    let pet_view = PetView::new(
        mtm,
        NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(PET_WINDOW_SIZE, PET_WINDOW_SIZE),
        ),
        Rc::clone(&runtime),
    );
    let pet_layer = pet_view
        .layer()
        .ok_or_else(|| "无法创建 CALayer".to_string())?;
    pet_panel.setContentView(Some(&pet_view));

    // 2. Create lightweight frosted-glass Chinese speech bubble panel (`PingFang SC`).
    let bubble_init = NSRect::new(NSPoint::new(origin.0, origin.1 + 150.0), NSSize::new(160.0, 40.0));
    let bubble_panel = NSPanel::initWithContentRect_styleMask_backing_defer(
        mtm.alloc(),
        bubble_init,
        style,
        NSBackingStoreType::Buffered,
        false,
    );
    bubble_panel.setOpaque(false);
    bubble_panel.setBackgroundColor(Some(&NSColor::clearColor()));
    bubble_panel.setHasShadow(true);
    bubble_panel.setLevel(NSFloatingWindowLevel + 1);
    bubble_panel.setHidesOnDeactivate(false);
    bubble_panel.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::Stationary
            | NSWindowCollectionBehavior::IgnoresCycle,
    );

    let bubble_root = BubbleClickView::new(
        mtm,
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(160.0, 40.0)),
        Rc::clone(&runtime),
    );
    let effect_view = NSVisualEffectView::initWithFrame(
        mtm.alloc(),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(160.0, 40.0)),
    );
    effect_view.setMaterial(NSVisualEffectMaterial::Popover);
    effect_view.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
    effect_view.setState(NSVisualEffectState::Active);
    effect_view.setWantsLayer(true);
    if let Some(elayer) = effect_view.layer() {
        elayer.setCornerRadius(12.0);
        elayer.setMasksToBounds(true);
        elayer.setBorderWidth(0.75);
    }

    let empty_text = NSString::from_str("");
    let label = NSTextField::wrappingLabelWithString(&empty_text, mtm);
    label.setFont(Some(&NSFont::systemFontOfSize(12.5)));
    label.setAlignment(NSTextAlignment::Center);
    label.setTextColor(Some(&NSColor::labelColor()));
    label.setDrawsBackground(false);
    label.setBezeled(false);
    label.setEditable(false);
    label.setSelectable(false);
    label.setFrame(NSRect::new(NSPoint::new(10.0, 6.0), NSSize::new(140.0, 28.0)));

    effect_view.addSubview(&label);
    bubble_root.addSubview(&effect_view);
    bubble_panel.setContentView(Some(&bubble_root));

    // 3. Install lightweight menubar status item (ensures recovery from stealth mode).
    let status_bar = NSStatusBar::systemStatusBar();
    let status_item = status_bar.statusItemWithLength(NSVariableStatusItemLength);
    if let Some(button) = status_item.button(mtm) {
        let title = NSString::from_str("奶鸟");
        button.setTitle(&title);
    }

    // 4. Register power-saving macOS notifications (sleep, wake, lock, unlock, fullscreen space/app).
    let ws_center = NSWorkspace::sharedWorkspace().notificationCenter();
    unsafe {
        ws_center.addObserver_selector_name_object(
            &delegate,
            sel!(onWorkspaceWillSleep:),
            Some(NSWorkspaceWillSleepNotification),
            None,
        );
        ws_center.addObserver_selector_name_object(
            &delegate,
            sel!(onWorkspaceDidWake:),
            Some(NSWorkspaceDidWakeNotification),
            None,
        );
        ws_center.addObserver_selector_name_object(
            &delegate,
            sel!(onWorkspaceDidWake:),
            Some(NSWorkspaceScreensDidWakeNotification),
            None,
        );
        ws_center.addObserver_selector_name_object(
            &delegate,
            sel!(onActiveAppOrSpaceChanged:),
            Some(NSWorkspaceActiveSpaceDidChangeNotification),
            None,
        );
        ws_center.addObserver_selector_name_object(
            &delegate,
            sel!(onActiveAppOrSpaceChanged:),
            Some(NSWorkspaceDidActivateApplicationNotification),
            None,
        );

        let dist_center = NSDistributedNotificationCenter::defaultCenter();
        let lock_name = NSString::from_str("com.apple.screenIsLocked");
        let unlock_name = NSString::from_str("com.apple.screenIsUnlocked");
        dist_center.addObserver_selector_name_object(
            &delegate,
            sel!(onScreenLocked:),
            Some(&lock_name),
            None,
        );
        dist_center.addObserver_selector_name_object(
            &delegate,
            sel!(onScreenUnlocked:),
            Some(&unlock_name),
            None,
        );
    }

    // 5. Wire runtime handles, upload initial frame, show window, and arm timers.
    {
        let mut rt = runtime.borrow_mut();
        rt.pet_panel = Some(pet_panel.clone());
        rt.pet_layer = Some(pet_layer);
        rt.bubble_panel = Some(bubble_panel);
        rt.bubble_label = Some(label);
        rt.bubble_effect = Some(effect_view);
        rt.status_item = Some(status_item);
        rt.delegate = Some(delegate.clone());
        rt.refresh_status_menu(mtm);
        rt.upload_current_frame();

        if check_fullscreen && detect_frontmost_fullscreen(mtm) {
            rt.session.set_fullscreen_active(true);
        }
        rt.sync_visibility();

        if let Some(secs) = options.duration_secs {
            let dur_timer = unsafe {
                NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                    secs,
                    &delegate,
                    sel!(onDurationExpired:),
                    None,
                    false,
                )
            };
            rt.duration_timer = Some(dur_timer);
        }
    }

    // Perform initial tick & schedule coalesced wakeup timer.
    run_tick_and_schedule(&runtime, mtm);

    app.run();
    Ok(())
}
