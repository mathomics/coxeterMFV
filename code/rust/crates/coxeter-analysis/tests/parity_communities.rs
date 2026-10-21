//! The published communities against the JS app (fixtures written by
//! rust/tools/fixtures-analysis.mjs): the registry, the Stolyar reader, the
//! four-auxotroph builder, the gut-nine workbook import and the Zuñiga case.
//!
//! Readers, converters and builders are deterministic and must agree exactly.
//! LP results are compared where they are unique: objective values and growth
//! rates (1e-6, or 1e-5 for SteadyCom, whose bisection stops at a 1e-6
//! resolution), and the fluxes of the toy models whose optimum is one point.

mod common;

use std::sync::Arc;

use coxeter_analysis::community_view::CommunityView;
use coxeter_analysis::cross_feeding_explorer::{CrossFeedingOptions, build_cross_feeding_index};
use coxeter_analysis::gut9::{self, Gut9ImportOptions, Gut9SpecOptions};
use coxeter_analysis::published::{
    FRAMEWORKS, blocked_communities, find_community, published_communities_json, reproduced_communities,
};
use coxeter_analysis::steadycom_ecoli::{self, AMINO_ACID_EXCHANGES, STRAINS};
use coxeter_analysis::stolyar::{self, JointOptions, StolyarFiles};
use coxeter_analysis::zuniga::{
    self, FetchResponse, ZunigaLoadOptions, ZunigaModels, ZunigaPrepared, ZunigaSourceModels, ZunigaStudy,
};
use coxeter_analysis::Result;
use coxeter_core::Model;
use coxeter_core::community::{MetaboliteOwner, ReactionOwner};
use coxeter_core::testkit::assert_json_eq;
use coxeter_io::mat5::{MatValue, TypedArray, TypedArrayKind, read_mat5};
use coxeter_solver::fba::{FluxProblem, FvaOptions};
use coxeter_solver::steadycom::{AtOptions, SteadyComOptions, max_total_biomass_at, steady_com};
use common::{close, cross_feeding_json, fixture, fixture_bytes, num, number_map};
use indexmap::IndexMap;
use serde_json::{Value, json};

// Registry

#[test]
fn the_registry_is_the_js_registry() {
    let fx = fixture("published.json");
    assert_json_eq(&published_communities_json(), &fx["communities"], 0.0, "PUBLISHED_COMMUNITIES");
    let f = &fx["frameworks"];
    assert_eq!(
        [FRAMEWORKS.joint_fba, FRAMEWORKS.optcom, FRAMEWORKS.steadycom],
        [&f["JOINT_FBA"], &f["OPTCOM"], &f["STEADYCOM"]].map(|v| v.as_str().unwrap())
    );
    let ids = |v: Vec<&coxeter_analysis::PublishedCommunity>| json!(v.iter().map(|c| c.id.as_str()).collect::<Vec<_>>());
    assert_eq!(ids(reproduced_communities()), fx["reproduced"]);
    assert_eq!(ids(blocked_communities()), fx["blocked"]);
    for case in fx["find"].as_array().unwrap() {
        let found = find_community(case["id"].as_str().unwrap()).map(|c| c.id.clone());
        assert_eq!(json!(found), case["found"]);
    }
}

// Stolyar

fn files_of(v: &Value) -> StolyarFiles<'_> {
    StolyarFiles {
        reactions: v["reactions"].as_str().unwrap(),
        metabolites: v["metabolites"].as_str().unwrap(),
        macromolecules: v["macromolecules"].as_str().unwrap(),
        synthesis: v["synthesis"].as_str().unwrap(),
    }
}

