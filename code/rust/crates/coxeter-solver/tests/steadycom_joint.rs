//! test/steadycom-joint-adapter.test.mjs, test/steadycom-numerics.test.mjs
//! and the solver parts of test/gut9.test.mjs, ported, plus parity with the
//! JS on the published assets (rust/fixtures/solver/runtime-published.json and
//! steadycom-joint-tiny.json).

mod common;

use std::sync::{Arc, OnceLock};

use coxeter_core::community::{Objective, ReactionRole};
use coxeter_core::testkit::{read_fixture, repo_root};
use coxeter_core::Model;
use coxeter_solver::fba::FluxProblem;
use coxeter_solver::gut9_figure4::{BT_FIBER, FP_FIBER, carbon_number, exchange_metabolite_of, figure_targets};
use coxeter_solver::lp::LpStatus;
use coxeter_solver::steadycom::{
    AtOptions, Constraint, ConstraintTerm, Fiber, SteadyComSpec, SteadyComStatus, SteadyComOptions, max_total_biomass_at,
    parsimonious_at, steady_com,
};
use coxeter_solver::steadycom_runtime::{
    JointSpecOptions, MemberModel, community_balances, joint_community_projection, joint_steady_com_spec, parse_media_csv,
    restore_steady_com_spec, serialize_steady_com_spec, steady_com_spec, to_merged_fluxes,
};
use common::{close, num, read_json};
use indexmap::IndexMap;
use serde_json::{Value, json};

fn gut_dir() -> std::path::PathBuf {
    repo_root().join("models/published/steadycom-gut-nine")
}

struct Gut9 {
    members: Vec<MemberModel>,
    bounds: String,
    manifest: Value,
    spec: SteadyComSpec,
}

fn gut9() -> &'static Gut9 {
    static GUT9: OnceLock<Gut9> = OnceLock::new();
    GUT9.get_or_init(|| {
        let manifest = read_json(&gut_dir().join("manifest.json"));
        let members: Vec<MemberModel> = manifest["members"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| {
                let doc = read_json(&gut_dir().join(m["model"].as_str().unwrap()));
                MemberModel::new(m["id"].as_str().unwrap(), Model::from_value(&doc).unwrap())
            })
            .collect();
        let bounds = std::fs::read_to_string(gut_dir().join("bounds.csv")).unwrap();
        let spec = joint_steady_com_spec(&members, &bounds, JointSpecOptions::default()).unwrap();
        Gut9 { members, bounds, manifest, spec }
    })
}

fn meta(spec: &SteadyComSpec) -> &coxeter_solver::steadycom::JointMetadata {
    spec.joint.as_ref().unwrap().metadata.as_ref().unwrap()
}

fn reaction<'a>(spec: &'a SteadyComSpec, id: &str) -> &'a coxeter_solver::steadycom::JointReaction {
    &spec.joint.as_ref().unwrap().reactions[id]
}

#[test]
fn published_e_coli_medium_aliases_resolve_through_their_exchange_reaction() {
    let dir = repo_root().join("models/published/steadycom-ecoli-auxotrophs");
    let model = Model::from_value(&read_json(&dir.join("model-Ec1.json"))).unwrap();
    let members = [MemberModel::new("Ec1", model)];
    let media = std::fs::read_to_string(dir.join("media.csv")).unwrap();
    let medium = parse_media_csv(&media, &members).unwrap();
    assert_eq!(medium["glc-D"], 8.0);
    assert!(!medium.contains_key("glc"));
    assert_eq!(medium["o2"], 18.5);
    assert_eq!(medium.len(), 22);
    let problem = steady_com_spec(&members, &medium, 1.0).unwrap();
    assert_eq!(problem.members[0].exchange_for["glc-D"], "EX_glc_e");
    let e = parse_media_csv("metabolite,exchange_reaction,lower_bound\nx,EX_missing,-1", &members).unwrap_err();
    assert!(e.to_string().contains("no matching member exchange"), "{e}");
}

#[test]
fn the_nine_member_assets_reconstruct_every_joint_reaction_and_coefficient() {
    let g = gut9();
    let spec = &g.spec;
    let joint = spec.joint.as_ref().unwrap();
    assert_eq!(spec.kind(), "joint");
    assert_eq!(joint.reactions.len(), 11009);
    assert_eq!(joint.metabolites.len(), 8160);
    assert_eq!(joint.reactions.values().map(|r| r.metabolites.len()).sum::<usize>(), 42420);
    assert_eq!(spec.members.len(), 9);
    assert_eq!(joint.community_tag, "comm");
    assert_eq!(spec.total_biomass, 1.0);
    let ids: Vec<(&str, &str)> = spec.members.iter().map(|m| (m.id.as_str(), m.biomass_reaction.as_str())).collect();
    assert_eq!(ids, [
        ("Bt", "Biomass_BT_v2_Bt"), ("Fp", "Biomass_FP_Fp"), ("Kp", "Biomass_Kp"), ("St", "biomass_STR_St"), ("Ba", "Biomass_Ba"),
        ("Ec", "Ec_biomass_iJO1366_WT_53p95M_Ec"), ("Ef", "R_BIOMASS_Ef"), ("Er", "r55_Er"), ("Lc", "bio06001_Lc"),
    ]);
    for line in g.bounds.trim().lines().skip(1) {
        let f: Vec<&str> = line.split(',').collect();
        let r = &joint.reactions[f[0]];
        assert_eq!(r.species, f[1], "{}", f[0]);
        assert_eq!(r.lower_bound, f[2].parse::<f64>().unwrap(), "{}", f[0]);
        assert_eq!(r.upper_bound, f[3].parse::<f64>().unwrap(), "{}", f[0]);
        assert_eq!(r.objective, f[4].parse::<f64>().unwrap(), "{}", f[0]);
    }
}

