import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile, access } from 'node:fs/promises';
import { join } from 'node:path';
import { deflateRawSync } from 'node:zlib';
import { readWorkbook } from '../src/core/xlsx.js';
import {
  importGut9, validateBounds, validateStoichiometry, reactionsBySpecies,
  communityReactionIds, diet, toSteadyComSpec, parseBounds, EXPECTED, PROVENANCE,
} from '../src/communities/gut9.js';
import {
  BT_FIBER, BT_CARBOHYDRATE, FP_FIBER, CARBON_NUMBER, FIBER_SWEEP,
  FIGURE_SPECIES, fiberConstraints, figureTargets, exchangeMetaboliteOf,
} from '../src/communities/gut9-figure4.js';

// The xlsx reader

/**
 * A workbook built here, so the reader is tested on something whose every value
 * is known. Entries are stored rather than deflated, which the reader supports
 * and which keeps this to the ZIP structure alone.
 */
function miniWorkbook(sheetXml, sharedStrings = []) {
  const files = [
    ['xl/workbook.xml',
      '<workbook><sheets><sheet name="data" sheetId="1" r:id="rId1"/></sheets></workbook>'],
    ['xl/_rels/workbook.xml.rels',
      '<Relationships><Relationship Id="rId1" Type="x/worksheet" Target="worksheets/sheet1.xml"/>'
      + '</Relationships>'],
    ['xl/sharedStrings.xml',
      `<sst>${sharedStrings.map((s) => `<si><t>${s}</t></si>`).join('')}</sst>`],
    ['xl/worksheets/sheet1.xml', `<worksheet><sheetData>${sheetXml}</sheetData></worksheet>`],
  ];

  const locals = [];
  const central = [];
  let offset = 0;
  for (const [name, text] of files) {
    const nameBuf = Buffer.from(name, 'utf8');
    const data = Buffer.from(text, 'utf8');
    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(0, 8);                 // flags
    local.writeUInt16LE(0, 10);                // stored
    local.writeUInt32LE(data.length, 18);      // compressed size
    local.writeUInt32LE(data.length, 22);      // uncompressed size
    local.writeUInt16LE(nameBuf.length, 26);
    locals.push(local, nameBuf, data);

    const dir = Buffer.alloc(46);
    dir.writeUInt32LE(0x02014b50, 0);
    dir.writeUInt16LE(0, 10);                  // stored
    dir.writeUInt32LE(data.length, 20);
    dir.writeUInt32LE(data.length, 24);
    dir.writeUInt16LE(nameBuf.length, 28);
    dir.writeUInt32LE(offset, 42);
    central.push(dir, nameBuf);
    offset += local.length + nameBuf.length + data.length;
  }

  const body = Buffer.concat(locals);
  const directory = Buffer.concat(central);
  const eocd = Buffer.alloc(22);
  eocd.writeUInt32LE(0x06054b50, 0);
  eocd.writeUInt16LE(files.length, 8);
  eocd.writeUInt16LE(files.length, 10);
  eocd.writeUInt32LE(directory.length, 12);
  eocd.writeUInt32LE(body.length, 16);
  return Buffer.concat([body, directory, eocd]);
}

test('the xlsx reader keys cells by column, not by position', () => {
  // The bug this guards: empty cells are omitted from the XML, so reading a row
  // as a list shifts every value after a gap one column to the left. Here B and
  // C are missing from the second row, and D must still read as "right".
  const xml = '<row r="1"><c r="A1" t="s"><v>0</v></c><c r="D1" t="s"><v>1</v></c></row>'
    + '<row r="2"><c r="A2" t="s"><v>2</v></c><c r="D2" t="s"><v>3</v></c></row>';
  const wb = readWorkbook(miniWorkbook(xml, ['head', 'tail', 'left', 'right']));
  const rows = [...wb.rows('data')];
  assert.deepEqual(rows[0], { __row: 1, A: 'head', D: 'tail' });
  assert.deepEqual(rows[1], { __row: 2, A: 'left', D: 'right' });
  assert.equal(rows[1].B, undefined, 'a missing cell must stay missing');
});

test('the xlsx reader handles numbers, inline strings and deflated entries', () => {
  const xml = '<row r="1"><c r="A1"><v>42.5</v></c>'
    + '<c r="B1" t="inlineStr"><is><t>in</t><t>line</t></is></c>'
    + '<c r="C1" t="s"><v>0</v></c><c r="D1"/></row>';
  const wb = readWorkbook(miniWorkbook(xml, ['shared &amp; escaped']));
  const [row] = [...wb.rows('data')];
  assert.equal(row.A, '42.5');
  assert.equal(row.B, 'inline', 'text runs are joined');
  assert.equal(row.C, 'shared & escaped', 'entities are decoded');
  assert.equal(row.D, undefined, 'an empty cell yields nothing');
});

