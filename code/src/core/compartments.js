/**
 * Compartment registry.
 *
 * Every compartment that the renderer draws gets an explicit visual identity:
 * a fill, a border (the "membrane"), a text colour and a short label. Colours
 * are chosen so that neighbouring compartments never share a hue and so that
 * every fill/text pair clears WCAG AA at 14px.
 *
 * Compartment ids follow the BiGG convention (`c`, `e`, `p`, `m`, ...). Unknown
 * ids fall back to a deterministic slot in EXTRA_PALETTE so that a user-supplied
 * model still renders with stable, distinct colours.
 */

/** @typedef {{fill:string, fillDark:string, stroke:string, strokeDark:string, text:string, textDark:string}} CompartmentTheme */

const T = (fill, fillDark, stroke, strokeDark, text, textDark) => ({
  fill, fillDark, stroke, strokeDark, text, textDark,
});

/**
 * Canonical compartments. `membrane` describes how the boundary is drawn:
 *   - `bilayer`  : double stroke, used for true lipid membranes
 *   - `wall`     : thick single stroke with a hatch, used for cell walls
 *   - `open`     : dashed stroke, used for spaces that are not membrane-bound
 */
export const COMPARTMENTS = {
  e:  { id: 'e',  name: 'Extracellular',            es: 'Extracelular',              membrane: 'open',    theme: T('#f1f5f9', '#1e293b', '#94a3b8', '#64748b', '#475569', '#cbd5e1') },
  s:  { id: 's',  name: 'Shared medium',            es: 'Medio compartido',          membrane: 'open',    theme: T('#f8fafc', '#0f172a', '#cbd5e1', '#475569', '#64748b', '#94a3b8') },
  w:  { id: 'w',  name: 'Cell wall',                es: 'Pared celular',             membrane: 'wall',    theme: T('#f5f5f4', '#292524', '#a8a29e', '#78716c', '#57534e', '#d6d3d1') },
  p:  { id: 'p',  name: 'Periplasm',                es: 'Periplasma',                membrane: 'bilayer', theme: T('#fef3c7', '#422006', '#d97706', '#b45309', '#92400e', '#fcd34d') },
  c:  { id: 'c',  name: 'Cytosol',                  es: 'Citosol',                   membrane: 'bilayer', theme: T('#e0f2fe', '#082f49', '#0284c7', '#0369a1', '#075985', '#7dd3fc') },
  im: { id: 'im', name: 'Intermembrane space',      es: 'Espacio intermembrana',     membrane: 'bilayer', theme: T('#ffe4e6', '#4c0519', '#fb7185', '#e11d48', '#9f1239', '#fda4af') },
  m:  { id: 'm',  name: 'Mitochondrial matrix',     es: 'Matriz mitocondrial',       membrane: 'bilayer', theme: T('#fee2e2', '#450a0a', '#dc2626', '#b91c1c', '#991b1b', '#fca5a5') },
  h:  { id: 'h',  name: 'Chloroplast stroma',       es: 'Estroma del cloroplasto',   membrane: 'bilayer', theme: T('#dcfce7', '#052e16', '#16a34a', '#15803d', '#166534', '#86efac') },
  u:  { id: 'u',  name: 'Thylakoid lumen',          es: 'Lumen tilacoidal',          membrane: 'bilayer', theme: T('#d1fae5', '#022c22', '#059669', '#047857', '#065f46', '#6ee7b7') },
  n:  { id: 'n',  name: 'Nucleus',                  es: 'Núcleo',                    membrane: 'bilayer', theme: T('#ede9fe', '#2e1065', '#7c3aed', '#6d28d9', '#5b21b6', '#c4b5fd') },
  x:  { id: 'x',  name: 'Peroxisome',               es: 'Peroxisoma',                membrane: 'bilayer', theme: T('#ffedd5', '#431407', '#ea580c', '#c2410c', '#9a3412', '#fdba74') },
  r:  { id: 'r',  name: 'Endoplasmic reticulum',    es: 'Retículo endoplásmico',     membrane: 'bilayer', theme: T('#fae8ff', '#4a044e', '#c026d3', '#a21caf', '#86198f', '#f0abfc') },
  g:  { id: 'g',  name: 'Golgi apparatus',          es: 'Aparato de Golgi',          membrane: 'bilayer', theme: T('#fce7f3', '#500724', '#db2777', '#be185d', '#9d174d', '#f9a8d4') },
  v:  { id: 'v',  name: 'Vacuole',                  es: 'Vacuola',                   membrane: 'bilayer', theme: T('#cffafe', '#083344', '#0891b2', '#0e7490', '#155e75', '#67e8f9') },
  l:  { id: 'l',  name: 'Lysosome',                 es: 'Lisosoma',                  membrane: 'bilayer', theme: T('#ecfccb', '#1a2e05', '#65a30d', '#4d7c0f', '#3f6212', '#bef264') },
  cx: { id: 'cx', name: 'Carboxysome',              es: 'Carboxisoma',               membrane: 'bilayer', theme: T('#e0e7ff', '#1e1b4b', '#4f46e5', '#4338ca', '#3730a3', '#a5b4fc') },
  f:  { id: 'f',  name: 'Flagellum',                es: 'Flagelo',                   membrane: 'open',    theme: T('#f0fdfa', '#042f2e', '#14b8a6', '#0d9488', '#0f766e', '#5eead4') },
  lp: { id: 'lp', name: 'Lipid particle',            es: 'Partícula lipídica',        membrane: 'bilayer', theme: T('#fef9c3', '#422006', '#ca8a04', '#a16207', '#854d0e', '#fde047') },
  // BiGG writes the eyespot as `s`, which between cells is the shared medium.
  // Inside a cell it is drawn under this id; see cellCompartmentVisualId().
  es: { id: 'es', name: 'Eyespot',                  es: 'Mancha ocular',             membrane: 'bilayer', theme: T('#ffe4d6', '#431407', '#f97316', '#c2410c', '#9a3412', '#fdba74') },
};

