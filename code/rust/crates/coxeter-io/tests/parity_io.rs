//! The io modules against what the JavaScript modules made of the same inputs
//! (`rust/tools/fixtures-io.mjs` → `rust/fixtures/io/`).

mod common;

use chrono::NaiveDate;
use coxeter_core::model::Model;
use coxeter_core::testkit::{assert_json_eq, read_fixture};
use coxeter_io::file_names::{community_data_file_name, export_file_name, file_stem, member_model_file_name, timestamped_file_name};
use coxeter_io::mat5::read_mat5;
use coxeter_io::model_edit::{add_metabolite_to_model, add_reaction_to_model, gene_ids_from_gpr, parse_stoichiometry};
use coxeter_io::model_import::{ImportFile, infer_community_medium, parse_model_file, parse_model_sbml, prepare_imported_models};
use coxeter_io::provenance::{annotation_links, annotation_object, extract_identifiers_org, normalize_annotations};
use coxeter_io::sbml::{UnmangleOptions, parse_cobra_sbml, unmangle_id};
use coxeter_io::sbml_write::{SbmlWriteOptions, genes_in_gpr, mangle_id, parse_gpr, to_sbml};
use serde_json::{Value, json};

use common::*;

fn fixture(name: &str) -> Value {
    read_fixture(&format!("io/{name}")).unwrap_or_else(|| panic!("io/{name} is missing: run node rust/tools/fixtures-io.mjs"))
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap()
}

#[test]
fn file_names_match_javascript() {
    let fx = fixture("file-names.json");
    for case in fx["fileStem"].as_array().unwrap() {
        assert_eq!(file_stem(s(&case["input"]), "coxeter"), s(&case["out"]), "fileStem({})", case["input"]);
        assert_eq!(file_stem(s(&case["input"]), "fb"), s(&case["fallback"]), "fileStem({}, 'fb')", case["input"]);
    }
    for case in fx["exportFileName"].as_array().unwrap() {
        let a = case["args"].as_array().unwrap();
        assert_eq!(export_file_name(s(&a[0]), s(&a[1]), s(&a[2])), s(&case["out"]));
    }
    for case in fx["memberModelFileName"].as_array().unwrap() {
        let a = case["args"].as_array().unwrap();
        assert_eq!(member_model_file_name(s(&a[0]), s(&a[1]), s(&a[2])), s(&case["out"]));
    }
    for case in fx["communityDataFileName"].as_array().unwrap() {
        let a = case["args"].as_array().unwrap();
        assert_eq!(community_data_file_name(s(&a[0]), s(&a[1])), s(&case["out"]), "{:?}", a);
    }
    for case in fx["timestampedFileName"].as_array().unwrap() {
        let a = case["args"].as_array().unwrap();
        let d = &case["date"];
        let n = |k: &str| d[k].as_i64().unwrap();
        let when = NaiveDate::from_ymd_opt(n("year") as i32, n("month") as u32, n("day") as u32)
            .unwrap()
            .and_hms_opt(n("hour") as u32, n("minute") as u32, 0)
            .unwrap();
        assert_eq!(timestamped_file_name(s(&a[0]), s(&a[1]), s(&a[2]), &when), s(&case["out"]));
    }
    let suffixes = &fx["suffixes"];
    use coxeter_io::file_names::export_suffix as x;
    for (k, v) in [("map", x::MAP), ("model", x::MODEL), ("scenario", x::SCENARIO), ("fluxes", x::FLUXES), ("repro", x::REPRO), ("methods", x::METHODS), ("findings", x::FINDINGS)] {
        assert_eq!(s(&suffixes[k]), v);
    }
}

#[test]
fn provenance_matches_javascript() {
    let fx = fixture("provenance.json");
    for case in fx["normalizeAnnotations"].as_array().unwrap() {
        let got: Vec<Value> = normalize_annotations(&case["annotation"], &case["notes"]).into_iter().map(|(k, v)| json!([k, v])).collect();
        assert_json_eq(&Value::Array(got), &case["out"], 0.0, &format!("normalizeAnnotations({})", case["annotation"]));
    }
    for case in fx["annotationLinks"].as_array().unwrap() {
        let got = serde_json::to_value(annotation_links(&case["annotation"], &case["notes"], s(&case["type"]))).unwrap();
        assert_json_eq(&got, &case["out"], 0.0, &format!("annotationLinks({})", case["annotation"]));
    }
    for case in fx["extractIdentifiersOrg"].as_array().unwrap() {
        let got = attempt(extract_identifiers_org(s(&case["body"])), Value::Object);
        assert_attempt(&got, case, &format!("extractIdentifiersOrg({})", case["body"]));
    }
    for case in fx["annotationObject"].as_array().unwrap() {
        assert_json_eq(&annotation_object(&case["raw"]), &case["out"], 0.0, "annotationObject");
    }
}

