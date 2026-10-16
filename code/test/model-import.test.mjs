import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Store } from '../src/ui/store.js';
import {
  parseModelJSON,
  parseModelSBML,
  parseModelFile,
  prepareImportedModels,
  inferCommunityMedium,
} from '../src/core/model-import.js';

function modelDoc(id = 'eco') {
  return {
    id,
    compartments: { c: 'Cytosol', e: 'Extracellular' },
    metabolites: [
      { id: 'a_c', name: 'A', compartment: 'c', formula: 'C', charge: 0 },
      { id: 'a_e', name: 'A', compartment: 'e', formula: 'C', charge: 0 },
    ],
    reactions: [
      {
        id: 'EX_a_e',
        name: 'A exchange',
        metabolites: { a_e: -1 },
        lower_bound: -10,
        upper_bound: 1000,
      },
      {
        id: 'At',
        name: 'A transport',
        metabolites: { a_e: -1, a_c: 1 },
        lower_bound: 0,
        upper_bound: 1000,
      },
      {
        id: 'BIOMASS',
        metabolites: { a_c: -1 },
        lower_bound: 0,
        upper_bound: 1000,
        objective_coefficient: 1,
      },
    ],
  };
}

test('a normal COBRA JSON model imports', () => {
  const model = parseModelJSON(JSON.stringify(modelDoc()), 'eco.json');
  assert.equal(model.id, 'eco');
  assert.equal(model.reactions.size, 3);
  assert.equal(model.metabolites.size, 2);
  assert.equal(model.biomassReactionId(), 'BIOMASS');
});

test('an exchange uptake lower bound becomes inferred community medium', () => {
  const model = parseModelJSON(JSON.stringify(modelDoc()), 'eco.json');
  assert.deepEqual(inferCommunityMedium([model]), { a: 10 });
});

test('multiple uploaded JSON files become distinct species', () => {
  const prepared = prepareImportedModels([
    { name: 'first.json', text: JSON.stringify(modelDoc('same')) },
    { name: 'second.json', text: JSON.stringify(modelDoc('same')) },
  ]);

  assert.equal(prepared.entries.length, 2);
  assert.deepEqual(prepared.entries.map((x) => x.speciesId), ['same', 'same_2']);
  assert.equal(prepared.entries[0].abundance, 0.5);
  assert.equal(prepared.entries[1].abundance, 0.5);
});

test('a reaction referencing an unknown metabolite is rejected clearly', () => {
  const raw = modelDoc();
  raw.reactions.push({
    id: 'broken',
    metabolites: { missing_c: -1 },
    lower_bound: 0,
    upper_bound: 1000,
  });

  assert.throws(
    () => parseModelJSON(JSON.stringify(raw), 'broken.json'),
    /unknown metabolite/,
  );
});

test('duplicate reaction ids are rejected before model construction', () => {
  const raw = modelDoc();
  raw.reactions.push({ ...raw.reactions[0] });

  assert.throws(
    () => parseModelJSON(JSON.stringify(raw), 'dup.json'),
    /duplicate reaction id/,
  );
});

test('invalid JSON names the offending file', () => {
  assert.throws(
    () => parseModelJSON('{not-json', 'bad.json'),
    /bad\.json: invalid JSON/,
  );
});


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

test('SBML Level 3 FBC imports compartments, bounds, objective and GPR', () => {
  const model = parseModelSBML(sbmlFixture, 'eco.xml');
  assert.equal(model.id, 'eco_sbml');
  assert.equal(model.compartmentNames.c, 'Cytosol');
  assert.equal(model.metabolites.get('a_e').formula, 'C');
  assert.equal(model.reactions.get('EX_a_e').lowerBound, -10);
  assert.equal(model.reactions.get('At').upperBound, 1000);
  assert.equal(model.reactions.get('At').geneReactionRule, 'g1 and g2');
  assert.equal(model.biomassReactionId(), 'BIOMASS');
  assert.deepEqual(inferCommunityMedium([model]), { a: 10 });
});

