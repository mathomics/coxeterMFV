/**
 * Camera zoom helpers shared by mouse/wheel/buttons and touch pinch gestures.
 * All points supplied here are browser client coordinates.
 */

export const MIN_ZOOM = 0.05;
export const MAX_ZOOM = 6;

/**
 * Multiply the camera scale by `factor`, clamped to MIN_ZOOM..MAX_ZOOM, keeping
 * the map point under `origin` (client coordinates) fixed on screen. Without an
 * origin the stage centre is used. Mutates and returns `camera`.
 */
export function zoomCameraAt(camera, factor, stageRect, origin = null) {
  if (!camera || !stageRect || !Number.isFinite(factor) || factor <= 0) return camera;

  const cx = origin ? origin.x - stageRect.left : stageRect.width / 2;
  const cy = origin ? origin.y - stageRect.top : stageRect.height / 2;
  const current = Number.isFinite(camera.scale) && camera.scale > 0 ? camera.scale : 1;
  const next = clamp(current * factor, MIN_ZOOM, MAX_ZOOM);
  const k = next / current;

  camera.x = cx - (cx - camera.x) * k;
  camera.y = cy - (cy - camera.y) * k;
  camera.scale = next;
  camera.fitted = true;
  return camera;
}

/**
 * Start a two-finger pinch from two client points. Returns the gesture state
 * that updatePinch needs (start distance, start scale and the map point under
 * the midpoint), or null when the fingers coincide.
 */
export function beginPinch(camera, first, second, stageRect) {
  const distance = pointDistance(first, second);
  if (!camera || !stageRect || distance <= 0) return null;

  const midpoint = localMidpoint(first, second, stageRect);
  const scale = Number.isFinite(camera.scale) && camera.scale > 0 ? camera.scale : 1;

  return {
    startDistance: distance,
    startScale: scale,
    anchorMapX: (midpoint.x - camera.x) / scale,
    anchorMapY: (midpoint.y - camera.y) / scale,
  };
}

/** Apply the current finger positions of a pinch begun by beginPinch. Mutates and returns `camera`. */
export function updatePinch(camera, gesture, first, second, stageRect) {
  if (!camera || !gesture || !stageRect) return camera;

  const distance = pointDistance(first, second);
  if (!(distance > 0) || !(gesture.startDistance > 0)) return camera;

  const midpoint = localMidpoint(first, second, stageRect);
  const next = clamp(
    gesture.startScale * distance / gesture.startDistance,
    MIN_ZOOM,
    MAX_ZOOM,
  );

  // Keep the map point that was initially between the fingers under the
  // current midpoint. Moving both fingers therefore pans while pinching.
  camera.scale = next;
  camera.x = midpoint.x - gesture.anchorMapX * next;
  camera.y = midpoint.y - gesture.anchorMapY * next;
  camera.fitted = true;
  return camera;
}

/** Euclidean distance between two {x, y} points, missing coordinates read as 0. */
export function pointDistance(a, b) {
  return Math.hypot(Number(b?.x ?? 0) - Number(a?.x ?? 0), Number(b?.y ?? 0) - Number(a?.y ?? 0));
}

function localMidpoint(a, b, stageRect) {
  return {
    x: (Number(a.x) + Number(b.x)) / 2 - stageRect.left,
    y: (Number(a.y) + Number(b.y)) / 2 - stageRect.top,
  };
}

const clamp = (value, min, max) => Math.min(max, Math.max(min, value));
