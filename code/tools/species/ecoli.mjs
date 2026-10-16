/**
 * Escherichia coli core metabolism, Gram-negative envelope made explicit.
 *
 * Reactions and stoichiometries follow the e_coli_core model (Orth, Fleming &
 * Palsson, EcoSal Plus 2010), extended with the periplasm so that the outer
 * membrane (porins, `tex`) and the inner membrane (`pp` transporters) are two
 * separate boundaries on the map instead of a single lumped exchange.
 */
import { r, ex, buildModel } from '../lib/dsl.mjs';

const GLY = 'Glycolysis';
const PPP = 'Pentose phosphate pathway';
const TCA = 'Citric acid cycle';
const OXP = 'Oxidative phosphorylation';
const FER = 'Fermentation';
const ANA = 'Anaplerotic reactions';
const NIT = 'Nitrogen metabolism';
const TOM = 'Transport, outer membrane';
const TIM = 'Transport, inner membrane';

export const spec = {
  id: 'e_coli_core_mc',
  name: 'Escherichia coli — core with periplasm',
  compartments: { e: 'Extracellular', p: 'Periplasm', c: 'Cytosol' },
  compartmentTree: { e: null, p: 'e', c: 'p' },
  cell: {
    name: 'E. coli',
    organism: 'Escherichia coli K-12 MG1655',
    color: '#0284c7',
    shape: 'rod',
    gram: 'negative',
    note: 'Facultative heterotroph. Ferments to acetate, formate and ethanol when O2 runs short.',
  },
  objective: 'BIOMASS_Ec_core',
  primary: ['accoa', 'coa'],
  reactions: [
    // Glycolysis
    r('GLCptspp', 'glc__D_p + pep_c --> g6p_c + pyr_c', { name: 'Glucose transport via PEP:PTS (periplasm)', sub: TIM, rank: 0, lane: 0 }),
    r('PGI', 'g6p_c <=> f6p_c', { name: 'Glucose-6-phosphate isomerase', sub: GLY, rank: 1, lane: 0 }),
    r('PFK', 'atp_c + f6p_c --> adp_c + fdp_c + h_c', { name: 'Phosphofructokinase', sub: GLY, rank: 2, lane: 0 }),
    r('FBP', 'fdp_c + h2o_c --> f6p_c + pi_c', { name: 'Fructose-bisphosphatase', sub: GLY, rank: 2, lane: 1 }),
    r('FBA', 'fdp_c <=> dhap_c + g3p_c', { name: 'Fructose-bisphosphate aldolase', sub: GLY, rank: 3, lane: 0 }),
    r('TPI', 'dhap_c <=> g3p_c', { name: 'Triose-phosphate isomerase', sub: GLY, rank: 4, lane: -1 }),
    r('GAPD', 'g3p_c + nad_c + pi_c <=> 13dpg_c + h_c + nadh_c', { name: 'Glyceraldehyde-3-phosphate dehydrogenase', sub: GLY, rank: 5, lane: 0 }),
    r('PGK', '3pg_c + atp_c <=> 13dpg_c + adp_c', { name: 'Phosphoglycerate kinase', sub: GLY, rank: 6, lane: 0 }),
    r('PGM', '2pg_c <=> 3pg_c', { name: 'Phosphoglycerate mutase', sub: GLY, rank: 7, lane: 0 }),
    r('ENO', '2pg_c <=> h2o_c + pep_c', { name: 'Enolase', sub: GLY, rank: 8, lane: 0 }),
    r('PYK', 'adp_c + h_c + pep_c --> atp_c + pyr_c', { name: 'Pyruvate kinase', sub: GLY, rank: 9, lane: 0 }),

    // Pentose phosphate
    r('G6PDH2r', 'g6p_c + nadp_c <=> 6pgl_c + h_c + nadph_c', { name: 'Glucose 6-phosphate dehydrogenase', sub: PPP, rank: 1, lane: -3 }),
    r('PGL', '6pgl_c + h2o_c --> 6pgc_c + h_c', { name: '6-phosphogluconolactonase', sub: PPP, rank: 2, lane: -3 }),
    r('GND', '6pgc_c + nadp_c --> co2_c + nadph_c + ru5p__D_c', { name: 'Phosphogluconate dehydrogenase', sub: PPP, rank: 3, lane: -3 }),
    r('RPE', 'ru5p__D_c <=> xu5p__D_c', { name: 'Ribulose 5-phosphate 3-epimerase', sub: PPP, rank: 4, lane: -3 }),
    r('RPI', 'r5p_c <=> ru5p__D_c', { name: 'Ribose-5-phosphate isomerase', sub: PPP, rank: 4, lane: -4 }),
    r('TKT1', 'r5p_c + xu5p__D_c <=> g3p_c + s7p_c', { name: 'Transketolase I', sub: PPP, rank: 5, lane: -3 }),
    r('TALA', 'g3p_c + s7p_c <=> e4p_c + f6p_c', { name: 'Transaldolase', sub: PPP, rank: 6, lane: -3 }),
    r('TKT2', 'e4p_c + xu5p__D_c <=> f6p_c + g3p_c', { name: 'Transketolase II', sub: PPP, rank: 7, lane: -3 }),

    // Pyruvate node and TCA
    r('PDH', 'coa_c + nad_c + pyr_c --> accoa_c + co2_c + nadh_c', { name: 'Pyruvate dehydrogenase', sub: TCA, rank: 11, lane: 0 }),
    r('CS', 'accoa_c + h2o_c + oaa_c --> cit_c + coa_c + h_c', { name: 'Citrate synthase', sub: TCA, rank: 12, lane: 0 }),
    r('ACONT', 'cit_c <=> icit_c', { name: 'Aconitase', sub: TCA, rank: 13, lane: 0 }),
    r('ICDHyr', 'icit_c + nadp_c <=> akg_c + co2_c + nadph_c', { name: 'Isocitrate dehydrogenase (NADP)', sub: TCA, rank: 14, lane: 0 }),
    r('AKGDH', 'akg_c + coa_c + nad_c --> co2_c + nadh_c + succoa_c', { name: '2-Oxoglutarate dehydrogenase', sub: TCA, rank: 15, lane: 0 }),
    r('SUCOAS', 'atp_c + coa_c + succ_c <=> adp_c + pi_c + succoa_c', { name: 'Succinyl-CoA synthetase', sub: TCA, rank: 16, lane: 0 }),
    r('SUCDi', 'q8_c + succ_c --> fum_c + q8h2_c', { name: 'Succinate dehydrogenase', sub: TCA, rank: 17, lane: 0 }),
    r('FRD7', 'fum_c + q8h2_c --> q8_c + succ_c', { name: 'Fumarate reductase', sub: TCA, rank: 17, lane: 1 }),
    r('FUM', 'fum_c + h2o_c <=> mal__L_c', { name: 'Fumarase', sub: TCA, rank: 18, lane: 0 }),
    r('MDH', 'mal__L_c + nad_c <=> h_c + nadh_c + oaa_c', { name: 'Malate dehydrogenase', sub: TCA, rank: 19, lane: 0 }),

    // Anaplerotic
    r('PPC', 'co2_c + h2o_c + pep_c --> h_c + oaa_c + pi_c', { name: 'Phosphoenolpyruvate carboxylase', sub: ANA, rank: 10, lane: 2 }),
    r('PPCK', 'atp_c + oaa_c --> adp_c + co2_c + pep_c', { name: 'PEP carboxykinase', sub: ANA, rank: 10, lane: 3 }),
    r('ME1', 'mal__L_c + nad_c --> co2_c + nadh_c + pyr_c', { name: 'Malic enzyme (NAD)', sub: ANA, rank: 20, lane: 2 }),
    r('ME2', 'mal__L_c + nadp_c --> co2_c + nadph_c + pyr_c', { name: 'Malic enzyme (NADP)', sub: ANA, rank: 20, lane: 3 }),
    // e_coli_core omits PEP synthase and the glyoxylate shunt, which is why it
    // cannot grow on pyruvate, lactate, acetate or ethanol. Both are genuine
    // E. coli enzymes (ppsA, aceA/aceB) and both are required for the
    // cross-feeding this project is built around.
    r('PPS', 'atp_c + h2o_c + pyr_c --> amp_c + 2.0 h_c + pep_c + pi_c', { name: 'PEP synthase (gluconeogenesis from pyruvate)', sub: ANA, rank: 9, lane: 4 }),
    r('ADK1', 'amp_c + atp_c <=> 2.0 adp_c', { name: 'Adenylate kinase', sub: ANA, rank: 9, lane: 5 }),
    r('ICL', 'icit_c --> glx_c + succ_c', { name: 'Isocitrate lyase (glyoxylate shunt)', sub: ANA, rank: 14, lane: 2 }),
    r('MALS', 'accoa_c + glx_c + h2o_c --> coa_c + h_c + mal__L_c', { name: 'Malate synthase', sub: ANA, rank: 15, lane: 2 }),

    // Fermentation
    r('PFL', 'coa_c + pyr_c --> accoa_c + for_c', { name: 'Pyruvate formate lyase', sub: FER, rank: 11, lane: -2 }),
    r('LDH_D', 'lac__D_c + nad_c <=> h_c + nadh_c + pyr_c', { name: 'D-lactate dehydrogenase', sub: FER, rank: 10, lane: -2 }),
    r('ACALD', 'acald_c + coa_c + nad_c <=> accoa_c + h_c + nadh_c', { name: 'Acetaldehyde dehydrogenase (acetylating)', sub: FER, rank: 13, lane: -2 }),
    r('ALCD2x', 'etoh_c + nad_c <=> acald_c + h_c + nadh_c', { name: 'Alcohol dehydrogenase', sub: FER, rank: 14, lane: -2 }),
    r('PTAr', 'accoa_c + pi_c <=> actp_c + coa_c', { name: 'Phosphotransacetylase', sub: FER, rank: 13, lane: -1 }),
    // Formate hydrogen lyase: the anaerobic route that turns formate into the
    // H2 + CO2 a hydrogenotrophic methanogen lives on.
    r('FHL', 'for_c + h_c --> co2_c + h2_c', { name: 'Formate hydrogen lyase', sub: FER, rank: 12, lane: -4 }),
    // L-lactate is oxidised by the quinone-linked LldD, so E. coli can only use
    // the lactate another species secretes when a terminal oxidase is running.
    r('L_LACD2', 'lac__L_c + q8_c --> pyr_c + q8h2_c', { name: 'L-lactate dehydrogenase (LldD, quinone-linked)', sub: FER, rank: 9, lane: -3 }),
    r('ACKr', 'ac_c + atp_c <=> actp_c + adp_c', { name: 'Acetate kinase', sub: FER, rank: 14, lane: -1 }),

    // Oxidative phosphorylation
    r('NADH16pp', '4.0 h_c + nadh_c + q8_c --> 3.0 h_p + nad_c + q8h2_c', { name: 'NADH dehydrogenase (pumps H+ into the periplasm)', sub: OXP, rank: 4, lane: 5 }),
    r('CYTBD', '2.0 h_c + 0.5 o2_c + q8h2_c --> h2o_c + 2.0 h_p + q8_c', { name: 'Cytochrome bd oxidase', sub: OXP, rank: 5, lane: 5 }),
    r('ATPS4rpp', 'adp_c + 4.0 h_p + pi_c <=> atp_c + h2o_c + 3.0 h_c', { name: 'ATP synthase (proton-motive force)', sub: OXP, rank: 6, lane: 5 }),
    r('ATPM', 'atp_c + h2o_c --> adp_c + h_c + pi_c', { name: 'Non-growth-associated maintenance', sub: OXP, lb: 8.39, ub: 1000, rank: 7, lane: 5 }),
    r('NADTRHD', 'nad_c + nadph_c --> nadh_c + nadp_c', { name: 'Transhidrogenasa soluble', sub: OXP, rank: 8, lane: 5 }),
    r('THD2pp', '2.0 h_p + nadh_c + nadp_c --> 2.0 h_c + nad_c + nadph_c', { name: 'Membrane transhydrogenase', sub: OXP, rank: 9, lane: 5 }),

    // Nitrogen
    r('GLUDy', 'glu__L_c + h2o_c + nadp_c <=> akg_c + h_c + nadph_c + nh4_c', { name: 'Glutamate dehydrogenase (NADP)', sub: NIT, rank: 15, lane: -4 }),
    r('GLNS', 'atp_c + glu__L_c + nh4_c --> adp_c + gln__L_c + h_c + pi_c', { name: 'Glutamine synthetase', sub: NIT, rank: 16, lane: -4 }),

    // Outer membrane (porins)
    r('GLCtex', 'glc__D_e <=> glc__D_p', { name: 'Glucose via porin', sub: TOM, rank: 0, lane: 0 }),
    r('O2tex', 'o2_e <=> o2_p', { name: 'O2 via porin', sub: TOM, rank: 4, lane: 5 }),
    r('CO2tex', 'co2_e <=> co2_p', { name: 'CO2 via porin', sub: TOM, rank: 12, lane: 4 }),
    r('ACtex', 'ac_e <=> ac_p', { name: 'Acetate via porin', sub: TOM, rank: 15, lane: -1 }),
    r('ETOHtex', 'etoh_e <=> etoh_p', { name: 'Ethanol via porin', sub: TOM, rank: 15, lane: -2 }),
    r('FORtex', 'for_e <=> for_p', { name: 'Formate via porin', sub: TOM, rank: 12, lane: -2 }),
    r('D_LACtex', 'lac__D_e <=> lac__D_p', { name: 'D-lactate via porin', sub: TOM, rank: 9, lane: -2 }),
    r('L_LACtex', 'lac__L_e <=> lac__L_p', { name: 'L-lactate via porin', sub: TOM, rank: 9, lane: -3 }),
    r('H2tex', 'h2_e <=> h2_p', { name: 'H2 via porin', sub: TOM, rank: 12, lane: -4 }),
    r('SUCCtex', 'succ_e <=> succ_p', { name: 'Succinate via porin', sub: TOM, rank: 18, lane: 1 }),
    r('NH4tex', 'nh4_e <=> nh4_p', { name: 'Ammonium via porin', sub: TOM, rank: 16, lane: -5 }),
    r('PItex', 'pi_e <=> pi_p', { name: 'Phosphate via porin', sub: TOM, rank: 6, lane: 6 }),
    r('H2Otex', 'h2o_e <=> h2o_p', { name: 'Water via porin', sub: TOM, rank: 7, lane: 6 }),
    r('Htex', 'h_e <=> h_p', { name: 'Proton via porin', sub: TOM, rank: 5, lane: 6 }),

    // Inner membrane
    r('O2tpp', 'o2_p <=> o2_c', { name: 'O2 diffusion (inner membrane)', sub: TIM, rank: 4, lane: 4 }),
    r('CO2tpp', 'co2_p <=> co2_c', { name: 'CO2 diffusion (inner membrane)', sub: TIM, rank: 12, lane: 3 }),
    r('H2Otpp', 'h2o_p <=> h2o_c', { name: 'Water diffusion (inner membrane)', sub: TIM, rank: 7, lane: 4 }),
    r('NH4tpp', 'nh4_p <=> nh4_c', { name: 'Ammonium (inner membrane)', sub: TIM, rank: 16, lane: -5 }),
    r('PIt2rpp', 'h_p + pi_p <=> h_c + pi_c', { name: 'Simporte fosfato/H+', sub: TIM, rank: 6, lane: 4 }),
    r('ACt2rpp', 'ac_p + h_p <=> ac_c + h_c', { name: 'Simporte acetato/H+', sub: TIM, rank: 15, lane: -1 }),
    r('ETOHtrpp', 'etoh_p <=> etoh_c', { name: 'Ethanol facilitated diffusion', sub: TIM, rank: 15, lane: -2 }),
    r('FORt2pp', 'for_p + h_p --> for_c + h_c', { name: 'Formiato simporte (entrada)', sub: TIM, rank: 12, lane: -2 }),
    r('FORtppi', 'for_c --> for_p', { name: 'Formiato canal (salida)', sub: TIM, rank: 12, lane: -3 }),
    r('D_LACt2pp', 'h_p + lac__D_p <=> h_c + lac__D_c', { name: 'Simporte D-lactato/H+', sub: TIM, rank: 9, lane: -2 }),
    r('L_LACt2pp', 'h_p + lac__L_p <=> h_c + lac__L_c', { name: 'Simporte L-lactato/H+', sub: TIM, rank: 9, lane: -3 }),
    r('H2tpp', 'h2_p <=> h2_c', { name: 'H2 diffusion', sub: TIM, rank: 12, lane: -4 }),
    r('SUCCt2rpp', 'h_p + succ_p <=> h_c + succ_c', { name: 'Simporte succinato/H+', sub: TIM, rank: 18, lane: 1 }),

    // Exchanges
    // Uptake is only opened for the substrates this organism is meant to live on.
    // Fermentation products keep lb = 0 here. The community layer re-opens them
    // when another species actually secretes them into the shared medium.
    ex('glc__D', { lb: -10, rank: 0, lane: 0 }),
    ex('o2', { lb: -1000, rank: 4, lane: 5 }),
    ex('co2', { lb: -1000, rank: 12, lane: 4 }),
    ex('nh4', { lb: -1000, rank: 16, lane: -5 }),
    ex('pi', { lb: -1000, rank: 6, lane: 6 }),
    ex('h2o', { lb: -1000, rank: 7, lane: 6 }),
    ex('h', { lb: -1000, rank: 5, lane: 6 }),
    ex('ac', { lb: 0, rank: 15, lane: -1 }),
    ex('etoh', { lb: 0, rank: 15, lane: -2 }),
    ex('for', { lb: 0, rank: 12, lane: -2 }),
    ex('lac__D', { lb: 0, rank: 9, lane: -2 }),
    ex('lac__L', { lb: 0, rank: 9, lane: -3 }),
    ex('h2', { lb: 0, rank: 12, lane: -4 }),
    ex('succ', { lb: 0, rank: 18, lane: 1 }),

    // Biomass
    r('BIOMASS_Ec_core',
      '1.496 3pg_c + 3.7478 accoa_c + 59.81 atp_c + 0.361 e4p_c + 0.0709 f6p_c + 0.129 g3p_c '
      + '+ 0.205 g6p_c + 0.2557 gln__L_c + 4.9414 glu__L_c + 59.81 h2o_c + 3.547 nad_c '
      + '+ 13.0279 nadph_c + 1.7867 oaa_c + 0.5191 pep_c + 2.8328 pyr_c + 0.8977 r5p_c '
      + '--> 59.81 adp_c + 4.1182 akg_c + 3.7478 coa_c + 59.81 h_c + 3.547 nadh_c '
      + '+ 13.0279 nadp_c + 59.81 pi_c',
      { name: 'Biomass (E. coli core, with GAM)', sub: 'Biomass', rank: 21, lane: 0 }),
  ],
};

export default () => buildModel(spec);
