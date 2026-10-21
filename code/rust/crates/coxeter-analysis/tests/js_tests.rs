//! The JavaScript unit tests of the ported modules, ported:
//! `test/gimme.test.mjs`, `test/omics-overlay.test.mjs` (all but the renderer
//! test, which belongs to coxeter-render), `test/community-omics.test.mjs`,
//! `test/cross-feeding-explorer.test.mjs`, `test/stolyar.test.mjs`,
//! `test/zuniga-core.test.mjs`, the gut9.js tests of `test/gut9.test.mjs`
//! (its xlsx reader tests are in coxeter-io and its Figure 4 list test in
//! coxeter-solver), and the parts of `test/published-communities.test.mjs`
//! that do not drive the `Store`.
//!
//! Tests the JS keeps opt-in stay opt-in, through the same environment
//! variables: `COXETER_STOLYAR_DIR` (the four supplement files),
//! `COXETER_GUT9_DIR` (model9.xlsx and its two TXT exports),
//! `COXETER_REPRODUCE=1` with `COXETER_MODEL_DIR` (iAF1260.xml). Each test
//! names the JS test it ports.

mod common;

use std::sync::Arc;

use coxeter_analysis::community_view::{CommunityView, TransferRef};
use coxeter_analysis::cross_feeding_explorer::{
    CrossFeedingFilter, CrossFeedingOptions, build_cross_feeding_index, exchanged_metabolites, filter_cross_feeding_index,
};
use coxeter_analysis::gimme::{GimmeOptions, run_gimme};
use coxeter_analysis::gut9::{self, Gut9Community, Gut9ImportOptions, Gut9SpecOptions};
use coxeter_analysis::overlay::{
    CompareMetric, CompareOptions, OmicsCsvOptions, OmicsSpecies, ScaleKind, compare_reaction_scores, diverging_omics_colors,
    evaluate_gpr, omics_key, parse_gpr, parse_omics_csv, reaction_omics_scores, sequential_omics_colors,
};
use coxeter_analysis::published::{
    FRAMEWORKS, PUBLISHED_COMMUNITIES, ReproductionStatus, blocked_communities, find_community, reproduced_communities,
};
use coxeter_analysis::steadycom_ecoli::{self, STRAINS, amino_acid_exchange};
use coxeter_analysis::stolyar::{self, EXPECTED, JointOptions, SPECIES, StolyarFiles};
use coxeter_analysis::zuniga::{
    self, FetchResponse, ZunigaLoadOptions, ZunigaModels, ZunigaSourceModels, ZunigaStudy,
};
use coxeter_analysis::{Result, community_omics_mapping_report, parse_metatranscriptomics};
use coxeter_core::community::{CrossFeeding, CrossFeedingLink, ReactionOwner, ReactionRole};
use coxeter_core::{CommunityModel, CommunitySpec, Model, Objective, SpeciesSpec};
use coxeter_io::mat5::{MatValue, SparseMatrix, TypedArray, TypedArrayKind};
use coxeter_solver::fba::{FluxProblem, FvaOptions};
use coxeter_solver::gut9_figure4::{BT_CARBOHYDRATE, BT_FIBER, FP_FIBER, carbon_number, exchange_metabolite_of, fiber_constraints, figure_targets};
use coxeter_solver::steadycom::{AtOptions, Fiber, SteadyComOptions, max_total_biomass_at, steady_com};
use common::repo;
use indexmap::IndexMap;
use regex::Regex;
use serde_json::{Value, json};

fn model(doc: Value) -> Model {
    Model::from_value(&doc).unwrap()
}

fn scores(pairs: &[(&str, f64)]) -> IndexMap<String, f64> {
    pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
}

/// A community as the JS tests write one: only `merged.reactions` and
/// `reactionOwner` (and, for the explorer, transfers and a fixed cross-feeding).
#[derive(Default)]
struct Stub {
    owners: IndexMap<String, ReactionOwner>,
    transfers: IndexMap<String, Vec<(String, String)>>,
    links: Vec<CrossFeedingLink>,
}

impl Stub {
    fn owned(pairs: &[(&str, &str, &str)]) -> Stub {
        let owners = pairs
            .iter()
            .map(|(rid, sp, orig)| {
                (rid.to_string(), ReactionOwner { species_id: Some(sp.to_string()), original_id: orig.to_string(), role: ReactionRole::Metabolic })
            })
            .collect();
        Stub { owners, ..Default::default() }
    }
}

impl CommunityView for Stub {
    fn merged_reaction_ids(&self) -> Vec<&str> {
        self.owners.keys().map(String::as_str).collect()
    }
    fn reaction_owner(&self, id: &str) -> Option<&ReactionOwner> {
        self.owners.get(id)
    }
    fn transfers(&self, base: &str) -> Vec<TransferRef<'_>> {
        self.transfers.get(base).map_or_else(Vec::new, |l| l.iter().map(|(s, r)| TransferRef { species_id: s, reaction_id: r }).collect())
    }
    fn cross_feeding(&self, _: &IndexMap<String, f64>, _: f64) -> CrossFeeding {
        CrossFeeding { links: self.links.clone(), per_metabolite: Vec::new() }
    }
}

// gimme.test.mjs

fn gimme_toy() -> Model {
    model(json!({"id": "gimme", "compartments": {"c": "c"}, "metabolites": [{"id": "a_c", "compartment": "c"}],
        "reactions": [
            {"id": "UP", "metabolites": {"a_c": 1}, "lower_bound": 0, "upper_bound": 10},
            {"id": "BIOMASS", "metabolites": {"a_c": -1}, "lower_bound": 0, "upper_bound": 1000, "objective_coefficient": 1},
            {"id": "WASTE", "metabolites": {"a_c": -1}, "lower_bound": 0, "upper_bound": 1000},
        ]}))
}

/// 'GIMME preserves requested fraction of metabolic objective'
#[test]
fn gimme_preserves_requested_fraction_of_metabolic_objective() {
    let p = FluxProblem::new(&gimme_toy());
    let r = run_gimme(&p, &scores(&[("UP", 10.0), ("BIOMASS", 10.0), ("WASTE", 0.0)]), &GimmeOptions { threshold: Some(5.0), objective_fraction: 0.9 })
        .unwrap();
    assert_eq!(r.solution.status.as_str(), "optimal");
    assert!(r.solution.objective_value >= 9.0 - 1e-7);
    assert_eq!(r.solution.fluxes["WASTE"], 0.0);
}

/// 'GIMME auto threshold is expression P25 and exposes penalties'
#[test]
fn gimme_auto_threshold_is_expression_p25_and_exposes_penalties() {
    let p = FluxProblem::new(&gimme_toy());
    let r = run_gimme(&p, &scores(&[("UP", 8.0), ("BIOMASS", 4.0), ("WASTE", 0.0)]), &GimmeOptions { threshold: None, objective_fraction: 0.8 })
        .unwrap();
    assert!(r.threshold.is_some_and(f64::is_finite));
    assert!(r.penalties["WASTE"] > 0.0);
}

/// 'GIMME exposes contextual duals and directional reduced costs'
#[test]
fn gimme_exposes_contextual_duals_and_directional_reduced_costs() {
    let p = FluxProblem::new(&gimme_toy());
    let r = run_gimme(&p, &scores(&[("UP", 10.0), ("BIOMASS", 10.0), ("WASTE", 0.0)]), &GimmeOptions { threshold: Some(5.0), objective_fraction: 0.9 })
        .unwrap();
    assert_eq!(r.solution.status.as_str(), "optimal");
    assert!(r.solution.dual_analysis.available);
    assert!(r.to_json()["dualAnalysis"]["context"].as_str().unwrap().contains("GIMME"));
    assert_eq!(r.solution.shadow_prices.len(), p.met_ids.len());
    assert_eq!(r.solution.reduced_costs.len(), p.rxn_ids.len());
    assert_eq!(r.solution.scaled_reduced_costs.len(), p.rxn_ids.len());
    assert_eq!(r.directional_reduced_costs.len(), p.rxn_ids.len());
    assert!(r.objective_constraint_shadow_price.is_some_and(f64::is_finite));
    for value in r.solution.scaled_reduced_costs.values() {
        assert!(value.abs() <= 1.0 + 1e-12);
    }
}

// omics-overlay.test.mjs

/// 'omics CSV accepts wide one- or multi-condition tables and optional species'
#[test]
fn omics_csv_accepts_wide_one_or_multi_condition_tables_and_optional_species() {
    let data = parse_omics_csv("species,gene,Control,Treatment\nA,g1,10,20\nA,g2,4,2\nB,g1,8,16\n", &OmicsCsvOptions::default()).unwrap();
    assert_eq!(data.conditions, ["Control", "Treatment"]);
    assert_eq!(data.row_count, 3);
    assert_eq!(data.values["A\u{0}g1"].values["Treatment"], 20.0);
}

/// 'GPR numeric evaluation uses AND=min and OR=max'
#[test]
fn gpr_numeric_evaluation_uses_and_min_and_or_max() {
    let values = scores(&[("g1", 8.0), ("g2", 3.0), ("g3", 5.0)]);
    let get = |id: &str| values.get(id).copied();
    assert_eq!(evaluate_gpr(parse_gpr("(g1 and g2) or g3").as_ref(), &get), Some(5.0));
    assert_eq!(evaluate_gpr(parse_gpr("g1 and missing").as_ref(), &get), None);
    assert_eq!(evaluate_gpr(parse_gpr("g1 or missing").as_ref(), &get), Some(8.0));
}

/// 'gene/protein identifiers map through local reaction GPRs with species disambiguation'
#[test]
fn gene_protein_identifiers_map_through_local_reaction_gprs_with_species_disambiguation() {
    let m = model(json!({"id": "m", "compartments": {"c": "cytosol"},
        "metabolites": [{"id": "a_c", "compartment": "c"}, {"id": "b_c", "compartment": "c"}],
        "reactions": [
            {"id": "R1", "metabolites": {"a_c": -1, "b_c": 1}, "gene_reaction_rule": "g1 and g2"},
            {"id": "R2", "metabolites": {"b_c": -1, "a_c": 1}, "gene_reaction_rule": "g3 or g4"},
        ]}));
    let community = Stub::owned(&[("sp@R1", "sp", "R1"), ("sp@R2", "sp", "R2")]);
    let models: IndexMap<String, Model> = [("model".to_string(), m)].into_iter().collect();
    let data = parse_omics_csv(
        "species,protein,C1\nsp,g1,10\nsp,g2,4\nsp,g3,2\nsp,g4,7\n",
        &OmicsCsvOptions { data_type: "protein".into(), ..Default::default() },
    )
    .unwrap();
    let s = reaction_omics_scores(&community, &[OmicsSpecies::new("sp", "model")], &models, Some(&data), "C1");
    assert_eq!(s["sp@R1"], 4.0);
    assert_eq!(s["sp@R2"], 7.0);
}

