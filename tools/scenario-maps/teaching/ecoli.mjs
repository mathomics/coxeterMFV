/**
 * E. coli core with periplasm (models/ecoli_core_mc.json): every one of its
 * 95 reactions, laid out after the official Escher e_coli_core map. Glucose
 * comes in at the top through the PTS onto a straight glycolytic spine; the
 * oxidative pentose phosphate branch runs right from glucose 6-phosphate and
 * its carbon shuffle hangs below ribulose 5-phosphate as a lattice of two
 * diamonds (transketolase, transaldolase), the second transketolase returning
 * the carbon to the spine; the TCA cycle is a ring right of PEP and pyruvate,
 * with the glyoxylate shunt on a concentric lane inside its lower arc; mixed-acid
 * fermentation hangs below pyruvate and leaves through the bottom membranes;
 * the respiratory chain sits on the top inner membrane with the quinone pool
 * between its two complexes, and ATP synthase, the transhydrogenases and the
 * ATP-only reactions stand together on the right membrane.
 */
import { Template, arcHandles } from './lib.mjs';

const BAND = 100;                 // width of the periplasm and of the cell's outer band
const CW = 1760, CH = 1900;       // the cytosol
const O = 2 * BAND;
const P = (x, y) => [O + x, O + y];

export default function ecoli(model) {
  const t = new Template(model, { width: CW + 2 * O, height: CH + 2 * O });
  t.box('e', 0, 0, CW + 2 * O, CH + 2 * O);
  t.box('p', BAND, BAND, CW + 2 * BAND, CH + 2 * BAND);
  t.box('c', O, O, CW, CH);
  const met = (id, x, y, label, key) => t.met(id, ...P(x, y), { label, key });
  const at = (x, y) => P(x, y);
  const title = (id, text, purpose, x, y, size) => t.title(id, text, purpose, ...P(x, y), size ? { size } : {});

  // Glycolysis: one straight spine down the left of the cytosol
  const X = 300;
  met('g6p_c', X, 170, 'W');
  met('f6p_c', X, 340, 'W');
  met('fdp_c', X, 510, 'W');
  met('g3p_c', X, 700, 'NE');
  met('dhap_c', 130, 700, 'S');
  met('13dpg_c', X, 860, 'W');
  met('3pg_c', X, 1020, 'W');
  met('2pg_c', X, 1150, 'W');
  met('pep_c', X, 1270, 'NW');
  met('pyr_c', X, 1600, 'SW');
  t.rxn('PGI', { line: ['g6p_c', 'f6p_c'] });
  t.rxn('PFK', { line: ['f6p_c', 'fdp_c'], cof: 'W' });
  t.rxn('FBP', { line: ['fdp_c', 'f6p_c'], bend: 'E', offset: 40, cof: 'E' });
  t.rxn('FBA', { line: ['fdp_c', 'g3p_c'], t: 0.42 });
  t.rxn('TPI', { line: ['dhap_c', 'g3p_c'] });
  t.rxn('GAPD', { line: ['g3p_c', '13dpg_c'], cof: 'W' });
  t.rxn('PGK', { line: ['13dpg_c', '3pg_c'], cof: 'W' });
  t.rxn('PGM', { line: ['3pg_c', '2pg_c'] });
  t.rxn('ENO', { line: ['2pg_c', 'pep_c'], cof: 'W' });
  t.rxn('PYK', { line: ['pep_c', 'pyr_c'], cof: 'E' });
  t.rxn('PPS', { line: ['pyr_c', 'pep_c'], bend: 'W', offset: 40, cof: 'W' });
  title('glycolysis', 'Glycolysis', 'Glucose to pyruvate', 20, 1080);

  // Pentose phosphate pathway: the oxidative row, then the carbon shuffle as a
  // lattice below ribulose 5-phosphate (TKT1 and TALA each a diamond)
  met('6pgl_c', 470, 170, 'S');
  met('6pgc_c', 640, 170, 'S');
  met('ru5p__D_c', 810, 170, 'NE');
  met('xu5p__D_c', 720, 340, 'NW');
  met('r5p_c', 900, 340, 'E');
  met('s7p_c', 720, 520, 'W');
  met('g3p_c', 900, 520, 'E', 'g3p_c#ppp');
  met('e4p_c', 720, 700, 'SW');
  met('f6p_c', 900, 700, 'E', 'f6p_c#ppp');
  met('g3p_c', 420, 250, 'N', 'g3p_c#tkt2');
  t.rxn('G6PDH2r', { line: ['g6p_c', '6pgl_c'], cof: 'N' });
  t.rxn('PGL', { line: ['6pgl_c', '6pgc_c'], cof: 'N' });
  t.rxn('GND', { line: ['6pgc_c', 'ru5p__D_c'], cof: 'N' });
  t.rxn('RPI', { line: ['r5p_c', 'ru5p__D_c'] });
  t.rxn('RPE', { line: ['ru5p__D_c', 'xu5p__D_c'] });
  const diamond = { use: { g3p_c: 'g3p_c#ppp' }, fork: ['r5p_c', 'xu5p__D_c', 'g3p_c', 's7p_c', 'e4p_c', 'f6p_c'] };
  t.rxn('TKT1', { at: at(810, 430), dir: [0, 1], ...diamond });
  t.rxn('TALA', { at: at(810, 610), dir: [0, 1], ...diamond, use: { g3p_c: 'g3p_c#ppp', f6p_c: 'f6p_c#ppp' } });
  // The second transketolase on the line back to F6P, its G3P beside it.
  t.rxn('TKT2', { line: ['xu5p__D_c', 'f6p_c'], use: { g3p_c: 'g3p_c#tkt2' }, fork: ['e4p_c'] });
  title('ppp', 'Pentose phosphate pathway', 'NADPH and pentoses; the carbon returns as F6P and G3P', 470, 34);

  // TCA cycle as a ring, the glyoxylate shunt on a lane inside its lower arc
  const C = [810, 1150];
  const R = 280;
  t.ring('tca', {
    centre: P(...C), radius: R,
    cycle: ['oaa_c', 'CS', 'cit_c', 'ACONT', 'icit_c', 'ICDHyr', 'akg_c', 'AKGDH', 'succoa_c', 'SUCOAS',
      'succ_c', 'SUCDi', 'fum_c', 'FUM', 'mal__L_c', 'MDH'],
    anchor: ['oaa_c', 135], turn: -1, inside: ['MDH', 'SUCDi', 'FUM'],
    label: { oaa_c: 'SW', cit_c: 'S', icit_c: 'SE', akg_c: 'E', succoa_c: 'NE', succ_c: 'N', fum_c: 'NW', mal__L_c: 'W' },
  });
  met('accoa_c', 600, 1570, 'S');
  // Acetyl-CoA climbs to citrate synthase and turns into its axis from the oaa side.
  t.ringRxn('CS', { use: { accoa_c: 'accoa_c' }, curves: { accoa_c: [P(600, 1490), P(650, 1387)] } });
  t.track('FRD7', 'tca', ['fum_c', 'succ_c'], 46);
  // The shunt on a lane concentric with the ring along its lower arc, from
  // isocitrate through glyoxylate to malate, its cofactors on the lane's inner
  // side; the centre keeps only the title.
  const lane = 190;
  const glxAt = (112.5 * Math.PI) / 180;
  met('glx_c', C[0] + lane * Math.cos(glxAt), C[1] + lane * Math.sin(glxAt), 'N');
  t.track('ICL', 'tca', ['icit_c', 'glx_c'], lane - R, { sec: ['succ_c'] });
  t.track('MALS', 'tca', ['glx_c', 'mal__L_c'], lane - R, { sec: ['accoa_c'] });
  title('tca', 'TCA cycle', 'Oxidizes acetyl-CoA', C[0] - 80, C[1] - 100, 36);

  // Anaplerosis: PEP carboxylation and its reverse, malic enzyme
  t.rxn('PPC', { line: ['pep_c', 'oaa_c'], bend: 'N', offset: 30, cof: 'N' });
  t.rxn('PPCK', { line: ['oaa_c', 'pep_c'], bend: 'S', offset: 30, cof: 'S' });
  met('pyr_c', 380, 1060, 'NW', 'pyr_c#me');
  t.rxn('ME1', { line: ['mal__L_c', 'pyr_c#me'], bend: 'N', offset: 26, cof: 'N' });
  t.rxn('ME2', { line: ['mal__L_c', 'pyr_c#me'], bend: 'S', offset: 26, cof: 'S' });

  // Pyruvate to acetyl-CoA and the mixed-acid fermentation below it
  t.rxn('PDH', { line: ['pyr_c', 'accoa_c'], bend: 'N', offset: 30, cof: 'N' });
  const Y = 1770;
  met('lac__D_c', X, Y, 'W');
  met('for_c', 460, Y, 'W');
  met('h2_c', 590, Y, 'E');
  met('actp_c', 720, 1620, 'NE');
  met('ac_c', 720, Y, 'W');
  met('acald_c', 880, 1580, 'NE');
  met('etoh_c', 880, Y, 'W');
  met('succ_c', 1040, Y, 'E', 'succ_c#ex');
  met('lac__L_c', 130, 1600, 'N');
  t.rxn('PFL', { line: ['pyr_c', 'accoa_c'], bend: 'S', offset: 30, cof: 'S' });
  t.rxn('LDH_D', { line: ['lac__D_c', 'pyr_c'], cof: 'E' });
  t.rxn('L_LACD2', { line: ['lac__L_c', 'pyr_c'], cof: 'S' });
  t.rxn('FHL', { line: ['for_c', 'h2_c'], cof: 'N' });
  t.rxn('PTAr', { line: ['accoa_c', 'actp_c'], cof: 'W' });
  t.rxn('ACKr', { line: ['ac_c', 'actp_c'], cof: 'E' });
  t.rxn('ACALD', { line: ['acald_c', 'accoa_c'], cof: 'N' });
  t.rxn('ALCD2x', { line: ['etoh_c', 'acald_c'], cof: 'E' });
  title('fermentation', 'Mixed-acid fermentation', 'Products released without O2', 1120, 1610);

  // Respiratory chain on the top inner membrane, the quinone pool between its
  // complexes: QH2 on a half circle under the membrane from one complex to the
  // other, Q below it
  const qc = P(1260, 17);
  met('q8h2_c', 1260, 127, 'S');
  met('q8_c', 1260, 225, 'S');
  const qArc = (along) => arcHandles(qc, 110, P(1260, 127), P(along, 17));
  t.rxn('NADH16pp', { edge: 'top', along: O + 1150, use: { q8_c: 'q8_c', q8h2_c: 'q8h2_c' }, cof: 'W',
    curves: { q8h2_c: qArc(1150) } });
  t.rxn('CYTBD', { edge: 'top', along: O + 1370, use: { q8_c: 'q8_c', q8h2_c: 'q8h2_c', o2_c: 'o2_c' }, cof: 'E',
    curves: { q8h2_c: qArc(1370) } });
  title('oxphos', 'Respiration and ATP synthesis', 'NADH to O2 pumps H+ out; ATP synthase lets it back', 960, 320);

  // ATP synthase and the transhydrogenase on the right membrane, the reactions
  // that only turn over ATP or NAD(P)H in a column beside them
  t.rxn('ATPS4rpp', { edge: 'right', along: O + 230, cof: 'S' });
  t.rxn('THD2pp', { edge: 'right', along: O + 520, cof: 'S' });
  t.rxn('ATPM', { at: at(1590, 200), dir: [0, 1], cof: 'W' });
  t.rxn('ADK1', { at: at(1590, 370), dir: [0, 1], cof: 'W' });
  t.rxn('NADTRHD', { at: at(1590, 560), dir: [0, 1], cof: 'W' });

  // Biomass: precursors on one arc, what it gives back on the other
  t.rxn('BIOMASS_Ec_core', { at: at(1380, 620), dir: [0, 1], stack: {} });
  title('biomass', 'Biomass', 'Precursors into new cells', 1290, 800);

  // Nitrogen assimilation off 2-oxoglutarate
  met('glu__L_c', 1360, 1150, 'NE');
  met('gln__L_c', 1360, 1330, 'E');
  t.rxn('GLUDy', { line: ['glu__L_c', 'akg_c'], cof: 'N', sec: ['nh4_c'] });
  t.rxn('GLNS', { line: ['glu__L_c', 'gln__L_c'], cof: 'E', sec: ['nh4_c'] });
  title('nitrogen', 'Nitrogen assimilation', 'NH4+ onto 2-oxoglutarate', 1330, 1420);

  // Transport across both membranes, square to them
  t.column({ edge: 'top', along: O + X, chain: ['GLCptspp', 'GLCtex', 'EX_glc__D_e'], mets: ['g6p_c', 'glc__D_p', 'glc__D_e'] });
  t.reactions.get('GLCptspp').cof = 'E';
  t.column({ edge: 'top', along: O + 1560, chain: ['O2tpp', 'O2tex', 'EX_o2_e'], mets: ['o2_c', 'o2_p', 'o2_e'] });
  t.column({ edge: 'top', along: O + 1670, chain: ['CO2tpp', 'CO2tex', 'EX_co2_e'], mets: ['co2_c', 'co2_p', 'co2_e'] });
  t.column({ edge: 'left', along: O + 1600, chain: ['L_LACt2pp', 'L_LACtex', 'EX_lac__L_e'], mets: ['lac__L_c', 'lac__L_p', 'lac__L_e'] });
  t.column({ edge: 'right', along: O + 1250, chain: ['NH4tpp', 'NH4tex', 'EX_nh4_e'], mets: ['nh4_c', 'nh4_p', 'nh4_e'] });
  const bottom = [['lac__D', 'D_LACt2pp', 'D_LACtex'], ['h2', 'H2tpp', 'H2tex'], ['ac', 'ACt2rpp', 'ACtex'],
    ['etoh', 'ETOHtrpp', 'ETOHtex']];
  for (const [b, inner, outer] of bottom) {
    const x = t.node(`${b}_c`).x;
    t.column({ edge: 'bottom', along: x, chain: [inner, outer, `EX_${b}_e`], mets: [`${b}_c`, `${b}_p`, `${b}_e`] });
  }
  t.column({ edge: 'bottom', along: O + 1040, chain: ['SUCCt2rpp', 'SUCCtex', 'EX_succ_e'], mets: ['succ_c#ex', 'succ_p', 'succ_e'] });
  // Formate leaves by two carriers, one each way, side by side on the inner membrane.
  t.column({ edge: 'bottom', along: O + 460, chain: ['FORt2pp', 'FORtex', 'EX_for_e'], mets: ['for_c', 'for_p', 'for_e'] });
  t.rxn('FORtppi', { edge: 'bottom', along: O + 460, colMets: ['for_c', 'for_p'] });
  t.reactions.get('FORt2pp').shift = [-26, 0];
  t.reactions.get('FORtppi').shift = [26, 0];
  for (const [b, x, inner, outer] of [['h2o', 1220, 'H2Otpp', 'H2Otex'], ['pi', 1380, 'PIt2rpp', 'PItex']]) {
    t.column({ edge: 'bottom', along: O + x, chain: [inner, outer, `EX_${b}_e`], mets: [`${b}_c`, `${b}_p`, `${b}_e`] });
  }
  t.column({ edge: 'bottom', along: O + 1540, chain: ['Htex', 'EX_h_e'], mets: ['h_p', 'h_e'], depth: BAND / 2 });
  return t;
}