/// Everything the fixture records about one import.
fn check_stolyar_outputs(name: &str, files: &StolyarFiles<'_>, js: &Value) {
    let model = match (stolyar::import_stolyar(files), js.get("error")) {
        (Err(e), Some(msg)) => {
            let msg = msg.as_str().unwrap();
            // Where the JS stops on a TypeError (a line that ends before its
            // bounds or position column) the Rust says which line instead.
            if msg.starts_with("Cannot read properties") {
                assert!(e.message.starts_with("reactions: Short has"), "{name}: {}", e.message);
            } else {
                assert_eq!(e.message, msg, "{name}");
            }
            return;
        }
        (Ok(m), None) => m,
        (r, e) => panic!("{name}: Rust {:?}, JS error {e:?}", r.map(|_| ())),
    };
    let what = |k: &str| format!("{name}: {k}");
    assert_json_eq(&model.to_json(), &js["model"], 0.0, &what("model"));
    assert_json_eq(&stolyar::validate_stolyar(&model).to_json(), &js["validation"], 0.0, &what("validation"));
    assert_json_eq(&json!(stolyar::external_metabolites(&model)), &js["external"], 0.0, &what("external"));
    let by = stolyar::reactions_by_species(&model);
    assert_json_eq(&json!(by.by_species), &js["bySpecies"], 0.0, &what("bySpecies"));
    assert_json_eq(&json!(by.community), &js["community"], 0.0, &what("community"));
    let exchange: Vec<Value> = stolyar::exchange_structure(&model).iter().map(|x| x.to_json()).collect();
    assert_json_eq(&json!(exchange), &js["exchange"], 0.0, &what("exchange"));
    assert_json_eq(&json!(stolyar::objective_of(&model)), &js["objective"], 0.0, &what("objective"));
    assert_json_eq(&stolyar::to_joint_cobra_json(&model, &JointOptions::default()), &js["joint"], 0.0, &what("joint"));
    let named = JointOptions { id: "j".into(), name: "Joint".into() };
    assert_json_eq(&stolyar::to_joint_cobra_json(&model, &named), &js["jointNamed"], 0.0, &what("jointNamed"));
    for sp in stolyar::SPECIES {
        let doc = stolyar::to_species_cobra_json(&model, sp.id, None).unwrap();
        assert_json_eq(&doc, &js["species"][sp.id], 0.0, &what(sp.id));
    }
    let cell = json!({"name": "M", "color": "#000"});
    assert_json_eq(&stolyar::to_species_cobra_json(&model, "Mm", Some(&cell)).unwrap(), &js["speciesWithCell"], 0.0, &what("cell"));
    assert_eq!(stolyar::to_species_cobra_json(&model, "Xx", None).unwrap_err().message, js["unknownSpecies"].as_str().unwrap());
}

#[test]
fn the_cellnetanalyzer_reader_matches_on_every_variant() {
    let fx = fixture("stolyar.json");
    let species: Vec<Value> = stolyar::SPECIES.iter().map(|s| s.to_json()).collect();
    assert_json_eq(&json!(species), &fx["species"], 0.0, "SPECIES");
    let e = stolyar::EXPECTED;
    assert_json_eq(
        &json!({
            "species": e.species, "reactions": e.reactions, "metabolites": e.metabolites, "macromolecules": e.macromolecules,
            "externalMetabolites": e.external_metabolites, "sharedMetabolites": e.shared_metabolites,
            "measuredReactions": e.measured_reactions, "objectiveReactions": e.objective_reactions,
        }),
        &fx["expected"],
        0.0,
        "EXPECTED",
    );
    for case in fx["terms"].as_array().unwrap() {
        let terms: Vec<Value> =
            stolyar::parse_terms(case["side"].as_str().unwrap()).iter().map(|t| json!({"coefficient": t.coefficient, "metabolite": t.metabolite})).collect();
        assert_json_eq(&json!(terms), &case["terms"], 0.0, case["side"].as_str().unwrap());
    }
    for case in fx["equations"].as_array().unwrap() {
        let st = stolyar::parse_equation(case["equation"].as_str().unwrap());
        assert_json_eq(&json!(st), &case["stoichiometry"], 0.0, case["equation"].as_str().unwrap());
    }
    for case in fx["synthesis"].as_array().unwrap() {
        let text = case["text"].as_str().unwrap();
        match (stolyar::parse_macromolecule_synthesis(text), case.get("error")) {
            (Err(e), Some(msg)) => assert_eq!(e.message, msg.as_str().unwrap(), "{text}"),
            (Ok(m), None) => assert_json_eq(&json!(m), &case["ok"], 0.0, text),
            (r, e) => panic!("{text}: Rust {r:?}, JS error {e:?}"),
        }
    }
    let variants = fx["variants"].as_object().unwrap();
    assert!(variants.len() >= 20);
    for (name, case) in variants {
        check_stolyar_outputs(name, &files_of(&case["files"]), case);
    }
}

