/**
 * Reusable pathway blocks.
 *
 * Four of the five demo organisms run glycolysis and the pentose phosphate
 * pathway, so the stoichiometry is written once and instantiated per
 * compartment. In every block `comp` is the suffix of the compartment the block
 * lives in, `suffix` is appended to each reaction id so a block can appear
 * twice in one model, and `lane`/`rank0` place it on the map. Each returns an
 * array of r() declarations.
 */
import { r } from './dsl.mjs';

const s = (id, c) => `${id}_${c}`;

/**
 * Lower glycolysis / Embden–Meyerhof–Parnas, from g6p to pyruvate.
 * `includeFBP` (default true) adds fructose-1,6-bisphosphatase.
 */
export function glycolysis({ comp = 'c', lane = 0, rank0 = 1, suffix = '', includeFBP = true } = {}) {
  const x = (id) => s(id, comp);
  const n = (id) => `${id}${suffix}`;
  const out = [
    r(n('PGI'), `${x('g6p')} <=> ${x('f6p')}`, { name: 'Glucose-6-phosphate isomerase', sub: 'Glycolysis', rank: rank0, lane }),
    r(n('PFK'), `${x('atp')} + ${x('f6p')} --> ${x('adp')} + ${x('fdp')} + ${x('h')}`, { name: 'Phosphofructokinase', sub: 'Glycolysis', rank: rank0 + 1, lane }),
    r(n('FBA'), `${x('fdp')} <=> ${x('dhap')} + ${x('g3p')}`, { name: 'Fructose-bisphosphate aldolase', sub: 'Glycolysis', rank: rank0 + 2, lane }),
    r(n('TPI'), `${x('dhap')} <=> ${x('g3p')}`, { name: 'Triose-phosphate isomerase', sub: 'Glycolysis', rank: rank0 + 3, lane: lane - 1 }),
    r(n('GAPD'), `${x('g3p')} + ${x('nad')} + ${x('pi')} <=> ${x('13dpg')} + ${x('h')} + ${x('nadh')}`, { name: 'Glyceraldehyde-3-phosphate dehydrogenase', sub: 'Glycolysis', rank: rank0 + 4, lane }),
    r(n('PGK'), `${x('3pg')} + ${x('atp')} <=> ${x('13dpg')} + ${x('adp')}`, { name: 'Phosphoglycerate kinase', sub: 'Glycolysis', rank: rank0 + 5, lane }),
    r(n('PGM'), `${x('2pg')} <=> ${x('3pg')}`, { name: 'Phosphoglycerate mutase', sub: 'Glycolysis', rank: rank0 + 6, lane }),
    r(n('ENO'), `${x('2pg')} <=> ${x('h2o')} + ${x('pep')}`, { name: 'Enolase', sub: 'Glycolysis', rank: rank0 + 7, lane }),
    r(n('PYK'), `${x('adp')} + ${x('h')} + ${x('pep')} --> ${x('atp')} + ${x('pyr')}`, { name: 'Pyruvate kinase', sub: 'Glycolysis', rank: rank0 + 8, lane }),
  ];
  if (includeFBP) {
    out.splice(2, 0, r(n('FBP'), `${x('fdp')} + ${x('h2o')} --> ${x('f6p')} + ${x('pi')}`, { name: 'Fructose-1,6-bisphosphatase', sub: 'Glycolysis', rank: rank0 + 1, lane: lane + 1 }));
  }
  return out;
}

/** Oxidative + non-oxidative pentose phosphate pathway. */
export function pentosePhosphate({ comp = 'c', lane = -3, rank0 = 1, suffix = '' } = {}) {
  const x = (id) => s(id, comp);
  const n = (id) => `${id}${suffix}`;
  return [
    r(n('G6PDH2r'), `${x('g6p')} + ${x('nadp')} <=> ${x('6pgl')} + ${x('h')} + ${x('nadph')}`, { name: 'Glucose 6-phosphate dehydrogenase', sub: 'Pentose phosphate pathway', rank: rank0, lane }),
    r(n('PGL'), `${x('6pgl')} + ${x('h2o')} --> ${x('6pgc')} + ${x('h')}`, { name: '6-phosphogluconolactonase', sub: 'Pentose phosphate pathway', rank: rank0 + 1, lane }),
    r(n('GND'), `${x('6pgc')} + ${x('nadp')} --> ${x('co2')} + ${x('nadph')} + ${x('ru5p__D')}`, { name: 'Phosphogluconate dehydrogenase', sub: 'Pentose phosphate pathway', rank: rank0 + 2, lane }),
    r(n('RPE'), `${x('ru5p__D')} <=> ${x('xu5p__D')}`, { name: 'Ribulose 5-phosphate 3-epimerase', sub: 'Pentose phosphate pathway', rank: rank0 + 3, lane }),
    r(n('RPI'), `${x('r5p')} <=> ${x('ru5p__D')}`, { name: 'Ribose-5-phosphate isomerase', sub: 'Pentose phosphate pathway', rank: rank0 + 3, lane: lane - 1 }),
    r(n('TKT1'), `${x('r5p')} + ${x('xu5p__D')} <=> ${x('g3p')} + ${x('s7p')}`, { name: 'Transketolase I', sub: 'Pentose phosphate pathway', rank: rank0 + 4, lane }),
    r(n('TALA'), `${x('g3p')} + ${x('s7p')} <=> ${x('e4p')} + ${x('f6p')}`, { name: 'Transaldolase', sub: 'Pentose phosphate pathway', rank: rank0 + 5, lane }),
    r(n('TKT2'), `${x('e4p')} + ${x('xu5p__D')} <=> ${x('f6p')} + ${x('g3p')}`, { name: 'Transketolase II', sub: 'Pentose phosphate pathway', rank: rank0 + 6, lane }),
  ];
}

