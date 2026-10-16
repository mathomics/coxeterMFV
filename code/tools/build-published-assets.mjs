#!/usr/bin/env node
/**
 * Generate the downloadable assets and map previews for the published communities.
 *
 *   node tools/build-published-assets.mjs --gut9=<dir with model9 files> \
 *                                         --iaf1260=<path to iAF1260.xml> \
 *                                         --stolyar=<dir with the four CellNetAnalyzer files>
 *
 * Any one of the three options is enough. The genome-scale sources are not in
 * this repository. They come from the authors' distributions, and
 * src/communities/published.js records where. What
 * this writes under models/published/ is derived from them:
 *
 *   <community>/manifest.json      what exists, for the UI to link
 *   <community>/media.csv          the medium: exchange reactions and bounds
 *   <community>/bounds.csv         every reaction's bounds and objective
 *   <community>/model-<species>.json   that member's model, COBRA JSON
 *   <community>/preview-<species>.json a small excerpt for the map
 *
 * The preview is an excerpt and says so in its own name field: a genome-scale
 * member has thousands of reactions and no map draws them usefully, so each one
 * is cut to the reactions around its biomass and its exchanges, at most
 * PREVIEW_REACTIONS of them.
 */
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const args = Object.fromEntries(process.argv.slice(2).map((a) => {
  const [k, v = 'true'] = a.replace(/^--/, '').split('=');
  return [k, v];
}));

/** How many reactions a map preview keeps per member. */
export const PREVIEW_REACTIONS = 20;

const csvCell = (v) => {
  const s = String(v ?? '');
  return /[",\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
};
const toCSV = (header, rows) =>
  [header, ...rows].map((r) => r.map(csvCell).join(',')).join('\n') + '\n';

/**
 * Cut a member down to something a map can draw.
 *
 * Biomass first, because it is what the community is solved for, then the
 * exchanges that connect this member to the shared medium, then whatever
 * reactions those metabolites reach. Taking the first N reactions of the file
 * instead would give an arbitrary slice with nothing joined up.
 */
function previewSubset(reactions, { biomass, exchanges, limit = PREVIEW_REACTIONS }) {
  const picked = new Map();
  const add = (id) => {
    if (picked.size >= limit || !id || picked.has(id)) return;
    const rxn = reactions.get(id);
    if (rxn) picked.set(id, rxn);
  };

  add(biomass);
  for (const id of exchanges) add(id);

  // Then reactions sharing a metabolite with what is already in, so the excerpt
  // is connected rather than a handful of loose arrows, smallest first. A
  // biomass reaction alone touches a hundred metabolites, and filling the rest
  // of the budget with more of those draws a hairball out of twenty reactions.
  const touched = new Set();
  for (const rxn of picked.values()) for (const m of Object.keys(rxn.metabolites)) touched.add(m);
  const candidates = [...reactions.entries()]
    .filter(([id, rxn]) => !picked.has(id)
      && Object.keys(rxn.metabolites).some((m) => touched.has(m)))
    .sort((a, b) => Object.keys(a[1].metabolites).length - Object.keys(b[1].metabolites).length);
  for (const [id] of candidates) {
    if (picked.size >= limit) break;
    add(id);
  }
  return picked;
}

/** Distinct cell colours, so a community map tells its members apart. */
const CELL_COLOURS = ['#0284c7', '#d97706', '#16a34a', '#7c3aed', '#db2777',
  '#0891b2', '#ca8a04', '#dc2626', '#4f46e5'];

/** A member as COBRA JSON, which is what this project's Model class reads. */
function toCobraJSON({ id, name, reactions, metabolites, objective, note, cell,
  sharedCompartment = null }) {
  /*
   * A member taken out of a joint community model carries the shared medium's
   * own metabolites: its species-to-medium reactions read `A[e_Bt] -> A[u]`.
   * Two metabolites in two compartments is a transport, so the map draws [u] as
   * a compartment belonging to that cell, and the shared medium ends up inside
   * the cell instead of between them. (It lands deep inside, because `u` is the
   * thylakoid lumen in the compartment registry, nested under the chloroplast.)
   *
   * Dropping the shared side leaves `A[e_Bt] ->`, an ordinary exchange, which is
   * exactly what CommunityModel re-points at the single shared medium sitting
   * between the cells. Same bounds, same direction, same flux: only the medium's
   * bookkeeping moves from the member to the community, which is where this app
   * keeps it for every other community.
   */
  const entries = [...reactions.entries()]
    .map(([rid, rxn]) => {
      if (!sharedCompartment) return [rid, rxn];
      const metabolites = Object.fromEntries(Object.entries(rxn.metabolites)
        .filter(([mid]) => compartmentOf(mid) !== sharedCompartment));
      return [rid, { ...rxn, metabolites }];
    })
    .filter(([, rxn]) => Object.keys(rxn.metabolites).length > 0);
  reactions = new Map(entries);

  const used = new Set();
  for (const rxn of reactions.values()) for (const m of Object.keys(rxn.metabolites)) used.add(m);
  const compartments = {};
  for (const mid of used) compartments[compartmentOf(mid)] = COMPARTMENT_NAMES[compartmentOf(mid)]
    ?? compartmentOf(mid);

  return {
    id,
    name: note ? `${name} — ${note}` : name,
    cell: cell ?? { name, organism: '', color: '#334155', shape: 'rounded' },
    compartments,
    metabolites: [...used].map((mid) => {
      const met = metabolites.get(mid);
      return {
        id: mid,
        name: met?.name ?? mid,
        compartment: compartmentOf(mid),
        formula: met?.formula ?? '',
        charge: met?.charge ?? 0,
      };
    }),
    reactions: [...reactions.values()].map((r) => ({
      id: r.id,
      name: r.name ?? r.id,
      metabolites: r.metabolites,
      lower_bound: r.lowerBound ?? r.lower_bound ?? 0,
      upper_bound: r.upperBound ?? r.upper_bound ?? 0,
      subsystem: r.subsystem ?? '',
    })),
    genes: [],
    objective: objective ? { [objective]: 1 } : null,
  };
}

const COMPARTMENT_NAMES = {
  c: 'Cytosol', e: 'Extracellular', p: 'Periplasm', u: 'Shared medium', m: 'Mitochondrion',
};

/** `glc[c_Bt]` -> `c`, `arg-L_e` -> `e`. Both spellings appear in these models. */
function compartmentOf(mid) {
  const bracket = /\[([a-z])(?:_[^\]]+)?\]$/.exec(mid);
  if (bracket) return bracket[1];
  const suffix = /_([a-z])$/.exec(mid);
  return suffix ? suffix[1] : 'c';
}