#[test]
fn the_joint_models_solve_as_the_js_solves_them() {
    let fx = fixture("stolyar.json");
    let fba = &fx["fba"];
    let tiny = stolyar::import_stolyar(&files_of(&fx["variants"]["tiny"]["files"])).unwrap();
    let joint = Model::from_value(&stolyar::to_joint_cobra_json(&tiny, &JointOptions::default())).unwrap();
    let solved = FluxProblem::new(&joint).optimize();
    assert_eq!(solved.status.as_str(), fba["tiny"]["status"].as_str().unwrap());
    assert!(close(solved.objective_value, num(&fba["tiny"]["objectiveValue"]), 1e-9));
    let mut cut = FluxProblem::new(&joint);
    cut.set_bounds("Mtake", Some(0.0), Some(0.0)).unwrap();
    let after = cut.optimize();
    assert!(close(after.objective_value, num(&fba["cut"]["objectiveValue"]), 1e-9));
    assert_eq!(after.fluxes["MC_BIO"], 0.0);

    // The bundled joint model the app ships (the supplement, translated).
    let doc: Value = serde_json::from_str(
        &std::fs::read_to_string(common::repo("models/published/stolyar-2007-syntrophy/joint-model.json")).unwrap(),
    )
    .unwrap();
    let model = Model::from_value(&doc).unwrap();
    let js = &fba["bundled"];
    for (label, lower) in [("syntrophic", 0.0), ("respiring", -1000.0)] {
        let mut p = FluxProblem::new(&model);
        p.set_bounds("EX_SO4", Some(lower), Some(1000.0)).unwrap();
        let r = p.optimize();
        assert_eq!(r.status.as_str(), js[label]["status"].as_str().unwrap());
        assert!(close(r.objective_value, num(&js[label]["objectiveValue"]), 1e-6), "{label}: {}", r.objective_value);
        if label == "syntrophic" {
            for id in ["mue", "MC_BIO", "M12", "D01"] {
                assert!(close(r.fluxes[id], num(&js[label]["fluxes"][id]), 1e-6), "{label} {id}: {}", r.fluxes[id]);
            }
            let ids = ["mue", "MC_BIO", "M12", "D81", "D80"];
            let ranges = p.variability(&ids, &FvaOptions { fraction: 1.0, threads: Some(1), ..Default::default() });
            for id in ids {
                let (min, max) = (num(&js["ranges"][id][0]), num(&js["ranges"][id][1]));
                assert!(close(ranges[id].min, min, 1e-6) && close(ranges[id].max, max, 1e-6), "{id}: {:?} vs [{min}, {max}]", ranges[id]);
            }
        }
    }
}

// Four-auxotroph E. coli

#[test]
fn the_four_auxotroph_builder_matches() {
    let fx = fixture("steadycom-ecoli.json");
    assert_json_eq(&steadycom_ecoli::PROVENANCE.to_json(), &fx["provenance"], 0.0, "PROVENANCE");
    assert_json_eq(
        &json!({"id": steadycom_ecoli::ADDED_REACTION.id, "tutorial": steadycom_ecoli::ADDED_REACTION.tutorial}),
        &fx["addedReaction"],
        0.0,
        "ADDED_REACTION",
    );
    assert_json_eq(&json!(STRAINS.iter().map(|s| s.to_json()).collect::<Vec<_>>()), &fx["strains"], 0.0, "STRAINS");
    let c = &fx["constants"];
    assert_eq!(steadycom_ecoli::EX_RATE, num(&c["EX_RATE"]));
    assert_eq!(steadycom_ecoli::EXCHANGE_UPPER_BOUND, num(&c["EXCHANGE_UPPER_BOUND"]));
    assert_eq!(steadycom_ecoli::EXPECTED_GROWTH_RATE, num(&c["EXPECTED_GROWTH_RATE"]));
    assert_eq!(steadycom_ecoli::SENTINEL_BOUND, num(&c["SENTINEL_BOUND"]));
    let table: IndexMap<&str, &str> = AMINO_ACID_EXCHANGES.into_iter().collect();
    assert_json_eq(&json!(table), &fx["aminoAcidExchanges"], 0.0, "AMINO_ACID_EXCHANGES");
    for pair in fx["exchangeBase"].as_array().unwrap() {
        assert_eq!(steadycom_ecoli::exchange_base(pair[0].as_str().unwrap()), pair[1].as_str().unwrap());
    }

    let xml = String::from_utf8(fixture_bytes("steadycom-ecoli-mini.xml")).unwrap();
    let built = steadycom_ecoli::build_community(&xml).unwrap();
    let js = &fx["community"];
    let members: Vec<Value> = built
        .members
        .iter()
        .map(|m| {
            json!({
                "id": m.id, "strain": m.strain.to_json(), "model": m.model.to_json(),
                "biomassReaction": m.biomass_reaction, "exchangeFor": m.exchange_for,
            })
        })
        .collect();
    assert_json_eq(&json!(members), &js["members"], 0.0, "members");
    assert_json_eq(&json!(built.community_uptake), &js["communityUptake"], 0.0, "communityUptake");
    assert_eq!(built.total_biomass, num(&js["totalBiomass"]));
    assert_json_eq(&built.document, &js["document"], 0.0, "document");

    let result = steady_com(&built.steady_com_spec().unwrap(), SteadyComOptions { upper_guess: 5.0, ..Default::default() }).unwrap();
    assert_eq!(result.status.as_str(), fx["steadyCom"]["status"].as_str().unwrap());
    let mu = num(&fx["steadyCom"]["growthRate"]);
    assert!(close(result.growth_rate, mu, 1e-5), "growth rate {} vs JS {mu}", result.growth_rate);
    let total: f64 = result.abundances.values().sum();
    assert!((total - 1.0).abs() < 1e-6, "abundances sum to {total}");

    for (file, key) in [("steadycom-ecoli-broken.xml", "missingReaction"), ("steadycom-ecoli-noexchange.xml", "missingExchange")] {
        let xml = String::from_utf8(fixture_bytes(file)).unwrap();
        let err = steadycom_ecoli::build_community(&xml).unwrap_err();
        assert_eq!(err.message, fx["errors"][key].as_str().unwrap());
    }
}

