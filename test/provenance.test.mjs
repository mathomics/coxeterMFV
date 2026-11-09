import test from 'node:test';
import assert from 'node:assert/strict';
import { annotationLinks, extractIdentifiersOrg, normalizeAnnotations } from '../src/core/provenance.js';
import { Model } from '../src/core/model.js';
import { parseModelSBML } from '../src/core/model-import.js';

test('COBRA annotations survive Model round trip',()=>{
 const raw={id:'p',compartments:{c:'c'},genes:[{id:'g1',annotation:{uniprot:'P12345'}}],
 metabolites:[{id:'a_c',compartment:'c',annotation:{chebi:'CHEBI:15377'},notes:{source:'curated'}}],
 reactions:[{id:'R',metabolites:{a_c:-1},annotation:{'kegg.reaction':'R00001'},notes:{pmid:'123'}}]};
 const m=new Model(raw),json=m.toJSON();
 assert.equal(m.metabolites.get('a_c').annotation.chebi,'CHEBI:15377');
 assert.equal(m.reactions.get('R').annotation['kegg.reaction'],'R00001');
 assert.equal(json.metabolites[0].annotation.chebi,'CHEBI:15377');
});
test('database links cover reaction metabolite and gene namespaces',()=>{
 assert.match(annotationLinks({chebi:'CHEBI:15377'}, {}, 'metabolite')[0].url,/ebi\.ac\.uk/);
 assert.match(annotationLinks({rhea:'RHEA:10000'}, {}, 'reaction')[0].url,/rhea-db\.org/);
 assert.match(annotationLinks({uniprot:'P12345'}, {}, 'gene')[0].url,/uniprot\.org/);
});
test('identifiers.org RDF resources are normalized',()=>{
 const a=extractIdentifiersOrg('<rdf:li rdf:resource="https://identifiers.org/kegg.reaction/R00010"/><rdf:li rdf:resource="https://identifiers.org/chebi:15377"/>');
 assert.deepEqual(a['kegg.reaction'],['R00010']); assert.deepEqual(a.chebi,['15377']);
});
test('SBML provenance is imported for species reaction and gene product',()=>{
 const xml=`<sbml level="3" version="1" xmlns:fbc="x"><model id="m"><listOfCompartments><compartment id="c"/></listOfCompartments>
 <listOfSpecies><species id="a_c" compartment="c"><annotation><rdf:li rdf:resource="https://identifiers.org/chebi:15377"/></annotation></species></listOfSpecies>
 <fbc:listOfGeneProducts><fbc:geneProduct fbc:id="g1"><annotation><rdf:li rdf:resource="https://identifiers.org/uniprot:P12345"/></annotation></fbc:geneProduct></fbc:listOfGeneProducts>
 <listOfReactions><reaction id="R" reversible="false"><listOfReactants><speciesReference species="a_c"/></listOfReactants><annotation><rdf:li rdf:resource="https://identifiers.org/rhea:10000"/></annotation></reaction></listOfReactions>
 </model></sbml>`;
 const m=parseModelSBML(xml,'p.xml');
 assert.deepEqual(m.metabolites.get('a_c').annotation.chebi,['15377']);
 assert.deepEqual(m.reactions.get('R').annotation.rhea,['10000']);
 assert.deepEqual(m.genes[0].annotation.uniprot,['P12345']);
});