#[test]
fn model_edits_match_javascript() {
    let fx = fixture("model-edit.json");
    for case in fx["parseStoichiometry"].as_array().unwrap() {
        let got = attempt(parse_stoichiometry(s(&case["text"])), |m| {
            Value::Array(m.into_iter().map(|(k, v)| json!([k, v])).collect())
        });
        assert_attempt(&got, case, &format!("parseStoichiometry({:?})", case["text"]));
    }
    for case in fx["geneIdsFromGPR"].as_array().unwrap() {
        assert_json_eq(&json!(gene_ids_from_gpr(s(&case["rule"]))), &case["out"], 0.0, &format!("geneIdsFromGPR({})", case["rule"]));
    }
    let edit = &fx["edit"];
    let mut model = Model::from_value(&edit["start"]).unwrap();
    for (i, step) in edit["steps"].as_array().unwrap().iter().enumerate() {
        let r = if step["kind"] == "metabolite" {
            add_metabolite_to_model(&mut model, &step["spec"]).map(|_| ())
        } else {
            add_reaction_to_model(&mut model, &step["spec"]).map(|_| ())
        };
        let got = attempt(r, |_| json!(true));
        assert_attempt(&got, step, &format!("edit step {i} {}", step["spec"]));
    }
    assert_json_eq(&model_json(&model), &edit["final"], 0.0, "model after the edits");
}

#[test]
fn bundled_models_write_the_same_sbml() {
    let fx = fixture("sbml-write.json");
    let core = fx["core"].as_array().unwrap();
    assert!(core.len() >= 5);
    for name in core {
        let name = s(name);
        let model = Model::from_json_str(&std::fs::read_to_string(repo(&format!("models/{name}.json"))).unwrap()).unwrap();
        let xml = to_sbml(&model, &SbmlWriteOptions::default()).unwrap();
        assert_same_text(&xml, &fixture_text(&format!("io/sbml/{name}.xml")), &format!("toSBML({name})"));
    }
    for (name, probe) in fx["probes"].as_object().unwrap() {
        let model = Model::from_value(&probe["doc"]).unwrap();
        let mut opts = SbmlWriteOptions::default();
        if let Some(o) = probe.get("opts") {
            opts.objective_id = s(&o["objectiveId"]).into();
            opts.objective_type = s(&o["objectiveType"]).into();
        }
        match (to_sbml(&model, &opts), probe.get("ok")) {
            (Ok(xml), Some(expected)) => assert_same_text(&xml, s(expected), &format!("toSBML probe {name}")),
            // The JS crashes with a TypeError on a dangling operator; here it is an error too.
            (Err(_), None) => {}
            (r, _) => panic!("probe {name}: Rust gave {r:?}, the JS gave {probe}"),
        }
    }
    for case in fx["mangleId"].as_array().unwrap() {
        let got = attempt(mangle_id(s(&case["id"]), s(&case["prefix"])), Value::from);
        assert_attempt(&got, case, &format!("mangleId({}, {})", case["id"], case["prefix"]));
    }
    for case in fx["parseGPR"].as_array().unwrap() {
        let rule = s(&case["rule"]);
        let tree = parse_gpr(rule).map_or(Value::Null, |t| t.to_json());
        assert_json_eq(&tree, &case["out"], 0.0, &format!("parseGPR({rule:?})"));
        match (genes_in_gpr(rule), case["genes"].get("ok")) {
            (Ok(genes), Some(expected)) => assert_json_eq(&json!(genes), expected, 0.0, &format!("genesInGPR({rule:?})")),
            (Err(_), None) => {}
            (r, _) => panic!("genesInGPR({rule:?}): Rust gave {r:?}, the JS gave {}", case["genes"]),
        }
    }
}

