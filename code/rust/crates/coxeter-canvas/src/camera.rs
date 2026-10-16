//! The camera: `src/ui/camera-zoom.js`, and the camera half of `src/ui/app.js`
//! (`fitToStage`, `applyCamera`, `zoomBy`, `screenToMap`, `centerOnReaction`,
//! `centerOnPathway`, the stage-resize recentring of `syncStageViewport`).
//!
//! The map is drawn at canvas resolution and the camera is applied as
//! `translate(x, y) scale(scale)` about the stage's top-left corner. Points
//! given to these functions are client coordinates (egui points); the stage is
//! the widget's rectangle in the same coordinates.

use coxeter_core::js;
use coxeter_core::mapdata::{MapGraph, Point};
use coxeter_map::drag::{Camera as DragCamera, stage_to_map};

/// The smallest camera scale.
pub const MIN_ZOOM: f64 = 0.05;
/// The largest camera scale.
pub const MAX_ZOOM: f64 = 6.0;
/// `MIN_HIT_PX`: the radius of the smallest pointer target, in screen pixels
/// (a 24 px target, the WCAG 2.2 AA minimum).
pub const MIN_HIT_PX: f64 = 12.0;

/// The stage's box in client coordinates (`getBoundingClientRect()`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct StageRect {
    pub left: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
}

impl StageRect {
    pub fn new(left: f64, top: f64, width: f64, height: f64) -> StageRect {
        StageRect { left, top, width, height }
    }
}

/// `{ x, y, scale, fitted }`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    pub x: f64,
    pub y: f64,
    pub scale: f64,
    /// False until the map has been fitted to the stage once (or the user
    /// moved the camera); the first paint fits it.
    pub fitted: bool,
}

impl Default for Camera {
    fn default() -> Self {
        Camera { x: 0.0, y: 0.0, scale: 1.0, fitted: false }
    }
}

const fn clamp(value: f64, min: f64, max: f64) -> f64 {
    // `Math.min(max, Math.max(min, value))`; NaN never reaches it here.
    if value < min {
        min
    } else if value > max {
        max
    } else {
        value
    }
}

/// `Number.isFinite(camera.scale) && camera.scale > 0 ? camera.scale : 1`.
fn usable_scale(scale: f64) -> f64 {
    if scale.is_finite() && scale > 0.0 { scale } else { 1.0 }
}

/// `zoomCameraAt(camera, factor, stageRect, origin)`: zoom by `factor` about
/// `origin` (client coordinates), or about the stage's centre.
pub fn zoom_camera_at(camera: &mut Camera, factor: f64, stage: StageRect, origin: Option<Point>) {
    if !factor.is_finite() || factor <= 0.0 {
        return;
    }
    let cx = origin.map_or(stage.width / 2.0, |o| o.x - stage.left);
    let cy = origin.map_or(stage.height / 2.0, |o| o.y - stage.top);
    let current = usable_scale(camera.scale);
    let next = clamp(current * factor, MIN_ZOOM, MAX_ZOOM);
    let k = next / current;
    camera.x = cx - (cx - camera.x) * k;
    camera.y = cy - (cy - camera.y) * k;
    camera.scale = next;
    camera.fitted = true;
}

/// What `beginPinch` remembers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PinchGesture {
    pub start_distance: f64,
    pub start_scale: f64,
    pub anchor_map_x: f64,
    pub anchor_map_y: f64,
}

/// `pointDistance(a, b)`.
pub fn point_distance(a: Point, b: Point) -> f64 {
    js::hypot(b.x - a.x, b.y - a.y)
}

fn local_midpoint(a: Point, b: Point, stage: StageRect) -> Point {
    Point::new((a.x + b.x) / 2.0 - stage.left, (a.y + b.y) / 2.0 - stage.top)
}

/// `beginPinch(camera, first, second, stageRect)`.
pub fn begin_pinch(camera: &Camera, first: Point, second: Point, stage: StageRect) -> Option<PinchGesture> {
    let distance = point_distance(first, second);
    if !(distance > 0.0) {
        return None;
    }
    let midpoint = local_midpoint(first, second, stage);
    let scale = usable_scale(camera.scale);
    Some(PinchGesture {
        start_distance: distance,
        start_scale: scale,
        anchor_map_x: (midpoint.x - camera.x) / scale,
        anchor_map_y: (midpoint.y - camera.y) / scale,
    })
}

/// `updatePinch(camera, gesture, first, second, stageRect)`. Keeps the map point
/// that was first between the fingers under their midpoint, so moving both
/// fingers pans while pinching.
pub fn update_pinch(camera: &mut Camera, gesture: &PinchGesture, first: Point, second: Point, stage: StageRect) {
    let distance = point_distance(first, second);
    if !(distance > 0.0) || !(gesture.start_distance > 0.0) {
        return;
    }
    let midpoint = local_midpoint(first, second, stage);
    let next = clamp(gesture.start_scale * distance / gesture.start_distance, MIN_ZOOM, MAX_ZOOM);
    camera.scale = next;
    camera.x = midpoint.x - gesture.anchor_map_x * next;
    camera.y = midpoint.y - gesture.anchor_map_y * next;
    camera.fitted = true;
}

/// `fitToStage(svgNode)`: the whole canvas, centred, 20 px from the stage's edges.
pub fn fit_to_stage(camera: &mut Camera, map_width: f64, map_height: f64, stage: StageRect) {
    let pad = 20.0;
    let scale = js::min((stage.width - pad * 2.0) / map_width, (stage.height - pad * 2.0) / map_height);
    camera.scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    camera.x = (stage.width - map_width * camera.scale) / 2.0;
    camera.y = (stage.height - map_height * camera.scale) / 2.0;
    camera.fitted = true;
}

