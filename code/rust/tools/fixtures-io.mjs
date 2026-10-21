// Reference outputs of the model input/output modules for the Rust port's
// parity tests (crate coxeter-io).
//
//   node rust/tools/fixtures-io.mjs
//
// Writes rust/fixtures/io/ (small, committed): every bundled model written as
// SBML and read back by both readers, uploads, edits, provenance links, export
// names, workbooks and MAT files built here, and reproducibility bundles.
//
// Writes rust/fixtures/large/io/ (git-ignored) from files outside the repo when
// they exist: the published community members, the COBRA Toolbox test models
// (SBML and MAT), the user's own genome-scale models and workbooks. Each large
// case records its input path. The Rust tests skip what is absent.
globalThis.crypto ??= (await import('node:crypto')).webcrypto;

import { readFile, writeFile, mkdir, readdir, stat } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { dirname, join, basename } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { homedir } from 'node:os';
import { deflateRawSync, deflateSync } from 'node:zlib';

const root = join(dirname(fileURLToPath(import.meta.url)), '../..');
const out = join(root, 'rust/fixtures/io');
const large = join(root, 'rust/fixtures/large/io');
for (const d of [out, join(out, 'sbml'), join(out, 'xlsx'), join(out, 'mat'), large, join(large, 'sbml')]) {
  await mkdir(d, { recursive: true });
}

const src = (p) => import(pathToFileURL(join(root, 'src', p)).href);
const { Model } = await src('core/model.js');
const { toSBML, mangleId, parseGPR, genesInGPR } = await src('core/sbml-write.js');
const { parseCOBRASBML, unmangleId } = await src('core/sbml.js');
const { parseModelJSON, parseModelSBML, parseModelFile, prepareImportedModels, inferCommunityMedium } = await src('core/model-import.js');
const { parseStoichiometry, geneIdsFromGPR, addMetaboliteToModel, addReactionToModel } = await src('core/model-edit.js');
const { normalizeAnnotations, annotationLinks, extractIdentifiersOrg, annotationObject } = await src('core/provenance.js');
const repro = await src('core/reproducibility.js');
const { readWorkbook } = await src('core/xlsx.js');
const { readMat5 } = await src('import/mat5.js');
const names = await src('ui/file-names.js');

const write = (dir, name, value) => writeFile(join(dir, name), JSON.stringify(value));
const attempt = (fn) => {
  try { return { ok: fn() }; } catch (error) { return { error: error.message }; }
};
const attemptAsync = async (fn) => {
  try { return { ok: await fn() }; } catch (error) { return { error: error.message }; }
};
const modelJSON = (m) => JSON.parse(JSON.stringify(m.toJSON()));

// File names

const stems = [
  'yeastGEM_v9.1.1', 'Zuñiga (2020): S. elongatus + E. coli', '  ../  ', 'x'.repeat(200), 'iML1515+yeastGEM',
  'Ångström ﬁle №5', '日本語のモデル', 'a__b', '..a..', '++x--', 'emoji 😀 model', '', 'ﬀ', `${'a'.repeat(79)}.b`,
  `${'b'.repeat(78)}_-_c`, 'Crème brûlée', 'tab\tand\nnewline', 'ẞ ß ǅ', 'Ω≈ç√∫', 'São Paulo/Río', 'x+y+z.json',
];
const fileNames = {
  fileStem: stems.map((s) => ({ input: s, out: names.fileStem(s), fallback: names.fileStem(s, 'fb') })),
  exportFileName: [
    ['yeastGEM_v9.1.1', names.EXPORT_SUFFIX.map, 'json'], ['yeastGEM_v9.1.1', 'model', 'xml'], ['', '', 'csv'],
    ['Zuñiga (2020)', 'méthodes', 'md'], ['iML1515+yeastGEM', 'map', 'json'], ['  ', '--', 'svg'],
  ].map((args) => ({ args, out: names.exportFileName(...args) })),
  memberModelFileName: [
    ['steadycom-gut-nine', 'Ec', 'json'], ['zuniga-2020-phea-rescue', 'Ec', 'json'], ['stolyar-2007-syntrophy', 'Dv', 'xml'],
    ['', '', 'json'], ['Gut nine!', '¿?', 'xml'],
  ].map((args) => ({ args, out: names.memberModelFileName(...args) })),
  communityDataFileName: [
    ['steadycom-gut-nine', 'media.csv'], ['x', 'bounds'], ['x', 'x.tar.gz'], ['x', '.csv'], ['x', 'file.'],
    ['x', 'a\nb.csv'], ['x', 'données.CSV'], ['x', ''], ['Gut nine', 'bounds.tsv'],
  ].map((args) => ({ args, out: names.communityDataFileName(...args) })),
  timestampedFileName: [
    ['consorcio-anaerobio', 'repro', 'json', [2026, 9, 10, 15, 7]],
    ['yeastGEM_v9.1.1', 'methods', 'md', [2026, 9, 10, 15, 7]],
    ['a', 'b', 'c', [999, 0, 1, 0, 0]], ['Zuñiga', 'repro', 'json', [2031, 11, 31, 23, 59]],
  ].map(([n, s, e, parts]) => ({
    args: [n, s, e], date: { year: parts[0], month: parts[1] + 1, day: parts[2], hour: parts[3], minute: parts[4] },
    out: names.timestampedFileName(n, s, e, new Date(...parts)),
  })),
  suffixes: names.EXPORT_SUFFIX,
};
await write(out, 'file-names.json', fileNames);

// Provenance

const annotationCases = [
  [{ chebi: 'CHEBI:15377' }, {}, 'metabolite'],
  [{ rhea: 'RHEA:10000' }, {}, 'reaction'],
  [{ uniprot: 'P12345' }, {}, 'gene'],
  [{ 'kegg.compound': 'C00001; C00002,C00003', x: ['a', ['b', null]], empty: '' }, { 'KEGG.compound': 'nope', pmid: 12 }, 'metabolite'],
  [{ 'bigg.metabolite': ['glc__D'], 'metanetx.chemical': ['MNXM41'], 'inchi_key': ['WQZGKKKJIJFFOK-GASJEMHNSA-N'] }, {}, 'metabolite'],
  [{ bigg: 'bigg:PGI', 'bigg.reaction': 'BIGG.REACTION:PGI', 'ec-code': ['5.3.1.9'], EC_number: '1.1.1.1', 'KEGG_Genes': 'eco:b4025' }, {}, 'reaction'],
  [{ doi: 'doi:10.1038/msb.2011.65', pubmed: '21988831', biocyc: 'META:PGLUCISOM', reactome: 'R-HSA-70171' }, {}, 'reaction'],
  [{ 'http://identifiers.org/kegg.reaction': 'R00771', unknown: ['CHEBI:17234', 'MNXR1', 'RHEA:1', '1.2.3.4', 'C00031', 'R00001', 'G00001', 'plain'] }, {}, 'metabolite'],
  [{ unknown: ['C00031', 'R00001'] }, {}, 'gene'],
  [{ ncbigene: '945642', refseq: 'NP_418449.1', hmdb: 'HMDB0000122', pubchem: 'CID:5793', 'kegg.reaction': 'rn:R00771' }, {}, 'gene'],
  [{}, { text: 'Confidence Level: 3', 'Pathway': 'glycolysis', n: 5, flag: true, list: ['a; b', 'c'] }, 'reaction'],
  [null, null, 'metabolite'],
  [{ sbo: 'SBO:0000176', 'chebi/chebi': ['57470'], s: 'a b/c?d&e=ü' }, { sbo: 'other' }, 'reaction'],
  ['text annotation', 'xy', 'metabolite'],
  [[['nested', 'array']], {}, 'metabolite'],
];
const identifierBodies = [
  '<rdf:li rdf:resource="https://identifiers.org/kegg.reaction/R00010"/><rdf:li rdf:resource="https://identifiers.org/chebi:15377"/>',
  '<rdf:li rdf:resource="http://identifiers.org/CHEBI:15377"/><rdf:li rdf:resource="https://registry.identifiers.org/MetaNetX.Chemical/MNXM1"/>',
  'HTTPS://IDENTIFIERS.ORG/ec-code/1.1.1.1 https://identifiers.org/ec-code/1.1.1.1 https://identifiers.org/pubmed/123/',
  "<x a='https://identifiers.org/taxonomy/559292'/> https://identifiers.org/kegg.compound/C00031#frag",
  'https://identifiers.org/chebi%3A15377 https://identifiers.org/a%2Fb/c https://identifiers.org/onlyns https://identifiers.org//lead/trail//',
  'https://identifiers.org/123/abc https://identifiers.org/10:x https://identifiers.org/b:y',
  'https://identifiers.org/bad%ZZ',
  'https://identifiers.org/sbo/SBO:0000176 http://identifiers.org/go/GO:0005739 https://identifiers.org/inchikey/XLYOFNOQVPJJNP-UHFFFAOYSA-N',
];
await write(out, 'provenance.json', {
  normalizeAnnotations: annotationCases.map(([a, n]) => ({ annotation: a, notes: n, out: [...normalizeAnnotations(a ?? undefined, n ?? undefined)] })),
  annotationLinks: annotationCases.map(([a, n, t]) => ({ annotation: a, notes: n, type: t, out: annotationLinks(a ?? undefined, n ?? undefined, t) })),
  extractIdentifiersOrg: identifierBodies.map((body) => ({ body, ...attempt(() => extractIdentifiersOrg(body)) })),
  annotationObject: [null, 'x', 5, { a: 1 }, [1]].map((raw) => ({ raw, out: annotationObject(raw) })),
});

// Model edit

