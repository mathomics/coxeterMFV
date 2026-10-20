/**
 * Scientific provenance and reproducibility bundle.
 * Pure data layer: deterministic canonical JSON + SHA-256 digests.
 */
export const REPRO_FORMAT='CoxeterReproducibilityBundle';
export const REPRO_VERSION=2;
export const COXETER_VERSION='3.4';

/** Version, Git commit, branch and build time of this build, from `__COXETER_BUILD__` when the build injected it. */
export function softwareIdentity(){
 const injected=globalThis.__COXETER_BUILD__??{};
 return {version:injected.version??COXETER_VERSION,gitCommit:injected.gitCommit??null,gitBranch:injected.gitBranch??null,buildTime:injected.buildTime??null};
}

/** Hex SHA-256 of a string, or of the canonical JSON of any other value. */
export async function sha256(value){
 const bytes=new TextEncoder().encode(typeof value==='string'?value:canonicalJSON(value));
 const hash=await crypto.subtle.digest('SHA-256',bytes);
 return [...new Uint8Array(hash)].map(x=>x.toString(16).padStart(2,'0')).join('');
}
/** JSON with object keys sorted at every level and undefined values dropped, so equal data hash equally. */
export function canonicalJSON(value){return JSON.stringify(sortDeep(value));}
/**
 * Snapshot of the store as a reproducibility bundle: the models with their
 * hashes, community, constraints, analysis settings, omics, map, the solution
 * and replay tolerances, sealed with a SHA-256 hash of the whole payload.
 */
export async function createReproducibilityBundle(store){
 const models={}; const modelHashes={};
 for(const sp of store.species){
  const model=store.models.get(sp.file); if(!model)continue;
  const doc=model.toJSON(); models[sp.id]=doc; modelHashes[sp.id]=await sha256(doc);
 }
 const analysis=analysisProvenance(store);
 const solution=solutionPayload(store);
 const map=store.map?.toJSON?.()??null;
 const omics=omicsPayload(store.omics);
 const payload={
  format:REPRO_FORMAT,version:REPRO_VERSION,
  metadata:{createdAt:new Date().toISOString(),generator:'Coxeter Multicell Flux Visualizer',software:softwareIdentity(),coxeterVersion:softwareIdentity().version,
    userAgent:typeof navigator!=='undefined'?navigator.userAgent:null},
  models,modelHashes,
  community:{scenario:store.scenario?.id??null,name:store.scenario?.name??null,species:store.species.map(x=>({...x})),objective:store.objective,productTarget:store.productTarget??null,
    ...(store.customObjective?{customObjective:store.customObjective}:{})},
  constraints:{medium:{...store.medium},reactionOverrides:Object.fromEntries(store.overrides??[]),
   ...(store.geneKnockouts?.size?{geneKnockouts:[...store.geneKnockouts].sort()}:{})},
  analysis,omics,map,solution,
  ...(store.isZunigaStudy ? { study: {
    ...store.zunigaState(), manifest: store.zunigaStudy.manifest,
    sourceModels: store.zunigaStudy.sourceModels,
    activeModel: store.community.merged.toJSON(),
    redistribution: 'Personal reproducibility export. Original models were downloaded privately; review their authors’ terms before sharing.',
  } } : {}),
  replay:{tolerances:{absolute:1e-7,relative:1e-6}},
 };
 payload.integrity={algorithm:'SHA-256',payloadHash:await sha256({...payload,integrity:undefined})};
 return payload;
}
/**
 * Rechecks a bundle's payload hash and every model hash. Returns
 * {valid, payloadValid, modelChecks, payloadHash}. Throws for a file that is not
 * a bundle or comes from a newer bundle version.
 */
