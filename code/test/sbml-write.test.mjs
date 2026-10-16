/**
 * SBML L3V1 + fbc-v2 writer.
 *
 * The round trip is the test that matters: every model this repository ships is
 * written to SBML, read back with the project's own reader, and compared
 * reaction by reaction. A serializer can look right and still drop a charge, a
 * bound or a sign, and only the comparison catches that.
 *
 * The published genome-scale members are the interesting half — their ids carry
 * the punctuation SBML forbids (`12ethd[c_Ba]`, `PFK(ppi)_Bt`, `12ppd-R_e`), so
 * they exercise the escaping that the small teaching models never reach.
 */
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

import { Model } from '../src/core/model.js';
import { toSBML, parseGPR, genesInGPR, mangleId } from '../src/core/sbml-write.js';
import { parseCOBRASBML, unmangleId } from '../src/core/sbml.js';
import { parseModelFile } from '../src/core/model-import.js';

const root = fileURLToPath(new URL('..', import.meta.url));

/** Stoichiometry is a map, so compare it as one: key order carries no meaning. */
const sameStoichiometry = (a, b) => {
  const ka = Object.keys(a);
  return ka.length === Object.keys(b).length && ka.every((k) => a[k] === b[k]);
};

function roundTrip(doc) {
  const before = new Model(doc);
  const xml = toSBML(before);
  // Faithful ids: the reader's default rewrites a trailing `[c_Ba]` to `_c_Ba`,
  // which is a deliberate normalisation and not invertible. See unmangleId.
  const after = new Model(parseCOBRASBML(xml, { normaliseCompartment: false }));
  return { before, after, xml };
}

function assertSameModel(before, after, label) {
  assert.equal(after.reactions.size, before.reactions.size, `${label}: reaction count`);
  assert.equal(after.metabolites.size, before.metabolites.size, `${label}: metabolite count`);

  for (const rxn of before.reactions.values()) {
    const back = after.reactions.get(rxn.id);
    assert.ok(back, `${label}: reaction ${rxn.id} is missing`);
    assert.ok(sameStoichiometry(rxn.metabolites, back.metabolites),
      `${label}: ${rxn.id} stoichiometry`);
    assert.equal(back.lowerBound, rxn.lowerBound, `${label}: ${rxn.id} lower bound`);
    assert.equal(back.upperBound, rxn.upperBound, `${label}: ${rxn.id} upper bound`);
    assert.equal(back.name, rxn.name, `${label}: ${rxn.id} name`);
  }

  for (const met of before.metabolites.values()) {
    const back = after.metabolites.get(met.id);
    assert.ok(back, `${label}: metabolite ${met.id} is missing`);
    assert.equal(back.formula, met.formula, `${label}: ${met.id} formula`);
    assert.equal(back.charge, met.charge, `${label}: ${met.id} charge`);
    assert.equal(back.compartment, met.compartment, `${label}: ${met.id} compartment`);
  }

  assert.deepEqual(after.objectiveReactions(), before.objectiveReactions(),
    `${label}: objective`);
}

// Identifiers

test('identifiers survive the trip into SBML and back', () => {
  // Real ids out of the published models: a leading digit, brackets, parens and
  // a dash are all things an SId may not contain.
  for (const id of ['12ethd[c_Ba]', 'EX_12ppd-R[u]tr_Bt', 'PFK(ppi)_Bt', '12ppd-R_e',
    'EX_glc__D_e', 'BIOMASS_Ec_iAF1260_core_59p81M', "3'5'-cyclic"]) {
    const written = mangleId(id, 'M_');
    assert.match(written, /^[A-Za-z_][A-Za-z0-9_]*$/, `${id} must become a valid SId`);
    assert.equal(unmangleId(written, 'M_', { normaliseCompartment: false }), id);
  }
});

test('the reader also decodes the COBRA Toolbox spelling', () => {
  // Files in the wild use `_DASH_`/`_LPAREN_` rather than the character codes.
  assert.equal(unmangleId('M_12ppd_DASH_R_LSQBKT_u_RSQBKT_', 'M_', { normaliseCompartment: false }),
    '12ppd-R[u]');
  assert.equal(unmangleId('R_EX_glc_LPAREN_e_RPAREN_', 'R_', { normaliseCompartment: false }),
    'EX_glc(e)');
});