// Gut nine

#[test]
fn the_gut_nine_workbook_import_matches() {
    let fx = fixture("gut9.json");
    let p = gut9::PROVENANCE;
    assert_json_eq(
        &json!({
            "paper": {"title": p.paper.title, "authors": p.paper.authors, "year": p.paper.year, "journal": p.paper.journal, "doi": p.paper.doi, "framework": p.paper.framework},
            "files": {"model": p.files.model, "readme": p.files.readme, "bounds": p.files.bounds, "stoichiometry": p.files.stoichiometry, "experiment": p.files.experiment},
        }),
        &fx["provenance"],
        0.0,
        "PROVENANCE",
    );
    let e = gut9::EXPECTED;
    assert_json_eq(
        &json!({
            "species": e.species, "reactions": e.reactions, "metabolites": e.metabolites, "stoichiometryEntries": e.stoichiometry_entries,
            "communityMetabolites": e.community_metabolites, "communityExchangeReactions": e.community_exchange_reactions,
            "speciesExchangeReactions": e.species_exchange_reactions, "dietReactions": e.diet_reactions,
        }),
        &fx["expected"],
        0.0,
        "EXPECTED",
    );

    let bytes = fixture_bytes("gut9-mini.xlsx");
    let bounds_text = fx["boundsText"].as_str().unwrap();
    let stoich_text = fx["stoichText"].as_str().unwrap();
    let plain = gut9::import_gut9(&bytes, &Gut9ImportOptions::default()).unwrap();
    let with_bounds = gut9::import_gut9(&bytes, &Gut9ImportOptions { bounds_text: Some(bounds_text) }).unwrap();
    assert_json_eq(&plain.to_json(), &fx["plain"], 0.0, "import");
    assert_json_eq(&with_bounds.to_json(), &fx["withBounds"], 0.0, "import with bounds");

    let mut fresh = gut9::import_gut9(&bytes, &Gut9ImportOptions::default()).unwrap();
    let applied = gut9::apply_bounds(&mut fresh, bounds_text).unwrap();
    let applied_json: Vec<Value> = applied
        .iter()
        .map(|c| json!({"id": c.id, "species": c.species, "from": c.from, "to": c.to}))
        .collect();
    assert_json_eq(&json!(applied_json), &fx["applied"], 0.0, "applyBounds");

    assert_json_eq(&gut9::validate_bounds(&plain, bounds_text).unwrap().to_json(), &fx["validateBounds"]["plain"], 0.0, "validateBounds plain");
    assert_json_eq(
        &gut9::validate_bounds(&with_bounds, bounds_text).unwrap().to_json(),
        &fx["validateBounds"]["withBounds"],
        0.0,
        "validateBounds with bounds",
    );
    assert_json_eq(
        &gut9::validate_stoichiometry(&plain, stoich_text).unwrap().to_json(),
        &fx["validateStoichiometry"]["good"],
        0.0,
        "validateStoichiometry",
    );
    assert_json_eq(
        &gut9::validate_stoichiometry(&plain, fx["badStoich"].as_str().unwrap()).unwrap().to_json(),
        &fx["validateStoichiometry"]["bad"],
        0.0,
        "validateStoichiometry, bad",
    );
    assert_json_eq(&json!(gut9::reactions_by_species(&with_bounds).unwrap()), &fx["reactionsBySpecies"], 0.0, "reactionsBySpecies");
    assert_json_eq(&json!(gut9::community_reaction_ids(&with_bounds)), &fx["communityReactionIds"], 0.0, "communityReactionIds");
    assert_json_eq(&json!(gut9::diet(&plain)), &fx["diet"]["plain"], 0.0, "diet");
    assert_json_eq(&json!(gut9::diet(&with_bounds)), &fx["diet"]["withBounds"], 0.0, "diet with bounds");

    for case in fx["parseBounds"].as_array().unwrap() {
        let text = case["text"].as_str().unwrap();
        match (gut9::parse_bounds(text), case.get("error")) {
            (Err(e), Some(msg)) => assert_eq!(e.message, msg.as_str().unwrap()),
            (Ok(b), None) => {
                let doc: serde_json::Map<String, Value> = b.iter().map(|(k, v)| (k.clone(), v.to_json())).collect();
                assert_json_eq(&Value::Object(doc), &case["ok"], 0.0, text);
            }
            (r, e) => panic!("{text}: Rust {r:?}, JS error {e:?}"),
        }
    }
    assert_eq!(gut9::validate_stoichiometry(&plain, "rxns\tx\n").unwrap_err().message, fx["headerErrors"]["stoich"].as_str().unwrap());
    assert_eq!(gut9::validate_bounds(&plain, "mets\n").unwrap_err().message, fx["headerErrors"]["bounds"].as_str().unwrap());
    let mut one = plain.clone();
    one.species.truncate(1);
    assert_eq!(gut9::reactions_by_species(&one).unwrap_err().message, fx["unknownSpecies"].as_str().unwrap());

    let spec = gut9::to_steady_com_spec(&with_bounds, &Gut9SpecOptions::default()).unwrap();
    let result = steady_com(&spec, SteadyComOptions { upper_guess: 1.0, ..Default::default() }).unwrap();
    let js = &fx["steadyCom"];
    assert_eq!(result.status.as_str(), js["status"].as_str().unwrap());
    assert!(close(result.growth_rate, num(&js["growthRate"]), 1e-5), "growth rate {} vs JS {}", result.growth_rate, js["growthRate"]);
    let at = max_total_biomass_at(&spec, num(&js["at"]["mu"]), &AtOptions::default()).unwrap();
    assert_eq!(at.feasible, js["at"]["feasible"].as_bool().unwrap());
    assert_eq!(at.status.as_str(), js["at"]["status"].as_str().unwrap());
    assert!(close(at.total, num(&js["at"]["total"]), 1e-9));

    for (name, msg) in fx["errors"].as_object().unwrap() {
        let err = gut9::import_gut9(&fixture_bytes(&format!("gut9-{name}.xlsx")), &Gut9ImportOptions::default()).unwrap_err();
        assert_eq!(err.message, msg.as_str().unwrap(), "{name}");
    }
}

