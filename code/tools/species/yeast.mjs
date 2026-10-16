/**
 * Saccharomyces cerevisiae core metabolism with an explicit mitochondrion.
 *
 * Four compartments, three of them membrane-bound:
 *   e  cell surface layer      c  cytosol
 *   im mitochondrial intermembrane space   m  mitochondrial matrix
 *
 * The respiratory chain is laid out across the inner membrane: NDI1/NDE1 feed
 * the ubiquinone pool without pumping (S. cerevisiae has no Complex I), the
 * lumped bc1/COX step pumps into the intermembrane space, and ATP synthase
 * brings the protons back into the matrix.
 */
import { r, ex, buildModel } from '../lib/dsl.mjs';
import { glycolysis, pentosePhosphate, tca } from '../lib/blocks.mjs';

const FER = 'Alcoholic fermentation';
const OXP = 'Oxidative phosphorylation';
const TMM = 'Mitochondrial transport';
const TPM = 'Transport, plasma membrane';
const ANA = 'Anaplerotic reactions';
const NIT = 'Nitrogen metabolism';

export const spec = {
  id: 'yeast_core_mc',
  name: 'Saccharomyces cerevisiae — core with mitochondrion',
  compartments: {
    e: 'Extracellular', c: 'Cytosol',
    im: 'Intermembrane space', m: 'Mitochondrial matrix',
  },
  compartmentTree: { e: null, c: 'e', im: 'c', m: 'im' },
  cell: {
    name: 'S. cerevisiae',
    organism: 'Saccharomyces cerevisiae S288C',
    color: '#d97706',
    shape: 'round',
    gram: null,
    note: 'Eukaryote. Crabtree positive: ferments to ethanol even with O2 when glucose is in excess.',
  },
  objective: 'BIOMASS_Sc_core',
  primary: ['accoa', 'coa', 'acald'],
  reactions: [
    // Plasma membrane
    r('GLCt1', 'glc__D_e <=> glc__D_c', { name: 'Glucose transport (Hxt, facilitated diffusion)', sub: TPM, rank: 0, lane: 0 }),
    r('HEX1', 'atp_c + glc__D_c --> adp_c + g6p_c + h_c', { name: 'Hexoquinasa', sub: 'Glycolysis', rank: 0, lane: 0 }),
    r('PMA1', 'atp_c + h2o_c --> adp_c + pi_c + h_e', { name: 'Plasma-membrane H+-ATPase (generates the proton-motive force)', sub: TPM, rank: 2, lane: 7 }),
    r('O2t', 'o2_e <=> o2_c', { name: 'O2 diffusion', sub: TPM, rank: 4, lane: 6 }),
    r('CO2t', 'co2_e <=> co2_c', { name: 'CO2 diffusion', sub: TPM, rank: 12, lane: 5 }),
    r('ETOHt', 'etoh_e <=> etoh_c', { name: 'Ethanol diffusion', sub: TPM, rank: 14, lane: -2 }),
    r('ACt2r', 'ac_e + h_e <=> ac_c + h_c', { name: 'Simporte acetato/H+', sub: TPM, rank: 13, lane: -1 }),
    r('GLYCt', 'glyc_e <=> glyc_c', { name: 'Glycerol facilitated diffusion', sub: TPM, rank: 6, lane: 3 }),
    r('PYRt2', 'h_e + pyr_e <=> h_c + pyr_c', { name: 'Simporte piruvato/H+', sub: TPM, rank: 10, lane: 2 }),
    r('NH4t', 'nh4_e <=> nh4_c', { name: 'Amonio', sub: TPM, rank: 16, lane: -5 }),
    r('PIt2r', 'h_e + pi_e <=> h_c + pi_c', { name: 'Simporte fosfato/H+', sub: TPM, rank: 3, lane: 7 }),
    r('H2Ot', 'h2o_e <=> h2o_c', { name: 'Agua (acuaporina)', sub: TPM, rank: 5, lane: 7 }),

    // Cytosol
    ...glycolysis({ comp: 'c', lane: 0, rank0: 1 }),
    ...pentosePhosphate({ comp: 'c', lane: -3, rank0: 1 }),

    // Alcoholic fermentation
    r('PDC', 'h_c + pyr_c --> acald_c + co2_c', { name: 'Piruvato descarboxilasa', sub: FER, rank: 11, lane: -2 }),
    r('ALCD2x', 'etoh_c + nad_c <=> acald_c + h_c + nadh_c', { name: 'Alcohol dehydrogenase', sub: FER, rank: 13, lane: -2 }),
    r('ALDD2y', 'acald_c + h2o_c + nadp_c --> ac_c + 2.0 h_c + nadph_c', { name: 'Acetaldehyde dehydrogenase (NADP)', sub: FER, rank: 12, lane: -1 }),
    // Yeast carries both cofactor specificities (Ald6 is NADP-linked and a major
    // NADPH source, Ald2/Ald3 are NAD-linked). With only the NADP form, ethanol
    // catabolism strands NADPH that nothing but biomass can reoxidise.
    r('ALDD2x', 'acald_c + h2o_c + nad_c --> ac_c + 2.0 h_c + nadh_c', { name: 'Acetaldehyde dehydrogenase (NAD)', sub: FER, rank: 12, lane: 0 }),
    r('ACS', 'ac_c + atp_c + coa_c --> accoa_c + amp_c + ppi_c', { name: 'Acetyl-CoA synthetase (PDH bypass)', sub: FER, rank: 13, lane: 0 }),
    r('PPA', 'h2o_c + ppi_c --> h_c + 2.0 pi_c', { name: 'Inorganic pyrophosphatase', sub: FER, rank: 14, lane: 1 }),
    r('ADK1', 'amp_c + atp_c <=> 2.0 adp_c', { name: 'Adenylate kinase', sub: FER, rank: 14, lane: 2 }),

    // Glycerol branch: the cytosolic redox sink
    r('G3PD1ir', 'dhap_c + h_c + nadh_c --> glyc3p_c + nad_c', { name: 'Glycerol-3-phosphate dehydrogenase', sub: FER, rank: 5, lane: 3 }),
    r('G3PT', 'glyc3p_c + h2o_c --> glyc_c + pi_c', { name: 'Glycerol-3-phosphatase', sub: FER, rank: 6, lane: 3 }),

    // Anaplerotic and cytosolic C4
    r('HCO3E', 'co2_c + h2o_c <=> h_c + hco3_c', { name: 'Carbonic anhydrase', sub: ANA, rank: 10, lane: 3 }),
    r('PC', 'atp_c + hco3_c + pyr_c --> adp_c + h_c + oaa_c + pi_c', { name: 'Pyruvate carboxylase', sub: ANA, rank: 11, lane: 3 }),
    r('MDH_c', 'mal__L_c + nad_c <=> h_c + nadh_c + oaa_c', { name: 'Cytosolic malate dehydrogenase (Mdh2)', sub: ANA, rank: 12, lane: 3 }),
    // Cytosolic glyoxylate cycle (Cit2 / Icl1 / Mls1) plus Pck1: the route that
    // lets yeast grow on the ethanol and acetate other species leave behind.
    r('CS_c', 'accoa_c + h2o_c + oaa_c --> cit_c + coa_c + h_c', { name: 'Cytosolic citrate synthase (Cit2)', sub: ANA, rank: 14, lane: 4 }),
    r('ACONT_c', 'cit_c <=> icit_c', { name: 'Cytosolic aconitase', sub: ANA, rank: 15, lane: 4 }),
    r('ICL', 'icit_c --> glx_c + succ_c', { name: 'Isocitrate lyase (Icl1)', sub: ANA, rank: 16, lane: 4 }),
    r('MALS', 'accoa_c + glx_c + h2o_c --> coa_c + h_c + mal__L_c', { name: 'Malate synthase (Mls1)', sub: ANA, rank: 17, lane: 4 }),
    r('PPCK', 'atp_c + oaa_c --> adp_c + co2_c + pep_c', { name: 'PEP carboxiquinasa (Pck1)', sub: ANA, rank: 13, lane: 4 }),
    r('SUCCtm', 'succ_c + mal__L_m <=> succ_m + mal__L_c', { name: 'Succinate/malate antiport (dicarboxylate carrier)', sub: TMM, rank: 18, lane: 4 }),

    // Nitrogen
    r('GLUDy', 'glu__L_c + h2o_c + nadp_c <=> akg_c + h_c + nadph_c + nh4_c', { name: 'Glutamate dehydrogenase (NADP)', sub: NIT, rank: 16, lane: -4 }),
    r('GLNS', 'atp_c + glu__L_c + nh4_c --> adp_c + gln__L_c + h_c + pi_c', { name: 'Glutamina sintetasa', sub: NIT, rank: 17, lane: -4 }),
    r('ATPM', 'atp_c + h2o_c --> adp_c + h_c + pi_c', { name: 'Non-growth-associated maintenance', sub: OXP, lb: 1.0, ub: 1000, rank: 8, lane: 4 }),

    // Mitochondrial transport (inner membrane)
    r('PYRt2m', 'h_c + pyr_c --> h_m + pyr_m', { name: 'Mitochondrial pyruvate carrier (MPC)', sub: TMM, rank: 10, lane: 0 }),
    r('O2tm', 'o2_c <=> o2_m', { name: 'O2 diffusion (mitochondrion)', sub: TMM, rank: 4, lane: 5 }),
    r('CO2tm', 'co2_m <=> co2_c', { name: 'CO2 diffusion (mitochondrion)', sub: TMM, rank: 12, lane: 4 }),
    r('H2Otm', 'h2o_c <=> h2o_m', { name: 'Agua (mitocondria)', sub: TMM, rank: 5, lane: 5 }),
    r('PIt2m', 'h_c + pi_c --> h_m + pi_m', { name: 'Simporte fosfato/H+ (mitocondria)', sub: TMM, rank: 6, lane: 5 }),
    r('ATPtm', 'adp_c + atp_m --> adp_m + atp_c', { name: 'Translocasa ADP/ATP (ANT)', sub: TMM, rank: 7, lane: 5 }),
    r('AKGMALtm', 'akg_m + mal__L_c <=> akg_c + mal__L_m', { name: 'Antiporte 2-oxoglutarato/malato', sub: TMM, rank: 15, lane: 2 }),
    r('ACALDtm', 'acald_c <=> acald_m', { name: 'Acetaldehyde diffusion (mitochondrion)', sub: TMM, rank: 11, lane: -1 }),
    r('ETOHtm', 'etoh_c <=> etoh_m', { name: 'Ethanol diffusion (mitochondrion)', sub: TMM, rank: 13, lane: -1 }),

    // Mitochondrial matrix
    r('PDHm', 'coa_m + nad_m + pyr_m --> accoa_m + co2_m + nadh_m', { name: 'Pyruvate dehydrogenase complex', sub: 'Citric acid cycle', rank: 11, lane: 0 }),
    ...tca({ comp: 'm', lane: 0, rank0: 12, suffix: 'm', icdhCofactor: 'NAD', quinone: 'q6' }),

    // Respiratory chain across the inner membrane (m -> im)
    // Ethanol/acetaldehyde redox shuttle: the route that exports matrix NADH to
    // the cytosol when the respiratory chain is off (Bakker et al., 2001).
    r('ALCD2xm', 'etoh_m + nad_m <=> acald_m + h_m + nadh_m', { name: 'Mitochondrial alcohol dehydrogenase (Adh3)', sub: OXP, rank: 12, lane: -1 }),
    r('NADH2_u6m', 'nadh_m + q6_m + h_m --> nad_m + q6h2_m', { name: 'Internal NADH dehydrogenase Ndi1 (does not pump H+)', sub: OXP, rank: 4, lane: 3 }),
    r('NADH2_u6cm', 'nadh_c + q6_m + h_c --> nad_c + q6h2_m', { name: 'External NADH dehydrogenase Nde1/Nde2', sub: OXP, rank: 4, lane: 2 }),
    r('CYOO_u6m', 'q6h2_m + 0.5 o2_m + 4.0 h_m --> q6_m + h2o_m + 4.0 h_im', { name: 'bc1 + cytochrome c oxidase (pumps H+ into the intermembrane space)', sub: OXP, rank: 5, lane: 3 }),
    r('ATPS4m', 'adp_m + pi_m + 3.0 h_im --> atp_m + h2o_m + 2.0 h_m', { name: 'Mitochondrial ATP synthase', sub: OXP, rank: 6, lane: 3 }),
    r('Htm', 'h_im <=> h_c', { name: 'Outer membrane pores (VDAC)', sub: TMM, rank: 7, lane: 3 }),
    r('ATPMm', 'atp_m + h2o_m --> adp_m + h_m + pi_m', { name: 'Mitochondrial maintenance', sub: OXP, lb: 0, ub: 1000, rank: 8, lane: 3 }),

    // Exchanges
    ex('glc__D', { lb: -10, rank: 0, lane: 0 }),
    ex('o2', { lb: -1000, rank: 4, lane: 6 }),
    ex('co2', { lb: -1000, rank: 12, lane: 5 }),
    ex('nh4', { lb: -1000, rank: 16, lane: -5 }),
    ex('pi', { lb: -1000, rank: 3, lane: 7 }),
    ex('h2o', { lb: -1000, rank: 5, lane: 7 }),
    ex('h', { lb: -1000, rank: 2, lane: 7 }),
    ex('etoh', { lb: 0, rank: 14, lane: -2 }),
    ex('ac', { lb: 0, rank: 13, lane: -1 }),
    ex('glyc', { lb: 0, rank: 6, lane: 3 }),
    ex('pyr', { lb: 0, rank: 10, lane: 2 }),

    // Biomass
    r('BIOMASS_Sc_core',
      '1.24 3pg_c + 2.91 accoa_c + 52.0 atp_c + 0.32 e4p_c + 0.15 f6p_c + 0.13 g3p_c '
      + '+ 0.30 g6p_c + 0.25 gln__L_c + 4.20 glu__L_c + 52.0 h2o_c + 3.10 nad_c '
      + '+ 11.50 nadph_c + 1.50 oaa_c + 0.52 pep_c + 2.50 pyr_c + 0.90 r5p_c '
      + '--> 52.0 adp_c + 3.60 akg_c + 2.91 coa_c + 52.0 h_c + 3.10 nadh_c '
      + '+ 11.50 nadp_c + 52.0 pi_c',
      { name: 'Biomass (yeast core, with GAM)', sub: 'Biomass', rank: 21, lane: 0 }),
  ],
};

export default () => buildModel(spec);
