/**
 * S. cerevisiae core with mitochondrion (models/yeast_core_mc.json): all 88
 * reactions. The glycolytic spine runs down the cytosol with the pentose
 * phosphate pathway beside it, its carbon shuffle a lattice of two diamonds;
 * the mitochondrion lies under the spine, so pyruvate, acetaldehyde and
 * ethanol each drop straight through the mitochondrial membranes, and the
 * products of alcoholic fermentation leave through the plasma membrane on the
 * left, acetyl-CoA synthetase above them with the two reactions that recycle
 * its AMP and pyrophosphate. The cytosolic glyoxylate cycle is a ring between
 * PEP and pyruvate on one side and the mitochondrial succinate carrier on the
 * other. In the matrix the TCA cycle is a ring under the PDH column; on the
 * inner membrane, facing the intermembrane space, stand ATP synthase and the
 * ADP/ATP carrier, and the two quinone-linked complexes with the ubiquinone
 * pool between them: QH2 on a half circle from one to the other, Q outside it,
 * and the internal NADH dehydrogenase as the rung between the two.
 */
import { Template, arcHandles } from './lib.mjs';

const E = 120;                    // the cell's outer band
const CW = 2400, CH = 2780;       // the cytosol
const IM = { x: 40, y: 1780, w: 2260, h: 920 };   // intermembrane space, around the matrix
const GAP = 55;                   // width of the intermembrane space
const P = (x, y) => [E + x, E + y];