async function writeAll(dir, files) {
  await mkdir(dir, { recursive: true });
  for (const [name, body] of Object.entries(files)) {
    await writeFile(join(dir, name), body);
  }
}

// The nine-species gut model

async function buildGut9(sourceDir) {
  const { importGut9, reactionsBySpecies, diet } = await import('../src/communities/gut9.js');
  const model = importGut9(
    await readFile(join(sourceDir, 'model9.xlsx')),
    { boundsText: await readFile(join(sourceDir, 'model9_rxns_params.txt'), 'utf8') },
  );

  const dir = join(root, 'models', 'published', 'steadycom-gut-nine');
  const bySpecies = reactionsBySpecies(model);
  const files = {};
  const members = [];

  // The medium, as the model states it: a community uptake reaction, the
  // metabolite it supplies and the rate the diet allows.
  const dietMap = diet(model);
  files['media.csv'] = toCSV(
    ['community_metabolite', 'uptake_reaction', 'lower_bound', 'upper_bound', 'export_reaction'],
    [...dietMap.keys()].sort().map((met) => {
      const ut = model.communityUptake.get(met);
      const rxn = model.reactions.get(ut);
      return [met, ut, rxn.lowerBound, rxn.upperBound, model.communityExport.get(met) ?? ''];
    }),
  );

  files['bounds.csv'] = toCSV(
    ['reaction', 'species', 'lower_bound', 'upper_bound', 'objective_coefficient'],
    [...model.reactions.values()].map((r) =>
      [r.id, r.species, r.lowerBound, r.upperBound, r.objective]),
  );

  for (const sp of model.species) {
    const ids = bySpecies.get(sp.id);
    const reactions = new Map(ids.map((id) => [id, model.reactions.get(id)]));
    files[`model-${sp.id}.json`] = JSON.stringify(toCobraJSON({
      id: sp.id, name: sp.name, reactions, metabolites: model.metabolites,
      objective: sp.biomassReaction,
    }));

    const exchanges = ids.filter((id) => /(^|_)I?EX_/.test(id)).slice(0, 8);
    const preview = previewSubset(reactions, { biomass: sp.biomassReaction, exchanges });
    files[`preview-${sp.id}.json`] = JSON.stringify(toCobraJSON({
      id: `${sp.id}_preview`, name: sp.name, reactions: preview, metabolites: model.metabolites,
      objective: sp.biomassReaction,
      note: `map excerpt, ${preview.size} of ${ids.length} reactions`,
      sharedCompartment: 'u',
      cell: {
        name: sp.id, organism: sp.name, shape: 'rounded',
        color: CELL_COLOURS[model.species.indexOf(sp) % CELL_COLOURS.length],
        note: `${ids.length} reactions in the full model`,
      },
    }), null, 1);

    members.push({
      id: sp.id,
      name: sp.name,
      reactions: ids.length,
      metabolites: sp.declaredMetabolites,
      model: `model-${sp.id}.json`,
      preview: `preview-${sp.id}.json`,
      previewReactions: preview.size,
    });
  }

  // The medium the previews get is the paper's own diet, narrowed to the
  // metabolites an excerpt can actually exchange. The excerpt's growth rate is
  // not the community's (the recorded results give that), but the medium it
  // runs on is not invented either.
  const previewBases = new Set();
  for (const name of Object.keys(files)) {
    if (!name.startsWith('preview-')) continue;
    for (const r of JSON.parse(files[name]).reactions) {
      const base = /^(?:I?EX|UT)_(.+?)(?:\[u\]tr|\(u\)|\(e\)|\[e\])?(?:_[A-Z][a-z])?$/.exec(r.id)?.[1];
      if (base) previewBases.add(base);
    }
  }
  const medium = {};
  for (const [met, rate] of dietMap) {
    const base = met.replace(/\[u\]$/, '');
    if (previewBases.has(base)) medium[base] = rate;
  }

  files['manifest.json'] = JSON.stringify({
    community: 'steadycom-gut-nine',
    medium,
    source: 'model9.xlsx + model9_rxns_params.txt, from the paper\'s distribution',
    generatedBy: 'tools/build-published-assets.mjs',
    media: 'media.csv',
    bounds: 'bounds.csv',
    // Curated by hand and with tools/gut9-map/; not written by this script.
    map: 'map.json',
    members,
  }, null, 2);

  await writeAll(dir, files);
  return { dir, members: members.length, files: Object.keys(files).length };
}