#[test]
fn shared_stoichiometry_supplies_exact_exchange_ownership() {
    let spec = &gut9().spec;
    let m = meta(spec);
    assert_eq!(m.species_exchange.len(), 1355);
    assert_eq!(m.bases.len(), 558);
    assert_eq!(m.community_uptake.len(), 558);
    assert_eq!(m.community_export.len(), 558);
    let ex: Vec<(&str, f64)> = reaction(spec, "EX_12ethd[u]tr_Bt").metabolites.iter().map(|(k, v)| (k.as_str(), *v)).collect();
    assert_eq!(ex, [("12ethd[e_Bt]", -1.0), ("12ethd[u]", 1.0)]);
    assert_eq!(m.species_exchange["glc-D[u]\u{0}Er"], "glcIn_Er");
    assert_eq!(m.species_exchange["2mba[u]\u{0}Ef"], "R_Ex_2mbut__e_Ef");
    assert_eq!(m.bases["glc-D[u]"], "glc-D");
    let joint = spec.joint.as_ref().unwrap();
    for met in ["ad[u]", "biomassPg[u]", "h2co3[u]", "pime[u]"] {
        assert_eq!(joint.metabolites[met]["species"], "comm");
    }
    assert_eq!(joint.reactions.values().filter(|r| r.species == "comm").count(), 1116);
}

#[test]
fn the_full_diet_and_atp_maintenance_survive() {
    let g = gut9();
    let spec = &g.spec;
    let open = meta(spec).community_uptake.values().filter(|id| reaction(spec, id).upper_bound > 0.0).count();
    assert_eq!(open, 88);
    assert_eq!(g.manifest["medium"].as_object().unwrap().len(), 8);
    assert_eq!(reaction(spec, "UT_glc-D(u)").upper_bound, 0.03832823);
    assert_eq!(reaction(spec, "UT_adocbl(u)").upper_bound, 0.1);
    assert_eq!(reaction(spec, "UT_adocbl(u)").metabolites["adocbl[u]"], 1.0);
    assert_eq!(reaction(spec, "EX_adocbl(u)").metabolites["adocbl[u]"], -1.0);
    for id in meta(spec).community_export.values() {
        assert_eq!((reaction(spec, id).lower_bound, reaction(spec, id).upper_bound), (0.0, 10000.0));
    }
    for id in ["ATPM_Bt", "ATPM_Fp", "ATPM_Kp", "ATPM_St", "ATPM_Ba", "ATPM_Ec", "R_ATPM_Ef", "r54_Er", "rxn00062_Lc"] {
        assert_eq!((reaction(spec, id).lower_bound, reaction(spec, id).upper_bound), (1.0, 1000.0), "{id}");
    }
}

#[test]
fn an_explicit_figure_4_fiber_condition_adds_the_uptake_only_constraints() {
    let g = gut9();
    let fiber = Fiber { bacteroides: 10.0, faecalibacterium: 5.0 };
    let mut conditioned = joint_steady_com_spec(&g.members, &g.bounds, JointSpecOptions { total_biomass: 2.0, fiber: Some(fiber) }).unwrap();
    assert_eq!(conditioned.total_biomass, 2.0);
    let constraints = &conditioned.joint.as_ref().unwrap().constraints;
    assert_eq!(constraints.iter().map(|c| c.terms.len()).collect::<Vec<_>>(), [12, 5]);
    assert!(constraints.iter().all(|c| c.hi == Some(0.0) && c.terms.iter().all(|t| t.uptake_only)));
    assert_eq!((constraints[0].biomass[0].species.as_str(), constraints[0].biomass[0].coef), ("Bt", -1.0));
    assert_eq!((constraints[1].biomass[0].species.as_str(), constraints[1].biomass[0].coef), ("Fp", -1.0));
    assert_eq!(constraints[0].terms.iter().find(|t| t.reaction == "EX_pect[u]tr_Bt").unwrap().coef, 253.5);
    assert!(g.spec.joint.as_ref().unwrap().constraints.is_empty(), "the unconditioned spec must remain independent");
    conditioned.joint.as_mut().unwrap().reactions.get_mut("ATPM_Bt").unwrap().lower_bound = 0.0;
    assert_eq!(g.members[0].model.reactions["ATPM_Bt"].lower_bound, 1.0, "joint reactions must not alias member models");
}

