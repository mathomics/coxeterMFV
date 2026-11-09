//! The Figure 4 experiment of the SteadyCom paper, transcribed from simFigure4.m.
//!
//! Ports `src/communities/gut9-figure4.js`.
//!
//!   Chan SHJ, Simons MN, Maranas CD (2017) PLoS Comput Biol 13(5): e1005539
//!
//! The question it asks: as Bacteroides thetaiotaomicron is allowed to take up
//! more dietary fiber, what happens to the community's growth rate, to who is
//! in it, and to how much fiber-derived substrate leaks back out to feed
//! everyone else? The sweep runs the maximum specific fiber uptake rate of B.
//! thetaiotaomicron from 1 to 30 C-mmol/gDW/h while holding F. prausnitzii at 5.
//!
//! Every list below is copied from the script, which is the canonical source
//! for them. None is inferred from the model or from the paper's prose.

use coxeter_core::js;
use indexmap::IndexMap;

use crate::steadycom::{BiomassTerm, Constraint, ConstraintTerm, Fiber, JointReaction, SteadyComError, SteadyComSpec, SteadyComTarget};

/// `carbonUTmax = [1:30; 5*ones(1,30)]`: B. thetaiotaomicron's sweep.
pub const FIBER_SWEEP_BACTEROIDES: [f64; 30] = [
    1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0, 17.0, 18.0, 19.0, 20.0, 21.0, 22.0,
    23.0, 24.0, 25.0, 26.0, 27.0, 28.0, 29.0, 30.0,
];
/// …and F. prausnitzii's fixed limit.
pub const FIBER_SWEEP_FAECALIBACTERIUM: f64 = 5.0;

/// `options.GRguess = 0.05`.
pub const SOLVER_GROWTH_GUESS: f64 = 0.05;
/// `options.GRtol = 1e-6`.
pub const SOLVER_TOLERANCE: f64 = 1e-6;

/// `BtF`: the fiber uptake reactions of B. thetaiotaomicron.
pub const BT_FIBER: [&str; 12] = [
    "EX_amannan140[u]tr_Bt",
    "EX_pect[u]tr_Bt",
    "EX_pecticgal[u]tr_Bt",
    "EX_arabinan101[u]tr_Bt",
    "EX_arabinogal[u]tr_Bt",
    "EX_homogal[u]tr_Bt",
    "EX_inulin[u]tr_Bt",
    "EX_kesto[u]tr_Bt",
    "EX_rhamnogalurII[u]tr_Bt",
    "EX_rhamnogalurI[u]tr_Bt",
    "EX_kestopt[u]tr_Bt",
    "EX_kestottr[u]tr_Bt",
];

/// `BtCB`: the carbohydrate exchange reactions of B. thetaiotaomicron,
/// fiber-derived substrates among them.
pub const BT_CARBOHYDRATE: [&str; 18] = [
    "EX_amylose300[u]tr_Bt",
    "EX_arab-L[u]tr_Bt",
    "EX_fru[u]tr_Bt",
    "EX_fuc-L[u]tr_Bt",
    "EX_gal[u]tr_Bt",
    "EX_galur[u]tr_Bt",
    "EX_glc-D[u]tr_Bt",
    "EX_glcur[u]tr_Bt",
    "EX_glycogen1500[u]tr_Bt",
    "EX_lcts[u]tr_Bt",
    "EX_malt[u]tr_Bt",
    "EX_man[u]tr_Bt",
    "EX_rmn[u]tr_Bt",
    "EX_starch1200[u]tr_Bt",
    "EX_strch1[u]tr_Bt",
    "EX_strch2[u]tr_Bt",
    "EX_sucr[u]tr_Bt",
    "EX_xyl-D[u]tr_Bt",
];

/// `FpF`: the fiber uptake reactions of F. prausnitzii.
pub const FP_FIBER: [&str; 5] =
    ["IEX_pect[u]tr_Fp", "IEX_inulin[u]tr_Fp", "IEX_kesto[u]tr_Fp", "IEX_kestopt[u]tr_Fp", "IEX_kestottr[u]tr_Fp"];

