/**
 * The Stolyar 2007 two-species model.
 *
 * The supplement is not in this repository — it is the paper's, and it reached
 * this project by hand — so the tests that need it are opt-in:
 *
 *   COXETER_STOLYAR_DIR=/path/to/supplement npm test
 *
 * What runs without it is everything that can be checked on a small model
 * written here: the file format, the sign convention of the macromolecule file,
 * and the rules by which a reaction is assigned to an organism. Those are where
 * a parser goes wrong quietly, and they do not need 180 reactions to show it.
 */
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join } from 'node:path';

import {
  importStolyar, validateStolyar, parseEquation, parseTerms,
  parseMacromoleculeSynthesis, toJointCobraJSON, toSpeciesCobraJSON,
  exchangeStructure, reactionsBySpecies, externalMetabolites, objectiveOf, EXPECTED, SPECIES,
} from '../src/communities/stolyar.js';
import { Model } from '../src/core/model.js';
import { FluxProblem } from '../src/solver/fba.js';
import { CommunityModel } from '../src/core/community.js';

// The file format

test('a term keeps an identifier that contains a plus sign', () => {
  // `H+` and `ZM_H+` are real metabolites here. Splitting on the character
  // rather than on " + " turns one of them into two, and the model still looks
  // plausible afterwards, which is what makes it worth pinning.
  assert.deepEqual(parseTerms('6 H+ + 1 H2S'), [
    { coefficient: 6, metabolite: 'H+' },
    { coefficient: 1, metabolite: 'H2S' },
  ]);
  assert.deepEqual(parseTerms('1 ATP + 1 SO4 + 1 H+').map((t) => t.metabolite),
    ['ATP', 'SO4', 'H+']);
  assert.deepEqual(parseTerms('1 Fdx(red)'), [{ coefficient: 1, metabolite: 'Fdx(red)' }]);
  assert.deepEqual(parseTerms(''), []);
});

test('an equation reads substrates negative and products positive', () => {
  assert.deepEqual(parseEquation('1 GAP + 1 ADP = 1 3PG + 1 ATP + 1 NADH'),
    { GAP: -1, ADP: -1, '3PG': 1, ATP: 1, NADH: 1 });
  // Either side may be empty: that is how this format writes a drain or a source.
  assert.deepEqual(parseEquation('1 H2(exch) = '), { 'H2(exch)': -1 });
  assert.deepEqual(parseEquation(' = 1 ZM_Ala'), { ZM_Ala: 1 });
  assert.deepEqual(parseEquation(''), {});
  assert.deepEqual(parseEquation('0.5 ATP = 0.5 ADP + 1 Lac'), { ATP: -0.5, ADP: 0.5, Lac: 1 });
});

test('the macromolecule file reads consumed as positive', () => {
  // The opposite of the usual convention, and the only reason to be sure of it
  // is that the methanogen's own synthesis reactions say the same thing the
  // ordinary way round. MC_RNA consumes 6.17 Asp and produces 3.47 fum; this
  // file writes `6.17 Asp` and `-3.47 fum` for the same thing.
  const parsed = parseMacromoleculeSynthesis('RNA = 6.17 Asp + -3.47 fum + 32.41 ATP\n');
  assert.equal(parsed.get('RNA').Asp, -6.17, 'a positive coefficient is consumed');
  assert.equal(parsed.get('RNA').fum, 3.47, 'a negative coefficient is produced');
  assert.equal(parsed.get('RNA').ATP, -32.41);
});

// The model, on a miniature of it

const TINY = {
  // Two organisms, one shared metabolite, one macromolecule each side.
  reactions: [
    'Dsrc      \t = 1 S \t| \t# \t0  10    0 \t0 0 1 1 \t0.01',
    'Dmake     \t1 S = 1 A + 1 SHARED \t| \t# \t0  100    0 \t0 0 1 1 \t0.01',
    'mue       \t \t \t| \t# \t0  100   -1 \t0 0 1 1 \t0.01',
    'Mtake     \t1 SHARED = 1 ZM_A \t| \t# \t0  100    0 \t0 0 3 1 \t0.01',
    'MC_BIO    \t1 ZM_A = 1 ZM_BIOMASS \t| \t# \t0  100   -0.1 \t0 0 3 1 \t0.01',
    'Sink      \t1 SHARED =  \t| \t0 \t0  100    0 \t0 0 3 1 \t0.01',
  ].join('\n'),
  metabolites: [
    'S \tsubstrate \t0.001 \t0',
    'A \tproduct \t0.001 \t0',
    'SHARED \tshared \t0.001 \t0',
    'ZM_A \tmethanogen product \t0.001 \t0',
    'ZM_BIOMASS \tmethanogen biomass \t0.001 \t1',
  ].join('\n'),
  macromolecules: 'BIO \tBiomass \t1 \t0 0 2 1',
  synthesis: 'BIO    = 2 A',
};

