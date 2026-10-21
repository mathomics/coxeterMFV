#!/usr/bin/env node
/**
 * The curated map of the yeast9-9-1-1 scenario, copied from the hand-made
 * Yeast9 map in the coxeter_mapping repository (cytoplasm, mitochondrion,
 * peroxisome).
 *
 * That map was drawn over yeastgem_v9.1.1-edited.json opened as an upload, so
 * every identifier starts with the species id the file declares
 * (`yeastGEM_v9.1.1@r_2034`, `met:yeastGEM_v9.1.1@s_1399`), while the scenario
 * calls its one species `sce` and its model `yeast9-9-1-1`. The edited JSON
 * matches the pinned yeast-GEM 9.1.1 SBML the scenario downloads in reactions,
 * coefficients and bounds, and both use the s_/r_ ids, so only the species id
 * changes: it is rewritten wherever it starts an identifier (renameMapSpecies,
 * the rewrite the app itself applies to a map saved over another export of the
 * same model) and the envelope records the scenario. The drawing is untouched.
 *
 * The source is read from a pinned commit of coxeter_mapping, so the output is
 * the same on every run; --from=<file> reads another copy instead.
 *
 * Usage: node tools/scenario-maps/yeast9-map.mjs [--from=<map.json>] [--mapping=<coxeter_mapping dir>]
 */
import { execFileSync } from 'node:child_process';
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseMapDocument, renameMapSpecies, MAP_DOCUMENT_FORMAT, MAP_DOCUMENT_VERSION } from '../../src/map/map-document.js';
import { YEAST9_SCENARIO_ID, YEAST9_MODEL_KEY } from '../../src/core/yeast9.js';

const root = join(dirname(fileURLToPath(import.meta.url)), '..', '..');
const OUT = join(root, 'models', 'maps', `${YEAST9_SCENARIO_ID}.json`);

// The map as the user last saved it, without the TAG module: 135 reactions.
const SOURCE = {
  commit: '472d9db1235b23e9d2bce8d8d0d51a0b54234246',
  path: 'Maps/yeastGEM_v9.1.1_map_cytomito.json',
  speciesId: 'yeastGEM_v9.1.1',
};
const SPECIES_ID = 'sce';

const option = (name) => process.argv.slice(2).find((arg) => arg.startsWith(`--${name}=`))?.slice(name.length + 3) ?? null;
const mapping = resolve(option('mapping') ?? join(root, '..', 'coxeter_mapping'));
const from = option('from');

const text = from
  ? await readFile(resolve(from), 'utf8')
  : execFileSync('git', ['-C', mapping, 'show', `${SOURCE.commit}:${SOURCE.path}`], { encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 });

const parsed = parseMapDocument(text);
const species = [...new Set((parsed.map.cells ?? []).map((cell) => cell.id))];
if (species.length !== 1 || species[0] !== SOURCE.speciesId) {
  throw new Error(`Expected one cell "${SOURCE.speciesId}" in the source map, found ${JSON.stringify(species)}.`);
}

const renamed = renameMapSpecies(parsed, SOURCE.speciesId, SPECIES_ID, { model: YEAST9_MODEL_KEY });
const left = JSON.stringify(renamed.map).match(new RegExp(`${SOURCE.speciesId.replace(/\./g, '\\.')}[@/]`, 'g'));
if (left) throw new Error(`${left.length} identifiers still name ${SOURCE.speciesId}.`);

const map = renamed.map;
const doc = {
  format: MAP_DOCUMENT_FORMAT,
  version: MAP_DOCUMENT_VERSION,
  metadata: {
    createdAt: parsed.metadata?.createdAt ?? null,
    scenario: YEAST9_SCENARIO_ID,
    species: [{ id: SPECIES_ID, model: YEAST9_MODEL_KEY }],
    reactionCount: map.reactions.length,
    nodeCount: map.nodes.length,
    segmentCount: map.segments.length,
    textCount: (map.texts ?? []).length,
  },
  pathwayManager: renamed.pathwayManager ?? null,
  fluxColorStops: parsed.fluxColorStops ?? null,
  map,
};

await mkdir(dirname(OUT), { recursive: true });
await writeFile(OUT, `${JSON.stringify(doc, null, 2)}\n`);
const compartments = map.cells[0].compartments.map((c) => c.id).join(', ');
console.log(`${OUT}: ${map.reactions.length} reactions, ${map.nodes.length} nodes, ${map.segments.length} segments; compartments ${compartments}`);