test('the xlsx reader reports a missing sheet by name', () => {
  const wb = readWorkbook(miniWorkbook('<row r="1"><c r="A1"><v>1</v></c></row>'));
  assert.deepEqual(wb.sheetNames, ['data']);
  assert.throws(() => [...wb.rows('nope')], /no sheet named "nope"/);
});

// The Figure 4 transcription

test('the Figure 4 lists are the ones simFigure4.m defines', () => {
  assert.equal(BT_FIBER.length, 12);
  assert.equal(BT_CARBOHYDRATE.length, 18);
  assert.equal(FP_FIBER.length, 5);
  assert.equal(Object.keys(CARBON_NUMBER).length, 31);
  // carbonUTmax = [1:30; 5*ones(1,30)]
  assert.deepEqual(FIBER_SWEEP.bacteroides, Array.from({ length: 30 }, (_, i) => i + 1));
  assert.equal(FIBER_SWEEP.faecalibacterium, 5);
  // The five the figure stacks, in its own order: minFlux([1 2 8 4 7],:) over
  // the member order Bt,Fp,Kp,St,Ba,Ec,Ef,Er,Lc.
  assert.deepEqual(FIGURE_SPECIES, ['Bt', 'Fp', 'Er', 'St', 'Ef']);
  // Every reaction the sweep names must be addressed to the species it belongs
  // to, and the two naming conventions are not interchangeable.
  assert.ok(BT_FIBER.every((r) => r.endsWith('_Bt')));
  assert.ok(FP_FIBER.every((r) => r.startsWith('IEX_') && r.endsWith('_Fp')));
  // A few carbon numbers, spot-checked against the script's own table.
  assert.equal(CARBON_NUMBER['pect[u]'], 2535);
  assert.equal(CARBON_NUMBER['glycogen1500[u]'], 9000);
  assert.equal(CARBON_NUMBER['xyl-D[u]'], 5);
});

test('the metadata cites the paper the model comes from', () => {
  assert.equal(PROVENANCE.paper.doi, '10.1371/journal.pcbi.1005539');
  assert.equal(PROVENANCE.paper.year, 2017);
  assert.equal(PROVENANCE.paper.authors, 'Chan SHJ, Simons MN, Maranas CD');
  assert.equal(PROVENANCE.paper.journal, 'PLoS Computational Biology');
  assert.equal(PROVENANCE.paper.framework, 'SteadyCom');
  // All five files the work is allowed to draw on are named.
  assert.deepEqual(Object.values(PROVENANCE.files).sort(), [
    'model9.xlsx', 'model9_readme.txt', 'model9_rxns_params.txt',
    'model9_stoich.txt', 'simFigure4.m',
  ].sort());
});

test('parseBounds reads the TXT export and rejects a foreign file', () => {
  const text = 'rxns\trev\tlb\tub\tc\nR1\t1\t-1.0e+00\t2.5e+00\t0\nR2\t0\t0\t1e+03\t1\n';
  const bounds = parseBounds(text);
  assert.equal(bounds.size, 2);
  assert.deepEqual(bounds.get('R1'), { reversible: true, lowerBound: -1, upperBound: 2.5, objective: 0 });
  assert.equal(bounds.get('R2').objective, 1);
  assert.throws(() => parseBounds('something\telse\n'), /unexpected header/);
});

// The model itself, when it is present

const DIR = process.env.COXETER_GUT9_DIR ?? '';

async function gut9() {
  if (!DIR) return null;
  try {
    await access(join(DIR, 'model9.xlsx'));
  } catch {
    return null;
  }
  const xlsx = await readFile(join(DIR, 'model9.xlsx'));
  const boundsText = await readFile(join(DIR, 'model9_rxns_params.txt'), 'utf8');
  const stoichText = await readFile(join(DIR, 'model9_stoich.txt'), 'utf8');
  return { xlsx, boundsText, stoichText, model: importGut9(xlsx, { boundsText }) };
}

const SKIP = 'set COXETER_GUT9_DIR to the folder holding model9.xlsx, '
  + 'model9_rxns_params.txt and model9_stoich.txt';

const loaded = await gut9();

