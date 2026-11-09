/**
 * COBRA SBML writer: Level 3 Version 1 with the FBC package, Version 2.
 *
 * The counterpart of `sbml.js`. That file reads what the field publishes, and
 * this one writes what the field reads. cobrapy and the COBRA Toolbox both load
 * L3V1 + fbc-v2, so a model built or edited here can leave for a real analysis
 * stack instead of staying in this app's JSON.
 *
 * What travels and what does not.
 * The metabolic model travels in full: compartments, species with their formula
 * and charge, stoichiometry, flux bounds, the objective, gene products and the
 * gene-protein-reaction rules, plus each reaction's subsystem in its notes.
 *
 * This project's own extensions do not: `cell`, `compartment_tree`, per-reaction
 * `layout`, `primary` and `pathway` describe how a model is *drawn*, have no
 * home in FBC, and are already carried by the map document. An SBML file written
 * here is the model, not the picture. Writing half of the drawing into a
 * private annotation would only look like preservation.
 *
 * SteadyCom is a further step out of reach: its coupling rows (`LB·X ≤ V ≤ UB·X`
 * and `Σ X = 1`) are not expressible in fbc-v2 at all, which stops at bounds, an
 * objective and GPR. Members export one file each, and the community LP does not
 * export. The place that would hold it is fbc **v3**'s UserDefinedConstraint.
 *
 * Identifiers.
 * SBML SIds are `(letter|_)(letter|digit|_)*`, and BiGG ids are not: `12ppd-R_e`
 * opens with a digit, `EX_glc__D(e)` and `12ethd[c_Bt]` carry punctuation. Two
 * escaping conventions are in use: the COBRA Toolbox writes `_DASH_` and cobrapy
 * writes `__45__` (the character code). This writer emits cobrapy's, because
 * cobrapy is what most readers will open the file with, and `sbml.js` decodes
 * both so anything written here reads back unchanged.
 */

/** A valid SBML SId. */
const SID = /^[A-Za-z_][A-Za-z0-9_]*$/;

/**
 * What fbc-v2 accepts as a chemical formula: letters and digits, opening with a
 * letter. Verified against libSBML, which accepts `Ferredoxin` and `Xy5` but
 * rejects anything carrying a `.` or a `_`.
 */
const FBC_FORMULA = /^[A-Za-z][A-Za-z0-9]*$/;

/**
 * Escape an identifier into an SId, cobrapy style: every character outside
 * `[A-Za-z0-9_]` becomes `__<code>__`. The type prefix (`R_`, `M_`, `G_`) also
 * rescues the ids that merely start with a digit.
 */
export function mangleId(id, prefix = '') {
  const escaped = String(id).replace(/[^A-Za-z0-9_]/g, (ch) => `__${ch.charCodeAt(0)}__`);
  const out = `${prefix}${escaped}`;
  if (!SID.test(out)) {
    throw new Error(`Cannot write "${id}" as an SBML identifier (got "${out}")`);
  }
  return out;
}

/** Compartment ids are usually already valid (`c`, `e`, `p`). Only the rest are escaped. */
const compartmentSId = (id) => (SID.test(id) ? id : mangleId(id, 'C_'));

