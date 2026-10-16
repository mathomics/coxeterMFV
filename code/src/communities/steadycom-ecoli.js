/**
 * Published community: four auxotrophic Escherichia coli strains (SteadyCom).
 *
 *   Chan SHJ, Simons MN, Maranas CD (2017) SteadyCom: Predicting microbial
 *   abundances while ensuring community stability.
 *   PLoS Comput Biol 13(5): e1005539.  doi:10.1371/journal.pcbi.1005539
 *
 * Everything here is transcribed from the authors' own COBRA Toolbox tutorial
 * (opencobra/COBRA.tutorials, analysis/steadyCom/tutorial_steadyCom.m), not
 * inferred from the paper's prose. The tutorial's MATLAB is quoted beside each
 * constraint so the two can be checked line by line.
 *
 * The community is four E. coli, each a double amino-acid auxotroph that is also
 * unable to export one amino acid it can still make. Only two auxotrophy pairs
 * exist, each used twice, so what separates Ec1 from Ec4 is which amino acid it
 * cannot export. No member grows alone, so the four must feed each other.
 *
 *   strain  auxotroph for  blocked reactions   cannot export  therefore supplies
 *   Ec1     Lys + Met      DAPDC, HSST         Phe (PHEt2rpp)  Arg
 *   Ec2     Arg + Phe      ARGSL, PPNDH        Met (METt3pp)   Lys
 *   Ec3     Arg + Phe      ARGSL, PPNDH        Lys (LYSt3pp)   Met
 *   Ec4     Lys + Met      DAPDC, HSST         Arg (ARGt3pp)   Phe
 *
 * The tutorial names its MATLAB variables after genes while they hold reaction
 * ids: `ilvE` holds PPNDH, prephenate dehydratase. The auxotrophy is imposed on
 * PPNDH, not on a transaminase.
 */

import { parseCOBRASBML } from '../core/sbml.js';
import { Model } from '../core/model.js';

/** Where the inputs come from, so a reader can fetch the same files. */
export const PROVENANCE = {
  paper: {
    title: 'SteadyCom: Predicting microbial abundances while ensuring community stability',
    authors: 'Chan SHJ, Simons MN, Maranas CD',
    year: 2017,
    journal: 'PLoS Computational Biology',
    doi: '10.1371/journal.pcbi.1005539',
    framework: 'SteadyCom',
  },
  procedure: {
    source: 'opencobra/COBRA.tutorials',
    path: 'analysis/steadyCom/tutorial_steadyCom.m',
    note: 'The authors\' own tutorial for this community; every bound below is quoted from it.',
  },
  model: {
    id: 'iAF1260',
    organism: 'Escherichia coli K-12 MG1655',
    reference: 'Feist AM et al. (2007) Mol Syst Biol 3:121. doi:10.1038/msb4100155',
    file: 'iAF1260.xml',
    source: 'opencobra/m_model_collection, sbml/iAF1260.xml',
    note: 'The tutorial loads iAF1260.mat from the COBRA Toolbox; this is the same '
      + 'reconstruction as distributed in SBML by the m_model_collection.',
  },
};

/** The one reaction the tutorial adds to iAF1260. */
export const ADDED_REACTION = {
  id: 'METt3pp',
  tutorial: "iAF1260 = addReaction(iAF1260,{'METt3pp',''},'met__L[c] + h[c] => met__L[p] + h[p]');",
};

// iAF1260 spells amino acids `arg-L`, not the newer `arg__L`, while its exchange
// reactions use `EX_arg_L_e` with an underscore. Metabolite and exchange ids are
// therefore kept apart rather than derived from one another.
const AA = { arg: 'arg-L', lys: 'lys-L', met: 'met-L', phe: 'phe-L' };
const AA_EXCHANGE = {
  arg: 'EX_arg_L_e', lys: 'EX_lys_L_e', met: 'EX_met_L_e', phe: 'EX_phe_L_e',
};

/** `argH`, `lysA`, `metA`, `ilvE` in the tutorial. */
const AUXOTROPHY = { arg: 'ARGSL', lys: 'DAPDC', met: 'HSST', phe: 'PPNDH' };
/** `argO`, `lysO`, `yjeH`, `yddG` in the tutorial. */
const EXPORTER = { arg: 'ARGt3pp', lys: 'LYSt3pp', met: 'METt3pp', phe: 'PHEt2rpp' };

/** The four strains of the table above, amino acids keyed as in `AA`. */
export const STRAINS = [
  { id: 'Ec1', auxotrophFor: ['lys', 'met'], cannotExport: 'phe', supplies: 'arg' },
  { id: 'Ec2', auxotrophFor: ['arg', 'phe'], cannotExport: 'met', supplies: 'lys' },
  { id: 'Ec3', auxotrophFor: ['arg', 'phe'], cannotExport: 'lys', supplies: 'met' },
  { id: 'Ec4', auxotrophFor: ['lys', 'met'], cannotExport: 'arg', supplies: 'phe' },
];