const stoichiometryCases = [
  'a_c:-2, b_c:1', 'a_c -1\nb_c 1', '{"a_c":-1,"b_c":1}', 'a_c:1;a_c:-1', 'a_c:1, a_c:-0.5', '  ', '', 'a_c', 'a_c:x',
  '{"a_c":"2","b_c":true,"c_c":[3],"d_c":null}', '{"a_c":{}}', '{"10":1,"2":-1,"z":1}', '{}', '[1]', '{bad json',
  'a_c : +1.5e2 ,\n b_c:.5', 'name with spaces : 1', 'x : 1', 'a_c:1e400', 'a_c:0', ':1', '1e-16_c:1',
  'a_c:1\r\nb_c:2', 'a_c:1, b_c:-1e-16',
];
const gprCases = [
  'g1 and (g2 or g3)', '', '  ', 'a AND b Or c', 'not a', '(a or b) and (c or d)', 'TRUE false x', 'a&&b||c',
  'g1 and g1 or g2', '((a))', 'x y', 'a and', '(a or', ')a(', 'a | b & c', 'a  or  b  and  c and (d or e)',
];
function editModelDoc() {
  return {
    id: 'demo', name: 'Demo', compartments: { c: 'cytosol', e: 'extracellular' },
    metabolites: [{ id: 'a_c', name: 'A', compartment: 'c' }, { id: 'b_c', name: 'B', compartment: 'c' }],
    reactions: [], genes: [{ id: 'g1', name: 'gene one' }, 'g0'],
  };
}
const editScript = [
  ['metabolite', { id: 'c_c', name: 'C', compartment: 'c' }],
  ['metabolite', { id: 'a_c', compartment: 'c' }],
  ['metabolite', { id: 'x_z', name: 'X', compartment: 'z' }],
  ['metabolite', { id: 'x_c', name: '', compartment: 'c' }],
  ['metabolite', { id: '', name: 'X', compartment: 'c' }],
  ['metabolite', { id: 'has space', name: 'X', compartment: 'c' }],
  ['metabolite', { id: 'q_c', name: 'Q', compartment: 'c', charge: 'abc' }],
  ['metabolite', { id: 'q_e', name: ' Q out ', compartment: ' e ', charge: '-2', formula: ' C6H12O6 ' }],
  ['metabolite', { id: 'r_e', name: 'R', compartment: 'e', charge: null, formula: null }],
  ['metabolite', { id: 'p_c', name: 'P', compartment: 'c', charge: '' }],
  ['metabolite', { id: 10, name: 7, compartment: 'c', charge: true }],
  ['metabolite', { name: 'no id', compartment: 'c' }],
  ['reaction', { id: 'R_NEW', name: 'New reaction', metabolites: 'a_c:-1, b_c:1', lowerBound: -10, upperBound: 25, geneReactionRule: 'geneA and (geneB or geneC)' }],
  ['reaction', { id: 'R_GPR', metabolites: { a_c: -1, b_c: 1 }, lowerBound: 0, upperBound: 1000, gpr: 'g1 and (g2 or g3)' }],
  ['reaction', { id: 'R_BAD', metabolites: 'missing_c:-1', lowerBound: 0, upperBound: 1000 }],
  ['reaction', { id: 'R_BAD2', metabolites: 'a_c:-1,b_c:1', lowerBound: 10, upperBound: 0 }],
  ['reaction', { id: 'R_NEW', metabolites: 'a_c:-1', lowerBound: 0, upperBound: 1 }],
  ['reaction', { id: 'R_EMPTY', metabolites: {}, lowerBound: 0, upperBound: 1 }],
  ['reaction', { id: 'R_ZERO', metabolites: { a_c: 0 }, lowerBound: 0, upperBound: 1 }],
  ['reaction', { id: 'R_STR', metabolites: { a_c: '-1', c_c: '2' }, lowerBound: '-5', upperBound: '5', subsystem: ' Glycolysis ', name: '  ' }],
  ['reaction', { id: 'R_NOBOUNDS', metabolites: 'a_c:-1' }],
  ['reaction', { id: 'R_NULLBOUNDS', metabolites: 'a_c:-1', lowerBound: null, upperBound: null, geneReactionRule: null, gpr: 'gx1 or gx2' }],
  ['reaction', { id: 'R_EX', metabolites: 'q_e:-1', lowerBound: -10, upperBound: 1000, gpr: 'g0 or g1 or NEW and not' }],
  ['reaction', { id: 'R_ARR', metabolites: [1, 2], lowerBound: 0, upperBound: 1 }],
  ['reaction', { id: 'R_BADJSON', metabolites: '{"a_c": }', lowerBound: 0, upperBound: 1 }],
  ['reaction', { id: 'R_INF', metabolites: 'a_c:-1', lowerBound: -Infinity, upperBound: 1 }],
];
const editModel = new Model(editModelDoc());
const editSteps = editScript.map(([kind, spec]) => {
  const fn = kind === 'metabolite' ? addMetaboliteToModel : addReactionToModel;
  const result = attempt(() => { fn(editModel, spec); return true; });
  // Infinity is no JSON: the spec is recorded with it spelled out.
  return { kind, spec: JSON.parse(JSON.stringify(spec, (k, v) => (v === -Infinity ? '-Infinity' : v))), ...result };
});
await write(out, 'model-edit.json', {
  parseStoichiometry: stoichiometryCases.map((text) => {
    const r = attempt(() => Object.entries(parseStoichiometry(text)));
    return { text, ...r };
  }),
  geneIdsFromGPR: gprCases.map((rule) => ({ rule, out: geneIdsFromGPR(rule) })),
  edit: { start: editModelDoc(), steps: editSteps, final: modelJSON(editModel) },
});

// SBML write

const catalogue = (await readdir(join(root, 'models'))).filter((f) => f.endsWith('.json') && !['index.json', 'scenarios.json'].includes(f)).sort();
const coreDocs = new Map();
for (const f of catalogue) coreDocs.set(f.replace(/\.json$/, ''), JSON.parse(await readFile(join(root, 'models', f), 'utf8')));

const probeDocs = {
  gpr: {
    id: 'gpr', name: 'GPR probe', compartments: { c: 'Cytosol' },
    metabolites: [{ id: 'a_c', name: 'A', compartment: 'c', formula: 'C', charge: 0 }, { id: 'b_c', name: 'B', compartment: 'c', formula: 'C', charge: 0 }],
    reactions: [
      { id: 'R1', name: 'R one', metabolites: { a_c: -1, b_c: 1 }, lower_bound: 0, upper_bound: 1000, gene_reaction_rule: '(b0001 and b0002) or b0003' },
      { id: 'R2', name: 'R two', metabolites: { b_c: -1 }, lower_bound: -1000, upper_bound: 1000, gene_reaction_rule: 'b0004' },
    ],
    genes: [{ id: 'b0001', name: 'gene one' }], objective: { R1: 1 },
  },
  esc: {
    id: 'esc', compartments: { c: 'C' },
    metabolites: [{ id: 'm_c', name: 'a < b & "c"', compartment: 'c', formula: 'CHO', charge: 0 }],
    reactions: [{ id: 'R', name: 'x > y & z', metabolites: { m_c: -1 }, lower_bound: 0, upper_bound: 1 }], objective: { R: 1 },
  },
  formulas: {
    id: 'formulas', compartments: { c: 'Cytosol' },
    metabolites: [
      { id: 'bio_c', name: 'Biomass', compartment: 'c', charge: 0, formula: 'C37.024041172583H67.803828393317O20.609573521004' },
      { id: 'acp_c', name: 'ACP', compartment: 'c', charge: -1, formula: 'C11H20O6N2PSApo_acp' },
      { id: 'glc_c', name: 'Glucose', compartment: 'c', charge: 0, formula: 'C6H12O6' },
    ],
    reactions: [{ id: 'R', name: 'R', metabolites: { bio_c: -1, acp_c: -1, glc_c: 1 }, lower_bound: 0, upper_bound: 1000 }], objective: { R: 1 },
  },
  upload_probe: {
    id: 'upload_probe', compartments: { c: 'Cytosol' },
    metabolites: [
      { id: 'a_c', name: 'A', compartment: 'c', formula: 'C11H20O6N2PSApo_acp', charge: -1 },
      { id: 'b_c', name: 'B', compartment: 'c', formula: 'C6H12O6', charge: 0 },
    ],
    reactions: [
      { id: '12ppd-R_t', name: 'Transport', metabolites: { a_c: -1, b_c: 1 }, lower_bound: -10, upper_bound: 1000, gene_reaction_rule: '(b0001 and b0002) or b0003' },
      { id: 'EX_b(e)', name: 'B exchange', metabolites: { b_c: -1 }, lower_bound: -10, upper_bound: 1000 },
    ],
    objective: { '12ppd-R_t': 1 },
  },
  oddities: {
    id: '1 odd model!', name: '1 odd model!', compartments: { c: 'c', 'c-x': 'Cyto [x]', e: '' },
    metabolites: [
      { id: 'a[c]', name: 'a[c]', compartment: 'c', formula: '', charge: 1.5 },
      { id: 'μ_c-x', name: 'Mu', compartment: 'c-x', formula: 'Xy5', charge: -0 },
      { id: '😀_e', compartment: 'e', formula: 'R', charge: 2 },
      { id: 'n_c', compartment: 'c', formula: 'C6H12O6.5', charge: 0 },
    ],
    reactions: [
      { id: 'R(1)', name: 'R(1)', metabolites: { 'a[c]': -1e-7, 'μ_c-x': 2.5e21, '😀_e': 0 }, lower_bound: -0, upper_bound: 1e-7, gene_reaction_rule: 'A && B || (C | D & E)', subsystem: 'S & <T>' },
      { id: 'R2', metabolites: { n_c: -1 }, lower_bound: -999999, upper_bound: 999999, gene_reaction_rule: 'g-1 or g.2' },
      { id: 'R3', metabolites: { n_c: 1 }, lower_bound: 0.1, upper_bound: 0.1, objective_coefficient: 2 },
      { id: 'R4', metabolites: { n_c: -1, 'a[c]': 1 }, lower_bound: -1000, upper_bound: 0, gene_reaction_rule: '  ' },
    ],
    genes: ['g0', { id: 'g-1', name: 'g-1' }, { id: 'A', name: 'Gene A & co' }, { id: 'A', name: 'second A' }, { id: 7, name: 7 }],
  },
};
const sbmlIndex = { core: [], probes: {} };
for (const [name, doc] of coreDocs) {
  const xml = toSBML(new Model(doc));
  await writeFile(join(out, 'sbml', `${name}.xml`), xml);
  sbmlIndex.core.push(name);
}
for (const [name, doc] of Object.entries(probeDocs)) {
  const r = attempt(() => toSBML(new Model(doc)));
  sbmlIndex.probes[name] = { doc, ...r };
}
sbmlIndex.probes.objectiveMinimize = { doc: probeDocs.gpr, ok: toSBML(new Model(probeDocs.gpr), { objectiveId: 'o&1', objectiveType: 'minimize' }), opts: { objectiveId: 'o&1', objectiveType: 'minimize' } };
const badGpr = { ...probeDocs.gpr, reactions: [{ ...probeDocs.gpr.reactions[0], gene_reaction_rule: 'a and' }] };
sbmlIndex.probes.danglingRule = { doc: badGpr, ...attempt(() => toSBML(new Model(badGpr))) };
const mangleCases = ['12ethd[c_Ba]', 'EX_12ppd-R[u]tr_Bt', 'PFK(ppi)_Bt', '12ppd-R_e', 'EX_glc__D_e', 'BIOMASS_Ec_iAF1260_core_59p81M', "3'5'-cyclic", 'a😀', 'ü', '', ' '];
sbmlIndex.mangleId = mangleCases.flatMap((id) => ['M_', '', 'G_'].map((p) => ({ id, prefix: p, ...attempt(() => mangleId(id, p)) })));
sbmlIndex.parseGPR = gprCases.map((rule) => ({ rule, out: JSON.parse(JSON.stringify(parseGPR(rule))), genes: attempt(() => genesInGPR(rule)) }));
await write(out, 'sbml-write.json', sbmlIndex);

