/**
 * GIMME: Gene Inactivity Moderated by Metabolism and Expression.
 * Becker & Palsson (2008), adapted to Coxeter's bounded LP solver.
 *
 * 1. Maximise the required metabolic objective.
 * 2. Require a configurable fraction of that optimum.
 * 3. Minimise flux through reactions whose expression is below threshold,
 *    weighted by (threshold - expression).
 *
 * Absolute flux is linearised as v = v+ - v-.
 */
import { SparseColumns, solveLP, LP_OPTIMAL } from '../solver/simplex.js';

/**
 * Runs GIMME on an FBA problem (with optimize(), A, lb, ub, c, rxnIds and
 * metIds). `expressionScores` maps reaction id to score.
 * `threshold` defaults to the lower quartile of the measured scores and
 * `objectiveFraction` to 0.9. Returns the fluxes, duals of the GIMME LP, the
 * inconsistency score and the set of penalised reactions left inactive.
 */
export function runGIMME(problem, expressionScores, {
  threshold = null, objectiveFraction = 0.9,
} = {}) {
  const base=problem.optimize();
  if(base.status!==LP_OPTIMAL) return {...base,gimme:true,reason:'base_objective_failed'};
  const measured=[...expressionScores.values()].filter(Number.isFinite);
  if(!measured.length) throw new Error('GIMME needs expression mapped to at least one reaction.');
  // Number(null) is 0 and Number.isFinite(0) is true, so testing the coerced
  // value alone would turn every automatic threshold into 0. With a cutoff of 0
  // no reaction is ever below it, and GIMME would quietly do nothing. The
  // absence of a threshold has to be checked before coercing it.
  const given = threshold === null || threshold === undefined || threshold === ''
    ? NaN
    : Number(threshold);
  const cutoff = Number.isFinite(given) ? given : quantile(measured, 0.25);
  const fraction=Math.min(1,Math.max(0,Number(objectiveFraction)||0));
  const n=problem.rxnIds.length,m=problem.metIds.length;
  const A=new SparseColumns(m+1,2*n+1), b=new Float64Array(m+1);
  const lb=new Float64Array(2*n+1),ub=new Float64Array(2*n+1),c=new Float64Array(2*n+1);
  const penalties=new Map();

  for(let j=0;j<n;j++){
    const col=problem.A.column(j);
    for(let k=0;k<col.length;k+=2){A.add(col[k],j,col[k+1]);A.add(col[k],n+j,-col[k+1]);}
    lb[j]=Math.max(0,problem.lb[j]); ub[j]=Math.max(0,problem.ub[j]);
    lb[n+j]=Math.max(0,-problem.ub[j]); ub[n+j]=Math.max(0,-problem.lb[j]);
    const rid=problem.rxnIds[j],expr=expressionScores.get(rid);
    const penalty=Number.isFinite(expr)&&expr<cutoff ? cutoff-expr : 0;
    penalties.set(rid,penalty); c[j]=penalty;c[n+j]=penalty;
    if(problem.c[j]){A.add(m,j,problem.c[j]);A.add(m,n+j,-problem.c[j]);}
  }
  // objective - slack = required objective, slack >= 0 => objective >= required
  const slack=2*n; A.add(m,slack,-1); lb[slack]=0; ub[slack]=Infinity;
  const required=base.objectiveValue*fraction; b[m]=required;
  const res=solveLP({A,b,c,lb,ub,maximize:false},{});
  if(res.status!==LP_OPTIMAL) return {status:res.status,objectiveValue:NaN,fluxes:new Map(),gimme:true,threshold:cutoff,requiredObjective:required,penalties,message:res.message};
  const fluxes=new Map(); let achieved=0;
  problem.rxnIds.forEach((rid,j)=>{const v=res.x[j]-res.x[n+j];fluxes.set(rid,Math.abs(v)<1e-9?0:v);achieved+=problem.c[j]*v;});
  const inactive=new Set([...penalties].filter(([rid,p])=>p>0 && Math.abs(fluxes.get(rid)??0)<1e-9).map(([rid])=>rid));

  // The first m duals belong to the original metabolite mass balances. The
  // final row is GIMME's minimum-objective constraint and is reported
  // separately: it is not a metabolite shadow price.
  const shadowPrices=new Map();
  problem.metIds.forEach((mid,i)=>shadowPrices.set(mid,res.duals?.[i]??0));
  const objectiveConstraintShadowPrice=res.duals?.[m]??null;

  // GIMME solves a split-variable LP (v = v+ - v-). Preserve both directional
  // reduced costs for auditability and expose one reaction-level value for the
  // currently active direction. At zero flux there is no unique signed
  // reaction-level derivative, so the aggregate value is deliberately 0 and
  // callers can inspect the directional pair instead.
  const directionalReducedCosts=new Map();
  const reducedCosts=new Map();
  problem.rxnIds.forEach((rid,j)=>{
    const forward=res.reducedCosts?.[j]??0;
    const reverse=res.reducedCosts?.[n+j]??0;
    directionalReducedCosts.set(rid,{forward,reverse});
    const v=fluxes.get(rid)??0;
    reducedCosts.set(rid,v>1e-9?forward:(v<-1e-9?-reverse:0));
  });
  const reducedCostScale=Math.max(0,...[...reducedCosts.values()].map(v=>Math.abs(v)));
  const scaledReducedCosts=new Map([...reducedCosts].map(([rid,v])=>[rid,reducedCostScale>0?v/reducedCostScale:0]));

  return {status:LP_OPTIMAL,objectiveValue:achieved,fluxes,shadowPrices,reducedCosts,scaledReducedCosts,reducedCostScale,
    directionalReducedCosts,objectiveConstraintShadowPrice,iterations:res.iterations,
    dualAnalysis:{available:true,context:'GIMME penalty-minimization LP',interpretation:'Duals and reduced costs refer to the contextual GIMME LP, not the unconstrained FBA optimum.'},
    gimme:true,threshold:cutoff,objectiveFraction:fraction,requiredObjective:required,
    inconsistencyScore:res.objective,penalties,inactive,message:''};
}
function quantile(values,q){const x=[...values].sort((a,b)=>a-b);const p=(x.length-1)*q,l=Math.floor(p),h=Math.ceil(p);return l===h?x[l]:x[l]*(h-p)+x[h]*(p-l);}
