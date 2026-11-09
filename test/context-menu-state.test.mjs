/** Contextual reaction filters survive selection and terminal changes, then reset when the panel is reopened. */
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { Model } from '../src/core/model.js';
import { CommunityModel } from '../src/core/community.js';
import { EditableMap } from '../src/map/editable-map.js';
import { Store } from '../src/ui/store.js';
import { CompartmentMenuController } from '../src/ui/compartment-menu.js';
import { MetaboliteMenuController } from '../src/ui/metabolite-menu.js';

async function menu(kind) {
  const raw = JSON.parse(await readFile(new URL('../models/ecoli_core_mc.json', import.meta.url), 'utf8'));
  const model = new Model(raw);
  const store = new Store('.');
  store.models.set('ecoli_core_mc', model);
  store.species = [{ id: 'eco', file: 'ecoli_core_mc', abundance: 1, enabled: true }];
  store.community = new CommunityModel({ species: [{ id: 'eco', model, abundance: 1 }], medium: {} });
  store.map = EditableMap.emptyFromCommunity(store.community);
  const stage = {
    clientWidth: 1200, clientHeight: 800,
    getBoundingClientRect: () => ({ left: 0, top: 0 }),
    addEventListener() {},
  };
  const options = { stage, store, screenToMap: (x, y) => ({ x, y }), onChange() {} };
  const at = { clientX: 500, clientY: 400 };
  if (kind === 'compartment') {
    const controller = new CompartmentMenuController(options);
    const open = () => controller.open('eco', 'c', at);
    open();
    return { controller, open };
  }
  store.addMapReaction('eco@PFK');
  const anchor = [...store.map.nodes.values()].find(node => node.mergedId === 'eco@fdp_c');
  const controller = new MetaboliteMenuController(options);
  const open = () => controller.open(anchor.id, at);
  open();
  return { controller, open };
}

function eventFor(selector, element) {
  return {
    target: {
      closest(value) {
        return value === selector ? element : null;
      },
    },
    preventDefault() {},
    stopPropagation() {},
  };
}

function input(value) {
  return {
    value,
    closest() {
      return {
        querySelectorAll: () => [],
        querySelector: () => null,
      };
    },
  };
}

for (const kind of ['compartment', 'metabolite']) {
  test(`the ${kind} reaction filter survives selection and both terminal controls`, async () => {
    const { controller } = await menu(kind);
    controller.handleFilterInput(eventFor('[data-action="filter-add-reactions"]', input('FBA')));
    controller.handleClick(eventFor('[data-select-add-rxn]', { dataset: { selectAddRxn: 'eco@FBA' } }));
    assert.match(controller.render(), /value="FBA"/);
    for (const step of ['1', '-1']) {
      controller.handleClick(eventFor('[data-rotate-terminal]', { dataset: { rotateTerminal: step } }));
      assert.match(controller.render(), /value="FBA"/);
    }
  });

  test(`the ${kind} filter is escaped and starts empty when reopened`, async () => {
    const { controller, open } = await menu(kind);
    controller.handleFilterInput(eventFor('[data-action="filter-add-reactions"]', input('FBA" <&')));
    controller.handleClick(eventFor('[data-select-add-rxn]', { dataset: { selectAddRxn: 'eco@FBA' } }));
    assert.match(controller.render(), /value="FBA&quot; &lt;&amp;"/);
    controller.close();
    open();
    assert.doesNotMatch(controller.render(), /value="FBA/);
  });
}