/// `carbonNum`: carbons per molecule of each carbohydrate and dietary fiber.
pub const CARBON_NUMBER: [(&str, f64); 31] = [
    ("14glucan[u]", 36.0),
    ("amannan140[u]", 840.0),
    ("amylose300[u]", 1800.0),
    ("arab-L[u]", 5.0),
    ("arabinan101[u]", 509.0),
    ("arabinogal[u]", 3862.0),
    ("fru[u]", 6.0),
    ("fuc-L[u]", 6.0),
    ("gal[u]", 6.0),
    ("galur[u]", 6.0),
    ("glc-D[u]", 6.0),
    ("glcur[u]", 6.0),
    ("glycogen1500[u]", 9000.0),
    ("homogal[u]", 660.0),
    ("inulin[u]", 180.0),
    ("kesto[u]", 18.0),
    ("kestopt[u]", 30.0),
    ("kestottr[u]", 24.0),
    ("lcts[u]", 12.0),
    ("malt[u]", 12.0),
    ("man[u]", 6.0),
    ("pect[u]", 2535.0),
    ("pecticgal[u]", 597.0),
    ("rhamnogalurII[u]", 317.0),
    ("rhamnogalurI[u]", 5840.0),
    ("rmn[u]", 6.0),
    ("starch1200[u]", 7200.0),
    ("strch1[u]", 66.0),
    ("strch2[u]", 18.0),
    ("sucr[u]", 12.0),
    ("xyl-D[u]", 5.0),
];

/// The carbon number of a community metabolite, if simFigure4.m gives one.
pub fn carbon_number(metabolite: &str) -> Option<f64> {
    CARBON_NUMBER.iter().find(|(m, _)| *m == metabolite).map(|(_, c)| *c)
}

/// The five species the figure names, in the order it stacks them:
/// `minFlux([1 2 8 4 7],:)` over the member order Bt,Fp,Kp,St,Ba,Ec,Ef,Er,Lc.
pub const FIGURE_SPECIES: [&str; 5] = ["Bt", "Fp", "Er", "St", "Ef"];

/// What the Figure 4 helpers read from a joint community: which reactions
/// exist, the species-to-community exchanges (`"<met>\0<species>"` → reaction)
/// and the members with their biomass reactions.
pub trait Gut9Model {
    fn has_reaction(&self, id: &str) -> bool;
    fn species_exchange(&self) -> &IndexMap<String, String>;
    /// `(member id, biomass reaction)` in member order.
    fn species(&self) -> Vec<(String, String)>;
}

static NO_EXCHANGES: std::sync::LazyLock<IndexMap<String, String>> = std::sync::LazyLock::new(IndexMap::new);

impl Gut9Model for SteadyComSpec {
    fn has_reaction(&self, id: &str) -> bool {
        self.joint.as_ref().is_some_and(|j| j.reactions.contains_key(id))
    }
    fn species_exchange(&self) -> &IndexMap<String, String> {
        self.joint.as_ref().and_then(|j| j.metadata.as_ref()).map_or(&NO_EXCHANGES, |m| &m.species_exchange)
    }
    fn species(&self) -> Vec<(String, String)> {
        self.members.iter().map(|m| (m.id.clone(), m.biomass_reaction.clone())).collect()
    }
}

/// The pieces of a joint model the helpers need, borrowed.
pub struct JointView<'a> {
    pub reactions: &'a IndexMap<String, JointReaction>,
    pub species_exchange: &'a IndexMap<String, String>,
    pub species: Vec<(String, String)>,
}

impl Gut9Model for JointView<'_> {
    fn has_reaction(&self, id: &str) -> bool {
        self.reactions.contains_key(id)
    }
    fn species_exchange(&self) -> &IndexMap<String, String> {
        self.species_exchange
    }
    fn species(&self) -> Vec<(String, String)> {
        self.species.clone()
    }
}

/// Invert the species-community exchange map: reaction id → community metabolite.
pub fn exchange_metabolite_of(model: &impl Gut9Model) -> IndexMap<String, String> {
    model
        .species_exchange()
        .iter()
        .map(|(key, rxn)| (rxn.clone(), key.split('\u{0}').next().unwrap_or_default().to_string()))
        .collect()
}

/// The carbon number each exchange reaction carries, which is the coefficient
/// that turns a flux in mmol/h into one in C-mmol/h.
fn carbon_coefficients(model: &impl Gut9Model, reaction_ids: &[&str]) -> Result<IndexMap<String, f64>, SteadyComError> {
    let metabolite_of = exchange_metabolite_of(model);
    let mut out = IndexMap::new();
    for rid in reaction_ids {
        if !model.has_reaction(rid) {
            return Err(SteadyComError::Invalid(format!("{rid} is not in the model")));
        }
        let met = metabolite_of.get(*rid).map_or("undefined", String::as_str);
        let Some(carbons) = carbon_number(met) else {
            return Err(SteadyComError::Invalid(format!("{rid} exchanges {met}, which simFigure4.m gives no carbon number for")));
        };
        out.insert(rid.to_string(), carbons);
    }
    js::js_key_order(&mut out);
    Ok(out)
}