const esc = (s) => String(s ?? '')
  .replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
  .replace(/"/g, '&quot;');

/** Numbers as SBML doubles: no thousands separators, no stray precision. */
const numeric = (value) => {
  const n = Number(value);
  if (!Number.isFinite(n)) return n > 0 ? 'INF' : '-INF';
  return String(n);
};

const attrs = (pairs) => Object.entries(pairs)
  .filter(([, v]) => v !== null && v !== undefined && v !== '')
  .map(([k, v]) => ` ${k}="${esc(v)}"`).join('');

// GPR

/**
 * Parse a gene-protein-reaction rule into a tree.
 *
 * Grammar, with `or` binding loosest, which is how COBRA writes and reads them:
 *   expr   := term ( 'or' term )*
 *   term   := factor ( 'and' factor )*
 *   factor := '(' expr ')' | gene
 *
 * @param {string} rule e.g. `(b0001 and b0002) or b0003`
 * @returns {{gene:string}|{op:'and'|'or', args:Array}|null} null for no rule
 */
export function parseGPR(rule) {
  const text = String(rule ?? '').trim();
  if (!text) return null;
  // `&&` and `|` turn up in hand-written rules. COBRA's own files use the words.
  const tokens = (text.match(/\(|\)|[^\s()]+/g) ?? [])
    .map((t) => (t === '&&' || t === '&' ? 'and' : (t === '||' || t === '|' ? 'or' : t)));
  let at = 0;

  const peek = () => tokens[at];
  const keyword = (word) => {
    const t = peek();
    return typeof t === 'string' && t.toLowerCase() === word;
  };

  const expr = () => {
    const args = [term()];
    while (keyword('or')) { at += 1; args.push(term()); }
    return args.length === 1 ? args[0] : { op: 'or', args };
  };
  const term = () => {
    const args = [factor()];
    while (keyword('and')) { at += 1; args.push(factor()); }
    return args.length === 1 ? args[0] : { op: 'and', args };
  };
  const factor = () => {
    const t = peek();
    if (t === '(') {
      at += 1;
      const inner = expr();
      if (peek() === ')') at += 1;
      return inner;
    }
    at += 1;
    return { gene: t };
  };

  const tree = expr();
  // A rule that parsed to nothing usable (stray punctuation) is no rule at all.
  return tree && (tree.gene || tree.args) ? tree : null;
}

/** Every gene id a rule mentions, in order of first appearance. */
export function genesInGPR(rule) {
  const out = [];
  const walk = (node) => {
    if (!node) return;
    if (node.gene) { if (!out.includes(node.gene)) out.push(node.gene); return; }
    node.args.forEach(walk);
  };
  walk(parseGPR(rule));
  return out;
}

const gprXML = (node, indent) => {
  if (node.gene) {
    return `${indent}<fbc:geneProductRef fbc:geneProduct="${esc(mangleId(node.gene, 'G_'))}"/>`;
  }
  const inner = node.args.map((a) => gprXML(a, `${indent}  `)).join('\n');
  return `${indent}<fbc:${node.op}>\n${inner}\n${indent}</fbc:${node.op}>`;
};

// Bounds

/**
 * Flux bounds are parameters in fbc-v2, not attributes, and `fbc:strict` wants
 * every one of them constant and defined. Pooling the values keeps the file the
 * size of the model rather than the size of twice its reaction count: a
 * genome-scale model uses a handful of distinct bounds for thousands of
 * reactions. The three shared names are cobrapy's, so its files and ours read
 * alike.
 */
function boundParameters(reactions) {
  const pool = new Map(); // value -> parameter id
  const order = [];
  const shared = new Map([[-1000, 'cobra_default_lb'], [1000, 'cobra_default_ub'], [0, 'cobra_0_bound']]);

  const nameFor = (value, reactionId, side) => {
    if (pool.has(value)) return pool.get(value);
    const id = shared.has(value)
      ? shared.get(value)
      : mangleId(`${reactionId}_${side}_bound`, 'R_');
    pool.set(value, id);
    order.push({ id, value });
    return id;
  };

  const perReaction = new Map();
  for (const rxn of reactions) {
    perReaction.set(rxn.id, {
      lower: nameFor(rxn.lowerBound, rxn.id, 'lower'),
      upper: nameFor(rxn.upperBound, rxn.id, 'upper'),
    });
  }
  return { parameters: order, perReaction };
}

// Writing

const NS_CORE = 'http://www.sbml.org/sbml/level3/version1/core';
const NS_FBC = 'http://www.sbml.org/sbml/level3/version1/fbc/version2';
const NS_HTML = 'http://www.w3.org/1999/xhtml';

/**
 * Serialize a model as SBML L3V1 + fbc-v2.
 *
 * @param {import('./model.js').Model} model
 * @param {object} [opts]
 * @param {string} [opts.objectiveId] name of the objective element
 * @param {'maximize'|'minimize'} [opts.objectiveType]
 * @returns {string} the XML document
 */
export function toSBML(model, opts = {}) {
  const objectiveId = opts.objectiveId ?? 'obj';
  const objectiveType = opts.objectiveType ?? 'maximize';

  const reactions = [...model.reactions.values()];
  const { parameters, perReaction } = boundParameters(reactions);

  const compartments = model.compartmentIds().map((id) => `    <compartment${attrs({
    id: compartmentSId(id),
    name: model.compartmentNames[id] && model.compartmentNames[id] !== id
      ? model.compartmentNames[id] : null,
    constant: 'true',
  })}/>`).join('\n');

  const species = [...model.metabolites.values()].map((met) => {
    /*
     * fbc:chemicalFormula takes letters and digits only. Published models carry
     * formulas that do not qualify: a biomass pseudo-metabolite with fractional
     * counts (`C37.024041172583H67.8…`) and macromolecule placeholders with an
     * underscore (`Apo_acp`, `C11H20O6N2PSApo_acp`). libSBML rejects those
     * outright, so writing them would hand the user an invalid file.
     *
     * They go into the species' notes instead, under COBRA's own `FORMULA:`
     * key, which is where Level 2 files keep every formula and where sbml.js
     * looks. Nothing is lost, and the file stays conformant.
     */
    const conformant = FBC_FORMULA.test(met.formula ?? '');
    const head = `    <species${attrs({
      id: mangleId(met.id, 'M_'),
      name: met.name && met.name !== met.id ? met.name : null,
      compartment: compartmentSId(met.compartment),
      hasOnlySubstanceUnits: 'false',
      boundaryCondition: 'false',
      constant: 'false',
      'fbc:charge': Number.isFinite(met.charge) ? String(met.charge) : null,
      'fbc:chemicalFormula': conformant ? met.formula : null,
    })}`;
    if (conformant || !met.formula) return `${head}/>`;
    return `${head}>\n      <notes>\n        <html:p xmlns:html="${NS_HTML}">FORMULA: `
      + `${esc(met.formula)}</html:p>\n      </notes>\n    </species>`;
  }).join('\n');

  // SBO:0000626 is "flux bound". Published FBC files carry it on these, and it
  // is what tells a reader these parameters are bounds rather than kinetics.
  const parameterXML = parameters.map(({ id, value }) => `    <parameter${attrs({
    id, value: numeric(value), constant: 'true', sboTerm: 'SBO:0000626',
    units: 'mmol_per_gDW_per_hr',
  })}/>`).join('\n');

  // Gene products: everything the rules mention, plus anything the model lists
  // on its own. An id appearing twice is one product.
  const geneNames = new Map(model.genes.map((g) => [g.id, g.name]));
  const geneIds = [];
  for (const g of model.genes) if (!geneIds.includes(g.id)) geneIds.push(g.id);
  for (const rxn of reactions) {
    for (const gene of genesInGPR(rxn.geneReactionRule)) {
      if (!geneIds.includes(gene)) geneIds.push(gene);
    }
  }

  const reactionXML = reactions.map((rxn) => {
    const side = (entries, tag) => {
      if (!entries.length) return '';
      const refs = entries.map(([mid, coef]) => `        <speciesReference${attrs({
        species: mangleId(mid, 'M_'),
        stoichiometry: numeric(Math.abs(coef)),
        constant: 'true',
      })}/>`).join('\n');
      return `\n      <listOf${tag}>\n${refs}\n      </listOf${tag}>`;
    };

    const bounds = perReaction.get(rxn.id);
    const gpr = parseGPR(rxn.geneReactionRule);
    const gprBlock = gpr
      ? `\n      <fbc:geneProductAssociation>\n${gprXML(gpr, '        ')}\n      </fbc:geneProductAssociation>`
      : '';
    // The subsystem has no fbc home. COBRA's own Level 2 convention is a note,
    // and `sbml.js` reads it back from there.
    const notes = rxn.subsystem
      ? `\n      <notes>\n        <html:p xmlns:html="${NS_HTML}">SUBSYSTEM: ${esc(rxn.subsystem)}</html:p>\n      </notes>`
      : '';

    const body = notes
      + side(rxn.substrates(), 'Reactants')
      + side(rxn.products(), 'Products')
      + gprBlock;

    const head = `    <reaction${attrs({
      id: mangleId(rxn.id, 'R_'),
      name: rxn.name && rxn.name !== rxn.id ? rxn.name : null,
      reversible: rxn.lowerBound < 0 ? 'true' : 'false',
      fast: 'false',
      'fbc:lowerFluxBound': bounds.lower,
      'fbc:upperFluxBound': bounds.upper,
    })}>`;
    return `${head}${body}\n    </reaction>`;
  }).join('\n');

  const fluxObjectives = model.objectiveReactions()
    .map(({ id, coefficient }) => `        <fbc:fluxObjective${attrs({
      'fbc:reaction': mangleId(id, 'R_'),
      'fbc:coefficient': numeric(coefficient),
    })}/>`).join('\n');

  const geneProducts = geneIds.map((id) => `    <fbc:geneProduct${attrs({
    'fbc:id': mangleId(id, 'G_'),
    'fbc:label': id,
    'fbc:name': geneNames.get(id) && geneNames.get(id) !== id ? geneNames.get(id) : null,
  })}/>`).join('\n');

  const block = (open, inner, close) => (inner ? `\n  ${open}\n${inner}\n  ${close}` : '');

  return `<?xml version="1.0" encoding="UTF-8"?>
<sbml xmlns="${NS_CORE}" xmlns:fbc="${NS_FBC}" level="3" version="1" fbc:required="false">
  <model${attrs({
    // Published files write the model's id plain (`iAF1260`), so only the ones
    // SBML would reject get the prefix.
    id: SID.test(model.id) ? model.id : mangleId(model.id, 'M_'),
    name: model.name && model.name !== model.id ? model.name : null,
    'fbc:strict': 'true',
  })}>
    <listOfUnitDefinitions>
      <unitDefinition id="mmol_per_gDW_per_hr">
        <listOfUnits>
          <unit kind="mole" exponent="1" scale="-3" multiplier="1"/>
          <unit kind="gram" exponent="-1" scale="0" multiplier="1"/>
          <unit kind="second" exponent="-1" scale="0" multiplier="3600"/>
        </listOfUnits>
      </unitDefinition>
    </listOfUnitDefinitions>${block('<listOfCompartments>', compartments, '</listOfCompartments>')}${
  block('<listOfSpecies>', species, '</listOfSpecies>')}${
  block('<listOfParameters>', parameterXML, '</listOfParameters>')}${
  block('<listOfReactions>', reactionXML, '</listOfReactions>')}
    <fbc:listOfObjectives fbc:activeObjective="${esc(objectiveId)}">
      <fbc:objective fbc:id="${esc(objectiveId)}" fbc:type="${esc(objectiveType)}">
${fluxObjectives ? `        <fbc:listOfFluxObjectives>\n${fluxObjectives}\n        </fbc:listOfFluxObjectives>\n` : ''}      </fbc:objective>
    </fbc:listOfObjectives>${block('<fbc:listOfGeneProducts>', geneProducts, '</fbc:listOfGeneProducts>')}
  </model>
</sbml>
`;
}
