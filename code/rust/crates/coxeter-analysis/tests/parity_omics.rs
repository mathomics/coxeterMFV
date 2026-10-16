//! Omics overlays, community metatranscriptomics and the cross-feeding
//! explorer against the JS app (fixtures written by
//! rust/tools/fixtures-analysis.mjs).
//!
//! Everything here is deterministic, so it must agree exactly (tolerance 0):
//! the parsers, the GPR scores, the comparisons and colour scales, the mapping
//! reports, and the explorer's index built from the JS FBA fluxes.

mod common;

use coxeter_analysis::community_view::{CommunityView, TransferRef};
use coxeter_analysis::cross_feeding_explorer::{
    CrossFeedingFilter, CrossFeedingOptions, build_cross_feeding_index, exchanged_metabolites, filter_cross_feeding_index,
};
use coxeter_analysis::overlay::{
    CompareMetric, CompareOptions, OmicsCsvOptions, OmicsDataset, OmicsSpecies, compare_reaction_scores, deserialize_omics_dataset,
    diverging_omics_colors, evaluate_gpr, parse_gpr, parse_omics_csv, reaction_omics_scores, scores_to_json, sequential_omics_colors,
    serialize_omics_dataset,
};
use coxeter_analysis::{community_omics_mapping_report, parse_metatranscriptomics, CROSS_FEEDING_GROUPS};
use coxeter_core::community::{CrossFeeding, CrossFeedingLink, ReactionOwner, ReactionRole};
use coxeter_core::testkit::assert_json_eq;
use coxeter_core::Model;
use common::{community_from_spec, cross_feeding_json, fixture, link_json, num, number_map, spec_models};
use indexmap::IndexMap;
use serde_json::{Value, json};

fn dataset_json(d: Option<&OmicsDataset>) -> Value {
    match d {
        None => Value::Null,
        Some(d) => {
            let mut v = serialize_omics_dataset(Some(d));
            v["keys"] = json!(d.values.keys().collect::<Vec<_>>());
            v
        }
    }
}

fn csv_options(o: &Value) -> OmicsCsvOptions {
    let mut options = OmicsCsvOptions::default();
    if let Some(s) = o.get("source").and_then(Value::as_str) {
        options.source = s.into();
    }
    if let Some(s) = o.get("dataType").and_then(Value::as_str) {
        options.data_type = s.into();
    }
    options
}

fn species_config(v: &Value) -> Vec<OmicsSpecies> {
    v.as_array().unwrap().iter().map(|s| OmicsSpecies::new(s["id"].as_str().unwrap(), s["file"].as_str().unwrap())).collect()
}

#[test]
fn omics_csv_tables_parse_as_the_js_parses_them() {
    let fx = fixture("overlay.json");
    for case in fx["parse"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let result = parse_omics_csv(case["text"].as_str().unwrap(), &csv_options(&case["options"]));
        match (&result, case.get("error")) {
            (Err(e), Some(msg)) => assert_eq!(e.message, msg.as_str().unwrap(), "{name}"),
            (Ok(d), None) => {
                assert_json_eq(&dataset_json(Some(d)), &case["dataset"], 0.0, name);
                assert_eq!(d.row_count as u64, case["rowCount"].as_u64().unwrap(), "{name}: rowCount");
            }
            (r, e) => panic!("{name}: Rust {r:?}, JS error {e:?}"),
        }
    }
}

#[test]
fn saved_omics_datasets_read_back_as_the_js_reads_them() {
    let fx = fixture("overlay.json");
    for (i, case) in fx["deserialize"].as_array().unwrap().iter().enumerate() {
        let back = deserialize_omics_dataset(&case["raw"]);
        assert_json_eq(&dataset_json(back.as_ref()), &case["dataset"], 0.0, &format!("deserialize case {i}"));
    }
}

#[test]
fn gene_rules_parse_and_evaluate_as_the_js_does() {
    let fx = fixture("overlay.json");
    let values = number_map(&fx["gpr"]["values"]);
    for case in fx["gpr"]["cases"].as_array().unwrap() {
        let rule = case["rule"].as_str().unwrap();
        let ast = parse_gpr(rule);
        assert_json_eq(&ast.as_ref().map_or(Value::Null, |a| a.to_json()), &case["ast"], 0.0, rule);
        let value = evaluate_gpr(ast.as_ref(), &|id| values.get(id).copied());
        assert_json_eq(&json!(value), &case["value"], 0.0, rule);
    }
}

