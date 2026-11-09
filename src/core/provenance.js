/**
 * Metabolic provenance helpers: normalize model annotations and turn identifiers
 * into safe links to authoritative biological databases.
 */
const DB = {
  bigg:{label:'BiGG',url:(id,t)=>`https://bigg.bio/universal/${t==='reaction'?'reactions':'metabolites'}/${enc(strip(id,'bigg:'))}`},
  'bigg.reaction':{label:'BiGG Reaction',url:id=>`https://bigg.bio/universal/reactions/${enc(strip(id,'bigg.reaction:'))}`},
  'bigg.metabolite':{label:'BiGG Metabolite',url:id=>`https://bigg.bio/universal/metabolites/${enc(strip(id,'bigg.metabolite:'))}`},
  kegg:{label:'KEGG',url:id=>`https://www.kegg.jp/entry/${enc(stripPrefix(id))}`},
  'kegg.reaction':{label:'KEGG Reaction',url:id=>`https://www.kegg.jp/entry/${enc(stripPrefix(id))}`},
  'kegg.compound':{label:'KEGG Compound',url:id=>`https://www.kegg.jp/entry/${enc(stripPrefix(id))}`},
  metanetx:{label:'MetaNetX',url:id=>`https://www.metanetx.org/chem_info/${enc(stripPrefix(id))}`},
  'metanetx.chemical':{label:'MetaNetX',url:id=>`https://www.metanetx.org/chem_info/${enc(stripPrefix(id))}`},
  'metanetx.reaction':{label:'MetaNetX',url:id=>`https://www.metanetx.org/equa_info/${enc(stripPrefix(id))}`},
  chebi:{label:'ChEBI',url:id=>`https://www.ebi.ac.uk/chebi/searchId.do?chebiId=${enc(normalizeChebi(id))}`},
  pubchem:{label:'PubChem',url:id=>`https://pubchem.ncbi.nlm.nih.gov/compound/${enc(stripPrefix(id))}`},
  hmdb:{label:'HMDB',url:id=>`https://www.hmdb.ca/metabolites/${enc(stripPrefix(id))}`},
  rhea:{label:'Rhea',url:id=>`https://www.rhea-db.org/rhea/${enc(stripPrefix(id))}`},
  ec:{label:'EC',url:id=>`https://enzyme.expasy.org/EC/${enc(stripPrefix(id))}`},
  'ec-code':{label:'EC',url:id=>`https://enzyme.expasy.org/EC/${enc(stripPrefix(id))}`},
  uniprot:{label:'UniProt',url:id=>`https://www.uniprot.org/uniprotkb/${enc(stripPrefix(id))}/entry`},
  ncbigene:{label:'NCBI Gene',url:id=>`https://www.ncbi.nlm.nih.gov/gene/${enc(stripPrefix(id))}`},
  'ncbigene':{label:'NCBI Gene',url:id=>`https://www.ncbi.nlm.nih.gov/gene/${enc(stripPrefix(id))}`},
  refseq:{label:'NCBI RefSeq',url:id=>`https://www.ncbi.nlm.nih.gov/search/all/?term=${enc(stripPrefix(id))}`},
  pubmed:{label:'PubMed',url:id=>`https://pubmed.ncbi.nlm.nih.gov/${enc(stripPrefix(id))}/`},
  doi:{label:'DOI',url:id=>`https://doi.org/${encDoi(strip(id,'doi:'))}`},
  biocyc:{label:'BioCyc',url:id=>`https://biocyc.org/gene?orgid=META&id=${enc(stripPrefix(id).replace(/^META:/i,''))}`},
  reactome:{label:'Reactome',url:id=>`https://reactome.org/content/detail/${enc(stripPrefix(id))}`},
};

/**
 * Merges an annotation (Map or plain object) and notes into a Map from a
 * normalised namespace (lower case, `_` read as `.`, identifiers.org prefix
 * dropped) to unique string values. Values listed with `;` or `,` are split.
 * Notes only fill namespaces the annotation does not have.
 */
