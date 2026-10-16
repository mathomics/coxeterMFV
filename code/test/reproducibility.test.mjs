import test from 'node:test';
import assert from 'node:assert/strict';
import { sha256, canonicalJSON, compareReplayedSolution, verifyReproducibilityBundle, REPRO_FORMAT } from '../src/core/reproducibility.js';

test('canonical JSON and SHA-256 are independent of object key order',async()=>{
 assert.equal(canonicalJSON({b:2,a:{d:4,c:3}}),canonicalJSON({a:{c:3,d:4},b:2}));
 assert.equal(await sha256({b:2,a:1}),await sha256({a:1,b:2}));
});
test('bundle verification detects model tampering',async()=>{
 const model={id:'m',reactions:[],metabolites:[]}; const modelHash=await sha256(model);
 const bundle={format:REPRO_FORMAT,version:1,metadata:{},models:{sp:model},modelHashes:{sp:modelHash},community:{},constraints:{},analysis:{},omics:null,map:null,solution:null};
 bundle.integrity={algorithm:'SHA-256',payloadHash:await sha256(bundle)};
 let check=await verifyReproducibilityBundle(bundle);assert.equal(check.valid,true);
 bundle.models.sp.id='changed';check=await verifyReproducibilityBundle(bundle);assert.equal(check.valid,false);assert.equal(check.modelChecks.sp,false);
});
test('bundle verification detects manifest tampering',async()=>{
 const bundle={format:REPRO_FORMAT,version:1,metadata:{},models:{},modelHashes:{},community:{objective:'growth'},constraints:{},analysis:{method:'FBA'},omics:null,map:null,solution:null};
 bundle.integrity={algorithm:'SHA-256',payloadHash:await sha256(bundle)};
 bundle.analysis.method='pFBA';const check=await verifyReproducibilityBundle(bundle);assert.equal(check.payloadValid,false);
});

test('numerical replay comparison accepts values within tolerance',()=>{
 const archived={status:'optimal',objectiveValue:1,fluxes:{R1:2,R2:-3},reducedCosts:{R1:0,R2:-.2},scaledReducedCosts:{R1:0,R2:-1},shadowPrices:{M1:.5}};
 const current={status:'optimal',objectiveValue:1+1e-9,fluxes:new Map([['R1',2+1e-9],['R2',-3]]),reducedCosts:new Map([['R1',0],['R2',-.2]]),scaledReducedCosts:new Map([['R1',0],['R2',-1]]),shadowPrices:new Map([['M1',.5]])};
 const r=compareReplayedSolution(archived,current,{absolute:1e-7,relative:1e-6});
 assert.equal(r.pass,true);assert.equal(r.fluxes.count,2);assert.ok(r.fluxes.maxAbsError>0);
});
test('numerical replay comparison reports divergent and missing values',()=>{
 const archived={status:'optimal',objectiveValue:1,fluxes:{R1:2,R2:3},reducedCosts:{},scaledReducedCosts:{},shadowPrices:{}};
 const current={status:'optimal',objectiveValue:1.1,fluxes:new Map([['R1',2.2]])};
 const r=compareReplayedSolution(archived,current,{absolute:1e-7,relative:1e-6});
 assert.equal(r.pass,false);assert.equal(r.checks.objective,false);assert.equal(r.checks.fluxes,false);assert.equal(r.fluxes.missing,1);
});

test('replay comparison exposes per-family checks for the visual report',()=>{
 const archived={status:'optimal',objectiveValue:2,fluxes:{R:1},reducedCosts:{R:0},scaledReducedCosts:{R:0},shadowPrices:{M:.25}};
 const current={status:'optimal',objectiveValue:2,fluxes:new Map([['R',1]]),reducedCosts:new Map([['R',0]]),scaledReducedCosts:new Map([['R',0]]),shadowPrices:new Map([['M',.25]])};
 const r=compareReplayedSolution(archived,current);
 assert.deepEqual(r.checks,{status:true,objective:true,fluxes:true,reducedCosts:true,scaledReducedCosts:true,shadowPrices:true});
 assert.equal(r.pass,true);
});