#[test]
fn condition_comparisons_match() {
    let fx = fixture("overlay.json");
    for case in fx["compare"].as_array().unwrap() {
        let o = &case["options"];
        let options = CompareOptions {
            metric: CompareMetric::parse(o["metric"].as_str().unwrap()),
            pseudocount: o.get("pseudocount").map_or(1.0, num),
        };
        let out = compare_reaction_scores(&number_map(&case["a"]), &number_map(&case["b"]), &options);
        assert_json_eq(&scores_to_json(&out), &case["result"], 0.0, &format!("compare {o}"));
    }
}

#[test]
fn colour_scales_match() {
    let fx = fixture("overlay.json");
    for case in fx["scales"].as_array().unwrap() {
        let scores = number_map(&case["scores"]);
        let dark = case["dark"].as_bool().unwrap();
        let what = format!("scale of {} dark={dark}", case["scores"]);
        assert_json_eq(&sequential_omics_colors(&scores, dark).to_json(), &case["sequential"], 0.0, &what);
        assert_json_eq(&diverging_omics_colors(&scores, dark).to_json(), &case["diverging"], 0.0, &what);
    }
}

/// The JS unit test's hand-made community: two reactions and their owners.
struct Hand {
    owners: IndexMap<String, ReactionOwner>,
}

impl CommunityView for Hand {
    fn merged_reaction_ids(&self) -> Vec<&str> {
        self.owners.keys().map(String::as_str).collect()
    }
    fn reaction_owner(&self, id: &str) -> Option<&ReactionOwner> {
        self.owners.get(id)
    }
    fn transfers(&self, _: &str) -> Vec<TransferRef<'_>> {
        Vec::new()
    }
    fn cross_feeding(&self, _: &IndexMap<String, f64>, _: f64) -> CrossFeeding {
        CrossFeeding::default()
    }
}

fn owner(species: &str, original: &str) -> ReactionOwner {
    ReactionOwner { species_id: Some(species.into()), original_id: original.into(), role: ReactionRole::Metabolic }
}

#[test]
fn scores_of_a_hand_made_community_match() {
    let fx = fixture("overlay.json");
    let model = Model::from_value(&json!({
        "id": "m", "compartments": {"c": "cytosol"},
        "metabolites": [{"id": "a_c", "compartment": "c"}, {"id": "b_c", "compartment": "c"}],
        "reactions": [
            {"id": "R1", "metabolites": {"a_c": -1, "b_c": 1}, "gene_reaction_rule": "g1 and g2"},
            {"id": "R2", "metabolites": {"b_c": -1, "a_c": 1}, "gene_reaction_rule": "g3 or g4"},
        ],
    }))
    .unwrap();
    let hand = Hand { owners: [("sp@R1", owner("sp", "R1")), ("sp@R2", owner("sp", "R2"))].into_iter().map(|(k, v)| (k.into(), v)).collect() };
    let data = parse_omics_csv(
        "species,protein,C1\nsp,g1,10\nsp,g2,4\nsp,g3,2\nsp,g4,7\n",
        &OmicsCsvOptions { data_type: "protein".into(), ..Default::default() },
    )
    .unwrap();
    let models: IndexMap<String, Model> = [("model".to_string(), model)].into_iter().collect();
    let scores = reaction_omics_scores(&hand, &[OmicsSpecies::new("sp", "model")], &models, Some(&data), "C1");
    assert_json_eq(&scores_to_json(&scores), &fx["hand"]["scores"], 0.0, "hand-made community");
}