export async function verifyReproducibilityBundle(bundle){
 if(bundle?.format!==REPRO_FORMAT)throw new Error('Not a Coxeter Reproducibility Bundle.');
 if(bundle.version>REPRO_VERSION)throw new Error(`Bundle v${bundle.version} is not supported.`);
 const modelChecks={}; for(const [id,doc] of Object.entries(bundle.models??{}))modelChecks[id]=(await sha256(doc))===bundle.modelHashes?.[id];
 const copy={...bundle};delete copy.integrity;
 const payloadHash=await sha256(copy),payloadValid=payloadHash===bundle.integrity?.payloadHash;
 return {valid:payloadValid&&Object.values(modelChecks).every(Boolean),payloadValid,modelChecks,payloadHash};
}
/** The analysis method, solver and settings of the store's current run, for the bundle and the report. */
export function analysisProvenance(store){
 if(store.isZunigaStudy)return {method:'Stationary source-model FBA',solver:'GLPK',objective:'authors’ original objective',study:store.zunigaState(),parsimonious:false,loopless:false,fvaEnabled:false,sourceConstraints:'Exact author bounds and stoichiometry; documented Ec PPNDH knockout only.',limitations:'Stationary FBA comparison; not reproduction of dynamic culture trajectories or experimental growth measurements.'};
 let method='FBA'; if(store.omics?.gimmeEnabled)method='GIMME'; else if(store.loopless)method='loopless FBA'; else if(store.parsimonious)method='pFBA';
 return {method,solver:'Coxeter bounded-variable revised simplex / MILP',objective:store.objective,productTarget:store.productTarget??null,
  parsimonious:Boolean(store.parsimonious),pfbaBiomassPercent:store.pfbaBiomassPercent,loopless:Boolean(store.loopless),
  fvaEnabled:Boolean(store.fvaEnabled),gimme:store.omics?.gimmeEnabled?{condition:store.omics.conditionA,threshold:store.omics.gimmeResult?.threshold??store.omics.gimmeThreshold,
   objectiveFraction:store.omics.gimmeObjectiveFraction,inconsistencyScore:store.omics.gimmeResult?.inconsistencyScore??null}:null};
}
function solutionPayload(store){
 const s=store.solution;if(!s)return null;
 return {status:s.status,objectiveValue:finite(s.objectiveValue),iterations:s.iterations??null,
  fluxes:Object.fromEntries([...(s.fluxes??new Map())].map(([k,v])=>[k,finite(v)])),
  reducedCosts:Object.fromEntries([...(s.reducedCosts??new Map())].map(([k,v])=>[k,finite(v)])),
  scaledReducedCosts:Object.fromEntries([...(s.scaledReducedCosts??new Map())].map(([k,v])=>[k,finite(v)])),
  reducedCostScale:finite(s.reducedCostScale),
  shadowPrices:Object.fromEntries([...(s.shadowPrices??new Map())].map(([k,v])=>[k,finite(v)])),
  dualAnalysis:s.dualAnalysis??null,
  ...(s.zuniga ? { caseResult: s.zuniga } : {}),
  fva:store.fvaEnabled?Object.fromEntries(store.fvaRanges??[]):null,
  growth:Object.fromEntries(store.growth??[])};
}
function omicsPayload(o){
 if(!o?.dataset)return null; const values={};for(const [k,v] of o.dataset.values??[])values[k]=v;
 return {source:o.dataset.source,dataType:o.dataset.dataType,conditions:o.dataset.conditions,values,
  settings:{enabled:o.enabled,mode:o.mode,conditionA:o.conditionA,conditionB:o.conditionB,metric:o.metric,pseudocount:o.pseudocount,gimmeEnabled:o.gimmeEnabled,gimmeThreshold:o.gimmeThreshold,gimmeObjectiveFraction:o.gimmeObjectiveFraction},
  mappingReport:o.report??null};
}
function sortDeep(v){if(Array.isArray(v))return v.map(sortDeep);if(v&&typeof v==='object'&&!(v instanceof Date)){return Object.fromEntries(Object.keys(v).filter(k=>v[k]!==undefined).sort().map(k=>[k,sortDeep(v[k])]))}return v;}
function finite(v){return Number.isFinite(v)?v:null;}

/**
 * Compares a replayed solution with the archived one: status, objective, fluxes,
 * reduced costs (raw and scaled) and shadow prices. A value passes within the
 * `absolute` or the `relative` tolerance (relative to max(1, |a|, |b|)).
 * Returns {pass, checks, tolerances} plus the details of each comparison.
 */