/// 'two-condition comparison supports log2 fold change and direct difference'
#[test]
fn two_condition_comparison_supports_log2_fold_change_and_direct_difference() {
    let a = scores(&[("R1", 3.0), ("R2", 7.0)]);
    let b = scores(&[("R1", 7.0), ("R2", 3.0)]);
    let fc = compare_reaction_scores(&a, &b, &CompareOptions { metric: CompareMetric::Log2Fc, pseudocount: 1.0 });
    assert_eq!(coxeter_core::js::to_fixed(fc["R1"], 6), "1.000000");
    assert_eq!(coxeter_core::js::to_fixed(fc["R2"], 6), "-1.000000");
    let delta = compare_reaction_scores(&a, &b, &CompareOptions { metric: CompareMetric::Difference, pseudocount: 1.0 });
    assert_eq!(delta["R1"], 4.0);
    assert_eq!(delta["R2"], -4.0);
}

/// 'omics colour scales are sequential for one condition and diverging for comparisons'
#[test]
fn omics_colour_scales_are_sequential_for_one_condition_and_diverging_for_comparisons() {
    let one = sequential_omics_colors(&scores(&[("R1", 1.0), ("R2", 10.0)]), false);
    assert_ne!(one.colors["R1"], one.colors["R2"]);
    assert_eq!(one.kind, ScaleKind::Sequential);
    let compare = diverging_omics_colors(&scores(&[("R1", -2.0), ("R2", 0.0), ("R3", 2.0)]), false);
    assert_eq!(compare.kind, ScaleKind::Diverging);
    assert_ne!(compare.colors["R1"], compare.colors["R3"]);
}

// community-omics.test.mjs

/// 'long metatranscriptomics CSV maps MAG, gene, condition and expression'
#[test]
fn long_metatranscriptomics_csv_maps_mag_gene_condition_and_expression() {
    let parsed = parse_metatranscriptomics(
        "MAG,gene,condition,TPM\nMAG1,g1,Control,10\nMAG1,g1,Treatment,40\nMAG1,g2,Control,5\nMAG1,g2,Treatment,10\n",
        None,
    )
    .unwrap();
    assert_eq!(parsed.format.as_str(), "long");
    assert_eq!(parsed.dataset.conditions, ["Control", "Treatment"]);
    assert_eq!(parsed.dataset.values["MAG1\u{0}g1"].values["Treatment"], 40.0);
}

/// 'TSV and common expression headers are accepted'
#[test]
fn tsv_and_common_expression_headers_are_accepted() {
    let parsed = parse_metatranscriptomics("species\tlocus_tag\tcondition\tcounts\nsp\tg1\tA\t7\n", None).unwrap();
    assert_eq!(parsed.dataset.values[&omics_key("sp", "g1")].values["A"], 7.0);
}

/// 'community report is species-aware and reports reaction coverage'
#[test]
fn community_report_is_species_aware_and_reports_reaction_coverage() {
    let m = model(json!({"id": "m", "compartments": {"c": "c"}, "genes": [{"id": "g1"}, {"id": "g2"}],
        "metabolites": [{"id": "a_c", "compartment": "c"}, {"id": "b_c", "compartment": "c"}],
        "reactions": [{"id": "R", "metabolites": {"a_c": -1, "b_c": 1}, "gene_reaction_rule": "g1 and g2"}]}));
    let community = Stub::owned(&[("MAG1@R", "MAG1", "R")]);
    let species = [OmicsSpecies::new("MAG1", "m")];
    let models: IndexMap<String, Model> = [("m".to_string(), m)].into_iter().collect();
    let dataset = parse_metatranscriptomics("MAG,gene,condition,expression\nMAG1,g1,A,8\nMAG1,g2,A,3\n", None).unwrap().dataset;
    let s = reaction_omics_scores(&community, &species, &models, Some(&dataset), "A");
    assert_eq!(s["MAG1@R"], 3.0);
    let report = community_omics_mapping_report(&community, &species, &models, &dataset);
    assert_eq!(report.measured_genes, 2);
    assert_eq!(report.mapped_reactions, 1);
    assert_eq!(report.reaction_coverage, 1.0);
}

/// 'unmatched MAGs are explicit in mapping report'
#[test]
fn unmatched_mags_are_explicit_in_mapping_report() {
    let m = model(json!({"id": "m", "compartments": {"c": "c"}, "metabolites": [{"id": "a_c", "compartment": "c"}],
        "reactions": [{"id": "EX_a_c", "metabolites": {"a_c": -1}}]}));
    let community = Stub::default();
    let models: IndexMap<String, Model> = [("m".to_string(), m)].into_iter().collect();
    let dataset = parse_metatranscriptomics("MAG,gene,condition,expression\nUNKNOWN,g1,A,8\n", None).unwrap().dataset;
    let report = community_omics_mapping_report(&community, &[OmicsSpecies::new("MAG1", "m")], &models, &dataset);
    assert_eq!(report.unmatched_species, ["UNKNOWN"]);
}

// cross-feeding-explorer.test.mjs

fn link(base: &str, from: Option<&str>, to: Option<&str>, flux: f64) -> CrossFeedingLink {
    CrossFeedingLink { base: base.into(), from: from.map(str::to_string), to: to.map(str::to_string), flux }
}

fn explorer_community() -> Stub {
    Stub {
        transfers: [("lac".to_string(), vec![("A".to_string(), "A@EX_lac".to_string()), ("B".to_string(), "B@EX_lac".to_string())])]
            .into_iter()
            .collect(),
        links: vec![link("lac", Some("A"), Some("B"), 3.0), link("ac", Some("A"), Some("B"), 1.0), link("lac", None, Some("B"), 2.0)],
        ..Default::default()
    }
}

/// 'builds active biological cross-feeding events and excludes environment'
#[test]
fn builds_active_biological_cross_feeding_events_and_excludes_environment() {
    let x = build_cross_feeding_index(&explorer_community(), &IndexMap::new(), &CrossFeedingOptions::default());
    assert_eq!(x.events.len(), 2);
    assert_eq!(x.events[0].base, "lac");
    assert_eq!(x.events[0].producer_transfer.as_deref(), Some("A@EX_lac"));
    assert_eq!(x.events[0].consumer_transfer.as_deref(), Some("B@EX_lac"));
    assert_eq!(x.pairs[0].total_flux, 4.0);
}

/// 'filters by metabolite producer consumer flux and query'
#[test]
fn filters_by_metabolite_producer_consumer_flux_and_query() {
    let x = build_cross_feeding_index(&explorer_community(), &IndexMap::new(), &CrossFeedingOptions::default());
    let filter = |f: CrossFeedingFilter| filter_cross_feeding_index(Some(&x), &f).len();
    assert_eq!(filter(CrossFeedingFilter { metabolite: "lac".into(), ..Default::default() }), 1);
    assert_eq!(filter(CrossFeedingFilter { producer: "A".into(), consumer: "B".into(), min_flux: 2.0, ..Default::default() }), 1);
    assert_eq!(filter(CrossFeedingFilter { query: "ac".into(), ..Default::default() }), 2);
}

/// 'exchanged metabolites come in ecological order: acids and lactate, carbon sources, other products, the rest'
#[test]
fn exchanged_metabolites_come_in_ecological_order() {
    let l = |base: &str, flux: f64| link(base, Some("A"), Some("B"), flux);
    let links = vec![
        l("nh4", 9.0), l("glc-D", 0.5), l("lac-D", 0.2), l("for", 3.0), l("ac", 0.1), link("ac", Some("C"), Some("B"), 0.3),
        l("galur", 2.0), l("ala-L", 5.0), l("lac-L", 1.0), l("h", 50.0), link("but", None, Some("B"), 0.01),
    ];
    let list = exchanged_metabolites(&links, |base| base == "h");
    let bases: Vec<&str> = list.iter().map(|m| m.base.as_str()).collect();
    assert_eq!(bases, ["ac", "lac-L", "lac-D", "galur", "glc-D", "for", "nh4", "ala-L"]);
    assert_eq!(list[0].lines, 2, "acetate has two producer-consumer lines");
    assert!((list[0].flux - 0.4).abs() < 1e-12);
    assert_eq!(list.iter().find(|m| m.base == "glc-D").unwrap().group, "Carbon sources");
    assert!(!list.iter().any(|m| m.base == "but"), "a medium uptake is not an exchange between species");
}

// stolyar.test.mjs

/// 'a term keeps an identifier that contains a plus sign'
#[test]
fn a_term_keeps_an_identifier_that_contains_a_plus_sign() {
    let terms = stolyar::parse_terms("6 H+ + 1 H2S");
    assert_eq!(terms.iter().map(|t| (t.coefficient, t.metabolite.as_str())).collect::<Vec<_>>(), [(6.0, "H+"), (1.0, "H2S")]);
    let ids: Vec<String> = stolyar::parse_terms("1 ATP + 1 SO4 + 1 H+").into_iter().map(|t| t.metabolite).collect();
    assert_eq!(ids, ["ATP", "SO4", "H+"]);
    let terms = stolyar::parse_terms("1 Fdx(red)");
    assert_eq!((terms[0].coefficient, terms[0].metabolite.as_str()), (1.0, "Fdx(red)"));
    assert!(stolyar::parse_terms("").is_empty());
}

/// 'an equation reads substrates negative and products positive'
#[test]
fn an_equation_reads_substrates_negative_and_products_positive() {
    let eq = |s: &str| json!(stolyar::parse_equation(s));
    assert_eq!(eq("1 GAP + 1 ADP = 1 3PG + 1 ATP + 1 NADH"), json!({"GAP": -1.0, "ADP": -1.0, "3PG": 1.0, "ATP": 1.0, "NADH": 1.0}));
    // Either side may be empty: that is how this format writes a drain or a source.
    assert_eq!(eq("1 H2(exch) = "), json!({"H2(exch)": -1.0}));
    assert_eq!(eq(" = 1 ZM_Ala"), json!({"ZM_Ala": 1.0}));
    assert_eq!(eq(""), json!({}));
    assert_eq!(eq("0.5 ATP = 0.5 ADP + 1 Lac"), json!({"ATP": -0.5, "ADP": 0.5, "Lac": 1.0}));
}