/// The constraint the script imposes at each point of the sweep:
///
/// ```text
///   sum_i C_i * V^k_i - C_max * X^k <= 0
/// ```
///
/// written there as coefficients `C_i / C_max` on the fluxes and `-1` on X^k,
/// with MCmode = 2 on the positive entries: "constrain the negative part of
/// the flux only", so it bounds uptake and leaves secretion alone. Dividing
/// through by C_max is what the script does. It is the same constraint either way.
pub fn fiber_constraints(model: &impl Gut9Model, fiber: Fiber) -> Result<Vec<Constraint>, SteadyComError> {
    let bt = carbon_coefficients(model, &BT_FIBER)?;
    let fp = carbon_coefficients(model, &FP_FIBER)?;
    let terms = |coefficients: &IndexMap<String, f64>, limit: f64| -> Vec<ConstraintTerm> {
        coefficients
            .iter()
            .map(|(reaction, carbons)| ConstraintTerm { reaction: reaction.clone(), coef: carbons / limit, uptake_only: true })
            .collect()
    };
    Ok(vec![
        Constraint {
            name: Some(format!("fiber|Bt|{}", js::number_to_string(fiber.bacteroides))),
            terms: terms(&bt, fiber.bacteroides),
            biomass: vec![BiomassTerm { species: "Bt".into(), coef: -1.0 }],
            lo: None,
            hi: Some(0.0),
        },
        Constraint {
            name: Some(format!("fiber|Fp|{}", js::number_to_string(fiber.faecalibacterium))),
            terms: terms(&fp, fiber.faecalibacterium),
            biomass: vec![BiomassTerm { species: "Fp".into(), coef: -1.0 }],
            lo: None,
            hi: Some(0.0),
        },
    ])
}

/// The eleven targets the script asks FVA for: each member's biomass flux,
/// then B. thetaiotaomicron's total fiber exchange and total carbohydrate
/// exchange, both in C-mmol/h.
///
/// A member's biomass flux is growth rate times abundance, which is why the
/// figure's stacked areas sum to the growth rate.
pub fn figure_targets(model: &impl Gut9Model) -> Result<Vec<SteadyComTarget>, SteadyComError> {
    let mut targets: Vec<SteadyComTarget> = model
        .species()
        .into_iter()
        .map(|(id, biomass)| SteadyComTarget {
            name: format!("Bm_{id}"),
            reactions: [(biomass, 1.0)].into_iter().collect(),
            ..Default::default()
        })
        .collect();
    targets.push(SteadyComTarget {
        name: "fiber_Bt_Cmmol".into(),
        reactions: carbon_coefficients(model, &BT_FIBER)?,
        ..Default::default()
    });
    targets.push(SteadyComTarget {
        name: "carbohydrate_Bt_Cmmol".into(),
        reactions: carbon_coefficients(model, &BT_CARBOHYDRATE)?,
        ..Default::default()
    });
    Ok(targets)
}

#[cfg(test)]
mod tests {
    use super::*;

    // test/gut9.test.mjs: 'the Figure 4 lists are the ones simFigure4.m defines'
    #[test]
    fn the_figure_4_lists_are_the_ones_sim_figure_4_defines() {
        assert_eq!(BT_FIBER.len(), 12);
        assert_eq!(BT_CARBOHYDRATE.len(), 18);
        assert_eq!(FP_FIBER.len(), 5);
        assert_eq!(CARBON_NUMBER.len(), 31);
        assert_eq!(FIBER_SWEEP_BACTEROIDES.to_vec(), (1..=30).map(f64::from).collect::<Vec<_>>());
        assert_eq!(FIBER_SWEEP_FAECALIBACTERIUM, 5.0);
        assert_eq!(FIGURE_SPECIES, ["Bt", "Fp", "Er", "St", "Ef"]);
        assert!(BT_FIBER.iter().all(|r| r.ends_with("_Bt")));
        assert!(FP_FIBER.iter().all(|r| r.starts_with("IEX_") && r.ends_with("_Fp")));
        assert_eq!(carbon_number("pect[u]"), Some(2535.0));
        assert_eq!(carbon_number("glycogen1500[u]"), Some(9000.0));
        assert_eq!(carbon_number("xyl-D[u]"), Some(5.0));
        assert_eq!((SOLVER_GROWTH_GUESS, SOLVER_TOLERANCE), (0.05, 1e-6));
    }
}