#[test]
fn joint_flux_ownership_maps_species_rates_to_drawing_ids() {
    let spec = &gut9().spec;
    let values: IndexMap<String, f64> = [("glcIn_Er", -2.0), ("Biomass_BT_v2_Bt", 0.005), ("UT_glc-D(u)", 0.01), ("EX_glc-D(u)", 0.003)]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
    let mapped = to_merged_fluxes(&values, "@", Some(&meta(spec).reaction_owners));
    assert_eq!(mapped["Er@glcIn_Er"], -2.0);
    assert_eq!(mapped["Bt@Biomass_BT_v2_Bt"], 0.005);
    assert_eq!(mapped["UT_glc-D(u)"], 0.01);
    assert_eq!(mapped["EX_glc-D(u)"], 0.003);
    assert_eq!(mapped.len(), values.len());
    let separate = to_merged_fluxes(&[("A|R".to_string(), 4.0)].into_iter().collect(), "@", None);
    assert_eq!(separate["A@R"], 4.0);
}

#[test]
fn community_balances_report_actual_rates_not_bounds() {
    let spec = &gut9().spec;
    let m = meta(spec);
    let er = m.species_exchange["glc-D[u]\u{0}Er"].clone();
    let bt = m.species_exchange["glc-D[u]\u{0}Bt"].clone();
    let fluxes: IndexMap<String, f64> =
        [(er, -0.2), (bt, 0.3), ("UT_glc-D(u)".into(), 0.012), ("EX_glc-D(u)".into(), 0.112)].into_iter().collect();
    let r = community_balances(spec, &fluxes);
    assert_eq!(r.species_exchanges["glc-D[u]"]["Er"], -0.2);
    assert_eq!(r.species_exchanges["glc-D[u]"]["Bt"], 0.3);
    assert_eq!(r.community_uptake["glc-D[u]"], 0.012);
    assert_eq!(r.community_export["glc-D[u]"], 0.112);
    assert_eq!(r.community_uptake["adocbl[u]"], 0.0, "absent flux columns are zero, not their diet bounds");
}

#[test]
fn joint_snapshots_preserve_maps_and_constraints() {
    let g = gut9();
    let conditioned = joint_steady_com_spec(
        &g.members,
        &g.bounds,
        JointSpecOptions { fiber: Some(Fiber { bacteroides: 10.0, faecalibacterium: 5.0 }), ..Default::default() },
    )
    .unwrap();
    let text = serde_json::to_string(&serialize_steady_com_spec(&conditioned)).unwrap();
    let mut restored = restore_steady_com_spec(&serde_json::from_str(&text).unwrap()).unwrap();
    assert_eq!(restored, conditioned);
    assert!(restored.members.iter().all(|m| m.model.is_none()));
    restored.joint.as_mut().unwrap().reactions.get_mut("ATPM_Bt").unwrap().lower_bound = 0.0;
    assert_eq!(conditioned.joint.as_ref().unwrap().reactions["ATPM_Bt"].lower_bound, 1.0);
}

#[test]
fn separate_snapshots_reconstruct_models() {
    let model = Model::from_value(&json!({
        "id": "generic", "compartments": {"e": "Extracellular", "c": "Cytosol"},
        "metabolites": [{"id": "arg-L_e", "compartment": "e"}, {"id": "arg-L_c", "compartment": "c"}],
        "reactions": [
            {"id": "EX_arg_L_e", "metabolites": {"arg-L_e": -1}, "lower_bound": -5, "upper_bound": 100},
            {"id": "T", "metabolites": {"arg-L_e": -1, "arg-L_c": 1}, "lower_bound": 0, "upper_bound": 100},
            {"id": "BIOMASS", "metabolites": {"arg-L_c": -1}, "lower_bound": 0, "upper_bound": 100}
        ], "objective": {"BIOMASS": 1}
    }))
    .unwrap();
    let members = [MemberModel::new("generic", model.clone())];
    assert_eq!(coxeter_solver::steadycom_runtime::exchange_index(&model).unwrap().into_iter().collect::<Vec<_>>(), [("arg-L".to_string(), "EX_arg_L_e".to_string())]);
    let original = steady_com_spec(&members, &[("arg-L".to_string(), 5.0)].into_iter().collect(), 1.0).unwrap();
    let snapshot = serialize_steady_com_spec(&original);
    let restored = restore_steady_com_spec(&snapshot).unwrap();
    assert_eq!(restored.kind(), "separate");
    let restored_model = restored.members[0].model.as_ref().unwrap();
    assert_eq!(restored_model.to_json(), model.to_json());
    assert_eq!(restored_model.biomass_reaction_id().as_deref(), Some("BIOMASS"));
    assert!(!Arc::ptr_eq(restored_model, original.members[0].model.as_ref().unwrap()), "every member gets its own model");
}