// Zuñiga

fn cell_items(vars: &coxeter_io::mat5::MatVariables, name: &str) -> Vec<MatValue> {
    match &vars[name] {
        MatValue::Cell(items) => items.clone(),
        other => panic!("{name} is not a cell: {other:?}"),
    }
}

fn toy_raw() -> (Vec<MatValue>, Vec<MatValue>) {
    let mono = read_mat5(&fixture_bytes("zuniga-M-models.mat")).unwrap();
    let co = read_mat5(&fixture_bytes("zuniga-CM-modelsKO.mat")).unwrap();
    (cell_items(&mono, "SingleM"), cell_items(&co, "modelCo2"))
}

fn toy_study(fx: &Value) -> ZunigaStudy {
    let (mono, co) = toy_raw();
    let mono_doc = zuniga::convert_zuniga_mat_model(&mono[3], "toy-mono").unwrap();
    let se_doc = zuniga::convert_zuniga_mat_model(&mono[1], "toy-se").unwrap();
    let joint_doc = zuniga::convert_zuniga_mat_model(&co[1], "toy-co").unwrap();
    assert_json_eq(&mono_doc, &fx["converted"]["mono"], 0.0, "converted mono");
    assert_json_eq(&se_doc, &fx["converted"]["se"], 0.0, "converted se");
    assert_json_eq(&joint_doc, &fx["converted"]["joint"], 0.0, "converted joint");
    ZunigaStudy {
        manifest: Value::Null,
        source_hashes: [("ownFixture".to_string(), "not-an-upstream-model".to_string())].into_iter().collect(),
        models: ZunigaModels { ec: Arc::new(Model::from_value(&mono_doc).unwrap()), se: Arc::new(Model::from_value(&se_doc).unwrap()) },
        source_models: ZunigaSourceModels { mono: mono_doc, joint: joint_doc },
    }
}

fn owner_json(o: &ReactionOwner) -> Value {
    json!({"speciesId": o.species_id, "originalId": o.original_id, "role": o.role.as_str()})
}

fn met_owner_json(o: &MetaboliteOwner) -> Value {
    json!({"speciesId": o.species_id, "originalId": o.original_id, "compartment": o.compartment})
}