export function compareReplayedSolution(archived,current,{absolute=1e-7,relative=1e-6}={}){
 const scalar=compareNumber(archived?.objectiveValue,current?.objectiveValue,{absolute,relative});
 const fluxes=compareNumericMaps(archived?.fluxes,current?.fluxes,{absolute,relative});
 const reducedCosts=compareNumericMaps(archived?.reducedCosts,current?.reducedCosts,{absolute,relative});
 const shadowPrices=compareNumericMaps(archived?.shadowPrices,current?.shadowPrices,{absolute,relative});
 const scaledReducedCosts=compareNumericMaps(archived?.scaledReducedCosts,current?.scaledReducedCosts,{absolute,relative});
 const statusMatch=(archived?.status??null)===(current?.status??null);
 const checks={status:statusMatch,objective:scalar.pass,fluxes:fluxes.pass,reducedCosts:reducedCosts.pass,scaledReducedCosts:scaledReducedCosts.pass,shadowPrices:shadowPrices.pass};
 return {pass:Object.values(checks).every(Boolean),checks,tolerances:{absolute,relative},objective:scalar,fluxes,reducedCosts,scaledReducedCosts,shadowPrices};
}
function compareNumericMaps(archived,current,opts){
 const a=archived??{}, c=current instanceof Map?Object.fromEntries(current):current??{};
 const keys=[...new Set([...Object.keys(a),...Object.keys(c)])].sort(); let maxAbs=0,maxRel=0,worst=null,missing=0;
 for(const key of keys){const av=a[key],cv=c[key];if(!Number.isFinite(av)||!Number.isFinite(cv)){if(av!==cv){missing++;worst=worst??key;}continue;}
  const abs=Math.abs(av-cv),rel=abs/Math.max(1,Math.abs(av),Math.abs(cv));if(abs>maxAbs){maxAbs=abs;worst=key;}maxRel=Math.max(maxRel,rel);
 }
 return {pass:missing===0&&(maxAbs<=opts.absolute||maxRel<=opts.relative),count:keys.length,missing,maxAbsError:maxAbs,maxRelativeError:maxRel,worstKey:worst};
}
function compareNumber(a,b,opts){if(!Number.isFinite(a)||!Number.isFinite(b))return {pass:a===b,archived:a,current:b,absError:null,relativeError:null};
 const abs=Math.abs(a-b),rel=abs/Math.max(1,Math.abs(a),Math.abs(b));return {pass:abs<=opts.absolute||rel<=opts.relative,archived:a,current:b,absError:abs,relativeError:rel};}

