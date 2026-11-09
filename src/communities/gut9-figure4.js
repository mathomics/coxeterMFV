/**
 * The Figure 4 experiment of the SteadyCom paper, transcribed from simFigure4.m.
 *
 *   Chan SHJ, Simons MN, Maranas CD (2017) PLoS Comput Biol 13(5): e1005539
 *
 * The question it asks: as Bacteroides thetaiotaomicron is allowed to take up
 * more dietary fiber, what happens to the community's growth rate, to who is in
 * it, and to how much fiber-derived substrate leaks back out to feed everyone
 * else? The sweep runs the maximum specific fiber uptake rate of B.
 * thetaiotaomicron from 1 to 30 C-mmol/gDW/h while holding F. prausnitzii at 5.
 *
 * Every list below is copied from the script, which is the canonical source for
 * them; none is inferred from the model or from the paper's prose.
 */

/** `carbonUTmax = [1:30; 5*ones(1,30)]` */
export const FIBER_SWEEP = {
  bacteroides: Array.from({ length: 30 }, (_, i) => i + 1),
  faecalibacterium: 5,
};

/** `options.GRguess = 0.05`, `options.GRtol = 1e-6` */
export const SOLVER_OPTIONS = { growthGuess: 0.05, tolerance: 1e-6 };

/** `BtF`: the fiber uptake reactions of B. thetaiotaomicron. */
export const BT_FIBER = [
  'EX_amannan140[u]tr_Bt', 'EX_pect[u]tr_Bt', 'EX_pecticgal[u]tr_Bt',
  'EX_arabinan101[u]tr_Bt', 'EX_arabinogal[u]tr_Bt', 'EX_homogal[u]tr_Bt',
  'EX_inulin[u]tr_Bt', 'EX_kesto[u]tr_Bt', 'EX_rhamnogalurII[u]tr_Bt',
  'EX_rhamnogalurI[u]tr_Bt', 'EX_kestopt[u]tr_Bt', 'EX_kestottr[u]tr_Bt',
];

/**
 * `BtCB`: the carbohydrate exchange reactions of B. thetaiotaomicron,
 * fiber-derived substrates among them.
 */
export const BT_CARBOHYDRATE = [
  'EX_amylose300[u]tr_Bt', 'EX_arab-L[u]tr_Bt', 'EX_fru[u]tr_Bt',
  'EX_fuc-L[u]tr_Bt', 'EX_gal[u]tr_Bt', 'EX_galur[u]tr_Bt', 'EX_glc-D[u]tr_Bt',
  'EX_glcur[u]tr_Bt', 'EX_glycogen1500[u]tr_Bt', 'EX_lcts[u]tr_Bt',
  'EX_malt[u]tr_Bt', 'EX_man[u]tr_Bt', 'EX_rmn[u]tr_Bt', 'EX_starch1200[u]tr_Bt',
  'EX_strch1[u]tr_Bt', 'EX_strch2[u]tr_Bt', 'EX_sucr[u]tr_Bt', 'EX_xyl-D[u]tr_Bt',
];

/** `FpF`: the fiber uptake reactions of F. prausnitzii. */
export const FP_FIBER = [
  'IEX_pect[u]tr_Fp', 'IEX_inulin[u]tr_Fp', 'IEX_kesto[u]tr_Fp',
  'IEX_kestopt[u]tr_Fp', 'IEX_kestottr[u]tr_Fp',
];

/** `carbonNum`: carbons per molecule of each carbohydrate and dietary fiber. */
export const CARBON_NUMBER = {
  '14glucan[u]': 36, 'amannan140[u]': 840, 'amylose300[u]': 1800,
  'arab-L[u]': 5, 'arabinan101[u]': 509, 'arabinogal[u]': 3862, 'fru[u]': 6,
  'fuc-L[u]': 6, 'gal[u]': 6, 'galur[u]': 6, 'glc-D[u]': 6, 'glcur[u]': 6,
  'glycogen1500[u]': 9000, 'homogal[u]': 660, 'inulin[u]': 180, 'kesto[u]': 18,
  'kestopt[u]': 30, 'kestottr[u]': 24, 'lcts[u]': 12, 'malt[u]': 12, 'man[u]': 6,
  'pect[u]': 2535, 'pecticgal[u]': 597, 'rhamnogalurII[u]': 317,
  'rhamnogalurI[u]': 5840, 'rmn[u]': 6, 'starch1200[u]': 7200, 'strch1[u]': 66,
  'strch2[u]': 18, 'sucr[u]': 12, 'xyl-D[u]': 5,
};