/**
 * Citric acid cycle.
 * @param {'NADP'|'NAD'} icdhCofactor which isocitrate dehydrogenase to use
 * @param {string} quinone base id of the membrane quinone used by succinate dehydrogenase
 * @param {boolean} full false stops the block after isocitrate dehydrogenase
 */
export function tca({ comp = 'c', lane = 0, rank0 = 12, suffix = '', icdhCofactor = 'NADP', quinone = 'q8', full = true } = {}) {
  const x = (id) => s(id, comp);
  const n = (id) => `${id}${suffix}`;
  const out = [
    r(n('CS'), `${x('accoa')} + ${x('h2o')} + ${x('oaa')} --> ${x('cit')} + ${x('coa')} + ${x('h')}`, { name: 'Citrate synthase', sub: 'Citric acid cycle', rank: rank0, lane }),
    r(n('ACONT'), `${x('cit')} <=> ${x('icit')}`, { name: 'Aconitase', sub: 'Citric acid cycle', rank: rank0 + 1, lane }),
  ];
  if (icdhCofactor === 'NADP') {
    out.push(r(n('ICDHyr'), `${x('icit')} + ${x('nadp')} <=> ${x('akg')} + ${x('co2')} + ${x('nadph')}`, { name: 'Isocitrate dehydrogenase (NADP)', sub: 'Citric acid cycle', rank: rank0 + 2, lane }));
  } else {
    out.push(r(n('ICDHxm'), `${x('icit')} + ${x('nad')} --> ${x('akg')} + ${x('co2')} + ${x('nadh')}`, { name: 'Isocitrate dehydrogenase (NAD)', sub: 'Citric acid cycle', rank: rank0 + 2, lane }));
  }
  if (!full) return out;
  out.push(
    r(n('AKGDH'), `${x('akg')} + ${x('coa')} + ${x('nad')} --> ${x('co2')} + ${x('nadh')} + ${x('succoa')}`, { name: '2-Oxoglutarate dehydrogenase', sub: 'Citric acid cycle', rank: rank0 + 3, lane }),
    r(n('SUCOAS'), `${x('atp')} + ${x('coa')} + ${x('succ')} <=> ${x('adp')} + ${x('pi')} + ${x('succoa')}`, { name: 'Succinyl-CoA synthetase', sub: 'Citric acid cycle', rank: rank0 + 4, lane }),
    r(n('SUCD'), `${x(quinone)} + ${x('succ')} --> ${x('fum')} + ${x(`${quinone}h2`)}`, { name: 'Succinate dehydrogenase', sub: 'Citric acid cycle', rank: rank0 + 5, lane }),
    r(n('FUM'), `${x('fum')} + ${x('h2o')} <=> ${x('mal__L')}`, { name: 'Fumarase', sub: 'Citric acid cycle', rank: rank0 + 6, lane }),
    r(n('MDH'), `${x('mal__L')} + ${x('nad')} <=> ${x('h')} + ${x('nadh')} + ${x('oaa')}`, { name: 'Malate dehydrogenase', sub: 'Citric acid cycle', rank: rank0 + 7, lane }),
  );
  return out;
}

/** Reductive pentose phosphate (Calvin–Benson–Bassham) cycle. */
export function calvin({ comp = 'c', lane = -5, rank0 = 1, suffix = '_h' } = {}) {
  const x = (id) => s(id, comp);
  const n = (id) => `${id}${suffix}`;
  return [
    r(n('RBPC'), `${x('co2')} + ${x('h2o')} + ${x('rb15bp')} --> 2.0 ${x('3pg')} + 2.0 ${x('h')}`, { name: 'RuBisCO (carboxilación)', sub: 'Calvin cycle', rank: rank0, lane }),
    r(n('PRK'), `${x('atp')} + ${x('ru5p__D')} --> ${x('adp')} + ${x('h')} + ${x('rb15bp')}`, { name: 'Fosforribuloquinasa', sub: 'Calvin cycle', rank: rank0 + 7, lane }),
    r(n('GAPDHn'), `${x('g3p')} + ${x('nadp')} + ${x('pi')} <=> ${x('13dpg')} + ${x('h')} + ${x('nadph')}`, { name: 'GAPDH dependiente de NADPH', sub: 'Calvin cycle', rank: rank0 + 2, lane }),
  ];
}
