/**
 * COBRA SBML reader.
 *
 * Published genome-scale models are distributed as SBML, so this is what lets a
 * Published Community be built from the authors' own file rather than from a
 * network retyped out of the paper. It reads the two dialects those files
 * actually come in:
 *
 *   SBML Level 2 + COBRA conventions
 *     bounds and objective live in each reaction's <kineticLaw> as parameters
 *     named LOWER_BOUND / UPPER_BOUND / OBJECTIVE_COEFFICIENT
 *   SBML Level 3 + the FBC package
 *     bounds are named parameters referenced by fbc:lowerFluxBound, and the
 *     objective is an fbc:listOfObjectives
 *
 * Identifiers are un-mangled back to the ids the papers use: SBML forbids a
 * leading digit and most punctuation, so exporters write `R_EX_glc__D_LPAREN_e_RPAREN_`
 * for `EX_glc__D_e`. Nothing else about the model is altered (no reactions are
 * added, removed or rescaled), so what comes out can be checked against the
 * source file reaction by reaction.
 */

const RE_SPECIES = /<species\b([^>]*)\/?>/g;
const RE_ATTR = (name) => new RegExp(`\\b${name}="([^"]*)"`);
const RE_PARAM = /<parameter\b([^>]*)\/?>/g;

/**
 * Undo the character escaping SBML exporters apply to BiGG identifiers.
 *
 * Two conventions are in the wild: the COBRA Toolbox writes `_DASH_`, cobrapy
 * writes `__45__` (the character code). Both are decoded, because files of both
 * kinds are published and this project writes the second.
 *
 * @param {object} [opts]
 * @param {boolean} [opts.normaliseCompartment] rewrite a trailing `(e)` or `[e]`
 *        to `_e`, so the three spellings of one metabolite become one id. On by
 *        default because it is what makes a BiGG file line up with this app's
 *        ids, but it is **not invertible**: `12ethd[c_Bt]` comes back as
 *        `12ethd_c_Bt`. Turn it off to read back exactly what was written.
 */
export function unmangleId(id, prefix, opts = {}) {
  const normaliseCompartment = opts.normaliseCompartment ?? true;
  let out = id;
  if (prefix && out.startsWith(prefix)) out = out.slice(prefix.length);
  out = out
    .replace(/__(\d+)__/g, (_, code) => String.fromCharCode(Number(code)))
    .replace(/_LPAREN_/g, '(')
    .replace(/_RPAREN_/g, ')')
    .replace(/_LSQBKT_/g, '[')
    .replace(/_RSQBKT_/g, ']')
    .replace(/_DASH_/g, '-')
    .replace(/_FSLASH_/g, '/')
    .replace(/_BSLASH_/g, '\\')
    .replace(/_COMMA_/g, ',')
    .replace(/_PERIOD_/g, '.')
    .replace(/_APOS_/g, "'");
  // `EX_glc__D(e)` and `glc__D[e]` are both spelled `..._e` in this project.
  if (normaliseCompartment) {
    out = out.replace(/\((\w+)\)$/, '_$1').replace(/\[(\w+)\]$/, '_$1');
  }
  return out;
}

const attr = (fragment, name) => {
  const m = RE_ATTR(name).exec(fragment);
  return m ? m[1] : null;
};

const num = (value, fallback) => {
  const n = Number(value);
  return Number.isFinite(n) ? n : fallback;
};

/** Decode the handful of XML entities that appear in model names. */
const decode = (s) => (s ?? '')
  .replace(/&lt;/g, '<').replace(/&gt;/g, '>')
  .replace(/&quot;/g, '"').replace(/&apos;/g, "'")
  .replace(/&amp;/g, '&');

/**
 * Parse a COBRA SBML file, Level 2 with COBRA notes or Level 3 with FBC.
 *
 * @param {string} xml
 * @param {object} [opts]
 * @param {string} [opts.id] model id to use when the file does not carry one
 * @param {boolean} [opts.normaliseCompartment] passed to unmangleId (default true)
 * @param {number} [opts.defaultBound] magnitude used when a bound is missing
 * @param {number} [opts.sentinelBound] bounds at least this large in magnitude are
 *        the file's way of writing "unbounded" and are rewritten to
 *        `defaultBound`. Off unless asked for: it changes the model, so the caller
 *        has to say so and justify it. The Feist 2007 iAF1260 supplement writes
 *        999999 where COBRA writes 1000, and a coefficient of 1e6 in a coupling
 *        row is enough on its own to make a community LP unsolvable.
 * @returns {object} a COBRA JSON document, ready for `new Model(...)`
 */
