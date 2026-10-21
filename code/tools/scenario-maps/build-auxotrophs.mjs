#!/usr/bin/env node
/**
 * The curated map of the four auxotrophic E. coli strains of SteadyCom (Chan et
 * al. 2017), models/published/steadycom-ecoli-auxotrophs/map.json.
 *
 *   node tools/scenario-maps/build-auxotrophs.mjs
 *
 * One strain template is drawn once and placed four times, so the cells compare
 * at a glance: E. coli central metabolism laid out as the official Escher
 * e_coli_core map is (glycolysis down the left, the oxidative pentose phosphate
 * branch across the top with its non-oxidative ladder below, the TCA cycle as a
 * round ring, PPC as the anaplerotic step, fermentation under pyruvate, the
 * respiratory chain and ATP synthase on the inner membrane), plus the four
 * routes to the amino acids the strains trade (Phe from erythrose 4-phosphate,
 * Arg from 2-oxoglutarate, Lys and Met from aspartate), each ending at its
 * inner-membrane transporters, its outer-membrane porin and its exchange on the
 * cell boundary, with a medium node of the cell's own just outside, where the
 * app's cross-feeding lines start. The cells stand in a 2 x 2 grid in the
 * shared medium; the strains that trade with each other are neighbours.
 *
 * Every reaction, equation and bound is read from the bundled member models
 * through the app's own CommunityModel, so the map draws exactly what the
 * strain's model holds: a knocked-out reaction is drawn in each cell and is
 * closed in that strain's model. The same knockouts are written to the
 * manifest (`knockouts`), which is how the app draws them in its knockout
 * style; they are the reactions closed in some strains and open in others,
 * read from bounds.csv. The manifest's `map` entry is set too.
 *
 * Geometry follows the hand-made maps (the Yeast9 map of coxeter_mapping and
 * the Zuñiga map, tools/zuniga/build-map.mjs): a reaction is a short axis from
 * its substrates' end (a1) to its products' end (a2); a cycle is a true circle
 * with each reaction halfway between its metabolites, axis on the tangent,
 * branches as circular arcs (handles 4/3 tan(dθ/4) R) and every cofactor on the
 * same side of it; every metabolite of an equation is drawn, the carbon
 * backbone through shared primary nodes and the rest as small secondary nodes
 * on one template; a transporter stands on the membrane it crosses, each
 * metabolite branching from the end on its own side of the membrane. Output is
 * the same on every run.
 */
import { readFile, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Model } from '../../src/core/model.js';
import { CommunityModel, sharedMetabolite, LINK_COMPARTMENT } from '../../src/core/community.js';
import { EditableMap } from '../../src/map/editable-map.js';
import { createMapDocument } from '../../src/map/map-document.js';
import { boundaryKey } from '../../src/map/boundaries.js';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
const COMMUNITY = 'steadycom-ecoli-auxotrophs';
const DIR = join(ROOT, 'models', 'published', COMMUNITY);

// Sizes, in map units

const L = 160;            // spacing of consecutive intermediates along a row
const STEP = 140;         // the same down the glycolysis spine and the pentose phosphate ladder
const AXIS = 17;          // half length of a reaction marker (Coxeter SIZES.reactionAxis)
const BAND = 110;         // width of the periplasm band, and of the extracellular band but on the right
// On the right edge the exchange chains run across the page, and a reaction's
// label is written to the right of its marker: the extracellular band and the
// medium node keep clear of the porin's and the exchange's labels there.
const E_RIGHT = 280;
const OUT = { top: 80, bottom: 80, left: 80, right: 170 };   // medium node outside the cell boundary
const eBand = (edge) => (edge === 'right' ? E_RIGHT : BAND);
const W = 2620;           // cytosol, as drawn
const H = 1900;
const GAP = 620;          // medium between neighbouring cells: room for the cross-feeding lines
const MARGIN = 360;       // medium between the cells and the edge of the system
const HEADER = 230;       // title above the cells

// Cofactor template: (along the axis from the marker's centre, across from it),
// rows from the inside out so that their curves nest without crossing.
const COFACTOR_ROWS = [[54, 40], [38, 68], [22, 96]];
// The same carriers take the same places on every reaction.
const COFACTOR_ORDER = ['atp', 'gtp', 'adp', 'gdp', 'amp', 'nadph', 'nadp', 'nadh', 'nad', 'q8h2', 'q8',
  'coa', 'accoa', 'succoa', 'pep', 'pyr', 'glu-L', 'akg', 'asp-L', 'cys-L', '5mthf', 'thf', 'cbp', 'orn',
  'fum', 'succ', 'ac', 'o2', 'co2', 'nh4', 'ppi', 'pi', 'h2o', 'h'];
const COMPASS = { N: [0, -1], S: [0, 1], E: [1, 0], W: [-1, 0] };
const EDGES = { top: [0, -1], bottom: [0, 1], right: [1, 0], left: [-1, 0] };

// What makes the strains differ (manifest `knockouts`), from the tutorial's table
const STRAINS = {
  Ec1: { needs: 'Lys + Met', blocked: 'Phe export', supplies: 'Arg' },
  Ec2: { needs: 'Arg + Phe', blocked: 'Met export', supplies: 'Lys' },
  Ec3: { needs: 'Arg + Phe', blocked: 'Lys export', supplies: 'Met' },
  Ec4: { needs: 'Lys + Met', blocked: 'Arg export', supplies: 'Phe' },
};
// Strains that trade stand side by side: Ec1 and Ec4 share nothing, nor do Ec2 and Ec3.
const GRID = [['Ec1', 'Ec2'], ['Ec3', 'Ec4']];
const COLOURS = { Ec1: '#0284c7', Ec2: '#d97706', Ec3: '#16a34a', Ec4: '#7c3aed' };

// Geometry helpers

