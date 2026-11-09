import test from 'node:test';
import assert from 'node:assert/strict';
import { buildCrossFeedingIndex, exchangedMetabolites, filterCrossFeedingIndex } from '../src/analysis/cross-feeding-explorer.js';

const community={
 transfersByBase:new Map([['lac',[{speciesId:'A',reactionId:'A@EX_lac'},{speciesId:'B',reactionId:'B@EX_lac'}]]]),
 crossFeeding(){return {links:[{base:'lac',from:'A',to:'B',flux:3},{base:'ac',from:'A',to:'B',flux:1},{base:'lac',from:null,to:'B',flux:2}],perMetabolite:[]};}
};
test('builds active biological cross-feeding events and excludes environment',()=>{
 const x=buildCrossFeedingIndex(community,new Map());
 assert.equal(x.events.length,2);assert.equal(x.events[0].base,'lac');assert.equal(x.events[0].producerTransfer,'A@EX_lac');assert.equal(x.events[0].consumerTransfer,'B@EX_lac');assert.equal(x.pairs[0].totalFlux,4);
});
test('filters by metabolite producer consumer flux and query',()=>{
 const x=buildCrossFeedingIndex(community,new Map());
 assert.equal(filterCrossFeedingIndex(x,{metabolite:'lac'}).length,1);
 assert.equal(filterCrossFeedingIndex(x,{producer:'A',consumer:'B',minFlux:2}).length,1);
 assert.equal(filterCrossFeedingIndex(x,{query:'ac'}).length,2);
});

test('exchanged metabolites come in ecological order: acids and lactate, carbon sources, other products, the rest', () => {
  const link = (base, flux, from = 'A', to = 'B') => ({ base, from, to, flux });
  const list = exchangedMetabolites([
    link('nh4', 9), link('glc-D', 0.5), link('lac-D', 0.2), link('for', 3), link('ac', 0.1), link('ac', 0.3, 'C', 'B'),
    link('galur', 2), link('ala-L', 5), link('lac-L', 1), link('h', 50), link('but', 0.01, null, 'B'),
  ], (base) => base === 'h');
  assert.deepEqual(list.map((m) => m.base), ['ac', 'lac-L', 'lac-D', 'galur', 'glc-D', 'for', 'nh4', 'ala-L']);
  assert.equal(list[0].lines, 2, 'acetate has two producer-consumer lines');
  assert.ok(Math.abs(list[0].flux - 0.4) < 1e-12);
  assert.equal(list.find((m) => m.base === 'glc-D').group, 'Carbon sources');
  assert.ok(!list.some((m) => m.base === 'but'), 'a medium uptake is not an exchange between species');
});