// SBML read

const sbmlFixture = `<?xml version="1.0" encoding="UTF-8"?>
<sbml xmlns="http://www.sbml.org/sbml/level3/version1/core"
 xmlns:fbc="http://www.sbml.org/sbml/level3/version1/fbc/version2"
 level="3" version="1" fbc:required="false">
 <model id="eco_sbml" name="SBML test model" fbc:strict="true">
  <listOfCompartments>
   <compartment id="c" name="Cytosol" constant="true"/>
   <compartment id="e" name="Extracellular" constant="true"/>
  </listOfCompartments>
  <listOfSpecies>
   <species id="a_e" name="A external" compartment="e" fbc:chemicalFormula="C" fbc:charge="0"/>
   <species id="a_c" name="A" compartment="c" fbc:chemicalFormula="C" fbc:charge="0"/>
  </listOfSpecies>
  <listOfParameters>
   <parameter id="LB_EX" value="-10" constant="true"/>
   <parameter id="LB_ZERO" value="0" constant="true"/>
   <parameter id="UB" value="1000" constant="true"/>
  </listOfParameters>
  <fbc:listOfGeneProducts>
   <fbc:geneProduct fbc:id="g1" fbc:label="gene one"/>
   <fbc:geneProduct fbc:id="g2" fbc:label="gene two"/>
  </fbc:listOfGeneProducts>
  <listOfReactions>
   <reaction id="EX_a_e" reversible="true" fbc:lowerFluxBound="LB_EX" fbc:upperFluxBound="UB">
    <listOfReactants><speciesReference species="a_e" stoichiometry="1"/></listOfReactants>
   </reaction>
   <reaction id="At" reversible="false" fbc:lowerFluxBound="LB_ZERO" fbc:upperFluxBound="UB">
    <listOfReactants><speciesReference species="a_e" stoichiometry="1"/></listOfReactants>
    <listOfProducts><speciesReference species="a_c" stoichiometry="1"/></listOfProducts>
    <fbc:geneProductAssociation><fbc:and>
      <fbc:geneProductRef fbc:geneProduct="g1"/>
      <fbc:geneProductRef fbc:geneProduct="g2"/>
    </fbc:and></fbc:geneProductAssociation>
   </reaction>
   <reaction id="BIOMASS" reversible="false" fbc:lowerFluxBound="LB_ZERO" fbc:upperFluxBound="UB">
    <listOfReactants><speciesReference species="a_c" stoichiometry="1"/></listOfReactants>
   </reaction>
  </listOfReactions>
  <fbc:listOfObjectives fbc:activeObjective="obj">
   <fbc:objective fbc:id="obj" fbc:type="maximize">
    <fbc:listOfFluxObjectives><fbc:fluxObjective fbc:reaction="BIOMASS" fbc:coefficient="1"/></fbc:listOfFluxObjectives>
   </fbc:objective>
  </fbc:listOfObjectives>
 </model>
</sbml>`;
const pairedFixture = sbmlFixture
  .replace('<species id="a_e" name="A external" compartment="e" fbc:chemicalFormula="C" fbc:charge="0"/>',
    '<species id="a_e" name="A external" compartment="e" fbc:chemicalFormula="C" fbc:charge="0"></species>')
  .replace('<species id="a_c" name="A" compartment="c" fbc:chemicalFormula="C" fbc:charge="0"/>',
    '<species id="a_c" name="A" compartment="c" fbc:chemicalFormula="C" fbc:charge="0"><notes><p>YEAST9_NOTE</p></notes></species>');
const provenanceXml = `<sbml level="3" version="1" xmlns:fbc="x"><model id="m"><listOfCompartments><compartment id="c"/></listOfCompartments>
 <listOfSpecies><species id="a_c" compartment="c"><annotation><rdf:li rdf:resource="https://identifiers.org/chebi:15377"/></annotation></species></listOfSpecies>
 <fbc:listOfGeneProducts><fbc:geneProduct fbc:id="g1"><annotation><rdf:li rdf:resource="https://identifiers.org/uniprot:P12345"/></annotation></fbc:geneProduct></fbc:listOfGeneProducts>
 <listOfReactions><reaction id="R" reversible="false"><listOfReactants><speciesReference species="a_c"/></listOfReactants><annotation><rdf:li rdf:resource="https://identifiers.org/rhea:10000"/></annotation></reaction></listOfReactions>
 </model></sbml>`;
