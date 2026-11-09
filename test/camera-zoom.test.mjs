import test from 'node:test';
import assert from 'node:assert/strict';
import {
  beginPinch,
  updatePinch,
  zoomCameraAt,
  MIN_ZOOM,
  MAX_ZOOM,
} from '../src/ui/camera-zoom.js';

const rect = { left: 100, top: 50, width: 800, height: 600 };

test('button/wheel zoom preserves the map point under the zoom origin', () => {
  const camera = { x: 20, y: 30, scale: 2, fitted: false };
  const origin = { x: 500, y: 350 };
  const localX = origin.x - rect.left;
  const localY = origin.y - rect.top;
  const before = {
    x: (localX - camera.x) / camera.scale,
    y: (localY - camera.y) / camera.scale,
  };

  zoomCameraAt(camera, 1.5, rect, origin);

  const after = {
    x: (localX - camera.x) / camera.scale,
    y: (localY - camera.y) / camera.scale,
  };
  assert.ok(Math.abs(after.x - before.x) < 1e-9);
  assert.ok(Math.abs(after.y - before.y) < 1e-9);
  assert.equal(camera.scale, 3);
  assert.equal(camera.fitted, true);
});

test('separating two fingers zooms in around their midpoint', () => {
  const camera = { x: 0, y: 0, scale: 1, fitted: true };
  const first = { x: 300, y: 300 };
  const second = { x: 500, y: 300 };
  const gesture = beginPinch(camera, first, second, rect);
  assert.ok(gesture);

  updatePinch(camera, gesture, { x: 200, y: 300 }, { x: 600, y: 300 }, rect);
  assert.equal(camera.scale, 2);

  // Initial midpoint was client (400,300) => stage-local (300,250).
  // That exact map point remains below the new midpoint.
  assert.ok(Math.abs((300 - camera.x) / camera.scale - gesture.anchorMapX) < 1e-9);
  assert.ok(Math.abs((250 - camera.y) / camera.scale - gesture.anchorMapY) < 1e-9);
});

test('bringing two fingers together zooms out', () => {
  const camera = { x: 0, y: 0, scale: 2, fitted: true };
  const gesture = beginPinch(
    camera,
    { x: 200, y: 250 },
    { x: 600, y: 250 },
    rect,
  );
  updatePinch(
    camera,
    gesture,
    { x: 300, y: 250 },
    { x: 500, y: 250 },
    rect,
  );
  assert.equal(camera.scale, 1);
});

test('moving the pinch midpoint pans while preserving the anchored map point', () => {
  const camera = { x: 10, y: 20, scale: 1, fitted: true };
  const gesture = beginPinch(camera, { x: 300, y: 200 }, { x: 500, y: 200 }, rect);

  updatePinch(camera, gesture, { x: 350, y: 240 }, { x: 550, y: 240 }, rect);

  const midpointLocal = { x: 350, y: 190 };
  assert.ok(Math.abs((midpointLocal.x - camera.x) / camera.scale - gesture.anchorMapX) < 1e-9);
  assert.ok(Math.abs((midpointLocal.y - camera.y) / camera.scale - gesture.anchorMapY) < 1e-9);
});

test('zoom is clamped to safe limits', () => {
  const camera = { x: 0, y: 0, scale: 1, fitted: true };
  zoomCameraAt(camera, 1000, rect);
  assert.equal(camera.scale, MAX_ZOOM);
  zoomCameraAt(camera, 0.000001, rect);
  assert.equal(camera.scale, MIN_ZOOM);
});