// The four-auxotroph E. coli set

async function buildAuxotrophs(xmlPath) {
  const { buildCommunity, AMINO_ACID_EXCHANGES } =
    await import('../src/communities/steadycom-ecoli.js');
  const spec = buildCommunity(await readFile(xmlPath, 'utf8'));

  const dir = join(root, 'models', 'published', 'steadycom-ecoli-auxotrophs');
  const files = {};
  const members = [];

  // Every strain is the same reconstruction with a few reactions shut, so the
  // medium and the bounds are taken from the first and the differences are
  // listed per strain rather than four near-identical files being written.
  const first = spec.members[0].model;
  files['media.csv'] = toCSV(
    ['metabolite', 'exchange_reaction', 'lower_bound', 'upper_bound'],
    Object.entries(spec.communityUptake).sort(([a], [b]) => a.localeCompare(b)).map(([met, rate]) => {
      const rid = spec.members[0].exchangeFor[met];
      const rxn = first.reactions.get(rid);
      return [met, rid, rxn ? rxn.lowerBound : -rate, rxn ? rxn.upperBound : 1000];
    }),
  );

  files['bounds.csv'] = toCSV(
    ['reaction', ...spec.members.map((m) => `${m.id}_lb`), ...spec.members.map((m) => `${m.id}_ub`)],
    [...first.reactions.keys()].map((rid) => [
      rid,
      ...spec.members.map((m) => m.model.reactions.get(rid)?.lowerBound ?? ''),
      ...spec.members.map((m) => m.model.reactions.get(rid)?.upperBound ?? ''),
    ]),
  );

  for (const member of spec.members) {
    const reactions = member.model.reactions;
    const metabolites = member.model.metabolites;
    files[`model-${member.id}.json`] = JSON.stringify(toCobraJSON({
      id: member.id, name: `${member.id} — iAF1260`, reactions, metabolites,
      objective: member.biomassReaction,
    }));

    // The cross-fed amino acids, plus the exchanges the diet actually supplies
    // most of. Otherwise the excerpt has nothing to take up and its medium
    // comes out empty.
    // By name, not by rate: sorting by rate picks the minerals that sit at 1000
    // and leaves out the carbon and nitrogen the excerpt actually runs on.
    const diet = ['glc', 'o2', 'nh4', 'pi', 'so4', 'h2o']
      .filter((met) => spec.communityUptake[met] !== undefined)
      .map((met) => member.exchangeFor[met]).filter(Boolean);
    const exchanges = [...Object.values(AMINO_ACID_EXCHANGES), ...diet];
    const preview = previewSubset(reactions, { biomass: member.biomassReaction, exchanges });
    files[`preview-${member.id}.json`] = JSON.stringify(toCobraJSON({
      id: `${member.id}_preview`, name: member.id, reactions: preview, metabolites,
      objective: member.biomassReaction,
      note: `map excerpt, ${preview.size} of ${reactions.size} reactions`,
      cell: {
        name: member.id, organism: 'Escherichia coli K-12 MG1655', shape: 'rod', gram: 'negative',
        color: CELL_COLOURS[spec.members.indexOf(member) % CELL_COLOURS.length],
        note: `auxotroph for ${member.strain.auxotrophFor.join(' + ')};`
          + ` cannot export ${member.strain.cannotExport}`,
      },
    }), null, 1);

    members.push({
      id: member.id,
      name: member.id,
      reactions: reactions.size,
      metabolites: metabolites.size,
      model: `model-${member.id}.json`,
      preview: `preview-${member.id}.json`,
      previewReactions: preview.size,
    });
  }

  const previewBases = new Set();
  for (const name of Object.keys(files)) {
    if (!name.startsWith('preview-')) continue;
    for (const r of JSON.parse(files[name]).reactions) {
      const base = /^EX_(.+?)_e$/.exec(r.id)?.[1];
      if (base) previewBases.add(base);
    }
  }
  const medium = {};
  for (const [met, rate] of Object.entries(spec.communityUptake)) {
    if (previewBases.has(met)) medium[met] = rate;
  }

  files['manifest.json'] = JSON.stringify({
    community: 'steadycom-ecoli-auxotrophs',
    medium,
    source: 'iAF1260.xml, with the tutorial\'s knockouts applied per strain',
    generatedBy: 'tools/build-published-assets.mjs',
    media: 'media.csv',
    bounds: 'bounds.csv',
    members,
  }, null, 2);

  await writeAll(dir, files);
  return { dir, members: members.length, files: Object.keys(files).length };
}

