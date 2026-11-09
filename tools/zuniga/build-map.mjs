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

const WIDTH = 2020, HEIGHT = 1327;
// The S. elongatus cell is taller than the E. coli one so that its photon
// uptake, chlorophyll pools and ATP synthase sit apart, above its pathway,
// instead of on top of one another in a corner, as in Sebastián Mendoza's
// hand-adjusted map, results/zuniga-2020-phea-rescue_map_manual.json. The
// E. coli cell is wide enough for its pathway's names to stay inside its
// cytosol, and the two cells together keep the map 2020 wide so that the
// figure does not shrink its text.
const SE = {x:60, y:170, w:875, h:1087}, EC = {x:1085, y:170, w:875, h:900};
const SE_PATH = {dx:15, dy:280};
const PATH = [
  ['DDPA', 'pep_c', '2dda7p_c'], ['DHQS', '2dda7p_c', '3dhq_c'],
  ['DHQTi', '3dhq_c', '3dhsk_c'], ['SHK3Dr', '3dhsk_c', 'skm_c'],
  ['SHKK', 'skm_c', 'skm5p_c'], ['PSCVT', 'skm5p_c', '3psme_c'],
  ['CHORS', '3psme_c', 'chor_c'], ['CHORM', 'chor_c', 'pphn_c'],
  ['PPNDH', 'pphn_c', 'phpyr_c'], ['PHETA1', 'phe__L_c', 'phpyr_c'],
];
// E. coli mirrors these. e4p stands above the start of DDPA rather than
// above pep, so that its branch comes down beside the name of pep instead of
// across it.
const POSITIONS = {
  pep_c: [150, 420], e4p_c: [200, 365], '2dda7p_c': [370, 420],
  '3dhq_c': [590, 420], '3dhsk_c': [790, 420], skm_c: [790, 630],
  skm5p_c: [590, 630], '3psme_c': [370, 630], chor_c: [150, 630],
  pphn_c: [150, 840], phpyr_c: [370, 840], phe__L_c: [590, 840],
};
// An arrow comes down into each of these from above, so its name is written
// below it.
const NAMED_BELOW = new Set(['skm_c', 'pphn_c']);
/**
 * How much farther from its reaction a cofactor stands than the editor's
 * default place. Branches here are drawn as wide as their flux (up to 15),
 * and an arrowhead is 2.4 times its line's width long, so at the default
 * distance a head filled the whole branch and looked cut off. At this
 * distance the heads also stand clear of the reaction's name.
 */
const SATELLITE_SPREAD = 1.6;
const owner = id => /^(Ec|Se)@/.exec(id)?.[1] ?? null;
const local = id => id.includes('@') ? id.slice(id.indexOf('@') + 1) : id;