export function parseCOBRASBML(xml, opts = {}) {
  const defaultBound = opts.defaultBound ?? 1000;
  const sentinelBound = opts.sentinelBound ?? Infinity;
  const idOpts = { normaliseCompartment: opts.normaliseCompartment ?? true };
  const unmangle = (id, prefix) => unmangleId(id, prefix, idOpts);

  const modelTag = /<model\b([^>]*)>/.exec(xml)?.[1] ?? '';
  const modelId = opts.id ?? unmangle(attr(modelTag, 'id') ?? 'model', 'M_');
  const modelName = decode(attr(modelTag, 'name')) || modelId;

  // Compartments
  const compartments = {};
  for (const m of xml.matchAll(/<compartment\b([^>]*)\/?>/g)) {
    const id = attr(m[1], 'id');
    if (!id) continue;
    compartments[unmangle(id, 'C_')] = decode(attr(m[1], 'name')) || id;
  }

  // Global parameters, for the FBC dialect
  const globalParams = new Map();
  const paramSection = /<listOfParameters>([\s\S]*?)<\/listOfParameters>/.exec(xml);
  if (paramSection) {
    for (const p of paramSection[1].matchAll(RE_PARAM)) {
      const id = attr(p[1], 'id');
      if (id) globalParams.set(id, num(attr(p[1], 'value'), null));
    }
  }

  // Gene products, for the FBC dialect.
  // Level 2 files keep the rule in a note. Level 3 + FBC keeps it as a nested
  // and/or tree over gene products declared once at the end of the model, so
  // reading only the note would drop every GPR in a modern BiGG file.
  const geneLabels = new Map();
  const genes = [];
  for (const g of xml.matchAll(/<fbc:geneProduct\b([^>]*)\/?>/g)) {
    const gid = attr(g[1], 'fbc:id');
    if (!gid) continue;
    const label = decode(attr(g[1], 'fbc:label')) || unmangle(gid, 'G_');
    geneLabels.set(gid, label);
    genes.push({ id: label, name: decode(attr(g[1], 'fbc:name')) || label });
  }

  // Metabolites
  const metabolites = [];
  const metIds = new Set();
  RE_SPECIES.lastIndex = 0;
  for (const m of xml.matchAll(RE_SPECIES)) {
    const rawId = attr(m[1], 'id');
    if (!rawId) continue;
    // Boundary species are the "outside" ends of exchange reactions. COBRA
    // models carry the balance on the real metabolite, so they are dropped.
    if (attr(m[1], 'boundaryCondition') === 'true') continue;
    const id = unmangle(rawId, 'M_');
    metIds.add(rawId);
    const compartment = unmangle(attr(m[1], 'compartment') ?? '', 'C_')
      || /_([a-z][a-z0-9]?)$/.exec(id)?.[1]
      || 'c';
    metabolites.push({
      id,
      name: decode(attr(m[1], 'name')) || id,
      compartment,
      formula: attr(m[1], 'fbc:chemicalFormula')
        ?? formulaFromNotes(xml, rawId)
        ?? '',
      charge: num(attr(m[1], 'fbc:charge') ?? attr(m[1], 'charge'), 0),
    });
  }

  // Reactions
  const reactions = [];
  const objective = {};
  for (const block of xml.split(/<reaction\b/).slice(1)) {
    const header = block.slice(0, block.indexOf('>'));
    const rawId = attr(header, 'id');
    if (!rawId) continue;
    const body = block.slice(0, block.indexOf('</reaction>') + 1);
    const id = unmangle(rawId, 'R_');

    const stoich = {};
    const collect = (tag, sign) => {
      const section = new RegExp(`<listOf${tag}>([\\s\\S]*?)</listOf${tag}>`).exec(body);
      if (!section) return;
      for (const ref of section[1].matchAll(/<speciesReference\b([^>]*)\/?>/g)) {
        const species = attr(ref[1], 'species');
        if (!species || !metIds.has(species)) continue;
        const mid = unmangle(species, 'M_');
        const coef = sign * num(attr(ref[1], 'stoichiometry'), 1);
        stoich[mid] = (stoich[mid] ?? 0) + coef;
      }
    };
    collect('Reactants', -1);
    collect('Products', 1);
    for (const key of Object.keys(stoich)) if (stoich[key] === 0) delete stoich[key];

    // Bounds: FBC first, then the Level 2 kineticLaw parameters.
    let lower = null;
    let upper = null;
    const fbcLower = attr(header, 'fbc:lowerFluxBound');
    const fbcUpper = attr(header, 'fbc:upperFluxBound');
    if (fbcLower && globalParams.has(fbcLower)) lower = globalParams.get(fbcLower);
    if (fbcUpper && globalParams.has(fbcUpper)) upper = globalParams.get(fbcUpper);
    if (lower === null || upper === null) {
      for (const p of body.matchAll(RE_PARAM)) {
        const pid = attr(p[1], 'id');
        const value = num(attr(p[1], 'value'), null);
        if (value === null) continue;
        if (pid === 'LOWER_BOUND' && lower === null) lower = value;
        else if (pid === 'UPPER_BOUND' && upper === null) upper = value;
        else if (pid === 'OBJECTIVE_COEFFICIENT' && value !== 0) objective[id] = value;
      }
    }
    if (lower === null) {
      lower = attr(header, 'reversible') === 'false' ? 0 : -defaultBound;
    }
    if (upper === null) upper = defaultBound;
    if (lower <= -sentinelBound) lower = -defaultBound;
    if (upper >= sentinelBound) upper = defaultBound;

    reactions.push({
      id,
      name: decode(attr(header, 'name')) || id,
      metabolites: stoich,
      lower_bound: lower,
      upper_bound: upper,
      gene_reaction_rule: gprFromFBC(body, geneLabels) ?? geneRuleFromNotes(body) ?? '',
      subsystem: noteValue(body, 'SUBSYSTEM') ?? '',
    });
  }

  // FBC objective
  const fbcObjective = /<fbc:fluxObjective\b([^>]*)\/?>/g;
  for (const m of xml.matchAll(fbcObjective)) {
    const rid = attr(m[1], 'fbc:reaction');
    const coef = num(attr(m[1], 'fbc:coefficient'), 1);
    if (rid && coef) objective[unmangle(rid, 'R_')] = coef;
  }

  for (const met of metabolites) {
    if (!compartments[met.compartment]) compartments[met.compartment] = met.compartment;
  }

  return {
    id: modelId,
    name: modelName,
    compartments,
    metabolites,
    reactions,
    genes,
    objective: Object.keys(objective).length ? objective : null,
  };
}