/** Fallback hues for compartment ids that are not in the canonical table. */
const EXTRA_PALETTE = [
  T('#e2e8f0', '#1e293b', '#475569', '#334155', '#334155', '#cbd5e1'),
  T('#fff7ed', '#431407', '#f97316', '#c2410c', '#9a3412', '#fdba74'),
  T('#eff6ff', '#172554', '#3b82f6', '#1d4ed8', '#1e40af', '#93c5fd'),
  T('#f0fdf4', '#052e16', '#22c55e', '#15803d', '#166534', '#86efac'),
  T('#fdf2f8', '#500724', '#ec4899', '#be185d', '#9d174d', '#f9a8d4'),
  T('#fefce8', '#422006', '#eab308', '#a16207', '#854d0e', '#fde047'),
];

/**
 * Default nesting used when a model does not declare `compartment_tree`.
 * Key = compartment id, value = id of the compartment that physically contains it.
 * `null` means "directly inside the cell boundary".
 */
export const DEFAULT_PARENT = {
  e: null,
  w: 'e',
  p: 'w',
  c: 'p',
  n: 'c', x: 'c', r: 'c', g: 'c', v: 'c', l: 'c', cx: 'c',
  // A cell's own `s` is BiGG's eyespot. The shared medium never appears in a
  // species' tree, so there is no second meaning to keep apart here.
  s: 'c',
  // The space between the two mitochondrial membranes has two BiGG spellings:
  // `im`, and `i` in the algal and human reconstructions, where it holds the
  // protons the respiratory chain pumps. Either way the matrix sits inside it.
  // Chained, so a model that declares only one of them still nests correctly.
  i: 'c',
  im: 'i',
  m: 'im',
  h: 'c',
  u: 'h',
  f: null,
};

/**
 * Yeast9 uses a different compartment vocabulary from BiGG for several
 * organelles (notably `p` = peroxisome, whereas BiGG commonly uses `p` for
 * periplasm). Detecting the model semantically avoids imposing a bacterial
 * hierarchy on a eukaryotic model.
 *
 * Current Yeast9 / yeast-GEM 9.1.x exposes 14 compartment ids:
 * c, ce, e, er, erm, g, gm, lp, m, mm, n, p, v and vm.
 */
export const YEAST9_PARENT = {
  e: null,
  ce: 'e',
  c: 'ce',
  erm: 'c',
  er: 'erm',
  gm: 'c',
  g: 'gm',
  lp: 'c',
  mm: 'c',
  m: 'mm',
  n: 'c',
  p: 'c',
  vm: 'c',
  v: 'vm',
};

function normaliseCompartmentLabel(value) {
  return String(value ?? '')
    .trim()
    .toLowerCase()
    .replace(/[._-]+/g, ' ')
    .replace(/\s+/g, ' ');
}