test('a miniature model splits, shares and assembles its biomass', () => {
  const model = importStolyar(TINY);

  // The macromolecule becomes a metabolite, its synthesis a reaction, and the
  // empty growth reaction gains the fractions.
  assert.ok(model.metabolites.has('BIO'));
  assert.deepEqual(model.reactions.get('BIO_syn').metabolites, { A: -2, BIO: 1 });
  assert.deepEqual(model.reactions.get('mue').metabolites, { BIO: -1 });

  // Shared is derived: both organisms' reactions touch SHARED, nothing else.
  assert.deepEqual([...model.shared], ['SHARED']);

  const { bySpecies, community } = reactionsBySpecies(model);
  assert.deepEqual(bySpecies.get('Dv').sort(), ['BIO_syn', 'Dmake', 'Dsrc', 'mue']);
  assert.deepEqual(bySpecies.get('Mm').sort(), ['MC_BIO', 'Mtake']);
  // A reaction touching nothing but the shared pool belongs to the pair.
  assert.deepEqual(community, ['Sink']);

  // CellNetAnalyzer minimises, this project maximises, so the signs flip.
  assert.deepEqual(objectiveOf(model), { mue: 1, MC_BIO: 0.1 });

  // The measured-rate column is a constraint: Sink is measured at 0.
  const joint = toJointCobraJSON(model);
  const sink = joint.reactions.find((r) => r.id === 'Sink');
  assert.equal(sink.lower_bound, 0);
  assert.equal(sink.upper_bound, 0);

  // Every external metabolite gains exactly one exchange, and nothing else is added.
  assert.deepEqual(externalMetabolites(model), ['ZM_BIOMASS']);
  assert.ok(joint.reactions.some((r) => r.id === 'EX_ZM_BIOMASS'));
  assert.equal(joint.reactions.length, model.reactions.size + 1);
});

test('a miniature model solves, and both partners grow only together', () => {
  const model = importStolyar(TINY);
  const joint = new Model(toJointCobraJSON(model));
  const solution = new FluxProblem(joint).optimize();
  assert.equal(solution.status, 'optimal');
  assert.ok(solution.fluxes.get('mue') > 0, 'D. vulgaris side must grow');
  assert.ok(solution.fluxes.get('MC_BIO') > 0, 'the methanogen side must grow');

  // Cut the one link between them and the methanogen stops.
  const cut = new FluxProblem(joint);
  cut.setBounds('Mtake', 0, 0);
  const after = cut.optimize();
  assert.equal(after.fluxes.get('MC_BIO'), 0, 'with the shared metabolite cut, nothing feeds it');
});

test('the split members keep the shared pool outside the cells', () => {
  const model = importStolyar(TINY);
  const species = SPECIES.map((sp) => ({
    id: sp.id,
    model: new Model(toSpeciesCobraJSON(model, sp.id)),
    abundance: 0.5,
    enabled: true,
  }));
  const community = new CommunityModel({
    name: 'tiny', objective: 'equal-growth', species, medium: {},
  });

  // Each member carries its own copy of SHARED plus an exchange for it, which is
  // what lets the community layer build one medium between the cells. If the
  // shared metabolite stayed inside a member, it would be drawn inside that cell.
  for (const sp of SPECIES) {
    const doc = toSpeciesCobraJSON(model, sp.id);
    assert.ok(doc.reactions.some((r) => r.id === `EX_SHARED_${sp.id}`),
      `${sp.id} needs an exchange for the shared metabolite`);
    assert.equal(doc.metabolites.find((m) => m.id === 'SHARED')?.compartment, 'e');
  }
  const tree = community.merged.compartmentTree();
  assert.equal(tree.byId.get('s')?.depth, 0, 'the shared medium sits outside every cell');
  assert.ok(community.transfersByBase.has('SHARED'));
});

test('a reaction whose layer and prefix disagree is refused', () => {
  // The layer column is what the split is built on. If it ever stopped agreeing
  // with the ZM_ prefix, every assignment would be wrong and nothing downstream
  // would notice, so the import stops instead.
  const broken = {
    ...TINY,
    reactions: TINY.reactions.replace(
      'MC_BIO    \t1 ZM_A = 1 ZM_BIOMASS \t| \t# \t0  100   -0.1 \t0 0 3 1 \t0.01',
      'MC_BIO    \t1 ZM_A = 1 ZM_BIOMASS \t| \t# \t0  100   -0.1 \t0 0 1 1 \t0.01',
    ),
  };
  assert.throws(() => importStolyar(broken), /D\. vulgaris layer but names ZM_/);
});

