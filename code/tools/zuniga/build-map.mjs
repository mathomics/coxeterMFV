#!/usr/bin/env node
/**
 * Build the curated map of the Zuñiga et al. (2020) ΔpheA rescue community. It
 * is a drawing only and is never used as a simulation model.
 *
 *   node tools/zuniga/build-map.mjs --sources=/private/original-mat-directory
 *   node tools/zuniga/build-map.mjs --study-pack=/private/calculated-directory
 *
 * --study-pack is a folder holding the full source COBRA JSON
 * (--community-source names that file directly), and --output overrides the
 * default models/published/zuniga-2020-phea-rescue/map.json. Only geometry and
 * the selected identifiers are written, so no genome-scale S is redistributed.
 */
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import { dirname, resolve, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { canonicalZunigaModel, convertZunigaMatModel } from '../../src/communities/zuniga.js';
import { readMat5 } from '../../src/import/mat5.js';
import { EditableMap } from '../../src/map/editable-map.js';
import { shapeBranch, satellitePlace } from '../../src/layout/reaction-edit.js';
import { boundaryKey } from '../../src/map/boundaries.js';

const WIDTH = 1800, HEIGHT = 1100;
const PATH = [
  ['DDPA', 'pep_c', '2dda7p_c'], ['DHQS', '2dda7p_c', '3dhq_c'],
  ['DHQTi', '3dhq_c', '3dhsk_c'], ['SHK3Dr', '3dhsk_c', 'skm_c'],
  ['SHKK', 'skm_c', 'skm5p_c'], ['PSCVT', 'skm5p_c', '3psme_c'],
  ['CHORS', '3psme_c', 'chor_c'], ['CHORM', 'chor_c', 'pphn_c'],
  ['PPNDH', 'pphn_c', 'phpyr_c'], ['PHETA1', 'phe__L_c', 'phpyr_c'],
];
const POSITIONS = {
  pep_c: [150, 420], e4p_c: [150, 390], '2dda7p_c': [350, 420],
  '3dhq_c': [550, 420], '3dhsk_c': [730, 420], skm_c: [730, 610],
  skm5p_c: [550, 610], '3psme_c': [350, 610], chor_c: [150, 610],
  pphn_c: [150, 800], phpyr_c: [350, 800], phe__L_c: [550, 800],
};
const owner = id => /^(Ec|Se)@/.exec(id)?.[1] ?? null;
const local = id => id.includes('@') ? id.slice(id.indexOf('@') + 1) : id;

function cell(id, x) {
  const compartments = [
    { id: 'p', label: 'Periplasm (p)', parentId: null, depth: 0, membrane: 'bilayer', rect: {x,y:170,w:740,h:860} },
    { id: 'c', label: 'Cytosol (c)', parentId: 'p', depth: 1, membrane: 'bilayer', rect: {x:x+(id==='Ec'?90:30),y:205,w:id==='Ec'?620:680,h:790} },
  ];
  if (id === 'Se') compartments.push(
    { id: 'k', label: 'Compartment k (source ID)', parentId: 'c', depth: 2, membrane: 'bilayer', rect: {x:x+190,y:242,w:500,h:95} },
    { id: 'u', label: 'Compartment u (source ID)', parentId: 'k', depth: 3, membrane: 'bilayer', rect: {x:x+235,y:264,w:405,h:60} },
  );
  return { id, name: id==='Se' ? 'S. elongatus cscB+' : 'E. coli K-12',
    color: id==='Se' ? '#047857' : '#b45309', outerCompartment:'p',
    rect:{x,y:170,w:740,h:860}, compartments };
}

/**
 * Build the same editable two-cell geometry for every experimental scenario.
 * `jointSource` is the joint community document in the source namespace, or
 * already canonical when `canonical` is true. Returns an EditableMap and
 * throws if the curated pathway names a reaction or metabolite the source lacks.
 */
export function buildZunigaMap(jointSource, { canonical = false } = {}) {
  const doc = canonical ? jointSource : canonicalZunigaModel(jointSource);
  const sourceReactions = new Map(doc.reactions.map(r => [r.id,r]));
  const sourceMetabolites = new Map(doc.metabolites.map(m => [m.id,m]));
  const map = new EditableMap({ canvas:{width:WIDTH,height:HEIGHT},
    environment:{id:'s',label:'Shared medium (original smp)',rect:{x:20,y:110,w:1760,h:955}},
    cells:[cell('Se',60),cell('Ec',1000)], medium:{nodes:[],exchanges:[]},
  });
  const primary = new Map();
  const position = (species, [x,y]) => ({x:species==='Ec' ? WIDTH-x+40 : x,y});
  function node(mid, point, {secondary=false, reactionId=null, labelAngle}={}) {
    const met = sourceMetabolites.get(mid);
    if (!met) throw new Error(`Curated map references an absent source metabolite: ${mid}`);
    const id = secondary ? `cur:${reactionId}:${mid}` : `met:${mid}`;
    if (map.nodes.has(id)) return map.nodes.get(id);
    const originalId = met.notes?.source_metabolite_id ?? local(mid);
    const n = {id,mergedId:mid,localId:local(mid),base:originalId,name:met.name ?? originalId,
      compartment:met.compartment,cellId:owner(mid),kind:mid.startsWith('M@')?'medium':secondary?'currency':'metabolite',
      x:point.x,y:point.y,r:secondary?7:13,primary:!secondary,visualRole:secondary?'secondary':'primary',
      ...(secondary?{reactionId}:{}),...(labelAngle===undefined?{}:{labelAngle})};
    map.nodes.set(id,n);
    if (!secondary) primary.set(mid,n);
    if (n.kind==='medium') map.medium.nodes.push(id);
    return n;
  }
  for (const species of ['Se','Ec']) {
    for (const [mid,point] of Object.entries(POSITIONS)) node(`${species}@${mid}`,position(species,point));
  }
  // Place each extracellular pool only once: both cells truly share this node.
  const sharedPhe = [...sourceMetabolites.keys()].find(id=>id==='M@phe__L');
  if (!sharedPhe) throw new Error('The original model has no shared phenylalanine pool.');
  node(sharedPhe,{x:900,y:920},{labelAngle:-Math.PI/2});
  node('Ec@phe__L_p',{x:1045,y:920},{labelAngle:Math.PI/2});

  function reaction(rid, fromMid, toMid, {center=null, membrane=null, linear=true}={}) {
    const source = sourceReactions.get(rid);
    if (!source) throw new Error(`Curated map references an absent source reaction: ${rid}`);
    if (map.reactions.has(rid)) return;
    const from = primary.get(fromMid), to = primary.get(toMid);
    if (!from || !to || !(source.metabolites[fromMid]<0) || !(source.metabolites[toMid]>0)) {
      throw new Error(`Curated pathway direction does not match source stoichiometry: ${rid}`);
    }
    const dx=to.x-from.x,dy=to.y-from.y,length=Math.hypot(dx,dy),dir={x:dx/length,y:dy/length};
    const p=center??{x:(from.x+to.x)/2,y:(from.y+to.y)/2};
    const a1={x:p.x-dir.x*13,y:p.y-dir.y*13},a2={x:p.x+dir.x*13,y:p.y+dir.y*13};
    const rxn={id:rid,localId:local(rid),name:source.name??local(rid),cellId:owner(rid),kind:membrane?'transfer':'reaction',
      x:p.x,y:p.y,axis:{dir,a1,a2},onMembrane:!!membrane,membrane,
      boundaryId:membrane?boundaryKey(owner(rid),membrane):null,
      compartments:[...new Set(Object.keys(source.metabolites).map(mid=>sourceMetabolites.get(mid)?.compartment).filter(Boolean))],
      bounds:{lower:source.lower_bound,upper:source.upper_bound},
      ...(linear&&!membrane?{linear:{from:from.id,to:to.id,t:0.5}}:{})};
    map.reactions.set(rid,rxn);
    map.segments.push({reactionId:rid,kind:'axis',from:a1,to:a2,arrow:false});
    const placed={substrate:0,product:0};
    for (const [mid,coefficient] of Object.entries(source.metabolites)) {
      const side=coefficient<0?'substrate':'product';
      let endpoint=primary.get(mid);
      if (!endpoint) {
        const at=satellitePlace(rxn,coefficient<0?-1:1,placed[side]++);
        endpoint=node(mid,at,{secondary:true,reactionId:rid});
      }
      const segment={reactionId:rid,nodeId:endpoint.id,kind:endpoint.primary?'primary':'currency',coefficient,side,
        from:side==='substrate'?{x:endpoint.x,y:endpoint.y}:a2,
        to:side==='product'?{x:endpoint.x,y:endpoint.y}:a1,backboneAnchor:true,arrow:side==='product'};
      shapeBranch(segment,rxn);
      map.segments.push(segment);
    }
  }
  for (const species of ['Se','Ec']) {
    for (const [rid,from,to] of PATH) reaction(`${species}@${rid}`,`${species}@${from}`,`${species}@${to}`);
  }
  reaction('Se@PHELtex',sharedPhe,'Se@phe__L_c',{center:{x:800,y:920},membrane:'p'});
  reaction('Ec@PHEtex',sharedPhe,'Ec@phe__L_p',{center:{x:1000,y:920},membrane:'p'});
  reaction('Ec@PHEt2rpp','Ec@phe__L_p','Ec@phe__L_c',{center:{x:1090,y:920},membrane:'c'});

  // Explicit source IDs keep phototrophic context traceable. This is a small
  // visible excerpt, not an invented lumped photosynthesis reaction.
  const photon='M@photon690', excited='Se@chla_qy1*_c';
  if (sourceReactions.has('Se@PHOA690um')&&sourceMetabolites.has(photon)&&sourceMetabolites.has(excited)) {
    node(photon,{x:900,y:230}); node(excited,{x:170,y:250});
    reaction('Se@PHOA690um',photon,excited,{center:{x:800,y:230},membrane:'p',linear:false});
  }
  if (sourceReactions.has('Se@ATPSum')) {
    node('Se@h_u',{x:600,y:290}); node('Se@atp_c',{x:600,y:385});
    reaction('Se@ATPSum','Se@h_u','Se@atp_c',{center:{x:600,y:324},membrane:'u',linear:false});
  }
  // Photochemical cofactors must remain in their source compartment, rather
  // than accidentally being placed inside the lumen by a generic satellite rule.
  const photoPositions={
    'cur:Se@PHOA690um:Se@cholphya_c':{x:170,y:220},
    'cur:Se@PHOA690um:Se@chla_qy2*_c':{x:170,y:240},
    'cur:Se@ATPSum:Se@pi_c':{x:555,y:365},
    'cur:Se@ATPSum:Se@adp_c':{x:645,y:365},
    'cur:Se@ATPSum:Se@h2o_c':{x:555,y:395},
    'cur:Se@ATPSum:Se@h_c':{x:645,y:395},
  };
  for(const [id,point] of Object.entries(photoPositions)){
    const visual=map.nodes.get(id);if(!visual)continue;
    Object.assign(visual,point);
    for(const segment of map.segments.filter(seg=>seg.nodeId===id)){
      if(segment.side==='substrate')segment.from={...point};else segment.to={...point};
      shapeBranch(segment,map.reactions.get(segment.reactionId));
    }
  }
  const annotations=[
    ['title','Zuñiga et al. (2020) · ΔpheA phenylalanine rescue',65,55,28],
    ['scope','Curated pathway map · the simulation uses the complete original models',65,87,17],
    ['Se-path','Phototroph · phenylalanine biosynthesis',120,350,16],
    ['Ec-path','Heterotroph · phenylalanine biosynthesis',1120,350,16],
    ['transfer','Phenylalanine transfer',795,1048,17],
    ['ko-gene','pheA / b2599 → PPNDH',1120,760,16],
  ];
  for (const [id,text,x,y,fontSize] of annotations) map.texts.set(id,{id,text,x,y,fontSize});
  return map;
}

async function main() {
  const args=Object.fromEntries(process.argv.slice(2).map(arg=>{
    const split=arg.indexOf('=');return [arg.slice(2,split<0?undefined:split),split<0?'true':arg.slice(split+1)];
  }));
  let joint;
  if(args.sources) {
    const raw=await readMat5(await readFile(join(resolve(args.sources),'CM-modelsKO.mat')));
    joint=convertZunigaMatModel(raw.modelCo2[1],{id:'zuniga-original-knockout-analysis-community'});
  } else if(args['community-source']||args['study-pack']) {
    const source=args['community-source']??join(resolve(args['study-pack']),'mutant-coculture-model.json');
    joint=JSON.parse(await readFile(source,'utf8'));
  } else throw new Error('Provide --sources=<original MAT directory> or --study-pack=<private calculated directory>.');
  const map=buildZunigaMap(joint);
  const output=resolve(args.output??new URL('../../models/published/zuniga-2020-phea-rescue/map.json',import.meta.url).pathname.replace(/^\/(\w:)/,'$1'));
  await mkdir(dirname(output),{recursive:true});
  await writeFile(output,JSON.stringify({...map.toJSON(),provenance:{source:'CM-modelsKO.mat:modelCo2{2}',
    purpose:'Editable curated geometry; not a simulation model',allReactionIdsVerifiedInOriginalSource:true,
    selectedReactions:map.reactions.size,displayLabels:'original model metabolite IDs',layout:'identical for all three scenarios'}},null,2)+'\n');
  console.log(`${output}: ${map.reactions.size} real reactions, ${map.nodes.size} visual metabolite instances.`);
}
if(process.argv[1]&&resolve(process.argv[1])===fileURLToPath(import.meta.url)) await main();