/**
 * Resolve model-specific compartment aliases to Coxeter's visual vocabulary.
 * This is intentionally separate from the biological id: Yeast9's `p` is a
 * peroxisome, while bacterial BiGG models commonly use `p` for periplasm.
 */
export function compartmentVisualId(id, names = {}) {
  const label = normaliseCompartmentLabel(names?.[id] ?? '');
  if (label.includes('peroxisome')) return 'x';
  if (label.includes('cell envelope') || label.includes('cell wall')) return 'w';
  if (label.includes('endoplasmic reticulum')) return 'r';
  if (label.includes('golgi')) return 'g';
  if (label.includes('lipid particle') || label.includes('lipid droplet')) return 'lp';
  if (label.includes('mitochond')) return 'm';
  if (label.includes('vacuol')) return 'v';

  // Stable Yeast9 aliases still work when an imported model omits names.
  if (id === 'ce') return 'w';
  if (id === 'er' || id === 'erm') return 'r';
  if (id === 'gm') return 'g';
  if (id === 'mm') return 'm';
  if (id === 'vm') return 'v';
  // BiGG's other spelling of the mitochondrial intermembrane space.
  if (id === 'i') return 'im';
  return id;
}

/**
 * The same resolution for a compartment that belongs to a cell.
 *
 * `s` is the one id with two meanings. Between cells it is Coxeter's shared
 * medium. Inside a BiGG algal model (iRC1080 and everything drafted from it) it
 * is the eyespot. A cell never contains the shared medium, so wherever a
 * compartment is known to be a cell's own, `s` is the organelle and must not be
 * drawn dashed and grey like the medium it shares a letter with.
 */
export function cellCompartmentVisualId(id, names = {}) {
  const visual = compartmentVisualId(id, names);
  return visual === 's' && !namesSharedMedium(names) ? 'es' : visual;
}

/**
 * Does this set of names call `s` the shared medium?
 *
 * The merged community model is the one model that really contains it, and it
 * says so by name. Everything else that has an `s` has an organelle.
 */
function namesSharedMedium(names = {}) {
  const label = normaliseCompartmentLabel(names?.s ?? '');
  return label.includes('shared medium') || label.includes('medio compartido');
}

/** True when the compartment ids/names match the yeast-GEM / Yeast9 topology. */
export function isYeast9CompartmentSet(ids, names = {}) {
  const present = new Set(ids);
  if (!(present.has('c') && present.has('e') && present.has('ce'))) return false;

  const labels = new Set(
    [...present].map((id) => normaliseCompartmentLabel(names?.[id] ?? id)),
  );
  const semantic = [
    'cell envelope',
    'endoplasmic reticulum',
    'golgi',
    'lipid particle',
    'mitochondrion',
    'nucleus',
    'peroxisome',
    'vacuole',
  ];
  const hits = semantic.filter((needle) =>
    [...labels].some((label) => label === needle || label.includes(needle)),
  ).length;

  // IDs are stable in yeast-GEM, while names make the detection robust to
  // imported SBML files that preserve the official compartment labels.
  const idSignature = ['er', 'erm', 'g', 'gm', 'lp', 'm', 'mm', 'n', 'p', 'v', 'vm']
    .filter((id) => present.has(id)).length;
  return hits >= 5 || idSignature >= 8;
}

/** Named profile used by the layout engine for model-specific geometry. */
export function compartmentLayoutProfile(ids, names = {}) {
  return isYeast9CompartmentSet(ids, names) ? 'yeast9' : 'default';
}

let extraAssignments = new Map();

/** Deterministically resolve the visual definition of a compartment id. */
export function compartmentInfo(id) {
  if (COMPARTMENTS[id]) return COMPARTMENTS[id];
  if (!extraAssignments.has(id)) {
    // Hash the id so the same unknown compartment always lands on the same hue.
    let h = 0;
    for (let i = 0; i < id.length; i++) h = (h * 31 + id.charCodeAt(i)) >>> 0;
    extraAssignments.set(id, EXTRA_PALETTE[h % EXTRA_PALETTE.length]);
  }
  return {
    id,
    name: id,
    es: id,
    membrane: 'bilayer',
    theme: extraAssignments.get(id),
  };
}