/// 'the macromolecule file reads consumed as positive'
#[test]
fn the_macromolecule_file_reads_consumed_as_positive() {
    let parsed = stolyar::parse_macromolecule_synthesis("RNA = 6.17 Asp + -3.47 fum + 32.41 ATP\n").unwrap();
    assert_eq!(parsed["RNA"]["Asp"], -6.17, "a positive coefficient is consumed");
    assert_eq!(parsed["RNA"]["fum"], 3.47, "a negative coefficient is produced");
    assert_eq!(parsed["RNA"]["ATP"], -32.41);
}

const TINY_REACTIONS: &str = "Dsrc      \t = 1 S \t| \t# \t0  10    0 \t0 0 1 1 \t0.01\n\
Dmake     \t1 S = 1 A + 1 SHARED \t| \t# \t0  100    0 \t0 0 1 1 \t0.01\n\
mue       \t \t \t| \t# \t0  100   -1 \t0 0 1 1 \t0.01\n\
Mtake     \t1 SHARED = 1 ZM_A \t| \t# \t0  100    0 \t0 0 3 1 \t0.01\n\
MC_BIO    \t1 ZM_A = 1 ZM_BIOMASS \t| \t# \t0  100   -0.1 \t0 0 3 1 \t0.01\n\
Sink      \t1 SHARED =  \t| \t0 \t0  100    0 \t0 0 3 1 \t0.01";
const TINY_METABOLITES: &str = "S \tsubstrate \t0.001 \t0\nA \tproduct \t0.001 \t0\nSHARED \tshared \t0.001 \t0\n\
ZM_A \tmethanogen product \t0.001 \t0\nZM_BIOMASS \tmethanogen biomass \t0.001 \t1";

fn tiny(reactions: &str) -> StolyarFiles<'_> {
    StolyarFiles { reactions, metabolites: TINY_METABOLITES, macromolecules: "BIO \tBiomass \t1 \t0 0 2 1", synthesis: "BIO    = 2 A" }
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

/// 'a miniature model splits, shares and assembles its biomass'
#[test]
fn a_miniature_model_splits_shares_and_assembles_its_biomass() {
    let model = stolyar::import_stolyar(&tiny(TINY_REACTIONS)).unwrap();
    // The macromolecule becomes a metabolite, its synthesis a reaction, and the
    // empty growth reaction gains the fractions.
    assert!(model.metabolites.contains_key("BIO"));
    assert_eq!(json!(model.reactions["BIO_syn"].metabolites), json!({"A": -2.0, "BIO": 1.0}));
    assert_eq!(json!(model.reactions["mue"].metabolites), json!({"BIO": -1.0}));
    // Shared is derived: both organisms' reactions touch SHARED, nothing else.
    assert_eq!(model.shared.iter().collect::<Vec<_>>(), ["SHARED"]);
    let by = stolyar::reactions_by_species(&model);
    assert_eq!(sorted(by.by_species["Dv"].clone()), ["BIO_syn", "Dmake", "Dsrc", "mue"]);
    assert_eq!(sorted(by.by_species["Mm"].clone()), ["MC_BIO", "Mtake"]);
    // A reaction touching nothing but the shared pool belongs to the pair.
    assert_eq!(by.community, ["Sink"]);
    // CellNetAnalyzer minimises, this project maximises, so the signs flip.
    assert_eq!(json!(stolyar::objective_of(&model)), json!({"mue": 1.0, "MC_BIO": 0.1}));
    // The measured-rate column is a constraint: Sink is measured at 0.
    let joint = stolyar::to_joint_cobra_json(&model, &JointOptions::default());
    let sink = joint["reactions"].as_array().unwrap().iter().find(|r| r["id"] == "Sink").unwrap();
    assert_eq!(sink["lower_bound"], json!(0.0));
    assert_eq!(sink["upper_bound"], json!(0.0));
    // Every external metabolite gains exactly one exchange, and nothing else is added.
    assert_eq!(stolyar::external_metabolites(&model), ["ZM_BIOMASS"]);
    assert!(joint["reactions"].as_array().unwrap().iter().any(|r| r["id"] == "EX_ZM_BIOMASS"));
    assert_eq!(joint["reactions"].as_array().unwrap().len(), model.reactions.len() + 1);
}

/// 'a miniature model solves, and both partners grow only together'
#[test]
fn a_miniature_model_solves_and_both_partners_grow_only_together() {
    let model = stolyar::import_stolyar(&tiny(TINY_REACTIONS)).unwrap();
    let joint = Model::from_value(&stolyar::to_joint_cobra_json(&model, &JointOptions::default())).unwrap();
    let solution = FluxProblem::new(&joint).optimize();
    assert_eq!(solution.status.as_str(), "optimal");
    assert!(solution.fluxes["mue"] > 0.0, "D. vulgaris side must grow");
    assert!(solution.fluxes["MC_BIO"] > 0.0, "the methanogen side must grow");
    // Cut the one link between them and the methanogen stops.
    let mut cut = FluxProblem::new(&joint);
    cut.set_bounds("Mtake", Some(0.0), Some(0.0)).unwrap();
    assert_eq!(cut.optimize().fluxes["MC_BIO"], 0.0, "with the shared metabolite cut, nothing feeds it");
}

/// 'the split members keep the shared pool outside the cells'
#[test]
fn the_split_members_keep_the_shared_pool_outside_the_cells() {
    let model = stolyar::import_stolyar(&tiny(TINY_REACTIONS)).unwrap();
    let species = SPECIES
        .iter()
        .map(|sp| {
            let m = Model::from_value(&stolyar::to_species_cobra_json(&model, sp.id, None).unwrap()).unwrap();
            SpeciesSpec { id: Some(sp.id.into()), model: Arc::new(m), abundance: Some(0.5), enabled: Some(true), color: None }
        })
        .collect();
    let community = CommunityModel::new(CommunitySpec {
        name: Some("tiny".into()),
        species,
        medium: IndexMap::new(),
        objective: Objective::parse("equal-growth"),
        product_target: None,
    })
    .unwrap();
    // Each member carries its own copy of SHARED plus an exchange for it, which is
    // what lets the community layer build one medium between the cells.
    for sp in SPECIES {
        let doc = stolyar::to_species_cobra_json(&model, sp.id, None).unwrap();
        assert!(doc["reactions"].as_array().unwrap().iter().any(|r| r["id"] == format!("EX_SHARED_{}", sp.id)));
        let shared = doc["metabolites"].as_array().unwrap().iter().find(|m| m["id"] == "SHARED").unwrap();
        assert_eq!(shared["compartment"], "e");
    }
    let tree = community.merged.compartment_tree();
    assert_eq!(tree.get("s").map(|n| n.depth), Some(0), "the shared medium sits outside every cell");
    assert!(community.transfers_by_base.contains_key("SHARED"));
}

/// 'a reaction whose layer and prefix disagree is refused'
#[test]
fn a_reaction_whose_layer_and_prefix_disagree_is_refused() {
    let broken = TINY_REACTIONS.replace(
        "MC_BIO    \t1 ZM_A = 1 ZM_BIOMASS \t| \t# \t0  100   -0.1 \t0 0 3 1 \t0.01",
        "MC_BIO    \t1 ZM_A = 1 ZM_BIOMASS \t| \t# \t0  100   -0.1 \t0 0 1 1 \t0.01",
    );
    let err = stolyar::import_stolyar(&tiny(&broken)).unwrap_err();
    assert!(Regex::new(r"D\. vulgaris layer but names ZM_").unwrap().is_match(&err.message), "{}", err.message);
}

/// 'a reaction naming an undeclared metabolite is refused'
#[test]
fn a_reaction_naming_an_undeclared_metabolite_is_refused() {
    let broken = format!("{TINY_REACTIONS}\nBad \t1 S = 1 GHOST \t| \t# \t0 100 0 \t0 0 1 1 \t0.01");
    assert!(stolyar::import_stolyar(&tiny(&broken)).unwrap_err().message.contains("GHOST"));
}

/// The supplement: `COXETER_STOLYAR_DIR`, or the copy fixtures-analysis.mjs
/// rebuilds from the bundled joint model in rust/fixtures/large.
fn supplement() -> Option<[String; 4]> {
    let dir = std::env::var("COXETER_STOLYAR_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| repo("rust/fixtures/large/analysis/stolyar-reconstructed"));
    let read = |f: &str| std::fs::read_to_string(dir.join(f)).ok();
    Some([read("reactions.txt")?, read("metabolites.txt")?, read("macromolecules.txt")?, read("macromolecule_synthesis.txt")?])
}

fn supplement_files(s: &[String; 4]) -> StolyarFiles<'_> {
    StolyarFiles { reactions: &s[0], metabolites: &s[1], macromolecules: &s[2], synthesis: &s[3] }
}

/// 'the supplement imports with the counts it should'
#[test]
fn the_supplement_imports_with_the_counts_it_should() {
    let Some(s) = supplement() else { return eprintln!("skipped: no Stolyar supplement (COXETER_STOLYAR_DIR)") };
    let model = stolyar::import_stolyar(&supplement_files(&s)).unwrap();
    let report = stolyar::validate_stolyar(&model);
    assert!(report.problems.is_empty(), "{:?}", report.problems);
    assert_eq!(model.reactions.len() - model.macromolecules.len(), EXPECTED.reactions);
    assert_eq!(model.shared.len(), EXPECTED.shared_metabolites);
}

/// 'the shared metabolites are the four the mutualism runs on'
#[test]
fn the_shared_metabolites_are_the_four_the_mutualism_runs_on() {
    let Some(s) = supplement() else { return eprintln!("skipped: no Stolyar supplement (COXETER_STOLYAR_DIR)") };
    let model = stolyar::import_stolyar(&supplement_files(&s)).unwrap();
    assert_eq!(sorted(model.shared.iter().cloned().collect()), ["AcOH(exch)", "Form(exch)", "H2(exch)", "NH3(ext)"]);
    let structure = stolyar::exchange_structure(&model);
    let hydrogen = structure.iter().find(|x| x.metabolite == "H2(exch)").unwrap();
    assert!(hydrogen.producers.iter().any(|p| p == "Dv"), "D. vulgaris releases the hydrogen");
    assert!(hydrogen.consumers.iter().any(|p| p == "Mm"), "the methanogen takes it");
}

fn supplement_joint(s: &[String; 4]) -> Model {
    let model = stolyar::import_stolyar(&supplement_files(s)).unwrap();
    Model::from_value(&stolyar::to_joint_cobra_json(&model, &JointOptions::default())).unwrap()
}