test('a reaction naming an undeclared metabolite is refused', () => {
  const broken = { ...TINY, reactions: `${TINY.reactions}\nBad \t1 S = 1 GHOST \t| \t# \t0 100 0 \t0 0 1 1 \t0.01` };
  assert.throws(() => importStolyar(broken), /GHOST/);
});

// The paper's own files

const dir = process.env.COXETER_STOLYAR_DIR;
const supplement = dir ? {
  reactions: await readFile(join(dir, 'reactions.txt'), 'utf8'),
  metabolites: await readFile(join(dir, 'metabolites.txt'), 'utf8'),
  macromolecules: await readFile(join(dir, 'macromolecules.txt'), 'utf8'),
  synthesis: await readFile(join(dir, 'macromolecule_synthesis.txt'), 'utf8'),
} : null;

test('the supplement imports with the counts it should', { skip: !supplement }, () => {
  const model = importStolyar(supplement);
  const report = validateStolyar(model);
  assert.deepEqual(report.problems, []);
  assert.equal(model.reactions.size - model.macromolecules.size, EXPECTED.reactions);
  assert.equal(model.shared.size, EXPECTED.sharedMetabolites);
});

test('the shared metabolites are the four the mutualism runs on', { skip: !supplement }, () => {
  const model = importStolyar(supplement);
  // Derived from which organism touches what, not listed anywhere in the files.
  assert.deepEqual([...model.shared].sort(),
    ['AcOH(exch)', 'Form(exch)', 'H2(exch)', 'NH3(ext)']);
  const hydrogen = exchangeStructure(model).find((x) => x.metabolite === 'H2(exch)');
  assert.ok(hydrogen.producers.includes('Dv'), 'D. vulgaris releases the hydrogen');
  assert.ok(hydrogen.consumers.includes('Mm'), 'the methanogen takes it');
});

test('without sulfate the partnership is obligate; with it, it is not',
  { skip: !supplement }, () => {
    const model = importStolyar(supplement);
    const joint = new Model(toJointCobraJSON(model));

    const syntrophic = new FluxProblem(joint);
    syntrophic.setBounds('EX_SO4', 0, 1000);
    const without = syntrophic.optimize();
    assert.equal(without.status, 'optimal');
    assert.ok(without.fluxes.get('mue') > 0, 'D. vulgaris grows');
    assert.ok(without.fluxes.get('MC_BIO') > 0, 'so does the methanogen');
    assert.ok(without.fluxes.get('M12') > 0, 'and it makes methane');
    assert.equal(without.fluxes.get('D01'), 0, 'with no sulfate, none is reduced');

    // Give D. vulgaris an acceptor of its own and it stops feeding its partner.
    const respiring = new FluxProblem(joint);
    respiring.setBounds('EX_SO4', -1000, 1000);
    const with_ = respiring.optimize();
    assert.ok(with_.fluxes.get('D01') > 0, 'sulfate is reduced when it is there');
    assert.equal(with_.fluxes.get('MC_BIO'), 0, 'and the methanogen does not grow');
    assert.ok(with_.fluxes.get('mue') > without.fluxes.get('mue'),
      'D. vulgaris does better alone than in the partnership');
  });

test('the carrier is not determined, and the growth rates are',
  { skip: !supplement }, () => {
    // Worth a test of its own because the recorded result states it: the two
    // growth rates and the methane are unique at the optimum, but hydrogen and
    // formate trade off against each other freely.
    const model = importStolyar(supplement);
    const joint = new Model(toJointCobraJSON(model));
    const problem = new FluxProblem(joint);
    problem.setBounds('EX_SO4', 0, 1000);
    problem.optimize();

    const ranges = problem.variability(['mue', 'MC_BIO', 'M12', 'D81', 'D80'], { fraction: 1 });
    const span = (id) => ranges.get(id).max - ranges.get(id).min;
    assert.ok(span('mue') < 1e-6, 'the D. vulgaris growth rate is pinned');
    assert.ok(span('MC_BIO') < 1e-6, 'so is the methanogen\'s');
    assert.ok(span('M12') < 1e-6, 'and the methane');
    assert.ok(span('D81') > 1, 'the hydrogen transfer is free to move');
    assert.ok(span('D80') > 1, 'and the formate takes up the slack');
  });