// A Level 2 file in the COBRA Toolbox dialect: bounds and objective in the
// kinetic law, formula and gene rule in the notes, a boundary species.
const level2Xml = `<?xml version="1.0" encoding="UTF-8"?>
<sbml xmlns="http://www.sbml.org/sbml/level2/version4" level="2" version="4">
<model id="M_toy_DASH_L2" name="Toy &amp; L2">
<listOfCompartments><compartment id="C_c" name="cytosol"/><compartment id="C_e" name="extracellular"/><compartment id="b" /></listOfCompartments>
<listOfSpecies>
<species id="M_glc_DASH_D_LSQBKT_e_RSQBKT_" name="D-Glucose &lt;ext&gt;" compartment="C_e" charge="0"><notes><html:p>FORMULA: C6H12O6</html:p><html:p>CHARGE: 0</html:p></notes></species>
<species id="M_glc_DASH_D_c" name="D-Glucose" compartment="C_c" charge="-1"><notes><html:p>FORMULA:   C6H11O6R  </html:p></notes></species>
<species id="M_atp_c" compartment="C_c"/>
<species id="M_x_b" name="boundary" compartment="b" boundaryCondition="true"/>
<species id="M_noformula_c" name="" compartment="" />
<species id="M_12ppd__45__R_e" name="12ppd-R" compartment="C_e" fbc:charge="x" charge="3"/>
</listOfSpecies>
<listOfReactions>
<reaction id="R_EX_glc_LPAREN_e_RPAREN_" name="EX glc" reversible="true">
<notes><html:p>GENE_ASSOCIATION: (b1 and b2) or b3</html:p><html:p>SUBSYSTEM: Exchange &amp; transport</html:p></notes>
<listOfReactants><speciesReference species="M_glc_DASH_D_LSQBKT_e_RSQBKT_" stoichiometry="1"/></listOfReactants>
<listOfProducts><speciesReference species="M_x_b" stoichiometry="1"/></listOfProducts>
<kineticLaw><math/><listOfParameters><parameter id="LOWER_BOUND" value="-10"/><parameter id="UPPER_BOUND" value="999999"/><parameter id="OBJECTIVE_COEFFICIENT" value="0"/><parameter id="FLUX_VALUE" value="0"/></listOfParameters></kineticLaw>
</reaction>
<reaction id="R_GLCt" reversible="false">
<notes><html:p>GENE ASSOCIATION: b9</html:p></notes>
<listOfReactants><speciesReference species="M_glc_DASH_D_LSQBKT_e_RSQBKT_"/><speciesReference species="M_atp_c" stoichiometry="2"/></listOfReactants>
<listOfProducts><speciesReference species="M_glc_DASH_D_c" stoichiometry="1.5"/><speciesReference species="M_atp_c" stoichiometry="2"/></listOfProducts>
</reaction>
<reaction id="R_BIOMASS" name="biomass" reversible="false">
<listOfReactants><speciesReference species="M_glc_DASH_D_c" stoichiometry="1"/><speciesReference species="M_12ppd__45__R_e" stoichiometry="0.5"/></listOfReactants>
<kineticLaw><listOfParameters><parameter id="LOWER_BOUND" value="-999999"/><parameter id="UPPER_BOUND"/><parameter id="OBJECTIVE_COEFFICIENT" value="1"/></listOfParameters></kineticLaw>
</reaction>
<reaction id="R_unclosed" reversible="false">
<listOfReactants><speciesReference species="M_atp_c" stoichiometry="1"/></listOfReactants>
</model></sbml>`;
// Level 3 oddities: prefixes other than fbc:, paired and self-closing species,
// gene products with names, groups, two objectives with an active one, bounds
// given as numbers, an unknown species reference.
const level3Xml = `<?xml version="1.0" encoding="UTF-8"?>
<sbml xmlns="http://www.sbml.org/sbml/level3/version1/core" xmlns:fbc2="http://www.sbml.org/sbml/level3/version1/fbc/version2" xmlns:groups="http://www.sbml.org/sbml/level3/version1/groups/version1" level="3" version="1">
<model metaid="meta_m" id="M_12model" name="Twelve &amp; co" fbc2:strict="true">
<listOfCompartments><compartment id="C_c__45__1" name="cyto-1" constant="true"/><compartment metaid="m" id="m" constant="true"/></listOfCompartments>
<listOfSpecies>
<species metaid="s1" id="M_a__91__c__93__" name='A "one"' compartment="C_c__45__1" fbc2:charge="-1" fbc2:chemicalFormula="C2H3"/>
<species metaid="s2" id="M_b_m" name="B" compartment="m" fbc2:charge="2"><notes><body xmlns="http://www.w3.org/1999/xhtml"><p>FORMULA: C11H20O6N2PSApo_acp</p>
<p>Other:   note &amp; more</p></body></notes><annotation><rdf:RDF><rdf:Description><bqbiol:is><rdf:Bag><rdf:li rdf:resource="https://identifiers.org/kegg.compound/C00002"/><rdf:li rdf:resource="https://identifiers.org/chebi/CHEBI:30616"/><rdf:li rdf:resource="https://identifiers.org/kegg.compound/C00002"/></rdf:Bag></bqbiol:is></rdf:Description></rdf:RDF></annotation></species>
<species id="M_c_m" compartment="m" hasOnlySubstanceUnits="false"></species>
</listOfSpecies>
<listOfParameters><parameter id="lb" value="-50" constant="true"/><parameter id="ub" value="INF" constant="true"/><parameter id="zero" value="0"/><parameter id="big" value="1e3"/></listOfParameters>
<listOfReactions>
<reaction metaid="r1" id="R_R1" name="One" reversible="true" fbc2:lowerFluxBound="lb" fbc2:upperFluxBound="big">
<annotation><rdf:li rdf:resource="https://identifiers.org/ec-code/1.1.1.1"/><rdf:li rdf:resource="https://identifiers.org/rhea/10000"/></annotation>
<listOfReactants><speciesReference species="M_a__91__c__93__" stoichiometry="2" constant="true"/><speciesReference species="M_ghost" stoichiometry="1"/></listOfReactants>
<listOfProducts><speciesReference species="M_b_m" stoichiometry="1" constant="true"/><speciesReference species="M_a__91__c__93__" stoichiometry="1" constant="true"/></listOfProducts>
<fbc2:geneProductAssociation><fbc2:or><fbc2:and><fbc2:geneProductRef fbc2:geneProduct="G_b0001"/><fbc2:geneProductRef fbc2:geneProduct="G_b0002"/></fbc2:and><fbc2:geneProductRef fbc2:geneProduct="G_b__45__3"/></fbc2:or></fbc2:geneProductAssociation>
</reaction>
<reaction id="R_R2" reversible="false" fbc2:lowerFluxBound="-5" fbc2:upperFluxBound="ub">
<listOfReactants><speciesReference species="M_b_m"/></listOfReactants>
<listOfProducts><speciesReference species="M_c_m" stoichiometry="3"/></listOfProducts>
<fbc2:geneProductAssociation><fbc2:geneProductRef fbc2:geneProduct="G_b0001"/></fbc2:geneProductAssociation>
</reaction>
<reaction id="R_R3" fbc2:upperFluxBound="nope">
<listOfReactants><speciesReference species="M_c_m" stoichiometry="1"/></listOfReactants>
</reaction>
</listOfReactions>
<fbc2:listOfObjectives fbc2:activeObjective="second">
<fbc2:objective fbc2:id="first" fbc2:type="maximize"><fbc2:listOfFluxObjectives><fbc2:fluxObjective fbc2:reaction="R_R1" fbc2:coefficient="1"/></fbc2:listOfFluxObjectives></fbc2:objective>
<fbc2:objective fbc2:id="second" fbc2:type="minimize"><fbc2:listOfFluxObjectives><fbc2:fluxObjective fbc2:reaction="R_R2" fbc2:coefficient="-2"/><fbc2:fluxObjective fbc2:reaction="R_R3"/></fbc2:listOfFluxObjectives></fbc2:objective>
</fbc2:listOfObjectives>
<fbc2:listOfGeneProducts>
<fbc2:geneProduct metaid="g1" fbc2:id="G_b0001" fbc2:name="thrA" fbc2:label="b0001"><annotation><rdf:li rdf:resource="https://identifiers.org/uniprot/P00561"/></annotation></fbc2:geneProduct>
<fbc2:geneProduct fbc2:id="G_b0002"/>
<fbc2:geneProduct fbc2:id="G_b__45__3" fbc2:label=""/>
</fbc2:listOfGeneProducts>
<groups:listOfGroups><groups:group groups:id="g1" groups:name="Glycolysis" groups:kind="partonomy"><groups:listOfMembers><groups:member groups:idRef="R_R1"/></groups:listOfMembers></groups:group></groups:listOfGroups>
</model>
</sbml>`;
const textCases = {
  modelImportFixture: sbmlFixture,
  pairedSpecies: pairedFixture,
  provenance: provenanceXml,
  level2: level2Xml,
  level3: level3Xml,
  bom: `﻿${sbmlFixture}`,
  notSbml: '<xml/>',
  noSpecies: '<sbml><model id="m"><listOfReactions><reaction id="R"></reaction></listOfReactions></model></sbml>',
  noReactions: '<sbml><model><listOfSpecies><species id="a"/></listOfSpecies></model></sbml>',
  badUri: '<sbml><model><listOfSpecies><species id="a"><annotation>https://identifiers.org/x%ZZ</annotation></species></listOfSpecies><listOfReactions><reaction id="R"></reaction></listOfReactions></model></sbml>',
  unknownMetabolite: '<sbml><model id="m"><listOfSpecies><species id="a"/></listOfSpecies><listOfReactions><reaction id="R"><listOfReactants><speciesReference species="ghost"/></listOfReactants></reaction></listOfReactions></model></sbml>',
};
const readOptions = [
  {}, { normaliseCompartment: false }, { sentinelBound: 999999 }, { id: 'given', defaultBound: 500, sentinelBound: 1000 },
];
const describeRead = (xml, filename) => ({
  cobra: readOptions.map((opts) => {
    const r = attempt(() => parseCOBRASBML(xml, opts));
    return { opts, ...r, model: r.ok ? attempt(() => modelJSON(new Model(r.ok))) : null };
  }),
  upload: attempt(() => modelJSON(parseModelSBML(xml, filename))),
  file: attempt(() => modelJSON(parseModelFile(xml, filename.replace(/\.xml$/, '.txt')))),
});
const sbmlRead = { texts: {}, core: {}, probes: {}, unmangle: [] };
for (const [name, xml] of Object.entries(textCases)) sbmlRead.texts[name] = { xml, ...describeRead(xml, `${name}.xml`) };
for (const name of sbmlIndex.core) {
  const xml = await readFile(join(out, 'sbml', `${name}.xml`), 'utf8');
  sbmlRead.core[name] = describeRead(xml, `${name}.xml`);
}
for (const [name, probe] of Object.entries(sbmlIndex.probes)) {
  if (probe.ok) sbmlRead.probes[name] = describeRead(probe.ok, `${name}.xml`);
}
const unmangleIds = [
  'M_12ppd_DASH_R_LSQBKT_u_RSQBKT_', 'R_EX_glc_LPAREN_e_RPAREN_', 'M_12ethd__91__c_Ba__93__', 'M_a__55357____56832__',
  'x__65601__', 'M_a__99999999999999999999999__b', 'M_glc(e)', 'M_glc[e]', 'glc(e)x', 'M_M_x', 'M_', '_COMMA__PERIOD__APOS__FSLASH__BSLASH_',
  'M___45____45__', 'R_a__0055__', 'M_ä(ü)', 'M_x__65__y',
  // A lone surrogate (`__55357__` alone) is left out: a Rust string cannot hold one, and it reads as U+FFFD.
];
sbmlRead.unmangle = unmangleIds.flatMap((id) => ['M_', '', 'R_'].flatMap((prefix) => [true, false].map((n) => ({
  id, prefix, normaliseCompartment: n, out: unmangleId(id, prefix, { normaliseCompartment: n }),
}))));
await write(out, 'sbml-read.json', sbmlRead);

// Model import

