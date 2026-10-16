// Reference outputs of src/core for the Rust port's parity tests.
//
//   node rust/tools/fixtures-core.mjs
//
// Writes rust/fixtures/core/: every bundled model read by Model (with what it
// derives) and every bundled scenario merged by CommunityModel.
import { readFile, writeFile, mkdir, readdir } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '../..');
const out = join(root, 'rust/fixtures/core');
await mkdir(out, { recursive: true });
const { Model } = await import(pathToFileURL(join(root, 'src/core/model.js')).href);
const { CommunityModel } = await import(pathToFileURL(join(root, 'src/core/community.js')).href);
const { modelBoundaryIndex, reactionsByBoundary, communityBoundaries } = await import(pathToFileURL(join(root, 'src/map/boundaries.js')).href);

const treeJSON = (tree) => ({
  roots: tree.roots.map((n) => n.id),
  byId: Object.fromEntries([...tree.byId].map(([id, n]) => [id, { children: n.children.map((c) => c.id), parent: n.parent, depth: n.depth }])),
});

function describeModel(model) {
  return {
    json: model.toJSON(),
    compartmentIds: model.compartmentIds(),
    compartmentNames: model.compartmentNames,
    tree: treeJSON(model.compartmentTree()),
    biomass: model.biomassReactionId(),
    objective: model.objectiveReactions(),
    reactions: Object.fromEntries([...model.reactions.values()].map((r) => [r.id, {
      kind: r.kind, compartments: r.compartments, equation: r.equation(), reversible: r.reversible,
    }])),
    metabolites: Object.fromEntries([...model.metabolites.values()].map((m) => [m.id, {
      base: m.base, primary: m.isPrimary, reactions: m.reactions, labelSources: m.labelSources,
    }])),
    massBalance: model.checkMassBalance(),
    boundaryIndex: Object.fromEntries(modelBoundaryIndex(model)),
  };
}

const index = JSON.parse(await readFile(join(root, 'models/index.json'), 'utf8'));
const files = (await readdir(join(root, 'models'))).filter((f) => f.endsWith('_mc.json'));
const models = new Map();
for (const f of files) {
  const raw = JSON.parse(await readFile(join(root, 'models', f), 'utf8'));
  const model = new Model(raw);
  models.set(f.replace(/\.json$/, ''), model);
  await writeFile(join(out, `model-${f}`), JSON.stringify(describeModel(model)));
}

const { scenarios } = JSON.parse(await readFile(join(root, 'models/scenarios.json'), 'utf8'));
const written = [];
for (const s of scenarios) {
  if (s.loader) continue; // fetched from the internet (Yeast9)
  const medium = {};
  for (const [base, limit] of Object.entries(s.medium ?? {})) medium[base] = { lower: -Math.abs(Number(limit) || 0), upper: 1000 };
  const spec = {
    name: s.name, objective: s.objective || 'equal-growth', productTarget: s.productTarget || null, medium,
    species: s.species.map((sp) => ({ id: sp.id, model: models.get(sp.model), abundance: sp.abundance ?? 1 / s.species.length, enabled: sp.enabled !== false })),
  };
  const c = new CommunityModel(spec);
  await writeFile(join(out, `community-${s.id}.json`), JSON.stringify({
    spec: { ...spec, species: s.species.map((sp, i) => ({ ...spec.species[i], model: sp.model })) },
    merged: describeModel(c.merged),
    reactionOwner: Object.fromEntries(c.reactionOwner),
    metaboliteOwner: Object.fromEntries(c.metaboliteOwner),
    transfersByBase: Object.fromEntries(c.transfersByBase),
    speciesCompartments: Object.fromEntries(c.speciesCompartments),
    biomassIds: Object.fromEntries(c.biomassIds),
    abundances: Object.fromEntries(c.abundances),
    reactionsByBoundary: Object.fromEntries(reactionsByBoundary(c)),
    boundaries: communityBoundaries(c),
  }));
  written.push(s.id);
}
await writeFile(join(out, 'index.json'), JSON.stringify({ models: [...models.keys()], scenarios: written, modelIndex: index }, null, 1));
console.log(`wrote ${models.size} models and ${written.length} scenarios to ${out}`);
