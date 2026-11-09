import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { EditableMap } from '../src/map/editable-map.js';
import { renderMap } from '../src/render/mapview.js';
import { isSecondaryNode, metaboliteLabelPosition } from '../src/map/metabolite-label.js';

const scale={reference:1,width:v=>Math.abs(v)>1?12:2,color:v=>v<0?'#2563eb':'#ef4444'};
async function fixture() {
  return EditableMap.fromJSON(JSON.parse(await readFile(new URL('../models/published/zuniga-2020-phea-rescue/map.json',import.meta.url),'utf8')));
}
const heads=(svg,rid)=>[...svg.matchAll(/<polygon class="emc-arrow"[^>]*data-rxn="([^"]+)"[^>]*data-node="([^"]+)"/g)]
  .filter(match=>match[1]===rid).map(match=>match[2]).sort();
const paths=(svg,rid)=>[...svg.matchAll(/<path class="emc-seg[^>]+>/g)].map(match=>match[0]).filter(tag=>tag.includes(`data-rxn="${rid}"`));

test('study map contains both cells, original compartment/label IDs and the complete visible phenylalanine path',async()=>{
  const map=await fixture();
  assert.deepEqual(map.cells.map(cell=>cell.id),['Se','Ec']);
  assert.ok(map.cells.find(cell=>cell.id==='Se').compartments.some(comp=>comp.id==='u'));
  for(const species of ['Se','Ec']) for(const rid of ['DDPA','DHQS','DHQTi','SHK3Dr','SHKK','PSCVT','CHORS','CHORM','PPNDH','PHETA1']) {
    assert.ok(map.reactions.has(`${species}@${rid}`));
  }
  for(const rid of ['Se@PHELtex','Ec@PHEtex','Ec@PHEt2rpp'])assert.ok(map.reactions.has(rid));
  const phe=[...map.nodes.values()].filter(node=>node.mergedId==='M@phe__L');
  assert.equal(phe.length,1,'both cells connect to one actual medium pool');
  const sourceNodes=[...map.nodes.values()];
  assert.ok(sourceNodes.some(node=>node.base==='phe__L_[smp]'));
  assert.ok(sourceNodes.some(node=>node.base==='photon690_[smp]'));
  assert.ok(sourceNodes.some(node=>node.base==='h_u_A'));
  assert.ok(map.reactions.size<50,'the drawing remains a readable excerpt');
  assert.ok(map.width>=1700&&map.height>=1000);
});

test('flux width is opt-in and all branches of a reaction share the same width',async()=>{
  const map=await fixture(),rid='Ec@CHORM';
  for(const reactionWidthMode of [undefined,'flux']){
    const low=renderMap(map,{scale,fluxes:new Map([[rid,.1]]),reactionWidthMode});
    const high=renderMap(map,{scale,fluxes:new Map([[rid,10]]),reactionWidthMode});
    const widths=svg=>paths(svg,rid).map(tag=>Number(tag.match(/stroke-width="([^"]+)"/)[1]));
    assert.ok(widths(low).length>=3);
    assert.ok(widths(low).every(width=>width===(reactionWidthMode==='flux'?2:7.5)));
    assert.ok(widths(high).every(width=>width===(reactionWidthMode==='flux'?12:7.5)));
  }
});

test('signed difference colours/widths do not reverse arrows that follow positive actual flux',async()=>{
  const map=await fixture(),rid='Ec@CHORM';
  const svg=renderMap(map,{scale,labels:'all',fluxes:new Map([[rid,-4]]),solvedFluxes:new Map([[rid,2]]),reactionWidthMode:'flux'});
  const products=map.segments.filter(seg=>seg.reactionId===rid&&seg.side==='product').map(seg=>seg.nodeId).sort();
  assert.deepEqual(heads(svg,rid),products);
  assert.ok(paths(svg,rid).every(tag=>tag.includes('stroke="#2563eb"')&&tag.includes('stroke-width="12"')));
  const negativeActual=renderMap(map,{scale,fluxes:new Map([[rid,4]]),solvedFluxes:new Map([[rid,-2]])});
  const substrates=map.segments.filter(seg=>seg.reactionId===rid&&seg.side==='substrate').map(seg=>seg.nodeId).sort();
  assert.deepEqual(heads(negativeActual,rid),substrates);
  const zeroActual=renderMap(map,{scale,fluxes:new Map([[rid,4]]),solvedFluxes:new Map([[rid,0]])});
  assert.deepEqual(heads(zeroActual,rid),[]);
});

test('knockout is visible as magenta dashed branches and a cross, with no flux arrow',async()=>{
  const map=await fixture(),rid='Ec@PPNDH';
  const svg=renderMap(map,{scale,labels:'auto',fluxes:new Map([[rid,0]]),disabledReactionIds:new Set([rid])});
  assert.ok(paths(svg,rid).length>=3);
  assert.ok(paths(svg,rid).every(tag=>tag.includes('is-disabled-reaction')&&tag.includes('stroke="#db2777"')&&tag.includes('stroke-dasharray="8 5"')));
  assert.match(svg,/data-disabled-reaction="Ec@PPNDH"/);
  assert.match(svg,/>PPNDH \[KO\]  0<\/text>/);
  assert.deepEqual(heads(svg,rid),[]);
  assert.doesNotMatch(paths(svg,rid).join(''),/is-active/);
});

test('the small but essential phenylalanine transfer remains labelled in auto mode',async()=>{
  const map=await fixture(),rid='Ec@PHEtex';
  const actual=new Map([[rid,.014371756]]);
  const svg=renderMap(map,{scale:{...scale,reference:.1},fluxes:actual,labels:'auto',emphasizedReactionIds:new Set([rid])});
  assert.match(svg,/>PHEtex  0\.014<\/text>/);
  const zero=renderMap(map,{scale:{...scale,reference:.1},fluxes:new Map(),labels:'auto',emphasizedReactionIds:new Set([rid])});
  assert.doesNotMatch(zero,/>PHEtex  /);
});

test('monoculture ghosts keep geometry/editability but are not shown as simulated growing cells',async()=>{
  const map=await fixture(),before=JSON.stringify(map.toJSON());
  const rid='Se@CHORM';
  const svg=renderMap(map,{scale,labels:'all',fluxes:new Map([[rid,100]]),growth:new Map([['Se',.1234],['Ec',.2]]),inactiveSpeciesIds:new Set(['Se'])});
  assert.match(svg,/<g class="emc-cell is-inactive-species" data-cell="Se" opacity="0.32">/);
  assert.match(svg,/S\. elongatus cscB\+ · inactive context/);
  assert.doesNotMatch(svg,/S\. elongatus cscB\+[^<]*µ/);
  assert.match(svg,/E\. coli K-12[^<]*µ = 0\.2/);
  assert.ok(paths(svg,rid).every(tag=>tag.includes('is-inactive-species')&&tag.includes('opacity="0.22"')));
  assert.deepEqual(heads(svg,rid),[]);
  assert.match(svg,/data-metabolite-label="met:Se@chor_c"[^>]*opacity="0.28"/);
  assert.equal(JSON.stringify(map.toJSON()),before,'renderer never edits shared geometry');
  const node=map.nodes.get('met:Se@chor_c');
  const previous=node.x; map.moveNode(node.id,node.x+10,node.y);
  assert.equal(node.x,previous+10,'a context cell still uses ordinary editing tools');
});

// Where a metabolite's name is drawn, as a box. The width is an estimate, 0.56 em
// a character, which the label fonts stay within; the sizes are map.css's.
function nameBox(node) {
  const size=node.kind==='medium'?15:isSecondaryNode(node)?9.5:12.5,w=String(node.base).length*size*0.56;
  const {x,y,anchor,baseline}=metaboliteLabelPosition(node);
  const left=anchor==='start'?x:anchor==='end'?x-w:x-w/2;
  const top=baseline==='auto'?y-size*0.8:baseline==='hanging'?y:y-size/2;
  return {left,top,right:left+w,bottom:top+size};
}
const overlap=(a,b)=>a.left<b.right&&b.left<a.right&&a.top<b.bottom&&b.top<a.bottom;
const onCircle=(box,node)=>Math.hypot(Math.max(box.left,Math.min(node.x,box.right))-node.x,
  Math.max(box.top,Math.min(node.y,box.bottom))-node.y)<node.r;

test('no metabolite name is written over another name or another metabolite',async()=>{
  const map=await fixture(),nodes=[...map.nodes.values()],boxes=new Map(nodes.map(node=>[node.id,nameBox(node)]));
  const clashes=[];
  for(const [i,a] of nodes.entries()) for(const b of nodes.slice(i+1)) {
    if(overlap(boxes.get(a.id),boxes.get(b.id)))clashes.push(`${a.id} / ${b.id}`);
  }
  for(const a of nodes) for(const b of nodes) if(a!==b&&onCircle(boxes.get(a.id),b))clashes.push(`${a.id} on ${b.id}`);
  assert.deepEqual(clashes,[]);
});

test('every compartment name plate reaches full size and, at it, covers no metabolite name',async()=>{
  const map=await fixture(),svg=renderMap(map,{scale,labels:'all'});
  const plates=[...svg.matchAll(/data-compartment-label="([^"]+)" transform="translate\(([^ ]+) ([^)]+)\)">\s*<g class="emc-frame-label" style="--fit:([^"]+)">\s*<rect x="([^"]+)" y="([^"]+)" width="([^"]+)" height="([^"]+)"/g)]
    .map(([,id,...v])=>{
      const [tx,ty,fit,x,y,w,h]=v.map(Number),s=Math.min(fit,1);
      return {id,fit,box:{left:tx+x*s,top:ty+y*s,right:tx+(x+w)*s,bottom:ty+(y+h)*s}};
    });
  assert.deepEqual(plates.map(plate=>plate.id),['s','p','c','k','u','p','c']);
  // k and u are nested closely in S. elongatus: k's plate used to shrink to a third.
  for(const plate of plates)assert.ok(plate.fit>=1,`${plate.id} plate fits at ${plate.fit}`);
  const covered=plates.flatMap(plate=>[...map.nodes.values()].filter(node=>overlap(plate.box,nameBox(node))).map(node=>`${plate.id}: ${node.id}`));
  assert.deepEqual(covered,[]);
});