#[test]
fn missing_duplicate_or_invalid_rows_cannot_produce_a_complete_joint_problem() {
    let g = gut9();
    let opts = JointSpecOptions::default();
    let without: String = g.bounds.lines().filter(|l| !l.starts_with("ATPM_Bt,")).collect::<Vec<_>>().join("\n");
    let e = joint_steady_com_spec(&g.members, &without, opts).unwrap_err().to_string();
    assert!(e.contains("ATPM_Bt") && e.contains("matching bounds"), "{e}");
    let duplicate = format!("{}\nATPM_Bt,Bt,1,1000,0\n", g.bounds.trim());
    let e = joint_steady_com_spec(&g.members, &duplicate, opts).unwrap_err().to_string();
    assert!(e.contains("Duplicate") && e.contains("ATPM_Bt"), "{e}");
    let nan = g.bounds.replace("UT_adocbl(u),comm,0,0.1,0", "UT_adocbl(u),comm,0,NaN,0");
    assert!(joint_steady_com_spec(&g.members, &nan, opts).unwrap_err().to_string().contains("invalid joint bounds"));
    let e = joint_steady_com_spec(&g.members, &g.bounds, JointSpecOptions { total_biomass: 0.0, fiber: None }).unwrap_err();
    assert!(e.to_string().contains("positive and finite"));
    let e = joint_steady_com_spec(
        &g.members,
        &g.bounds,
        JointSpecOptions { fiber: Some(Fiber { bacteroides: 0.0, faecalibacterium: 5.0 }), ..Default::default() },
    )
    .unwrap_err();
    assert!(e.to_string().contains("fiber uptake limits") && e.to_string().contains("positive and finite"));
}

// The tiny joint pair

fn tiny_docs() -> Vec<Value> {
    ["A", "B"]
        .iter()
        .map(|id| {
            json!({
                "id": id, "compartments": {"e": "Extracellular", "c": "Cytosol", "u": "Shared medium"},
                "metabolites": [
                    {"id": format!("a[e_{id}]"), "compartment": "e"},
                    {"id": format!("a[c_{id}]"), "compartment": "c"},
                    {"id": "a[u]", "compartment": "u"}
                ],
                "reactions": [
                    {"id": format!("EX_{id}"), "metabolites": {format!("a[e_{id}]"): -1, "a[u]": 1}, "lower_bound": -20, "upper_bound": 1000},
                    {"id": format!("T_{id}"), "metabolites": {format!("a[e_{id}]"): -1, format!("a[c_{id}]"): 1}, "lower_bound": 0, "upper_bound": 1000},
                    {"id": format!("BIO_{id}"), "metabolites": {format!("a[c_{id}]"): -1}, "lower_bound": 0, "upper_bound": 1000}
                ],
                "objective": {format!("BIO_{id}"): 1}
            })
        })
        .collect()
}

fn tiny_joint() -> SteadyComSpec {
    let members: Vec<MemberModel> =
        tiny_docs().iter().map(|d| MemberModel::new(d["id"].as_str().unwrap(), Model::from_value(d).unwrap())).collect();
    let mut csv = vec!["reaction,species,lower_bound,upper_bound,objective_coefficient".to_string()];
    for id in ["A", "B"] {
        csv.push(format!("EX_{id},{id},-20,1000,0"));
        csv.push(format!("T_{id},{id},0,1000,0"));
        csv.push(format!("BIO_{id},{id},0,1000,1"));
    }
    csv.push("UT_a(u),comm,0,10,0".into());
    csv.push("EX_a(u),comm,0,10000,0".into());
    joint_steady_com_spec(&members, &csv.join("\n"), JointSpecOptions::default()).unwrap()
}

fn amounts(a: f64, b: f64) -> IndexMap<String, f64> {
    [("A".to_string(), a), ("B".to_string(), b)].into_iter().collect()
}

fn assert_mass_balanced(model: &Model, fluxes: &IndexMap<String, f64>) {
    let mut balances: IndexMap<&str, f64> = IndexMap::new();
    for r in model.reactions.values() {
        for (mid, c) in &r.metabolites {
            *balances.entry(mid).or_default() += c * fluxes.get(&r.id).copied().unwrap_or(0.0);
        }
    }
    for (mid, residual) in balances {
        assert!(residual.abs() < 1e-7, "{mid} mass residual {residual}");
    }
}

#[test]
fn unequal_fixed_abundances_scale_only_shared_coefficients() {
    let mut joint = tiny_joint();
    joint.joint.as_mut().unwrap().constraints =
        vec![Constraint { name: Some("steadycom-only".into()), terms: vec![ConstraintTerm { reaction: "EX_A".into(), coef: 1.0, uptake_only: true }], hi: Some(0.0), ..Default::default() }];
    let p = joint_community_projection(&joint, &amounts(0.25, 0.75), Objective::EqualGrowth, None).unwrap();
    let mets = |id: &str| p.model.reactions[id].metabolites.iter().map(|(k, v)| (k.clone(), *v)).collect::<Vec<_>>();
    assert_eq!(mets("A@EX_A"), [("A@a[e_A]".to_string(), -1.0), ("M@a_s".to_string(), 0.25)]);
    assert_eq!(mets("B@EX_B"), [("B@a[e_B]".to_string(), -1.0), ("M@a_s".to_string(), 0.75)]);
    assert_eq!(mets("UT_a(u)"), [("M@a_s".to_string(), 1.0)]);
    assert_eq!(p.model.reactions["UT_a(u)"].upper_bound, 10.0);
    assert_eq!(p.model.metabolites["M@a_s"].compartment, "s");
    let r = FluxProblem::new(&p.model).optimize();
    assert_eq!(r.status, LpStatus::Optimal);
    assert!((r.objective_value - 10.0).abs() < 1e-7);
    assert!((r.fluxes["A@BIO_A"] - 10.0).abs() < 1e-7);
    assert!((r.fluxes["B@BIO_B"] - 10.0).abs() < 1e-7);
    assert_mass_balanced(&p.model, &r.fluxes);
    assert_eq!(joint.joint.as_ref().unwrap().reactions["EX_A"].metabolites["a[u]"], 1.0, "projection must not mutate the source");
}