#[test]
fn scenario_communities_score_compare_colour_and_report_as_the_js_does() {
    let fx = fixture("overlay.json");
    let cases = fx["scenarios"].as_array().unwrap();
    assert!(!cases.is_empty());
    for case in cases {
        let name = case["scenario"].as_str().unwrap();
        let models = spec_models(&case["spec"], Some(&case["gprs"]));
        let community = community_from_spec(&case["spec"], &models);
        let species = species_config(&case["speciesConfig"]);
        let options = OmicsCsvOptions { source: format!("{name}.csv"), data_type: "gene".into() };
        let dataset = parse_omics_csv(case["csv"].as_str().unwrap(), &options).unwrap();
        assert_json_eq(&dataset_json(Some(&dataset)), &case["dataset"], 0.0, &format!("{name}: dataset"));

        for (condition, expected) in case["scores"].as_object().unwrap() {
            let scores = reaction_omics_scores(&community, &species, &models, Some(&dataset), condition);
            assert_json_eq(&scores_to_json(&scores), expected, 0.0, &format!("{name}: scores {condition}"));
        }
        let a = reaction_omics_scores(&community, &species, &models, Some(&dataset), "C1");
        let b = reaction_omics_scores(&community, &species, &models, Some(&dataset), "T2");
        let fc = compare_reaction_scores(&a, &b, &CompareOptions { metric: CompareMetric::Log2Fc, pseudocount: 1.0 });
        let diff = compare_reaction_scores(&a, &b, &CompareOptions { metric: CompareMetric::Difference, pseudocount: 1.0 });
        assert_json_eq(&scores_to_json(&fc), &case["log2fc"], 0.0, &format!("{name}: log2fc"));
        assert_json_eq(&scores_to_json(&diff), &case["difference"], 0.0, &format!("{name}: difference"));
        assert_json_eq(&sequential_omics_colors(&a, false).to_json(), &case["sequential"], 0.0, &format!("{name}: sequential"));
        assert_json_eq(&sequential_omics_colors(&a, true).to_json(), &case["sequentialDark"], 0.0, &format!("{name}: sequential dark"));
        assert_json_eq(&diverging_omics_colors(&fc, false).to_json(), &case["diverging"], 0.0, &format!("{name}: diverging"));
        assert_json_eq(&diverging_omics_colors(&diff, true).to_json(), &case["divergingDark"], 0.0, &format!("{name}: diverging dark"));
        let report = community_omics_mapping_report(&community, &species, &models, &dataset);
        assert_json_eq(&report.to_json(), &case["report"], 0.0, &format!("{name}: report"));
    }
}

#[test]
fn metatranscriptomics_tables_parse_as_the_js_parses_them() {
    let fx = fixture("community-omics.json");
    for case in fx["parse"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let source = format!("{name}.csv");
        match (parse_metatranscriptomics(case["text"].as_str().unwrap(), Some(&source)), case.get("error")) {
            (Err(e), Some(msg)) => assert_eq!(e.message, msg.as_str().unwrap(), "{name}"),
            (Ok(m), None) => {
                assert_eq!(m.format.as_str(), case["format"].as_str().unwrap(), "{name}: format");
                assert_json_eq(&dataset_json(Some(&m.dataset)), &case["dataset"], 0.0, name);
            }
            (r, e) => panic!("{name}: Rust {r:?}, JS error {e:?}"),
        }
    }
}

#[test]
fn the_mapping_report_of_the_js_unit_test_matches() {
    let fx = fixture("community-omics.json");
    let model = Model::from_value(&json!({
        "id": "m", "compartments": {"c": "c"}, "genes": [{"id": "g1"}, {"id": "g2"}],
        "metabolites": [{"id": "a_c", "compartment": "c"}, {"id": "b_c", "compartment": "c"}],
        "reactions": [{"id": "R", "metabolites": {"a_c": -1, "b_c": 1}, "gene_reaction_rule": "g1 and g2"}],
    }))
    .unwrap();
    let hand = Hand { owners: [("MAG1@R".to_string(), owner("MAG1", "R"))].into_iter().collect() };
    let models: IndexMap<String, Model> = [("m".to_string(), model)].into_iter().collect();
    let data = parse_metatranscriptomics("MAG,gene,condition,expression\nMAG1,g1,A,8\nMAG1,g2,A,3\n", None).unwrap().dataset;
    let report = community_omics_mapping_report(&hand, &[OmicsSpecies::new("MAG1", "m")], &models, &data);
    assert_json_eq(&report.to_json(), &fx["report"], 0.0, "report");
}

// Explorer

fn filter_of(v: &Value) -> CrossFeedingFilter {
    let text = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    CrossFeedingFilter {
        query: text("query"),
        metabolite: text("metabolite"),
        producer: text("producer"),
        consumer: text("consumer"),
        min_flux: v.get("minFlux").map_or(0.0, num),
    }
}

