/**
 * Synechocystis core with thylakoid (models/cyano_core_mc.json): all 61
 * reactions. Light comes in at the top onto the thylakoid membrane, where the
 * two photosystems, the respiratory complexes that share their plastoquinone
 * pool, and the ATP synthase stand facing the stroma, ATP maintenance below
 * the synthase. Below it the Calvin-Benson-Bassham cycle is a ring, fed from
 * the left by the carbon-concentrating mechanism. The co-substrates of its
 * carbon shuffle are small nodes outside the ring beside the steps that use
 * them, as the hand-made maps draw a cofactor, so that inside it stand only
 * the title and the short lane on which transketolase's xylulose 5-phosphate
 * returns to ribulose 5-phosphate. The oxidative pentose phosphate pathway
 * arcs over the ring from F6P back to Ru5P; lower glycolysis leaves the ring
 * at 3-PGA, and the incomplete TCA cycle (no 2-oxoglutarate dehydrogenase) is
 * drawn as its two open branches. Phosphate and water cross the right
 * membranes, beside the nitrogen assimilation that NH4+ feeds from below.
 */
import { Template, onCircle } from './lib.mjs';

const BAND = 100;
const CW = 1750, CH = 2100;
const O = 2 * BAND;
const P = (x, y) => [O + x, O + y];

