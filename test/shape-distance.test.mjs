/** Curved node constraints use actual outline distances, including flat ellipses and circular corners. */
import test from 'node:test';
import assert from 'node:assert/strict';
import { clampToShape, shapeBoundaryPoint, shapeSignedDistance, signedDistance } from '../src/layout/geometry.js';

test('elliptical projection lands on the nearest outline with a unit outward normal', () => {
  const rect = { x: 100, y: 50, w: 1200, h: 120 };
  const corners = { rx: 600, ry: 60 };
  for (const point of [
    { x: 700, y: 110 }, { x: 103, y: 53 }, { x: 1190, y: 90 },
    { x: 950, y: 200 }, { x: 650, y: 115 }, { x: 1300, y: 110 },
    { x: 1200, y: 110 }, { x: 1280, y: 110 },
  ]) {
    const edge = shapeBoundaryPoint(rect, point, corners);
    const u = (edge.x - 700) / 600;
    const v = (edge.y - 110) / 60;
    assert.ok(Math.abs(u * u + v * v - 1) < 1e-8);
    assert.ok(Math.abs(Math.hypot(edge.nx, edge.ny) - 1) < 1e-8);
    const distance = Math.hypot(point.x - edge.x, point.y - edge.y);
    let sampled = Infinity;
    for (let index = 0; index < 4096; index += 1) {
      const angle = index * Math.PI / 2048;
      const x = 700 + 600 * Math.cos(angle);
      const y = 110 + 60 * Math.sin(angle);
      sampled = Math.min(sampled, Math.hypot(point.x - x, point.y - y));
    }
    assert.ok(distance <= sampled + 1e-7, JSON.stringify(point));
  }
});

test('outline distance preserves rectangular and circular distances and the inside sign', () => {
  const rect = { x: 10, y: 20, w: 300, h: 300 };
  for (const point of [{ x: 160, y: 170 }, { x: 320, y: 170 }, { x: 15, y: 25 }]) {
    assert.equal(shapeSignedDistance(rect, point, { rx: 0, ry: 0 }), signedDistance(rect, point));
    const expected = Math.hypot(point.x - 160, point.y - 170) - 150;
    assert.ok(Math.abs(shapeSignedDistance(rect, point, { rx: 150, ry: 150 }) - expected) < 1e-8);
  }
});

test('elliptical corners find the nearest arc beside a straight edge', () => {
  for (const vertical of [false, true]) {
    const rect = { x: 0, y: 0, w: 1200, h: 140 };
    const corners = { rx: 600, ry: 60 };
    const point = { x: 1100, y: 79 };
    if (vertical) {
      rect.w = 140;
      rect.h = 1200;
      corners.rx = 60;
      corners.ry = 600;
      point.x = 79;
      point.y = 1100;
    }
    const edge = shapeBoundaryPoint(rect, point, corners);
    const distance = Math.hypot(point.x - edge.x, point.y - edge.y);
    for (let index = 0; index <= 4096; index += 1) {
      const angle = index * Math.PI / 8192;
      const x = rect.w - corners.rx + corners.rx * Math.cos(angle);
      const y = rect.h - corners.ry + corners.ry * Math.sin(angle);
      assert.ok(distance <= Math.hypot(point.x - x, point.y - y) + 1e-7);
    }
  }
});

test('clamping a circle into a flat ellipse preserves its entire clearance and is idempotent', () => {
  const rect = { x: 100, y: 50, w: 1200, h: 120 };
  const corners = { rx: 600, ry: 60 };
  for (const point of [
    { x: 101, y: 51 }, { x: 1299, y: 51 }, { x: 1299, y: 169 },
    { x: 700, y: 110 }, { x: 1280, y: 110 },
  ]) {
    const result = clampToShape(point, rect, 19, corners);
    assert.ok(shapeSignedDistance(rect, result, corners) <= -19 + 1e-7);
    for (let index = 0; index < 128; index += 1) {
      const angle = index * Math.PI / 64;
      const u = (result.x + 19 * Math.cos(angle) - 700) / 600;
      const v = (result.y + 19 * Math.sin(angle) - 110) / 60;
      assert.ok(u * u + v * v <= 1 + 1e-7);
    }
    const repeated = clampToShape(result, rect, 19, corners);
    assert.ok(Math.hypot(result.x - repeated.x, result.y - repeated.y) < 1e-8);
  }
});