test('the model has exactly nine species, each with its biomass and maintenance', (t) => {
  if (!loaded) return t.skip(SKIP);
  const { model } = loaded;
  assert.equal(model.species.length, EXPECTED.species);
  assert.deepEqual(model.species.map((s) => s.id), ['Bt', 'Fp', 'Kp', 'St', 'Ba', 'Ec', 'Ef', 'Er', 'Lc']);

  for (const sp of model.species) {
    const biomass = model.reactions.get(sp.biomassReaction);
    const atpm = model.reactions.get(sp.atpMaintenance);
    assert.ok(biomass, `${sp.id}: no biomass reaction ${sp.biomassReaction}`);
    assert.ok(atpm, `${sp.id}: no ATP maintenance reaction ${sp.atpMaintenance}`);
    assert.equal(biomass.species, sp.id);
    assert.equal(atpm.species, sp.id);
    // Maintenance is a floor, not an option: the paper's model holds every
    // member to 1 mmol ATP per gDW per hour whether it grows or not.
    assert.equal(atpm.lowerBound, 1, `${sp.id}: ATPM lower bound`);
    assert.equal(atpm.upperBound, 1000, `${sp.id}: ATPM upper bound`);
    assert.ok(biomass.objective !== 0, `${sp.id}: biomass is not in the objective`);
  }

  // Nothing else is in the objective.
  const objectives = [...model.reactions.values()].filter((r) => r.objective !== 0);
  assert.equal(objectives.length, EXPECTED.species);
});

test('the imported dimensions are the ones the workbook declares', (t) => {
  if (!loaded) return t.skip(SKIP);
  const { model } = loaded;
  assert.equal(model.reactions.size, EXPECTED.reactions);
  assert.equal(model.metabolites.size, EXPECTED.metabolites);
  assert.equal(model.stoichiometryEntries, EXPECTED.stoichiometryEntries);

  // comSp declares a count per species; the import must agree with all nine.
  const bySpecies = reactionsBySpecies(model);
  const metsBySpecies = new Map(model.species.map((s) => [s.id, 0]));
  for (const met of model.metabolites.values()) {
    if (metsBySpecies.has(met.species)) {
      metsBySpecies.set(met.species, metsBySpecies.get(met.species) + 1);
    }
  }
  for (const sp of model.species) {
    assert.equal(bySpecies.get(sp.id).length, sp.declaredReactions, `${sp.id} reactions`);
    assert.equal(metsBySpecies.get(sp.id), sp.declaredMetabolites, `${sp.id} metabolites`);
  }
  // Species reactions plus community exchange reactions account for everything.
  const speciesTotal = [...bySpecies.values()].reduce((s, list) => s + list.length, 0);
  assert.equal(speciesTotal + communityReactionIds(model).length, EXPECTED.reactions);
});

test('the stoichiometric matrix matches model9_stoich.txt exactly', (t) => {
  if (!loaded) return t.skip(SKIP);
  const report = validateStoichiometry(loaded.model, loaded.stoichText);
  assert.equal(report.entries, EXPECTED.stoichiometryEntries);
  assert.equal(report.identical, EXPECTED.stoichiometryEntries);
  // Both directions, so neither a dropped nor an invented coefficient passes.
  assert.deepEqual(report.differing, []);
  assert.deepEqual(report.onlyInText, []);
  assert.deepEqual(report.onlyInModel, []);
});

test('every bound and objective coefficient matches model9_rxns_params.txt', (t) => {
  if (!loaded) return t.skip(SKIP);
  const report = validateBounds(loaded.model, loaded.boundsText);
  assert.equal(report.identical, EXPECTED.reactions);
  assert.deepEqual(report.differing, []);
  assert.deepEqual(report.onlyInModel, []);
  assert.deepEqual(report.onlyInText, []);
});

test('the workbook carries no diet, and the TXT carries it on 88 uptake reactions', (t) => {
  if (!loaded) return t.skip(SKIP);
  // This is the one place the workbook is not the primary source, so it is
  // asserted rather than described: in model9.xlsx every community uptake
  // reaction is shut, and the 88 bounds the TXT differs on are all of them.
  const fromWorkbook = importGut9(loaded.xlsx);
  assert.equal(diet(fromWorkbook).size, 0, 'the workbook opens no uptake at all');

  const report = validateBounds(fromWorkbook, loaded.boundsText);
  assert.equal(report.differing.length, EXPECTED.dietReactions);
  assert.ok(report.differing.every((d) => d.id.startsWith('UT_')),
    'the disagreement is confined to community uptake reactions');
  assert.equal(report.identical, EXPECTED.reactions - EXPECTED.dietReactions);

  // And with the TXT applied, that is exactly what opens.
  assert.equal(diet(loaded.model).size, EXPECTED.dietReactions);
  assert.equal(loaded.model.boundChanges.length, EXPECTED.dietReactions);
  assert.equal(loaded.model.boundsFrom, 'model9_rxns_params.txt');
});

