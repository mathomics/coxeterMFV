import test from 'node:test';
import assert from 'node:assert/strict';
import { createWheelNavigation, wheelPanDelta } from '../src/ui/wheel-navigation.js';

const wheel = (deltaY, extra = {}) => ({ deltaX: 0, deltaY, deltaMode: 0, ...extra });

test('automatic navigation preserves conventional mouse notches and line/page wheels', () => {
  for (const event of [wheel(100), wheel(-100), wheel(120), wheel(-240), wheel(300),
    wheel(3, { deltaMode: 1 }), wheel(-1, { deltaMode: 2 }),
    wheel(60, { wheelDeltaY: -120 }), wheel(-60, { wheelDelta: 120 })]) {
    assert.equal(createWheelNavigation().classify(event), 'zoom', JSON.stringify(event));
  }
});

test('automatic navigation pans small, fractional, horizontal and diagonal pixel scrolling', () => {
  for (const event of [wheel(12), wheel(-24), wheel(40), wheel(0.75), wheel(75.5),
    wheel(0, { deltaX: -18 }), wheel(10, { deltaX: 6 }), wheel(84, { momentum: true })]) {
    assert.equal(createWheelNavigation().classify(event), 'pan', JSON.stringify(event));
  }
});

test('continuous pixel scrolling takes precedence over ambiguous Chromium legacy notches', () => {
  for (const event of [wheel(8.5, { wheelDeltaY: -120 }),
    wheel(-12, { wheelDeltaY: 120 }), wheel(75.5, { wheelDelta: -120 })]) {
    assert.equal(createWheelNavigation().classify(event), 'pan', JSON.stringify(event));
    assert.equal(createWheelNavigation().classify(event, 'mouse'), 'zoom', JSON.stringify(event));
  }
});

test('a pan gesture continues as it accelerates, then expires after inactivity', () => {
  let time = 0;
  const navigation = createWheelNavigation({ now: () => time });
  assert.equal(navigation.classify(wheel(8)), 'pan');
  time += 30;
  assert.equal(navigation.classify(wheel(80)), 'pan');
  time += 150;
  assert.equal(navigation.classify(wheel(160)), 'pan');
  time += 201;
  assert.equal(navigation.classify(wheel(160)), 'zoom');
});

test('recognisable mouse input can follow touchpad input immediately', () => {
  const navigation = createWheelNavigation({ now: () => 0 });
  assert.equal(navigation.classify(wheel(12)), 'pan');
  assert.equal(navigation.classify(wheel(100)), 'zoom');
  assert.equal(navigation.classify(wheel(12)), 'pan');
  assert.equal(navigation.classify(wheel(3, { deltaMode: 1 })), 'zoom');
  assert.equal(navigation.classify(wheel(12)), 'pan');
  assert.equal(navigation.classify(wheel(60, { wheelDeltaY: -120 })), 'zoom');
});

test('Ctrl+wheel and pinch retain zoom while pure horizontal input cannot change scale', () => {
  for (const mode of ['auto', 'mouse', 'touchpad']) {
    const navigation = createWheelNavigation();
    assert.equal(navigation.classify(wheel(-2.5, { ctrlKey: true }), mode), 'zoom');
    assert.equal(navigation.classify(wheel(0, { deltaX: 10, ctrlKey: true }), mode), null);
  }
});

test('explicit device preferences handle otherwise indistinguishable event streams', () => {
  const navigation = createWheelNavigation();
  assert.equal(navigation.classify(wheel(8), 'mouse'), 'zoom');
  assert.equal(navigation.classify(wheel(120), 'touchpad'), 'pan');
  assert.equal(navigation.classify(wheel(3, { deltaMode: 1 }), 'touchpad'), 'pan');
  assert.equal(navigation.classify(wheel(0, { deltaX: 18 }), 'mouse'), null);
});

test('changing preferences or resetting does not carry a prior pan transaction', () => {
  const navigation = createWheelNavigation({ now: () => 0 });
  assert.equal(navigation.classify(wheel(12)), 'pan');
  assert.equal(navigation.classify(wheel(80), 'mouse'), 'zoom');
  assert.equal(navigation.classify(wheel(80), 'auto'), 'zoom');
  assert.equal(navigation.classify(wheel(12)), 'pan');
  navigation.reset();
  assert.equal(navigation.classify(wheel(80)), 'zoom');
});

test('zero, missing, tiny and invalid values are neutral without extending a transaction', () => {
  let time = 0;
  const navigation = createWheelNavigation({ now: () => time });
  assert.equal(navigation.classify(wheel(12)), 'pan');
  time = 150;
  for (const event of [null, {}, wheel(0), wheel(1e-8), wheel(NaN), wheel(Infinity)]) {
    assert.equal(navigation.classify(event), null);
  }
  time = 210;
  assert.equal(navigation.classify(wheel(80)), 'zoom');
});

test('pixel pan deltas retain both axes and line/page overrides use screen units', () => {
  assert.deepEqual(wheelPanDelta(wheel(-7.5, { deltaX: 12.25 })), { x: 12.25, y: -7.5 });
  assert.deepEqual(wheelPanDelta(wheel(2, { deltaX: -3, deltaMode: 1 })), { x: -48, y: 32 });
  assert.deepEqual(wheelPanDelta(wheel(-1, { deltaX: 2, deltaMode: 2 }),
    { pageWidth: 900, pageHeight: 650 }), { x: 1800, y: -650 });
});

test('pan deltas never introduce non-finite camera coordinates', () => {
  for (const event of [null, {}, wheel(NaN, { deltaX: Infinity }),
    wheel(Number.MAX_VALUE, { deltaX: Number.MAX_VALUE, deltaMode: 1 })]) {
    const delta = wheelPanDelta(event);
    assert.equal(Number.isFinite(delta.x), true);
    assert.equal(Number.isFinite(delta.y), true);
  }
  assert.deepEqual(wheelPanDelta(wheel(2, { deltaMode: 1 }), { lineHeight: NaN }), { x: 0, y: 0 });
});
