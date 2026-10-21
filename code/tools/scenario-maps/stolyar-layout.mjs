/**
 * Layout of the Stolyar et al. (2007) map, read by build-stolyar.mjs.
 *
 * Positions are absolute map units. Per species:
 *   nodes   primary metabolites (local id: [x, y] or [x, y, label angle in degrees])
 *   rings   cycles drawn on a circle: centre, radius and the angle of each metabolite
 *   rx      every reaction, placed one of these ways:
 *             line: [a, b]   on the line between two primary nodes, halfway
 *                            (t: share of the way, off: to the right of a -> b on
 *                            the page, for a parallel reaction drawn as an arc;
 *                            ring: on that ring between a and b; track: on a
 *                            concentric circle that much further out)
 *             at: [x, y], dir   at a point, axis along dir (substrate to product)
 *             mem: [edge, pos]  on the cytosol membrane, `pos` along that edge
 *             ex: [edge, pos]   on the cell's outer boundary (the exchange with the medium)
 *           prim lists the other metabolites drawn to their primary node; every
 *           other metabolite gets a small copy beside the reaction, on `side`
 *           (compass point; 'out' or 'in' on a ring), or in a block (grid), or
 *           where `sat` puts it.
 *   medium  shared-medium nodes and the stub of their community exchange
 */

const W = 5200, H = 2800;

// Rows where a metabolite crosses the gap between the cells, the same in both
// cells so each transfer is one straight line through the medium.
const Y = { nh3: 520, n2: 650, form: 877, h2: 1040, ch4: 1203, ac: 1320, etoh: 1500 };