/// 'without sulfate the partnership is obligate; with it, it is not'
#[test]
fn without_sulfate_the_partnership_is_obligate_with_it_it_is_not() {
    let Some(s) = supplement() else { return eprintln!("skipped: no Stolyar supplement (COXETER_STOLYAR_DIR)") };
    let joint = supplement_joint(&s);
    let mut syntrophic = FluxProblem::new(&joint);
    syntrophic.set_bounds("EX_SO4", Some(0.0), Some(1000.0)).unwrap();
    let without = syntrophic.optimize();
    assert_eq!(without.status.as_str(), "optimal");
    assert!(without.fluxes["mue"] > 0.0, "D. vulgaris grows");
    assert!(without.fluxes["MC_BIO"] > 0.0, "so does the methanogen");
    assert!(without.fluxes["M12"] > 0.0, "and it makes methane");
    assert_eq!(without.fluxes["D01"], 0.0, "with no sulfate, none is reduced");
    // Give D. vulgaris an acceptor of its own and it stops feeding its partner.
    let mut respiring = FluxProblem::new(&joint);
    respiring.set_bounds("EX_SO4", Some(-1000.0), Some(1000.0)).unwrap();
    let with = respiring.optimize();
    assert!(with.fluxes["D01"] > 0.0, "sulfate is reduced when it is there");
    assert!(with.fluxes["MC_BIO"].abs() < 1e-9, "and the methanogen does not grow");
    assert!(with.fluxes["mue"] > without.fluxes["mue"], "D. vulgaris does better alone than in the partnership");
}

/// 'the carrier is not determined, and the growth rates are'
#[test]
fn the_carrier_is_not_determined_and_the_growth_rates_are() {
    let Some(s) = supplement() else { return eprintln!("skipped: no Stolyar supplement (COXETER_STOLYAR_DIR)") };
    let joint = supplement_joint(&s);
    let mut problem = FluxProblem::new(&joint);
    problem.set_bounds("EX_SO4", Some(0.0), Some(1000.0)).unwrap();
    problem.optimize();
    let ranges = problem.variability(&["mue", "MC_BIO", "M12", "D81", "D80"], &FvaOptions { fraction: 1.0, ..Default::default() });
    let span = |id: &str| ranges[id].max - ranges[id].min;
    assert!(span("mue") < 1e-6, "the D. vulgaris growth rate is pinned");
    assert!(span("MC_BIO") < 1e-6, "so is the methanogen's");
    assert!(span("M12") < 1e-6, "and the methane");
    assert!(span("D81") > 1.0, "the hydrogen transfer is free to move");
    assert!(span("D80") > 1.0, "and the formate takes up the slack");
}

// gut9.test.mjs (gut9.js)

/// 'the metadata cites the paper the model comes from'
#[test]
fn the_metadata_cites_the_paper_the_model_comes_from() {
    let p = gut9::PROVENANCE;
    assert_eq!(p.paper.doi, "10.1371/journal.pcbi.1005539");
    assert_eq!(p.paper.year, 2017);
    assert_eq!(p.paper.authors, "Chan SHJ, Simons MN, Maranas CD");
    assert_eq!(p.paper.journal, "PLoS Computational Biology");
    assert_eq!(p.paper.framework, "SteadyCom");
    // All five files the work is allowed to draw on are named.
    let mut files = p.files.all().to_vec();
    files.sort();
    assert_eq!(files, ["model9.xlsx", "model9_readme.txt", "model9_rxns_params.txt", "model9_stoich.txt", "simFigure4.m"]);
}

/// 'parseBounds reads the TXT export and rejects a foreign file'
#[test]
fn parse_bounds_reads_the_txt_export_and_rejects_a_foreign_file() {
    let bounds = gut9::parse_bounds("rxns\trev\tlb\tub\tc\nR1\t1\t-1.0e+00\t2.5e+00\t0\nR2\t0\t0\t1e+03\t1\n").unwrap();
    assert_eq!(bounds.len(), 2);
    let r1 = bounds["R1"];
    assert_eq!((r1.reversible, r1.lower_bound, r1.upper_bound, r1.objective), (true, -1.0, 2.5, 0.0));
    assert_eq!(bounds["R2"].objective, 1.0);
    assert!(gut9::parse_bounds("something\telse\n").unwrap_err().message.contains("unexpected header"));
}

struct Gut9Files {
    xlsx: Vec<u8>,
    bounds_text: String,
    stoich_text: String,
    model: Gut9Community,
}

fn gut9_files() -> Option<Gut9Files> {
    let dir = std::path::PathBuf::from(std::env::var("COXETER_GUT9_DIR").ok()?);
    let xlsx = std::fs::read(dir.join("model9.xlsx")).ok()?;
    let bounds_text = std::fs::read_to_string(dir.join("model9_rxns_params.txt")).ok()?;
    let stoich_text = std::fs::read_to_string(dir.join("model9_stoich.txt")).ok()?;
    let model = gut9::import_gut9(&xlsx, &Gut9ImportOptions { bounds_text: Some(&bounds_text) }).unwrap();
    Some(Gut9Files { xlsx, bounds_text, stoich_text, model })
}

const GUT9_SKIP: &str = "skipped: set COXETER_GUT9_DIR to the folder holding model9.xlsx, model9_rxns_params.txt and model9_stoich.txt";

/// 'the model has exactly nine species, each with its biomass and maintenance'
#[test]
fn the_model_has_exactly_nine_species_each_with_its_biomass_and_maintenance() {
    let Some(loaded) = gut9_files() else { return eprintln!("{GUT9_SKIP}") };
    let model = &loaded.model;
    assert_eq!(model.species.len(), gut9::EXPECTED.species);
    assert_eq!(model.species.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["Bt", "Fp", "Kp", "St", "Ba", "Ec", "Ef", "Er", "Lc"]);
    for sp in &model.species {
        let biomass = &model.reactions[sp.biomass_reaction.as_deref().unwrap()];
        let atpm = &model.reactions[sp.atp_maintenance.as_deref().unwrap()];
        assert_eq!(biomass.species.as_deref(), Some(sp.id.as_str()));
        assert_eq!(atpm.species.as_deref(), Some(sp.id.as_str()));
        // Maintenance is a floor, not an option.
        assert_eq!((atpm.lower_bound, atpm.upper_bound), (1.0, 1000.0), "{}: ATPM bounds", sp.id);
        assert_ne!(biomass.objective, 0.0, "{}: biomass is not in the objective", sp.id);
    }
    assert_eq!(model.reactions.values().filter(|r| r.objective != 0.0).count(), gut9::EXPECTED.species);
}

/// 'the imported dimensions are the ones the workbook declares'
#[test]
fn the_imported_dimensions_are_the_ones_the_workbook_declares() {
    let Some(loaded) = gut9_files() else { return eprintln!("{GUT9_SKIP}") };
    let model = &loaded.model;
    assert_eq!(model.reactions.len(), gut9::EXPECTED.reactions);
    assert_eq!(model.metabolites.len(), gut9::EXPECTED.metabolites);
    assert_eq!(model.stoichiometry_entries, gut9::EXPECTED.stoichiometry_entries);
    let by_species = gut9::reactions_by_species(model).unwrap();
    for sp in &model.species {
        let mets = model.metabolites.values().filter(|m| m.species.as_deref() == Some(sp.id.as_str())).count();
        assert_eq!(by_species[&sp.id].len() as f64, sp.declared_reactions, "{} reactions", sp.id);
        assert_eq!(mets as f64, sp.declared_metabolites, "{} metabolites", sp.id);
    }
    let species_total: usize = by_species.values().map(Vec::len).sum();
    assert_eq!(species_total + gut9::community_reaction_ids(model).len(), gut9::EXPECTED.reactions);
}

/// 'the stoichiometric matrix matches model9_stoich.txt exactly'
#[test]
fn the_stoichiometric_matrix_matches_model9_stoich_txt_exactly() {
    let Some(loaded) = gut9_files() else { return eprintln!("{GUT9_SKIP}") };
    let report = gut9::validate_stoichiometry(&loaded.model, &loaded.stoich_text).unwrap();
    assert_eq!(report.entries, gut9::EXPECTED.stoichiometry_entries);
    assert_eq!(report.identical, gut9::EXPECTED.stoichiometry_entries);
    assert!(report.differing.is_empty() && report.only_in_text.is_empty() && report.only_in_model.is_empty());
}

/// 'every bound and objective coefficient matches model9_rxns_params.txt'
#[test]
fn every_bound_and_objective_coefficient_matches_model9_rxns_params_txt() {
    let Some(loaded) = gut9_files() else { return eprintln!("{GUT9_SKIP}") };
    let report = gut9::validate_bounds(&loaded.model, &loaded.bounds_text).unwrap();
    assert_eq!(report.identical, gut9::EXPECTED.reactions);
    assert!(report.differing.is_empty() && report.only_in_model.is_empty() && report.only_in_text.is_empty());
}

/// 'the workbook carries no diet, and the TXT carries it on 88 uptake reactions'
#[test]
fn the_workbook_carries_no_diet_and_the_txt_carries_it_on_88_uptake_reactions() {
    let Some(loaded) = gut9_files() else { return eprintln!("{GUT9_SKIP}") };
    let from_workbook = gut9::import_gut9(&loaded.xlsx, &Gut9ImportOptions::default()).unwrap();
    assert_eq!(gut9::diet(&from_workbook).len(), 0, "the workbook opens no uptake at all");
    let report = gut9::validate_bounds(&from_workbook, &loaded.bounds_text).unwrap();
    assert_eq!(report.differing.len(), gut9::EXPECTED.diet_reactions);
    assert!(report.differing.iter().all(|d| d.id.starts_with("UT_")));
    assert_eq!(report.identical, gut9::EXPECTED.reactions - gut9::EXPECTED.diet_reactions);
    assert_eq!(gut9::diet(&loaded.model).len(), gut9::EXPECTED.diet_reactions);
    assert_eq!(loaded.model.bound_changes.as_ref().unwrap().len(), gut9::EXPECTED.diet_reactions);
    assert_eq!(loaded.model.bounds_from, "model9_rxns_params.txt");
}

