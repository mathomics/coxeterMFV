import test from 'node:test';
import assert from 'node:assert/strict';
import { Model } from '../src/core/model.js';
import { FluxProblem } from '../src/solver/fba.js';
import { runGIMME } from '../src/omics/gimme.js';

function toy(){
 return new Model({id:'gimme',compartments:{c:'c'},metabolites:[{id:'a_c',compartment:'c'}],
 reactions:[
  {id:'UP',metabolites:{a_c:1},lower_bound:0,upper_bound:10},
  {id:'BIOMASS',metabolites:{a_c:-1},lower_bound:0,upper_bound:1000,objective_coefficient:1},
  {id:'WASTE',metabolites:{a_c:-1},lower_bound:0,upper_bound:1000},
 ]});
}
test('GIMME preserves requested fraction of metabolic objective',()=>{
 const p=new FluxProblem(toy());
 const r=runGIMME(p,new Map([['UP',10],['BIOMASS',10],['WASTE',0]]),{threshold:5,objectiveFraction:.9});
 assert.equal(r.status,'optimal'); assert.ok(r.objectiveValue>=9-1e-7); assert.equal(r.fluxes.get('WASTE'),0);
});
test('GIMME auto threshold is expression P25 and exposes penalties',()=>{
 const p=new FluxProblem(toy());
 const r=runGIMME(p,new Map([['UP',8],['BIOMASS',4],['WASTE',0]]),{objectiveFraction:.8});
 assert.equal(r.gimme,true); assert.ok(Number.isFinite(r.threshold)); assert.ok(r.penalties.get('WASTE')>0);
});

test('GIMME exposes contextual duals and directional reduced costs',()=>{
 const p=new FluxProblem(toy());
 const r=runGIMME(p,new Map([['UP',10],['BIOMASS',10],['WASTE',0]]),{threshold:5,objectiveFraction:.9});
 assert.equal(r.status,'optimal');
 assert.equal(r.dualAnalysis.available,true);
 assert.match(r.dualAnalysis.context,/GIMME/);
 assert.equal(r.shadowPrices.size,p.metIds.length);
 assert.equal(r.reducedCosts.size,p.rxnIds.length);
 assert.equal(r.scaledReducedCosts.size,p.rxnIds.length);
 assert.equal(r.directionalReducedCosts.size,p.rxnIds.length);
 assert.ok(Number.isFinite(r.objectiveConstraintShadowPrice));
 for(const value of r.scaledReducedCosts.values()) assert.ok(Math.abs(value)<=1+1e-12);
});
