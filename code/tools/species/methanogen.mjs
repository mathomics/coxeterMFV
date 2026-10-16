/**
 * Methanosarcina-like methanogenic archaeon: two compartments, two ways to make
 * methane.
 *
 *   acetoclastic    CH3COO- + H2O  ->  CH4 + HCO3-      (~0.5 ATP net)
 *   hydrogenotrophic 4 H2 + HCO3- + H+ -> CH4 + 3 H2O   (~1 ATP)
 *
 * The chemiosmotic detail (Ech, Rnf, methanophenazine) is lumped into the ATP
 * stoichiometry of the two methanogenesis reactions. The CODH/ACS assimilation
 * step is charged 1 ATP so that running it together with acetoclastic
 * methanogenesis cannot become a net ATP source.
 */
import { r, ex, buildModel } from '../lib/dsl.mjs';

const MET = 'Methanogenesis';
const ASM = 'Carbon assimilation';
const TRA = 'Membrane transport';

export const spec = {
  id: 'methanogen_core_mc',
  name: 'Methanosarcina sp. — methanogenic core',
  compartments: { e: 'Extracellular', c: 'Cytoplasm' },
  compartmentTree: { e: null, c: 'e' },
  cell: {
    name: 'Methanosarcina',
    organism: 'Methanosarcina barkeri',
    color: '#7c3aed',
    shape: 'round',
    gram: null,
    note: 'Strictly anaerobic archaeon. Consumes the acetate and H2/CO2 the bacteria ferment, and releases methane.',
  },
  objective: 'BIOMASS_Mb_core',
  primary: ['accoa', 'coa', 'ac', 'ch4', 'hco3'],
  reactions: [
    // Acetate activation
    r('ACKr', 'ac_c + atp_c <=> actp_c + adp_c', { name: 'Acetate kinase', sub: ASM, rank: 1, lane: 0 }),
    r('PTAr', 'accoa_c + pi_c <=> actp_c + coa_c', { name: 'Fosfotransacetilasa', sub: ASM, rank: 2, lane: 0 }),

    // Methanogenesis
    r('MCRac', 'accoa_c + 0.5 h2o_c + 1.5 adp_c + 1.5 pi_c + 0.5 h_c --> ch4_c + hco3_c + coa_c + 1.5 atp_c',
      { name: 'Acetoclastic methanogenesis (CODH/ACS + MCR, lumped energetics)', sub: MET, rank: 3, lane: 0 }),
    r('MCRh2', '4.0 h2_c + hco3_c + 2.0 h_c + adp_c + pi_c --> ch4_c + 4.0 h2o_c + atp_c',
      { name: 'Hydrogenotrophic methanogenesis (lumped energetics)', sub: MET, rank: 3, lane: 2 }),

    // Carbon assimilation
    r('CODH_ACS', '2.0 hco3_c + 4.0 h2_c + coa_c + atp_c + h_c --> accoa_c + adp_c + pi_c + 4.0 h2o_c',
      { name: 'Wood-Ljungdahl pathway (CODH/ACS): fixes CO2 into acetyl-CoA', sub: ASM, rank: 2, lane: 3 }),
    r('HCO3E', 'co2_c + h2o_c <=> h_c + hco3_c', { name: 'Carbonic anhydrase', sub: ASM, rank: 4, lane: 2 }),
    r('ATPM', 'atp_c + h2o_c --> adp_c + h_c + pi_c', { name: 'Non-growth-associated maintenance', sub: MET, lb: 1.0, ub: 1000, rank: 5, lane: 4 }),

    // Transport
    r('ACt', 'ac_e + h_e <=> ac_c + h_c', { name: 'Simporte acetato/H+', sub: TRA, rank: 0, lane: 0 }),
    r('CH4t', 'ch4_c <=> ch4_e', { name: 'Methane diffusion', sub: TRA, rank: 4, lane: 0 }),
    r('H2t', 'h2_e <=> h2_c', { name: 'H2 diffusion', sub: TRA, rank: 2, lane: 2 }),
    r('CO2t', 'co2_e <=> co2_c', { name: 'CO2 diffusion', sub: TRA, rank: 5, lane: 2 }),
    r('H2Ot', 'h2o_e <=> h2o_c', { name: 'Agua', sub: TRA, rank: 6, lane: 4 }),
    r('NH4t', 'nh4_e <=> nh4_c', { name: 'Amonio', sub: TRA, rank: 6, lane: -1 }),
    r('PIt', 'pi_e <=> pi_c', { name: 'Fosfato', sub: TRA, rank: 6, lane: 5 }),
    r('Ht', 'h_e <=> h_c', { name: 'Proton', sub: TRA, rank: 7, lane: 5 }),

    // Exchanges
    ex('ac', { lb: -10, rank: 0, lane: 0 }),
    ex('h2', { lb: -20, rank: 2, lane: 2 }),
    ex('co2', { lb: -1000, rank: 5, lane: 2 }),
    ex('ch4', { lb: 0, rank: 4, lane: 0 }),
    ex('h2o', { lb: -1000, rank: 6, lane: 4 }),
    ex('nh4', { lb: -1000, rank: 6, lane: -1 }),
    ex('pi', { lb: -1000, rank: 6, lane: 5 }),
    ex('h', { lb: -1000, rank: 7, lane: 5 }),

    // Biomass
    r('BIOMASS_Mb_core',
      '3.0 accoa_c + 30.0 atp_c + 2.0 nh4_c + 30.0 h2o_c '
      + '--> 30.0 adp_c + 3.0 coa_c + 30.0 pi_c + 32.0 h_c',
      { name: 'Biomass (methanogen core, lumped)', sub: 'Biomass', rank: 8, lane: 0 }),
  ],
};

export default () => buildModel(spec);
