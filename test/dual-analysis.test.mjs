import test from 'node:test';
import assert from 'node:assert/strict';
import { SparseColumns, solveLP, LP_OPTIMAL } from '../src/solver/simplex.js';
import { FluxProblem } from '../src/solver/fba.js';
import { Model } from '../src/core/model.js';

test('simplex returns dual and reduced-cost arrays',()=>{
 const A=new SparseColumns(1,2);A.set(0,0,1);A.set(0,1,1);
 const r=solveLP({A,b:[1],c:[1,0],lb:[0,0],ub:[10,10],maximize:true});
 assert.equal(r.status,LP_OPTIMAL);assert.equal(r.duals.length,1);assert.equal(r.reducedCosts.length,2);
 assert.ok(Math.abs(r.duals[0]-1)<1e-7);assert.ok(Math.abs(r.reducedCosts[0])<1e-7);assert.ok(Math.abs(r.reducedCosts[1]+1)<1e-7);
});
test('FluxProblem maps duals to metabolites and reduced costs to reactions',()=>{
 const model=new Model({id:'dual-test',metabolites:[{id:'m_c',name:'m',compartment:'c'}],reactions:[
  {id:'SOURCE',name:'source',metabolites:{m_c:1},lower_bound:0,upper_bound:1},
  {id:'BIOMASS',name:'biomass',metabolites:{m_c:-1},lower_bound:0,upper_bound:1000,objective_coefficient:1}
 ],genes:[],compartments:{c:'cytosol'}});
 const r=new FluxProblem(model).optimize();
 assert.equal(r.status,LP_OPTIMAL);assert.equal(r.dualAnalysis.available,true);
 assert.equal(r.shadowPrices.has('m_c'),true);assert.equal(r.reducedCosts.has('SOURCE'),true);assert.equal(r.reducedCosts.has('BIOMASS'),true);
 assert.equal(r.scaledReducedCosts.size,r.reducedCosts.size);
 assert.ok([...r.scaledReducedCosts.values()].every(v=>v>=-1-1e-12&&v<=1+1e-12));
 const maxAbs=Math.max(...[...r.reducedCosts.values()].map(Math.abs));assert.equal(r.reducedCostScale,maxAbs);
 if(maxAbs>0)assert.ok(Math.abs(Math.max(...[...r.scaledReducedCosts.values()].map(Math.abs))-1)<1e-12);
});