fn community_json(p: &ZunigaPrepared) -> Value {
    let c = &p.community;
    let transfers: serde_json::Map<String, Value> = c
        .transfers_by_base
        .iter()
        .map(|(base, list)| {
            let list: Vec<Value> = list
                .iter()
                .map(|t| {
                    let mut v = json!({"speciesId": t.species_id, "reactionId": t.reaction_id, "coefficient": t.coefficient});
                    if let Some(local) = &t.local_id {
                        v["localId"] = json!(local);
                    }
                    v
                })
                .collect();
            (base.clone(), json!(list))
        })
        .collect();
    json!({
        "name": c.name, "objectiveType": c.objective_type,
        "reactionOwner": c.reaction_owner.iter().map(|(k, o)| (k.clone(), owner_json(o))).collect::<serde_json::Map<_, _>>(),
        "metaboliteOwner": c.metabolite_owner.iter().map(|(k, o)| (k.clone(), met_owner_json(o))).collect::<serde_json::Map<_, _>>(),
        "transfersByBase": transfers,
        "biomassIds": c.biomass_ids, "abundances": c.abundances, "speciesCompartments": c.species_compartments,
        "activeSpecies": c.active_species().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        "normalisedAbundances": c.normalised_abundances(),
        "json": c.to_json(),
    })
}

fn bounds_of(p: &FluxProblem) -> (Vec<f64>, Vec<f64>) {
    p.rxn_ids.iter().map(|id| p.get_bounds(id).unwrap()).unzip()
}

#[test]
fn gene_rules_canonical_ids_and_conversion_errors_match() {
    let fx = fixture("zuniga.json");
    assert_json_eq(&json!(zuniga::ZUNIGA_SCENARIOS.iter().map(|s| s.to_json()).collect::<Vec<_>>()), &fx["scenarios"], 0.0, "scenarios");
    assert_eq!(zuniga::ZUNIGA_GENE, fx["gene"].as_str().unwrap());
    assert_eq!(zuniga::ZUNIGA_ID, fx["id"].as_str().unwrap());
    for case in fx["geneRules"].as_array().unwrap() {
        let rule = case["rule"].as_str().unwrap();
        match (zuniga::evaluate_zuniga_gene_rule(rule, &["b2599"]), case.get("error")) {
            (Err(e), Some(msg)) => assert_eq!(e.message, msg.as_str().unwrap(), "{rule:?}"),
            (Ok(v), None) => assert_eq!(json!(v), case["ok"], "{rule:?}"),
            (r, e) => panic!("{rule:?}: Rust {r:?}, JS error {e:?}"),
        }
        assert_eq!(json!(zuniga::evaluate_zuniga_gene_rule(rule, &[]).ok()), case["none"], "{rule:?} with nothing deleted");
    }

    let study = toy_study(&fx);
    let joint = &study.source_models.joint;
    let mono = &study.source_models.mono;
    assert_json_eq(&zuniga::canonical_zuniga_model(joint, None).unwrap(), &fx["canonical"]["joint"], 0.0, "canonical joint");
    assert_json_eq(&zuniga::canonical_zuniga_model(mono, Some("Ec")).unwrap(), &fx["canonical"]["mono"], 0.0, "canonical mono");
    assert_json_eq(&zuniga::canonical_zuniga_model(mono, None).unwrap(), &fx["canonical"]["monoNoSpecies"], 0.0, "canonical mono, no species");

    let mut collision = joint.clone();
    let mut last = collision["metabolites"].as_array().unwrap().last().unwrap().clone();
    last["id"] = json!("phe__L[smp]");
    collision["metabolites"].as_array_mut().unwrap().push(last);
    assert_eq!(zuniga::canonical_zuniga_model(&collision, None).unwrap_err().message, fx["collision"].as_str().unwrap());
    let mut collision = mono.clone();
    let first = collision["reactions"][0].clone();
    collision["reactions"].as_array_mut().unwrap().push(first);
    assert_eq!(zuniga::canonical_zuniga_model(&collision, None).unwrap_err().message, fx["reactionCollision"].as_str().unwrap());

    // The JS mutates the reader's objects; the same mutations on the MatValue.
    for case in fx["convert"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let (mut mono, _) = toy_raw();
        let MatValue::Struct(fields) = &mut mono[3] else { panic!("SingleM{{4}} is a structure") };
        let set_number = |v: &mut MatValue, i: usize, x: f64| match v {
            MatValue::Numeric { data, .. } => data.values[i] = x,
            _ => panic!("not numeric"),
        };
        let set_cell = |v: &mut MatValue, i: usize, s: &str| match v {
            MatValue::Cell(items) => items[i] = MatValue::Text(s.into()),
            _ => panic!("not a cell"),
        };
        match name {
            "rhs" => set_number(fields.get_mut("b").unwrap(), 0, 1.0),
            "sense" => *fields.get_mut("csense").unwrap() = MatValue::Text("LEE".into()),
            "sense-column" => *fields.get_mut("csense").unwrap() = MatValue::TextRows(vec!["E".into(), "E".into(), "E".into()]),
            "sense-spaces" => *fields.get_mut("csense").unwrap() = MatValue::Text(" E E\tE ".into()),
            "sense-empty" => *fields.get_mut("csense").unwrap() = MatValue::Text(String::new()),
            "bound" => set_number(fields.get_mut("lb").unwrap(), 0, 4.0),
            "nan-objective" => set_number(fields.get_mut("c").unwrap(), 1, f64::NAN),
            "vector" => {
                *fields.get_mut("ub").unwrap() =
                    MatValue::Numeric { shape: vec![1, 1], data: TypedArray { kind: TypedArrayKind::Float64, values: vec![3.0] } }
            }
            "rules-absent" => set_cell(fields.get_mut("rules").unwrap(), 4, ""),
            "rules-unknown-gene" => set_cell(fields.get_mut("rules").unwrap(), 4, "x(9999)"),
            "rules-zero-gene" => set_cell(fields.get_mut("rules").unwrap(), 4, "x(0)"),
            "rules-operators" => set_cell(fields.get_mut("rules").unwrap(), 4, "(x(1) | x(2)) & x(3)"),
            "rules-invalid" => set_cell(fields.get_mut("rules").unwrap(), 4, "x(1) |"),
            "shape" => match fields.get_mut("S").unwrap() {
                MatValue::Sparse(s) => s.shape = vec![2, 5],
                _ => panic!("S is sparse"),
            },
            "nan-stoichiometry" => match fields.get_mut("S").unwrap() {
                MatValue::Sparse(s) => s.values.values[0] = f64::NAN,
                _ => panic!("S is sparse"),
            },
            "no-description" => {
                fields.shift_remove("description");
            }
            "charge-field" => {
                let charges = fields.shift_remove("metCharges").unwrap();
                fields.insert("metCharge".into(), charges);
            }
            other => panic!("no mutation for {other}"),
        }
        match (zuniga::convert_zuniga_mat_model(&mono[3], name), case.get("error")) {
            (Err(e), Some(msg)) => assert_eq!(e.message, msg.as_str().unwrap(), "{name}"),
            (Ok(doc), None) => assert_json_eq(&doc, &case["ok"], 0.0, name),
            (r, e) => panic!("{name}: Rust {:?}, JS error {e:?}", r.map(|_| ())),
        }
    }
}