test('the community exchange space is species <-> medium <-> environment', (t) => {
  if (!loaded) return t.skip(SKIP);
  const { model } = loaded;

  assert.equal(model.communityMetabolites.length, EXPECTED.communityMetabolites);
  assert.equal(model.communityUptake.size, EXPECTED.communityMetabolites);
  assert.equal(model.communityExport.size, EXPECTED.communityMetabolites);
  assert.equal(model.speciesExchange.size, EXPECTED.speciesExchangeReactions);
  assert.equal(communityReactionIds(model).length, EXPECTED.communityExchangeReactions);
  assert.deepEqual(model.exspOrder, ['Bt', 'Fp', 'Kp', 'St', 'Ba', 'Ec', 'Ef', 'Er', 'Lc']);

  // Environment -> medium: a UT reaction produces its community metabolite and
  // nothing else. Medium -> environment: an EX reaction consumes it.
  for (const [met, rxnId] of model.communityUptake) {
    const rxn = model.reactions.get(rxnId);
    assert.ok(rxn, `${rxnId} is missing`);
    assert.equal(rxn.species, 'comm');
    assert.deepEqual(rxn.metabolites, { [met]: 1 });
  }
  for (const [met, rxnId] of model.communityExport) {
    const rxn = model.reactions.get(rxnId);
    assert.equal(rxn.species, 'comm');
    assert.deepEqual(rxn.metabolites, { [met]: -1 });
  }

  // Species <-> medium: one species metabolite out, one community metabolite in,
  // so a positive flux is secretion into the shared medium.
  for (const [key, rxnId] of model.speciesExchange) {
    const met = key.slice(0, key.indexOf('\u0000'));
    const speciesId = key.slice(key.indexOf('\u0000') + 1);
    const rxn = model.reactions.get(rxnId);
    assert.ok(rxn, `${rxnId} is missing`);
    assert.equal(rxn.species, speciesId, `${rxnId} should belong to ${speciesId}`);
    assert.equal(rxn.metabolites[met], 1, `${rxnId} should produce ${met}`);
    const other = Object.entries(rxn.metabolites).filter(([m]) => m !== met);
    assert.equal(other.length, 1, `${rxnId} should touch one species metabolite`);
    assert.equal(other[0][1], -1);
    assert.equal(model.metabolites.get(other[0][0])?.species, speciesId);
  }

  // Every community metabolite is reachable by at least one species.
  const traded = new Set([...model.speciesExchange.keys()]
    .map((k) => k.slice(0, k.indexOf('\u0000'))));
  assert.ok(traded.size > 0);
  for (const met of traded) assert.ok(model.communityUptake.has(met));
});

test('the Figure 4 reactions all exist and carry a carbon number', (t) => {
  if (!loaded) return t.skip(SKIP);
  const { model } = loaded;
  const metaboliteOf = exchangeMetaboliteOf(model);

  for (const [list, speciesId] of [[BT_FIBER, 'Bt'], [BT_CARBOHYDRATE, 'Bt'], [FP_FIBER, 'Fp']]) {
    for (const rid of list) {
      const rxn = model.reactions.get(rid);
      assert.ok(rxn, `${rid} is not in the model`);
      assert.equal(rxn.species, speciesId);
      const met = metaboliteOf.get(rid);
      assert.ok(met, `${rid} is not a species-community exchange reaction`);
      assert.ok(CARBON_NUMBER[met] > 0, `${met} has no carbon number in simFigure4.m`);
    }
  }

  // The constraint the sweep imposes, and the eleven targets it reports.
  const constraints = fiberConstraints(model, { bacteroides: 10, faecalibacterium: 5 });
  assert.equal(constraints.length, 2);
  assert.equal(constraints[0].terms.length, BT_FIBER.length);
  assert.equal(constraints[1].terms.length, FP_FIBER.length);
  // Only the uptake side is constrained — the script's MCmode = 2.
  assert.ok(constraints.every((c) => c.terms.every((term) => term.part === 'uptake')));
  assert.deepEqual(constraints[0].biomass, [{ species: 'Bt', coef: -1 }]);
  assert.equal(constraints[0].hi, 0);
  // C_i / C_max, as the script writes it.
  const pect = constraints[0].terms.find((term) => term.reaction === 'EX_pect[u]tr_Bt');
  assert.equal(pect.coef, CARBON_NUMBER['pect[u]'] / 10);

  const targets = figureTargets(model);
  assert.equal(targets.length, EXPECTED.species + 2);
  assert.deepEqual(targets.slice(0, 9).map((x) => x.name),
    model.species.map((s) => `Bm_${s.id}`));
  assert.deepEqual(targets.slice(9).map((x) => x.name),
    ['fiber_Bt_Cmmol', 'carbohydrate_Bt_Cmmol']);
});