export function normalizeAnnotations(annotation={}, notes={}){
 const out=new Map();
 const add=(key,value)=>{
  if(value==null||value==='')return; const k=normalizeNamespace(key);
  const vals=Array.isArray(value)?value:[value];
  if(!out.has(k))out.set(k,[]);
  for(const v of vals.flatMap(splitValue)){if(v!==''&&!out.get(k).includes(String(v)))out.get(k).push(String(v));}
 };
 if(annotation instanceof Map) for(const [k,v] of annotation)add(k,v); else for(const [k,v] of Object.entries(annotation||{}))add(k,v);
 for(const [k,v] of Object.entries(notes||{})) if(!out.has(normalizeNamespace(k))) add(k,v);
 return out;
}
/**
 * One row {namespace, label, value, url} per annotation value, with url null
 * when the database is not known. `type` ('metabolite', 'reaction' or 'gene')
 * picks the BiGG page and keeps gene ids from being read as KEGG entries.
 */
export function annotationLinks(annotation={},notes={},type='metabolite'){
 const normalized=normalizeAnnotations(annotation,notes),rows=[];
 for(const [namespace,values] of normalized){
  for(const value of values){
   const db=DB[namespace]??inferDb(namespace,value,type);
   rows.push({namespace,label:db?.label??namespace,value,url:db?db.url(value,type):null});
  }
 }
 return rows;
}
/**
 * Identifiers.org URIs found in a text (such as an SBML annotation), as
 * {namespace: [unique ids]}. Both the `ns:id` and the `ns/id` forms are read.
 */
export function extractIdentifiersOrg(body){
 const out={}; const re=/https?:\/\/(?:identifiers\.org|registry\.identifiers\.org)\/([^"'<>\s]+)[^"'<>\s]*/gi; let m;
 while((m=re.exec(String(body??'')))){const path=decodeURIComponent(m[1]).replace(/^\/+|\/+$/g,'');let ns,id;
  if(path.includes(':')){const i=path.indexOf(':');ns=path.slice(0,i);id=path.slice(i+1);}
  else {const parts=path.split('/');ns=parts.shift();id=parts.join('/');}
  if(ns&&id){ns=normalizeNamespace(ns);(out[ns]??=[]).push(id);}
 }
 for(const k of Object.keys(out))out[k]=[...new Set(out[k])]; return out;
}
/** The annotation when it is an object, otherwise an empty one. */
export function annotationObject(raw){if(!raw)return{};if(typeof raw==='object')return raw;return{};}
function inferDb(ns,value,type){
 const n=normalizeNamespace(ns);
 if(DB[n])return DB[n];
 if(/^CHEBI:/i.test(value))return DB.chebi;if(/^MNX[MR]/i.test(value))return DB.metanetx;
 if(/^RHEA:/i.test(value))return DB.rhea;if(/^\d+\.\d+\.\d+\./.test(value))return DB.ec;
 if(type!=='gene' && /^[CRG]\d{5}$/.test(value))return DB.kegg;
 return null;
}
function normalizeNamespace(x){return String(x??'').trim().toLowerCase().replace(/^https?:\/\/identifiers\.org\//,'').replace(/_/g,'.').replace(/^kegg\.genes?$/,'kegg').replace(/^ec\.number$/,'ec').replace(/^inchi.?key$/,'inchikey');}
function splitValue(v){if(Array.isArray(v))return v.flatMap(splitValue);return typeof v==='string'&&/[;,]\s*/.test(v)?v.split(/[;,]\s*/).filter(Boolean):[v];}
function strip(v,p){const s=String(v);return s.toLowerCase().startsWith(p)?s.slice(p.length):s;}
function stripPrefix(v){return String(v).replace(/^[A-Za-z][\w.-]*:/,'');}
function normalizeChebi(v){const id=stripPrefix(v);return `CHEBI:${id}`;}
function enc(v){return encodeURIComponent(String(v));}
function encDoi(v){return String(v).split('/').map(enc).join('/');}
