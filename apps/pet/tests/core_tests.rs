use milkbird_pet::model::{EyeExpression, LAYER_ORDER, PetPose, PetRig};
use milkbird_pet::physics::{PetPhysics, PhysicsTargets, Spring1D};
use milkbird_pet::render::PetRenderer;
use milkbird_pet::scheduler::{FpsTier, FrameDecision, FrameScheduler};
use milkbird_pet::state::{BehaviorMode, PetController, PetEvent, PetState, ScreenEdge};
use std::mem::size_of;
use std::time::Instant;

#[test]
fn procedural_model_stays_under_32kb_budget() {
    let rig_bytes = size_of::<PetRig>();
    let pose_bytes = size_of::<PetPose>();
    assert!(rig_bytes < 1024, "PetRig too large: {rig_bytes}");
    assert!(pose_bytes < 512, "PetPose too large: {pose_bytes}");
    assert_eq!(LAYER_ORDER.len(), 12);
}

#[test]
fn spring_converges_cleanly_even_with_large_dt_spikes() {
    let mut s = Spring1D::new(0.0, 240.0, 14.0);
    s.impulse(5.0);
    // Simulate severe OS frame stall (250 ms) — must remain finite and stable.
    s.step(1.0, 0.25);
    assert!(s.value.is_finite() && s.velocity.is_finite());
    for _ in 0..180 {
        s.step(1.0, 1.0 / 60.0);
    }
    assert!(s.is_settled(1.0, 0.002, 0.01));
}

#[test]
fn physics_drag_drop_landing_triggers_volume_preserving_squash_and_settles() {
    let mut phys = PetPhysics::default();
    let mut drag_targets = PhysicsTargets {
        gravity_enabled: false,
        held_altitude: 16.0,
        ..PhysicsTargets::default()
    };
    phys.apply_drag_delta(35.0, -10.0);
    for _ in 0..10 {
        phys.step(&drag_targets, 1.0 / 60.0);
    }
    assert!(phys.tilt.value.abs() > 0.01);
    assert!(phys.ahoge.value.abs() > 0.01);
    assert!(phys.scarf_sway.value.abs() > 0.01);

    phys.release_drop(22.0);
    drag_targets.gravity_enabled = true;
    let mut landed = false;
    let mut min_scale_y = 1.0f32;
    for _ in 0..180 {
        if phys.step(&drag_targets, 1.0 / 60.0) {
            landed = true;
        }
        let (sx, sy) = phys.body_scales();
        // Check volume-preserving bounds.
        assert!((0.74..=1.30).contains(&sx));
        assert!((0.74..=1.28).contains(&sy));
        min_scale_y = min_scale_y.min(sy);
    }
    assert!(landed, "drop must land on ground");
    assert!(
        min_scale_y < 0.96,
        "landing should trigger visible squash (min_scale_y={min_scale_y})"
    );
    assert!(phys.is_settled(&drag_targets), "physics must settle cleanly");
}

#[test]
fn all_eight_states_and_seven_expressions_render_valid_pixels() {
    let mut renderer = PetRenderer::new(2.0);
    assert_eq!(renderer.pixel_size(), (320, 320));

    let states = [
        PetState::Idle,
        PetState::WalkHop,
        PetState::DragDangle,
        PetState::Petted,
        PetState::AlertMatch,
        PetState::Sleep,
        PetState::Perch,
        PetState::Suspended,
    ];
    for state in states {
        let mut ctrl = PetController::new(99);
        ctrl.handle_event(PetEvent::DemoState(state));
        let (pose, _) = ctrl.tick(1.0 / 60.0);
        let frame = renderer.render(&pose);
        let opaque_pixels = frame
            .data()
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|px| px[3] > 200)
            .count();
        assert!(
            opaque_pixels > 5_000,
            "state {state:?} rendered too few opaque pixels: {opaque_pixels}"
        );
    }

    let expressions = [
        EyeExpression::Normal,
        EyeExpression::Blink,
        EyeExpression::Happy,
        EyeExpression::Drowsy,
        EyeExpression::Surprised,
        EyeExpression::Focused,
        EyeExpression::Sleeping,
    ];
    for eyes in expressions {
        let pose = PetPose {
            eyes,
            ..PetPose::default()
        };
        let frame = renderer.render(&pose);
        assert_eq!(frame.width(), 320);
        assert!(renderer.hit_test(80.0, 96.0), "body center must hit-test true");
        assert!(!renderer.hit_test(4.0, 4.0), "transparent corner must pass through");
    }
}

