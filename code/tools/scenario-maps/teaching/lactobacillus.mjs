/**
 * Lactic acid bacterium core (models/lactobacillus_core_mc.json): all 47
 * reactions. Glucose comes in at the top onto the glycolytic spine, the pentose
 * phosphate pathway runs right from glucose 6-phosphate, and the fermentation
 * that regenerates NAD+ hangs below pyruvate: lactate straight down, and the
 * mixed branch (formate, acetate, ethanol) through acetyl-CoA, every product
 * leaving through the bottom membrane.
 */
import { Template } from './lib.mjs';

const E = 120;
const CW = 1250, CH = 1900;
const P = (x, y) => [E + x, E + y];

export default function lactobacillus(model) {
  const t = new Template(model, { width: CW + 2 * E, height: CH + 2 * E });
  t.box('e', 0, 0, CW + 2 * E, CH + 2 * E);
  t.box('c', E, E, CW, CH);
  const met = (id, x, y, label, key) => t.met(id, ...P(x, y), { label, key });
  const at = (x, y) => P(x, y);
  const title = (id, text, purpose, x, y, size) => t.title(id, text, purpose, ...P(x, y), size ? { size } : {});

  // Glycolysis
  const X = 300;
  t.column({ edge: 'top', along: E + X, chain: ['GLCt1', 'EX_glc__D_e'], mets: ['glc__D_c', 'glc__D_e'], depth: 110 });
  met('g6p_c', X, 260, 'W');
  met('f6p_c', X, 420, 'W');
  met('fdp_c', X, 580, 'W');
  met('g3p_c', X, 760, 'NE');
  met('dhap_c', 130, 760, 'S');
  met('13dpg_c', X, 920, 'W');
  met('3pg_c', X, 1080, 'W');
  met('2pg_c', X, 1240, 'W');
  met('pep_c', X, 1400, 'W');
  met('pyr_c', X, 1560, 'SW');
  t.rxn('HEX1', { line: ['glc__D_c', 'g6p_c'], cof: 'W' });
  t.rxn('PGI', { line: ['g6p_c', 'f6p_c'] });
  t.rxn('PFK', { line: ['f6p_c', 'fdp_c'], cof: 'W' });
  t.rxn('FBA', { line: ['fdp_c', 'g3p_c'], t: 0.42 });
  t.rxn('TPI', { line: ['dhap_c', 'g3p_c'] });
  t.rxn('GAPD', { line: ['g3p_c', '13dpg_c'], cof: 'W' });
  t.rxn('PGK', { line: ['13dpg_c', '3pg_c'], cof: 'W' });
  t.rxn('PGM', { line: ['3pg_c', '2pg_c'] });
  t.rxn('ENO', { line: ['2pg_c', 'pep_c'], cof: 'W' });
  t.rxn('PYK', { line: ['pep_c', 'pyr_c'], cof: 'W' });
  title('glycolysis', 'Glycolysis', 'Glucose to pyruvate', 20, 1170);

  // Pentose phosphate pathway: the oxidative row, then the carbon shuffle as a
  // lattice below ribulose 5-phosphate (TKT1 and TALA each a diamond)
  met('6pgl_c', 470, 260, 'S');
  met('6pgc_c', 640, 260, 'S');
  met('ru5p__D_c', 810, 260, 'NE');
  met('xu5p__D_c', 720, 420, 'NW');
  met('r5p_c', 900, 420, 'E');
  met('s7p_c', 720, 600, 'W');
  met('g3p_c', 900, 600, 'E', 'g3p_c#ppp');
  met('e4p_c', 720, 780, 'SW');
  met('f6p_c', 900, 780, 'E', 'f6p_c#ppp');
  met('g3p_c', 420, 340, 'N', 'g3p_c#tkt2');
  t.rxn('G6PDH2r', { line: ['g6p_c', '6pgl_c'], cof: 'N' });
  t.rxn('PGL', { line: ['6pgl_c', '6pgc_c'], cof: 'N' });
  t.rxn('GND', { line: ['6pgc_c', 'ru5p__D_c'], cof: 'N' });
  t.rxn('RPI', { line: ['r5p_c', 'ru5p__D_c'] });
  t.rxn('RPE', { line: ['ru5p__D_c', 'xu5p__D_c'] });
  const diamond = { use: { g3p_c: 'g3p_c#ppp' }, fork: ['r5p_c', 'xu5p__D_c', 'g3p_c', 's7p_c', 'e4p_c', 'f6p_c'] };
  t.rxn('TKT1', { at: at(810, 510), dir: [0, 1], ...diamond });
  t.rxn('TALA', { at: at(810, 690), dir: [0, 1], ...diamond, use: { g3p_c: 'g3p_c#ppp', f6p_c: 'f6p_c#ppp' } });
  // The second transketolase on the line back to F6P, its G3P beside it.
  t.rxn('TKT2', { line: ['xu5p__D_c', 'f6p_c'], use: { g3p_c: 'g3p_c#tkt2' }, fork: ['e4p_c'] });
  title('ppp', 'Pentose phosphate pathway', 'NADPH and pentoses', 470, 120);

  // Fermentation below pyruvate
  const Y = 1770;
  met('lac__L_c', X, Y, 'W');
  met('accoa_c', 620, 1480, 'N');
  met('for_c', 470, Y, 'E');
  met('actp_c', 620, 1660, 'NW');
  met('ac_c', 620, Y, 'W');
  met('acald_c', 800, 1660, 'NE');
  met('etoh_c', 800, Y, 'W');
  t.rxn('LDH_L', { line: ['lac__L_c', 'pyr_c'], cof: 'E' });
  t.rxn('PDH', { line: ['pyr_c', 'accoa_c'], bend: 'N', offset: 30, cof: 'N' });
  t.rxn('PFL', { line: ['pyr_c', 'accoa_c'], bend: 'S', offset: 30, cof: 'S' });
  t.rxn('PTAr', { line: ['accoa_c', 'actp_c'], cof: 'W' });
  t.rxn('ACKr', { line: ['ac_c', 'actp_c'], cof: 'E' });
  t.rxn('ACALD', { line: ['acald_c', 'accoa_c'], cof: 'N' });
  t.rxn('ALCD2x', { line: ['etoh_c', 'acald_c'], cof: 'E' });
  title('fermentation', 'Fermentation', 'Regenerates NAD+: lactate, or formate, acetate and ethanol', 700, 1440);
  t.column({ edge: 'bottom', along: E + X, chain: ['L_LACt2r', 'EX_lac__L_e'], mets: ['lac__L_c', 'lac__L_e'] });
  t.column({ edge: 'bottom', along: E + 470, chain: ['FORt', 'EX_for_e'], mets: ['for_c', 'for_e'] });
  t.column({ edge: 'bottom', along: E + 620, chain: ['ACt2r', 'EX_ac_e'], mets: ['ac_c', 'ac_e'] });
  t.column({ edge: 'bottom', along: E + 800, chain: ['ETOHt', 'EX_etoh_e'], mets: ['etoh_c', 'etoh_e'] });

  // Biomass, and the ATP spent on upkeep beside it
  t.rxn('BIOMASS_Lb_core', { at: at(1100, 930), dir: [0, 1], stack: {} });
  title('biomass', 'Biomass and maintenance', 'Precursors into new cells; ATP for upkeep', 560, 900);
  t.rxn('ATPM', { at: at(1100, 1170), dir: [1, 0], cof: 'S' });

  // CO2, NH4+, phosphate, water and protons
  for (const [b, x, tr] of [['co2', 890, 'CO2t'], ['nh4', 980, 'NH4t'], ['pi', 1070, 'PIt2r'], ['h2o', 1160, 'H2Ot']]) {
    t.column({ edge: 'bottom', along: E + x, chain: [tr, `EX_${b}_e`], mets: [`${b}_c`, `${b}_e`] });
  }
  t.column({ edge: 'top', along: E + 1120, chain: ['Ht', 'EX_h_e'], mets: ['h_c', 'h_e'] });
  return t;
}
