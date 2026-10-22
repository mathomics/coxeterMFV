/**
 * Methanosarcina core (models/methanogen_core_mc.json): all 24 reactions.
 * Acetyl-CoA and bicarbonate turn into each other round a ring: methyl-CoM
 * reductase splits acetyl-CoA into methane and bicarbonate along its upper
 * arc, CO dehydrogenase/acetyl-CoA synthase builds it back from bicarbonate
 * and H2 along the lower one. Acetate comes in from the left and is activated
 * to acetyl-CoA; on the right H2, coming down from the top membrane, reduces
 * bicarbonate straight to methane, which leaves through the right membrane,
 * and CO2 from the bottom membrane hydrates to bicarbonate below it.
 */
import { Template } from './lib.mjs';

const E = 120;
const CW = 1180, CH = 1000;
const P = (x, y) => [E + x, E + y];

export default function methanogen(model) {
  const t = new Template(model, { width: CW + 2 * E, height: CH + 2 * E });
  t.box('e', 0, 0, CW + 2 * E, CH + 2 * E);
  t.box('c', E, E, CW, CH);
  const met = (id, x, y, label, key) => t.met(id, ...P(x, y), { label, key });
  const at = (x, y) => P(x, y);
  const title = (id, text, purpose, x, y, size) => t.title(id, text, purpose, ...P(x, y), size ? { size } : {});

  // The acetyl-CoA / bicarbonate ring; the synthase's cofactors stand inside it
  t.ring('methano', {
    centre: P(640, 560), radius: 170, cycle: ['accoa_c', 'MCRac', 'hco3_c', 'CODH_ACS'],
    angles: { accoa_c: 180, hco3_c: 0 }, inside: ['CODH_ACS'], label: { accoa_c: 'W', hco3_c: 'NE' },
  });
  met('ch4_c', 1040, 560, 'N');
  met('h2_c', 860, 330, 'E');
  met('co2_c', 810, 840, 'W');
  // The acetoclastic methane is a small node beside the reductase, so that the
  // H2 line reaches the other reductase without crossing it.
  t.ringRxn('MCRac', { sec: ['ch4_c'] });
  t.ringRxn('CODH_ACS', { sec: ['h2_c'] });
  // H2 comes down from the top into the reductase's substrate end.
  t.rxn('MCRh2', { line: ['hco3_c', 'ch4_c'], use: { h2_c: 'h2_c' }, fork: ['h2_c'], cof: 'N' });
  t.rxn('HCO3E', { line: ['co2_c', 'hco3_c'], cof: 'E' });
  title('methano', 'Methanogenesis', 'Acetate or H2 + CO2 to methane', 470, 230, 32);

  // Acetate activation
  met('actp_c', 300, 560, 'N');
  met('ac_c', 140, 560, 'N');
  t.rxn('ACKr', { line: ['ac_c', 'actp_c'], cof: 'S' });
  t.rxn('PTAr', { line: ['accoa_c', 'actp_c'], cof: 'S' });
  t.column({ edge: 'left', along: E + 560, chain: ['ACt', 'EX_ac_e'], mets: ['ac_c', 'ac_e'] });

  // Methane out, H2 and CO2 in
  t.column({ edge: 'right', along: E + 560, chain: ['CH4t', 'EX_ch4_e'], mets: ['ch4_c', 'ch4_e'] });
  t.column({ edge: 'top', along: E + 860, chain: ['H2t', 'EX_h2_e'], mets: ['h2_c', 'h2_e'] });
  t.column({ edge: 'bottom', along: E + 810, chain: ['CO2t', 'EX_co2_e'], mets: ['co2_c', 'co2_e'] });

  // Biomass, and the ATP spent on upkeep beside it
  t.rxn('BIOMASS_Mb_core', { at: at(330, 830), dir: [0, 1], stack: {} });
  t.rxn('ATPM', { at: at(330, 700), dir: [1, 0], cof: 'N' });
  title('biomass', 'Biomass and maintenance', 'Acetyl-CoA and ATP into new cells; ATP for upkeep', 40, 955);

  // Water, NH4+, phosphate and protons
  for (const [b, x, tr] of [['h2o', 200, 'H2Ot'], ['nh4', 340, 'NH4t'], ['pi', 480, 'PIt'], ['h', 620, 'Ht']]) {
    t.column({ edge: 'top', along: E + x, chain: [tr, `EX_${b}_e`], mets: [`${b}_c`, `${b}_e`] });
  }
  return t;
}