fn check_read(xml: &str, filename: &str, expected: &Value, what: &str) {
    for case in expected["cobra"].as_array().unwrap() {
        let opts = read_options(&case["opts"]);
        let raw = parse_cobra_sbml(xml, &opts);
        assert_json_eq(&raw, &case["ok"], 0.0, &format!("{what}: parseCOBRASBML {}", case["opts"]));
        let model = attempt(Model::from_value(&raw).map_err(|e| coxeter_io::Error::new(coxeter_io::ErrorKind::InvalidModel, e.to_string())), |m| model_json(&m));
        assert_attempt(&model, &case["model"], &format!("{what}: Model of parseCOBRASBML {}", case["opts"]));
    }
    let upload = attempt(parse_model_sbml(xml, filename), |m| model_json(&m));
    assert_attempt(&upload, &expected["upload"], &format!("{what}: parseModelSBML"));
    let file = attempt(parse_model_file(xml, &filename.replace(".xml", ".txt")), |m| model_json(&m));
    assert_attempt(&file, &expected["file"], &format!("{what}: parseModelFile"));
}

#[test]
fn sbml_reads_like_javascript() {
    let fx = fixture("sbml-read.json");
    for (name, case) in fx["texts"].as_object().unwrap() {
        check_read(s(&case["xml"]), &format!("{name}.xml"), case, name);
    }
    let core = fx["core"].as_object().unwrap();
    assert!(core.len() >= 5);
    for (name, case) in core {
        let xml = fixture_text(&format!("io/sbml/{name}.xml"));
        check_read(&xml, &format!("{name}.xml"), case, name);
    }
    let probes = fixture("sbml-write.json");
    for (name, case) in fx["probes"].as_object().unwrap() {
        check_read(s(&probes["probes"][name]["ok"]), &format!("{name}.xml"), case, name);
    }
    for case in fx["unmangle"].as_array().unwrap() {
        let opts = UnmangleOptions { normalise_compartment: case["normaliseCompartment"].as_bool().unwrap() };
        assert_eq!(unmangle_id(s(&case["id"]), s(&case["prefix"]), opts), s(&case["out"]), "unmangleId {case}");
    }
}

#[test]
fn uploads_import_like_javascript() {
    let fx = fixture("model-import.json");
    for (name, case) in fx["cases"].as_object().unwrap() {
        let files: Vec<ImportFile> = case["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| ImportFile {
                name: s(&f["name"]).to_string(),
                text: match f.get("path") {
                    Some(p) => std::fs::read_to_string(repo(s(p))).unwrap(),
                    None => s(&f["text"]).to_string(),
                },
            })
            .collect();
        let got = attempt(prepare_imported_models(&files), |p| describe_prepared(&p));
        assert_attempt(&got, case, &format!("prepareImportedModels {name}"));
    }
    let models: Vec<Model> = ["cyano_core_mc", "ecoli_core_mc", "lactobacillus_core_mc", "methanogen_core_mc", "yeast_core_mc"]
        .iter()
        .map(|n| Model::from_json_str(&std::fs::read_to_string(repo(&format!("models/{n}.json"))).unwrap()).unwrap())
        .collect();
    let medium = infer_community_medium(&models.iter().collect::<Vec<_>>());
    assert_json_eq(&json!(medium), &fx["medium"]["core"], 0.0, "inferCommunityMedium of the bundled models");
}

#[test]
fn workbooks_read_like_javascript() {
    let fx = fixture("xlsx.json");
    for (name, expected) in fx.as_object().unwrap() {
        let got = describe_workbook(fixture_bytes(&format!("io/xlsx/{name}.xlsx")));
        match (got.get("error"), expected.get("error")) {
            (Some(a), Some(e)) => assert_eq!(a, e, "workbook {name}"),
            _ => {
                for (sheet, rows) in expected["sheets"].as_object().unwrap() {
                    assert_attempt(&got["sheets"][sheet], rows, &format!("workbook {name}, sheet {sheet}"));
                }
                assert_json_eq(&got["sheetNames"], &expected["sheetNames"], 0.0, &format!("workbook {name}"));
            }
        }
    }
}

#[test]
fn mat_files_read_like_javascript() {
    let fx = fixture("mat5.json");
    for (name, expected) in fx.as_object().unwrap() {
        let got = describe_mat(read_mat5(&fixture_bytes(&format!("io/mat/{name}.mat"))));
        assert_json_eq(&got, expected, 0.0, &format!("readMat5({name})"));
    }
}