const r2 = (v) => Math.round(v * 100) / 100;
const pt = (x, y) => ({ x: r2(x), y: r2(y) });
const unit = (dx, dy) => { const l = Math.hypot(dx, dy) || 1; return [dx / l, dy / l]; };
const lerp = (a, b, t) => [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
const wrap = (a) => { let x = a; while (x > 180) x -= 360; while (x <= -180) x += 360; return x; };
const rad = (d) => (d * Math.PI) / 180;
const onCircle = (c, r, a) => [c[0] + r * Math.cos(rad(a)), c[1] + r * Math.sin(rad(a))];
const tangent = (a) => [-Math.sin(rad(a)), Math.cos(rad(a))];
const angleOf = (c, p) => (Math.atan2(p[1] - c[1], p[0] - c[0]) * 180) / Math.PI;
const arcHandle = (span, r) => (4 / 3) * Math.tan(rad(Math.abs(span)) / 4) * r;

/** Handles on the straight line from the metabolite to the reaction. */
const straight = (node, anchor) => [lerp(node, anchor, 0.34), lerp(node, anchor, 0.72)];

/**
 * Coxeter's shapeBranch(): out of the reaction along its axis (u points away
 * from the marker), then square across to a metabolite beside it: a C, never an S.
 */
function sideCurve(node, anchor, u) {
  const vx = node[0] - anchor[0], vy = node[1] - anchor[1];
  const along = vx * u[0] + vy * u[1];
  const side = -vx * u[1] + vy * u[0];
  const across = Math.abs(side);
  if (across < 4 && along > 0) return straight(node, anchor);
  const hand = side < 0 ? -1 : 1;
  const n = [-u[1] * hand, u[0] * hand];
  const reach = Math.max(along, 0.5 * across, 14);
  return [[node[0] - n[0] * across * 0.55, node[1] - n[1] * across * 0.55],
    [anchor[0] + u[0] * reach * 0.8, anchor[1] + u[1] * reach * 0.8]];
}

/**
 * A metabolite of a reaction whose products (or substrates) fork to both sides
 * of its axis: a short way out along the axis, then on towards the node, so the
 * branches of two reactions meeting at that node arrive at an angle instead of
 * running into it side by side.
 */
function forkCurve(node, anchor, u) {
  return [lerp(node, anchor, 0.3), [anchor[0] + u[0] * 16, anchor[1] + u[1] * 16]];
}

/** A line metabolite beside the axis of a reaction drawn as an arc beside its line. */
function arcCurve(node, anchor, u) {
  const vx = node[0] - anchor[0], vy = node[1] - anchor[1];
  const along = vx * u[0] + vy * u[1];
  const b2 = [anchor[0] + u[0] * along * 0.5, anchor[1] + u[1] * along * 0.5];
  return [lerp(node, b2, 0.45), b2];
}

// The strain template, in the cytosol's own coordinates (0,0 its top left)

const nodes = new Map();       // key -> {mid, x, y, label (degrees)}
const reactions = new Map();   // reaction id -> spec
const rings = new Map();       // ring reaction id -> {centre, R, a, b, inside}
const texts = [];

function met(key, x, y, label = 90, mid = key) { nodes.set(key, { key, mid, x, y, label }); }
function rxn(id, spec) {
  if (reactions.has(id)) throw new Error(`reaction ${id} declared twice`);
  reactions.set(id, { id, ...spec });
}
/** Consecutive reactions along a row or column: `chain` alternates metabolites and reactions. */
function lane(chain, [x0, y0], [dx, dy], { cof, label }) {
  for (let i = 0; i < chain.length; i += 2) {
    const k = i / 2;
    if (!nodes.has(chain[i])) met(chain[i], x0 + dx * k, y0 + dy * k, label);
  }
  for (let i = 1; i < chain.length; i += 2) rxn(chain[i], { line: [chain[i - 1], chain[i + 1]], cof });
}
function text(id, title, purpose, x, y, size = 26) {
  texts.push({ id: `${id}`, text: title, x, y, fontSize: size });
  if (purpose) texts.push({ id: `${id}:purpose`, text: purpose, x, y: y + size * 0.95, fontSize: Math.round(size * 0.62) });
}

const SX = 300;               // the glycolysis spine
const PEP_Y = 160 + 7 * STEP;  // oxaloacetate stands level with PEP, so PPC runs straight across
const PYR_Y = PEP_Y + 200;
const TCA = { centre: [860, PEP_Y], R: 240 };
const PHE_Y = 160 + 3 * STEP; // shikimate and phenylalanine row, level with erythrose 4-phosphate
const ARG_Y = TCA.centre[1] + TCA.R * Math.sin(rad(20));  // arginine row, level with 2-oxoglutarate
const LYS_Y = 1580;           // aspartate and lysine row
const MET_Y = 1780;           // methionine row

// Glycolysis: one straight spine from glucose 6-phosphate to pyruvate
lane(['g6p_c', 'PGI', 'f6p_c', 'PFK', 'fdp_c', 'FBA', 'g3p_c', 'GAPD', '13dpg_c', 'PGK', '3pg_c',
  'PGM', '2pg_c', 'ENO', 'pep_c'], [SX, 160], [0, STEP], { cof: 'E', label: 180 });
reactions.get('FBA').use = ['dhap_c'];
reactions.get('FBA').t = 0.4;
reactions.get('ENO').cof = 'W';
reactions.get('PGI').cof = 'W';
// Dihydroxyacetone phosphate stands level with the end of FBA its branch leaves
// from, so that branch peels off the spine in one bend and TPI runs down to
// glyceraldehyde 3-phosphate apart from it, instead of both sharing one line.
const FBA_END = 160 + 2 * STEP + 0.4 * STEP + AXIS;
met('dhap_c', 140, FBA_END, -90);
reactions.get('FBA').bends = { dhap_c: { node: [55, 12], anchor: 28 } };
met('pyr_c', SX, PYR_Y, -135);
rxn('PYK', { line: ['pep_c', 'pyr_c'], cof: 'E' });
rxn('TPI', { line: ['dhap_c', 'g3p_c'], cof: 'S' });
// FBP runs beside PFK, far enough out that its label clears the spine.
rxn('FBP', { line: ['fdp_c', 'f6p_c'], offset: -64, cof: 'W' });
text('glycolysis', 'Glycolysis', 'Glucose 6-phosphate to pyruvate', 24, 760);

// Oxidative pentose phosphate branch across the top, the non-oxidative ladder below
lane(['g6p_c', 'G6PDH2r', '6pgl_c', 'PGL', '6pgc_c', 'GND', 'ru5p-D_c'], [SX, 160], [L, 0], { cof: 'S', label: -90 });
reactions.get('GND').cof = 'N';
met('xu5p-D_c', 700, 160 + STEP, 180);
met('r5p_c', 860, 160 + STEP, 0);
met('g3p_c#ppp', 700, 160 + 2 * STEP, 180, 'g3p_c');
met('s7p_c', 860, 160 + 2 * STEP, 0);
met('f6p_c#ppp', 700, PHE_Y, 180, 'f6p_c');
met('e4p_c', 860, PHE_Y, 60);
rxn('RPE', { line: ['ru5p-D_c', 'xu5p-D_c'], cof: 'W' });
rxn('RPI', { line: ['r5p_c', 'ru5p-D_c'], cof: 'E' });
rxn('TKT1', { at: [780, 160 + 1.5 * STEP], dir: [0, 1], fork: true, use: ['r5p_c', 'xu5p-D_c', 'g3p_c#ppp', 's7p_c'] });
rxn('TALA', { at: [780, 160 + 2.5 * STEP], dir: [0, 1], fork: true, use: ['g3p_c#ppp', 's7p_c', 'e4p_c', 'f6p_c#ppp'] });
rxn('TKT2', { line: ['e4p_c', 'f6p_c#ppp'], offset: -56, cof: 'S' });
text('ppp', 'Pentose phosphate pathway', 'NADPH, ribose 5-phosphate and erythrose 4-phosphate', 930, 330, 24);

// Shikimate pathway to phenylalanine, ending at the inner membrane
lane(['e4p_c', 'DDPA', '2dda7p_c', 'DHQS', '3dhq_c', 'DHQTi', '3dhsk_c', 'SHK3Dr', 'skm_c', 'SHKK', 'skm5p_c',
  'PSCVT', '3psme_c', 'CHORS', 'chor_c', 'CHORM', 'pphn_c', 'PPNDH', 'phpyr_c', 'PHETA1', 'phe-L_c'],
[860, PHE_Y], [L, 0], { cof: 'N', label: 90 });
text('phe', 'Shikimate pathway and phenylalanine', 'Erythrose 4-phosphate and PEP to L-phenylalanine (PPNDH is pheA)', 1020, 414, 24);

// TCA cycle as a ring, anticlockwise on the page as in e_coli_core, PPC from PEP.
// Every cofactor of the ring stands inside it, where nothing else is drawn, so
// the routes that join the ring from outside (PPC, acetyl-CoA, glutamate) meet
// nothing on the way in.
const tcaCycle = ['oaa_c', 'CS', 'cit_c', 'ACONTa', 'acon-C_c', 'ACONTb', 'icit_c', 'ICDHyr', 'akg_c', 'AKGDH',
  'succoa_c', 'SUCOAS', 'succ_c', 'SUCDi', 'fum_c', 'FUM', 'mal-L_c', 'MDH'];
const tcaMets = tcaCycle.filter((_, i) => i % 2 === 0);
tcaMets.forEach((key, i) => {
  const angle = 180 - 40 * i;
  const [x, y] = onCircle(TCA.centre, TCA.R, angle);
  met(key, x, y, angle);
});
for (let i = 1; i < tcaCycle.length; i += 2) {
  const id = tcaCycle[i];
  rings.set(id, { ...TCA, a: tcaCycle[i - 1], b: tcaCycle[(i + 1) % tcaCycle.length], inside: true });
  rxn(id, { ring: true });
}
reactions.get('CS').use = ['accoa_c'];
// Acetyl-CoA rises from below the ring and turns into the end of citrate
// synthase from outside, a short hook rather than a run alongside the ring.
reactions.get('CS').bends = { accoa_c: { node: [0, -110], anchor: 25 } };
nodes.get('oaa_c').label = -125;
nodes.get('akg_c').label = -40;
nodes.get('succoa_c').label = 0;
nodes.get('icit_c').label = 0;
nodes.get('succ_c').label = -60;
nodes.get('fum_c').label = -90;
nodes.get('mal-L_c').label = -140;
rxn('PPC', { line: ['pep_c', 'oaa_c'], cof: 'N' });
// The title stands in the middle of the ring, clear of the cofactors inside it.
texts.push({ id: 'tca', text: 'TCA cycle', x: TCA.centre[0] - 64, y: PEP_Y - 4, fontSize: 26 });
texts.push({ id: 'tca:purpose', text: 'Acetyl-CoA to CO2 and NADH', x: TCA.centre[0] - 114, y: PEP_Y + 21, fontSize: 16 });

// Pyruvate dehydrogenase and fermentation under pyruvate
met('accoa_c', 560, PYR_Y, -60);
met('lac-D_c', 140, PYR_Y, -90);
met('for_c', SX, PYR_Y + 200, 180);
met('actp_c', 560, PYR_Y + 160, 180);
met('ac_c', 560, PYR_Y + 320, 180);
rxn('PDH', { line: ['pyr_c', 'accoa_c'], cof: 'N' });
rxn('LDH_D', { line: ['pyr_c', 'lac-D_c'], cof: 'N' });
rxn('PFL', { line: ['pyr_c', 'for_c'], cof: 'E' });
rxn('PTAr', { line: ['accoa_c', 'actp_c'], cof: 'E' });
rxn('ACKr', { line: ['actp_c', 'ac_c'], cof: 'E' });
text('fermentation', 'Fermentation', 'Lactate, formate and acetate from pyruvate', 24, 1660, 24);

// Arginine from 2-oxoglutarate, ending at the inner membrane
// Glutamate stands well clear of the ring, so that the cofactors of GLUDy keep
// out of the way of the labels of AKGDH and ICDHyr.
met('glu-L_c', 1326, ARG_Y, 90);
lane(['glu-L_c', 'ACGS', 'acglu_c', 'ACGK', 'acg5p_c', 'AGPR', 'acg5sa_c', 'ACOTA', 'acorn_c',
  'ACODA', 'orn_c', 'OCBT', 'citr-L_c', 'ARGSS', 'argsuc_c', 'ARGSL', 'arg-L_c'],
[1326, ARG_Y], [140, 0], { cof: 'N', label: 90 });
rxn('GLUDy', { line: ['akg_c', 'glu-L_c'], cof: 'S' });
text('arg', 'Arginine biosynthesis', '2-Oxoglutarate via glutamate and ornithine (ARGSL is argH)', 1240, ARG_Y - 147, 24);

// Lysine and methionine from aspartate. The aspartate route starts at a second
// drawing of oxaloacetate: the first stands on the ring between PPC and CS.
met('oaa_c#asp', 900, LYS_Y, 90, 'oaa_c');
lane(['oaa_c#asp', 'ASPTA', 'asp-L_c', 'ASPK', '4pasp_c', 'ASAD', 'aspsa_c', 'DHDPS', '23dhdp_c', 'DHDPRy',
  'thdp_c', 'THDPS', 'sl2a6o_c', 'SDPTA', 'sl26da_c', 'SDPDS', '26dap-LL_c', 'DAPE', '26dap-M_c', 'DAPDC', 'lys-L_c'],
[900, LYS_Y], [150, 0], { cof: 'N', label: 90 });
met('hom-L_c', 1350, MET_Y, 90);
rxn('HSDy', { line: ['aspsa_c', 'hom-L_c'], cof: 'W' });
lane(['hom-L_c', 'HSST', 'suchms_c', 'SHSL1', 'cyst-L_c', 'CYSTL', 'hcys-L_c', 'METS', 'met-L_c'],
  [1350, MET_Y], [150, 0], { cof: 'N', label: 90 });
nodes.get('met-L_c').label = -45;
text('lys', 'Lysine biosynthesis', 'Aspartate semialdehyde via diaminopimelate (DAPDC is lysA)', 1560, LYS_Y - 155, 24);
text('met', 'Methionine biosynthesis', 'Homoserine via cystathionine (HSST is metA)', 880, MET_Y + 64, 24);

// Transport. `chain` runs from the cytosol to the medium along one edge: the
// inner-membrane transporters (spread along the membrane when there are
// several), the periplasmic metabolite, the porin, the extracellular
// metabolite, the exchange on the cell boundary and the cell's medium node.
const chains = [];
function chain(edge, along, { inner, cyt, p, porin, e, ex, spread = 80, shift = 0 }) {
  chains.push({ edge, along, inner, cyt, p, porin, e, ex, spread, shift });
}
chain('top', SX, { inner: ['GLCptspp'], cyt: 'g6p_c', p: 'glc-D_p', porin: 'GLCtex', e: 'glc-D_e', ex: 'EX_glc_e' });
chain('top', 1000, { inner: ['CO2tpp'], cyt: 'co2_c', p: 'co2_p', porin: 'CO2tex', e: 'co2_e', ex: 'EX_co2_e' });
chain('top', 1130, { inner: ['NH4tpp'], cyt: 'nh4_c', p: 'nh4_p', porin: 'NH4tex', e: 'nh4_e', ex: 'EX_nh4_e' });
chain('top', 1560, { inner: ['O2tpp'], cyt: 'o2_c', p: 'o2_p', porin: 'O2tex', e: 'o2_e', ex: 'EX_o2_e' });
// PHEt2rpp stands a little off the line, so that its label, written to the
// right of the marker, passes the periplasmic node instead of crossing it.
chain('right', PHE_Y, { inner: ['PHEt2rpp'], cyt: 'phe-L_c', p: 'phe-L_p', porin: 'PHEtex', e: 'phe-L_e', ex: 'EX_phe_L_e', shift: -40 });
chain('right', ARG_Y, { inner: ['ARGt3pp', 'ARGORNt7pp'], cyt: 'arg-L_c', p: 'arg-L_p', porin: 'ARGtex', e: 'arg-L_e', ex: 'EX_arg_L_e' });
chain('right', LYS_Y, { inner: ['LYSt3pp', 'LYSt2pp'], cyt: 'lys-L_c', p: 'lys-L_p', porin: 'LYStex', e: 'lys-L_e', ex: 'EX_lys_L_e' });
// Labels are written to the right of a marker: on a bottom edge the two
// transporters stand far enough apart for the first one's label.
chain('bottom', 1950, { inner: ['METt3pp', 'METabcpp'], cyt: 'met-L_c', p: 'met-L_p', porin: 'METtex', e: 'met-L_e', ex: 'EX_met_L_e', spread: 160 });
met('co2_c', 1000, 150, 0);
met('nh4_c', 1130, 150, 0);
met('o2_c', 1560, 150, 180);

// Respiratory chain and ATP synthase on the inner membrane, axis along it
rxn('NADH16pp', { membrane: 'top', along: 1330, tangent: true });
rxn('CYTBO3_4pp', { membrane: 'top', along: 1730, tangent: true, use: ['o2_c'] });
rxn('ATPS4rpp', { membrane: 'top', along: 2080, tangent: true });
text('resp', 'Respiration and ATP synthesis', 'NADH dehydrogenase, cytochrome bo3, ATP synthase', 1290, 230, 24);

// Building one cell

function speciesModel(doc) { return new Model(doc); }

/** Every metabolite of the merged reaction drawn as a node, except the growth link. */
function stoichiometry(community, id) {
  const r = community.merged.reactions.get(id);
  if (!r) throw new Error(`The model has no reaction ${id}`);
  return Object.entries(r.metabolites).filter(([mid]) => community.merged.metabolites.get(mid)?.compartment !== LINK_COMPARTMENT);
}

function buildCell(map, community, cellId, origin) {
  const [ox, oy] = origin;
  const at = (x, y) => [ox + x, oy + y];
  const cid = (local) => `${cellId}@${local}`;
  const placed = new Map();   // template key -> map node
  const ringNode = new Map(); // map node id -> angle on the TCA ring

  const metaboliteInfo = (mid) => {
    const m = community.merged.metabolites.get(mid);
    if (!m) throw new Error(`The model has no metabolite ${mid}`);
    return m;
  };
  function addNode(id, mid, [x, y], { secondary = false, label, kind } = {}) {
    if (map.nodes.has(id)) return map.nodes.get(id);
    const m = metaboliteInfo(mid);
    const owner = community.metaboliteOwner.get(mid);
    const node = {
      id, kind: kind ?? (secondary ? 'currency' : 'metabolite'), mergedId: mid,
      localId: owner?.originalId ?? mid, base: m.base, name: m.name, formula: m.formula || undefined,
      compartment: m.compartment, cellId: owner?.speciesId ?? null,
      x: r2(x), y: r2(y), r: secondary ? 7 : 13, primary: !secondary, visualRole: secondary ? 'secondary' : 'primary',
      ...(label === undefined ? {} : { labelAngle: r2(rad(label) * 1e4) / 1e4 }),
    };
    map.nodes.set(id, node);
    if (node.kind === 'medium') map.medium.nodes.push(id);
    return node;
  }
  for (const n of nodes.values()) {
    const id = n.key.includes('#') ? `met:${cid(n.mid)}:copy:${n.key.split('#')[1]}` : `met:${cid(n.mid)}`;
    placed.set(n.key, addNode(id, cid(n.mid), at(n.x, n.y), { label: n.label }));
  }
  for (const [key, node] of placed) {
    if (tcaMets.includes(key)) ringNode.set(node.id, angleOf(at(...TCA.centre), [node.x, node.y]));
  }

  // The reaction record, its axis and its branches. `main` maps a metabolite to
  // the drawn node its branch reaches; every other metabolite gets a small node
  // of this reaction's own, at `cofactorAt(mid, side, index)` (x, y and, when
  // it is given, the angle of its label in degrees). A branch leaves
  // the substrates' or the products' end of the axis, unless `anchorOf` names
  // another end for it.
  function addReaction(local, { centre, dir, main, cofactorAt, membrane = null, normal = null, linear = null, curve, anchorOf }) {
    const id = cid(local);
    const src = community.merged.reactions.get(id);
    const owner = community.reactionOwner.get(id);
    const [cx, cy] = centre;
    const a1 = [cx - dir[0] * AXIS, cy - dir[1] * AXIS];
    const a2 = [cx + dir[0] * AXIS, cy + dir[1] * AXIS];
    const record = {
      id, localId: local, name: src.name, subsystem: src.subsystem ?? '', cellId,
      kind: owner?.role ?? src.kind, compartments: [...src.compartments],
      membrane: membrane ?? null, boundaryId: membrane ? boundaryKey(cellId, membrane) : null,
      onMembrane: Boolean(membrane), normal: normal ? pt(...normal) : null,
      x: r2(cx), y: r2(cy), equation: src.equation(),
      bounds: { lower: src.lowerBound, upper: src.upperBound },
      axis: { a1: pt(...a1), a2: pt(...a2), dir: pt(...dir) },
      ...(linear ? { linear } : {}),
    };
    map.reactions.set(id, record);
    map.segments.push({ reactionId: id, kind: 'axis', from: record.axis.a1, to: record.axis.a2, arrow: false });
    const count = { substrate: 0, product: 0 };
    const own = { substrate: [], product: [] };
    for (const [mid, coefficient] of stoichiometry(community, id)) {
      const side = coefficient < 0 ? 'substrate' : 'product';
      if (main.has(mid)) continue;
      own[side].push(mid);
    }
    const rank = (mid) => {
      const base = metaboliteInfo(mid).base;
      const i = COFACTOR_ORDER.indexOf(base);
      return i < 0 ? COFACTOR_ORDER.length : i;
    };
    const cofactorNode = new Map();
    for (const side of ['substrate', 'product']) {
      own[side].sort((a, b) => rank(a) - rank(b) || a.localeCompare(b));
      own[side].forEach((mid, i) => {
        const [x, y, label] = cofactorAt(mid, side, i);
        cofactorNode.set(mid, addNode(`cur:${id}:${mid}`, mid, [x, y], { secondary: true, label }));
      });
    }
    for (const [mid, coefficient] of stoichiometry(community, id)) {
      const side = coefficient < 0 ? 'substrate' : 'product';
      const node = main.get(mid) ?? cofactorNode.get(mid);
      count[side] += 1;
      const end = anchorOf?.(mid);
      const anchor = end?.anchor ?? (side === 'substrate' ? a1 : a2);
      const u = end?.u ?? (side === 'substrate' ? [-dir[0], -dir[1]] : dir);
      const here = [node.x, node.y];
      const [b1, b2] = (main.has(mid) && curve?.(node, anchor, u, mid)) || sideCurve(here, anchor, u);
      map.segments.push({
        reactionId: id, kind: node.primary ? 'primary' : 'currency', nodeId: node.id, coefficient, side,
        from: side === 'substrate' ? pt(...here) : pt(...anchor),
        to: side === 'substrate' ? pt(...anchor) : pt(...here),
        b1: pt(...b1), b2: pt(...b2), arrow: side === 'product', backboneAnchor: true,
      });
    }
    return record;
  }

  /** The model's side of a metabolite in a reaction. */
  const sideOf = (local, mid) => {
    const c = community.merged.reactions.get(cid(local)).metabolites[mid];
    if (c === undefined) throw new Error(`${local} has no ${mid}`);
    return c < 0 ? 'substrate' : 'product';
  };
  const keyNode = (key) => {
    const n = placed.get(key);
    if (!n) throw new Error(`No node ${key}`);
    return n;
  };
  /** Cofactors of a metabolic reaction on one hand of its axis, by the template. */
  const templateAt = (centre, dir, hand) => (mid, side, i) => {
    const sign = side === 'substrate' ? -1 : 1;
    let row = i, h = hand;
    if (row >= COFACTOR_ROWS.length) { row -= COFACTOR_ROWS.length; h = -hand; }
    const [along, across] = COFACTOR_ROWS[Math.min(row, COFACTOR_ROWS.length - 1)];
    const p = [-dir[1], dir[0]];
    return [centre[0] + dir[0] * along * sign + p[0] * across * h, centre[1] + dir[1] * along * sign + p[1] * across * h];
  };
  /** A branch shaped by hand (`bends`): handles offset from the node and along the axis from its end. */
  const bendOf = (spec, node, anchor, u) => {
    const b = spec.bends && Object.entries(spec.bends).find(([key]) => node.mergedId === cid(key))?.[1];
    if (!b) return null;
    return [[node.x + b.node[0], node.y + b.node[1]], [anchor[0] + u[0] * b.anchor, anchor[1] + u[1] * b.anchor]];
  };
  const handOf = (dir, compass) => {
    const p = [-dir[1], dir[0]];
    const s = COMPASS[compass ?? 'E'];
    return p[0] * s[0] + p[1] * s[1] < 0 ? -1 : 1;
  };

  // Metabolic reactions: on their line, at a place, or on the ring
  for (const spec of reactions.values()) {
    if (spec.membrane) continue;
    const local = spec.id;
    const main = new Map();
    for (const key of [...(spec.line ?? []), ...(spec.use ?? [])]) {
      const node = keyNode(key);
      main.set(node.mergedId, node);
    }
    if (spec.ring) {
      const ring = rings.get(local);
      const A = keyNode(ring.a), B = keyNode(ring.b);
      main.set(A.mergedId, A);
      main.set(B.mergedId, B);
      const c = at(...ring.centre);
      const aa = ringNode.get(A.id), bb = ringNode.get(B.id);
      const middle = aa + wrap(bb - aa) / 2;
      let sense = wrap(bb - aa) > 0 ? 1 : -1;
      if (sideOf(local, A.mergedId) !== 'substrate') sense = -sense;
      const t = tangent(middle);
      const dir = [t[0] * sense, t[1] * sense];
      const centre = onCircle(c, ring.R, middle);
      const hand = ring.inside ? -1 : 1;
      addReaction(local, {
        centre, dir, main,
        cofactorAt: (mid, side, i) => {
          const sign = side === 'substrate' ? -1 : 1;
          let row = i, h = hand;
          if (row >= COFACTOR_ROWS.length) { row -= COFACTOR_ROWS.length; h = -hand; }
          const [along, across] = COFACTOR_ROWS[row];
          const turn = (along / ring.R) * (180 / Math.PI) * sign * sense;
          return onCircle(c, ring.R + across * h, middle + turn);
        },
        curve: (node, anchor, u) => {
          const angle = ringNode.get(node.id);
          if (angle === undefined) return bendOf(spec, node, anchor, u);
          const span = wrap(angleOf(c, anchor) - angle);
          const h = arcHandle(span, ring.R);
          const tn = tangent(angle);
          const toward = span < 0 ? -1 : 1;
          return [[node.x + tn[0] * toward * h, node.y + tn[1] * toward * h], [anchor[0] + u[0] * h, anchor[1] + u[1] * h]];
        },
      });
      continue;
    }
    let centre, dir, linear = null, offset = 0;
    if (spec.line) {
      const [A, B] = spec.line.map(keyNode);
      // The axis runs from the model's substrate to its product.
      const [S, P] = sideOf(local, A.mergedId) === 'substrate' ? [A, B] : [B, A];
      dir = unit(P.x - S.x, P.y - S.y);
      const t = spec.t ?? 0.5;
      const along = lerp([A.x, A.y], [B.x, B.y], t);
      offset = spec.offset ?? 0;
      const perp = unit(-(B.y - A.y), B.x - A.x);
      centre = [along[0] + perp[0] * offset, along[1] + perp[1] * offset];
      if (!offset) linear = { from: S.id, to: P.id, t: S === A ? t : 1 - t };
    } else {
      centre = at(...spec.at);
      dir = unit(...spec.dir);
    }
    const lineIds = new Set((spec.line ?? []).map((k) => keyNode(k).id));
    addReaction(local, {
      centre, dir, main, linear,
      cofactorAt: templateAt(centre, dir, handOf(dir, spec.cof)),
      curve: (node, anchor, u) => {
        const bent = bendOf(spec, node, anchor, u);
        if (bent) return bent;
        if (spec.fork) return forkCurve([node.x, node.y], anchor, u);
        if (!lineIds.has(node.id)) return null;
        return offset ? arcCurve([node.x, node.y], anchor, u) : straight([node.x, node.y], anchor);
      },
    });
  }

  // Transport chains, one per exchanged metabolite
  const edgePoint = (edge, along, depth) => {
    // depth: 0 on the cytosol's edge, positive outwards
    if (edge === 'top') return at(along, -depth);
    if (edge === 'bottom') return at(along, H + depth);
    if (edge === 'right') return at(W + depth, along);
    return at(-depth, along);
  };
  const tangentOf = (edge) => (edge === 'top' || edge === 'bottom' ? [1, 0] : [0, 1]);
  for (const c of chains) {
    const n = EDGES[c.edge];
    const tdir = tangentOf(c.edge);
    const cyt = placed.get(c.cyt) ?? (() => { throw new Error(`No node ${c.cyt}`); })();
    const pNode = addNode(`met:${cid(c.p)}`, cid(c.p), edgePoint(c.edge, c.along, BAND / 2), { label: c.edge === 'right' ? -90 : 0 });
    const eNode = addNode(`met:${cid(c.e)}`, cid(c.e), edgePoint(c.edge, c.along, BAND + eBand(c.edge) / 2), { label: c.edge === 'right' ? -90 : 0 });
    const mediumMid = sharedMetabolite(metaboliteInfo(cid(c.e)).base);
    const exId = cid(c.ex);
    const mNode = addNode(`met:${exId}:${mediumMid}`, mediumMid, edgePoint(c.edge, c.along, BAND + eBand(c.edge) + OUT[c.edge]),
      { kind: 'medium', label: c.edge === 'right' ? 0 : c.edge === 'top' ? -90 : 90 });
    mNode.cellId = null;
    mNode.reactionLocalFor = exId;
    mNode.reactionLocal = true;
    // Several inner-membrane transporters of one metabolite stand side by side.
    c.inner.forEach((local, i) => {
      const shift = (i - (c.inner.length - 1) / 2) * c.spread + c.shift;
      const centre = edgePoint(c.edge, c.along + shift, 0);
      const inward = sideOf(local, cyt.mergedId) === 'product';
      const dir = inward ? [-n[0], -n[1]] : n;
      const main = new Map([[cyt.mergedId, cyt], [pNode.mergedId, pNode]]);
      let k = { in: 0, out: 0 };
      // Each metabolite branches from the end of the axis on its own side of
      // the membrane, so no branch crosses the membrane to reach its node.
      const outer = edgePoint(c.edge, c.along + shift, AXIS), inner = edgePoint(c.edge, c.along + shift, -AXIS);
      addReaction(local, {
        centre, dir, main, membrane: 'c', normal: n,
        anchorOf: (mid) => (metaboliteInfo(mid).compartment === 'c'
          ? { anchor: inner, u: [-n[0], -n[1]] } : { anchor: outer, u: n }),
        // The cytosolic cofactors in one column beside the axis, one below the
        // other, so each branch peels off the axis clear of the ones before it
        // and the labels stand beside their nodes; the periplasmic ones in the
        // middle of the band.
        cofactorAt: (mid) => {
          const outside = metaboliteInfo(mid).compartment !== 'c';
          const sideSign = shift < 0 ? -1 : shift > 0 ? 1 : (outside ? 1 : -1);
          const j = outside ? k.out++ : k.in++;
          const depth = outside ? BAND / 2 : -(44 + 28 * j);
          const off = sideSign * (outside ? 34 + 28 * (j % 2) : 40);
          const base = edgePoint(c.edge, c.along + shift, depth);
          const away = Math.atan2(tdir[1] * sideSign, tdir[0] * sideSign) * 180 / Math.PI;
          return [base[0] + tdir[0] * off, base[1] + tdir[1] * off, outside ? undefined : away];
        },
      });
    });
    addReaction(c.porin, {
      centre: edgePoint(c.edge, c.along, BAND), dir: [-n[0], -n[1]], membrane: 'p', normal: n,
      main: new Map([[pNode.mergedId, pNode], [eNode.mergedId, eNode]]), cofactorAt: () => { throw new Error(`${c.porin} has a cofactor`); },
    });
    addReaction(c.ex, {
      centre: edgePoint(c.edge, c.along, BAND + eBand(c.edge)), dir: n, membrane: 'e', normal: n,
      main: new Map([[eNode.mergedId, eNode], [mediumMid, mNode]]), cofactorAt: () => { throw new Error(`${c.ex} has a cofactor`); },
    });
  }

  // Respiratory chain, along the membrane: substrates on its left end, products
  // on its right. A cytosolic cofactor's branch comes down onto it from above,
  // so its name goes underneath.
  for (const spec of reactions.values()) {
    if (!spec.membrane) continue;
    const n = EDGES[spec.membrane];
    const dir = tangentOf(spec.membrane);
    const centre = edgePoint(spec.membrane, spec.along, 0);
    const main = new Map((spec.use ?? []).map((key) => [keyNode(key).mergedId, keyNode(key)]));
    const k = { substrate: { in: 0, out: 0 }, product: { in: 0, out: 0 } };
    addReaction(spec.id, {
      centre, dir, main, membrane: 'c', normal: n,
      cofactorAt: (mid, side) => {
        const outside = metaboliteInfo(mid).compartment !== 'c';
        const sign = side === 'substrate' ? -1 : 1;
        const j = outside ? k[side].out++ : k[side].in++;
        const depth = outside ? BAND / 2 : -(44 + 30 * j);
        const off = sign * (outside ? 40 : 30 + 26 * j);
        const base = edgePoint(spec.membrane, spec.along + off, depth);
        return [...base, outside ? undefined : 90];
      },
    });
  }

  // Module titles, in every cell
  for (const t of texts) map.texts.set(`${cellId}:${t.id}`, { id: `${cellId}:${t.id}`, text: t.text, ...pt(...at(t.x, t.y)), fontSize: t.fontSize });
}

// The cells, the medium and the document

function cellShell(id, origin, model) {
  const [ox, oy] = origin;
  const c = { x: ox, y: oy, w: W, h: H };
  const p = { x: ox - BAND, y: oy - BAND, w: W + 2 * BAND, h: H + 2 * BAND };
  const e = { x: ox - 2 * BAND, y: oy - 2 * BAND, w: W + 3 * BAND + E_RIGHT, h: H + 4 * BAND };
  const s = STRAINS[id];
  return {
    id, name: `${id} · needs ${s.needs} · supplies ${s.supplies}`,
    organism: 'Escherichia coli K-12 MG1655 (iAF1260)',
    note: `Auxotroph for ${s.needs}; ${s.blocked} closed, so it supplies ${s.supplies}`,
    color: COLOURS[id], abundance: 0.25, rect: { ...e }, outerCompartment: 'e',
    compartments: [
      { id: 'e', rect: { ...e }, depth: 0, parentId: null, label: model.compartmentNames.e ?? 'Extracellular', membrane: 'open', visualId: 'e' },
      { id: 'p', rect: { ...p }, depth: 1, parentId: 'e', label: model.compartmentNames.p ?? 'Periplasm', membrane: 'bilayer', visualId: 'p' },
      { id: 'c', rect: { ...c }, depth: 2, parentId: 'p', label: model.compartmentNames.c ?? 'Cytosol', membrane: 'bilayer', visualId: 'c' },
    ],
  };
}

/** Reactions closed in some strains and open in others: the tutorial's knockouts. */
function knockoutsFromBounds(csv, ids) {
  const lines = csv.trim().split(/\r?\n/);
  const head = lines[0].split(',');
  const col = (name) => head.indexOf(name);
  const out = Object.fromEntries(ids.map((id) => [id, []]));
  for (const line of lines.slice(1)) {
    const cells = line.split(',');
    const closed = ids.filter((id) => Number(cells[col(`${id}_lb`)]) === 0 && Number(cells[col(`${id}_ub`)]) === 0);
    if (closed.length && closed.length < ids.length) for (const id of closed) out[id].push(cells[0]);
  }
  for (const id of ids) out[id].sort();
  return out;
}

export async function buildAuxotrophMap() {
  const manifest = JSON.parse(await readFile(join(DIR, 'manifest.json'), 'utf8'));
  const species = [];
  for (const member of manifest.members) {
    species.push({ id: member.id, file: `published/${COMMUNITY}/${member.model.replace(/\.json$/, '')}`,
      model: speciesModel(JSON.parse(await readFile(join(DIR, member.model), 'utf8'))) });
  }
  // As the store builds it: equal abundances, the members' own bounds.
  const community = new CommunityModel({
    name: COMMUNITY, species: species.map((s) => ({ id: s.id, model: s.model, abundance: 1 / species.length, enabled: true })),
  });

  const cellW = W + 3 * BAND + E_RIGHT, cellH = H + 4 * BAND;
  const width = 2 * MARGIN + 2 * cellW + GAP;
  const height = HEADER + 2 * MARGIN + 2 * cellH + GAP;
  const map = new EditableMap({
    canvas: { width, height },
    environment: { id: 's', label: 'Shared medium', rect: { x: 20, y: HEADER - 40, w: width - 40, h: height - HEADER + 20 } },
    cells: [], medium: { nodes: [], exchanges: [] },
  });
  GRID.forEach((row, r) => row.forEach((id, k) => {
    const sp = species.find((s) => s.id === id);
    const ex = MARGIN + k * (cellW + GAP), ey = HEADER + MARGIN + r * (cellH + GAP);
    const origin = [ex + 2 * BAND, ey + 2 * BAND];
    map.cells.push(cellShell(id, origin, sp.model));
    buildCell(map, community, id, origin);
  }));
  map.texts.set('title', { id: 'title', text: 'Four auxotrophic E. coli strains (SteadyCom; Chan et al. 2017)', x: MARGIN, y: 92, fontSize: 64 });
  map.texts.set('subtitle', { id: 'subtitle',
    text: 'One curated iAF1260 central-metabolism map per strain; each strain\'s knockouts are closed in its own model and drawn as knockouts. The simulation uses the complete models.',
    x: MARGIN, y: 150, fontSize: 30 });
  // The maps of EditableMap derive boundaries from the cells: rebuild them now
  // the cells are in.
  const finished = EditableMap.fromJSON(map.toJSON());

  const doc = createMapDocument(finished, {
    scenario: `published:${COMMUNITY}`,
    species: species.map((s) => ({ id: s.id, file: s.file, enabled: true })),
  });
  doc.metadata.createdAt = null;
  doc.provenance = {
    builder: 'tools/scenario-maps/build-auxotrophs.mjs',
    source: 'model-Ec1..4.json (iAF1260 with the SteadyCom tutorial knockouts)',
    layout: 'one strain template placed four times; the strains that trade are neighbours',
  };
  return { doc, map: finished, community, knockouts: knockoutsFromBounds(await readFile(join(DIR, 'bounds.csv'), 'utf8'), species.map((s) => s.id)) };
}

async function main() {
  const { doc, map, knockouts } = await buildAuxotrophMap();
  await writeFile(join(DIR, 'map.json'), JSON.stringify(doc, null, 1) + '\n');
  // Point the manifest at the map and list the knockouts, leaving the rest of it as it is.
  const manifestPath = join(DIR, 'manifest.json');
  const raw = await readFile(manifestPath, 'utf8');
  const manifest = JSON.parse(raw);
  manifest.map = 'map.json';
  manifest.knockouts = knockouts;
  const eol = raw.includes('\r\n') ? '\r\n' : '\n';
  await writeFile(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`.replace(/\n/g, eol));
  const perCell = map.reactions.size / map.cells.length;
  console.log(`map.json: ${map.reactions.size} reactions (${perCell} per cell), ${map.nodes.size} nodes, `
    + `canvas ${map.width} x ${map.height}; knockouts ${JSON.stringify(knockouts)}`);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) await main();