test('normalising the compartment suffix is documented as one-way', () => {
  // Pinned on purpose: this is the one id shape that does not come back, and it
  // is why the round-trip tests ask for faithful ids.
  assert.equal(unmangleId('M_12ethd__91__c_Ba__93__', 'M_'), '12ethd_c_Ba');
  assert.equal(unmangleId('M_12ethd__91__c_Ba__93__', 'M_', { normaliseCompartment: false }),
    '12ethd[c_Ba]');
});

// GPR

test('gene rules parse with `or` binding loosest', () => {
  assert.deepEqual(parseGPR('b0001'), { gene: 'b0001' });
  assert.deepEqual(parseGPR('a and b or c'), {
    op: 'or',
    args: [{ op: 'and', args: [{ gene: 'a' }, { gene: 'b' }] }, { gene: 'c' }],
  });
  assert.deepEqual(parseGPR('a or b and c'), {
    op: 'or',
    args: [{ gene: 'a' }, { op: 'and', args: [{ gene: 'b' }, { gene: 'c' }] }],
  });
  assert.deepEqual(parseGPR('(a or b) and c'), {
    op: 'and',
    args: [{ op: 'or', args: [{ gene: 'a' }, { gene: 'b' }] }, { gene: 'c' }],
  });
  assert.equal(parseGPR(''), null);
  assert.equal(parseGPR('   '), null);
  assert.deepEqual(genesInGPR('(a and b) or a'), ['a', 'b']);
});

test('gene rules survive the trip through fbc:geneProductAssociation', () => {
  // No model in this repository carries a GPR, so this is the only place the
  // association is exercised against data.
  const doc = {
    id: 'gpr', name: 'GPR probe',
    compartments: { c: 'Cytosol' },
    metabolites: [
      { id: 'a_c', name: 'A', compartment: 'c', formula: 'C', charge: 0 },
      { id: 'b_c', name: 'B', compartment: 'c', formula: 'C', charge: 0 },
    ],
    reactions: [
      {
        id: 'R1', name: 'R one', metabolites: { a_c: -1, b_c: 1 },
        lower_bound: 0, upper_bound: 1000,
        gene_reaction_rule: '(b0001 and b0002) or b0003',
      },
      {
        id: 'R2', name: 'R two', metabolites: { b_c: -1 },
        lower_bound: -1000, upper_bound: 1000,
        gene_reaction_rule: 'b0004',
      },
    ],
    genes: [{ id: 'b0001', name: 'gene one' }],
    objective: { R1: 1 },
  };

  const { before, after, xml } = roundTrip(doc);
  assert.match(xml, /<fbc:geneProductAssociation>/);
  assert.match(xml, /<fbc:listOfGeneProducts>/);
  assertSameModel(before, after, 'gpr');

  // The rule comes back as an equivalent tree, which is what is preserved —
  // the exact spacing of the original string is not.
  assert.deepEqual(
    parseGPR(after.reactions.get('R1').geneReactionRule),
    parseGPR('(b0001 and b0002) or b0003'),
  );
  assert.equal(after.reactions.get('R2').geneReactionRule, 'b0004');
  assert.deepEqual(after.genes.map((g) => g.id).sort(), ['b0001', 'b0002', 'b0003', 'b0004']);
  assert.equal(after.genes.find((g) => g.id === 'b0001').name, 'gene one');
});

// Document