/// 'the community exchange space is species <-> medium <-> environment'
#[test]
fn the_community_exchange_space_is_species_medium_environment() {
    let Some(loaded) = gut9_files() else { return eprintln!("{GUT9_SKIP}") };
    let model = &loaded.model;
    assert_eq!(model.community_metabolites.len(), gut9::EXPECTED.community_metabolites);
    assert_eq!(model.community_uptake.len(), gut9::EXPECTED.community_metabolites);
    assert_eq!(model.community_export.len(), gut9::EXPECTED.community_metabolites);
    assert_eq!(model.species_exchange.len(), gut9::EXPECTED.species_exchange_reactions);
    assert_eq!(gut9::community_reaction_ids(model).len(), gut9::EXPECTED.community_exchange_reactions);
    assert_eq!(model.exsp_order, ["Bt", "Fp", "Kp", "St", "Ba", "Ec", "Ef", "Er", "Lc"]);
    for (met, rid) in &model.community_uptake {
        let rxn = &model.reactions[rid];
        assert_eq!(rxn.species.as_deref(), Some("comm"));
        assert_eq!(json!(rxn.metabolites), json!({met.clone(): 1.0}));
    }
    for (met, rid) in &model.community_export {
        let rxn = &model.reactions[rid];
        assert_eq!(rxn.species.as_deref(), Some("comm"));
        assert_eq!(json!(rxn.metabolites), json!({met.clone(): -1.0}));
    }
    for (key, rid) in &model.species_exchange {
        let (met, species) = key.split_once('\u{0}').unwrap();
        let rxn = &model.reactions[rid];
        assert_eq!(rxn.species.as_deref(), Some(species));
        assert_eq!(rxn.metabolites[met], 1.0);
        let other: Vec<(&String, &f64)> = rxn.metabolites.iter().filter(|(m, _)| *m != met).collect();
        assert_eq!(other.len(), 1);
        assert_eq!(*other[0].1, -1.0);
        assert_eq!(model.metabolites[other[0].0].species.as_deref(), Some(species));
    }
}

/// 'the Figure 4 reactions all exist and carry a carbon number'
#[test]
fn the_figure_4_reactions_all_exist_and_carry_a_carbon_number() {
    let Some(loaded) = gut9_files() else { return eprintln!("{GUT9_SKIP}") };
    let model = &loaded.model;
    let metabolite_of = exchange_metabolite_of(model);
    for (list, species) in [(&BT_FIBER[..], "Bt"), (&BT_CARBOHYDRATE[..], "Bt"), (&FP_FIBER[..], "Fp")] {
        for rid in list {
            assert_eq!(model.reactions[*rid].species.as_deref(), Some(species));
            let met = &metabolite_of[*rid];
            assert!(carbon_number(met).is_some_and(|c| c > 0.0), "{met} has no carbon number");
        }
    }
    let constraints = fiber_constraints(model, Fiber { bacteroides: 10.0, faecalibacterium: 5.0 }).unwrap();
    assert_eq!(constraints.len(), 2);
    assert_eq!(constraints[0].terms.len(), BT_FIBER.len());
    assert_eq!(constraints[1].terms.len(), FP_FIBER.len());
    assert!(constraints.iter().all(|c| c.terms.iter().all(|t| t.uptake_only)));
    assert_eq!(constraints[0].biomass.iter().map(|b| (b.species.as_str(), b.coef)).collect::<Vec<_>>(), [("Bt", -1.0)]);
    assert_eq!(constraints[0].hi, Some(0.0));
    let pect = constraints[0].terms.iter().find(|t| t.reaction == "EX_pect[u]tr_Bt").unwrap();
    assert_eq!(pect.coef, carbon_number("pect[u]").unwrap() / 10.0);
    let targets = figure_targets(model).unwrap();
    assert_eq!(targets.len(), gut9::EXPECTED.species + 2);
    let names: Vec<&str> = targets.iter().map(|t| t.name.as_str()).collect();
    let expected: Vec<String> = model.species.iter().map(|s| format!("Bm_{}", s.id)).collect();
    assert_eq!(names[..9], expected.iter().map(String::as_str).collect::<Vec<_>>()[..]);
    assert_eq!(names[9..], ["fiber_Bt_Cmmol", "carbohydrate_Bt_Cmmol"]);
}

/// 'the community is feasible and grows on the authors' diet'
#[test]
fn the_community_is_feasible_and_grows_on_the_authors_diet() {
    let Some(loaded) = gut9_files() else { return eprintln!("{GUT9_SKIP}") };
    if std::env::var("COXETER_REPRODUCE").as_deref() != Ok("1") {
        return eprintln!("skipped: set COXETER_REPRODUCE=1 to solve it (28 000-row LPs)");
    }
    let spec = gut9::to_steady_com_spec(&loaded.model, &Gut9SpecOptions::default()).unwrap();
    let at = max_total_biomass_at(&spec, 0.05, &AtOptions::default()).unwrap();
    assert_eq!(at.status.as_str(), "optimal", "{}", at.message);
    assert!((at.total - 1.0).abs() < 1e-6, "total biomass {}", at.total);
}

// zuniga-core.test.mjs

const EC_BIOMASS: &str = "BIOMASS_Ec_iJO1366_core_53p95M";

fn f64s(values: &[f64]) -> MatValue {
    MatValue::Numeric { shape: vec![values.len(), 1], data: TypedArray { kind: TypedArrayKind::Float64, values: values.to_vec() } }
}

fn texts(values: &[&str]) -> MatValue {
    MatValue::Cell(values.iter().map(|s| MatValue::Text(s.to_string())).collect())
}

struct RawReaction {
    id: String,
    metabolites: Vec<(String, f64)>,
    lb: f64,
    ub: f64,
    gpr: &'static str,
    objective: f64,
}

/// The JS test's own synthetic model, as the MAT reader would give it.
fn fixture_raw(coculture: bool) -> MatValue {
    let suffix = if coculture { "_D" } else { "" };
    let (food, pre, phe) = (format!("food_c{suffix}"), format!("pre_c{suffix}"), format!("phe__L_c{suffix}"));
    let mut mets = vec![food.clone(), pre.clone(), phe.clone()];
    let r = |id: &str, metabolites: Vec<(&String, f64)>, lb: f64, ub: f64, gpr: &'static str, objective: f64| RawReaction {
        id: format!("{id}{suffix}"),
        metabolites: metabolites.into_iter().map(|(m, c)| (m.clone(), c)).collect(),
        lb,
        ub,
        gpr,
        objective,
    };
    let mut reactions = vec![
        r("SOURCE", vec![(&food, 1.0)], 0.0, 3.0, "", 0.0),
        r("CHORM", vec![(&food, -1.0), (&pre, 1.0)], 0.0, 1000.0, "(b2599 or isozyme)", 0.0),
        r("PPNDH", vec![(&pre, -1.0), (&phe, 1.0)], 0.0, 1000.0, "b2599", 0.0),
        r(EC_BIOMASS, vec![(&phe, -1.0)], 0.0, 1000.0, "", 1.0),
        r("BROKEN_TEXT", vec![(&food, -1.0)], 0.0, 0.0, "((other)", 0.0),
    ];
    if coculture {
        mets.push("phe__L_c_A".into());
        mets.push("phe__L_[smp]".into());
        let plain = |id: &str, m: Vec<(&str, f64)>, lb: f64, ub: f64, objective: f64| RawReaction {
            id: id.into(),
            metabolites: m.into_iter().map(|(k, c)| (k.to_string(), c)).collect(),
            lb,
            ub,
            gpr: "",
            objective,
        };
        reactions.push(plain("SOURCE_A", vec![("phe__L_c_A", 1.0)], 0.0, 4.0, 0.0));
        reactions.push(plain("BOF_A", vec![("phe__L_c_A", -1.0)], 1.0, 1.0, 1.0));
        reactions.push(plain("PHELtex_A", vec![("phe__L_c_A", 1.0), ("phe__L_[smp]", -1.0)], -1000.0, 1000.0, 0.0));
        reactions.push(plain("PHEtex_D", vec![("phe__L_[smp]", -1.0), ("phe__L_c_D", 1.0)], -1000.0, 1000.0, 0.0));
        reactions.push(plain("EX_phe__L_[smp]", vec![("phe__L_[smp]", -1.0)], 0.0, 0.0, 0.0));
    }
    let row = |m: &str| mets.iter().position(|x| x == m).unwrap() as f64;
    let (mut ir, mut jc, mut values) = (Vec::new(), vec![0.0], Vec::new());
    for reaction in &reactions {
        let mut entries = reaction.metabolites.clone();
        entries.sort_by(|a, b| row(&a.0).total_cmp(&row(&b.0)));
        for (m, c) in entries {
            ir.push(row(&m));
            values.push(c);
        }
        jc.push(values.len() as f64);
    }
    let ids: Vec<&str> = reactions.iter().map(|x| x.id.as_str()).collect();
    let met_ids: Vec<&str> = mets.iter().map(String::as_str).collect();
    let typed = |kind, values: Vec<f64>| TypedArray { kind, values };
    let fields: IndexMap<String, MatValue> = [
        ("rxns", texts(&ids)),
        ("mets", texts(&met_ids)),
        ("genes", texts(&["b2599", "isozyme", "other"])),
        ("rxnNames", texts(&ids)),
        ("metNames", texts(&met_ids)),
        ("lb", f64s(&reactions.iter().map(|x| x.lb).collect::<Vec<_>>())),
        ("ub", f64s(&reactions.iter().map(|x| x.ub).collect::<Vec<_>>())),
        ("c", f64s(&reactions.iter().map(|x| x.objective).collect::<Vec<_>>())),
        ("b", f64s(&vec![0.0; mets.len()])),
        ("csense", MatValue::Text("E".repeat(mets.len()))),
        ("grRules", texts(&reactions.iter().map(|x| x.gpr).collect::<Vec<_>>())),
        ("rules", texts(&reactions.iter().map(|x| if x.id.starts_with("BROKEN_TEXT") { "x(3)" } else { "" }).collect::<Vec<_>>())),
        (
            "S",
            MatValue::Sparse(SparseMatrix {
                shape: vec![mets.len(), reactions.len()],
                ir: typed(TypedArrayKind::Uint32, ir),
                jc: typed(TypedArrayKind::Uint32, jc),
                values: typed(TypedArrayKind::Float64, values),
            }),
        ),
        ("description", MatValue::Text("Own synthetic validation fixture".into())),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    MatValue::Struct(fields)
}

fn field_mut<'a>(raw: &'a mut MatValue, name: &str) -> &'a mut MatValue {
    match raw {
        MatValue::Struct(f) => f.get_mut(name).unwrap(),
        _ => panic!("a structure"),
    }
}