/** Pull `KEY: value` out of a COBRA <notes> block. */
function noteValue(fragment, key) {
  const m = new RegExp(`${key}:\\s*([^<]*)`).exec(fragment);
  const value = m?.[1]?.trim();
  return value ? decode(value) : null;
}

/**
 * Read `<fbc:geneProductAssociation>` back into a COBRA rule string.
 *
 * The nesting is arbitrary, so this walks the tags with a stack rather than
 * matching a fixed shape. Parentheses go back in only where a child's operator
 * differs from its parent's, which is what makes the string re-parse to the
 * same tree.
 */
function gprFromFBC(body, labels) {
  const section = /<fbc:geneProductAssociation\b[^>]*>([\s\S]*?)<\/fbc:geneProductAssociation>/
    .exec(body);
  if (!section) return null;

  const root = { op: null, args: [] };
  const stack = [root];
  const tags = /<(\/?)fbc:(and|or|geneProductRef)\b([^>]*?)(\/?)>/g;
  for (const m of section[1].matchAll(tags)) {
    const [, closing, kind, attrText, selfClosing] = m;
    const top = stack[stack.length - 1];
    if (kind === 'geneProductRef') {
      const ref = attr(attrText, 'fbc:geneProduct');
      if (ref) top.args.push({ gene: labels.get(ref) ?? unmangleId(ref, 'G_') });
    } else if (closing) {
      if (stack.length > 1) {
        const node = stack.pop();
        stack[stack.length - 1].args.push(node);
      }
    } else if (!selfClosing) {
      stack.push({ op: kind, args: [] });
    }
  }

  const render = (node) => {
    if (node.gene) return node.gene;
    if (!node.op) return node.args.length === 1 ? render(node.args[0]) : node.args.map(render).join(' and ');
    return node.args
      .map((a) => (a.op && a.op !== node.op ? `(${render(a)})` : render(a)))
      .join(` ${node.op} `);
  };
  const rule = render(root).trim();
  return rule || null;
}

function geneRuleFromNotes(body) {
  return noteValue(body, 'GENE_ASSOCIATION') || noteValue(body, 'GENE ASSOCIATION');
}

/**
 * Level 2 files keep the elemental formula in the species' notes rather than in
 * an attribute. Scanning the whole document per species would be quadratic, so
 * the notes are indexed once on first use.
 */
let formulaIndex = null;
let formulaIndexSource = null;
function formulaFromNotes(xml, rawId) {
  if (formulaIndexSource !== xml) {
    formulaIndex = new Map();
    for (const block of xml.split(/<species\b/).slice(1)) {
      const header = block.slice(0, block.indexOf('>'));
      const id = attr(header, 'id');
      if (!id) continue;
      const notes = block.slice(0, Math.max(block.indexOf('</species>'), 0) + 1);
      const f = /FORMULA:\s*([^<\s]*)/.exec(notes);
      if (f?.[1]) formulaIndex.set(id, f[1]);
    }
    formulaIndexSource = xml;
  }
  return formulaIndex.get(rawId) ?? null;
}

/** Convenience: read a model straight into the project's `Model` class. */
export async function loadSBMLModel(xml, ModelClass, opts = {}) {
  return new ModelClass(parseCOBRASBML(xml, opts));
}