#[test]
fn zero_abundance_members_are_closed_without_forcing_growth_to_zero() {
    let p = joint_community_projection(&tiny_joint(), &amounts(1.0, 0.0), Objective::EqualGrowth, None).unwrap();
    for id in ["B@EX_B", "B@T_B", "B@BIO_B"] {
        assert_eq!((p.model.reactions[id].lower_bound, p.model.reactions[id].upper_bound), (0.0, 0.0), "{id}");
    }
    assert!(!p.model.metabolites.contains_key("GROWTHLINK@B"));
    assert_eq!(p.active_species.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["A"]);
    let r = FluxProblem::new(&p.model).optimize();
    assert_eq!(r.status, LpStatus::Optimal);
    assert!((r.objective_value - 10.0).abs() < 1e-7);
    assert_eq!(r.fluxes["B@BIO_B"], 0.0);
    assert_mass_balanced(&p.model, &r.fluxes);
}

#[test]
fn fixed_projection_supports_weighted_biomass_and_net_secretion() {
    let joint = tiny_joint();
    let biomass = joint_community_projection(&joint, &amounts(0.25, 0.75), Objective::SumBiomass, None).unwrap();
    assert_eq!(biomass.model.raw_objective, Some(json!({"A@BIO_A": 0.25, "B@BIO_B": 0.75})));
    assert!(!biomass.model.reactions.contains_key("COMMUNITY_GROWTH"));
    let product = joint_community_projection(&joint, &amounts(0.25, 0.75), Objective::Product, Some("a")).unwrap();
    assert_eq!(product.model.raw_objective, Some(json!({"EX_a(u)": 1, "UT_a(u)": -1})));
    let r = FluxProblem::new(&product.model).optimize();
    assert_eq!(r.status, LpStatus::Optimal);
    assert!(r.objective_value.abs() < 1e-7, "importing and immediately exporting substrate is not net product");
    assert_mass_balanced(&product.model, &r.fluxes);
    assert!(Objective::parse("unknown").is_none(), "an unknown objective cannot be asked for");
    let e = joint_community_projection(&joint, &amounts(0.5, 0.5), Objective::Product, Some("missing")).unwrap_err();
    assert!(e.to_string().contains("No community boundary"));
    let e = joint_community_projection(&joint, &amounts(0.0, 0.0), Objective::EqualGrowth, None).unwrap_err();
    assert!(e.to_string().contains("positive abundance"));
}

#[test]
fn full_gut9_projection_keeps_every_biological_column() {
    let spec = &gut9().spec;
    let even: IndexMap<String, f64> = spec.members.iter().map(|m| (m.id.clone(), 1.0 / 9.0)).collect();
    let p = joint_community_projection(spec, &even, Objective::EqualGrowth, None).unwrap();
    assert_eq!(p.model.reactions.len(), 11010);
    assert_eq!(p.model.metabolites.len(), 8169);
    assert_eq!(p.biomass_ids.len(), 9);
    assert_eq!(p.transfers_by_base.len(), 554);
    let t = p.transfers_by_base["glc-D"].iter().find(|t| t.species_id == "Er").unwrap();
    assert_eq!((t.reaction_id.as_str(), t.local_id.as_str()), ("Er@glcIn_Er", "Er@cpd00027[e_Er]"));
    let owner = &p.reaction_owner["Er@glcIn_Er"];
    assert_eq!((owner.species_id.as_deref(), owner.original_id.as_str(), owner.role), (Some("Er"), "glcIn_Er", ReactionRole::Transfer));
    let owner = &p.reaction_owner["UT_glc-D(u)"];
    assert_eq!((owner.species_id.as_deref(), owner.role), (None, ReactionRole::Community));
    let ids = &p.community_exchange_ids["glc-D"];
    assert_eq!((ids.uptake.as_str(), ids.export.as_str()), ("UT_glc-D(u)", "EX_glc-D(u)"));
    assert_eq!(p.model.metabolites["M@glc-D_s"].compartment, "s");
    assert!(p.model.compartment_ids().iter().all(|c| ["c", "e", "p", "s", "link"].contains(&c.as_str())));
}

// gut9.test.mjs