fn fixture_study() -> ZunigaStudy {
    let mono = zuniga::convert_zuniga_mat_model(&fixture_raw(false), "toy-mono").unwrap();
    let joint = zuniga::convert_zuniga_mat_model(&fixture_raw(true), "toy-co").unwrap();
    let se = json!({"id": "toy-se", "metabolites": [{"id": "phe__L_c", "compartment": "c"}],
        "reactions": [{"id": "SOURCE", "metabolites": {"phe__L_c": 1}, "lower_bound": 0, "upper_bound": 4}]});
    ZunigaStudy {
        manifest: Value::Null,
        source_hashes: [("ownFixture".to_string(), "not-an-upstream-model".to_string())].into_iter().collect(),
        models: ZunigaModels { ec: Arc::new(model(mono.clone())), se: Arc::new(model(se)) },
        source_models: ZunigaSourceModels { mono, joint },
    }
}

/// 'GPR knockout preserves alternative isozymes and parses nested AND/OR without skipping terms'
#[test]
fn gpr_knockout_preserves_alternative_isozymes_and_parses_nested_and_or() {
    let deleted = ["b2599"];
    let eval = |rule: &str| zuniga::evaluate_zuniga_gene_rule(rule, &deleted);
    assert!(!eval("b2599").unwrap());
    assert!(eval("b2599 or isozyme").unwrap());
    assert!(eval("(b2599 and other) or (isozyme and other)").unwrap());
    assert!(!eval("b2599 and (isozyme or other)").unwrap());
    assert!(eval("").unwrap());
    for rule in ["b2599 or", "(b2599", "b2599 )", "b2599 other", "and b2599"] {
        assert!(eval(rule).is_err(), "{rule}");
    }
}

/// 'MAT converter preserves complete sparse coefficients and uses the valid indexed original GPR'
#[test]
fn mat_converter_preserves_complete_sparse_coefficients_and_uses_the_valid_indexed_original_gpr() {
    let raw = fixture_raw(true);
    let doc = zuniga::convert_zuniga_mat_model(&raw, "zuniga-source").unwrap();
    let count = |v: &Value| v.as_array().unwrap().len();
    assert_eq!(count(&doc["metabolites"]), 5);
    assert_eq!(count(&doc["reactions"]), 10);
    assert_eq!(count(&doc["genes"]), 3);
    let rule = doc["reactions"].as_array().unwrap().iter().find(|r| r["id"] == "BROKEN_TEXT_D").unwrap();
    assert_eq!(rule["gene_reaction_rule"], "other");
    assert_eq!(rule["notes"]["original_gr_rule"], "((other)");
    assert_eq!(rule["notes"]["original_rules"], "x(3)");
    assert_eq!(doc["reactions"][2]["metabolites"], json!({"pre_c_D": -1.0, "phe__L_c_D": 1.0}));
    let column = |key: &str| doc["reactions"].as_array().unwrap().iter().map(|r| r[key].as_f64().unwrap()).collect::<Vec<_>>();
    let raw_numbers = |name: &str| match &raw {
        MatValue::Struct(f) => f[name].numbers().unwrap().to_vec(),
        _ => unreachable!(),
    };
    assert_eq!(column("objective_coefficient"), raw_numbers("c"));
    assert_eq!(column("lower_bound"), raw_numbers("lb"));
    assert_eq!(column("upper_bound"), raw_numbers("ub"));
}

/// 'Source converter rejects nonzero mass-balance RHS, non-equality rows, and invalid bound vectors'
#[test]
fn source_converter_rejects_nonzero_rhs_non_equality_rows_and_invalid_bound_vectors() {
    let convert = |raw: &MatValue| zuniga::convert_zuniga_mat_model(raw, "zuniga-source").unwrap_err().message;
    let mut rhs = fixture_raw(false);
    if let MatValue::Numeric { data, .. } = field_mut(&mut rhs, "b") {
        data.values[0] = 1.0;
    }
    assert!(convert(&rhs).contains("RHS"));
    let mut sense = fixture_raw(false);
    *field_mut(&mut sense, "csense") = MatValue::Text("LEE".into());
    assert!(convert(&sense).contains("sense"));
    let mut bound = fixture_raw(false);
    if let MatValue::Numeric { data, .. } = field_mut(&mut bound, "lb") {
        data.values[0] = 4.0;
    }
    assert!(convert(&bound).contains("bounds"));
    let mut vector = fixture_raw(false);
    *field_mut(&mut vector, "ub") = f64s(&[3.0]);
    assert!(convert(&vector).contains("vectors"));
}

/// 'Malformed source GPR needs its valid indexed expression and never invents a replacement'
#[test]
fn malformed_source_gpr_needs_its_valid_indexed_expression() {
    let with_rule = |rule: &str| {
        let mut raw = fixture_raw(false);
        if let MatValue::Cell(items) = field_mut(&mut raw, "rules") {
            items[4] = MatValue::Text(rule.into());
        }
        zuniga::convert_zuniga_mat_model(&raw, "zuniga-source").unwrap_err().message
    };
    assert!(with_rule("").contains("No valid original GPR"));
    assert!(with_rule("x(9999)").contains("unknown gene"));
}

/// 'Source loader rejects failed downloads and unverified bytes before importing any model'
#[test]
fn source_loader_rejects_failed_downloads_and_unverified_bytes() {
    let manifest = json!({"sources": [{"file": "M-models.mat", "url": "https://example.invalid/own-fixture.mat", "sha256": "0".repeat(64)}]});
    let down = |_: &str| -> Result<FetchResponse> { Ok(FetchResponse { status: 503, body: Vec::new() }) };
    let err = zuniga::load_zuniga_study(&manifest, ZunigaLoadOptions { fetcher: &down, cache: None, on_progress: None }).unwrap_err();
    assert!(err.message.contains("HTTP 503"), "{}", err.message);
    let wrong = |_: &str| -> Result<FetchResponse> { Ok(FetchResponse { status: 200, body: vec![1, 2, 3, 4] }) };
    let err = zuniga::load_zuniga_study(&manifest, ZunigaLoadOptions { fetcher: &wrong, cache: None, on_progress: None }).unwrap_err();
    assert!(err.message.contains("integrity check failed"), "{}", err.message);
}

/// 'Canonical IDs keep independent species compartments and shared phenylalanine without a trailing underscore'
#[test]
fn canonical_ids_keep_independent_species_compartments_and_shared_phenylalanine() {
    let original = zuniga::convert_zuniga_mat_model(&fixture_raw(true), "zuniga-source").unwrap();
    let before = original.to_string();
    let canonical = zuniga::canonical_zuniga_model(&original, None).unwrap();
    assert_eq!(original.to_string(), before, "Canonicalization must not mutate source models");
    let mets = canonical["metabolites"].as_array().unwrap();
    assert!(mets.iter().any(|m| m["id"] == "Ec@phe__L_c" && m["compartment"] == "c"));
    assert!(mets.iter().any(|m| m["id"] == "Se@phe__L_c" && m["compartment"] == "c"));
    assert!(mets.iter().any(|m| m["id"] == "M@phe__L" && m["base"] == "phe__L" && m["compartment"] == "s"));
    let reaction = |id: &str| canonical["reactions"].as_array().unwrap().iter().find(|r| r["id"] == id).unwrap().clone();
    assert_eq!(reaction("Se@PHELtex")["metabolites"], json!({"Se@phe__L_c": 1.0, "M@phe__L": -1.0}));
    assert_eq!(reaction("EX_phe__L_s")["notes"]["source_reaction_id"], "EX_phe__L_[smp]");
    let mut collision = original.clone();
    let mut last = collision["metabolites"].as_array().unwrap().last().unwrap().clone();
    last["id"] = json!("phe__L[smp]");
    collision["metabolites"].as_array_mut().unwrap().push(last);
    assert!(zuniga::canonical_zuniga_model(&collision, None).unwrap_err().message.contains("collision"));
}

/// 'Scenario preparation changes exactly the GPR knockout and leaves source objective and nutrition intact'
#[test]
fn scenario_preparation_changes_exactly_the_gpr_knockout() {
    let study = fixture_study();
    let before = (study.source_models.mono.to_string(), study.source_models.joint.to_string());
    let mutant = zuniga::create_zuniga_scenario(&study, "mutant-coculture").unwrap();
    assert_eq!(mutant.knockouts, ["Ec@PPNDH"]);
    assert_eq!(mutant.model.reactions["Ec@PPNDH"].upper_bound, 0.0);
    assert_eq!(mutant.model.reactions["Ec@CHORM"].upper_bound, 1000.0);
    assert_eq!(mutant.model.objective_reactions(), [(format!("Ec@{EC_BIOMASS}"), 1.0), ("Se@BOF".to_string(), 1.0)]);
    assert_eq!(mutant.model.reactions["Se@BOF"].lower_bound, 1.0);
    assert_eq!(mutant.model.reactions["EX_phe__L_s"].upper_bound, 0.0);
    assert_eq!((study.source_models.mono.to_string(), study.source_models.joint.to_string()), before, "Preparing a knockout must not alter saved sources");
    let wt = zuniga::create_zuniga_scenario(&study, "wt-monoculture").unwrap();
    assert_eq!(wt.model.reactions["Ec@PPNDH"].upper_bound, 1000.0);
    assert!(wt.knockouts.is_empty());
    assert!(zuniga::create_zuniga_scenario(&study, "missing").is_err());
}

/// 'Synthetic stationary model reproduces WT growth, auxotroph arrest, and a donor-dependent rescue'
#[test]
fn synthetic_stationary_model_reproduces_wt_growth_auxotroph_arrest_and_rescue() {
    let study = fixture_study();
    let expected: [(&str, f64, Value); 3] = [
        ("wt-monoculture", 3.0, json!({"Ec": 3.0})),
        ("mutant-monoculture", 0.0, json!({"Ec": 0.0})),
        ("mutant-coculture", 4.0, json!({"Ec": 3.0, "Se": 1.0})),
    ];
    for (id, objective, growth) in expected {
        let prepared = zuniga::create_zuniga_scenario(&study, id).unwrap();
        let solved = zuniga::solve_zuniga_scenario(&prepared);
        assert_eq!(solved.result.status.as_str(), "optimal");
        assert!((solved.result.objective_value - objective).abs() < 1e-9, "{id}: {}", solved.result.objective_value);
        let rates = prepared.community.growth_rates(&solved.result.fluxes);
        for (k, v) in growth.as_object().unwrap() {
            assert!((rates[k] - v.as_f64().unwrap()).abs() < 1e-9, "{id} {k}");
        }
        assert_eq!(rates.len(), growth.as_object().unwrap().len());
        assert_eq!(solved.zuniga.source, if id.ends_with("coculture") { "CM-modelsKO.mat:modelCo2{2}" } else { "M-models.mat:SingleM{4}" });
        if id == "mutant-coculture" {
            let f = &solved.result.fluxes;
            assert!((f["Ec@PHEtex"] - 3.0).abs() < 1e-9);
            assert!((f["Se@PHELtex"] + 3.0).abs() < 1e-9);
            assert!(f["EX_phe__L_s"].abs() < 1e-9);
            assert!((solved.zuniga.original_fluxes["PHEtex_D"] - 3.0).abs() < 1e-9);
            assert!((solved.zuniga.original_fluxes["PHELtex_A"] + 3.0).abs() < 1e-9);
            let links = prepared.community.cross_feeding(f, 1e-6).links;
            assert_eq!(links.len(), 1);
            assert_eq!((links[0].base.as_str(), links[0].from.as_deref(), links[0].to.as_deref()), ("phe__L", Some("Se"), Some("Ec")));
            assert!((links[0].flux - 3.0).abs() < 1e-9);
        }
    }
}

