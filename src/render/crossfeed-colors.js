/**
 * Cross-feeding line colours: each exchanged metabolite has one colour, the
 * same on every map, so a colour names a metabolite wherever its lines go.
 *
 * The exchanges that matter most between species (the short-chain fatty acids,
 * lactate, the other fermentation products, glycerol and glucose) each have a
 * colour no other metabolite gets: acetate is always blue, glycerol always red.
 * Sugars, amino acids and the other common exchanges share a second set of
 * colours, and any other metabolite is placed in that set by a hash of its id.
 * On one map two metabolites that would share a colour are told apart: the one
 * later in the order below takes the next free colour of the shared set.
 *
 * Every colour has a lighter variant for the dark theme, and both keep at least
 * 3:1 contrast with the shared medium they are drawn over.
 */

/** [light, dark] per colour. */
const PALETTE = {
  blue: ['#2563eb', '#60a5fa'],
  red: ['#dc2626', '#f87171'],
  green: ['#16a34a', '#4ade80'],
  lime: ['#4d7c0f', '#bef264'],
  orange: ['#ea580c', '#fb923c'],
  violet: ['#7c3aed', '#a78bfa'],
  fuchsia: ['#c026d3', '#e879f9'],
  teal: ['#0d9488', '#2dd4bf'],
  gold: ['#a16207', '#facc15'],
  sky: ['#0284c7', '#7dd3fc'],
  slate: ['#475569', '#cbd5e1'],
  brown: ['#92400e', '#d6a77a'],
  indigo: ['#3730a3', '#a5b4fc'],
  // the shared set
  rose: ['#be123c', '#fda4af'],
  pink: ['#db2777', '#f472b6'],
  cyan: ['#155e75', '#67e8f9'],
  emerald: ['#065f46', '#bbf7d0'],
  maroon: ['#7f1d1d', '#fed7aa'],
  plum: ['#701a75', '#f5d0fe'],
  khaki: ['#7c6f2a', '#fef08a'],
};

const SHARED = ['rose', 'pink', 'cyan', 'emerald', 'maroon', 'plum', 'khaki'];

/**
 * Metabolites with a colour of their own, by key (see metaboliteKey), with the
 * BiGG and ModelSEED spellings of each.
 */
const OWN = {
  ac: 'blue', cpd00029: 'blue',
  glyc: 'red', cpd00100: 'red',
  lac: 'green', lacl: 'green', cpd00159: 'green',
  lacd: 'lime', cpd00221: 'lime',
  but: 'orange', cpd00211: 'orange',
  ppa: 'violet', cpd00141: 'violet',
  succ: 'fuchsia', cpd00036: 'fuchsia',
  for: 'teal', cpd00047: 'teal',
  etoh: 'gold', cpd00363: 'gold',
  h2: 'sky', cpd11640: 'sky',
  co2: 'slate', cpd00011: 'slate',
  pyr: 'brown', cpd00020: 'brown',
  glcd: 'indigo', cpd00027: 'indigo',
};

/**
 * Common exchanges given a colour of the shared set. Within each group the
 * ones most often exchanged together come first and get different colours.
 */
const SHARED_TABLE = {
  // sugars and other carbon sources
  fru: 'rose', gal: 'pink', man: 'cyan', xyld: 'emerald', arabl: 'maroon', ribd: 'plum', cellb: 'khaki',
  malt: 'rose', sucr: 'pink', lcts: 'cyan', rmn: 'emerald', fucl: 'maroon', acgam: 'plum', glcur: 'khaki', galur: 'rose',
  // amino acids
  glul: 'pink', glnl: 'cyan', aspl: 'emerald', asnl: 'maroon', alal: 'plum', gly: 'khaki', serl: 'rose',
  thrl: 'pink', prol: 'cyan', vall: 'emerald', leul: 'maroon', ilel: 'plum', metl: 'khaki', cysl: 'rose',
  lysl: 'pink', argl: 'cyan', hisl: 'emerald', phel: 'maroon', tyrl: 'plum', trpl: 'khaki',
  // other fermentation products and common exchanges
  nh4: 'cyan', fum: 'emerald', mall: 'maroon', cit: 'plum', acald: 'khaki', meoh: 'rose',
  '12ppds': 'pink', isobut: 'cyan', isoval: 'emerald', '2mba': 'maroon', ch4: 'plum', h2s: 'khaki',
};

/** The id a table entry matches: lower case, letters and digits only, so lac__L, lac_L and lac-L are one. */
const metaboliteKey = (base) => String(base ?? '').toLowerCase().replace(/[^a-z0-9]/g, '');

// The same string hash as for unknown compartments, so it is stable everywhere.
function stableHash(text) {
  let h = 0;
  for (let i = 0; i < text.length; i++) h = (h * 31 + text.charCodeAt(i)) >>> 0;
  return h;
}

/** The colour a metabolite asks for, and its tier: 0 its own, 1 shared from the table, 2 hashed. */
function colorName(base) {
  const key = metaboliteKey(base);
  if (Object.hasOwn(OWN, key)) return { tier: 0, name: OWN[key] };
  if (Object.hasOwn(SHARED_TABLE, key)) return { tier: 1, name: SHARED_TABLE[key] };
  return { tier: 2, name: SHARED[stableHash(key) % SHARED.length] };
}

const hex = (name, dark) => PALETTE[name][dark ? 1 : 0];

/** The colour of a metabolite's cross-feeding lines taken on its own. */
export function crossFeedingColor(base, dark = false) {
  return hex(colorName(base).name, dark);
}

/**
 * The colours of the metabolites drawn together on one map, as a Map from base
 * to colour: each its own colour (crossFeedingColor), except that a colour
 * already taken sends the later metabolite to the next free colour of the
 * shared set. Metabolites with a colour of their own come first, then the
 * table's, then the rest, each in id order; flux plays no part, so a new
 * solution does not repaint the lines. Once every shared colour is taken,
 * colours repeat.
 */
export function crossFeedingColors(bases, dark = false) {
  const entries = [...new Set(bases)].map((base) => ({ base, ...colorName(base) }))
    .sort((a, b) => a.tier - b.tier || (a.base < b.base ? -1 : a.base > b.base ? 1 : 0));
  const used = new Set();
  const out = new Map();
  for (const entry of entries) {
    let name = entry.name;
    if (used.has(name)) {
      const from = entry.tier === 0 ? 0 : SHARED.indexOf(name) + 1;
      for (let k = 0; k < SHARED.length; k++) {
        const next = SHARED[(from + k) % SHARED.length];
        if (!used.has(next)) { name = next; break; }
      }
    }
    used.add(name);
    out.set(entry.base, hex(name, dark));
  }
  return out;
}