#[test]
fn the_explorer_indexes_solved_scenarios_as_the_js_does() {
    let fx = fixture("explorer.json");
    let cases = fx["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 14);
    for case in cases {
        let name = format!("{} {}", case["scenario"].as_str().unwrap(), case["objective"].as_str().unwrap());
        let models = spec_models(&case["spec"], None);
        let community = community_from_spec(&case["spec"], &models);
        let fluxes = number_map(&case["fluxes"]);
        assert_json_eq(&cross_feeding_json(&community.cross_feeding(&fluxes, 1e-6)), &case["raw"], 0.0, &format!("{name}: raw"));
        for idx in case["indexes"].as_array().unwrap() {
            let threshold = num(&idx["threshold"]);
            let index = build_cross_feeding_index(&community, &fluxes, &CrossFeedingOptions { threshold, raw: None });
            assert_json_eq(&index.to_json(), &idx["index"], 0.0, &format!("{name}: index at {threshold}"));
            for f in idx["filters"].as_array().unwrap() {
                let ids: Vec<&str> = filter_cross_feeding_index(Some(&index), &filter_of(&f["filter"])).iter().map(|e| e.id.as_str()).collect();
                assert_json_eq(&json!(ids), &f["ids"], 0.0, &format!("{name}: filter {}", f["filter"]));
            }
        }
        let links = community.cross_feeding(&fluxes, 1e-6).links;
        let skip = |b: &str| b == "h" || b == "h2o";
        let exchanged: Vec<Value> = exchanged_metabolites(&links, skip).iter().map(|m| m.to_json()).collect();
        assert_json_eq(&json!(exchanged), &case["exchanged"], 0.0, &format!("{name}: exchanged"));
        let all: Vec<Value> = exchanged_metabolites(&links, |_| false).iter().map(|m| m.to_json()).collect();
        assert_json_eq(&json!(all), &case["exchangedAll"], 0.0, &format!("{name}: exchanged, all"));
    }
}

fn links_of(v: &Value) -> Vec<CrossFeedingLink> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|l| CrossFeedingLink {
            base: l["base"].as_str().unwrap().into(),
            from: l["from"].as_str().map(str::to_string),
            to: l["to"].as_str().map(str::to_string),
            flux: num(&l["flux"]),
        })
        .collect()
}

/// The JS unit test's community: transfers for `lac`, a fixed cross-feeding.
struct Fixed {
    links: Vec<CrossFeedingLink>,
}

impl CommunityView for Fixed {
    fn merged_reaction_ids(&self) -> Vec<&str> {
        Vec::new()
    }
    fn reaction_owner(&self, _: &str) -> Option<&ReactionOwner> {
        None
    }
    fn transfers(&self, base: &str) -> Vec<TransferRef<'_>> {
        if base == "lac" {
            vec![TransferRef { species_id: "A", reaction_id: "A@EX_lac" }, TransferRef { species_id: "B", reaction_id: "B@EX_lac" }]
        } else {
            Vec::new()
        }
    }
    fn cross_feeding(&self, _: &IndexMap<String, f64>, _: f64) -> CrossFeeding {
        CrossFeeding { links: self.links.clone(), per_metabolite: Vec::new() }
    }
}

#[test]
fn the_explorer_of_the_js_unit_test_and_the_ecological_order_match() {
    let fx = fixture("explorer.json");
    let fixed = Fixed { links: links_of(&fx["test"]["links"]) };
    let index = build_cross_feeding_index(&fixed, &IndexMap::new(), &CrossFeedingOptions { threshold: 0.5, raw: None });
    assert_json_eq(&index.to_json(), &fx["test"]["index"], 0.0, "test index");
    // The same, handed the raw cross-feeding instead of asking for it.
    let supplied = CrossFeedingOptions { threshold: 0.5, raw: Some(CrossFeeding { links: fixed.links.clone(), per_metabolite: Vec::new() }) };
    let empty = Fixed { links: Vec::new() };
    assert_json_eq(&build_cross_feeding_index(&empty, &IndexMap::new(), &supplied).to_json(), &fx["test"]["index"], 0.0, "supplied raw");

    let links = links_of(&fx["exchanged"]["links"]);
    let list: Vec<Value> = exchanged_metabolites(&links, |b| b == "h").iter().map(|m| m.to_json()).collect();
    assert_json_eq(&json!(list), &fx["exchanged"]["list"], 0.0, "exchanged order");
    assert_json_eq(&json!(CROSS_FEEDING_GROUPS), &fx["groups"], 0.0, "groups");
    // A link's JSON is the JS link.
    assert_json_eq(&link_json(&links[0]), &fx["exchanged"]["links"][0], 0.0, "link");
}