const labelDoc = {
  id: 'labels', compartments: { c: 'Cytosol' },
  metabolites: [
    { id: 'a_c', name: 'Original A', compartment: 'c', annotation: { 'bigg.metabolite': ['a'], 'kegg.compound': ['C00001'], 'metanetx.chemical': ['MNXM1'] } },
    { id: 'b_c', name: 'Original B', compartment: 'c', annotation: { 'bigg.metabolite': ['b'], 'metacyc.compound': ['B-META'], 'seed.compound': ['cpd00002'] } },
  ],
  metNames: ['Alternative A', 'Alternative B'], metModelSEEDID: ['cpd00001', 'cpd00002'], metFormulas: ['H2O', 'CO2'],
  reactions: [{ id: 'R', name: 'Original reaction', metabolites: { a_c: -1, b_c: 1 }, lower_bound: 0, upper_bound: 1000, annotation: { 'bigg.reaction': ['RXN-A'], 'kegg.reaction': ['R00001'], 'kegg.pathway': ['path:map00010'] } }],
  rxnNames: ['Alternative reaction'], rxnMetaNetXID: ['MNXR1'], rxnMetaCycID: ['RXN-META'], rxnModelSEEDID: ['rxn00001'],
};
function modelDoc(id = 'eco') {
  return {
    id, compartments: { c: 'Cytosol', e: 'Extracellular' },
    metabolites: [{ id: 'a_c', name: 'A', compartment: 'c', formula: 'C', charge: 0 }, { id: 'a_e', name: 'A', compartment: 'e', formula: 'C', charge: 0 }],
    reactions: [
      { id: 'EX_a_e', name: 'A exchange', metabolites: { a_e: -1 }, lower_bound: -10, upper_bound: 1000 },
      { id: 'At', name: 'A transport', metabolites: { a_e: -1, a_c: 1 }, lower_bound: 0, upper_bound: 1000 },
      { id: 'BIOMASS', metabolites: { a_c: -1 }, lower_bound: 0, upper_bound: 1000, objective_coefficient: 1 },
    ],
  };
}
const withBroken = (patch) => { const d = modelDoc(); patch(d); return JSON.stringify(d); };
const describePrepared = (prepared) => ({
  entries: prepared.entries.map((e) => ({
    filename: e.filename, modelKey: e.modelKey, speciesId: e.speciesId, abundance: e.abundance, enabled: e.enabled,
    summary: e.summary, model: modelJSON(e.model),
  })),
  medium: prepared.medium,
});
const importCases = {
  core: catalogue.map((f) => ({ name: f, path: `models/${f}` })),
  json_and_sbml: [{ name: 'first.json', text: JSON.stringify(modelDoc('json_model')) }, { name: 'second.xml', text: sbmlFixture }],
  same_ids: [{ name: 'first.json', text: JSON.stringify(modelDoc('same')) }, { name: 'second.json', text: JSON.stringify(modelDoc('same')) }, { name: '', text: JSON.stringify(modelDoc('same')) }, { name: 'first.JSON', text: JSON.stringify(modelDoc('')) }],
  odd_names: [{ name: ' my model (v2).sbml', text: sbmlFixture }, { name: 'ünï.json', text: JSON.stringify(modelDoc('ü ñ')) }, { name: '.json', text: JSON.stringify(modelDoc('__')) }],
  labels: [{ name: 'labels.json', text: JSON.stringify(labelDoc) }],
  sbml_content_without_extension: [{ name: 'model.txt', text: `  \n${sbmlFixture}` }],
  level3_upload: [{ name: 'l3.xml', text: level3Xml }],
  empty: [],
  invalid_json: [{ name: 'bad.json', text: '{not-json' }],
  saved_map: [{ name: 'map.json', text: JSON.stringify({ format: 'EscherMCMCMap', metabolites: [], reactions: [] }) }],
  root_array: [{ name: 'arr.json', text: '[1,2]' }],
  root_null: [{ name: 'null.json', text: 'null' }],
  no_metabolites: [{ name: 'x.json', text: withBroken((d) => { delete d.metabolites; }) }],
  no_reactions: [{ name: 'x.json', text: withBroken((d) => { d.reactions = {}; }) }],
  dup_reaction: [{ name: 'dup.json', text: withBroken((d) => { d.reactions.push({ ...d.reactions[0] }); }) }],
  dup_metabolite: [{ name: 'dup.json', text: withBroken((d) => { d.metabolites.push({ ...d.metabolites[0] }); }) }],
  bad_metabolite_id: [{ name: 'x.json', text: withBroken((d) => { d.metabolites[1].id = 5; }) }],
  bad_reaction_id: [{ name: 'x.json', text: withBroken((d) => { d.reactions[1] = 'R'; }) }],
  bad_stoichiometry: [{ name: 'x.json', text: withBroken((d) => { d.reactions[1].metabolites = [1]; }) }],
  bad_coefficient: [{ name: 'x.json', text: withBroken((d) => { d.reactions[1].metabolites.a_c = 'x'; }) }],
  unknown_metabolite: [{ name: 'broken.json', text: withBroken((d) => { d.reactions.push({ id: 'broken', metabolites: { missing_c: -1 } }); }) }],
  no_reaction_rows: [{ name: 'x.json', text: withBroken((d) => { d.reactions = []; }) }],
  no_metabolite_rows: [{ name: 'x.json', text: JSON.stringify({ id: 'x', metabolites: [], reactions: [{ id: 'R', metabolites: {} }] }) }],
  string_bounds: [{ name: 'x.json', text: withBroken((d) => { d.reactions[0].lower_bound = '-7'; d.reactions[1].upper_bound = 'abc'; }) }],
  invalid_sbml: [{ name: 'bad.xml', text: '<xml/>' }],
  second_fails: [{ name: 'ok.json', text: JSON.stringify(modelDoc()) }, { name: 'bad.xml', text: textCases.noSpecies }],
};
const modelImport = { cases: {}, medium: {}, parseModelJSON: {} };
for (const [name, files] of Object.entries(importCases)) {
  const resolved = await Promise.all(files.map(async (f) => (f.path ? { name: f.name, text: await readFile(join(root, f.path), 'utf8') } : f)));
  const r = attempt(() => describePrepared(prepareImportedModels(resolved)));
  modelImport.cases[name] = { files: files.map((f) => (f.path ? { name: f.name, path: f.path } : f)), ...r };
}
modelImport.medium.core = inferCommunityMedium([...coreDocs.values()].map((d) => new Model(d)));
await write(out, 'model-import.json', modelImport);

// Workbooks

function zip(files, { method = 0, comment = '' } = {}) {
  const locals = [];
  const central = [];
  let offset = 0;
  for (const [name, text] of files) {
    const nameBuf = Buffer.from(name, 'utf8');
    const raw = Buffer.isBuffer(text) ? text : Buffer.from(text, 'utf8');
    const data = method === 8 ? deflateRawSync(raw) : raw;
    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(method, 8);
    local.writeUInt32LE(data.length, 18);
    local.writeUInt32LE(raw.length, 22);
    local.writeUInt16LE(nameBuf.length, 26);
    locals.push(local, nameBuf, data);
    const dir = Buffer.alloc(46);
    dir.writeUInt32LE(0x02014b50, 0);
    dir.writeUInt16LE(method, 10);
    dir.writeUInt32LE(data.length, 20);
    dir.writeUInt32LE(raw.length, 24);
    dir.writeUInt16LE(nameBuf.length, 28);
    dir.writeUInt32LE(offset, 42);
    central.push(dir, nameBuf);
    offset += local.length + nameBuf.length + data.length;
  }
  const body = Buffer.concat(locals);
  const directory = Buffer.concat(central);
  const commentBuf = Buffer.from(comment, 'utf8');
  const eocd = Buffer.alloc(22);
  eocd.writeUInt32LE(0x06054b50, 0);
  eocd.writeUInt16LE(files.length, 8);
  eocd.writeUInt16LE(files.length, 10);
  eocd.writeUInt32LE(directory.length, 12);
  eocd.writeUInt32LE(body.length, 16);
  eocd.writeUInt16LE(commentBuf.length, 20);
  return Buffer.concat([body, directory, eocd, commentBuf]);
}
function miniWorkbook(sheetXml, sharedStrings = [], opts = {}) {
  return zip([
    ['xl/workbook.xml', '<workbook><sheets><sheet name="data" sheetId="1" r:id="rId1"/></sheets></workbook>'],
    ['xl/_rels/workbook.xml.rels', '<Relationships><Relationship Id="rId1" Type="x/worksheet" Target="worksheets/sheet1.xml"/></Relationships>'],
    ['xl/sharedStrings.xml', `<sst>${sharedStrings.map((s) => `<si><t>${s}</t></si>`).join('')}</sst>`],
    ['xl/worksheets/sheet1.xml', `<worksheet><sheetData>${sheetXml}</sheetData></worksheet>`],
  ], opts);
}
const books = {
  by_column: miniWorkbook('<row r="1"><c r="A1" t="s"><v>0</v></c><c r="D1" t="s"><v>1</v></c></row><row r="2"><c r="A2" t="s"><v>2</v></c><c r="D2" t="s"><v>3</v></c></row>', ['head', 'tail', 'left', 'right']),
  kinds: miniWorkbook('<row r="1"><c r="A1"><v>42.5</v></c><c r="B1" t="inlineStr"><is><t>in</t><t>line</t></is></c><c r="C1" t="s"><v>0</v></c><c r="D1"/></row>', ['shared &amp; escaped']),
  deflated: miniWorkbook('<row r="1"><c r="A1"><v>1</v></c><c r="B1" t="str"><v>a &lt;b&gt; &#65;&#x42;&amp;#67;</v></c><c r="C1" t="b"><v>1</v></c><c r="AA1" t="e"><v>#N/A</v></c></row>', [], { method: 8 }),
  odd_cells: miniWorkbook([
    '<row r="7" spans="1:3"><c r="A7" s="1" t="s"><v> 1 </v></c><c r="B7" t="s"><v>9</v></c><c r="C7" t="s"><v></v></c><c r="D7" t="s"/><c r="E7" t="s"><v>1.5</v></c></row>',
    '<row><c r="a1"><v>lower</v></c><c><v>no ref</v></c><c r="B"><v>no digits</v></c><c r="C3"><v></v></c><c r="D3" t="inlineStr"><is><r><t xml:space="preserve"> sp </t></r><r><t/></r><t /><t>after</t></is></c></row>',
    '<row r="4"/><row r="5"><c r="A5"><v>swallowed?</v></c></row>',
    '<row r="6"><c r="A6" t="s"><v>0</v></c><c r="A6" t="s"><v>1</v></c></row>',
  ].join(''), ['zero', 'one']),
  two_sheets: zip([
    ['xl/workbook.xml', '<workbook><sheets><sheet name="first &amp; one" r:id="rId1"/><sheet name="second" sheetId="2" r:id="rId2"/><sheet name="orphan" r:id="rId9"/><sheet name="" r:id="rId1"/></sheets></workbook>'],
    ['xl/_rels/workbook.xml.rels', '<Relationships><Relationship Id="rId1" Target="/xl/worksheets/a.xml"/><Relationship Id="rId2" Target="xl/worksheets/b.xml" /><Relationship Id="rId3"/></Relationships>'],
    ['xl/worksheets/a.xml', '<worksheet><row r="1"><c r="A1"><v>α</v></c></row></worksheet>'],
    ['xl/worksheets/b.xml', '<worksheet><row r="2"><c r="B2"><v>β</v></c></row></worksheet>'],
  ], { method: 8, comment: 'a trailing comment' }),
  missing_sheet_part: zip([
    ['xl/workbook.xml', '<workbook><sheets><sheet name="gone" r:id="rId1"/></sheets></workbook>'],
    ['xl/_rels/workbook.xml.rels', '<Relationships><Relationship Id="rId1" Target="worksheets/gone.xml"/></Relationships>'],
  ]),
  no_workbook: zip([['xl/_rels/workbook.xml.rels', '<Relationships/>']]),
  bad_code_point: miniWorkbook('<row r="1"><c r="A1"><v>&#99999999;</v></c></row>'),
  not_zip: Buffer.from('this is not a zip file at all'),
  short: Buffer.from('PK'),
};
const xlsxIndex = {};
async function describeWorkbook(bytes) {
  const wb = attempt(() => readWorkbook(bytes));
  if (wb.error) return { error: wb.error };
  const sheets = {};
  for (const name of [...wb.ok.sheetNames, 'nope']) {
    sheets[name] = attempt(() => [...wb.ok.rows(name)]);
  }
  return { sheetNames: wb.ok.sheetNames, sheets };
}
for (const [name, bytes] of Object.entries(books)) {
  await writeFile(join(out, 'xlsx', `${name}.xlsx`), bytes);
  xlsxIndex[name] = await describeWorkbook(bytes);
}
await write(out, 'xlsx.json', xlsxIndex);

// MAT files