// The solve, which is slow

test('the community is feasible and grows on the authors\' diet', async (t) => {
  if (!loaded) return t.skip(SKIP);
  if (process.env.COXETER_REPRODUCE !== '1') {
    return t.skip('set COXETER_REPRODUCE=1 to solve it (minutes, 28 000-row LPs)');
  }
  const { initGLPK } = await import('../src/solver/glpk.js');
  const { maxTotalBiomassAt } = await import('../src/core/steadycom.js');
  assert.ok(await initGLPK(), 'GLPK is required for a nine-species genome-scale community');

  const spec = toSteadyComSpec(loaded.model);
  // Feasible at a growth rate the paper's Figure 4 reports, and that is the
  // claim worth making here: one LP rather than a whole bisection.
  const at = maxTotalBiomassAt(spec, 0.05);
  assert.equal(at.status, 'optimal', at.message);
  assert.ok(Math.abs(at.total - 1) < 1e-6, `total biomass ${at.total}`);
});

test('the sweep point the figure plots reproduces from the recorded run', async (t) => {
  // The recorded sweep is the artefact; this checks it rather than re-solving.
  let recorded;
  try {
    recorded = JSON.parse(await readFile(
      new URL('../results/gut9-figure4.json', import.meta.url), 'utf8'));
  } catch {
    return t.skip('results/gut9-figure4.json not present; run tools/sweep-figure4.mjs');
  }

  assert.equal(recorded.sweep.faecalibacterium, FIBER_SWEEP.faecalibacterium);
  assert.ok(recorded.results.length > 0);

  for (const row of recorded.results) {
    assert.ok(FIBER_SWEEP.bacteroides.includes(row.bacteroides),
      `${row.bacteroides} is outside the sweep simFigure4.m defines`);
    assert.equal(row.status, undefined, `FUR ${row.bacteroides} did not solve`);
    // Figure 4B's axis runs to 0.082 /h.
    assert.ok(row.growthRate > 0 && row.growthRate < 0.082,
      `FUR ${row.bacteroides}: mu = ${row.growthRate}`);
    // The abundances are relative, so they sum to one.
    const total = Object.values(row.abundances).reduce((s, x) => s + x, 0);
    assert.ok(Math.abs(total - 1) < 1e-4, `FUR ${row.bacteroides}: abundances sum to ${total}`);
    // Each member's minimum biomass production is growth rate times abundance,
    // so it cannot exceed the growth rate itself.
    for (const [id, value] of Object.entries(row.biomassMin)) {
      assert.ok(value <= row.growthRate + 1e-9, `FUR ${row.bacteroides}, ${id}: ${value}`);
    }
    // Uptake cannot exceed what the constraint allows: C-mmol per hour at a
    // total biomass of one gram, so at most the specific rate being swept.
    assert.ok(row.fiberUptake <= row.bacteroides + 1e-6,
      `FUR ${row.bacteroides}: fiber uptake ${row.fiberUptake} exceeds the cap`);

    // A point where an FVA target never solved is a point with a hole in it.
    // It is recorded rather than written as null, so the test can refuse it.
    assert.deepEqual(row.unsolved ?? [], [],
      `FUR ${row.bacteroides}: targets that never solved`);
    for (const key of ['growthRate', 'fiberUptake', 'fdsExport']) {
      assert.ok(Number.isFinite(row[key]), `FUR ${row.bacteroides}: ${key} is ${row[key]}`);
    }
  }

  // And the file has to say which of the thirty points it actually holds.
  assert.ok(Array.isArray(recorded.computedPoints), 'the merged sweep must list its points');
  assert.deepEqual(recorded.computedPoints, recorded.results.map((r) => r.bacteroides));
  assert.deepEqual(
    [...recorded.computedPoints, ...recorded.missingPoints].sort((a, b) => a - b),
    [...FIBER_SWEEP.bacteroides],
    'every point of the sweep is either computed or named as missing',
  );
});