test('the document is Level 3 Version 1 with fbc-v2, and strict', () => {
  const model = new Model(JSON.parse(
    readFileSync(join(root, 'models/ecoli_core_mc.json'), 'utf8')));
  const xml = toSBML(model);

  assert.match(xml, /^<\?xml version="1\.0" encoding="UTF-8"\?>/);
  assert.match(xml, /xmlns="http:\/\/www\.sbml\.org\/sbml\/level3\/version1\/core"/);
  assert.match(xml, /xmlns:fbc="http:\/\/www\.sbml\.org\/sbml\/level3\/version1\/fbc\/version2"/);
  assert.match(xml, /level="3" version="1" fbc:required="false"/);
  assert.match(xml, /fbc:strict="true"/);

  // Core requires these on every species and speciesReference; a file without
  // them is rejected outright rather than read with defaults.
  for (const required of ['hasOnlySubstanceUnits=', 'boundaryCondition=', 'constant=']) {
    assert.ok(xml.includes(required), `species must carry ${required}`);
  }
  assert.match(xml, /<speciesReference[^>]*constant="true"/);
  assert.match(xml, /<reaction[^>]*fast="false"/);

  // Bounds are parameters, pooled by value: a model with thousands of reactions
  // and three distinct bounds writes three parameters, not thousands.
  assert.match(xml, /<parameter[^>]*id="cobra_default_lb"[^>]*value="-1000"/);
  assert.match(xml, /<parameter[^>]*sboTerm="SBO:0000626"/);
  const parameters = (xml.match(/<parameter\b/g) ?? []).length;
  assert.ok(parameters < model.reactions.size,
    `expected pooled bounds, got ${parameters} parameters for ${model.reactions.size} reactions`);

  assert.match(xml, /<fbc:listOfObjectives fbc:activeObjective="obj">/);
  assert.match(xml, /<fbc:objective fbc:id="obj" fbc:type="maximize">/);
  assert.match(xml, /<fbc:fluxObjective[^>]*fbc:coefficient="1"/);
});

test('names and formulas are XML-escaped', () => {
  const { xml, before, after } = roundTrip({
    id: 'esc', compartments: { c: 'C' },
    metabolites: [{ id: 'm_c', name: 'a < b & "c"', compartment: 'c', formula: 'CHO', charge: 0 }],
    reactions: [{ id: 'R', name: 'x > y & z', metabolites: { m_c: -1 }, lower_bound: 0, upper_bound: 1 }],
    objective: { R: 1 },
  });
  assert.ok(!/name="[^"]*[<&]/.test(xml.replace(/&(amp|lt|gt|quot);/g, '')),
    'no raw markup may survive inside an attribute');
  assertSameModel(before, after, 'escaping');
  assert.equal(after.metabolites.get('m_c').name, 'a < b & "c"');
});

test('a formula fbc cannot hold is kept in the notes instead', () => {
  // Both of these are real: the first is a biomass pseudo-metabolite from the
  // nine-species gut model, the second an acyl-carrier placeholder. libSBML
  // rejects either as fbc:chemicalFormula, so they travel as COBRA notes.
  const doc = {
    id: 'formulas', compartments: { c: 'Cytosol' },
    metabolites: [
      { id: 'bio_c', name: 'Biomass', compartment: 'c', charge: 0,
        formula: 'C37.024041172583H67.803828393317O20.609573521004' },
      { id: 'acp_c', name: 'ACP', compartment: 'c', charge: -1, formula: 'C11H20O6N2PSApo_acp' },
      { id: 'glc_c', name: 'Glucose', compartment: 'c', charge: 0, formula: 'C6H12O6' },
    ],
    reactions: [{
      id: 'R', name: 'R', metabolites: { bio_c: -1, acp_c: -1, glc_c: 1 },
      lower_bound: 0, upper_bound: 1000,
    }],
    objective: { R: 1 },
  };

  const { before, after, xml } = roundTrip(doc);
  assert.match(xml, /fbc:chemicalFormula="C6H12O6"/, 'a conformant formula stays an attribute');
  assert.ok(!xml.includes('fbc:chemicalFormula="C11H20O6N2PSApo_acp"'),
    'an underscore may not reach fbc:chemicalFormula');
  assert.match(xml, /FORMULA: C11H20O6N2PSApo_acp/);
  assertSameModel(before, after, 'formulas');
});

// Round trips

const catalogue = readdirSync(join(root, 'models'))
  .filter((f) => f.endsWith('.json') && !['index.json', 'scenarios.json'].includes(f));

for (const file of catalogue) {
  test(`${file} survives the round trip through SBML`, () => {
    const doc = JSON.parse(readFileSync(join(root, 'models', file), 'utf8'));
    const { before, after } = roundTrip(doc);
    assertSameModel(before, after, file);
  });
}

const publishedDir = join(root, 'models', 'published');
const published = existsSync(publishedDir)
  ? readdirSync(publishedDir).flatMap((dir) => readdirSync(join(publishedDir, dir))
    .filter((f) => f.startsWith('model-'))
    .map((f) => join(publishedDir, dir, f)))
  : [];