const describeTyped = (a) => ({ $typed: a.constructor.name, ...(a.shape ? { shape: a.shape } : {}), values: Array.from(a, (v) => (Number.isFinite(v) ? v : String(v))) });
function describeMat(v) {
  if (ArrayBuffer.isView(v)) return describeTyped(v);
  if (typeof v === 'string') return v;
  if (Array.isArray(v)) return v.map(describeMat);
  const keys = Object.keys(v);
  if (keys.join() === 'shape,ir,jc,values' && ArrayBuffer.isView(v.ir) && !Object.hasOwn(v.ir, 'shape')) {
    return { $sparse: { shape: v.shape, ir: describeTyped(v.ir), jc: describeTyped(v.jc), values: describeTyped(v.values) } };
  }
  return Object.fromEntries(keys.map((k) => [k, describeMat(v[k])]));
}
const scipyFixture = 'TUFUTEFCIDUuMCBNQVQtZmlsZSBQbGF0Zm9ybTogbnQsIENyZWF0ZWQgb246IFRodSBPY3QgIDggMjE6MDI6MjMgMjAyNgAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABSU0PAAAAhgEAAHicnVTNSsNAEJ78VYslehDxmIMPoDdviiDoQQRzEUQkTRYNtEnIj7Q3jz6Cj+DFNxAUvItHH8Gjj+Buukkm09SUTBkyM51v8n2d7ZoA8GwA9PhzlbsCMzNkrsqa8HXu9jRI71jqu2ehx0YJmLz2qFd4leAV5LO6DgOZT7jHkyCRX8GYpWVsQ2WjYRVnKEYhuCiO2X0ZeyxxYz9K/TAANo7SqawL3l+wWLdGeIv+XdSvN+jUUf8Gz44vb5xlcGoNp8LF3nLv02o4DY5Oz3Pc0z+68D676nKu2HU3nJvjHhDOkH2YH9Zl5NjKCu64XvwWYk6fu1V2/76R50GRCx4niEeP8Kf7F3O3y7k770BMzNtH89Za5umy90cTn/k99Ame7k2V/7Q2HH2vKitUv9mCG0j9Rd6kH89rOg+bgM8DwBZ3++MlSFniJ9bnq+UxK4ozNnTmddH9FIb3U/D4hm73kdCH7wt8B9H9NulbIfpEnrDbLPDCHH8I9XOvwGI9BuKH9f0BYC1C2A8AAAAuAAAAeJzjY2BgcABiNiDmAGIeBghghfIZgZgJSoPUFGem56WmIOSBoKEaSAAAWUMEAg8AAAAwAAAAeJzjY2BgcABiNiDmAGJeBghghfIZgZgJSoP4pXnFmel5qSkw9f+BgB1IAwB/PwfxDwAAADMAAAB4nONjYGBwAGI2IOYAYnYGCGCF8hmBmAlKg/hpRYnJJZn5eexQPgPDAXsGBoEDAGmFBbEPAAAANAAAAHic42NgYHAAYjYg5gBiFgYIYIXymaCYEYi5gDg5I7EoMbkktagYok4AqCMxOSkFAHH8Bjs=';
function concat(...items) {
  const result = new Uint8Array(items.reduce((size, item) => size + item.length, 0));
  let offset = 0;
  for (const item of items) { result.set(item, offset); offset += item.length; }
  return result;
}
function numeric(values, type = 5, little = true) {
  const widths = { 1: 1, 2: 1, 3: 2, 4: 2, 5: 4, 6: 4, 7: 4, 9: 8, 12: 8, 13: 8 };
  const methods = { 1: 'setInt8', 2: 'setUint8', 3: 'setInt16', 4: 'setUint16', 5: 'setInt32', 6: 'setUint32', 7: 'setFloat32', 9: 'setFloat64', 12: 'setBigInt64', 13: 'setBigUint64' };
  const bytes = new Uint8Array(values.length * widths[type]);
  const view = new DataView(bytes.buffer);
  values.forEach((value, i) => (widths[type] === 1 ? view[methods[type]](i, value) : view[methods[type]](i * widths[type], value, little)));
  return bytes;
}
function tag(type, data, little = true, small = false) {
  const bytes = new Uint8Array(small ? 8 : 8 + data.length + (type === 15 ? 0 : (8 - data.length % 8) % 8));
  const view = new DataView(bytes.buffer);
  if (small) { view.setUint16(0, type, little); view.setUint16(2, data.length, little); bytes.set(data, 4); }
  else { view.setUint32(0, type, little); view.setUint32(4, data.length, little); bytes.set(data, 8); }
  return bytes;
}
function matrix({ name = 'v', cls = 6, shape = [1, 2], type = 9, values = [1, 2], little = true, raw, flags = cls, extra = new Uint8Array(), nzmax = 0 } = {}) {
  return tag(14, concat(
    tag(6, numeric([flags, nzmax], 6, little), little),
    tag(5, numeric(shape, 5, little), little),
    tag(1, new TextEncoder().encode(name), little, name.length > 0 && name.length <= 4),
    tag(type, raw ?? numeric(values, type, little), little), extra,
  ), little);
}
function matFile(data, little = true) {
  const header = new Uint8Array(128);
  header.set(new TextEncoder().encode('MATLAB 5.0 MAT-file, Coxeter synthetic fixture'));
  new DataView(header.buffer).setUint16(124, 0x100, little);
  header[126] = little ? 73 : 77; header[127] = little ? 77 : 73;
  return concat(header, data);
}
function sparse(ir, jc, values, shape = [2, 2], nzmax = values.length, name = 'v') {
  const data = concat(tag(5, numeric(jc)), tag(9, numeric(values, 9)));
  return matrix({ name, cls: 5, shape, type: 5, raw: numeric(ir), extra: data, nzmax });
}
function structMatrix(name, fields, items, shape = [1, items.length]) {
  const width = 32;
  const namesBytes = new Uint8Array(width * fields.length);
  fields.forEach((f, i) => namesBytes.set(new TextEncoder().encode(f), i * width));
  const body = concat(
    tag(6, numeric([2, 0], 6)), tag(5, numeric(shape, 5)), tag(1, new TextEncoder().encode(name)),
    tag(5, numeric([width], 5), true, true), tag(1, namesBytes), ...items.flatMap((item) => fields.map((f) => item[f])),
  );
  return tag(14, body);
}
function cellMatrix(name, items, shape = [1, items.length]) {
  return tag(14, concat(tag(6, numeric([1, 0], 6)), tag(5, numeric(shape, 5)), tag(1, new TextEncoder().encode(name)), ...items));
}
const charMatrix = (name, text, shape = [1, [...text].length], type = 16) => matrix({ name, cls: 4, shape, type, raw: new TextEncoder().encode(text) });
const mats = {
  scipy: Uint8Array.from(Buffer.from(scipyFixture, 'base64')),
  little: matFile(matrix({ values: [Math.PI, -4.5] })),
  big: matFile(matrix({ little: false, values: [Math.PI, -4.5] }), false),
  utf16_little: matFile(matrix({ cls: 4, shape: [1, 2], type: 17, raw: numeric([0x3bc, 0x3b1], 4) })),
  utf16_big: matFile(matrix({ little: false, cls: 4, shape: [1, 2], type: 17, raw: numeric([0x3bc, 0x3b1], 4, false) }), false),
  utf32: matFile(matrix({ cls: 4, shape: [1, 2], type: 18, raw: numeric([0x3bc, 0x1f9ea], 6) })),
  surrogate_pair_utf16: matFile(matrix({ cls: 4, shape: [1, 2], type: 17, raw: numeric([0xd83d, 0xde00], 4) })),
  char_rows: matFile(charMatrix('c', 'adbecf', [3, 2])),
  char_bom: matFile(charMatrix('c', '﻿ab', [1, 2])),
  compressed_then_plain: matFile(concat(tag(15, deflateSync(matrix({ name: 'one', values: [3, 7] }))), matrix({ name: 'two', values: [4, 9] }))),
  every_type: matFile(concat(
    matrix({ name: 'i8', cls: 8, type: 1, values: [-128, 127] }), matrix({ name: 'u8', cls: 9, type: 2, values: [0, 255] }),
    matrix({ name: 'i16', cls: 10, type: 3, values: [-32768, 7] }), matrix({ name: 'u16', cls: 11, type: 4, values: [65535, 1] }),
    matrix({ name: 'i32', cls: 12, type: 5, values: [-2147483648, 1] }), matrix({ name: 'u32', cls: 13, type: 6, values: [4294967295, 1] }),
    matrix({ name: 'f32', cls: 7, type: 7, values: [0.1, -Infinity] }), matrix({ name: 'f64', values: [NaN, Infinity] }),
    matrix({ name: 'i64', cls: 14, type: 12, values: [-9007199254740991n, 5n] }), matrix({ name: 'u64', cls: 15, type: 13, values: [9007199254740991n, 0n] }),
    matrix({ name: 'empty', shape: [0, 0], values: [] }), matrix({ name: 'cube', shape: [1, 2, 1], values: [1, 2] }),
  )),
  structs: matFile(concat(
    structMatrix('one', ['a', 'b'], [{ a: matrix({ name: '', values: [1, 2] }), b: charMatrix('', 'text') }]),
    structMatrix('many', ['x'], [{ x: matrix({ name: '', shape: [1, 1], values: [1] }) }, { x: matrix({ name: '', shape: [1, 1], values: [2] }) }]),
    structMatrix('none', ['x'], [], [0, 0]),
    cellMatrix('cells', [charMatrix('', 'α'), matrix({ name: '', shape: [1, 1], values: [5] }), cellMatrix('', [charMatrix('', 'deep')])]),
    sparse([0, 1], [0, 1, 2], [2, -3], [2, 2], 2, 'sp'),
    sparse([], [0, 0, 0], [], [2, 2], 0, 'sp0'),
  )),
  // What the zlib layer makes of streams that are not clean: bytes after the
  // end of the stream, a stream cut short, a wrong checksum, no stream at all.
  zlib_trailing: matFile(tag(15, concat(deflateSync(matrix({ name: 'one', values: [3, 7] })), Uint8Array.of(1, 2, 3)))),
  zlib_truncated: matFile(tag(15, deflateSync(matrix()).subarray(0, -6))),
  zlib_checksum: (() => { const z = Uint8Array.from(deflateSync(matrix())); z[z.length - 1] ^= 0xff; return matFile(tag(15, z)); })(),
  zlib_empty: matFile(tag(15, new Uint8Array())),
  zlib_raw_deflate: matFile(tag(15, deflateRawSync(matrix()))),
  err_truncated_header: new Uint8Array(127),
  err_endian: (() => { const h = matFile(new Uint8Array()); h[126] = 0; return h; })(),
  err_version: (() => { const h = matFile(new Uint8Array()); h[124] = 2; return h; })(),
  err_truncated_tag: matFile(new Uint8Array(4)),
  err_clipped: matFile(matrix()).slice(0, -1),
  err_type: matFile(tag(8, new Uint8Array())),
  err_trailing: matFile(matrix({ extra: tag(9, numeric([1], 9)) })),
  err_dims: matFile(matrix({ shape: [-1, 2] })),
  err_limit: matFile(matrix({ shape: [20000001, 1] })),
  err_count: matFile(matrix({ shape: [1, 3] })),
  err_complex: matFile(matrix({ flags: 6 | 0x800 })),
  err_class: matFile(matrix({ cls: 3 })),
  err_int64: matFile(matrix({ type: 12, values: [9007199254740992n, 0n] })),
  err_utf8: matFile(matrix({ cls: 4, type: 16, raw: Uint8Array.of(255, 255) })),
  err_zlib: matFile(tag(15, Uint8Array.of(1, 2, 3, 4))),
  err_nested: matFile(tag(15, deflateSync(tag(15, deflateSync(matrix()))))),
  err_duplicate: matFile(concat(matrix(), matrix())),
  err_empty_name: matFile(matrix({ name: '' })),
  err_sparse_pointers: matFile(sparse([0, 1], [1, 1, 2], [2, -3])),
  err_sparse_monotone: matFile(sparse([0, 1], [0, 2, 1], [2, -3])),
  err_sparse_row: matFile(sparse([0, 2], [0, 1, 2], [2, -3])),
  err_sparse_storage: matFile(sparse([0], [0, 1, 2], [2, -3])),
  err_small_length: matFile(Uint8Array.of(1, 0, 0, 0, 0, 0, 0, 0, 9, 0, 0, 0, 0, 0, 0, 0).map((b, i) => (i === 2 ? 5 : b))),
  err_not_matrix: matFile(tag(9, numeric([1], 9))),
  err_char_3d: matFile(charMatrix('c', 'ab', [1, 1, 2])),
  err_field_dup: matFile(structMatrix('s', ['a', 'a'], [{ a: matrix({ name: '' }) }])),
};
const matIndex = {};
for (const [name, bytes] of Object.entries(mats)) {
  await writeFile(join(out, 'mat', `${name}.mat`), bytes);
  const r = await attemptAsync(async () => describeMat(await readMat5(bytes)));
  matIndex[name] = r.error ? { error: r.error } : { variables: r.ok };
}
await write(out, 'mat5.json', matIndex);