/// 'Monoculture has no enabled S. elongatus growth and replay preserves the full prepared model'
#[test]
fn monoculture_has_no_enabled_se_growth_and_replay_preserves_the_prepared_model() {
    let study = fixture_study();
    let prepared = zuniga::create_zuniga_scenario(&study, "mutant-monoculture").unwrap();
    assert_eq!(prepared.community.active_species().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["Ec"]);
    let fluxes = scores(&[(&format!("Ec@{EC_BIOMASS}"), 0.0), ("Se@BOF", 100.0)]);
    assert_eq!(json!(prepared.community.growth_rates(&fluxes)), json!({"Ec": 0.0}));
    let replay = zuniga::restore_zuniga_scenario(&study, prepared.scenario.id, model(prepared.model.to_json()), None).unwrap();
    assert_eq!(replay.model.objective_reactions(), prepared.model.objective_reactions());
    assert_eq!(replay.problem.rxn_ids, prepared.problem.rxn_ids);
    for id in &replay.problem.rxn_ids {
        assert_eq!(replay.problem.get_bounds(id), prepared.problem.get_bounds(id));
    }
    assert_eq!(replay.knockouts, ["Ec@PPNDH"]);
}

/// 'Cross-feeding reports net transfer between cells rather than simultaneous transport circulation'
#[test]
fn cross_feeding_reports_net_transfer_rather_than_circulation() {
    let mut study = fixture_study();
    let reactions = study.source_models.joint["reactions"].as_array_mut().unwrap();
    reactions.push(json!({"id": "PHEreturn_D", "metabolites": {"phe__L_c_D": -1, "phe__L_[smp]": 1}, "lower_bound": 0, "upper_bound": 1000,
        "objective_coefficient": 0, "gene_reaction_rule": "", "notes": {"source_reaction_id": "PHEreturn_D"}}));
    reactions.push(json!({"id": "PHEreuptake_A", "metabolites": {"phe__L_[smp]": -1, "phe__L_c_A": 1}, "lower_bound": 0, "upper_bound": 1000,
        "objective_coefficient": 0, "gene_reaction_rule": "", "notes": {"source_reaction_id": "PHEreuptake_A"}}));
    let prepared = zuniga::create_zuniga_scenario(&study, "mutant-coculture").unwrap();
    let mut fluxes: IndexMap<String, f64> = prepared.model.reactions.keys().map(|id| (id.clone(), 0.0)).collect();
    for (id, v) in [("Se@SOURCE", 4.0), ("Se@BOF", 1.0), (&format!("Ec@{EC_BIOMASS}"), 3.0), ("Se@PHELtex", -5.0), ("Se@PHEreuptake", 2.0), ("Ec@PHEtex", 4.0), ("Ec@PHEreturn", 1.0)] {
        fluxes.insert(id.to_string(), v);
    }
    // This is a feasible optimum of the own synthetic LP, including the cycles.
    for mid in prepared.model.metabolites.keys() {
        let balance: f64 = prepared.model.reactions.values().map(|r| r.metabolites.get(mid).copied().unwrap_or(0.0) * fluxes[&r.id]).sum();
        assert_eq!(balance, 0.0, "{mid}");
    }
    let feeding = prepared.community.cross_feeding(&fluxes, 1e-6);
    assert_eq!(feeding.links, [link("phe__L", Some("Se"), Some("Ec"), 3.0)]);
    let phe = feeding.per_metabolite.iter().find(|e| e.base == "phe__L").unwrap();
    assert_eq!(phe.producers.iter().map(|p| (p.species_id.as_deref(), p.flux)).collect::<Vec<_>>(), [(Some("Se"), 3.0)]);
    assert_eq!(phe.consumers.iter().map(|p| (p.species_id.as_deref(), p.flux)).collect::<Vec<_>>(), [(Some("Ec"), 3.0)]);
}

// published-communities.test.mjs (no Store)

/// 'every community carries the metadata a citation needs'
#[test]
fn every_community_carries_the_metadata_a_citation_needs() {
    let doi = Regex::new(r"^10\.\d{4,9}/\S+$").unwrap();
    for c in PUBLISHED_COMMUNITIES.iter() {
        assert!(!c.id.is_empty() && !c.name.is_empty());
        assert!(c.description.chars().count() > 40, "{}: description is too thin to be useful", c.id);
        let p = &c.paper;
        assert!(!p.title.is_empty() && !p.authors.is_empty() && !p.journal.is_empty() && !p.framework.is_empty(), "{}", c.id);
        assert!(doi.is_match(&p.doi), "{}: {} is not a DOI", c.id, p.doi);
        // `new Date().getFullYear()`, near enough for a publication year.
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        let this_year = 1970 + (secs / 31_556_952) as u32;
        assert!(p.year >= 2000 && p.year <= this_year, "{}: year {}", c.id, p.year);
        assert!(FRAMEWORKS.all().contains(&p.framework.as_str()), "{}: {} is not a known framework", c.id, p.framework);
    }
}

/// 'ids are unique'
#[test]
fn ids_are_unique() {
    let ids: std::collections::HashSet<&str> = PUBLISHED_COMMUNITIES.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids.len(), PUBLISHED_COMMUNITIES.len());
}

/// 'a citation says whether it was verified, one way or the other'
#[test]
fn a_citation_says_whether_it_was_verified() {
    // The field is required by the type: a catalogue without it would not load.
    // Written out, it is present in every paper, as null or as a file.
    for c in PUBLISHED_COMMUNITIES.iter() {
        let paper = c.to_json()["paper"].clone();
        assert!(paper.as_object().unwrap().contains_key("doiVerifiedFrom"), "{}", c.id);
        assert!(paper["doiVerifiedFrom"].is_null() || paper["doiVerifiedFrom"].is_string());
    }
}

/// 'species entries are complete and referenced consistently'
#[test]
fn species_entries_are_complete_and_referenced_consistently() {
    for c in PUBLISHED_COMMUNITIES.iter() {
        let ids: Vec<&str> = c.species.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids.iter().collect::<std::collections::HashSet<_>>().len(), ids.len(), "{}: duplicate species id", c.id);
        for sp in &c.species {
            assert!(!sp.id.is_empty() && !sp.name.is_empty() && !sp.organism.is_empty() && !sp.role.is_empty(), "{}/{}", c.id, sp.id);
            let doc = c.to_json();
            let sp_doc = doc["species"].as_array().unwrap().iter().find(|s| s["id"] == sp.id.as_str()).unwrap().clone();
            assert!(sp_doc.as_object().unwrap().contains_key("model"), "{}/{}: say which model, or null", c.id, sp.id);
            assert!(sp_doc.as_object().unwrap().contains_key("abundance"), "{}/{}: say the abundance, or null", c.id, sp.id);
        }
        // An exchange may only name a species of this community, or the medium.
        for x in &c.exchanges {
            assert!(ids.contains(&x.from.as_str()), "{}: exchange from unknown {}", c.id, x.from);
            for to in x.recipients() {
                assert!(ids.contains(&to), "{}: exchange to unknown {to}", c.id);
            }
        }
    }
}

/// 'a model reference always says where the file came from'
#[test]
fn a_model_reference_always_says_where_the_file_came_from() {
    use coxeter_analysis::published::SpeciesModel;
    for c in PUBLISHED_COMMUNITIES.iter() {
        let inline = c.species.iter().filter_map(|s| match &s.model {
            Some(SpeciesModel::Inline(m)) => Some(m),
            _ => None,
        });
        for m in c.models.iter().chain(inline) {
            assert!(!m.reference.is_empty() && !m.source.is_empty() && !m.file.is_empty(), "{}/{}", c.id, m.id);
        }
        // A { ref } has to resolve against this community's own model list.
        for sp in &c.species {
            if let Some(SpeciesModel::Ref { reference }) = &sp.model {
                assert!(c.models.iter().any(|m| &m.id == reference), "{}/{}: refers to model {reference}, which the community does not list", c.id, sp.id);
                assert!(c.species_model(sp).is_some());
            }
        }
    }
}

/// 'status is one of the two the registry defines'
#[test]
fn status_is_one_of_the_two_the_registry_defines() {
    assert_eq!(reproduced_communities().len() + blocked_communities().len(), PUBLISHED_COMMUNITIES.len());
}

/// 'a reproduced community says what it reproduces, what it adapts, and how to run it'
#[test]
fn a_reproduced_community_says_what_it_reproduces_adapts_and_how_to_run_it() {
    for c in reproduced_communities() {
        let rep = &c.reproduction;
        assert!(!rep.reproduces.is_empty(), "{}: claims to reproduce the paper but lists nothing", c.id);
        assert!(!rep.adaptations.is_empty(), "{}: no adaptations listed", c.id);
        assert!(rep.expected.is_some(), "{}: no expected values", c.id);
        assert!(rep.builder.is_some() && rep.runner.is_some(), "{}: no builder or runner", c.id);
        assert!(rep.needs.is_empty(), "{}: reproduced, yet still lists what it needs", c.id);
        assert!(c.conditions.is_some(), "{}: reproduced, so the conditions must be on record", c.id);
    }
}