export default function cyano(model) {
  const t = new Template(model, { width: CW + 2 * O, height: CH + 2 * O });
  t.box('e', 0, 0, CW + 2 * O, CH + 2 * O);
  t.box('p', BAND, BAND, CW + 2 * BAND, CH + 2 * BAND);
  t.box('c', O, O, CW, CH);
  const U = { x: 540, y: 250, w: 1150, h: 150 };
  t.box('u', ...P(U.x, U.y), U.w, U.h, { shape: { corner: 0.5, oval: 0 } });
  const met = (id, x, y, label, key) => t.met(id, ...P(x, y), { label, key });
  const at = (x, y) => P(x, y);
  const title = (id, text, purpose, x, y, size) => t.title(id, text, purpose, ...P(x, y), size ? { size } : {});
  const ring = (r, a) => onCircle(C, r, a);

  // Light reactions on the thylakoid membrane, the plastoquinone pool between them
  met('pq_c', 940, 160, 'N');
  met('pqh2_c', 940, 60, 'N');
  const pool = { use: { pq_c: 'pq_c', pqh2_c: 'pqh2_c' } };
  const top = O + U.x;
  t.rxn('PSII', { edge: 'top', along: top + 100, ...pool, cof: 'W', sec: ['photon_c', 'o2_c'] });
  t.rxn('NDH1', { edge: 'top', along: top + 280, cof: 'W' });
  t.rxn('COX', { edge: 'top', along: top + 520, cof: 'E' });
  t.rxn('PSI_FNR', { edge: 'top', along: top + 700, ...pool, cof: 'E', sec: ['photon_c'] });
  t.rxn('CEF', { edge: 'top', along: top + 880, cof: 'E', sec: ['photon_c'] });
  t.rxn('ATPS_u', { edge: 'top', along: top + 1060, cof: 'W' });
  t.column({ edge: 'top', along: O + 1370, chain: ['PHOTONt', 'EX_photon_e'], mets: ['photon_c', 'photon_e'], depth: 110 });
  title('light', 'Light reactions', 'Water split by light; NADPH, ATP and O2', 560, 470);

  // Calvin-Benson-Bassham cycle
  const C = P(820, 1060);
  const R = 300;
  t.ring('cbb', {
    centre: C, radius: R,
    cycle: ['rb15bp_c', 'RBPC', '3pg_c', 'PGK', '13dpg_c', 'GAPDHn', 'g3p_c', 'FBA', 'fdp_c', 'FBP', 'f6p_c',
      'TKT2', 'e4p_c', 'TALA', 's7p_c', 'TKT1', 'r5p_c', 'RPI', 'ru5p__D_c', 'PRK'],
    anchor: ['rb15bp_c', 180], turn: -1, inside: ['PGK', 'GAPDHn'],
    label: { rb15bp_c: 'W', '3pg_c': 'SW', '13dpg_c': 'S', g3p_c: 'S', fdp_c: 'SE', f6p_c: 'NE', e4p_c: 'NE', s7p_c: 'N', r5p_c: 'N', ru5p__D_c: 'NW' },
  });
  // Dihydroxyacetone phosphate just outside the aldolase step, with the
  // triose phosphate isomerase back to G3P
  t.met('dhap_c', ...ring(R + 95, 56), { key: 'dhap_c', label: 'E' });
  t.ringRxn('FBA', { use: { dhap_c: 'dhap_c' } });
  t.rxn('TPI', { line: ['dhap_c', 'g3p_c'] });
  // The carbon shuffle's co-substrates as small nodes outside the ring; the
  // xylulose 5-phosphate of the last transketolase returns to Ru5P on a short
  // lane inside it, beside ribose 5-phosphate isomerase
  const lane = R - 70;
  t.met('xu5p__D_c', ...ring(lane, -112), { key: 'xu5p__D_c', label: 'S' });
  t.ringRxn('TKT2', { sec: ['g3p_c', 'xu5p__D_c'] });
  t.ringRxn('TALA', { sec: ['g3p_c', 'f6p_c'] });
  t.ringRxn('TKT1', { sec: ['g3p_c'], use: { xu5p__D_c: 'xu5p__D_c' } });
  t.track('RPE', 'cbb', ['xu5p__D_c', 'ru5p__D_c'], lane - R);
  t.ringRxn('RBPC', { use: { co2_c: 'co2_c' } });
  t.track('GAPD', 'cbb', ['g3p_c', '13dpg_c'], 46);
  t.track('PFK', 'cbb', ['f6p_c', 'fdp_c'], 46);
  title('cbb', 'Calvin-Benson cycle', 'Fixes CO2 with ATP and NADPH', 650, 1020, 32);

  // Oxidative pentose phosphate pathway, an outer arc from F6P over the top to Ru5P
  const outer = R + 150;
  t.ring('oppp', {
    centre: C, radius: outer, cycle: ['g6p_c', 'G6PDH2r', '6pgl_c', 'PGL', '6pgc_c'], open: true,
    angles: { g6p_c: 0, '6pgl_c': -50, '6pgc_c': -100 }, label: { g6p_c: 'E', '6pgl_c': 'NE', '6pgc_c': 'N' },
  });
  t.arcRxn('GND', 'oppp', -125, -1, { cof: 'N' });
  t.rxn('PGI', { line: ['g6p_c', 'f6p_c'] });
  title('oppp', 'Oxidative pentose phosphate pathway', 'NADPH in the dark', 30, 560);

  // Carbon-concentrating mechanism, on the left of the ring
  met('co2_c', 300, 1140, 'N');
  met('hco3_c', 300, 1320, 'S');
  t.rxn('HCO3E', { line: ['co2_c', 'hco3_c'], cof: 'W' });
  t.column({ edge: 'left', along: O + 1140, chain: ['CO2tpp', 'CO2tex', 'EX_co2_e'], mets: ['co2_c', 'co2_p', 'co2_e'] });
  t.column({ edge: 'left', along: O + 1320, chain: ['BCT1', 'HCO3tex', 'EX_hco3_e'], mets: ['hco3_c', 'hco3_p', 'hco3_e'] });
  t.reactions.get('BCT1').cof = 'S';
  title('ccm', 'CO2 concentration', 'HCO3- pumped in, CO2 at Rubisco', 20, 1010);

  // Lower glycolysis down from 3-PGA, and the TCA cycle as its two open branches
  const [gx] = ring(R, 144);
  const X = Math.round(gx - O);
  met('2pg_c', X, 1480, 'W');
  met('pep_c', X, 1720, 'W');
  met('pyr_c', X, 1900, 'W');
  met('accoa_c', 760, 1900, 'S');
  met('oaa_c', 760, 1720, 'SW');
  met('cit_c', 930, 1720, 'S');
  met('icit_c', 1100, 1720, 'S');
  met('akg_c', 1270, 1720, 'S');
  met('mal__L_c', 760, 1560, 'W');
  met('fum_c', 930, 1560, 'N');
  t.rxn('PGM', { line: ['3pg_c', '2pg_c'] });
  t.rxn('ENO', { line: ['2pg_c', 'pep_c'], cof: 'W' });
  t.rxn('PYK', { line: ['pep_c', 'pyr_c'], cof: 'W' });
  t.rxn('PDH', { line: ['pyr_c', 'accoa_c'], cof: 'S' });
  t.rxn('PPC', { line: ['pep_c', 'oaa_c'], cof: 'S' });
  t.rxn('CS', { line: ['oaa_c', 'cit_c'], cof: 'N' });
  t.rxn('ACONT', { line: ['cit_c', 'icit_c'] });
  t.rxn('ICDHyr', { line: ['icit_c', 'akg_c'], cof: 'N' });
  t.rxn('MDH', { line: ['mal__L_c', 'oaa_c'], cof: 'W' });
  t.rxn('FUM', { line: ['fum_c', 'mal__L_c'], cof: 'N' });
  title('tca', 'TCA branches', 'No 2-oxoglutarate dehydrogenase: two open arms', 900, 1990);

  // Nitrogen assimilation off 2-oxoglutarate
  met('glu__L_c', 1440, 1720, 'N');
  met('gln__L_c', 1440, 1900, 'E');
  t.rxn('GLUDy', { line: ['glu__L_c', 'akg_c'], cof: 'S', sec: ['nh4_c'] });
  t.rxn('GLNS', { line: ['glu__L_c', 'gln__L_c'], cof: 'E', sec: ['nh4_c'] });
  title('nitrogen', 'Nitrogen assimilation', 'NH4+ onto 2-oxoglutarate', 1380, 1560);

  // Biomass: precursors on one arc, what it gives back on the other
  t.rxn('BIOMASS_Syn_core', { at: at(1460, 1140), dir: [0, 1], stack: {} });
  title('biomass', 'Biomass', 'Precursors into new cells', 1340, 1340);
  // ATP maintenance under the thylakoid ATP synthase
  t.rxn('ATPM', { at: at(1600, 520), dir: [0, 1], cof: 'W' });

  // Exchange of O2, NH4+, phosphate, water and protons
  t.column({ edge: 'top', along: O + 300, chain: ['O2tpp', 'O2tex', 'EX_o2_e'], mets: ['o2_c', 'o2_p', 'o2_e'] });
  t.column({ edge: 'bottom', along: O + 1580, chain: ['NH4tpp', 'NH4tex', 'EX_nh4_e'], mets: ['nh4_c', 'nh4_p', 'nh4_e'] });
  t.column({ edge: 'right', along: O + 1640, chain: ['PIt2rpp', 'PItex', 'EX_pi_e'], mets: ['pi_c', 'pi_p', 'pi_e'] });
  t.column({ edge: 'right', along: O + 1780, chain: ['H2Otpp', 'H2Otex', 'EX_h2o_e'], mets: ['h2o_c', 'h2o_p', 'h2o_e'] });
  t.column({ edge: 'top', along: O + 150, chain: ['Htex', 'EX_h_e'], mets: ['h_p', 'h_e'], depth: BAND / 2 });
  return t;
}
