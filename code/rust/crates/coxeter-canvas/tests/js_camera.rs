//! `test/camera-zoom.test.mjs` and `test/wheel-navigation.test.mjs`, ported.

use coxeter_canvas::camera::{Camera, MAX_ZOOM, MIN_ZOOM, StageRect, begin_pinch, update_pinch, zoom_camera_at};
use coxeter_canvas::host::ScrollInput;
use coxeter_canvas::wheel::{DeltaMode, WheelAction, WheelInput, WheelNavigation, wheel_pan_delta};
use coxeter_core::mapdata::Point;

const RECT: StageRect = StageRect { left: 100.0, top: 50.0, width: 800.0, height: 600.0 };

fn cam(x: f64, y: f64, scale: f64, fitted: bool) -> Camera {
    Camera { x, y, scale, fitted }
}

#[test]
fn button_and_wheel_zoom_preserve_the_map_point_under_the_origin() {
    let mut camera = cam(20.0, 30.0, 2.0, false);
    let origin = Point::new(500.0, 350.0);
    let (lx, ly) = (origin.x - RECT.left, origin.y - RECT.top);
    let before = ((lx - camera.x) / camera.scale, (ly - camera.y) / camera.scale);
    zoom_camera_at(&mut camera, 1.5, RECT, Some(origin));
    let after = ((lx - camera.x) / camera.scale, (ly - camera.y) / camera.scale);
    assert!((after.0 - before.0).abs() < 1e-9 && (after.1 - before.1).abs() < 1e-9);
    assert_eq!(camera.scale, 3.0);
    assert!(camera.fitted);
}

#[test]
fn separating_two_fingers_zooms_in_around_their_midpoint() {
    let mut camera = cam(0.0, 0.0, 1.0, true);
    let gesture = begin_pinch(&camera, Point::new(300.0, 300.0), Point::new(500.0, 300.0), RECT).expect("a pinch");
    update_pinch(&mut camera, &gesture, Point::new(200.0, 300.0), Point::new(600.0, 300.0), RECT);
    assert_eq!(camera.scale, 2.0);
    // The first midpoint, client (400, 300), is stage-local (300, 250): that
    // map point stays under the new midpoint.
    assert!(((300.0 - camera.x) / camera.scale - gesture.anchor_map_x).abs() < 1e-9);
    assert!(((250.0 - camera.y) / camera.scale - gesture.anchor_map_y).abs() < 1e-9);
}

#[test]
fn bringing_two_fingers_together_zooms_out() {
    let mut camera = cam(0.0, 0.0, 2.0, true);
    let gesture = begin_pinch(&camera, Point::new(200.0, 250.0), Point::new(600.0, 250.0), RECT).unwrap();
    update_pinch(&mut camera, &gesture, Point::new(300.0, 250.0), Point::new(500.0, 250.0), RECT);
    assert_eq!(camera.scale, 1.0);
}

#[test]
fn moving_the_pinch_midpoint_pans_while_preserving_the_anchored_map_point() {
    let mut camera = cam(10.0, 20.0, 1.0, true);
    let gesture = begin_pinch(&camera, Point::new(300.0, 200.0), Point::new(500.0, 200.0), RECT).unwrap();
    update_pinch(&mut camera, &gesture, Point::new(350.0, 240.0), Point::new(550.0, 240.0), RECT);
    let mid = (350.0, 190.0);
    assert!(((mid.0 - camera.x) / camera.scale - gesture.anchor_map_x).abs() < 1e-9);
    assert!(((mid.1 - camera.y) / camera.scale - gesture.anchor_map_y).abs() < 1e-9);
}

#[test]
fn zoom_is_clamped_to_safe_limits() {
    let mut camera = cam(0.0, 0.0, 1.0, true);
    zoom_camera_at(&mut camera, 1000.0, RECT, None);
    assert_eq!(camera.scale, MAX_ZOOM);
    zoom_camera_at(&mut camera, 0.000001, RECT, None);
    assert_eq!(camera.scale, MIN_ZOOM);
}

// wheel-navigation