#[test]
fn the_figure_4_reactions_exist_and_carry_a_carbon_number() {
    let g = gut9();
    let spec = &g.spec;
    let metabolite_of = exchange_metabolite_of(spec);
    for (list, species) in [(&BT_FIBER[..], "Bt"), (&coxeter_solver::gut9_figure4::BT_CARBOHYDRATE[..], "Bt"), (&FP_FIBER[..], "Fp")] {
        for rid in list {
            assert_eq!(reaction(spec, rid).species, species);
            let met = &metabolite_of[*rid];
            assert!(carbon_number(met).unwrap() > 0.0, "{met}");
        }
    }
    let conditioned = joint_steady_com_spec(
        &g.members,
        &g.bounds,
        JointSpecOptions { fiber: Some(Fiber { bacteroides: 10.0, faecalibacterium: 5.0 }), ..Default::default() },
    )
    .unwrap();
    let constraints = &conditioned.joint.as_ref().unwrap().constraints;
    assert_eq!(constraints.len(), 2);
    let pect = constraints[0].terms.iter().find(|t| t.reaction == "EX_pect[u]tr_Bt").unwrap();
    assert_eq!(pect.coef, carbon_number("pect[u]").unwrap() / 10.0);
    let targets = figure_targets(spec).unwrap();
    assert_eq!(targets.len(), 11);
    assert_eq!(targets[..9].iter().map(|t| t.name.clone()).collect::<Vec<_>>(), spec.members.iter().map(|m| format!("Bm_{}", m.id)).collect::<Vec<_>>());
    assert_eq!(targets[9].name, "fiber_Bt_Cmmol");
    assert_eq!(targets[10].name, "carbohydrate_Bt_Cmmol");
}

// Parity with the JS outputs

fn strings(v: &Value) -> IndexMap<String, String> {
    v.as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string())).collect()
}