// Reproducibility

const RealDate = Date;
const FIXED = RealDate.UTC(2026, 9, 13, 12, 34, 56, 789);
async function frozenClock(fn) {
  globalThis.Date = class extends RealDate {
    constructor(...a) { super(...(a.length ? a : [FIXED])); }
    static now() { return FIXED; }
  };
  try { return await fn(); } finally { globalThis.Date = RealDate; }
}
// A store described in plain JSON (maps as entry lists) and rebuilt with Maps.
const storeDescriptions = {
  pair: {
    species: [{ id: 'Ec', file: 'ecoli_core_mc', abundance: 0.5, enabled: true }, { id: 'Sc', file: 'yeast_core_mc', abundance: 0.5, enabled: true, color: '#fff' }, { id: 'ghost', file: 'missing' }],
    scenario: { id: 'consorcio', name: 'Consorcio & co' }, objective: 'equal-growth', productTarget: null,
    medium: { glc__D: 10, o2: 20, '10': 1 }, overrides: [['Ec@PGI', { lower: 0, upper: 0 }]],
    parsimonious: true, pfbaBiomassPercent: 95, loopless: false, fvaEnabled: true,
    fvaRanges: [['Ec@PGI', { min: -1, max: 2 }]], growth: [['Ec', 0.4], ['Sc', 0.3]],
    map: { format: 'EscherMCMCMap', nodes: [], z: { b: 1, a: [1.5, 1e-7, 1e21] } },
    omics: {
      dataset: { source: 'rnaseq.csv', dataType: 'transcriptomics', conditions: ['A', 'B'], values: [['b0001', { A: 1, B: 2 }]] },
      enabled: true, mode: 'single', conditionA: 'A', conditionB: null, metric: 'log2fc', pseudocount: 1,
      gimmeEnabled: false, gimmeThreshold: null, gimmeObjectiveFraction: 0.9, gimmeResult: null, report: { geneCoverage: 0.51234, reactionCoverage: 1 },
    },
    solution: {
      status: 'optimal', objectiveValue: 0.873921, iterations: 42,
      fluxes: [['Ec@PGI', 4.86], ['Ec@PFK', 7.477], ['Sc@X', 'NaN']], reducedCosts: [['Ec@PGI', 0]], scaledReducedCosts: [['Ec@PGI', -0]],
      reducedCostScale: 'Infinity', shadowPrices: [['Ec@g6p_c', -0.02]], dualAnalysis: { available: true }, zuniga: null,
    },
  },
  gimme: {
    species: [{ id: 'm', file: 'methanogen_core_mc', abundance: 1, enabled: true }], scenario: null, objective: 'sum-biomass',
    medium: {}, overrides: [], parsimonious: false, pfbaBiomassPercent: 100, loopless: true, fvaEnabled: false, fvaRanges: [], growth: [],
    map: null,
    omics: {
      dataset: { source: null, dataType: 'proteomics', conditions: [], values: [] }, enabled: false, mode: 'pair', conditionA: 'x', conditionB: 'y',
      metric: 'diff', pseudocount: 0.5, gimmeEnabled: true, gimmeThreshold: 3, gimmeObjectiveFraction: 0.75, gimmeResult: { threshold: null, inconsistencyScore: 12.5 }, report: null,
    },
    solution: null,
  },
  bare: {
    species: [], scenario: { name: 'unnamed' }, objective: 'equal-growth', medium: {}, overrides: [], parsimonious: false, pfbaBiomassPercent: 100,
    loopless: false, fvaEnabled: false, fvaRanges: [], growth: [], map: null, omics: null, solution: null,
  },
};
const numberOf = (v) => (typeof v === 'string' ? Number(v) : v);
function buildStore(desc) {
  const models = new Map();
  for (const sp of desc.species) if (coreDocs.has(sp.file)) models.set(sp.file, new Model(coreDocs.get(sp.file)));
  const solution = desc.solution && {
    ...desc.solution,
    objectiveValue: numberOf(desc.solution.objectiveValue), reducedCostScale: numberOf(desc.solution.reducedCostScale),
    fluxes: new Map(desc.solution.fluxes.map(([k, v]) => [k, numberOf(v)])),
    reducedCosts: new Map(desc.solution.reducedCosts.map(([k, v]) => [k, numberOf(v)])),
    scaledReducedCosts: new Map(desc.solution.scaledReducedCosts.map(([k, v]) => [k, numberOf(v)])),
    shadowPrices: new Map(desc.solution.shadowPrices.map(([k, v]) => [k, numberOf(v)])),
  };
  const omics = desc.omics && { ...desc.omics, dataset: desc.omics.dataset && { ...desc.omics.dataset, values: new Map(desc.omics.dataset.values) } };
  return {
    ...desc, models, solution, omics, isZunigaStudy: false,
    overrides: new Map(desc.overrides), fvaRanges: new Map(desc.fvaRanges), growth: new Map(desc.growth),
    map: desc.map && { toJSON: () => desc.map },
  };
}
const reproFixture = { canonical: [], bundles: {}, compare: [], reports: [] };
const canonicalValues = [
  { b: 2, a: { d: 4, c: 3 } }, { a: 1, b: 2 }, 'plain string ü😀', '', [1, 'a', null, true, { z: 1, y: [] }],
  { '10': 1, '9': 2, a: 3, B: 4, _: 5, '01': 6, '4294967295': 7, '4294967294': 8 }, { n: [0.1, 1e21, 1e-7, -0, 123456789012345680000, 5e-324, 1.7976931348623157e308] },
  { s: 'quote " backslash \\ newline \n tab \t control \u0001 del \u007f sep  ' }, {},
  { id: 'm', reactions: [], metabolites: [] },
];
for (const v of canonicalValues) reproFixture.canonical.push({ value: v, canonical: repro.canonicalJSON(v), sha256: await repro.sha256(v) });
for (const [name, doc] of coreDocs) {
  const json = new Model(doc).toJSON();
  reproFixture.canonical.push({ model: name, canonical: repro.canonicalJSON(json), sha256: await repro.sha256(json) });
}
for (const [name, desc] of Object.entries(storeDescriptions)) {
  const store = buildStore(desc);
  const bundle = JSON.parse(JSON.stringify(await frozenClock(() => repro.createReproducibilityBundle(store))));
  const analysis = JSON.parse(JSON.stringify(repro.analysisProvenance(store)));
  const verified = await repro.verifyReproducibilityBundle(bundle);
  const tampered = JSON.parse(JSON.stringify(bundle));
  if (tampered.models.Ec) tampered.models.Ec.id = 'changed';
  tampered.analysis.method = 'tampered';
  reproFixture.bundles[name] = {
    store: desc, bundle, analysis, verify: verified,
    tampered: { bundle: tampered, verify: await repro.verifyReproducibilityBundle(tampered) },
    report: repro.methodsProvenanceReport(bundle),
  };
}
reproFixture.software = repro.softwareIdentity();
reproFixture.createdAt = new RealDate(FIXED).toISOString();
reproFixture.verifyErrors = await Promise.all([{ format: 'x' }, { format: repro.REPRO_FORMAT, version: 3 }, { format: repro.REPRO_FORMAT, version: '9' }, null].map(
  async (b) => ({ bundle: b, ...(await attemptAsync(() => repro.verifyReproducibilityBundle(b))) }),
));
reproFixture.verifyOdd = await Promise.all([
  { format: repro.REPRO_FORMAT, models: { a: 'string model', b: { x: 1 } }, modelHashes: { a: await repro.sha256('string model') } },
  { format: repro.REPRO_FORMAT, version: 2, models: ['x'], integrity: { payloadHash: 'nope' } },
].map(async (b) => ({ bundle: b, out: await repro.verifyReproducibilityBundle(b) })));
const solutions = [
  [{ status: 'optimal', objectiveValue: 1, fluxes: { R1: 2, R2: -3 }, reducedCosts: { R1: 0, R2: -0.2 }, scaledReducedCosts: { R1: 0, R2: -1 }, shadowPrices: { M1: 0.5 } },
    { status: 'optimal', objectiveValue: 1 + 1e-9, fluxes: { R1: 2 + 1e-9, R2: -3 }, reducedCosts: { R1: 0, R2: -0.2 }, scaledReducedCosts: { R1: 0, R2: -1 }, shadowPrices: { M1: 0.5 } }],
  [{ status: 'optimal', objectiveValue: 1, fluxes: { R1: 2, R2: 3 }, reducedCosts: {}, scaledReducedCosts: {}, shadowPrices: {} },
    { status: 'optimal', objectiveValue: 1.1, fluxes: { R1: 2.2 } }],
  [{ status: 'optimal', objectiveValue: null, fluxes: { b: 1, a: null, c: 'x', d: 5 } }, { status: 'infeasible', fluxes: { b: 1.5, a: null, c: 'x', e: 1 } }],
  [{}, {}], [null, null], [{ objectiveValue: 'x' }, { objectiveValue: 'x' }],
  [{ fluxes: { R: 1000 } }, { fluxes: { R: 1000.0005 } }], [{ fluxes: { R: 1e-8 } }, { fluxes: { R: 2e-8 } }],
  [{ status: { s: 1 }, fluxes: { R: { v: 1 } } }, { status: { s: 1 }, fluxes: { R: { v: 1 } } }],
];
for (const [a, c] of solutions) {
  for (const tol of [undefined, { absolute: 1e-3, relative: 0 }]) {
    reproFixture.compare.push({ archived: a, current: c, tolerances: tol ?? null, out: JSON.parse(JSON.stringify(repro.compareReplayedSolution(a, c, tol))) });
  }
}
const reportBundles = [
  {}, { metadata: { coxeterVersion: '3.1', createdAt: '2020-01-01' } },
  {
    metadata: { software: { version: '3.4', gitCommit: 'abc123', gitBranch: 'main', buildTime: 'today' }, createdAt: 'now' },
    community: { species: [{ id: 'a', model: 'm.json', abundance: 0.25 }, { id: 'b' }, 'odd'], objective: null, productTarget: 'EX_ac_e' },
    analysis: { objective: 'sum', method: 'pFBA', solver: 'S', fvaEnabled: 1, loopless: true, gimme: { condition: null, objectiveFraction: '0.555' } },
    constraints: { medium: { a: 1, b: 2 }, reactionOverrides: ['x'] }, modelHashes: { a: 'h1' },
    omics: { dataType: null, conditions: ['c1', null, 'c2'], mappingReport: { geneCoverage: 0.12345, reactionCoverage: 'x' } },
    solution: { status: 'optimal', objectiveValue: 0, dualAnalysis: { available: true }, reducedCosts: { a: 1 }, shadowPrices: {}, reducedCostScale: 2.5 },
    integrity: { algorithm: 'SHA-256', payloadHash: 'deadbeef' },
    study: { id: 'zuniga', scenarioId: 's1', comparison: 'mutant', sourceHashes: { 'M-models.mat': 'h2', 'CM.mat': 'h3' } },
  },
  { analysis: { method: 'pFBA', pfbaBiomassPercent: null, gimme: true }, omics: { conditions: [] }, solution: { status: null } },
];
for (const b of reportBundles) reproFixture.reports.push({ bundle: b, report: repro.methodsProvenanceReport(b) });
await write(out, 'reproducibility.json', reproFixture);