#[test]
fn the_three_scenarios_build_solve_and_replay_as_the_js_does() {
    let fx = fixture("zuniga.json");
    let study = toy_study(&fx);
    for js in fx["scenarioResults"].as_array().unwrap() {
        let id = js["id"].as_str().unwrap();
        let prepared = zuniga::create_zuniga_scenario(&study, id).unwrap();
        assert_json_eq(&prepared.model.to_json(), &js["model"], 0.0, &format!("{id}: model"));
        assert_json_eq(&json!(prepared.knockouts), &js["knockouts"], 0.0, &format!("{id}: knockouts"));
        assert_eq!(prepared.source, js["source"].as_str().unwrap());
        assert_json_eq(&json!(prepared.source_hashes), &js["sourceHashes"], 0.0, &format!("{id}: hashes"));
        assert_json_eq(&community_json(&prepared), &js["community"], 0.0, &format!("{id}: community"));
        let (lb, ub) = bounds_of(&prepared.problem);
        assert_json_eq(&json!(lb), &js["lb"], 0.0, &format!("{id}: lb"));
        assert_json_eq(&json!(ub), &js["ub"], 0.0, &format!("{id}: ub"));

        let solved = zuniga::solve_zuniga_scenario(&prepared);
        let r = &js["result"];
        assert_eq!(solved.result.status.as_str(), r["status"].as_str().unwrap());
        assert!(close(solved.result.objective_value, num(&r["objectiveValue"]), 1e-9), "{id}: objective");
        assert_json_eq(&json!(solved.result.fluxes), &r["fluxes"], 1e-9, &format!("{id}: fluxes"));
        assert_json_eq(&solved.zuniga.to_json(), &r["zuniga"], 1e-9, &format!("{id}: zuniga record"));
        let fluxes = &solved.result.fluxes;
        assert_json_eq(&json!(prepared.community.growth_rates(fluxes)), &js["growth"], 1e-9, &format!("{id}: growth"));
        assert_json_eq(&cross_feeding_json(&prepared.community.cross_feeding(fluxes, 1e-6)), &js["crossFeeding"], 1e-9, &format!("{id}: cross-feeding"));
        let index = build_cross_feeding_index(&prepared.community, fluxes, &CrossFeedingOptions::default());
        assert_json_eq(&index.to_json(), &js["explorer"], 1e-9, &format!("{id}: explorer"));
        // The trait sees the same community.
        assert_eq!(CommunityView::merged_reaction_ids(&prepared.community).len(), prepared.model.reactions.len());

        let replay = zuniga::restore_zuniga_scenario(&study, id, Model::from_value(&prepared.model.to_json()).unwrap(), None).unwrap();
        assert_json_eq(&json!(replay.knockouts), &js["replay"]["knockouts"], 0.0, &format!("{id}: replay knockouts"));
        let (lb, ub) = bounds_of(&replay.problem);
        assert_json_eq(&json!(lb), &js["replay"]["lb"], 0.0, &format!("{id}: replay lb"));
        assert_json_eq(&json!(ub), &js["replay"]["ub"], 0.0, &format!("{id}: replay ub"));
    }
    assert_eq!(zuniga::create_zuniga_scenario(&study, "missing").unwrap_err().message, fx["unknownScenario"].as_str().unwrap());
    let ec = (*study.models.ec).clone();
    assert_eq!(zuniga::restore_zuniga_scenario(&study, "missing", ec, None).unwrap_err().message, fx["unknownRestore"].as_str().unwrap());

    // Net transfer between cells, not the circulation through two transporters.
    let mut circulation = study.clone();
    let reactions = circulation.source_models.joint["reactions"].as_array_mut().unwrap();
    reactions.push(json!({"id": "PHEreturn_D", "metabolites": {"phe__L_c_D": -1, "phe__L_[smp]": 1}, "lower_bound": 0, "upper_bound": 1000,
        "objective_coefficient": 0, "gene_reaction_rule": "", "notes": {"source_reaction_id": "PHEreturn_D"}}));
    reactions.push(json!({"id": "PHEreuptake_A", "metabolites": {"phe__L_[smp]": -1, "phe__L_c_A": 1}, "lower_bound": 0, "upper_bound": 1000,
        "objective_coefficient": 0, "gene_reaction_rule": "", "notes": {"source_reaction_id": "PHEreuptake_A"}}));
    let prepared = zuniga::create_zuniga_scenario(&circulation, "mutant-coculture").unwrap();
    let fluxes = number_map(&fx["circulation"]["fluxes"]);
    assert_json_eq(&cross_feeding_json(&prepared.community.cross_feeding(&fluxes, 1e-6)), &fx["circulation"]["crossFeeding"], 0.0, "circulation");
    assert_json_eq(&cross_feeding_json(&prepared.community.cross_feeding(&fluxes, 5.0)), &fx["circulation"]["threshold5"], 0.0, "circulation at 5");
}

