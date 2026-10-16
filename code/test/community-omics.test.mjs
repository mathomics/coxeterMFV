import test from 'node:test';
import assert from 'node:assert/strict';
import { parseMetatranscriptomics, communityOmicsMappingReport } from '../src/omics/community-omics.js';
import { reactionOmicsScores } from '../src/omics/overlay.js';
import { Model } from '../src/core/model.js';

test('long metatranscriptomics CSV maps MAG, gene, condition and expression',()=>{
 const {dataset,format}=parseMetatranscriptomics('MAG,gene,condition,TPM\nMAG1,g1,Control,10\nMAG1,g1,Treatment,40\nMAG1,g2,Control,5\nMAG1,g2,Treatment,10\n');
 assert.equal(format,'long'); assert.deepEqual(dataset.conditions,['Control','Treatment']);
 assert.equal(dataset.values.get('MAG1\u0000g1').values.Treatment,40);
});
test('TSV and common expression headers are accepted',()=>{
 const {dataset}=parseMetatranscriptomics('species\tlocus_tag\tcondition\tcounts\nsp\tg1\tA\t7\n');
 assert.equal(dataset.values.get('sp\u0000g1').values.A,7);
});
test('community report is species-aware and reports reaction coverage',()=>{
 const model=new Model({id:'m',compartments:{c:'c'},genes:[{id:'g1'},{id:'g2'}],
 metabolites:[{id:'a_c',compartment:'c'},{id:'b_c',compartment:'c'}],
 reactions:[{id:'R',metabolites:{a_c:-1,b_c:1},gene_reaction_rule:'g1 and g2'}]});
 const community={merged:{reactions:new Map([['MAG1@R',{}]])},reactionOwner:new Map([['MAG1@R',{speciesId:'MAG1',originalId:'R'}]])};
 const species=[{id:'MAG1',file:'m'}],models=new Map([['m',model]]);
 const {dataset}=parseMetatranscriptomics('MAG,gene,condition,expression\nMAG1,g1,A,8\nMAG1,g2,A,3\n');
 const scores=reactionOmicsScores(community,species,models,dataset,'A');
 assert.equal(scores.get('MAG1@R'),3);
 const report=communityOmicsMappingReport(community,species,models,dataset);
 assert.equal(report.measuredGenes,2); assert.equal(report.mappedReactions,1); assert.equal(report.reactionCoverage,1);
});
test('unmatched MAGs are explicit in mapping report',()=>{
 const model=new Model({id:'m',compartments:{c:'c'},metabolites:[{id:'a_c',compartment:'c'}],reactions:[{id:'EX_a_c',metabolites:{a_c:-1}}]});
 const community={merged:{reactions:new Map()},reactionOwner:new Map()};
 const species=[{id:'MAG1',file:'m'}],models=new Map([['m',model]]);
 const {dataset}=parseMetatranscriptomics('MAG,gene,condition,expression\nUNKNOWN,g1,A,8\n');
 const report=communityOmicsMappingReport(community,species,models,dataset);
 assert.deepEqual(report.unmatchedSpecies,['UNKNOWN']);
});