fn wheel(delta_y: f64) -> WheelInput {
    WheelInput { delta_y, ..Default::default() }
}

fn classify(e: WheelInput) -> Option<WheelAction> {
    WheelNavigation::new().classify(Some(&e), ScrollInput::Auto, 0.0)
}

const ZOOM: Option<WheelAction> = Some(WheelAction::Zoom);
const PAN: Option<WheelAction> = Some(WheelAction::Pan);

#[test]
fn automatic_navigation_preserves_mouse_notches_and_line_page_wheels() {
    for e in [
        wheel(100.0),
        wheel(-100.0),
        wheel(120.0),
        wheel(-240.0),
        wheel(300.0),
        WheelInput { delta_mode: DeltaMode::Line, ..wheel(3.0) },
        WheelInput { delta_mode: DeltaMode::Page, ..wheel(-1.0) },
        WheelInput { legacy_delta_y: -120.0, ..wheel(60.0) },
        WheelInput { legacy_delta_y: 120.0, ..wheel(-60.0) },
    ] {
        assert_eq!(classify(e), ZOOM, "{e:?}");
    }
}

#[test]
fn automatic_navigation_pans_small_fractional_horizontal_and_diagonal_scrolling() {
    for e in [
        wheel(12.0),
        wheel(-24.0),
        wheel(40.0),
        wheel(0.75),
        wheel(75.5),
        WheelInput { delta_x: -18.0, ..wheel(0.0) },
        WheelInput { delta_x: 6.0, ..wheel(10.0) },
        WheelInput { momentum: true, ..wheel(84.0) },
    ] {
        assert_eq!(classify(e), PAN, "{e:?}");
    }
}

#[test]
fn continuous_pixel_scrolling_beats_ambiguous_legacy_notches() {
    for e in [
        WheelInput { legacy_delta_y: -120.0, ..wheel(8.5) },
        WheelInput { legacy_delta_y: 120.0, ..wheel(-12.0) },
        WheelInput { legacy_delta_y: -120.0, ..wheel(75.5) },
    ] {
        assert_eq!(classify(e), PAN, "{e:?}");
        assert_eq!(WheelNavigation::new().classify(Some(&e), ScrollInput::Mouse, 0.0), ZOOM, "{e:?}");
    }
}

#[test]
fn a_pan_gesture_continues_as_it_accelerates_then_expires() {
    let mut nav = WheelNavigation::new();
    let mut time = 0.0;
    assert_eq!(nav.classify(Some(&wheel(8.0)), ScrollInput::Auto, time), PAN);
    time += 30.0;
    assert_eq!(nav.classify(Some(&wheel(80.0)), ScrollInput::Auto, time), PAN);
    time += 150.0;
    assert_eq!(nav.classify(Some(&wheel(160.0)), ScrollInput::Auto, time), PAN);
    time += 201.0;
    assert_eq!(nav.classify(Some(&wheel(160.0)), ScrollInput::Auto, time), ZOOM);
}

#[test]
fn recognisable_mouse_input_can_follow_touchpad_input_immediately() {
    let mut nav = WheelNavigation::new();
    let mut c = |e: WheelInput| nav.classify(Some(&e), ScrollInput::Auto, 0.0);
    assert_eq!(c(wheel(12.0)), PAN);
    assert_eq!(c(wheel(100.0)), ZOOM);
    assert_eq!(c(wheel(12.0)), PAN);
    assert_eq!(c(WheelInput { delta_mode: DeltaMode::Line, ..wheel(3.0) }), ZOOM);
    assert_eq!(c(wheel(12.0)), PAN);
    assert_eq!(c(WheelInput { legacy_delta_y: -120.0, ..wheel(60.0) }), ZOOM);
}

#[test]
fn ctrl_wheel_and_pinch_keep_zoom_while_horizontal_input_cannot_change_scale() {
    for mode in [ScrollInput::Auto, ScrollInput::Mouse, ScrollInput::Touchpad] {
        let mut nav = WheelNavigation::new();
        assert_eq!(nav.classify(Some(&WheelInput { ctrl_key: true, ..wheel(-2.5) }), mode, 0.0), ZOOM);
        assert_eq!(nav.classify(Some(&WheelInput { ctrl_key: true, delta_x: 10.0, ..wheel(0.0) }), mode, 0.0), None);
    }
}