test('generic model parser detects SBML by extension or XML content', () => {
  assert.equal(parseModelFile(sbmlFixture, 'eco.sbml').id, 'eco_sbml');
  assert.equal(parseModelFile(sbmlFixture, 'eco.xml').reactions.size, 3);
});

test('paired SBML species keep notes after the genome-scale import optimisation', () => {
  const paired = sbmlFixture
    .replace(
      '<species id="a_e" name="A external" compartment="e" fbc:chemicalFormula="C" fbc:charge="0"/>',
      '<species id="a_e" name="A external" compartment="e" fbc:chemicalFormula="C" fbc:charge="0"></species>',
    )
    .replace(
      '<species id="a_c" name="A" compartment="c" fbc:chemicalFormula="C" fbc:charge="0"/>',
      '<species id="a_c" name="A" compartment="c" fbc:chemicalFormula="C" fbc:charge="0"><notes><p>YEAST9_NOTE</p></notes></species>',
    );
  const model = parseModelSBML(paired, 'paired.xml');
  assert.match(model.metabolites.get('a_c').notes.text, /YEAST9_NOTE/);
});

test('JSON and SBML files can be imported together as a community', () => {
  const prepared = prepareImportedModels([
    { name: 'first.json', text: JSON.stringify(modelDoc('json_model')) },
    { name: 'second.xml', text: sbmlFixture },
  ]);
  assert.equal(prepared.entries.length, 2);
  assert.equal(prepared.entries[1].model.id, 'eco_sbml');
  assert.equal(prepared.entries[0].abundance, 0.5);
  assert.equal(prepared.entries[1].abundance, 0.5);
});

test('invalid XML is rejected with a clear SBML error', () => {
  assert.throws(() => parseModelSBML('<xml/>', 'bad.xml'), /valid SBML document/);
});

// What an upload leaves on screen

const importRoot = join(dirname(fileURLToPath(import.meta.url)), '..');

/** A model with more metabolites than the automatic draw is willing to lay out. */
function oversizedDoc() {
  const metabolites = [];
  const reactions = [];
  for (let i = 0; i < 600; i += 1) {
    metabolites.push({ id: `m${i}_c`, name: `M${i}`, compartment: 'c', formula: 'C', charge: 0 });
  }
  for (let i = 0; i < 599; i += 1) {
    reactions.push({
      id: `R${i}`, name: `R${i}`,
      metabolites: { [`m${i}_c`]: -1, [`m${i + 1}_c`]: 1 },
      lower_bound: 0, upper_bound: 1000,
    });
  }
  reactions.push({
    id: 'BIOMASS', name: 'Biomass', metabolites: { m599_c: -1 },
    lower_bound: 0, upper_bound: 1000, objective_coefficient: 1,
  });
  return { id: 'oversized', compartments: { c: 'Cytosol' }, metabolites, reactions };
}

test('uploading a small model draws it, rather than leaving an empty cell', async () => {
  const store = new Store('.');
  const text = await readFile(join(importRoot, 'models', 'ecoli_core_mc.json'), 'utf8');

  await store.importModelFiles([{ name: 'ecoli_core_mc.json', text }]);

  // The case this covers: a shell (cells and compartments) drawn with nothing
  // inside it reads as an import that silently failed.
  assert.ok(store.map.cells.length === 1, 'the cell is drawn');
  assert.ok(store.map.reactions.size > 50, `reactions drawn: ${store.map.reactions.size}`);
  assert.ok(store.map.nodes.size > 50, `metabolites drawn: ${store.map.nodes.size}`);
  assert.equal(store.mapMode, 'full');
  assert.match(store.status.message, /Drawn in full/);
});

test('uploading a model too large to lay out says so instead of going quiet', async () => {
  const store = new Store('.');

  await store.importModelFiles([{ name: 'oversized.json', text: JSON.stringify(oversizedDoc()) }]);

  // Too crowded to lay out in the time anyone will wait, and with no annotated
  // central carbon metabolism to fall back on — so it stays empty, but the
  // status has to say that, and say what to press.
  assert.equal(store.map.reactions.size, 0);
  assert.equal(store.mapMode, 'empty');
  assert.match(store.status.message, /too large to lay out/);
  assert.match(store.status.message, /Full map/);
});
