/** The Add Metabolite panel keeps its query and placement and closes correctly after a failed insertion. */
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { Model } from '../src/core/model.js';
import { CommunityModel } from '../src/core/community.js';
import { EditableMap } from '../src/map/editable-map.js';
import { Store } from '../src/ui/store.js';
import { MetaboliteAddMenuController } from '../src/ui/metabolite-add-menu.js';

async function makeMenu(width = 900, height = 500) {
  const raw = JSON.parse(await readFile(new URL('../models/ecoli_core_mc.json', import.meta.url), 'utf8'));
  const model = new Model(raw);
  const store = new Store('.');
  store.models.set('ecoli_core_mc', model);
  store.species = [{ id: 'eco', file: 'ecoli_core_mc', abundance: 1, enabled: true }];
  store.community = new CommunityModel({ species: [{ id: 'eco', model, abundance: 1 }], medium: {} });
  store.map = EditableMap.emptyFromCommunity(store.community);
  const listeners = {};
  const stage = {
    clientWidth: width, clientHeight: height,
    getBoundingClientRect: () => ({ left: 0, top: 0 }),
    addEventListener(type, listener) {
      listeners[type] = listener;
    },
  };
  let paints = 0;
  const controller = new MetaboliteAddMenuController({
    stage, store, screenToMap: (x, y) => ({ x, y }),
    onChange() {
      paints += 1;
    },
  });
  controller.open('eco', 'c', { clientX: 300, clientY: 200 });
  return { controller, store, listeners, paints: () => paints };
}

function filter(controller, value) {
  const input = {
    value,
    closest() {
      return { querySelectorAll: () => [] };
    },
  };
  controller.input({ target: { closest: () => input }, stopPropagation() {} });
}

test('the manual metabolite filter survives repaints and escapes its input value', async () => {
  const { controller } = await makeMenu();
  filter(controller, 'g6p');
  assert.match(controller.render(), /value="g6p"/);
  filter(controller, 'g6p" <&');
  assert.match(controller.render(), /value="g6p&quot; &lt;&amp;"/);
  controller.close();
  controller.open('eco', 'c', { clientX: 300, clientY: 200 });
  assert.doesNotMatch(controller.render(), /value="g6p/);
});

test('the manual metabolite header moves the panel without changing its insertion origin or query', async () => {
  const { controller, listeners } = await makeMenu();
  filter(controller, 'g6p');
  assert.equal(typeof listeners.pointerdown, 'function');
  const origin = { ...controller.state.origin };
  const moves = {};
  const menu = {
    offsetLeft: 100, offsetTop: 60, style: {},
    classList: { add() {}, remove() {} },
  };
  const head = {
    closest(selector) {
      if (selector === '.add-reaction-head') {
        return head;
      }
      if (selector === '.add-reaction-menu') {
        return menu;
      }
      return null;
    },
    setPointerCapture() {}, releasePointerCapture() {},
    addEventListener(type, listener) {
      moves[type] = listener;
    },
    removeEventListener(type) {
      delete moves[type];
    },
  };
  listeners.pointerdown({
    target: head, pointerType: 'mouse', button: 0, pointerId: 72,
    clientX: 300, clientY: 200, preventDefault() {},
  });
  moves.pointermove({ pointerId: 72, clientX: 420, clientY: 330 });
  moves.pointerup({ pointerId: 72 });
  assert.deepEqual(controller.state.origin, origin);
  assert.match(controller.render(), /left:220px;top:190px/);
  assert.match(controller.render(), /value="g6p"/);
  assert.equal(moves.pointermove, undefined);
});

test('a failed manual menu insertion closes and repaints the panel', async () => {
  const { controller, store, paints } = await makeMenu();
  store.addMapMetabolite = () => null;
  const before = paints();
  const button = { dataset: { addMetabolite: 'eco@g6p_c' } };
  controller.click({
    target: {
      closest(selector) {
        return selector === '[data-add-metabolite]' ? button : null;
      },
    },
    preventDefault() {}, stopPropagation() {},
  });
  assert.equal(controller.state, null);
  assert.equal(paints(), before + 1);
});

test('the manual menu declares a stage-sized scrolling viewport on a short stage', async () => {
  const { controller } = await makeMenu(400, 280);
  assert.match(controller.render(), /width:380px;max-height:264px/);
});
