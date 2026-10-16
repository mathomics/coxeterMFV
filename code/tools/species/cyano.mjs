/**
 * Synechocystis-like cyanobacterium: a photoautotroph with four compartments.
 *
 *   e  extracellular      p  periplasm (inside the outer membrane)
 *   c  cytoplasm          u  thylakoid lumen
 *
 * Photosynthesis and respiration share the thylakoid membrane (c | u), which is
 * why both the light reactions and the terminal oxidase pump protons into the
 * same lumen and feed the same ATP synthase. Light enters as `photon`, a
 * shared medium resource: in a community the species compete for it.
 */
import { r, ex, buildModel } from '../lib/dsl.mjs';
import { glycolysis, pentosePhosphate, tca } from '../lib/blocks.mjs';

const LIGHT = 'Reacciones luminosas';
const CBB = 'Calvin-Benson-Bassham cycle';
const RESP = 'Respiration';
const CCM = 'Carbon-concentrating mechanism';
const TOM = 'Transport, outer membrane';
const TIM = 'Transport, plasma membrane';
const NIT = 'Nitrogen metabolism';

export const spec = {
  id: 'cyano_core_mc',
  name: 'Synechocystis sp. — photoautotrophic core with thylakoid',
  compartments: {
    e: 'Extracellular', p: 'Periplasm', c: 'Cytoplasm', u: 'Thylakoid lumen',
  },
  compartmentTree: { e: null, p: 'e', c: 'p', u: 'c' },
  cell: {
    name: 'Synechocystis',
    organism: 'Synechocystis sp. PCC 6803',
    color: '#16a34a',
    shape: 'round',
    gram: 'negative',
    note: 'Oxygenic photoautotroph. Fixes CO2 with RuBisCO and releases O2; its TCA cycle is incomplete (no AKGDH).',
  },
  objective: 'BIOMASS_Syn_core',
  primary: ['accoa', 'coa', 'rb15bp'],
  reactions: [
    // Light reactions (thylakoid membrane: c | u)
    r('PHOTONt', 'photon_e --> photon_c', { name: 'Light absorption by the antennae', sub: LIGHT, rank: 0, lane: 4 }),
    r('PSII', '2.0 h2o_c + 2.0 pq_c + 4.0 h_c + 4.0 photon_c --> o2_c + 2.0 pqh2_c + 4.0 h_u',
      { name: 'Photosystem II (splits water, releases O2)', sub: LIGHT, rank: 1, lane: 4 }),
    r('PSI_FNR', '2.0 pqh2_c + 2.0 nadp_c + 2.0 h_c + 4.0 photon_c --> 2.0 pq_c + 2.0 nadph_c + 4.0 h_u',
      { name: 'Cit b6f + Fotosistema I + FNR (flujo lineal a NADPH)', sub: LIGHT, rank: 2, lane: 4 }),
    r('CEF', '2.0 h_c + 2.0 photon_c --> 2.0 h_u',
      { name: 'Cyclic electron flow around PSI (ATP only)', sub: LIGHT, rank: 2, lane: 5 }),
    r('ATPS_u', 'adp_c + pi_c + 4.0 h_u --> atp_c + h2o_c + 3.0 h_c',
      { name: 'Thylakoid ATP synthase', sub: LIGHT, rank: 3, lane: 4 }),

    // Respiratory chain (same membrane)
    r('NDH1', 'nadh_c + pq_c + 4.0 h_c --> nad_c + pqh2_c + 3.0 h_u', { name: 'NDH-1 (bombea H+ al lumen)', sub: RESP, rank: 4, lane: 6 }),
    r('COX', '0.5 o2_c + pqh2_c + 2.0 h_c --> h2o_c + pq_c + 2.0 h_u', { name: 'Terminal cytochrome oxidase', sub: RESP, rank: 5, lane: 6 }),
    r('ATPM', 'atp_c + h2o_c --> adp_c + h_c + pi_c', { name: 'Non-growth-associated maintenance', sub: RESP, lb: 1.3, ub: 1000, rank: 6, lane: 6 }),

    // Calvin-Benson-Bassham
    r('RBPC', 'co2_c + h2o_c + rb15bp_c --> 2.0 3pg_c + 2.0 h_c', { name: 'RuBisCO (carboxylation)', sub: CBB, rank: 8, lane: -1 }),
    r('PRK', 'atp_c + ru5p__D_c --> adp_c + h_c + rb15bp_c', { name: 'Fosforribuloquinasa', sub: CBB, rank: 7, lane: -1 }),
    r('GAPDHn', 'g3p_c + nadp_c + pi_c <=> 13dpg_c + h_c + nadph_c', { name: 'NADPH-dependent GAPDH', sub: CBB, rank: 9, lane: -1 }),

    // Shared central carbon
    ...glycolysis({ comp: 'c', lane: 0, rank0: 10 }),
    ...pentosePhosphate({ comp: 'c', lane: -3, rank0: 10 }),
    r('PDH', 'coa_c + nad_c + pyr_c --> accoa_c + co2_c + nadh_c', { name: 'Pyruvate dehydrogenase', sub: 'Citric acid cycle', rank: 20, lane: 0 }),
    // Cyanobacteria have no 2-oxoglutarate dehydrogenase: the cycle stops at akg,
    // which is exactly where nitrogen assimilation takes over.
    ...tca({ comp: 'c', lane: 0, rank0: 21, icdhCofactor: 'NADP', full: false }),
    r('PPC', 'co2_c + h2o_c + pep_c --> h_c + oaa_c + pi_c', { name: 'PEP carboxylase', sub: 'Anaplerotic reactions', rank: 19, lane: 2 }),
    r('MDH', 'mal__L_c + nad_c <=> h_c + nadh_c + oaa_c', { name: 'Malate dehydrogenase', sub: 'Anaplerotic reactions', rank: 20, lane: 2 }),
    r('FUM', 'fum_c + h2o_c <=> mal__L_c', { name: 'Fumarasa', sub: 'Anaplerotic reactions', rank: 21, lane: 2 }),

    // Nitrogen
    r('GLUDy', 'glu__L_c + h2o_c + nadp_c <=> akg_c + h_c + nadph_c + nh4_c', { name: 'Glutamate dehydrogenase (NADP)', sub: NIT, rank: 24, lane: -4 }),
    r('GLNS', 'atp_c + glu__L_c + nh4_c --> adp_c + gln__L_c + h_c + pi_c', { name: 'Glutamina sintetasa', sub: NIT, rank: 25, lane: -4 }),

    // Carbon concentrating mechanism
    r('HCO3E', 'co2_c + h2o_c <=> h_c + hco3_c', { name: 'Carbonic anhydrase', sub: CCM, rank: 7, lane: -2 }),
    r('BCT1', 'atp_c + h2o_c + hco3_p --> adp_c + h_c + hco3_c + pi_c', { name: 'ATP-dependent bicarbonate transporter (BCT1)', sub: CCM, rank: 6, lane: -2 }),

    // Outer membrane
    r('CO2tex', 'co2_e <=> co2_p', { name: 'CO2 via porin', sub: TOM, rank: 6, lane: -1 }),
    r('HCO3tex', 'hco3_e <=> hco3_p', { name: 'Bicarbonate via porin', sub: TOM, rank: 6, lane: -2 }),
    r('O2tex', 'o2_e <=> o2_p', { name: 'O2 via porin', sub: TOM, rank: 4, lane: 7 }),
    r('NH4tex', 'nh4_e <=> nh4_p', { name: 'Ammonium via porin', sub: TOM, rank: 25, lane: -5 }),
    r('PItex', 'pi_e <=> pi_p', { name: 'Phosphate via porin', sub: TOM, rank: 3, lane: 7 }),
    r('H2Otex', 'h2o_e <=> h2o_p', { name: 'Water via porin', sub: TOM, rank: 5, lane: 7 }),
    r('Htex', 'h_e <=> h_p', { name: 'Proton via porin', sub: TOM, rank: 2, lane: 7 }),

    // Plasma membrane
    r('CO2tpp', 'co2_p <=> co2_c', { name: 'CO2 diffusion', sub: TIM, rank: 6, lane: -1 }),
    r('O2tpp', 'o2_p <=> o2_c', { name: 'O2 diffusion', sub: TIM, rank: 4, lane: 6 }),
    r('NH4tpp', 'nh4_p <=> nh4_c', { name: 'Amonio', sub: TIM, rank: 25, lane: -5 }),
    r('PIt2rpp', 'h_p + pi_p <=> h_c + pi_c', { name: 'Simporte fosfato/H+', sub: TIM, rank: 3, lane: 6 }),
    r('H2Otpp', 'h2o_p <=> h2o_c', { name: 'Agua', sub: TIM, rank: 5, lane: 6 }),

    // Exchanges
    ex('photon', { lb: -100, ub: 0, rank: 0, lane: 4 }),
    ex('co2', { lb: -1000, rank: 6, lane: -1 }),
    ex('hco3', { lb: -1000, rank: 6, lane: -2 }),
    ex('o2', { lb: -1000, rank: 4, lane: 7 }),
    ex('nh4', { lb: -1000, rank: 25, lane: -5 }),
    ex('pi', { lb: -1000, rank: 3, lane: 7 }),
    ex('h2o', { lb: -1000, rank: 5, lane: 7 }),
    ex('h', { lb: -1000, rank: 2, lane: 7 }),

    // Biomass
    r('BIOMASS_Syn_core',
      '1.40 3pg_c + 3.20 accoa_c + 48.0 atp_c + 0.34 e4p_c + 0.12 f6p_c + 0.13 g3p_c '
      + '+ 0.22 g6p_c + 0.26 gln__L_c + 4.60 glu__L_c + 48.0 h2o_c + 3.30 nad_c '
      + '+ 12.60 nadph_c + 1.70 oaa_c + 0.50 pep_c + 2.60 pyr_c + 0.92 r5p_c '
      + '--> 48.0 adp_c + 3.90 akg_c + 3.20 coa_c + 48.0 h_c + 3.30 nadh_c '
      + '+ 12.60 nadp_c + 48.0 pi_c',
      { name: 'Biomass (cyanobacterium core)', sub: 'Biomass', rank: 30, lane: 0 }),
  ],
};

export default () => buildModel(spec);