/** A Markdown methods and provenance report for a bundle, ending with a suggested Methods paragraph. */
export function methodsProvenanceReport(bundle){
 const m=bundle.metadata??{},sw=m.software??{version:m.coxeterVersion},a=bundle.analysis??{},c=bundle.community??{},cons=bundle.constraints??{};
 const species=c.species??[], hashes=bundle.modelHashes??{}, overrides=cons.reactionOverrides??{}, om=bundle.omics;
 const lines=[
 '# Coxeter Methods & Provenance Report','',
 '## Software',
 `Analysis generated with Coxeter Multicell Flux Visualizer version ${sw.version??'unknown'}${sw.gitCommit?` (Git commit ${sw.gitCommit})`:''}.`,
 sw.gitBranch?`Git branch: ${sw.gitBranch}.`:'',
 sw.buildTime?`Build timestamp: ${sw.buildTime}.`:'',
 `Bundle created: ${m.createdAt??'unknown'}.`,'',
 '## Metabolic models',
 `The community contained ${species.length} model(s).`,
 ...species.map(sp=>`- ${sp.id}: model ${sp.file??sp.model??'embedded'}; abundance ${sp.abundance??'not specified'}; SHA-256 ${hashes[sp.id]??'not available'}.`),'',
 '## Community and constraints',
 `Community objective: ${c.objective??a.objective??'not specified'}${c.productTarget?`; product target: ${c.productTarget}`:''}.`,
 `Medium contained ${Object.keys(cons.medium??{}).length} explicitly recorded component(s); ${Object.keys(overrides).length} reaction-bound override(s)/knockout(s) were applied.`,
 cons.geneKnockouts?.length?`${cons.geneKnockouts.length} gene knockout(s) (${cons.geneKnockouts.join(', ')}) closed every reaction whose gene-protein-reaction rule they make false.`:'','',
 ...(bundle.study ? [
 '## Published case adaptation',
 `Case: ${bundle.study.id}; scenario: ${bundle.study.scenarioId}; display: ${bundle.study.comparison}; fixed flux reference: ${bundle.study.fluxReference}.`,
 'The complete original monoculture and joint community source models are embedded in this personal bundle, together with the exact canonical active LP. Identifier namespacing preserves stoichiometry, source bounds and objective coefficients. The documented pheA deletion disables Ec PPNDH in mutant scenarios.',
 'The source objective and population coefficients are retained. S. elongatus is absent from monoculture calculations even when its faint cell remains visible for map comparison. Displaying fewer reactions or rearranging their geometry does not change the LP.',
 'These stationary FBA solutions are adaptations of the published model conditions. They do not reproduce dynamic/spatial culture trajectories or experimental growth measurements.',
 ...Object.entries(bundle.study.sourceHashes??{}).map(([file,hash])=>`- Original source ${file}: SHA-256 ${hash}.`),
 'Original source files were downloaded privately. Review the authors’ terms before redistributing a personal bundle.', '',
 ] : []),
 '## Flux analysis',
 `Method: ${a.method??'not specified'}. Solver: ${a.solver??'not specified'}.`,
 a.method==='pFBA'?`pFBA biomass target: ${a.pfbaBiomassPercent??100}% of the primary optimum.`:'',
 a.fvaEnabled?'Flux variability analysis (FVA) was enabled.':'',
 a.loopless?'Loopless constraints were enabled.':'',
 a.gimme?`GIMME used condition "${a.gimme.condition??'unspecified'}", expression threshold ${a.gimme.threshold??'auto'}, and required ${Math.round(100*(a.gimme.objectiveFraction??0.9))}% of the unconstrained metabolic objective.`:'',
 '',
 '## Omics',
 om?`A ${om.dataType??'omics'} dataset (${om.source??'source not recorded'}) was integrated. Conditions: ${(om.conditions??[]).join(', ')||'not recorded'}.`:'No omics dataset was integrated.',
 om?.mappingReport?`Mapping coverage: ${pct(om.mappingReport.geneCoverage)} of model genes and ${pct(om.mappingReport.reactionCoverage)} of GPR-associated reactions.`:'',
 '',
 '## Integrity and reproducibility',
 `Bundle integrity algorithm: ${bundle.integrity?.algorithm??'not recorded'}.`,
 `Bundle SHA-256: ${bundle.integrity?.payloadHash??'not recorded'}.`,
 'The bundle embeds the metabolic models, constraints, analysis settings, map state, omics dataset and archived numerical solution. Re-importing restores these inputs, reruns the archived analysis, and numerically compares the replayed result with the archived solution using recorded tolerances.',
 '',
 '## Numerical result',
 bundle.solution?`Archived solver status: ${bundle.solution.status}; objective value: ${bundle.solution.objectiveValue??'not available'}.`:'No numerical solution was archived.',
 bundle.solution?.dualAnalysis?.available?`LP dual analysis archived: ${Object.keys(bundle.solution.reducedCosts??{}).length} reduced costs and ${Object.keys(bundle.solution.shadowPrices??{}).length} shadow prices. Scaled reduced costs use RC/max|RC| with scale ${bundle.solution.reducedCostScale??'not available'}.`:'',
 '',
 '### Suggested Methods text',
 `Metabolic simulations were performed with Coxeter Multicell Flux Visualizer (version ${sw.version??'unknown'}${sw.gitCommit?`, commit ${sw.gitCommit}`:''}) using ${a.method??'flux balance analysis'}. Model inputs and analysis parameters were archived in a Coxeter Reproducibility Bundle and integrity-checked using SHA-256 hashes.`,
 ];
 return lines.filter((x,i)=>x!==''||lines[i-1]!=='').join('\n').trim()+'\n';
}
function pct(x){return Number.isFinite(x)?`${(100*x).toFixed(1)}%`:'not available';}