function cell(id) {
  const r = id==='Se' ? SE : EC, {x,y} = r;
  const compartments = [
    { id: 'p', label: 'Periplasm (p)', parentId: null, depth: 0, membrane: 'bilayer', rect: {...r} },
    id==='Se'
      ? { id: 'c', label: 'Cytosol (c)', parentId: 'p', depth: 1, membrane: 'bilayer', rect: {x:x+63,y:y+63,w:755,h:959}, labelPosition: {x:0.71,y:1} }
      : { id: 'c', label: 'Cytosol (c)', parentId: 'p', depth: 1, membrane: 'bilayer', rect: {x:x+110,y:y+35,w:745,h:830} },
  ];
  // A name plate never grows into a nested compartment, so k keeps a band
  // above u for its plate; with u only 22 below it, the plate shrank to a
  // third of the others. u's plate sits on its lower edge, well left of the
  // ATP synthase, where it covers neither k's plate, the lumen proton nor the
  // ADP and phosphate rising to the synthase.
  if (id === 'Se') compartments.push(
    { id: 'k', label: 'Thylakoid membrane (k)', parentId: 'c', depth: 2, membrane: 'bilayer', rect: {x:x+206,y:y+199,w:500,h:127} },
    { id: 'u', label: 'Thylakoid lumen (u)', parentId: 'k', depth: 3, membrane: 'bilayer', rect: {x:x+251,y:y+246,w:405,h:60}, labelPosition: {x:0.25,y:1} },
  );
  return { id, name: id==='Se' ? 'S. elongatus cscB+' : 'E. coli K-12',
    color: id==='Se' ? '#047857' : '#b45309', outerCompartment:'p',
    rect:{...r}, compartments };
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
    environment:{id:'s',label:'Shared medium (original smp)',rect:{x:20,y:110,w:WIDTH-40,h:HEIGHT-145}},
    cells:[cell('Se'),cell('Ec')], medium:{nodes:[],exchanges:[]},
  });
  const primary = new Map();
  const position = (species, [x,y]) => species==='Ec' ? {x:EC.x+967-x,y} : {x:x+SE_PATH.dx,y:y+SE_PATH.dy};
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
    for (const [mid,point] of Object.entries(POSITIONS)) {
      const options = NAMED_BELOW.has(mid) ? {labelAngle:Math.PI/2} : {};
      node(`${species}@${mid}`,position(species,point),options);
    }
  }
  // Place each extracellular pool only once: both cells truly share this node.
  // It stands halfway across the medium between the two cells.
  const sharedPhe = [...sourceMetabolites.keys()].find(id=>id==='M@phe__L');
  if (!sharedPhe) throw new Error('The original model has no shared phenylalanine pool.');
  const mediumX = (SE.x+SE.w+EC.x)/2;
  node(sharedPhe,{x:mediumX,y:960},{labelAngle:-Math.PI/2});
  node('Ec@phe__L_p',{x:EC.x+55,y:960},{labelAngle:Math.PI/2});

  // `dir` overrides the axis a reaction takes from its two metabolites, for a
  // transporter that has to cross its membrane square on although they are not
  // in line with it. `labelOffset` moves the reaction's name off its default
  // place, where it would cross a line or a membrane.
  function reaction(rid, fromMid, toMid, {center=null, membrane=null, linear=true, dir:axisDir=null, labelOffset=null}={}) {
    const source = sourceReactions.get(rid);
    if (!source) throw new Error(`Curated map references an absent source reaction: ${rid}`);
    if (map.reactions.has(rid)) return;
    const from = primary.get(fromMid), to = primary.get(toMid);
    if (!from || !to || !(source.metabolites[fromMid]<0) || !(source.metabolites[toMid]>0)) {
      throw new Error(`Curated pathway direction does not match source stoichiometry: ${rid}`);
    }
    const dx=to.x-from.x,dy=to.y-from.y,length=Math.hypot(dx,dy),dir=axisDir??{x:dx/length,y:dy/length};
    const p=center??{x:(from.x+to.x)/2,y:(from.y+to.y)/2};
    const a1={x:p.x-dir.x*13,y:p.y-dir.y*13},a2={x:p.x+dir.x*13,y:p.y+dir.y*13};
    const rxn={id:rid,localId:local(rid),name:source.name??local(rid),cellId:owner(rid),kind:membrane?'transfer':'reaction',
      x:p.x,y:p.y,axis:{dir,a1,a2},onMembrane:!!membrane,membrane,
      boundaryId:membrane?boundaryKey(owner(rid),membrane):null,
      compartments:[...new Set(Object.keys(source.metabolites).map(mid=>sourceMetabolites.get(mid)?.compartment).filter(Boolean))],
      bounds:{lower:source.lower_bound,upper:source.upper_bound},
      ...(linear&&!membrane?{linear:{from:from.id,to:to.id,t:0.5}}:{}),...(labelOffset?{labelOffset}:{})};
    map.reactions.set(rid,rxn);
    map.segments.push({reactionId:rid,kind:'axis',from:a1,to:a2,arrow:false});
    const placed={substrate:0,product:0};
    for (const [mid,coefficient] of Object.entries(source.metabolites)) {
      const side=coefficient<0?'substrate':'product';
      let endpoint=primary.get(mid);
      if (!endpoint) {
        const at=satellitePlace(rxn,coefficient<0?-1:1,placed[side]++);
        let offset={x:at.x-rxn.x,y:at.y-rxn.y};
        // E. coli is drawn as the mirror image of S. elongatus, so its
        // cofactors are mirrored across their reaction's axis as well.
        if (owner(rid)==='Ec') {
          const along=offset.x*dir.x+offset.y*dir.y;
          offset={x:2*along*dir.x-offset.x,y:2*along*dir.y-offset.y};
        }
        const spread={x:rxn.x+offset.x*SATELLITE_SPREAD,y:rxn.y+offset.y*SATELLITE_SPREAD};
        endpoint=node(mid,spread,{secondary:true,reactionId:rid});
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
  // The S. elongatus pathway runs close to the right of its cytosol, so the
  // name of SHK3Dr stands inside the pathway's loop, left of its bar.
  map.reactions.get('Se@SHK3Dr').labelOffset={x:-17,y:0};
  // pep comes straight down into PSCVT. The general branch shape runs on past
  // the reaction and back along its bar, across the phosphate it releases.
  for (const species of ['Se','Ec']) {
    const segment=map.segments.find(seg=>seg.reactionId===`${species}@PSCVT`&&seg.nodeId===`met:${species}@pep_c`);
    segment.b1={x:segment.from.x,y:segment.from.y+100};
    segment.b2={x:segment.to.x,y:segment.to.y-80};
  }
  // The transporters' names stand clear of the line to the shared pool and of
  // the line on into the cytosol: below their markers, except that of PHEtex,
  // which stands above its arrow inside the periplasm.
  reaction('Se@PHELtex',sharedPhe,'Se@phe__L_c',{center:{x:SE.x+SE.w,y:1030},membrane:'p',labelOffset:{x:8,y:18}});
  reaction('Ec@PHEtex',sharedPhe,'Ec@phe__L_p',{center:{x:EC.x,y:960},membrane:'p',labelOffset:{x:8,y:-20}});
  reaction('Ec@PHEt2rpp','Ec@phe__L_p','Ec@phe__L_c',{center:{x:EC.x+110,y:960},membrane:'c',labelOffset:{x:16,y:12}});

  // Explicit source IDs keep phototrophic context traceable. This is a small
  // visible excerpt, not an invented lumped photosynthesis reaction. The
  // excited chlorophyll is named to its left, away from the arrows coming in
  // from the right.
  const photon='M@photon690', excited='Se@chla_qy1*_c';
  if (sourceReactions.has('Se@PHOA690um')&&sourceMetabolites.has(photon)&&sourceMetabolites.has(excited)) {
    node(photon,{x:mediumX,y:230}); node(excited,{x:544,y:342},{labelAngle:Math.PI});
    reaction('Se@PHOA690um',photon,excited,{center:{x:SE.x+SE.w,y:286},membrane:'p',linear:false,labelOffset:{x:8,y:18}});
  }
  if (sourceReactions.has('Se@ATPSum')) {
    // The lumen proton is named to its right: above it, the name would run
    // into k's plate, and the ATP synthase is below. ATP stands straight
    // below the synthase and is named to its right, beside the arrow into it.
    // The synthase's own name runs along k below the lumen, right of it.
    node('Se@h_u',{x:618,y:435},{labelAngle:0}); node('Se@atp_c',{x:616,y:626},{labelAngle:0});
    reaction('Se@ATPSum','Se@h_u','Se@atp_c',{center:{x:616,y:476},membrane:'u',linear:false,dir:{x:0,y:1},labelOffset:{x:30,y:9}});
  }
  // Photochemical cofactors must remain in their source compartment, rather
  // than accidentally being placed inside the lumen by a generic satellite rule.
  // The ATP synthase's branches are as wide as the widest flux, so its
  // cofactors stand well apart: ADP and phosphate come in from the left, and
  // water and the cytosolic proton leave to the right, far enough out for
  // their arrowheads to be drawn in full.
  // In the corners where SHK3Dr meets DHQTi and SHKK, their cofactors would
  // stand on or against one another, so SHK3Dr's proton and NADP sit closer
  // to its bar.
  // The periplasmic proton of PHEt2rpp comes in from above: from below, as
  // the generic rule puts it, it and its branch cross the name of phe__L_p.
  const pinned={
    'cur:Se@PHOA690um:Se@cholphya_c':{x:820,y:262},
    'cur:Se@PHOA690um:Se@chla_qy2*_c':{x:554,y:260},
    'cur:Se@ATPSum:Se@pi_c':{x:556,y:521},
    'cur:Se@ATPSum:Se@adp_c':{x:511,y:521},
    'cur:Se@ATPSum:Se@h2o_c':{x:696,y:526},
    'cur:Se@ATPSum:Se@h_c':{x:696,y:581},
    'cur:Se@SHK3Dr:Se@h_c':{x:763,y:767},
    'cur:Se@SHK3Dr:Se@nadp_c':{x:763,y:840},
    'cur:Ec@SHK3Dr:Ec@h_c':{x:EC.x+219,y:487},
    'cur:Ec@SHK3Dr:Ec@nadp_c':{x:EC.x+219,y:560},
    'cur:Ec@PHEt2rpp:Ec@h_p':{x:EC.x+97,y:912},
  };
  for(const [id,point] of Object.entries(pinned)){
    const visual=map.nodes.get(id);if(!visual)continue;
    Object.assign(visual,point);
    for(const segment of map.segments.filter(seg=>seg.nodeId===id)){
      if(segment.side==='substrate')segment.from={...point};else segment.to={...point};
      shapeBranch(segment,map.reactions.get(segment.reactionId));
    }
  }
  // ADP and phosphate are cytosolic but join the ATP synthase at its lumen
  // end. The general branch shape would loop up past that end into the lumen,
  // so these rise from the cytosol and turn into the synthase from the side.
  for (const id of ['cur:Se@ATPSum:Se@pi_c','cur:Se@ATPSum:Se@adp_c']) {
    const segment=map.segments.find(seg=>seg.nodeId===id);
    if (!segment) continue;
    const toward=Math.sign(segment.from.x-segment.to.x);
    segment.b1={x:segment.from.x,y:segment.from.y-30};
    segment.b2={x:segment.to.x+toward*40,y:segment.to.y};
  }
  const annotations=[
    ['title','Zuñiga et al. (2020) · ΔpheA phenylalanine rescue',65,55,28],
    ['scope','Curated pathway map · the simulation uses the complete original models',65,87,17],
    ['Se-path','Phototroph · phenylalanine biosynthesis',139,590,16],
    ['Ec-path','Heterotroph · phenylalanine biosynthesis',EC.x+130,350,16],
    ['transfer','Phenylalanine transfer',SE.x+SE.w+15,1110,17],
    ['ko-gene','pheA / b2599 → PPNDH',EC.x+130,800,16],
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