#[test]
fn published_runtime_outputs_match_the_javascript() {
    let fx = read_fixture("solver/runtime-published.json").expect("run rust/tools/fixtures-solver.mjs");
    let dir = repo_root().join("models/published/steadycom-ecoli-auxotrophs");
    let ec1 = Model::from_value(&read_json(&dir.join("model-Ec1.json"))).unwrap();
    let media = std::fs::read_to_string(dir.join("media.csv")).unwrap();
    let members = [MemberModel::new("Ec1", ec1.clone())];
    assert_eq!(coxeter_solver::steadycom_runtime::exchange_index(&ec1).unwrap(), strings(&fx["ecoli"]["exchangeIndex"]));
    let js_media = |v: &Value| v.as_object().unwrap().iter().map(|(k, v)| (k.clone(), num(v))).collect::<IndexMap<_, _>>();
    let media_with = parse_media_csv(&media, &members).unwrap();
    assert_eq!(media_with, js_media(&fx["ecoli"]["media"]));
    assert!(media_with.keys().eq(js_media(&fx["ecoli"]["media"]).keys()), "key order");
    assert_eq!(parse_media_csv(&media, &[]).unwrap(), js_media(&fx["ecoli"]["mediaBare"]));

    let g = gut9();
    let joint = joint_steady_com_spec(
        &g.members,
        &g.bounds,
        JointSpecOptions { fiber: Some(Fiber { bacteroides: 10.0, faecalibacterium: 5.0 }), ..Default::default() },
    )
    .unwrap();
    let js = &fx["gut9"];
    let j = joint.joint.as_ref().unwrap();
    let m = j.metadata.as_ref().unwrap();
    assert_eq!(j.reactions.len() as u64, js["reactions"].as_u64().unwrap());
    assert_eq!(j.metabolites.len() as u64, js["metabolites"].as_u64().unwrap());
    for (i, member) in joint.members.iter().enumerate() {
        assert_eq!(member.id, js["members"][i]["id"].as_str().unwrap());
        assert_eq!(member.biomass_reaction, js["members"][i]["biomassReaction"].as_str().unwrap());
        assert_eq!(member.name.as_deref(), js["members"][i]["name"].as_str());
    }
    for (id, r) in js["reactionSample"].as_object().unwrap() {
        let rust = &j.reactions[id];
        assert_eq!(rust.species, r["species"].as_str().unwrap());
        assert_eq!((rust.lower_bound, rust.upper_bound, rust.objective), (num(&r["lowerBound"]), num(&r["upperBound"]), num(&r["objective"])));
        assert_eq!(rust.name, r["name"].as_str().unwrap());
        let mets: IndexMap<String, f64> = r["metabolites"].as_object().unwrap().iter().map(|(k, v)| (k.clone(), num(v))).collect();
        assert!(rust.metabolites.iter().eq(mets.iter()), "{id}: stoichiometry and its order");
    }
    for (id, doc) in js["metaboliteSample"].as_object().unwrap() {
        coxeter_core::testkit::assert_json_eq(&Value::Object(j.metabolites[id].clone()), doc, 0.0, id);
    }
    let constraints: Vec<Value> = j
        .constraints
        .iter()
        .map(|c| {
            json!({
                "name": c.name, "hi": c.hi,
                "terms": c.terms.iter().map(|t| json!({"reaction": t.reaction, "coef": t.coef, "part": "uptake"})).collect::<Vec<_>>(),
                "biomass": c.biomass.iter().map(|b| json!({"species": b.species, "coef": b.coef})).collect::<Vec<_>>(),
            })
        })
        .collect();
    coxeter_core::testkit::assert_json_eq(&Value::Array(constraints), &js["constraints"], 0.0, "fiber constraints");
    assert_eq!(m.species_exchange.len() as u64, js["speciesExchangeCount"].as_u64().unwrap());
    assert_eq!(m.bases.len() as u64, js["bases"].as_u64().unwrap());
    for (key, sample) in [("speciesExchangeSample", &m.species_exchange), ("communityUptakeSample", &m.community_uptake), ("communityExportSample", &m.community_export)] {
        let expected = strings(&js[key]);
        let first: IndexMap<String, String> = sample.iter().take(expected.len()).map(|(k, v)| (k.clone(), v.clone())).collect();
        assert_eq!(first, expected, "{key}");
    }
    let targets: Vec<Value> = figure_targets(&joint)
        .unwrap()
        .iter()
        .map(|t| json!({"name": t.name, "reactions": t.reactions}))
        .collect();
    coxeter_core::testkit::assert_json_eq(&Value::Array(targets), &js["targets"], 0.0, "figure targets");

    let even: IndexMap<String, f64> = joint.members.iter().map(|m| (m.id.clone(), 1.0 / 9.0)).collect();
    let p = joint_community_projection(&joint, &even, Objective::EqualGrowth, None).unwrap();
    let jp = &js["projection"];
    assert_eq!(p.model.reactions.len() as u64, jp["reactions"].as_u64().unwrap());
    assert_eq!(p.model.metabolites.len() as u64, jp["metabolites"].as_u64().unwrap());
    assert_eq!(p.model.compartment_ids(), jp["compartments"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect::<Vec<_>>());
    assert_eq!(p.transfers_by_base.len() as u64, jp["transfers"].as_u64().unwrap());
    let glc: Vec<Value> = p.transfers_by_base["glc-D"]
        .iter()
        .map(|t| json!({"speciesId": t.species_id, "reactionId": t.reaction_id, "localId": t.local_id}))
        .collect();
    coxeter_core::testkit::assert_json_eq(&Value::Array(glc), &jp["transfersGlc"], 0.0, "glc transfers");
    let objective: Vec<Value> = p.model.objective_reactions().iter().map(|(id, c)| json!({"id": id, "coefficient": c})).collect();
    coxeter_core::testkit::assert_json_eq(&Value::Array(objective), &jp["objective"], 0.0, "objective");
    coxeter_core::testkit::assert_json_eq(&p.model.reactions["COMMUNITY_GROWTH"].to_json(), &jp["growth"], 0.0, "growth reaction");
    for (id, doc) in jp["sampleReactions"].as_object().unwrap() {
        coxeter_core::testkit::assert_json_eq(&p.model.reactions[id].to_json(), doc, 0.0, id);
    }
    for (id, owner) in jp["reactionOwnerSample"].as_object().unwrap() {
        let r = &p.reaction_owner[id];
        let rust = json!({"speciesId": r.species_id, "originalId": r.original_id, "role": r.role.as_str()});
        coxeter_core::testkit::assert_json_eq(&rust, owner, 0.0, id);
    }
    let ids = &p.community_exchange_ids["glc-D"];
    coxeter_core::testkit::assert_json_eq(&json!({"uptake": ids.uptake, "export": ids.export}), &jp["communityExchangeGlc"], 0.0, "glc ids");
}

#[test]
fn the_tiny_joint_community_matches_the_javascript() {
    let fx = read_fixture("solver/steadycom-joint-tiny.json").expect("run rust/tools/fixtures-solver.mjs");
    let joint = tiny_joint();
    let range = steady_com(&joint, SteadyComOptions { upper_guess: 1.0, tolerance: 1e-5, biomass_cap: Some(2.0), ..Default::default() }).unwrap();
    assert_eq!(range.status.as_str(), fx["range"]["status"]);
    assert!(close(range.growth_rate, num(&fx["range"]["growthRate"]), 2e-5), "{} vs {}", range.growth_rate, fx["range"]["growthRate"]);
    let at = max_total_biomass_at(&joint, 0.5, &AtOptions::default()).unwrap();
    assert_eq!(at.status.as_str(), fx["at"]["status"]);
    assert_eq!(at.feasible, fx["at"]["feasible"].as_bool().unwrap());
    assert!(close(at.total, num(&fx["at"]["total"]), 1e-7));
    let lean = parsimonious_at(&joint, 0.5, &AtOptions::default()).unwrap();
    assert!(close(lean.total_flux.unwrap(), num(&fx["parsimonious"]["totalFlux"]), 1e-6));
    for case in fx["projections"].as_array().unwrap() {
        let a = case["abundances"].as_object().unwrap();
        let amounts: IndexMap<String, f64> = a.iter().map(|(k, v)| (k.clone(), num(v))).collect();
        let objective = case["opts"]["objective"].as_str().map_or(Objective::EqualGrowth, |o| Objective::parse(o).unwrap());
        let p = joint_community_projection(&joint, &amounts, objective, case["opts"]["productTarget"].as_str()).unwrap();
        coxeter_core::testkit::assert_json_eq(&p.model.to_json(), &case["model"], 0.0, "projected model");
        let r = FluxProblem::new(&p.model).optimize();
        assert_eq!(r.status.as_str(), case["status"]);
        assert!(close(r.objective_value, num(&case["objectiveValue"]), 1e-7));
        let owners: serde_json::Map<String, Value> = p
            .reaction_owner
            .iter()
            .map(|(k, r)| (k.clone(), json!({"speciesId": r.species_id, "originalId": r.original_id, "role": r.role.as_str()})))
            .collect();
        coxeter_core::testkit::assert_json_eq(&Value::Object(owners), &case["reactionOwner"], 0.0, "reaction owners");
        let met_owners: serde_json::Map<String, Value> = p
            .metabolite_owner
            .iter()
            .map(|(k, m)| (k.clone(), json!({"speciesId": m.species_id, "originalId": m.original_id, "compartment": m.compartment})))
            .collect();
        coxeter_core::testkit::assert_json_eq(&Value::Object(met_owners), &case["metaboliteOwner"], 0.0, "metabolite owners");
        assert_eq!(
            p.active_species.iter().map(|m| m.id.clone()).collect::<Vec<_>>(),
            case["activeSpecies"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect::<Vec<_>>()
        );
        let sc: serde_json::Map<String, Value> = p.species_compartments.iter().map(|(k, v)| (k.clone(), json!(v))).collect();
        coxeter_core::testkit::assert_json_eq(&Value::Object(sc), &case["speciesCompartments"], 0.0, "species compartments");
    }
}

// steadycom-numerics.test.mjs

/// Tiny populations in the full nine-species community keep their requested
/// specific growth. Each LP has ~28 000 rows.
#[test]
fn tiny_populations_in_the_nine_species_community_retain_their_specific_growth() {
    let g = gut9();
    for (mu, fiber) in [(0.03984680175781251, None), (0.04, None), (0.036019897460937506, Some(Fiber { bacteroides: 10.0, faecalibacterium: 5.0 }))] {
        let spec = joint_steady_com_spec(&g.members, &g.bounds, JointSpecOptions { fiber, ..Default::default() }).unwrap();
        let r = max_total_biomass_at(&spec, mu, &AtOptions { time_limit_seconds: Some(30.0), ..Default::default() }).unwrap();
        assert_eq!(r.status, LpStatus::Optimal, "mu {mu}: {}", r.message);
        assert!(r.feasible);
        assert!((r.total - 1.0).abs() < 1e-8);
        assert!(r.abundances.values().any(|x| *x > 1e-9 && *x < 1e-3), "some population is tiny: {:?}", r.abundances);
        for member in &spec.members {
            let x = r.abundances[&member.id];
            let biomass = r.fluxes[&member.biomass_reaction];
            if x > 1e-9 {
                assert!((biomass / x - mu).abs() < 1e-7, "{}: biomass/X must equal the requested µ", member.id);
            } else {
                assert!(biomass.abs() < 1e-8);
            }
        }
        let joint = spec.joint.as_ref().unwrap();
        let mut balances: IndexMap<&str, f64> = IndexMap::new();
        for (id, reaction) in &joint.reactions {
            let value = r.fluxes[id];
            let x = if reaction.species == "comm" { 1.0 } else { r.abundances[&reaction.species] };
            let scale = 1f64.max(reaction.lower_bound.abs()).max(reaction.upper_bound.abs());
            assert!(value >= reaction.lower_bound * x - 1e-5 * scale, "{id}: lower bound");
            assert!(value <= reaction.upper_bound * x + 1e-5 * scale, "{id}: upper bound");
            for (mid, c) in &reaction.metabolites {
                *balances.entry(mid).or_default() += c * value;
            }
        }
        for (mid, residual) in balances {
            assert!(residual.abs() < 1e-5, "{mid}: mass balance {residual}");
        }
    }
}

/// The joint range search with the app's options for the nine-species
/// community. Minutes in the JS; seconds here. Run with `--ignored`.
#[test]
#[ignore]
fn the_nine_species_range_search_finishes() {
    let spec = &gut9().spec;
    let t = std::time::Instant::now();
    let r = steady_com(
        spec,
        SteadyComOptions { upper_guess: 0.1, tolerance: 1e-5, biomass_cap: Some(2.0), time_limit_seconds: 180.0, ..Default::default() },
    )
    .unwrap();
    eprintln!("gut9 range: {} mu={} in {:.1} s, {} LPs", r.status.as_str(), r.growth_rate, t.elapsed().as_secs_f64(), r.iterations);
    assert_eq!(r.status, SteadyComStatus::Optimal, "{}", r.message);
}