export default function yeast(model) {
  const t = new Template(model, { width: CW + 2 * E, height: CH + 2 * E });
  t.box('e', 0, 0, CW + 2 * E, CH + 2 * E);
  t.box('c', E, E, CW, CH);
  // Rounded, but not so round that the carriers near its ends leave the membrane.
  const shape = { corner: 0.22, oval: 0 };
  t.box('im', E + IM.x, E + IM.y, IM.w, IM.h, { shape });
  t.box('m', E + IM.x + GAP, E + IM.y + GAP, IM.w - 2 * GAP, IM.h - 2 * GAP, { shape: { ...shape } });
  const met = (id, x, y, label, key) => t.met(id, ...P(x, y), { label, key });
  const at = (x, y) => P(x, y);
  const title = (id, text, purpose, x, y, size) => t.title(id, text, purpose, ...P(x, y), size ? { size } : {});
  const mTop = IM.y + GAP, mRight = IM.x + IM.w - GAP, mBottom = IM.y + IM.h - GAP;

  // Glycolysis
  const X = 600;
  met('g6p_c', X, 260, 'W');
  met('f6p_c', X, 420, 'W');
  met('fdp_c', X, 580, 'W');
  met('g3p_c', X, 760, 'NE');
  met('dhap_c', 430, 760, 'S');
  met('13dpg_c', X, 920, 'W');
  met('3pg_c', X, 1080, 'W');
  met('2pg_c', X, 1240, 'W');
  met('pep_c', X, 1400, 'NW');
  met('pyr_c', X, 1650, 'NE');
  t.column({ edge: 'top', along: E + X, chain: ['GLCt1', 'EX_glc__D_e'], mets: ['glc__D_c', 'glc__D_e'], depth: 110 });
  t.rxn('HEX1', { line: ['glc__D_c', 'g6p_c'], cof: 'W' });
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
  title('glycolysis', 'Glycolysis', 'Glucose to pyruvate', 660, 1095);

  // Glycerol, the cytosol's outlet for NADH
  met('glyc3p_c', 250, 760, 'N');
  met('glyc_c', 250, 920, 'E');
  t.rxn('G3PD1ir', { line: ['dhap_c', 'glyc3p_c'], cof: 'N' });
  t.rxn('G3PT', { line: ['glyc3p_c', 'glyc_c'], cof: 'W' });
  t.column({ edge: 'left', along: E + 920, chain: ['GLYCt', 'EX_glyc_e'], mets: ['glyc_c', 'glyc_e'] });
  title('glycerol', 'Glycerol', 'Reoxidizes NADH', 30, 680);

  // Pentose phosphate pathway: the oxidative row, then the carbon shuffle as a
  // lattice below ribulose 5-phosphate (TKT1 and TALA each a diamond)
  met('6pgl_c', 770, 260, 'S');
  met('6pgc_c', 940, 260, 'S');
  met('ru5p__D_c', 1110, 260, 'NE');
  met('xu5p__D_c', 1020, 420, 'NW');
  met('r5p_c', 1200, 420, 'E');
  met('s7p_c', 1020, 600, 'W');
  met('g3p_c', 1200, 600, 'E', 'g3p_c#ppp');
  met('e4p_c', 1020, 780, 'SW');
  met('f6p_c', 1200, 780, 'E', 'f6p_c#ppp');
  met('g3p_c', 720, 340, 'N', 'g3p_c#tkt2');
  t.rxn('G6PDH2r', { line: ['g6p_c', '6pgl_c'], cof: 'N' });
  t.rxn('PGL', { line: ['6pgl_c', '6pgc_c'], cof: 'N' });
  t.rxn('GND', { line: ['6pgc_c', 'ru5p__D_c'], cof: 'N' });
  t.rxn('RPI', { line: ['r5p_c', 'ru5p__D_c'] });
  t.rxn('RPE', { line: ['ru5p__D_c', 'xu5p__D_c'] });
  const diamond = { use: { g3p_c: 'g3p_c#ppp' }, fork: ['r5p_c', 'xu5p__D_c', 'g3p_c', 's7p_c', 'e4p_c', 'f6p_c'] };
  t.rxn('TKT1', { at: at(1110, 510), dir: [0, 1], ...diamond });
  t.rxn('TALA', { at: at(1110, 690), dir: [0, 1], ...diamond, use: { g3p_c: 'g3p_c#ppp', f6p_c: 'f6p_c#ppp' } });
  // The second transketolase on the line back to F6P, its G3P beside it.
  t.rxn('TKT2', { line: ['xu5p__D_c', 'f6p_c'], use: { g3p_c: 'g3p_c#tkt2' }, fork: ['e4p_c'] });
  title('ppp', 'Pentose phosphate pathway', 'NADPH and pentoses; the carbon returns as F6P and G3P', 770, 120);

  // Alcoholic fermentation, left of pyruvate, its products out through the left membrane
  const F = 1650;
  met('acald_c', 420, F, 'NE');
  met('etoh_c', 240, F, 'N');
  met('ac_c', 420, 1490, 'E');
  t.rxn('PDC', { line: ['pyr_c', 'acald_c'], cof: 'N' });
  t.rxn('ALCD2x', { line: ['etoh_c', 'acald_c'], cof: 'S' });
  t.rxn('ALDD2x', { line: ['acald_c', 'ac_c'], bend: 'W', offset: 30, cof: 'W' });
  t.rxn('ALDD2y', { line: ['acald_c', 'ac_c'], bend: 'E', offset: 30, cof: 'E' });
  t.column({ edge: 'left', along: E + F, chain: ['ETOHt', 'EX_etoh_e'], mets: ['etoh_c', 'etoh_e'] });
  t.column({ edge: 'left', along: E + 1490, chain: ['ACt2r', 'EX_ac_e'], mets: ['ac_c', 'ac_e'] });
  title('fermentation', 'Alcoholic fermentation', 'Pyruvate to ethanol and CO2', 30, 1250);

  // Acetate activated to acetyl-CoA above it; its AMP and pyrophosphate are
  // recycled by adenylate kinase and pyrophosphatase beside it
  met('accoa_c', 420, 1330, 'N', 'accoa_c#acs');
  t.rxn('ACS', { line: ['ac_c', 'accoa_c#acs'], cof: 'W' });
  t.rxn('ADK1', { at: at(250, 1400), dir: [-1, 0], cof: 'N' });
  t.rxn('PPA', { at: at(100, 1400), dir: [-1, 0], cof: 'N' });

  // Cytosolic glyoxylate cycle as a ring, fed by pyruvate carboxylase. The two
  // anaplerotic steps, pyruvate to oxaloacetate and back to PEP, are one round
  // arc from the spine through oxaloacetate, which the ring touches there.
  const D = { c: [715, 1525], r: 170 };
  const oaaX = D.c[0] + D.r;
  const G = [oaaX + 170, 1525];
  t.ring('glyox', {
    centre: P(...G), radius: 170,
    cycle: ['oaa_c', 'CS_c', 'cit_c', 'ACONT_c', 'icit_c', 'ICL', 'glx_c', 'MALS', 'mal__L_c', 'MDH_c'],
    anchor: ['oaa_c', 180], turn: 1, inside: ['CS_c', 'MDH_c', 'ACONT_c'],
    label: { oaa_c: 'S', cit_c: 'N', icit_c: 'NE', glx_c: 'SE', mal__L_c: 'S' },
  });
  met('succ_c', 1370, 1525, 'NE');
  t.ringRxn('CS_c', { sec: ['accoa_c'] });
  t.ringRxn('MALS', { sec: ['accoa_c'] });
  t.circle('ana', P(...D.c), D.r);
  const angle = (p) => (Math.atan2(p[1] - D.c[1], p[0] - D.c[0]) * 180) / Math.PI;
  const aPyr = angle([X, 1650]), aPep = angle([X, 1400]);
  // Pyruvate carboxylase takes the bicarbonate that carbonic anhydrase makes
  // inside the arc, just behind its substrate end.
  met('hco3_c', D.c[0], 1635, 'W');
  t.arcRxn('PC', 'ana', aPyr / 2, -1, { use: { hco3_c: 'hco3_c' } });
  t.arcRxn('PPCK', 'ana', aPep / 2, -1, {});
  t.rxn('HCO3E', { at: at(D.c[0], 1560), dir: [0, 1], cof: 'E', use: { hco3_c: 'hco3_c' } });
  title('glyox', 'Glyoxylate cycle', 'Acetyl-CoA to C4 acids', 940, 1240);

  // Nitrogen assimilation beside the 2-oxoglutarate carrier
  met('akg_c', 1500, 1650, 'E');
  met('glu__L_c', 1500, 1490, 'NW');
  met('gln__L_c', 1680, 1490, 'E');
  t.rxn('GLUDy', { line: ['glu__L_c', 'akg_c'], cof: 'E', sec: ['nh4_c'] });
  t.rxn('GLNS', { line: ['glu__L_c', 'gln__L_c'], cof: 'N', sec: ['nh4_c'] });
  t.column({ edge: 'right', along: E + 1490, chain: ['NH4t', 'EX_nh4_e'], mets: ['nh4_c', 'nh4_e'] });
  title('nitrogen', 'Nitrogen assimilation', 'NH4+ onto 2-oxoglutarate', 1640, 1330);

  // Biomass: precursors on one arc, what it gives back on the other
  t.rxn('BIOMASS_Sc_core', { at: at(1980, 1000), dir: [1, 0], fan: { r: 220, rp: 170 } });
  title('biomass', 'Biomass', 'Precursors into new cells', 1450, 1000);

  // The plasma membrane H+ pump, and ATP maintenance beside it
  t.column({ edge: 'top', along: E + 1750, chain: ['PMA1', 'EX_h_e'], mets: ['h_e'], depth: E / 2 });
  t.reactions.get('PMA1').cof = 'W';
  t.rxn('ATPM', { at: at(1900, 190), dir: [0, 1], cof: 'W' });
  title('energy', 'H+ pump and maintenance', 'ATP spent on the gradient and upkeep', 1580, 330);

  // Into the mitochondrion, straight down from the cytosol
  const mDepth = 80;
  met('pyr_m', X, mTop + mDepth, 'E');
  met('acald_m', 420, mTop + mDepth, 'S');
  met('etoh_m', 240, mTop + mDepth, 'S');
  t.rxn('PYRt2m', { edge: 'top', along: E + X, colMets: ['pyr_c', 'pyr_m'], outAlong: 100 });
  t.rxn('ACALDtm', { edge: 'top', along: E + 420, colMets: ['acald_c', 'acald_m'] });
  t.rxn('ETOHtm', { edge: 'top', along: E + 240, colMets: ['etoh_c', 'etoh_m'] });
  t.rxn('ALCD2xm', { line: ['etoh_m', 'acald_m'], cof: 'S' });
  t.rxn('SUCCtm', { edge: 'top', along: E + 1370, colMets: ['succ_c'], outAlong: 100 });
  t.rxn('AKGMALtm', { edge: 'top', along: E + 1500, colMets: ['akg_c'], outAlong: 100 });

  // TCA cycle in the matrix, under the PDH column
  const C = [820, 2300];
  met('accoa_m', 760, mTop + mDepth, 'N');
  t.rxn('PDHm', { line: ['pyr_m', 'accoa_m'], cof: 'S' });
  t.ring('tca', {
    centre: P(...C), radius: 255,
    cycle: ['oaa_m', 'CSm', 'cit_m', 'ACONTm', 'icit_m', 'ICDHxmm', 'akg_m', 'AKGDHm', 'succoa_m', 'SUCOASm',
      'succ_m', 'SUCDm', 'fum_m', 'FUMm', 'mal__L_m', 'MDHm'],
    anchor: ['oaa_m', -112.5], turn: 1, inside: ['CSm', 'MDHm'],
    label: { oaa_m: 'W', cit_m: 'NE', icit_m: 'E', akg_m: 'E', succoa_m: 'SE', succ_m: 'S', fum_m: 'SW', mal__L_m: 'W' },
  });
  t.ringRxn('CSm', { use: { accoa_m: 'accoa_m' } });
  title('tca', 'TCA cycle', 'Oxidizes acetyl-CoA', C[0] - 110, C[1] - 20, 36);

  // ATP synthase and the ADP/ATP carrier on the inner membrane, maintenance beside them
  t.rxn('Htm', { edge: 'right', along: E + 2030, inAlong: 27, outAlong: 50 });
  t.rxn('ATPtm', { edge: 'right', along: E + 1960, cof: 'N', outAlong: 105 });
  t.rxn('ATPS4m', { edge: 'right', along: E + 2100, cof: 'S', outAlong: 27 });
  t.rxn('ATPMm', { at: at(mRight - 120, 2030), dir: [0, 1], cof: 'W' });
  // The quinone pool between the external NADH dehydrogenase and the oxidase:
  // QH2 on a half circle from one complex to the other, Q outside it, and the
  // internal NADH dehydrogenase as the rung between the two
  const qy = 2400, qr = 110;
  met('q6h2_m', mRight - qr, qy, 'E');
  met('q6_m', mRight - qr - 150, qy, 'W');
  const qc = P(mRight, qy);
  const qArc = (y) => arcHandles(qc, qr, P(mRight - qr, qy), P(mRight - 17, y));
  const rq = { use: { q6_m: 'q6_m', q6h2_m: 'q6h2_m' } };
  t.rxn('NADH2_u6cm', { edge: 'right', along: E + qy - qr, ...rq, cof: 'N', outAlong: 105, curves: { q6h2_m: qArc(qy - qr) } });
  t.rxn('CYOO_u6m', { edge: 'right', along: E + qy + qr, use: { ...rq.use, o2_m: 'o2_m' }, cof: 'S', outAlong: 27,
    curves: { q6h2_m: qArc(qy + qr) } });
  t.rxn('NADH2_u6m', { line: ['q6_m', 'q6h2_m'], cof: 'N' });
  title('resp', 'Respiration and ATP synthesis', 'NADH to O2 pumps H+ out of the matrix', 1350, 1990);

  // O2, CO2, water and phosphate cross both the plasma and the mitochondrial membranes
  for (const [b, x, tm, t1] of [['pi', 1650, 'PIt2m', 'PIt2r'], ['h2o', 1800, 'H2Otm', 'H2Ot'],
    ['co2', 1950, 'CO2tm', 'CO2t'], ['o2', 2100, 'O2tm', 'O2t']]) {
    t.column({ edge: 'bottom', along: E + x, chain: [tm, t1, `EX_${b}_e`], mets: [`${b}_m`, `${b}_c`, `${b}_e`] });
  }
  t.reactions.get('PIt2m').outAlong = 95;
  // The two phosphate carriers' protons on opposite sides of their column.
  t.reactions.get('PIt2m').cof = 'W';
  t.reactions.get('PIt2r').cof = 'E';
  // Pyruvate is also exchanged with the medium, from a second drawing beside the fermentation outlets.
  met('pyr_c', 110, 1170, 'N', 'pyr_c#ex');
  t.column({ edge: 'left', along: E + 1170, chain: ['PYRt2', 'EX_pyr_e'], mets: ['pyr_c#ex', 'pyr_e'] });
  return t;
}