#[test]
fn explicit_device_preferences_handle_indistinguishable_streams() {
    let mut nav = WheelNavigation::new();
    assert_eq!(nav.classify(Some(&wheel(8.0)), ScrollInput::Mouse, 0.0), ZOOM);
    assert_eq!(nav.classify(Some(&wheel(120.0)), ScrollInput::Touchpad, 0.0), PAN);
    assert_eq!(nav.classify(Some(&WheelInput { delta_mode: DeltaMode::Line, ..wheel(3.0) }), ScrollInput::Touchpad, 0.0), PAN);
    assert_eq!(nav.classify(Some(&WheelInput { delta_x: 18.0, ..wheel(0.0) }), ScrollInput::Mouse, 0.0), None);
}

#[test]
fn changing_preferences_or_resetting_drops_a_prior_pan() {
    let mut nav = WheelNavigation::new();
    assert_eq!(nav.classify(Some(&wheel(12.0)), ScrollInput::Auto, 0.0), PAN);
    assert_eq!(nav.classify(Some(&wheel(80.0)), ScrollInput::Mouse, 0.0), ZOOM);
    assert_eq!(nav.classify(Some(&wheel(80.0)), ScrollInput::Auto, 0.0), ZOOM);
    assert_eq!(nav.classify(Some(&wheel(12.0)), ScrollInput::Auto, 0.0), PAN);
    nav.reset();
    assert_eq!(nav.classify(Some(&wheel(80.0)), ScrollInput::Auto, 0.0), ZOOM);
}

#[test]
fn neutral_values_do_not_extend_a_transaction() {
    let mut nav = WheelNavigation::new();
    assert_eq!(nav.classify(Some(&wheel(12.0)), ScrollInput::Auto, 0.0), PAN);
    for e in [None, Some(WheelInput::default()), Some(wheel(0.0)), Some(wheel(1e-8)), Some(wheel(f64::NAN)), Some(wheel(f64::INFINITY))] {
        assert_eq!(nav.classify(e.as_ref(), ScrollInput::Auto, 150.0), None);
    }
    assert_eq!(nav.classify(Some(&wheel(80.0)), ScrollInput::Auto, 210.0), ZOOM);
}

#[test]
fn pixel_pan_deltas_keep_both_axes_and_lines_pages_use_screen_units() {
    assert_eq!(wheel_pan_delta(Some(&WheelInput { delta_x: 12.25, ..wheel(-7.5) }), 16.0, 800.0, 800.0), (12.25, -7.5));
    assert_eq!(
        wheel_pan_delta(Some(&WheelInput { delta_x: -3.0, delta_mode: DeltaMode::Line, ..wheel(2.0) }), 16.0, 800.0, 800.0),
        (-48.0, 32.0)
    );
    assert_eq!(
        wheel_pan_delta(Some(&WheelInput { delta_x: 2.0, delta_mode: DeltaMode::Page, ..wheel(-1.0) }), 16.0, 900.0, 650.0),
        (1800.0, -650.0)
    );
}

#[test]
fn pan_deltas_never_introduce_non_finite_camera_coordinates() {
    for e in [
        None,
        Some(WheelInput::default()),
        Some(WheelInput { delta_x: f64::INFINITY, ..wheel(f64::NAN) }),
        Some(WheelInput { delta_x: f64::MAX, delta_mode: DeltaMode::Line, ..wheel(f64::MAX) }),
    ] {
        let (x, y) = wheel_pan_delta(e.as_ref(), 16.0, 800.0, 800.0);
        assert!(x.is_finite() && y.is_finite());
    }
    assert_eq!(wheel_pan_delta(Some(&WheelInput { delta_mode: DeltaMode::Line, ..wheel(2.0) }), f64::NAN, 800.0, 800.0), (0.0, 0.0));
}
