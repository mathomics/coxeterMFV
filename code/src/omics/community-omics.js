/**
 * Community metatranscriptomics layer.
 *
 * Accepts long tables (species/MAG, gene, condition, expression) and converts
 * them to the common wide omics dataset consumed by Coxeter's GPR mapper.
 * Also reports mapping coverage by species and condition.
 */
import { omicsKey, parseOmicsCSV, reactionOmicsScores } from './overlay.js';

const SPECIES = new Set(['species','organism','cell','mag','genome','bin']);
const IDS = new Set(['gene','gene_id','geneid','locus','locus_tag','identifier','id']);
const CONDITIONS = new Set(['condition','sample','state','group','treatment']);
const VALUES = new Set(['expression','abundance','value','count','counts','tpm','fpkm','rpkm','cpm']);

/**
 * Reads a CSV or TSV (delimiter detected from the header) of community
 * expression. A long table becomes a dataset keyed by omicsKey(species, gene),
 * one value per condition. A table with species and gene columns but no
 * condition or value column is read as a wide one by parseOmicsCSV.
 * Returns {dataset, format: 'long' | 'wide'}.
 */
export function parseMetatranscriptomics(text, { source='metatranscriptomics.csv' }={}) {
  const delimiter = detectDelimiter(text);
  const rows = parseDelimited(text, delimiter);
  if (rows.length < 2) throw new Error('The metatranscriptomics table needs a header row and data.');
  const header=rows[0].map(x=>String(x).trim());
  const norm=header.map(normalize);
  const si=find(norm,SPECIES), gi=find(norm,IDS), ci=find(norm,CONDITIONS), vi=find(norm,VALUES);
  if (si<0) throw new Error('Falta una columna species/MAG/organism.');
  if (gi<0) throw new Error('Falta una columna gene/gene_id/locus_tag.');
  if (ci<0 || vi<0) {
    // Already-wide community omics is accepted as a convenience.
    return { dataset: parseOmicsCSV(text,{source,dataType:'metatranscriptomics'}), format:'wide' };
  }

  const conditions=[]; const seen=new Set(); const records=new Map();
  for(const row of rows.slice(1)){
    const species=String(row[si]??'').trim(), gene=String(row[gi]??'').trim();
    const condition=String(row[ci]??'').trim(), value=Number(String(row[vi]??'').trim());
    if(!species || !gene || !condition || !Number.isFinite(value)) continue;
    if(!seen.has(condition)){seen.add(condition);conditions.push(condition);}
    const key=omicsKey(species,gene);
    const rec=records.get(key)??{id:gene,species,values:{}};
    rec.values[condition]=value; records.set(key,rec);
  }
  if(!records.size) throw new Error('No usable metatranscriptomics values were found.');
  return {dataset:{
    source,dataType:'metatranscriptomics',idColumn:header[gi],speciesColumn:header[si],
    conditions,values:records,rowCount:records.size,
  },format:'long'};
}

/**
 * How well a dataset covers the community: per species, the model genes with a
 * measurement and the GPR reactions that get a score (the best condition
 * counts), plus overall coverage fractions and the dataset species that match
 * no configured species.
 */
export function communityOmicsMappingReport(community,speciesConfig,models,dataset){
  const speciesRows=[]; let genesInModels=0, measuredGenes=0, mappedReactions=0, gprReactions=0;
  for(const sp of speciesConfig??[]){
    const model=models.get(sp.file); if(!model) continue;
    const geneIds=new Set((model.genes??[]).map(g=>String(g.id??g)));
    // GPR-only genes are common in imperfect reconstructions, so include them.
    for(const r of model.reactions.values()) for(const g of genesFromGPR(r.geneReactionRule)) geneIds.add(g);
    const measured=[...geneIds].filter(g=>dataset.values.has(omicsKey(sp.id,g))||dataset.values.has(omicsKey('',g))).length;
    const localGpr=[...model.reactions.values()].filter(r=>r.geneReactionRule?.trim()).length;
    const perCondition={};
    for(const condition of dataset.conditions){
      const scores=reactionOmicsScores(community,speciesConfig,models,dataset,condition);
      perCondition[condition]=[...scores.keys()].filter(rid=>community.reactionOwner.get(rid)?.speciesId===sp.id).length;
    }
    const mapped=Math.max(0,...Object.values(perCondition));
    genesInModels+=geneIds.size; measuredGenes+=measured; gprReactions+=localGpr; mappedReactions+=mapped;
    speciesRows.push({species:sp.id,genes:geneIds.size,measuredGenes:measured,gprReactions:localGpr,mappedReactions:mapped,conditions:perCondition});
  }
  const inputSpecies=new Set([...dataset.values.values()].map(r=>r.species).filter(Boolean));
  const configured=new Set((speciesConfig??[]).map(s=>s.id));
  const unmatchedSpecies=[...inputSpecies].filter(s=>!configured.has(s)).sort();
  return {
    inputRows:dataset.rowCount,inputSpecies:inputSpecies.size,modelSpecies:speciesRows.length,
    genesInModels,measuredGenes,gprReactions,mappedReactions,
    geneCoverage:genesInModels?measuredGenes/genesInModels:0,
    reactionCoverage:gprReactions?mappedReactions/gprReactions:0,
    unmatchedSpecies,species:speciesRows,
  };
}

function genesFromGPR(rule){
  return String(rule??'').replace(/[()]/g,' ').split(/\s+/).filter(x=>x && !/^(and|or)$/i.test(x));
}
function find(header,set){return header.findIndex(x=>set.has(x));}
function normalize(x){return String(x??'').trim().toLowerCase().replace(/[\s.-]+/g,'_');}
function detectDelimiter(text){const first=String(text??'').split(/\r?\n/,1)[0]??''; return (first.match(/\t/g)||[]).length>(first.match(/,/g)||[]).length?'\t':',';}
function parseDelimited(text,delimiter){
 const rows=[]; let row=[],field='',quoted=false; const input=String(text??'').replace(/^\uFEFF/,'');
 for(let i=0;i<input.length;i++){const ch=input[i];
  if(quoted){if(ch==='"'&&input[i+1]==='"'){field+='"';i++;}else if(ch==='"')quoted=false;else field+=ch;}
  else if(ch==='"')quoted=true; else if(ch===delimiter){row.push(field);field='';}
  else if(ch==='\n'){row.push(field.replace(/\r$/,''));if(row.some(x=>String(x).trim()))rows.push(row);row=[];field='';}
  else field+=ch;
 }
 row.push(field.replace(/\r$/,''));if(row.some(x=>String(x).trim()))rows.push(row);return rows;
}