/** `exRate = 1`: the most of a cross-fed amino acid any member may take up. */
export const EX_RATE = 1;

/**
 * `EcCom.ub(EcCom.indCom.EXsp(:)) = 1000;`: every member may secrete anything up
 * to 1000, which is also COBRA's default flux bound.
 */
export const EXCHANGE_UPPER_BOUND = 1000;

/**
 * The growth rate the tutorial reports for this community, and therefore the
 * number a reproduction has to land on.
 */
export const EXPECTED_GROWTH_RATE = 0.73599;

/**
 * Documented departure from the source file.
 *
 * The Feist 2007 SBML writes 999999 where COBRA writes 1000 to mean "no limit",
 * and SteadyCom turns every bound into a coefficient: `V - UB*X <= 0`. A
 * coefficient of 1e6 beside a growth rate of 1e-6 spans twelve orders of
 * magnitude, which is enough on its own to make GLPK abandon the LP (GLP_UNDEF).
 * These bounds are therefore read as the infinity they stand for and rewritten to
 * COBRA's 1000. SENTINEL_BOUND is the threshold for that: any bound at or above
 * 1e5 in magnitude, which catches the file's 999999 and nothing a reaction needs.
 *
 * This is checked, not assumed, by test/steadycom.test.mjs: a single strain grows
 * at exactly the same rate either way, and so does the community at a fixed
 * growth rate, because no reaction carrying mass ever approaches 1000 when
 * glucose is capped at 8 mmol/gDW/h. The only fluxes that reached 999999 were
 * futile cycles, which carry no net mass.
 */
export const SENTINEL_BOUND = 1e5;

/**
 * Build the SteadyCom problem for this community.
 *
 * @param {string} xml  contents of iAF1260.xml
 * @returns {{members: object[], communityUptake: Object<string,number>,
 *            totalBiomass: number, document: object}}
 */
export function buildCommunity(xml) {
  const doc = parseCOBRASBML(xml, { sentinelBound: SENTINEL_BOUND });

  doc.reactions.push({
    id: ADDED_REACTION.id,
    name: 'L-methionine export via proton antiport (added by the SteadyCom tutorial)',
    metabolites: { [`${AA.met}_c`]: -1, h_c: -1, [`${AA.met}_p`]: 1, h_p: 1 },
    lower_bound: 0,
    upper_bound: EXCHANGE_UPPER_BOUND,
  });

  const members = STRAINS.map((strain) => {
    const copy = structuredClone(doc);
    const byId = new Map(copy.reactions.map((r) => [r.id, r]));
    const block = (rid) => {
      const r = byId.get(rid);
      if (!r) throw new Error(`${strain.id}: reaction ${rid} is not in iAF1260`);
      r.lower_bound = 0;
      r.upper_bound = 0;
    };

    // changeRxnBounds(Ec1, [lysA; metA; yddG], 0, 'b') and its three siblings.
    for (const aa of strain.auxotrophFor) block(AUXOTROPHY[aa]);
    block(EXPORTER[strain.cannotExport]);

    // EcCom.ub(EXsp(:)) = 1000: secretion of anything, capped at 1000.
    for (const r of copy.reactions) {
      if (r.id.startsWith('EX_')) r.upper_bound = EXCHANGE_UPPER_BOUND;
    }

    // Only the amino acids a strain lacks may be taken up, and only at exRate.
    for (const [aa, exId] of Object.entries(AA_EXCHANGE)) {
      const ex = byId.get(exId);
      if (!ex) throw new Error(`${strain.id}: no exchange reaction ${exId}`);
      ex.lower_bound = strain.auxotrophFor.includes(aa) ? -EX_RATE : 0;
    }

    const model = new Model(copy);
    return {
      id: strain.id,
      strain,
      model,
      biomassReaction: model.biomassReactionId(),
      exchangeFor: exchangeIndex(model),
    };
  });

  // EcCom.lb(EXcom(:,1)) = lbEx: the community takes up what iAF1260 itself
  // allows, so the medium is the model's own and nothing is invented here.
  const communityUptake = {};
  for (const r of doc.reactions) {
    if (r.id.startsWith('EX_') && r.lower_bound < 0) {
      communityUptake[exchangeBase(r.id)] = -r.lower_bound;
    }
  }

  return { members, communityUptake, totalBiomass: 1, document: doc };
}

/** `EX_glc_e` -> `glc`: the metabolite an exchange reaction trades. */
export const exchangeBase = (rid) => rid.replace(/^EX_/, '').replace(/_e$/, '');

function exchangeIndex(model) {
  const out = {};
  for (const rid of model.reactions.keys()) {
    if (rid.startsWith('EX_')) out[exchangeBase(rid)] = rid;
  }
  return out;
}

/** The amino-acid exchange each strain uses, for reporting cross-feeding. */
export const AMINO_ACID_EXCHANGES = AA_EXCHANGE;