/**
 * The five species the figure names, in the order it stacks them:
 * `minFlux([1 2 8 4 7],:)` over the member order Bt,Fp,Kp,St,Ba,Ec,Ef,Er,Lc.
 */
export const FIGURE_SPECIES = ['Bt', 'Fp', 'Er', 'St', 'Ef'];

/** Invert the species-community exchange map: reaction id -> community metabolite. */
export function exchangeMetaboliteOf(model) {
  const out = new Map();
  for (const [key, rxnId] of model.speciesExchange) {
    out.set(rxnId, key.slice(0, key.indexOf('\u0000')));
  }
  return out;
}

/**
 * The carbon number each exchange reaction carries, which is the coefficient
 * that turns a flux in mmol/h into one in C-mmol/h.
 */
function carbonCoefficients(model, reactionIds) {
  const metaboliteOf = exchangeMetaboliteOf(model);
  const out = {};
  for (const rid of reactionIds) {
    if (!model.reactions.has(rid)) throw new Error(`${rid} is not in the model`);
    const met = metaboliteOf.get(rid);
    const carbons = CARBON_NUMBER[met];
    if (carbons === undefined) {
      throw new Error(`${rid} exchanges ${met}, which simFigure4.m gives no carbon number for`);
    }
    out[rid] = carbons;
  }
  return out;
}

/**
 * The constraint the script imposes at each point of the sweep:
 *
 *   sum_i C_i * V^k_i - C_max * X^k <= 0
 *
 * written there as coefficients `C_i / C_max` on the fluxes and `-1` on X^k,
 * with MCmode = 2 on the positive entries ("constrain the negative part of the
 * flux only"), so it bounds uptake and leaves secretion alone. Dividing through
 * by C_max is what the script does, and it is the same constraint either way.
 *
 * `bacteroides` and `faecalibacterium` are the two C_max values in
 * C-mmol/gDW/h. Returns one constraint per species.
 */
export function fiberConstraints(model, { bacteroides, faecalibacterium }) {
  const bt = carbonCoefficients(model, BT_FIBER);
  const fp = carbonCoefficients(model, FP_FIBER);

  return [
    {
      name: `fiber|Bt|${bacteroides}`,
      terms: Object.entries(bt).map(([reaction, carbons]) => ({
        reaction, coef: carbons / bacteroides, part: 'uptake',
      })),
      biomass: [{ species: 'Bt', coef: -1 }],
      hi: 0,
    },
    {
      name: `fiber|Fp|${faecalibacterium}`,
      terms: Object.entries(fp).map(([reaction, carbons]) => ({
        reaction, coef: carbons / faecalibacterium, part: 'uptake',
      })),
      biomass: [{ species: 'Fp', coef: -1 }],
      hi: 0,
    },
  ];
}

/**
 * The eleven targets the script asks FVA for: each member's biomass flux, then
 * B. thetaiotaomicron's total fiber exchange and total carbohydrate exchange,
 * both in C-mmol/h.
 *
 * A member's biomass flux is growth rate times abundance, which is why the
 * figure's stacked areas sum to the growth rate.
 */
export function figureTargets(model) {
  const targets = model.species.map((sp) => ({
    name: `Bm_${sp.id}`,
    reactions: { [sp.biomassReaction]: 1 },
  }));
  targets.push({ name: 'fiber_Bt_Cmmol', reactions: carbonCoefficients(model, BT_FIBER) });
  targets.push({
    name: 'carbohydrate_Bt_Cmmol',
    reactions: carbonCoefficients(model, BT_CARBOHYDRATE),
  });
  return targets;
}