/// `screenToMap(clientX, clientY)`: through `stageToMap` of `layout/drag.js`.
pub fn screen_to_map(camera: &Camera, stage: StageRect, client: Point) -> Point {
    stage_to_map(client, DragCamera { x: camera.x, y: camera.y, scale: camera.scale }, stage.left, stage.top)
}

/// The inverse: where a map point is drawn, in client coordinates.
pub fn map_to_screen(camera: &Camera, stage: StageRect, p: Point) -> Point {
    Point::new(stage.left + camera.x + p.x * camera.scale, stage.top + camera.y + p.y * camera.scale)
}

/// `centerOnReaction(reactionId)`.
pub fn center_on_reaction(camera: &mut Camera, map: &MapGraph, reaction_id: &str, stage: StageRect) -> bool {
    let Some(rxn) = map.reactions.get(reaction_id) else { return false };
    camera.x = stage.width / 2.0 - rxn.x * camera.scale;
    camera.y = stage.height / 2.0 - rxn.y * camera.scale;
    camera.fitted = true;
    true
}

/// `centerOnPathway(pathwayKey)`, given the pathway's reactions.
pub fn center_on_reactions(camera: &mut Camera, map: &MapGraph, reaction_ids: &[String], stage: StageRect) -> bool {
    let points: Vec<Point> = reaction_ids.iter().filter_map(|rid| map.reactions.get(rid)).map(|r| Point::new(r.x, r.y)).collect();
    if points.is_empty() {
        return false;
    }
    let min_x = js::min_n(points.iter().map(|p| p.x));
    let max_x = js::max_n(points.iter().map(|p| p.x));
    let min_y = js::min_n(points.iter().map(|p| p.y));
    let max_y = js::max_n(points.iter().map(|p| p.y));
    let pad = 90.0;
    let width = js::max(120.0, max_x - min_x + pad * 2.0);
    let height = js::max(120.0, max_y - min_y + pad * 2.0);
    let next = js::min(2.4, js::max(0.08, js::min(stage.width / width, stage.height / height)));
    let cx = (min_x + max_x) / 2.0;
    let cy = (min_y + max_y) / 2.0;
    camera.scale = next;
    camera.x = stage.width / 2.0 - cx * camera.scale;
    camera.y = stage.height / 2.0 - cy * camera.scale;
    camera.fitted = true;
    true
}

/// `syncStageViewport()`: when the stage changes size, a fitted camera keeps the
/// middle of the view where it was.
pub fn sync_stage_viewport(camera: &mut Camera, previous: Option<(f64, f64)>, width: f64, height: f64) -> bool {
    if width <= 0.0 || height <= 0.0 {
        return false;
    }
    match previous {
        Some((pw, ph)) if camera.fitted && (pw != width || ph != height) => {
            camera.x += (width - pw) / 2.0;
            camera.y += (height - ph) / 2.0;
            true
        }
        _ => false,
    }
}

/// What `applyCamera()` hands the stylesheet and the pointer targets for a zoom.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ZoomVars {
    /// `--label-scale`: `max(1, min(2.6, 0.95 / scale)).toFixed(3)`.
    pub label_scale: f64,
    /// `--frame-label-scale`: `frameLabelScale(MIN_HIT_PX / scale).toFixed(3)`.
    pub frame_label_scale: f64,
    /// `is-far` on the map: `scale < 0.34`.
    pub is_far: bool,
}

impl ZoomVars {
    /// The values `applyCamera()` sets at this zoom (as the browser reads the
    /// strings it is given: three decimals).
    pub fn at(scale: f64) -> ZoomVars {
        let inverse = js::max(1.0, js::min(2.6, 0.95 / scale));
        let label_scale = js::string_to_number(&js::to_fixed(inverse, 3));
        let frame = coxeter_render::frame_label_scale(MIN_HIT_PX / scale);
        let frame_label_scale = js::string_to_number(&js::to_fixed(frame, 3));
        ZoomVars { label_scale, frame_label_scale, is_far: scale < 0.34 }
    }

    /// The stylesheet's own defaults (`--label-scale: 1`, `--frame-label-scale: 1`).
    pub fn base() -> ZoomVars {
        ZoomVars { label_scale: 1.0, frame_label_scale: 1.0, is_far: false }
    }
}

/// `MIN_HIT_PX / (camera.scale > 0 ? camera.scale : 1)`: the smallest pointer
/// target in map units, which `mapRenderState()` hands the renderer.
pub fn min_hit_radius(scale: f64) -> f64 {
    MIN_HIT_PX / if scale > 0.0 { scale } else { 1.0 }
}

/// The zoom readout: `${Math.round(camera.scale * 100)}%`.
pub fn zoom_readout(scale: f64) -> String {
    format!("{}%", js::number_to_string(js::round(scale * 100.0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readout_and_vars() {
        assert_eq!(zoom_readout(0.284), "28%");
        let v = ZoomVars::at(0.2);
        assert_eq!(v.label_scale, 2.6);
        assert!(v.is_far);
        assert_eq!(v.frame_label_scale, 5.0);
        let v = ZoomVars::at(2.0);
        assert_eq!((v.label_scale, v.frame_label_scale, v.is_far), (1.0, 1.0, false));
        assert_eq!(ZoomVars::at(0.7).label_scale, 1.357);
    }
}
