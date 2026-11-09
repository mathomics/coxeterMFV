import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { EditableMap } from '../src/map/editable-map.js';
import { renderMap } from '../src/render/mapview.js';
import { crossFeedingColor, crossFeedingColors } from '../src/render/crossfeed-colors.js';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const scale = { reference: 1, width: () => 2, color: () => '#64748b' };

/** The published gut-nine map with cross-feeding lines for these links drawn on it. */
async function drawGut9(links, dark = false) {
  const doc = JSON.parse(await readFile(join(root, 'models', 'published', 'steadycom-gut-nine', 'map.json'), 'utf8'));
  const map = EditableMap.fromJSON(doc.map);
  const svg = renderMap(map, {
    fluxes: new Map(), scale, dark, showCrossFeeding: true,
    crossFeeding: { links: links.map(([from, base, to]) => ({ from, base, to, flux: 1 })), perMetabolite: [] },
  });
  // producer|base|consumer -> the colour of its line
  const re = /data-crossfeed="([^"]+)" data-crossfeed-from="([^"]+)" data-crossfeed-to="([^"]+)"><title>[^<]*<\/title><path class="emc-crossfeed" d="[^"]*" fill="none" stroke="([^"]+)"/g;
  return new Map([...svg.matchAll(re)].map(([, base, from, to, stroke]) => [`${from}|${base}|${to}`, stroke]));
}

test('every line of one metabolite has one colour, whichever species produces it', async () => {
  const lines = await drawGut9([['Bt', 'ac', 'Fp'], ['Ef', 'ac', 'Lc'], ['Kp', 'ac', 'St'], ['St', 'lac-L', 'Bt'], ['Ef', 'lac-L', 'St']]);
  assert.equal(lines.size, 5, 'every link is routed');
  const acetate = new Set(['Bt|ac|Fp', 'Ef|ac|Lc', 'Kp|ac|St'].map((key) => lines.get(key)));
  assert.equal(acetate.size, 1);
  assert.equal(lines.get('St|lac-L|Bt'), lines.get('Ef|lac-L|St'));
  assert.equal(crossFeedingColor('lac__L'), crossFeedingColor('lac-L'), 'BiGG and gut-model spellings are one metabolite');
});

test('acetate is blue and glycerol red, in either theme', async () => {
  assert.equal(crossFeedingColor('ac'), '#2563eb');
  assert.equal(crossFeedingColor('glyc'), '#dc2626');
  assert.equal(crossFeedingColor('ac', true), '#60a5fa');
  assert.equal(crossFeedingColor('glyc', true), '#f87171');
  const dark = await drawGut9([['Bt', 'ac', 'Fp'], ['Lc', 'glyc', 'Kp']], true);
  assert.deepEqual([...dark.values()], ['#60a5fa', '#f87171']);
});

test('two metabolites on one map never share a colour while a free one is left', async () => {
  const lines = await drawGut9([['Bt', 'ac', 'Fp'], ['St', 'lac-L', 'Bt'], ['Bt', 'succ', 'Ef'], ['Fp', 'for', 'Kp'], ['Bt', 'glc-D', 'St']]);
  assert.equal(new Set(lines.values()).size, 5);
  // fructose, maltose and galacturonate all ask for the same shared colour
  const sugars = crossFeedingColors(['malt', 'galur', 'fru']);
  assert.equal(new Set(sugars.values()).size, 3);
  assert.equal(sugars.get('fru'), crossFeedingColor('fru'), 'the first in id order keeps its colour');
});
