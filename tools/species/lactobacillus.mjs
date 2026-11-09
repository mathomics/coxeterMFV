/**
 * Lactococcus/Lactobacillus-like homofermentative lactic acid bacterium.
 *
 * Two compartments and no respiratory chain: everything is substrate-level
 * phosphorylation, and the lactate it dumps into the medium is the hook that
 * feeds the other species in the demo community.
 */
import { r, ex, buildModel } from '../lib/dsl.mjs';
import { glycolysis, pentosePhosphate } from '../lib/blocks.mjs';

const TRA = 'Membrane transport';
const FER = 'Lactic fermentation';

export const spec = {
  id: 'lactobacillus_core_mc',
  name: 'Lactococcus lactis — homofermentative core',
  compartments: { e: 'Extracellular', c: 'Cytoplasm' },
  compartmentTree: { e: null, c: 'e' },
  cell: {
    name: 'L. lactis',
    organism: 'Lactococcus lactis subsp. lactis',
    color: '#db2777',
    shape: 'round',
    gram: 'positive',
    note: 'Aerotolerant anaerobe. Converts glucose almost quantitatively into L-lactate.',
  },
  objective: 'BIOMASS_Lb_core',
  primary: ['accoa', 'coa', 'lac__L'],
  reactions: [
    r('GLCt1', 'glc__D_e <=> glc__D_c', { name: 'Glucose transport', sub: TRA, rank: 0, lane: 0 }),
    r('HEX1', 'atp_c + glc__D_c --> adp_c + g6p_c + h_c', { name: 'Hexoquinasa', sub: 'Glycolysis', rank: 1, lane: 0 }),
    ...glycolysis({ comp: 'c', lane: 0, rank0: 2, includeFBP: false }),
    ...pentosePhosphate({ comp: 'c', lane: -3, rank0: 2 }),

    r('LDH_L', 'lac__L_c + nad_c <=> h_c + nadh_c + pyr_c', { name: 'L-lactate dehydrogenase', sub: FER, rank: 11, lane: -1 }),
    // Pure stoichiometry always prefers mixed-acid fermentation (acetate kinase
    // yields extra ATP). Real L. lactis stays homolactic at high glycolytic flux
    // because triose phosphates inhibit PFL. The cap stands in for that
    // regulation, which FBA cannot represent on its own. Raise it to 1000 to
    // recover the unconstrained mixed-acid optimum.
    r('PFL', 'coa_c + pyr_c --> accoa_c + for_c', { name: 'Pyruvate formate lyase', sub: FER, lb: 0, ub: 4.0, rank: 11, lane: 1 }),
    r('PDH', 'coa_c + nad_c + pyr_c --> accoa_c + co2_c + nadh_c', { name: 'Pyruvate dehydrogenase', sub: FER, rank: 11, lane: 2 }),
    r('PTAr', 'accoa_c + pi_c <=> actp_c + coa_c', { name: 'Fosfotransacetilasa', sub: FER, rank: 12, lane: 1 }),
    r('ACKr', 'ac_c + atp_c <=> actp_c + adp_c', { name: 'Acetate kinase', sub: FER, rank: 13, lane: 1 }),
    // Homolactic fermentation is exactly redox neutral, so without a reduced
    // overflow product there is no spare NADH to divert carbon into biomass.
    // L. lactis shifts to mixed-acid fermentation exactly for this reason.
    r('ACALD', 'acald_c + coa_c + nad_c <=> accoa_c + h_c + nadh_c', { name: 'Acetaldehyde dehydrogenase (acetylating)', sub: FER, rank: 12, lane: 3 }),
    r('ALCD2x', 'etoh_c + nad_c <=> acald_c + h_c + nadh_c', { name: 'Alcohol dehydrogenase', sub: FER, rank: 13, lane: 3 }),
    r('ATPM', 'atp_c + h2o_c --> adp_c + h_c + pi_c', { name: 'Non-growth-associated maintenance', sub: FER, lb: 1.5, ub: 1000, rank: 14, lane: 4 }),

    r('L_LACt2r', 'lac__L_e + h_e <=> lac__L_c + h_c', { name: 'Simporte L-lactato/H+', sub: TRA, rank: 12, lane: -1 }),
    r('ACt2r', 'ac_e + h_e <=> ac_c + h_c', { name: 'Simporte acetato/H+', sub: TRA, rank: 14, lane: 1 }),
    r('FORt', 'for_c <=> for_e', { name: 'Formiato canal', sub: TRA, rank: 12, lane: 2 }),
    r('ETOHt', 'etoh_e <=> etoh_c', { name: 'Ethanol diffusion', sub: TRA, rank: 14, lane: 3 }),
    r('CO2t', 'co2_e <=> co2_c', { name: 'CO2 diffusion', sub: TRA, rank: 12, lane: 3 }),
    r('NH4t', 'nh4_e <=> nh4_c', { name: 'Amonio', sub: TRA, rank: 13, lane: -3 }),
    r('PIt2r', 'h_e + pi_e <=> h_c + pi_c', { name: 'Simporte fosfato/H+', sub: TRA, rank: 5, lane: 4 }),
    r('H2Ot', 'h2o_e <=> h2o_c', { name: 'Agua', sub: TRA, rank: 6, lane: 4 }),
    r('Ht', 'h_e <=> h_c', { name: 'Proton', sub: TRA, rank: 7, lane: 4 }),

    ex('glc__D', { lb: -10, rank: 0, lane: 0 }),
    ex('lac__L', { lb: 0, rank: 12, lane: -1 }),
    ex('ac', { lb: 0, rank: 14, lane: 1 }),
    ex('for', { lb: 0, rank: 12, lane: 2 }),
    ex('etoh', { lb: 0, rank: 14, lane: 3 }),
    ex('co2', { lb: -1000, rank: 12, lane: 3 }),
    ex('nh4', { lb: -1000, rank: 13, lane: -3 }),
    ex('pi', { lb: -1000, rank: 5, lane: 4 }),
    ex('h2o', { lb: -1000, rank: 6, lane: 4 }),
    ex('h', { lb: -1000, rank: 7, lane: 4 }),

    r('BIOMASS_Lb_core',
      '0.20 g6p_c + 0.10 f6p_c + 0.90 r5p_c + 0.30 e4p_c + 1.30 3pg_c + 0.50 pep_c '
      + '+ 2.80 pyr_c + 2.00 accoa_c + 2.50 nh4_c + 45.0 atp_c + 45.0 h2o_c + 10.0 nadph_c '
      + '--> 45.0 adp_c + 45.0 pi_c + 2.00 coa_c + 10.0 nadp_c + 47.5 h_c',
      { name: 'Biomass (lactic acid bacterium core)', sub: 'Biomass', rank: 16, lane: 0 }),
  ],
};

export default () => buildModel(spec);