#[test]
fn the_loader_verifies_before_it_reads_and_reports_as_the_js_does() {
    let fx = fixture("zuniga.json");
    let m_models = fixture_bytes("zuniga-M-models.mat");
    let cm_models = fixture_bytes("zuniga-CM-modelsKO.mat");
    assert_eq!(coxeter_io::reproducibility::sha256_hex(&m_models), fx["hashes"]["M-models.mat"].as_str().unwrap());
    for case in fx["loader"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let fetch = |url: &str| -> Result<FetchResponse> {
            Ok(match name {
                "http-503" => FetchResponse { status: 503, body: Vec::new() },
                "integrity" => FetchResponse { status: 200, body: vec![1, 2, 3, 4] },
                "not-a-mat-file" => FetchResponse { status: 200, body: vec![1, 2, 3] },
                _ => FetchResponse { status: 200, body: if url.ends_with("CM-modelsKO.mat") { cm_models.clone() } else { m_models.clone() } },
            })
        };
        let mut progress: Vec<String> = Vec::new();
        let mut on_progress = |m: &str| progress.push(m.to_string());
        let result = zuniga::load_zuniga_study(
            &case["manifest"],
            ZunigaLoadOptions { fetcher: &fetch, cache: None, on_progress: Some(&mut on_progress) },
        );
        match (result, case.get("error")) {
            (Err(e), Some(msg)) => assert_eq!(e.message, msg.as_str().unwrap(), "{name}"),
            (r, e) => panic!("{name}: Rust {:?}, JS error {e:?}", r.map(|_| ())),
        }
        assert_json_eq(&json!(progress), &case["progress"], 0.0, &format!("{name}: progress"));
    }
}
