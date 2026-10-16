//! Every bundled model and scenario, read by the Rust core, against what the
//! JavaScript core made of them (`rust/tools/fixtures-core.mjs`).

use std::sync::Arc;

use coxeter_core::boundaries;
use coxeter_core::community::{CommunityModel, CommunitySpec, MediumLimit, Objective, SpeciesSpec};
use coxeter_core::label_sources::label_sources_to_value;
use coxeter_core::model::Model;
use coxeter_core::testkit::{assert_json_eq, read_fixture, repo_root};
use serde_json::{Map, Value, json};

fn load_model(name: &str) -> Model {
    let text = std::fs::read_to_string(repo_root().join(format!("models/{name}.json"))).unwrap();
    Model::from_json_str(&text).unwrap()
}

fn describe(model: &Model) -> Value {
    let tree = model.compartment_tree();
    let by_id: Map<String, Value> = tree
        .by_id
        .iter()
        .map(|(id, n)| (id.clone(), json!({"children": n.children, "parent": n.parent, "depth": n.depth})))
        .collect();
    json!({
        "json": model.to_json(),
        "compartmentIds": model.compartment_ids(),
        "compartmentNames": model.compartment_names,
        "tree": {"roots": tree.roots, "byId": by_id},
        "biomass": model.biomass_reaction_id(),
        "objective": model.objective_reactions().iter().map(|(id, c)| json!({"id": id, "coefficient": c})).collect::<Vec<_>>(),
        "reactions": model.reactions.values().map(|r| (r.id.clone(), json!({
            "kind": r.kind.as_str(), "compartments": r.compartments, "equation": r.equation(), "reversible": r.reversible(),
        }))).collect::<Map<_, _>>(),
        "metabolites": model.metabolites.values().map(|m| (m.id.clone(), json!({
            "base": m.base, "primary": m.is_primary(), "reactions": m.reactions, "labelSources": label_sources_to_value(&m.label_sources),
        }))).collect::<Map<_, _>>(),
        "massBalance": model.check_mass_balance(1e-6).iter().map(|(id, imb)| json!({"id": id, "imbalance": imb})).collect::<Vec<_>>(),
        "boundaryIndex": boundaries::model_boundary_index(model),
    })
}

#[test]
fn bundled_models_match_javascript() {
    let index = read_fixture("core/index.json").expect("run: node rust/tools/fixtures-core.mjs");
    let names = index["models"].as_array().unwrap();
    assert!(names.len() >= 5);
    for name in names {
        let name = name.as_str().unwrap();
        let expected = read_fixture(&format!("core/model-{name}.json")).unwrap();
        assert_json_eq(&describe(&load_model(name)), &expected, 0.0, name);
    }
}

#[test]
fn bundled_scenarios_merge_like_javascript() {
    let index = read_fixture("core/index.json").unwrap();
    for id in index["scenarios"].as_array().unwrap() {
        let id = id.as_str().unwrap();
        let fx = read_fixture(&format!("core/community-{id}.json")).unwrap();
        let spec = &fx["spec"];
        let medium = spec["medium"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), MediumLimit { lower: v["lower"].as_f64(), upper: v["upper"].as_f64() }))
            .collect();
        let species = spec["species"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| SpeciesSpec {
                id: Some(s["id"].as_str().unwrap().into()),
                model: Arc::new(load_model(s["model"].as_str().unwrap())),
                abundance: s["abundance"].as_f64(),
                enabled: s["enabled"].as_bool(),
                color: None,
            })
            .collect();
        let c = CommunityModel::new(CommunitySpec {
            name: spec["name"].as_str().map(String::from),
            species,
            medium,
            objective: Objective::parse(spec["objective"].as_str().unwrap()),
            product_target: spec["productTarget"].as_str().map(String::from),
        })
        .unwrap();
        assert_json_eq(&describe(&c.merged), &fx["merged"], 0.0, &format!("{id} merged model"));
        let owners: Map<String, Value> = c
            .reaction_owner
            .iter()
            .map(|(k, o)| (k.clone(), json!({"speciesId": o.species_id, "originalId": o.original_id, "role": o.role.as_str()})))
            .collect();
        assert_json_eq(&Value::Object(owners), &fx["reactionOwner"], 0.0, &format!("{id} reaction owners"));
        let met_owners: Map<String, Value> = c
            .metabolite_owner
            .iter()
            .map(|(k, o)| (k.clone(), json!({"speciesId": o.species_id, "originalId": o.original_id, "compartment": o.compartment})))
            .collect();
        assert_json_eq(&Value::Object(met_owners), &fx["metaboliteOwner"], 0.0, &format!("{id} metabolite owners"));
        let transfers: Map<String, Value> = c
            .transfers_by_base
            .iter()
            .map(|(b, ts)| {
                (b.clone(), ts.iter().map(|t| json!({"speciesId": t.species_id, "reactionId": t.reaction_id, "localId": t.local_id})).collect())
            })
            .collect();
        assert_json_eq(&Value::Object(transfers), &fx["transfersByBase"], 0.0, &format!("{id} transfers"));
        assert_json_eq(&json!(c.species_compartments), &fx["speciesCompartments"], 0.0, id);
        assert_json_eq(&json!(c.biomass_ids), &fx["biomassIds"], 0.0, id);
        assert_json_eq(&json!(c.abundances), &fx["abundances"], 0.0, id);
        assert_json_eq(&json!(boundaries::reactions_by_boundary(&c)), &fx["reactionsByBoundary"], 0.0, &format!("{id} reactions by boundary"));
        let infos: Vec<Value> = boundaries::community_boundaries(&c)
            .iter()
            .map(|b| json!({"id": b.id, "kind": b.kind.as_str(), "cellId": b.cell_id, "inside": b.inside, "outside": b.outside, "parentId": b.parent_id, "depth": b.depth}))
            .collect();
        assert_json_eq(&json!(infos), &fx["boundaries"], 0.0, &format!("{id} boundaries"));
    }
}
