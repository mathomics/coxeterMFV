/** Floating menu bounds and interrupted header gestures leave new menus and drawing coordinates alone. */
import test from 'node:test';
import assert from 'node:assert/strict';
import { beginPanelDrag, panelPlacement } from '../src/ui/add-reaction-panel.js';

function dragFixture() {
  const listeners = {};
  const menu = {
    offsetLeft: 100, offsetTop: 60, style: {}, isConnected: true,
    classList: { add() {}, remove() {} },
  };
  const head = {
    isConnected: true,
    closest(selector) {
      if (selector === '.add-reaction-head') {
        return head;
      }
      if (selector === '.add-reaction-menu') {
        return menu;
      }
      return null;
    },
    setPointerCapture() {},
    releasePointerCapture() {},
    addEventListener(type, listener) {
      listeners[type] = listener;
    },
    removeEventListener(type) {
      delete listeners[type];
    },
  };
  const controller = {
    stage: { clientWidth: 900, clientHeight: 500 },
    state: { left: 100, top: 60, origin: { x: 50, y: 80 }, filter: 'PFK' },
  };
  beginPanelDrag(controller, {
    target: head, pointerType: 'touch', button: 0, pointerId: 41,
    clientX: 300, clientY: 200, preventDefault() {},
  });
  return { controller, head, menu, listeners };
}

test('an unopened panel fits inside stages below the old width and height minima', () => {
  for (const [width, height] of [[210, 180], [180, 280], [420, 160]]) {
    const stage = { clientWidth: width, clientHeight: height };
    const place = panelPlacement(stage, { left: 500, top: 500 });
    assert.ok(place.left >= 0 && place.top >= 0);
    assert.ok(place.left + place.width <= width);
    assert.ok(place.top + place.maxHeight <= height);
  }
});

test('a dragged panel keeps its header reachable even on stages smaller than the header allowance', () => {
  const stage = { clientWidth: 50, clientHeight: 30 };
  for (const value of [-500, 500]) {
    const place = panelPlacement(stage, { left: value, top: value, moved: true });
    const visible = Math.min(stage.clientWidth, place.left + place.width) - Math.max(0, place.left);
    assert.ok(visible >= Math.min(64, stage.clientWidth, place.width));
    assert.equal(place.top, 0);
  }
});

test('an old header drag cannot change a newly opened menu', () => {
  const { controller, listeners } = dragFixture();
  controller.state = null;
  controller.state = { left: 250, top: 140, origin: { x: 90, y: 110 }, filter: 'PGI' };
  const before = structuredClone(controller.state);
  listeners.pointermove({ pointerId: 41, clientX: 400, clientY: 300 });
  assert.deepEqual(controller.state, before);
  assert.equal(listeners.pointermove, undefined);
});

test('another pointer cannot move the panel or finish its active header drag', () => {
  const { controller, head, listeners } = dragFixture();
  const before = structuredClone(controller.state);
  assert.equal(beginPanelDrag(controller, {
    target: head, pointerType: 'touch', button: 0, pointerId: 42,
    clientX: 500, clientY: 400, preventDefault() {},
  }), false);
  listeners.pointermove({ pointerId: 42, clientX: 500, clientY: 400 });
  assert.deepEqual(controller.state, before);
  listeners.pointerup({ pointerId: 42 });
  assert.equal(typeof listeners.pointermove, 'function');
  listeners.pointermove({ pointerId: 41, clientX: 400, clientY: 300 });
  assert.deepEqual([controller.state.left, controller.state.top], [200, 160]);
  listeners.pointerup({ pointerId: 41 });
  assert.equal(listeners.pointermove, undefined);
});

test('lost pointer capture ends a header drag and removes its listeners', () => {
  const { listeners } = dragFixture();
  assert.equal(typeof listeners.lostpointercapture, 'function');
  listeners.lostpointercapture({ pointerId: 41 });
  assert.deepEqual(Object.keys(listeners), []);
});

test('a detached header cannot keep updating its menu after a repaint', () => {
  const { controller, head, listeners } = dragFixture();
  const before = structuredClone(controller.state);
  head.isConnected = false;
  listeners.pointermove({ pointerId: 41, clientX: 400, clientY: 300 });
  assert.deepEqual(controller.state, before);
  assert.deepEqual(Object.keys(listeners), []);
});

test('pointer cancellation leaves the last position, origin and query intact and ends the drag', () => {
  const { controller, listeners } = dragFixture();
  listeners.pointermove({ pointerId: 41, clientX: 400, clientY: 300 });
  const before = structuredClone(controller.state);
  listeners.pointercancel({ pointerId: 41 });
  assert.deepEqual(controller.state, before);
  assert.deepEqual(Object.keys(listeners), []);
});