/**
 * A name the model actually supplies for a compartment.
 *
 * An exporter with nothing to write repeats the id (cobrapy turns an SBML
 * compartment without a name into `"c": "c"`), and that is an absent name, not a
 * compartment called "c".
 */
function suppliedName(id, names) {
  const name = names?.[id];
  return typeof name === 'string' && name.trim() && name.trim() !== id ? name : null;
}

function registryName(visualId, locale) {
  const info = compartmentInfo(visualId);
  return locale === 'es' ? info.es : info.name;
}

/** Human readable compartment name in the requested locale. */
export function compartmentName(id, names = {}, locale = 'en') {
  return suppliedName(id, names) ?? registryName(compartmentVisualId(id, names), locale);
}

/** Human readable name of a compartment that belongs to a cell. */
export function cellCompartmentName(id, names = {}, locale = 'en') {
  return suppliedName(id, names) ?? registryName(cellCompartmentVisualId(id, names), locale);
}

/** Pick the theme colours for the active colour scheme. */
export function compartmentColors(id, dark = false, names = {}) {
  const { theme } = compartmentInfo(compartmentVisualId(id, names));
  return dark
    ? { fill: theme.fillDark, stroke: theme.strokeDark, text: theme.textDark }
    : { fill: theme.fill, stroke: theme.stroke, text: theme.text };
}

/** How the boundary of this compartment should be stroked. */
export function membraneStyle(id, names = {}) {
  return compartmentInfo(compartmentVisualId(id, names)).membrane;
}

/**
 * Build a containment tree for a set of compartment ids.
 *
 * @param {string[]} ids compartment ids present in the model
 * @param {Object<string,string|null>} [explicit] model supplied parent map
 * @param {Object<string,string>} [names] compartment names, used to detect Yeast9
 *   and the shared medium
 * @returns {{roots: TreeNode[], byId: Map<string, TreeNode>}}
 * @typedef {{id:string, children:TreeNode[], parent:string|null, depth:number}} TreeNode
 */
export function buildCompartmentTree(ids, explicit = null, names = {}) {
  const present = new Set(ids);
  const parentOf = new Map();
  const defaults = isYeast9CompartmentSet(ids, names) ? YEAST9_PARENT : DEFAULT_PARENT;

  // The shared medium, where a model has one, contains the cells rather than
  // sitting in one of them.
  const sharedMedium = namesSharedMedium(names);

  for (const id of present) {
    let parent = explicit && Object.prototype.hasOwnProperty.call(explicit, id)
      ? explicit[id]
      : (Object.prototype.hasOwnProperty.call(defaults, id) && !(id === 's' && sharedMedium)
        ? defaults[id]
        : null);

    // Walk up the default chain until we find a parent that actually exists in
    // this model. A model with c + m but no `im` should nest m directly in c.
    const seen = new Set();
    while (parent && !present.has(parent)) {
      if (seen.has(parent)) { parent = null; break; }
      seen.add(parent);
      parent = Object.prototype.hasOwnProperty.call(defaults, parent) ? defaults[parent] : null;
    }
    parentOf.set(id, parent && parent !== id ? parent : null);
  }

  // Break accidental cycles so the layout never recurses forever.
  for (const id of present) {
    const chain = new Set([id]);
    let p = parentOf.get(id);
    while (p) {
      if (chain.has(p)) { parentOf.set(id, null); break; }
      chain.add(p);
      p = parentOf.get(p);
    }
  }

  const byId = new Map();
  for (const id of present) byId.set(id, { id, children: [], parent: parentOf.get(id) ?? null, depth: 0 });

  const roots = [];
  for (const node of byId.values()) {
    if (node.parent && byId.has(node.parent)) byId.get(node.parent).children.push(node);
    else { node.parent = null; roots.push(node); }
  }

  const setDepth = (node, d) => { node.depth = d; node.children.forEach((ch) => setDepth(ch, d + 1)); };
  roots.forEach((r) => setDepth(r, 0));

  // Stable ordering: siblings by id at every level. Children follow their
  // parent, so wide shells are drawn first.
  const sortRec = (nodes) => {
    nodes.sort((a, b) => a.id.localeCompare(b.id));
    nodes.forEach((n) => sortRec(n.children));
  };
  sortRec(roots);

  return { roots, byId };
}

/** All compartment ids known to the registry, in display order. */
export function knownCompartmentIds() {
  return Object.keys(COMPARTMENTS);
}