#[test]
fn adaptive_scheduler_manages_fps_tiers_and_skips_duplicate_frames() {
    let mut ctrl = PetController::new(123);
    ctrl.set_sleep_timeout_secs(5.0);
    ctrl.handle_event(PetEvent::SetBehaviorMode(BehaviorMode::StayPut));

    // Settle after mode change -> IdleBreeze (15 Hz).
    for _ in 0..90 {
        ctrl.tick(1.0 / 60.0);
    }
    assert_eq!(ctrl.suggested_fps_tier(), FpsTier::IdleBreeze);
    assert_eq!(FpsTier::IdleBreeze.target_hz(), 15);

    // Click boosts instantaneously to 60 Hz!
    ctrl.handle_event(PetEvent::Click {
        local_x: 80.0,
        local_y: 80.0,
    });
    assert_eq!(ctrl.state(), PetState::Petted);
    assert_eq!(ctrl.suggested_fps_tier(), FpsTier::ActiveSmooth);
    assert_eq!(FpsTier::ActiveSmooth.target_hz(), 60);

    // Force sleep -> settles into SleepLow (<= 6 Hz).
    ctrl.handle_event(PetEvent::ForceSleep);
    for _ in 0..180 {
        ctrl.tick(1.0 / 60.0);
    }
    assert_eq!(ctrl.state(), PetState::Sleep);
    assert_eq!(ctrl.suggested_fps_tier(), FpsTier::SleepLow);
    assert!(FpsTier::SleepLow.target_hz() <= 6);

    // Suspended -> 0 Hz and no wakeup interval.
    ctrl.handle_event(PetEvent::SetSuspended(true));
    assert_eq!(ctrl.suggested_fps_tier(), FpsTier::Suspended);
    assert_eq!(FpsTier::Suspended.target_hz(), 0);
    assert_eq!(FpsTier::Suspended.interval(), None);

    // Dirty-frame skipping deduplicates unchanged poses.
    let mut sched = FrameScheduler::new();
    let pose = PetPose::default();
    assert!(matches!(
        sched.plan_frame(FpsTier::IdleBreeze, &pose),
        FrameDecision::Render { .. }
    ));
    assert!(matches!(
        sched.plan_frame(FpsTier::IdleBreeze, &pose),
        FrameDecision::Skip { .. }
    ));
    assert_eq!(sched.stats().rendered_frames, 1);
    assert_eq!(sched.stats().skipped_frames, 1);
}

#[test]
fn tray_unread_and_edge_perch_transitions_work_as_designed() {
    let mut ctrl = PetController::new(7);
    ctrl.handle_event(PetEvent::TrayStatus {
        unread_count: 2,
        active_demands: 1,
        urgent_pulse: true,
    });
    assert_eq!(ctrl.state(), PetState::AlertMatch);
    assert_eq!(ctrl.unread_count(), 2);

    // Clearing unread returns bird smoothly to Idle.
    ctrl.handle_event(PetEvent::TrayStatus {
        unread_count: 0,
        active_demands: 1,
        urgent_pulse: false,
    });
    assert_eq!(ctrl.state(), PetState::Idle);

    // Drag and release near right edge enters Perch facing left into screen.
    ctrl.handle_event(PetEvent::DragStart);
    assert_eq!(ctrl.state(), PetState::DragDangle);
    ctrl.handle_event(PetEvent::DragEnd {
        near_edge: Some(ScreenEdge::Right),
    });
    assert_eq!(ctrl.state(), PetState::Perch);
    for _ in 0..30 {
        ctrl.tick(1.0 / 60.0);
    }
    assert_eq!(ctrl.compose_pose().facing, -1.0);
}

#[test]
fn hit_test_is_sub_50us() {
    let mut renderer = PetRenderer::new(2.0);
    let _ = renderer.render(&PetPose::default());
    let start = Instant::now();
    for i in 0..10_000u32 {
        let _ = renderer.hit_test((i % 160) as f32, ((i / 160) % 160) as f32);
    }
    let avg_us = start.elapsed().as_secs_f64() * 1e6 / 10_000.0;
    assert!(avg_us < 50.0, "hit_test avg too slow: {avg_us:.3} us");
}