/// 'a blocked community states what is missing and claims nothing'
#[test]
fn a_blocked_community_states_what_is_missing_and_claims_nothing() {
    for c in blocked_communities() {
        let rep = &c.reproduction;
        assert_eq!(rep.status, ReproductionStatus::Blocked);
        assert!(!rep.needs.is_empty(), "{}: blocked but does not say what is missing", c.id);
        assert!(rep.reproduces.is_empty(), "{}: blocked, yet claims to reproduce something", c.id);
        assert!(rep.expected.is_none(), "{}: blocked, yet carries expected values", c.id);
        for x in &c.exchanges {
            assert!(!x.verified, "{}: exchange {} is marked verified in a blocked community", c.id, x.metabolite);
        }
    }
}

/// 'the four benchmark papers are all present'
#[test]
fn the_four_benchmark_papers_are_all_present() {
    for id in ["stolyar-2007-syntrophy", "steadycom-ecoli-auxotrophs", "steadycom-gut-nine", "zuniga-2020-phea-rescue"] {
        assert!(find_community(id).is_some(), "{id} is not in the registry");
    }
}

/// 'the registry agrees with the SteadyCom builder it points at'
#[test]
fn the_registry_agrees_with_the_steadycom_builder_it_points_at() {
    let c = find_community("steadycom-ecoli-auxotrophs").unwrap();
    assert_eq!(c.reproduction.expected.as_ref().unwrap().growth_rate, Some(steadycom_ecoli::EXPECTED_GROWTH_RATE));
    assert_eq!(c.paper.doi, steadycom_ecoli::PROVENANCE.paper.doi);
    assert_eq!(c.species.len(), STRAINS.len());
    let mentions = |text: &str, word: &str| Regex::new(&format!("(?i){}", regex::escape(word))).unwrap().is_match(text);
    for strain in STRAINS {
        let sp = c.species.iter().find(|s| s.id == strain.id).unwrap();
        for aa in strain.auxotroph_for {
            assert!(mentions(&sp.role, aa), "{}: role omits its auxotrophy for {aa}", strain.id);
        }
        assert!(mentions(&sp.role, strain.cannot_export), "{}: role omits what it cannot export", strain.id);
        assert!(mentions(&sp.role, strain.supplies), "{}: role omits what it supplies", strain.id);
    }
    // And the cross-feeding listed is the cross-feeding the strains imply.
    for strain in STRAINS {
        let supplied = c.exchanges.iter().find(|x| x.from == strain.id).unwrap();
        assert!(mentions(&supplied.metabolite, strain.supplies));
        let consumers = supplied.recipients();
        for other in STRAINS {
            if other.auxotroph_for.contains(&strain.supplies) {
                assert!(consumers.contains(&other.id), "{}: {} needs it but is not listed", supplied.metabolite, other.id);
            }
        }
    }
    let conditions = c.conditions.as_ref().unwrap();
    assert!(conditions["crossFeedingUptake"].contains(&coxeter_core::js::number_to_string(steadycom_ecoli::EX_RATE)));
    assert!(conditions["secretion"].contains(&coxeter_core::js::number_to_string(steadycom_ecoli::EXCHANGE_UPPER_BOUND)));
}

fn scenarios_json() -> Value {
    serde_json::from_str(&std::fs::read_to_string(repo("models/scenarios.json")).unwrap()).unwrap()
}

/// 'models/scenarios.json carries the registry and keeps the hand-written scenarios'
#[test]
fn scenarios_json_carries_the_registry_and_keeps_the_hand_written_scenarios() {
    let scenarios = scenarios_json();
    assert!(scenarios["scenarios"].as_array().unwrap().len() >= 5, "the teaching scenarios must survive regeneration");
    let published = scenarios["publishedCommunities"].as_array().unwrap();
    assert_eq!(published.len(), PUBLISHED_COMMUNITIES.len());
    for c in PUBLISHED_COMMUNITIES.iter() {
        let mut copy = published.iter().find(|p| p["id"] == c.id.as_str()).unwrap().clone();
        // `computed` is added by the runner, so compare everything else.
        copy.as_object_mut().unwrap().remove("computed");
        coxeter_core::testkit::assert_json_eq(&copy, &c.to_json(), 0.0, &format!("{}: scenarios.json is stale", c.id));
    }
}

/// 'the recorded result still matches what the paper reports'
#[test]
fn the_recorded_result_still_matches_what_the_paper_reports() {
    let scenarios = scenarios_json();
    for c in reproduced_communities() {
        let copy = scenarios["publishedCommunities"].as_array().unwrap().iter().find(|p| p["id"] == c.id.as_str()).unwrap();
        let computed = &copy["computed"];
        assert!(computed.is_object(), "{}: no recorded result", c.id);
        let expected = c.reproduction.expected.as_ref().unwrap();
        if let Some(rate) = expected.growth_rate {
            assert!((computed["growthRate"].as_f64().unwrap() - rate).abs() <= expected.tolerance, "{}", c.id);
        } else if let Some(rates) = &expected.growth_rates {
            for (member, rate) in rates {
                let got = computed["growthRates"][member].as_f64().unwrap_or_else(|| panic!("{}: no recorded rate for {member}", c.id));
                assert!((got - rate).abs() <= expected.tolerance, "{}/{member}: recorded {got}, expected {rate}", c.id);
            }
            if let Some(count) = expected.scenario_count {
                assert_eq!(computed["scenarios"].as_array().unwrap().len(), count as usize);
                let mutant = computed["scenarios"].as_array().unwrap().iter().find(|s| s["id"] == "mutant-monoculture").unwrap();
                assert_eq!(mutant["growth"]["Ec"].as_f64(), expected.mutant_monoculture);
                let transfer = computed["phenylalanineTransfer"].as_f64().unwrap();
                assert!((transfer - expected.phenylalanine_transfer.unwrap()).abs() <= expected.tolerance);
            } else {
                assert!(computed["sweepPoints"].as_f64().unwrap() > 0.0, "{}: the recorded sweep is empty", c.id);
            }
        } else {
            let range = expected.growth_rate_range.unwrap_or_else(|| panic!("{}: expected neither a growth rate nor a range", c.id));
            let (lo, hi) = (computed["growthRateRange"][0].as_f64().unwrap(), computed["growthRateRange"][1].as_f64().unwrap());
            assert!(lo >= range[0] && hi <= range[1], "{}: recorded {lo}..{hi}", c.id);
            assert!(computed["sweepPoints"].as_f64().unwrap() > 0.0);
        }
        // SteadyCom solves for relative abundances, which have to sum to one.
        if let Some(abundances) = computed["abundances"].as_object() {
            let total: f64 = abundances.values().map(|v| v.as_f64().unwrap()).sum();
            assert!((total - 1.0).abs() < 1e-4, "{}: recorded abundances sum to {total}", c.id);
        } else {
            assert_ne!(c.paper.framework, FRAMEWORKS.steadycom, "{}: a SteadyCom reproduction has to record its abundances", c.id);
        }
    }
}

/// 'the recorded cross-feeding has every blocked exporter at exactly zero'
#[test]
fn the_recorded_cross_feeding_has_every_blocked_exporter_at_exactly_zero() {
    let scenarios = scenarios_json();
    let recorded = &scenarios["publishedCommunities"].as_array().unwrap().iter().find(|p| p["id"] == "steadycom-ecoli-auxotrophs").unwrap()["computed"];
    for strain in STRAINS {
        let row = &recorded["exchanges"][strain.id];
        assert_eq!(row[strain.cannot_export].as_f64(), Some(0.0), "{} cannot export {}", strain.id, strain.cannot_export);
        assert!(row[strain.supplies].as_f64().unwrap() > 0.0, "{} should be supplying {}", strain.id, strain.supplies);
        for aa in strain.auxotroph_for {
            assert!(row[aa].as_f64().unwrap() < 0.0, "{} is auxotrophic for {aa} but is not taking it up", strain.id);
        }
    }
}

/// 'the four-E. coli published community keeps full models separate from map previews'
/// (the model files; the half that reads store.js's source is the Store's)
#[test]
fn the_four_e_coli_published_community_keeps_full_models_separate_from_map_previews() {
    let dir = repo("models/published/steadycom-ecoli-auxotrophs");
    let read = |f: &str| -> Value { serde_json::from_str(&std::fs::read_to_string(dir.join(f)).unwrap()).unwrap() };
    let manifest = read("manifest.json");
    for member in manifest["members"].as_array().unwrap() {
        let full = read(member["model"].as_str().unwrap());
        let preview = read(member["preview"].as_str().unwrap());
        let acald = full["reactions"].as_array().unwrap().iter().find(|r| r["id"] == "ACALD").expect("full model has ACALD");
        assert_eq!(acald["metabolites"]["accoa_c"], json!(1), "ACALD should contain cytosolic acetyl-CoA");
        assert!(full["metabolites"].as_array().unwrap().iter().any(|m| m["id"] == "accoa_c"));
        assert!(!preview["reactions"].as_array().unwrap().iter().any(|r| r["id"] == "ACALD"));
    }
}

/// 'the four-auxotroph community solves to the published growth rate'
#[test]
fn the_four_auxotroph_community_solves_to_the_published_growth_rate() {
    if std::env::var("COXETER_REPRODUCE").as_deref() != Ok("1") {
        return eprintln!("skipped: set COXETER_REPRODUCE=1 to solve it");
    }
    let dir = std::env::var("COXETER_MODEL_DIR").unwrap_or_else(|_| "/home/user/mmc/sbml".into());
    let Ok(xml) = std::fs::read_to_string(std::path::Path::new(&dir).join("iAF1260.xml")) else {
        return eprintln!("skipped: iAF1260.xml not found; set COXETER_MODEL_DIR");
    };
    let spec = steadycom_ecoli::build_community(&xml).unwrap().steady_com_spec().unwrap();
    let result = steady_com(&spec, SteadyComOptions { upper_guess: 1.0, ..Default::default() }).unwrap();
    assert_eq!(result.status.as_str(), "optimal", "{}", result.message);
    assert!((result.growth_rate - steadycom_ecoli::EXPECTED_GROWTH_RATE).abs() < 1e-4, "got {}", result.growth_rate);
    let total: f64 = result.abundances.values().sum();
    assert!((total - 1.0).abs() < 1e-4);
    for strain in STRAINS {
        let flux = |aa: &str| result.fluxes.get(&format!("{}|{}", strain.id, amino_acid_exchange(aa).unwrap())).copied().unwrap_or(0.0);
        assert!(flux(strain.cannot_export).abs() < 1e-9, "{} exported {}", strain.id, strain.cannot_export);
        assert!(flux(strain.supplies) > 0.0, "{} is not supplying {}", strain.id, strain.supplies);
        for aa in strain.auxotroph_for {
            assert!(flux(aa) < 0.0, "{} is not taking up the {aa} it cannot make", strain.id);
        }
    }
}