for (const path of published) {
  const label = path.slice(publishedDir.length + 1);
  test(`published ${label} survives the round trip through SBML`, () => {
    const doc = JSON.parse(readFileSync(path, 'utf8'));
    const { before, after } = roundTrip(doc);
    assertSameModel(before, after, label);
  });
}

test('an SBML file this app writes can be uploaded back into it', () => {
  // The upload path is a different reader from the one the round trips use, so
  // it gets its own check: without un-mangling, every identifier would arrive
  // renamed (`R_GLCptspp`, `M_13dpg_c`) and still look like a working model.
  const doc = JSON.parse(readFileSync(join(root, 'models/ecoli_core_mc.json'), 'utf8'));
  const before = new Model(doc);
  const uploaded = parseModelFile(toSBML(before), 'uploaded.xml');

  assert.equal(uploaded.reactions.size, before.reactions.size);
  assert.equal(uploaded.metabolites.size, before.metabolites.size);
  for (const rxn of before.reactions.values()) {
    const back = uploaded.reactions.get(rxn.id);
    assert.ok(back, `reaction ${rxn.id} arrived under a different id`);
    assert.ok(sameStoichiometry(rxn.metabolites, back.metabolites), `${rxn.id} stoichiometry`);
    assert.equal(back.lowerBound, rxn.lowerBound);
    assert.equal(back.upperBound, rxn.upperBound);
  }
  for (const met of before.metabolites.values()) {
    const back = uploaded.metabolites.get(met.id);
    assert.ok(back, `metabolite ${met.id} arrived under a different id`);
    assert.equal(back.formula, met.formula);
    assert.equal(back.charge, met.charge);
    assert.equal(back.compartment, met.compartment);
  }
  assert.deepEqual(uploaded.objectiveReactions(), before.objectiveReactions());
});

test('uploading carries gene rules and odd formulas across', () => {
  const doc = {
    id: 'upload_probe', compartments: { c: 'Cytosol' },
    metabolites: [
      { id: 'a_c', name: 'A', compartment: 'c', formula: 'C11H20O6N2PSApo_acp', charge: -1 },
      { id: 'b_c', name: 'B', compartment: 'c', formula: 'C6H12O6', charge: 0 },
    ],
    reactions: [
      {
        id: '12ppd-R_t', name: 'Transport', metabolites: { a_c: -1, b_c: 1 },
        lower_bound: -10, upper_bound: 1000,
        gene_reaction_rule: '(b0001 and b0002) or b0003',
      },
      { id: 'EX_b(e)', name: 'B exchange', metabolites: { b_c: -1 }, lower_bound: -10, upper_bound: 1000 },
    ],
    objective: { '12ppd-R_t': 1 },
  };
  const uploaded = parseModelFile(toSBML(new Model(doc)), 'probe.xml');

  // A dash and a leading digit are both things SBML forbids in an id.
  const rxn = uploaded.reactions.get('12ppd-R_t');
  assert.ok(rxn, 'the punctuated reaction id must survive the upload');
  assert.deepEqual(parseGPR(rxn.geneReactionRule), parseGPR('(b0001 and b0002) or b0003'));
  assert.equal(uploaded.metabolites.get('a_c').formula, 'C11H20O6N2PSApo_acp');
  assert.deepEqual(uploaded.objectiveReactions(), [{ id: '12ppd-R_t', coefficient: 1 }]);

  // Pinned, because it is a rename and not a bug: an upload normalises the
  // compartment suffix, so `EX_b(e)`, `EX_b[e]` and `EX_b_e` all arrive as one
  // id. That is what lines a BiGG file up with this app's medium handling.
  assert.ok(uploaded.reactions.has('EX_b_e'));
  assert.ok(!uploaded.reactions.has('EX_b(e)'));
});

test('writing is idempotent', () => {
  // Reading what we wrote and writing it again has to produce the same bytes,
  // or something in the file depends on the order it was built in.
  const doc = JSON.parse(readFileSync(join(root, 'models/methanogen_core_mc.json'), 'utf8'));
  const once = toSBML(new Model(doc));
  const twice = toSBML(new Model(parseCOBRASBML(once, { normaliseCompartment: false })));
  assert.equal(twice, once);
});