export const LAYOUT = {
  canvas: { width: W, height: H },
  environment: { x: 20, y: 110, w: W - 40, h: H - 130 },
  cells: {
    // The bands facing the other cell are wider: a transporter's label is written
    // to the right of its marker, and there it needs room before the next node.
    Dv: { name: 'D. vulgaris Hildenborough', e: { x: 80, y: 230, w: 2350, h: 2470 }, band: { left: 64, right: 134, top: 64, bottom: 64 } },
    Mm: { name: 'M. maripaludis S2', e: { x: 2650, y: 230, w: 2470, h: 2470 }, band: { left: 214, right: 64, top: 64, bottom: 64 } },
  },

  labels: {
    common: {
      ATP: 'ATP', ADP: 'ADP', NADH: 'NADH', NADPH: 'NADPH', 'H+': 'H⁺', CO2: 'CO₂', NH3: 'NH₃', 'NH3(ext)': 'NH₃',
      '3PG': '3-phosphoglycerate', ALC: 'acetolactate', ASA: 'Asp-semialdehyde', AcCoA: 'acetyl-CoA',
      'AcOH(exch)': 'acetate', Ala: 'alanine', Arg: 'arginine', Asn: 'asparagine', Asp: 'aspartate',
      CHR: 'chorismate', Cys: 'cysteine', DDP: 'dihydrodipicolinate', DNA: 'DNA', E4P: 'erythrose-4-P',
      F6P: 'fructose-6-P', FBP: 'fructose-1,6-P₂', 'Fdx(red)': 'ferredoxin (red)', G6P: 'glucose-6-P',
      GAP: 'glyceraldehyde-3-P', Gln: 'glutamine', Glu: 'glutamate', Gly: 'glycine', His: 'histidine',
      Hse: 'homoserine', Ile: 'isoleucine', Ind: 'indole', Leu: 'leucine', Lys: 'lysine', Mal: 'malate',
      Met: 'methionine', OAA: 'oxaloacetate', PEP: 'phosphoenolpyruvate', PPA: 'prephenate', PROT: 'protein',
      PRPP: 'PRPP', Phe: 'phenylalanine', Pro: 'proline', Pyr: 'pyruvate', R5P: 'ribose-5-P', RNA: 'RNA',
      SDAP: 'N-succinyl-DAP', SKA: 'shikimate', Sed7P: 'sedoheptulose-7-P', Ser: 'serine', Suc: 'succinate',
      SucCoA: 'succinyl-CoA', THDP: 'tetrahydrodipicolinate', Thr: 'threonine', Trp: 'tryptophan',
      Tyr: 'tyrosine', Val: 'valine', X5P: 'xylulose-5-P', aIVA: '2-oxoisovalerate', aKG: '2-oxoglutarate',
      fum: 'fumarate', mDAP: 'meso-DAP', PLIP: 'phospholipid', 'Form(exch)': 'formate', 'H2(exch)': 'H₂',
      H2: 'H₂', Form: 'formate',
    },
    Dv: {
      APS: 'APS', 'C3(red)': 'cytochrome c3 (red)', Cit: 'citrate', EtOH: 'ethanol', FADH: 'FADH₂',
      Glyc: 'glycogen', Glyox: 'glyoxylate', H2S: 'H₂S', Isocit: 'isocitrate', LPS: 'lipopolysaccharide',
      Lac: 'lactate', PEPTID: 'peptidoglycan', QH2: 'quinol (QH₂)', Ribu5P: 'ribulose-5-P', SO3: 'sulfite',
      SO4: 'sulfate', glycerol: 'glycerol', 'glycerol-P': 'glycerol-3-P', mTHF: 'methyl-THF', AcP: 'acetyl-P',
    },
    Mm: {
      'ZM_Ac-CODH': 'acetyl-CODH', ZM_BIOMASS: 'biomass', 'ZM_CH3-B12': 'methyl-B₁₂', 'ZM_CH3-CoM': 'methyl-CoM',
      'ZM_CH3-H4MPT': 'methyl-H₄MPT', ZM_CH4: 'CH₄', 'ZM_CO-CODH': 'CO-CODH', 'ZM_CoM-SH': 'HS-CoM',
      'ZM_F420(red)': 'F₄₂₀H₂', 'ZM_Formyl-H4MPT': 'formyl-H₄MPT', 'ZM_Formyl-MFR': 'formyl-MFR', ZM_G1P: 'glucose-1-P',
      'ZM_Me-H4MPT': 'methylene-H₄MPT', 'ZM_Mn-H4MPT': 'methenyl-H₄MPT', ZM_N2: 'N₂', ZM_glyc: 'glycogen',
      ZM_DNA: 'DNA', ZM_RNA: 'RNA', ZM_PLIP: 'phospholipid', ZM_PROT: 'protein',
    },
  },

  species: {
    // D. vulgaris: lactate comes in from the right to pyruvate, which stands on
    // the TCA ring; acetyl-CoA and acetate go down the column beside the membrane
    // it shares with the methanogen, so acetate, formate and hydrogen leave the
    // cell close to where they are made.
    Dv: {
      nodes: {
        Lac: [2200, 800], AcCoA: [2000, 1100], AcP: [2000, Y.ac],
        Form: [2130, Y.form], H2: [2130, Y.h2], 'Fdx(red)': [2130, 1180], NH3: [2150, Y.nh3],
        'AcOH(exch)': [2390, Y.ac], 'Form(exch)': [2390, Y.form], 'H2(exch)': [2390, Y.h2], 'NH3(ext)': [2390, Y.nh3],
        EtOH: [2390, Y.etoh],
        // Gluconeogenesis along the top, from PEP to glucose-6-P
        PEP: [1990, 470], '3PG': [1730, 470], GAP: [1580, 470], FBP: [1430, 470], F6P: [1280, 470], G6P: [1130, 470],
        'glycerol-P': [1580, 370], glycerol: [1580, 262],
        // Pentose phosphates
        Ribu5P: [980, 470], R5P: [830, 470], PRPP: [680, 470], X5P: [980, 620], Sed7P: [830, 770], E4P: [830, 920],
        // Glutamate family
        Glu: [1300, 980], Gln: [1150, 980], Pro: [1300, 1130], His: [1150, 840], Arg: [1150, 1130],
        // Aspartate family
        Asp: [1700, 1250], Asn: [1550, 1250], ASA: [1700, 1400], Hse: [1700, 1550], Thr: [1700, 1700], Ile: [1700, 1850],
        Met: [1850, 1550], DDP: [1550, 1400], THDP: [1400, 1400], SDAP: [1250, 1400], mDAP: [1100, 1400], Lys: [950, 1400],
        // Aromatics
        SKA: [680, 920], CHR: [530, 920], PPA: [380, 920], Tyr: [230, 920], Phe: [380, 1070], Ind: [530, 1070], Trp: [680, 1070],
        // Serine family and C1
        Ser: [380, 1400], Gly: [530, 1400], mTHF: [530, 1270], Cys: [230, 1530],
        // Pyruvate family
        ALC: [830, 1580], aIVA: [980, 1580], Val: [1130, 1580], Leu: [980, 1720], Ala: [1280, 1580],
        // Sulfate respiration and electron carriers, along the bottom
        SO4: [1575, 2668], APS: [1575, 2490], SO3: [1825, 2490], H2S: [1825, 2668],
        'C3(red)': [1700, 2350], QH2: [1575, 2350], FADH: [1450, 2350],
        // Energy, beside the sulfate respiration that makes the gradient
        ATP: [2130, 2240], ADP: [2130, 2440],
        // Macromolecules in one column, each fed by its block from the left,
        // biomass to their right
        DNA: [1200, 1850], PROT: [1200, 1960], PEPTID: [1200, 2080], PLIP: [1200, 2220], Glyc: [1200, 2320],
        RNA: [1200, 2410], LPS: [1200, 2530],
      },
      rings: {
        // Closed through pyruvate (malic enzyme and pyruvate carboxylase), which
        // stands on its rightmost point; its cofactors stand inside, leaving the
        // outside to the lines that leave pyruvate.
        tca: {
          c: [1680, 800], r: 200, labelsOut: true, label: { Pyr: -40, aKG: -160 },
          nodes: { Pyr: 0, OAA: 40, Cit: 80, Isocit: 120, aKG: 160, SucCoA: 200, Suc: 240, fum: 280, Mal: 320 },
        },
      },
      rx: {
        // Lactate to acetate, formate and hydrogen
        D86: { at: [2200, 710], dir: 'S', prim: ['Lac'], side: 'E' },
        D12: { line: ['Lac', 'Pyr'], side: 'S', rows: { all: [[18, 42]] } },
        D13: { line: ['Lac', 'Pyr'], off: 40, side: 'N', rows: { all: [[-12, 42]] } },
        // Pyruvate-formate lyase bows out high, so formate leaves it level with
        // its node; the dehydrogenase's cofactors stand below that branch.
        D14: { line: ['Pyr', 'AcCoA'], side: 'W', rows: { all: [[54, 40], [20, 40]] } },
        D77: { line: ['Pyr', 'AcCoA'], off: -40, t: 0.35, prim: ['Form'] },
        D30: { line: ['AcCoA', 'AcP'] },
        // Acetate kinase and its blocked reverse side by side, their cofactors on
        // opposite sides so that neither crosses the other.
        D31: { mem: ['right', Y.ac], prim: ['AcP', 'AcOH(exch)'], hands: { in: 1 } },
        D33: { mem: ['right', Y.ac + 60], prim: ['AcOH(exch)'], hands: { in: -1 } },
        D78: { mem: ['right', Y.etoh], prim: ['EtOH'], hands: { in: -1 } },
        D80: { mem: ['right', Y.form], prim: ['Form', 'Form(exch)'] },
        D79: { at: [2130, 950], dir: 'S', prim: ['Form'], side: 'E' },
        D81: { mem: ['right', Y.h2], prim: ['H2(exch)', 'H2'] },
        D04: { line: ['H2', 'Fdx(red)'] },
        D84: { mem: ['right', Y.nh3], prim: ['NH3(ext)', 'NH3'] },
        'EX_AcOH(exch)_Dv': { ex: ['right', Y.ac], prim: ['AcOH(exch)', 'M@AcOH(exch)'] },
        'EX_Form(exch)_Dv': { ex: ['right', Y.form], prim: ['Form(exch)', 'M@Form(exch)'] },
        'EX_H2(exch)_Dv': { ex: ['right', Y.h2], prim: ['H2(exch)', 'M@H2(exch)'] },
        'EX_NH3(ext)_Dv': { ex: ['right', Y.nh3], prim: ['NH3(ext)', 'M@NH3(ext)'] },
        EX_EtOH_Dv: { ex: ['right', Y.etoh], prim: ['EtOH', 'M@EtOH'] },
        EX_SO4_Dv: { ex: ['bottom', 1575], prim: ['SO4', 'M@SO4'] },
        EX_H2S_Dv: { ex: ['bottom', 1825], prim: ['H2S', 'M@H2S'] },
        EX_glycerol_Dv: { ex: ['top', 1580], prim: ['glycerol', 'M@glycerol'] },
        D_CO2: { at: [2230, 1250], dir: 'E', side: 'N' },
        // TCA, closed through pyruvate
        // The two steps through pyruvate keep their cofactors clear of its node.
        D29: { line: ['Pyr', 'OAA'], ring: 'tca', side: 'in', rows: { substrate: [[-20, 40], [-20, 68]] } },
        // Acetyl-CoA turns into citrate synthase from outside the ring, close to its marker.
        D32: { line: ['OAA', 'Cit'], ring: 'tca', side: 'in', prim: ['AcCoA'], curve: { AcCoA: [1920, 1100, 1810, 956] } },
        D34: { line: ['Cit', 'Isocit'], ring: 'tca', side: 'in' },
        D35: { line: ['Isocit', 'aKG'], ring: 'tca', side: 'in' },
        D87: { line: ['aKG', 'SucCoA'], ring: 'tca', side: 'in' },
        D36: { line: ['SucCoA', 'Suc'], ring: 'tca', side: 'in' },
        D37: { line: ['Suc', 'fum'], ring: 'tca', side: 'in' },
        D38: { line: ['fum', 'Mal'], ring: 'tca', side: 'in' },
        D39: { line: ['Mal', 'Pyr'], ring: 'tca', side: 'in', rows: { product: [[-20, 40], [-20, 68]] } },
        D76: { at: [1830, 570], dir: 'S', prim: ['Mal'], side: 'E' },
        // Gluconeogenesis and glycerol
        // The ring arcs away from the PEP line on its left, so the cofactors of
        // both PEP reactions stand to its right or up by PEP.
        D28: { line: ['Pyr', 'PEP'], side: 'W', rows: { all: [[-40, 40]] } },
        D27: { line: ['PEP', 'Pyr'], off: -40, side: 'E' },
        D26: { line: ['3PG', 'PEP'] },
        D25: { line: ['GAP', '3PG'], side: 'S' },
        D19: { line: ['FBP', 'GAP'], side: 'S' },
        D18: { line: ['FBP', 'F6P'], side: 'N' },
        D17: { line: ['F6P', 'FBP'], off: 40, side: 'S' },
        D15: { line: ['G6P', 'F6P'] },
        D82: { line: ['GAP', 'glycerol-P'], side: 'W', rows: { all: [[10, 40]] } },
        D83: { mem: ['top', 1580], prim: ['glycerol-P', 'glycerol'] },
        // Pentose phosphates
        D16: { line: ['G6P', 'Ribu5P'], side: 'N', rows: { all: [[20, 40]] } },
        D23: { line: ['Ribu5P', 'R5P'] },
        D24: { line: ['Ribu5P', 'X5P'] },
        D70: { line: ['R5P', 'PRPP'], side: 'N' },
        // Transketolase low on its line, so xylulose-5-P turns into it in one bend;
        // the second one below xylulose-5-P, erythrose-4-P turning down into it.
        D20: { line: ['R5P', 'Sed7P'], t: 0.75, prim: ['X5P'], side: 'W' },
        D21: { line: ['Sed7P', 'E4P'], side: 'W' },
        D22: { at: [980, 1000], dir: 'S', prim: ['X5P', 'E4P'], side: 'W', curve: { E4P: [920, 920, 962, 962] } },
        // Glutamate family
        // The blocked glutamate synthase bows below, clear of 2-oxoglutarate's name.
        D40: { line: ['aKG', 'Glu'], side: 'N', rows: { all: [[-30, 40], [-30, 68]] } },
        D41: { line: ['aKG', 'Glu'], off: -40, side: 'S' },
        D42: { line: ['Glu', 'Gln'], side: 'N' },
        D51: { line: ['Glu', 'Pro'], side: 'E' },
        D71: { line: ['Gln', 'His'], side: 'N' },
        D52: { line: ['Gln', 'Arg'], side: 'W', rows: { substrate: [[50, 40], [30, 68], [10, 96]], product: [[50, 40], [30, 68]] } },
        // Aspartate family
        D43: { at: [1700, 1170], dir: 'S', prim: ['Asp'], side: 'E' },
        D44: { line: ['Asp', 'Asn'], side: 'N' },
        D45: { line: ['Asp', 'ASA'], side: 'E' },
        D53: { line: ['ASA', 'Hse'], side: 'E' },
        D74: { line: ['Hse', 'Thr'], side: 'W' },
        D54: { line: ['Thr', 'Ile'], side: 'E' },
        D75: { line: ['Hse', 'Met'], side: 'S' },
        D46: { line: ['ASA', 'DDP'], side: 'S' },
        D47: { line: ['DDP', 'THDP'], side: 'S' },
        D48: { line: ['THDP', 'SDAP'], side: 'S' },
        D49: { line: ['SDAP', 'mDAP'], side: 'S' },
        D50: { line: ['mDAP', 'Lys'], side: 'S' },
        // Aromatics
        D58: { line: ['E4P', 'SKA'], side: 'S' },
        D59: { line: ['SKA', 'CHR'], side: 'N' },
        D60: { line: ['CHR', 'PPA'], side: 'N' },
        D61: { line: ['PPA', 'Tyr'], side: 'N' },
        D62: { line: ['PPA', 'Phe'], side: 'W' },
        D63: { line: ['CHR', 'Ind'], side: 'E' },
        D64: { line: ['Ind', 'Trp'], side: 'S' },
        // Serine family, C1 and cysteine
        D73: { at: [380, 1320], dir: 'S', prim: ['Ser'], side: 'W' },
        D55: { line: ['Ser', 'Gly'], prim: ['mTHF'], side: 'S' },
        D57: { line: ['Gly', 'mTHF'], side: 'E' },
        D56: { at: [530, 1190], dir: 'N', prim: ['mTHF'], side: 'E' },
        D72: { mem: ['left', 1450], prim: ['Ser', 'Cys'] },
        // Pyruvate family
        D65: { at: [830, 1500], dir: 'S', prim: ['ALC'], side: 'W' },
        D66: { line: ['ALC', 'aIVA'], side: 'N' },
        D67: { line: ['aIVA', 'Val'], side: 'N' },
        D69: { line: ['aIVA', 'Leu'], side: 'W' },
        D68: { line: ['Val', 'Ala'], side: 'N' },
        // Sulfate respiration and the carriers that feed it
        D01: { mem: ['bottom', 1575], prim: ['SO4', 'APS'] },
        D02: { line: ['APS', 'SO3'], prim: ['C3(red)'] },
        D05: { mem: ['bottom', 1725], prim: ['SO3', 'H2S'], sat: { NADH: [1690, 2580], 'H+': [1700, 2550] } },
        D06: { mem: ['bottom', 1825], prim: ['SO3', 'H2S'], sat: { FADH: [1790, 2580], 'H+': [1860, 2580] } },
        D07: { mem: ['bottom', 1925], prim: ['SO3', 'H2S'], sat: { 'Fdx(red)': [1960, 2580], 'H+': [1950, 2550] } },
        D03: { at: [1700, 2270], dir: 'S', prim: ['C3(red)'], side: 'E' },
        D09: { line: ['QH2', 'C3(red)'] },
        D08: { line: ['FADH', 'QH2'] },
        // Energy
        D10: { line: ['ADP', 'ATP'], off: -45, side: 'W' },
        D11: { line: ['ATP', 'ADP'] },
        D_ATPase: { line: ['ATP', 'ADP'], off: -45, side: 'E' },
        // Biomass: each block's precursors in a column on its left. Protein and
        // RNA stand furthest out, their product lines passing between the others.
        PROT_syn: { at: [560, 1960], dir: 'E', prim: ['PROT'], grid: { side: 'substrate', x: 300, y: 1960 }, side: 'N' },
        RNA_syn: { at: [560, 2410], dir: 'E', prim: ['RNA'], grid: { side: 'substrate', x: 300, y: 2410 }, side: 'S' },
        DNA_syn: { at: [1020, 1850], dir: 'E', prim: ['DNA'], grid: { side: 'substrate', x: 800, y: 1850 }, side: 'N' },
        PEPTID_syn: { at: [1020, 2080], dir: 'E', prim: ['PEPTID'], grid: { side: 'substrate', x: 800, y: 2080 }, side: 'N' },
        PLIP_syn: { at: [1020, 2220], dir: 'E', prim: ['PLIP'], grid: { side: 'substrate', x: 800, y: 2220 }, side: 'S' },
        Glyc_syn: { at: [1020, 2320], dir: 'E', prim: ['Glyc'], side: 'S' },
        LPS_syn: { mem: ['bottom', 1200], prim: ['LPS'], grid: { side: 'substrate', x: 1010, y: 2540 } },
        mue: { at: [1400, 2190], dir: 'E', prim: ['PROT', 'RNA', 'DNA', 'PEPTID', 'LPS', 'PLIP', 'Glyc'], merge: true },
      },
    },

    // M. maripaludis: formate, hydrogen and acetate come in on the side facing
    // its partner. Formate and hydrogen go straight into the methanogenesis
    // ring, which opens towards that side, and methane leaves there too.
    Mm: {
      nodes: {
        'H2(exch)': [2832, Y.h2], 'Form(exch)': [2832, Y.form], 'AcOH(exch)': [2832, Y.ac], 'NH3(ext)': [2832, Y.nh3],
        ZM_CH4: [2832, Y.ch4], ZM_N2: [2832, Y.n2], ZM_BIOMASS: [5088, 960],
        ZM_Form: [3010, Y.form], ZM_H2: [3080, Y.h2], ZM_NH3: [3010, Y.nh3],
        // Carriers reduced by hydrogen, between the membrane and the ring
        'ZM_Fdx(red)': [3250, 955], 'ZM_F420(red)': [3250, 1040], 'ZM_CoM-SH': [3250, 1125],
        ZM_NADH: [3380, 985], ZM_NADPH: [3380, 1095],
        // Acetyl-CoA from methyl-H4MPT and CO (Wood-Ljungdahl), and from acetate
        'ZM_CH3-B12': [3640, 1410], 'ZM_Ac-CODH': [3420, 1410], 'ZM_CO-CODH': [3530, 1500], ZM_AcCoA: [3120, 1410],
        // Gluconeogenesis down the left
        ZM_Pyr: [3120, 1600], ZM_PEP: [3120, 1740], ZM_3PG: [3120, 1880], ZM_GAP: [3120, 2020], ZM_FBP: [3120, 2160],
        ZM_F6P: [3120, 2300], ZM_G6P: [2980, 2300], ZM_G1P: [2980, 2170],
        // Reductive branch of the TCA cycle, to 2-oxoglutarate and glutamate
        ZM_OAA: [3270, 1600], ZM_Mal: [3420, 1600], ZM_fum: [3570, 1600], ZM_Suc: [3720, 1600], ZM_SucCoA: [3870, 1600],
        ZM_aKG: [4020, 1600], ZM_Glu: [4170, 1600], ZM_Gln: [4320, 1600], ZM_Arg: [4470, 1600], ZM_Pro: [4170, 1460], ZM_His: [4320, 1460],
        // Aspartate family
        ZM_Asp: [3270, 1750], ZM_Asn: [3270, 1890], ZM_ASA: [3420, 1750], ZM_Hse: [3570, 1750], ZM_Thr: [3720, 1750],
        ZM_Met: [3570, 1890], ZM_DDP: [3420, 1890], ZM_THDP: [3420, 2030], ZM_SDAP: [3570, 2030], ZM_mDAP: [3720, 2030], ZM_Lys: [3870, 2030],
        // Aromatics
        ZM_SKA: [4020, 1750], ZM_CHR: [4170, 1750], ZM_PPA: [4320, 1750], ZM_Tyr: [4470, 1750], ZM_Phe: [4320, 1890],
        ZM_Ind: [4170, 1890], ZM_Trp: [4170, 2030],
        // Serine family
        ZM_Ser: [2980, 1880], ZM_Gly: [2980, 2020], ZM_Cys: [2950, 1760],
        // Pentose phosphates
        ZM_X5P: [3420, 2170], ZM_R5P: [3570, 2170], ZM_PRPP: [3720, 2170], ZM_Sed7P: [3495, 2340], ZM_E4P: [3345, 2340],
        // Pyruvate family, isoleucine and alanine
        ZM_ALC: [4020, 2190], ZM_aIVA: [4170, 2190], ZM_Val: [4320, 2190], ZM_Leu: [4170, 2320], ZM_Ala: [4470, 2190], ZM_Ile: [4020, 2320],
        // Energy
        ZM_ATP: [3300, 560], ZM_ADP: [3300, 740],
        // Macromolecules
        // Macromolecules in one column, biomass on the membrane to their right
        ZM_DNA: [4760, 560], ZM_PROT: [4760, 820], ZM_RNA: [4760, 1080], ZM_PLIP: [4760, 1250], ZM_glyc: [4760, 1370],
      },
      rings: {
        methanogenesis: {
          c: [3640, Y.h2], r: 230,
          nodes: { ZM_CO2: 225, 'ZM_Formyl-MFR': 270, 'ZM_Formyl-H4MPT': 315, 'ZM_Mn-H4MPT': 0, 'ZM_Me-H4MPT': 45, 'ZM_CH3-H4MPT': 90, 'ZM_CH3-CoM': 135 },
        },
      },
      rx: {
        // Substrates from the partner
        M15: { mem: ['left', Y.h2], prim: ['H2(exch)', 'ZM_H2'] },
        M20: { mem: ['left', Y.form], prim: ['Form(exch)', 'ZM_Form'] },
        M38: { mem: ['left', Y.ac], prim: ['AcOH(exch)', 'ZM_AcCoA'] },
        M78: { mem: ['left', Y.nh3], prim: ['NH3(ext)', 'ZM_NH3'] },
        M81: { mem: ['left', Y.n2], prim: ['ZM_N2', 'ZM_NH3'],
          sat: { ZM_ATP: [2920, 610], 'ZM_Fdx(red)': [2950, 640], ZM_ADP: [2920, 690], ZM_H2: [2950, 720] } },
        M74: { mem: ['left', 1820], prim: ['ZM_Ser', 'ZM_Cys'] },
        'EX_H2(exch)_Mm': { ex: ['left', Y.h2], prim: ['H2(exch)', 'M@H2(exch)'] },
        'EX_Form(exch)_Mm': { ex: ['left', Y.form], prim: ['Form(exch)', 'M@Form(exch)'] },
        'EX_AcOH(exch)_Mm': { ex: ['left', Y.ac], prim: ['AcOH(exch)', 'M@AcOH(exch)'] },
        'EX_NH3(ext)_Mm': { ex: ['left', Y.nh3], prim: ['NH3(ext)', 'M@NH3(ext)'] },
        EX_ZM_CH4_Mm: { ex: ['left', Y.ch4], prim: ['ZM_CH4', 'M@ZM_CH4'] },
        EX_ZM_N2_Mm: { ex: ['left', Y.n2], prim: ['ZM_N2', 'M@ZM_N2'] },
        // Methanogenesis
        M19: { line: ['ZM_Form', 'ZM_CO2'], side: 'N' },
        M_CO2: { at: [3437, 837], dir: 'NW', prim: ['ZM_CO2'] },
        M01: { line: ['ZM_CO2', 'ZM_Formyl-MFR'], ring: 'methanogenesis' },
        M03: { line: ['ZM_Formyl-MFR', 'ZM_Formyl-H4MPT'], ring: 'methanogenesis' },
        M04: { line: ['ZM_Formyl-H4MPT', 'ZM_Mn-H4MPT'], ring: 'methanogenesis' },
        M05: { line: ['ZM_Mn-H4MPT', 'ZM_Me-H4MPT'], ring: 'methanogenesis' },
        M06: { line: ['ZM_Mn-H4MPT', 'ZM_Me-H4MPT'], ring: 'methanogenesis', track: -45, side: 'in' },
        M07: { line: ['ZM_Me-H4MPT', 'ZM_CH3-H4MPT'], ring: 'methanogenesis' },
        M11: { line: ['ZM_CH3-H4MPT', 'ZM_CH3-CoM'], ring: 'methanogenesis' },
        M12: { mem: ['left', Y.ch4], prim: ['ZM_CH3-CoM', 'ZM_CH4'] },
        // Electron carriers from hydrogen
        M14: { line: ['ZM_H2', 'ZM_Fdx(red)'], side: 'N' },
        M13: { line: ['ZM_H2', 'ZM_CoM-SH'], side: 'S' },
        M16: { line: ['ZM_H2', 'ZM_F420(red)'] },
        M17: { line: ['ZM_F420(red)', 'ZM_NADH'] },
        M18: { line: ['ZM_F420(red)', 'ZM_NADPH'] },
        // Acetyl-CoA from methyl-H4MPT and CO
        M08: { line: ['ZM_CH3-H4MPT', 'ZM_CH3-B12'] },
        M09: { line: ['ZM_CH3-B12', 'ZM_Ac-CODH'], prim: ['ZM_CO-CODH'] },
        M02: { at: [3680, 1500], dir: 'W', prim: ['ZM_CO-CODH'], side: 'N' },
        M10: { line: ['ZM_Ac-CODH', 'ZM_AcCoA'] },
        // Gluconeogenesis
        M23: { line: ['ZM_AcCoA', 'ZM_Pyr'], side: 'W' },
        M36: { line: ['ZM_Pyr', 'ZM_PEP'], side: 'E' },
        M35: { line: ['ZM_PEP', 'ZM_Pyr'], off: -40, side: 'W' },
        M34: { line: ['ZM_3PG', 'ZM_PEP'] },
        M33: { line: ['ZM_GAP', 'ZM_3PG'], side: 'E' },
        M32: { line: ['ZM_GAP', 'ZM_3PG'], off: -40, side: 'W', rows: { all: [[18, 40], [0, 66]] } },
        M28: { line: ['ZM_FBP', 'ZM_GAP'], side: 'W' },
        M27: { line: ['ZM_FBP', 'ZM_F6P'], side: 'W' },
        M26: { line: ['ZM_F6P', 'ZM_FBP'], off: 40, side: 'E' },
        M24: { line: ['ZM_G6P', 'ZM_F6P'] },
        M25: { line: ['ZM_G6P', 'ZM_G1P'] },
        // Reductive TCA
        M37: { line: ['ZM_Pyr', 'ZM_OAA'], side: 'N' },
        M43: { line: ['ZM_OAA', 'ZM_Mal'], side: 'N' },
        M42: { line: ['ZM_Mal', 'ZM_fum'] },
        M41: { line: ['ZM_fum', 'ZM_Suc'], side: 'N' },
        M40: { line: ['ZM_Suc', 'ZM_SucCoA'], side: 'N' },
        M39: { line: ['ZM_SucCoA', 'ZM_aKG'], side: 'N' },
        M44: { line: ['ZM_aKG', 'ZM_Glu'], side: 'N' },
        M45: { line: ['ZM_Glu', 'ZM_Gln'], side: 'N' },
        M46: { line: ['ZM_OAA', 'ZM_Asp'], side: 'W' },
        // Pentose phosphates
        M77: { line: ['ZM_X5P', 'ZM_R5P'] },
        M71: { line: ['ZM_R5P', 'ZM_PRPP'], side: 'N' },
        M29: { at: [3495, 2250], dir: 'S', prim: ['ZM_X5P', 'ZM_R5P', 'ZM_Sed7P'], side: 'E' },
        M30: { line: ['ZM_Sed7P', 'ZM_E4P'], side: 'S' },
        M31: { at: [3330, 2255], dir: 'W', prim: ['ZM_X5P', 'ZM_E4P'], side: 'S' },
        // Amino acids
        M47: { line: ['ZM_Asp', 'ZM_Asn'], side: 'W' },
        M48: { line: ['ZM_Asp', 'ZM_ASA'], side: 'N' },
        M55: { line: ['ZM_ASA', 'ZM_Hse'], side: 'N' },
        M56: { line: ['ZM_Hse', 'ZM_Thr'], side: 'N' },
        M76: { line: ['ZM_Hse', 'ZM_Met'], side: 'E' },
        M49: { line: ['ZM_ASA', 'ZM_DDP'], side: 'W' },
        M50: { line: ['ZM_DDP', 'ZM_THDP'], side: 'W' },
        M51: { line: ['ZM_THDP', 'ZM_SDAP'], side: 'S' },
        M52: { line: ['ZM_SDAP', 'ZM_mDAP'], side: 'S' },
        M53: { line: ['ZM_mDAP', 'ZM_Lys'], side: 'S' },
        M75: { line: ['ZM_Glu', 'ZM_Pro'], side: 'W' },
        M72: { line: ['ZM_Gln', 'ZM_His'], side: 'W' },
        M54: { line: ['ZM_Gln', 'ZM_Arg'], side: 'N' },
        M58: { line: ['ZM_3PG', 'ZM_Ser'] },
        M73: { line: ['ZM_Ser', 'ZM_Gly'], side: 'W' },
        M59: { at: [3945, 1750], dir: 'E', prim: ['ZM_SKA'], side: 'S' },
        M60: { line: ['ZM_SKA', 'ZM_CHR'], side: 'N' },
        M61: { line: ['ZM_CHR', 'ZM_PPA'] },
        M62: { line: ['ZM_PPA', 'ZM_Tyr'], side: 'N' },
        M63: { line: ['ZM_PPA', 'ZM_Phe'], side: 'E' },
        M64: { line: ['ZM_CHR', 'ZM_Ind'], side: 'W' },
        M65: { line: ['ZM_Ind', 'ZM_Trp'], side: 'W' },
        M66: { at: [3945, 2190], dir: 'E', prim: ['ZM_ALC'], side: 'N' },
        M67: { line: ['ZM_ALC', 'ZM_aIVA'], side: 'N' },
        M68: { line: ['ZM_aIVA', 'ZM_Val'], side: 'N' },
        M70: { line: ['ZM_aIVA', 'ZM_Leu'], side: 'E' },
        M69: { line: ['ZM_Val', 'ZM_Ala'], side: 'N' },
        M57: { at: [3945, 2320], dir: 'E', prim: ['ZM_Ile'], side: 'N' },
        M80: { at: [4590, 2190], dir: 'E', prim: ['ZM_Ala'], side: 'N' },
        M79: { at: [4470, 2290], dir: 'N', prim: ['ZM_Ala'] },
        // Energy
        M21: { line: ['ZM_ADP', 'ZM_ATP'], off: -45, side: 'W' },
        M22: { line: ['ZM_ATP', 'ZM_ADP'] },
        M_ATPase: { line: ['ZM_ATP', 'ZM_ADP'], off: -45, side: 'E' },
        // Biomass
        // Each block's precursors in a column on its left; protein stands
        // furthest out, its product line passing between the others.
        MC_PROT: { at: [4360, 820], dir: 'E', prim: ['ZM_PROT'], grid: { side: 'substrate', x: 4100, y: 820 }, side: 'N' },
        MC_DNA: { at: [4620, 560], dir: 'E', prim: ['ZM_DNA'], grid: { side: 'substrate', x: 4440, y: 560 }, side: 'S' },
        MC_RNA: { at: [4620, 1080], dir: 'E', prim: ['ZM_RNA'], grid: { side: 'substrate', x: 4440, y: 1080 }, side: 'N' },
        MC_PLIP: { at: [4620, 1250], dir: 'E', prim: ['ZM_PLIP'], grid: { side: 'substrate', x: 4440, y: 1250 }, side: 'S' },
        MC_glyc: { at: [4620, 1370], dir: 'E', prim: ['ZM_glyc'], side: 'S' },
        MC_BIO: { mem: ['right', 960], prim: ['ZM_PROT', 'ZM_RNA', 'ZM_DNA', 'ZM_PLIP', 'ZM_glyc', 'ZM_BIOMASS'], merge: true },
        EX_ZM_BIOMASS_Mm: { at: [5088, 1050], dir: 'S', prim: ['ZM_BIOMASS'] },
      },
    },
  },

  // Shared medium: the gap between the cells, the band above them and the one below
  medium: {
    'H2(exch)': { at: [2605, Y.h2], stub: 'N', keepId: true },
    'Form(exch)': { at: [2605, Y.form], stub: 'N', keepId: true },
    'AcOH(exch)': { at: [2605, Y.ac], stub: 'S', keepId: true },
    'NH3(ext)': { at: [2605, Y.nh3], stub: 'N' },
    EtOH: { at: [2560, Y.etoh], stub: 'S' },
    ZM_CH4: { at: [2560, Y.ch4], stub: 'S' },
    ZM_N2: { at: [2560, Y.n2], stub: 'S' },
    SO4: { at: [1575, 2740], stub: 'W' },
    H2S: { at: [1825, 2740], stub: 'E' },
    glycerol: { at: [1580, 175], stub: 'E' },
  },

  texts: [
    ['title', 'Stolyar et al. (2007) · D. vulgaris + M. maripaludis syntrophy', 40, 52, 28],
    ['scope', 'Both CellNetAnalyzer models drawn whole · lactate is oxidised to acetate, H₂ and formate, which the methanogen turns into methane', 40, 86, 17],
    // One title and one line of purpose per module
    ...module('Dv-lactate', 'Lactate oxidation', 'To acetate, formate and H₂', 1950, 380),
    ...module('Dv-gng', 'Gluconeogenesis', 'Pyruvate back to glucose-6-P; glycerol enters at GAP', 1080, 330),
    ...module('Dv-ppp', 'Pentose phosphates', 'Ribose-5-P, PRPP and erythrose-4-P', 600, 330),
    ...module('Dv-tca', 'TCA cycle', 'Closed through pyruvate', 1624, 795),
    ...module('Dv-arom', 'Aromatic amino acids', 'From erythrose-4-P and PEP', 160, 760),
    ...module('Dv-glu', 'Glutamate family', 'From 2-oxoglutarate', 930, 1200),
    ...module('Dv-asp', 'Aspartate family', 'Lysine, threonine, methionine, isoleucine', 1900, 1760),
    ...module('Dv-ser', 'Serine family and C1', 'From 3-phosphoglycerate', 160, 1200),
    ...module('Dv-pyr', 'Pyruvate family', 'Valine, leucine and alanine', 880, 1490),
    ...module('Dv-bio', 'Biomass', 'Precursors into macromolecules, and these into cells', 160, 1715),
    ...module('Dv-sulfate', 'Sulfate respiration', 'Sulfate reduced to H₂S', 1440, 2262),
    ...module('Dv-energy', 'Energy', 'ATP synthase and maintenance', 1990, 2160),
    ...module('Mm-carriers', 'Electron carriers', 'H₂ reduces the carriers', 2890, 960),
    ...module('Mm-methanogenesis', 'Methanogenesis', 'CO₂ to CH₄ on H₂ and formate', 3545, 1030),
    ...module('Mm-energy', 'Energy', 'ATP synthase and maintenance', 3380, 520),
    ...module('Mm-acetyl', 'Acetyl-CoA synthesis', 'From methyl-H₄MPT and CO, or from acetate', 3160, 1495),
    ...module('Mm-bio', 'Biomass', 'Precursors into macromolecules, and these into cells', 4060, 440),
    // Below the acetyl-CoA row, in the coordinates the stretch gives that half
    ...module('Mm-gng', 'Gluconeogenesis', 'Pyruvate to glucose-6-P', 3150, 2215),
    ...module('Mm-tca', 'Reductive TCA branch', 'To 2-oxoglutarate and glutamate', 3760, 1700),
    ...module('Mm-asp', 'Aspartate family', 'Lysine, threonine, methionine', 3620, 2060),
    ...module('Mm-arom', 'Aromatic amino acids', 'From erythrose-4-P and PEP', 4360, 2090),
    ...module('Mm-ppp', 'Pentose phosphates', 'Ribose-5-P and PRPP', 3620, 2440),
    ...module('Mm-pyr', 'Pyruvate family', 'Valine, leucine, isoleucine and alanine', 4560, 2480),
  ],
};

/** A module's title and, below it, its purpose. */
function module(id, title, purpose, x, y) {
  return [[`pathway:${id}`, title, x, y, 24], [`purpose:${id}`, purpose, x, y + 21, 15]];
}

// The methanogen's biosynthesis, everything below its acetyl-CoA row, is spread
// over the lower half of the cell, where it has the room.
stretchBelow(LAYOUT.species.Mm, 1560, 1.25);

function stretchBelow(spec, from, k) {
  const y = (v) => (v > from ? Math.round(from + (v - from) * k) : v);
  for (const v of Object.values(spec.nodes)) v[1] = y(v[1]);
  for (const r of Object.values(spec.rx)) {
    if (r.at) r.at = [r.at[0], y(r.at[1])];
    for (const p of Object.values(r.sat ?? {})) p[1] = y(p[1]);
    if (r.grid) r.grid.y = y(r.grid.y);
    for (const key of ['mem', 'ex']) {
      if (r[key] && (r[key][0] === 'left' || r[key][0] === 'right')) r[key] = [r[key][0], y(r[key][1])];
    }
  }
}