// The two-species mutualism, 2007

/**
 * Stolyar et al.'s joint model, from the four CellNetAnalyzer supplement files.
 *
 * These two members are small (96 and 89 reactions), so the map excerpt is the
 * whole model rather than a cut of it, and the growth rate the app solves on
 * that map is the model's own. The manifest records both counts, so a reader
 * can tell this case from the genome-scale ones where they differ.
 */
async function buildStolyar(sourceDir) {
  const { importStolyar, validateStolyar, toSpeciesCobraJSON, toJointCobraJSON, SPECIES,
    reactionsBySpecies, externalMetabolites } = await import('../src/communities/stolyar.js');

  const read = (name) => readFile(join(sourceDir, name), 'utf8');
  const model = importStolyar({
    reactions: await read('reactions.txt'),
    metabolites: await read('metabolites.txt'),
    macromolecules: await read('macromolecules.txt'),
    synthesis: await read('macromolecule_synthesis.txt'),
  });
  const check = validateStolyar(model);
  if (!check.ok) throw new Error(`Stolyar import does not validate: ${check.problems.join('; ')}`);

  const dir = join(root, 'models', 'published', 'stolyar-2007-syntrophy');
  const files = {};
  const members = [];
  const { bySpecies } = reactionsBySpecies(model);

  // The medium here is what enters or leaves the pair: the metabolites the file
  // marks external, plus the shared pool. Lactate is not among them. It is
  // produced inside D. vulgaris by D86, whose measured rate is the supply.
  files['media.csv'] = toCSV(
    ['metabolite', 'name', 'kind', 'exchange_reaction'],
    [
      ...externalMetabolites(model).map((met) => [met, model.metabolites.get(met).name,
        'external', `EX_${met}`]),
      ...[...model.shared].sort().map((met) => [met, model.metabolites.get(met).name,
        'shared between the two organisms', SPECIES.map((sp) => `EX_${met}_${sp.id}`).join(' ')]),
    ],
  );

  files['bounds.csv'] = toCSV(
    ['reaction', 'species', 'lower_bound', 'upper_bound', 'objective_coefficient', 'measured_rate'],
    [...model.reactions.values()].map((r) => [r.id, r.species ?? 'pair',
      r.lowerBound, r.upperBound, r.objective, r.measured ?? '']),
  );

  files['joint-model.json'] = JSON.stringify(toJointCobraJSON(model), null, 1);

  for (const [index, sp] of SPECIES.entries()) {
    const doc = toSpeciesCobraJSON(model, sp.id, {
      cell: {
        name: sp.id, organism: sp.name, shape: 'rounded',
        color: CELL_COLOURS[index % CELL_COLOURS.length],
        note: `${bySpecies.get(sp.id).length} reactions in the joint model`,
      },
    });
    files[`model-${sp.id}.json`] = JSON.stringify(doc);
    // Small enough to draw whole, so the preview is the model.
    files[`preview-${sp.id}.json`] = JSON.stringify(doc, null, 1);
    members.push({
      id: sp.id,
      name: sp.name,
      reactions: doc.reactions.length,
      metabolites: doc.metabolites.length,
      model: `model-${sp.id}.json`,
      preview: `preview-${sp.id}.json`,
      previewReactions: doc.reactions.length,
      wholeModel: true,
    });
  }

  files['manifest.json'] = JSON.stringify({
    community: 'stolyar-2007-syntrophy',
    /*
     * Ammonium is free, as the file's external flag makes it. Everything else
     * stays closed, which is the syntrophic condition, and sulfate appears in
     * the medium panel at zero so it can be opened by hand.
     *
     * Opening it does not break the partnership on this map, and that is not a
     * fault: the app solves for one growth rate shared by both cells, so the
     * optimum keeps feeding the methanogen because the common rate is what is
     * being maximised. The collapse the paper describes belongs to its own
     * weighted objective, which is what the recorded sweep uses.
     *
     * Lactate is not here because it is not an input: D86 makes it inside the
     * cell at the rate the file measures.
     */
    medium: { 'NH3(ext)': 1000 },
    source: 'Stolyar et al. 2007 supplementary material, four CellNetAnalyzer files',
    generatedBy: 'tools/build-published-assets.mjs',
    media: 'media.csv',
    bounds: 'bounds.csv',
    joint: 'joint-model.json',
    members,
  }, null, 2);

  await writeAll(dir, files);
  return { dir, members: members.length, files: Object.keys(files).length };
}

// Main

if (args.gut9) {
  const r = await buildGut9(args.gut9);
  console.log(`steadycom-gut-nine: ${r.members} members, ${r.files} files`);
}
if (args.iaf1260) {
  const r = await buildAuxotrophs(args.iaf1260);
  console.log(`steadycom-ecoli-auxotrophs: ${r.members} members, ${r.files} files`);
}
if (args.stolyar) {
  const r = await buildStolyar(args.stolyar);
  console.log(`stolyar-2007-syntrophy: ${r.members} members, ${r.files} files`);
}
if (!args.gut9 && !args.iaf1260 && !args.stolyar) {
  console.error('nothing to do: pass --gut9=<dir>, --iaf1260=<file> and/or --stolyar=<dir>');
  process.exit(2);
}
