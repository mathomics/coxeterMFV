/**
 * Cross-Feeding Explorer V1.
 * Builds an explicit, filterable event index from the solved community fluxes.
 * An event is a link from one species to a different one, with the transfer
 * reaction on each side. `threshold` is the smallest flux counted (default
 * 1e-6) and `raw` reuses a `community.crossFeeding` result already computed.
 * Returns {events, metabolites, species, pairs, threshold}, events and pairs
 * sorted by flux, largest first.
 */
export function buildCrossFeedingIndex(community, fluxes, { threshold=1e-6, raw: suppliedRaw=null }={}){
 const raw=suppliedRaw??community.crossFeeding(fluxes,threshold);
 const events=raw.links.filter(x=>x.from&&x.to&&x.from!==x.to).map((x,i)=>{
  const transfers=community.transfersByBase.get(x.base)??[];
  const producerTransfer=transfers.find(t=>t.speciesId===x.from)?.reactionId??null;
  const consumerTransfer=transfers.find(t=>t.speciesId===x.to)?.reactionId??null;
  return {...x,id:`cf:${x.from}:${x.base}:${x.to}:${i}`,producerTransfer,consumerTransfer,evidence:'active-flux'};
 }).sort((a,b)=>b.flux-a.flux);
 const metabolites=[...new Set(events.map(x=>x.base))].sort();
 const species=[...new Set(events.flatMap(x=>[x.from,x.to]))].sort();
 const pairs=new Map();
 for(const e of events){const k=`${e.from}→${e.to}`;const p=pairs.get(k)??{from:e.from,to:e.to,totalFlux:0,events:0,metabolites:[]};p.totalFlux+=e.flux;p.events++;p.metabolites.push(e.base);pairs.set(k,p);}
 return {events,metabolites,species,pairs:[...pairs.values()].sort((a,b)=>b.totalFlux-a.totalFlux),threshold};
}
/**
 * Events of an index that pass every filter given: a text query matched against
 * metabolite and species ids, an exact metabolite, producer or consumer, and a
 * minimum flux.
 */
export function filterCrossFeedingIndex(index,{query='',metabolite='',producer='',consumer='',minFlux=0}={}){
 const q=String(query).trim().toLowerCase();
 return (index?.events??[]).filter(e=>(!metabolite||e.base===metabolite)&&(!producer||e.from===producer)&&(!consumer||e.to===consumer)&&e.flux>=Number(minFlux||0)&&(!q||[e.base,e.from,e.to].some(x=>String(x).toLowerCase().includes(q))));
}

/*
 * Which exchanged metabolites the map shows. Ecologically, the short-chain
 * fatty acids and lactate come first, in a fixed order. Then come the carbon
 * sources (sugars, uronic acids, oligo- and polysaccharides), the other
 * fermentation products and everything else. Within a group, the one that
 * moves the most between species comes first.
 */
const KEY_ACIDS = ['ac', 'but', 'ppa', 'lacl', 'lacd'];
const CARBON_SOURCE = /^(glc|fru|gal|man|xyl|arab|rib|rmn|fuc|sucr|lcts|malt|cellb|tre|acgam|acnam|glcur|kesto|inulin|strch|starch|pect|amylose|glucan|xylan|arabinan|homogal|rhamnogal|dextrin|raffin|stys|melib|lactul)/;
const OTHER_PRODUCTS = new Set(['succ', 'for', 'etoh', 'acald', 'h2', 'pyr', '12ppds', '12ppdr', 'ibut', 'ival', '2mbut',
  'isobut', 'isoval', '2mba', 'btd', 'actn', 'diact']);
export const CROSS_FEEDING_GROUPS = ['Short-chain fatty acids and lactate', 'Carbon sources', 'Other fermentation products', 'Other'];

const metaboliteKey = (base) => String(base ?? '').toLowerCase().replace(/[^a-z0-9]/g, '');

function crossFeedingGroup(base) {
  const k = metaboliteKey(base);
  const acid = KEY_ACIDS.indexOf(k);
  if (acid >= 0) return [0, acid];
  if (CARBON_SOURCE.test(k)) return [1, 0];
  if (OTHER_PRODUCTS.has(k)) return [2, 0];
  return [3, 0];
}

/**
 * The metabolites species pass to each other (links with a producer and a
 * consumer), most important first: {base, flux, lines, group}. `skip(base)`
 * leaves some out (protons and water).
 */
export function exchangedMetabolites(links, skip = () => false) {
  const byBase = new Map();
  for (const link of links ?? []) {
    if (!link.from || !link.to || skip(link.base)) continue;
    const entry = byBase.get(link.base) ?? { base: link.base, flux: 0, lines: 0 };
    entry.flux += link.flux;
    entry.lines += 1;
    byBase.set(link.base, entry);
  }
  return [...byBase.values()]
    .map((entry) => ({ entry, rank: crossFeedingGroup(entry.base) }))
    .sort((a, b) => a.rank[0] - b.rank[0] || a.rank[1] - b.rank[1] || b.entry.flux - a.entry.flux
      || (a.entry.base < b.entry.base ? -1 : 1))
    .map(({ entry, rank }) => ({ ...entry, group: CROSS_FEEDING_GROUPS[rank[0]] }));
}
