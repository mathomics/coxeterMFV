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

test('GLPK reports a reduced cost for every reaction, and each meets the optimality conditions as the simplex ones do', async () => {
  const { readFile } = await import('node:fs/promises');
  // The vendored build the browser runs, handed over as the single-file build does.
  const vendored = await import('../assets/vendor/glpk.js');
  globalThis.__GLPK_FACTORY__ = vendored.default ?? vendored;
  globalThis.__GLPK_WASM__ = (await readFile(new URL('../assets/vendor/glpk.wasm', import.meta.url))).buffer;
  const { initGLPK } = await import('../src/solver/glpk.js');
  assert.ok(await initGLPK(), 'GLPK loads');
  const model = new Model(JSON.parse(await readFile(new URL('../models/ecoli_core_mc.json', import.meta.url), 'utf8')));
  for (const backend of ['glpk', 'simplex']) {
    const problem = new FluxProblem(model);
    problem.backendChoice = backend;
    const result = problem.optimize();
    assert.equal(result.solver, backend);
    assert.equal(result.reducedCosts.size, problem.rxnIds.length);
    // A maximum: a reaction held at its lower bound may only lose by rising
    // (rc <= 0), one at its upper bound only by falling (rc >= 0), and one
    // strictly between its bounds has rc = 0.
    let limiting = 0;
    problem.rxnIds.forEach((rid, j) => {
      const flux = result.fluxes.get(rid);
      const rc = result.reducedCosts.get(rid);
      const atLower = Math.abs(flux - problem.lb[j]) < 1e-6;
      const atUpper = Math.abs(flux - problem.ub[j]) < 1e-6;
      if (atLower && atUpper) return;
      if (atLower) {
        assert.ok(rc <= 1e-6, `${backend} ${rid} at its lower bound has rc ${rc}`);
      } else if (atUpper) {
        assert.ok(rc >= -1e-6, `${backend} ${rid} at its upper bound has rc ${rc}`);
      } else {
        assert.ok(Math.abs(rc) <= 1e-6, `${backend} ${rid} between its bounds has rc ${rc}`);
      }
      if (Math.abs(rc) > 1e-6) limiting += 1;
    });
    assert.ok(limiting > 0, `${backend} finds limiting reactions`);
    assert.ok([...result.scaledReducedCosts.values()].some((value) => Math.abs(Math.abs(value) - 1) < 1e-12));
  }
});