// Large inputs

const home = homedir();
const largeIndex = { sbml: [], json: [], published: [], mat: [], xlsx: [] };
const exists = (p) => existsSync(p);

// Published community members: written as SBML (exact text) and read back.
const publishedDir = join(root, 'models', 'published');
for (const dir of (await readdir(publishedDir)).sort()) {
  for (const f of (await readdir(join(publishedDir, dir))).filter((x) => x.startsWith('model-') || x === 'joint-model.json').sort()) {
    const rel = `models/published/${dir}/${f}`;
    const doc = JSON.parse(await readFile(join(root, rel), 'utf8'));
    const key = `${dir}-${f.replace(/\.json$/, '')}`;
    const xml = toSBML(new Model(doc));
    await writeFile(join(large, 'sbml', `${key}.xml`), xml);
    await write(large, `published-${key}.json`, {
      input: rel,
      faithful: modelJSON(new Model(parseCOBRASBML(xml, { normaliseCompartment: false }))),
      cobra: modelJSON(new Model(parseCOBRASBML(xml))),
      upload: attempt(() => modelJSON(parseModelFile(xml, `${key}.xml`))),
    });
    largeIndex.published.push({ key, input: rel });
  }
}

async function sbmlCase(key, path) {
  if (!exists(path)) return;
  const xml = await readFile(path, 'utf8');
  const raw = attempt(() => parseCOBRASBML(xml));
  const upload = attempt(() => parseModelSBML(xml, basename(path)));
  const prepared = attempt(() => describePrepared(prepareImportedModels([{ name: basename(path), text: xml }])));
  const written = upload.ok ? attempt(() => toSBML(upload.ok)) : null;
  if (written?.ok) await writeFile(join(large, 'sbml', `${key}-written.xml`), written.ok);
  await write(large, `sbml-${key}.json`, {
    input: path,
    cobra: raw.ok ? { raw: raw.ok, model: attempt(() => modelJSON(new Model(raw.ok))) } : raw,
    faithful: attempt(() => modelJSON(new Model(parseCOBRASBML(xml, { normaliseCompartment: false, sentinelBound: 999999 })))),
    upload: upload.ok ? { ok: modelJSON(upload.ok) } : upload,
    prepared,
    written: written ? (written.ok ? { file: `${key}-written.xml` } : written) : null,
    links: upload.ok ? linksOf(upload.ok) : null,
  });
  largeIndex.sbml.push({ key, input: path });
}
function linksOf(model) {
  return {
    metabolites: [...model.metabolites.values()].map((m) => annotationLinks(m.annotation, m.notes, 'metabolite')),
    reactions: [...model.reactions.values()].map((r) => annotationLinks(r.annotation, r.notes, 'reaction')),
    genes: model.genes.map((g) => annotationLinks(g.annotation, g.notes, 'gene')),
  };
}
const cobraXmlDir = join(home, 'Documents/GitHub/cobratoolbox/test/models/xml');
if (exists(cobraXmlDir)) for (const f of (await readdir(cobraXmlDir)).filter((x) => x.endsWith('.xml')).sort()) await sbmlCase(`cobratoolbox-${f.replace(/\.xml$/, '')}`, join(cobraXmlDir, f));
await sbmlCase('yeast-GEM', join(home, 'Documents/GitHub/pc_model_porcine/other_models/yeast9/yeast-GEM.xml'));

async function jsonCase(key, path) {
  if (!exists(path)) return;
  const text = await readFile(path, 'utf8');
  const prepared = prepareImportedModels([{ name: basename(path), text }]);
  const model = prepared.entries[0].model;
  const written = attempt(() => toSBML(model));
  const xml = written.ok;
  if (xml) await writeFile(join(large, 'sbml', `${key}-written.xml`), xml);
  await write(large, `json-${key}.json`, {
    input: path,
    prepared: describePrepared(prepared),
    written: xml ? { file: `${key}-written.xml` } : written,
    roundTrip: xml ? {
      faithful: attempt(() => modelJSON(new Model(parseCOBRASBML(xml, { normaliseCompartment: false })))),
      upload: attempt(() => modelJSON(parseModelFile(xml, `${key}.xml`))),
    } : null,
    links: linksOf(model),
  });
  largeIndex.json.push({ key, input: path });
}
await jsonCase('yeastgem-9.1.1', join(home, 'Downloads/yeastgem_v9.1.1-17.40.json'));
// Both real models uploaded together, as one community.
const together = [
  join(home, 'Documents/GitHub/pc_model_porcine/other_models/yeast9/yeast-GEM.xml'),
  join(home, 'Downloads/yeastgem_v9.1.1-17.40.json'),
];
if (together.every(exists)) {
  const files = await Promise.all(together.map(async (p) => ({ name: basename(p), text: await readFile(p, 'utf8') })));
  const prepared = prepareImportedModels(files);
  await write(large, 'json-together.json', {
    inputs: together,
    summary: prepared.entries.map((e) => ({ filename: e.filename, modelKey: e.modelKey, speciesId: e.speciesId, abundance: e.abundance, summary: e.summary })),
    medium: prepared.medium,
  });
  largeIndex.together = together;
}

const cobraMatDir = join(home, 'Documents/GitHub/cobratoolbox/test/models/mat');
for (const f of ['iAF1260', 'ecoli_core_model', 'AntCore', 'DAS', 'subDas', 'cardiac_mit_glcuptake_atpmax', 'modelReg', 'Ec_iAF1260_flux1', 'Recon1.0model']) {
  const path = join(cobraMatDir, `${f}.mat`);
  if (!exists(path)) continue;
  const bytes = new Uint8Array(await readFile(path));
  const r = await attemptAsync(async () => describeMat(await readMat5(bytes)));
  await write(large, `mat-${f}.json`, { input: path, ...(r.error ? { error: r.error } : { variables: r.ok }) });
  largeIndex.mat.push({ key: f, input: path });
}

const xlsxInputs = [
  'Documents/GitHub/pc_model_porcine/analysis/Flux_distribution_tyrosine.xlsx',
  'Documents/GitHub/pc_model_porcine/analysis/categorias_reacciones_diferencia_humano_cerdo.xlsx',
  'Documents/GitHub/pc_model_porcine/analysis/analysis_yields.xlsx',
  'Documents/GitHub/pc_model_porcine/manuscript/tablas_manuscrito.xlsx',
  'Documents/GitHub/evaluation_reconstruction_tools/MNX/manualDictionary.xlsx',
  'Downloads/elife-61968-supp1-v2.xlsx',
  'Downloads/Metagenomic samples.xlsx',
];
for (const rel of xlsxInputs) {
  const path = join(home, rel);
  if (!exists(path) || (await stat(path)).size > 20e6) continue;
  const key = basename(rel).replace(/\.xlsx$/, '').replace(/[^A-Za-z0-9_.-]+/g, '_');
  await write(large, `xlsx-${key}.json`, { input: path, ...(await describeWorkbook(await readFile(path))) });
  largeIndex.xlsx.push({ key, input: path });
}
await writeFile(join(large, 'index.json'), JSON.stringify(largeIndex, null, 1));

console.log(`wrote ${out} and ${large}`);
