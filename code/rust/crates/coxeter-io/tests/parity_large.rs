//! Genome-scale parity: the published community members, the COBRA Toolbox
//! test models (SBML and MAT), the user's own models and workbooks, against
//! what the JS modules made of them (`rust/fixtures/large/io/`, git-ignored).
//! Every case is skipped when its fixture or its input file is absent.

mod common;

use std::path::Path;

use coxeter_core::model::Model;
use coxeter_core::testkit::{assert_json_eq, read_fixture};
use coxeter_io::mat5::read_mat5_file;
use coxeter_io::model_import::{ImportFile, parse_model_file, parse_model_sbml, prepare_imported_models};
use coxeter_io::sbml::{SbmlReadOptions, parse_cobra_sbml};
use coxeter_io::sbml_write::{SbmlWriteOptions, to_sbml};
use serde_json::{Value, json};

use common::*;

fn index() -> Option<Value> {
    let index = read_fixture("large/io/index.json");
    if index.is_none() {
        eprintln!("skipped: rust/fixtures/large/io is absent (run node rust/tools/fixtures-io.mjs)");
    }
    index
}

fn input(case: &Value) -> Option<String> {
    let path = case["input"].as_str()?;
    let full = if Path::new(path).is_absolute() { path.into() } else { repo(path) };
    match std::fs::read_to_string(&full) {
        Ok(text) => Some(text),
        Err(_) => {
            eprintln!("skipped: {path} is absent");
            None
        }
    }
}

fn file_name(path: &str) -> String {
    Path::new(path).file_name().unwrap().to_string_lossy().into_owned()
}

fn model_error(e: coxeter_core::model::ModelError) -> coxeter_io::Error {
    coxeter_io::Error::new(coxeter_io::ErrorKind::InvalidModel, e.to_string())
}

fn faithful() -> SbmlReadOptions {
    SbmlReadOptions { normalise_compartment: false, ..SbmlReadOptions::default() }
}

#[test]
fn published_members_write_and_read_back_like_javascript() {
    let Some(index) = index() else { return };
    let mut checked = 0;
    for case in index["published"].as_array().unwrap() {
        let key = case["key"].as_str().unwrap();
        let Some(fx) = read_fixture(&format!("large/io/published-{key}.json")) else { continue };
        let Some(text) = input(case) else { continue };
        let model = Model::from_json_str(&text).unwrap();
        let xml = to_sbml(&model, &SbmlWriteOptions::default()).unwrap();
        assert_same_text(&xml, &fixture_text(&format!("large/io/sbml/{key}.xml")), &format!("toSBML({key})"));
        let back = Model::from_value(&parse_cobra_sbml(&xml, &faithful())).unwrap();
        assert_json_eq(&model_json(&back), &fx["faithful"], 0.0, &format!("{key}: read back faithfully"));
        let back = Model::from_value(&parse_cobra_sbml(&xml, &SbmlReadOptions::default())).unwrap();
        assert_json_eq(&model_json(&back), &fx["cobra"], 0.0, &format!("{key}: read back normalised"));
        let upload = attempt(parse_model_file(&xml, &format!("{key}.xml")), |m| model_json(&m));
        assert_attempt(&upload, &fx["upload"], &format!("{key}: uploaded"));
        checked += 1;
    }
    eprintln!("published members checked: {checked}");
}

#[test]
fn sbml_files_read_like_javascript() {
    let Some(index) = index() else { return };
    let mut checked = 0;
    for case in index["sbml"].as_array().unwrap() {
        let key = case["key"].as_str().unwrap();
        let Some(fx) = read_fixture(&format!("large/io/sbml-{key}.json")) else { continue };
        let Some(xml) = input(case) else { continue };
        let name = file_name(case["input"].as_str().unwrap());

        let raw = parse_cobra_sbml(&xml, &SbmlReadOptions::default());
        assert_json_eq(&raw, &fx["cobra"]["raw"], 0.0, &format!("{key}: parseCOBRASBML"));
        let model = attempt(Model::from_value(&raw).map_err(model_error), |m| model_json(&m));
        assert_attempt(&model, &fx["cobra"]["model"], &format!("{key}: Model of parseCOBRASBML"));

        let opts = SbmlReadOptions { normalise_compartment: false, sentinel_bound: 999999.0, ..SbmlReadOptions::default() };
        let faithful = attempt(Model::from_value(&parse_cobra_sbml(&xml, &opts)).map_err(model_error), |m| model_json(&m));
        assert_attempt(&faithful, &fx["faithful"], &format!("{key}: faithful read"));

        let upload = parse_model_sbml(&xml, &name);
        let got = match &upload {
            Ok(m) => json!({"ok": model_json(m)}),
            Err(e) => json!({"error": e.message}),
        };
        assert_attempt(&got, &fx["upload"], &format!("{key}: parseModelSBML"));

        let prepared = attempt(prepare_imported_models(&[ImportFile { name: name.clone(), text: xml.clone() }]), |p| describe_prepared(&p));
        assert_attempt(&prepared, &fx["prepared"], &format!("{key}: prepareImportedModels"));

        if let Ok(model) = &upload {
            match (to_sbml(model, &SbmlWriteOptions::default()), fx["written"].get("file")) {
                (Ok(written), Some(file)) => {
                    assert_same_text(&written, &fixture_text(&format!("large/io/sbml/{}", file.as_str().unwrap())), &format!("{key}: toSBML"))
                }
                (Err(_), None) => {}
                (r, _) => panic!("{key}: toSBML gave {:?}, the JS {}", r.map(|x| x.len()), fx["written"]),
            }
            assert_json_eq(&links_of(model), &fx["links"], 0.0, &format!("{key}: annotation links"));
        }
        checked += 1;
    }
    eprintln!("SBML files checked: {checked}");
}

#[test]
fn json_models_import_and_export_like_javascript() {
    let Some(index) = index() else { return };
    let mut checked = 0;
    for case in index["json"].as_array().unwrap() {
        let key = case["key"].as_str().unwrap();
        let Some(fx) = read_fixture(&format!("large/io/json-{key}.json")) else { continue };
        let Some(text) = input(case) else { continue };
        let name = file_name(case["input"].as_str().unwrap());
        let prepared = prepare_imported_models(&[ImportFile { name, text }]).unwrap();
        assert_json_eq(&describe_prepared(&prepared), &fx["prepared"], 0.0, &format!("{key}: prepareImportedModels"));
        let model = &prepared.entries[0].model;
        match (to_sbml(model, &SbmlWriteOptions::default()), fx["written"].get("file")) {
            (Ok(xml), Some(file)) => {
                assert_same_text(&xml, &fixture_text(&format!("large/io/sbml/{}", file.as_str().unwrap())), &format!("{key}: toSBML"));
                let faithful = attempt(Model::from_value(&parse_cobra_sbml(&xml, &faithful())).map_err(model_error), |m| model_json(&m));
                assert_attempt(&faithful, &fx["roundTrip"]["faithful"], &format!("{key}: SBML read back faithfully"));
                let upload = attempt(parse_model_file(&xml, &format!("{key}.xml")), |m| model_json(&m));
                assert_attempt(&upload, &fx["roundTrip"]["upload"], &format!("{key}: SBML uploaded back"));
            }
            (Err(_), None) => {}
            (r, _) => panic!("{key}: toSBML gave {:?}, the JS {}", r.map(|x| x.len()), fx["written"]),
        }
        assert_json_eq(&links_of(model), &fx["links"], 0.0, &format!("{key}: annotation links"));
        checked += 1;
    }
    if let (Some(inputs), Some(fx)) = (index.get("together").and_then(Value::as_array), read_fixture("large/io/json-together.json")) {
        let files: Option<Vec<ImportFile>> = inputs
            .iter()
            .map(|p| {
                let p = p.as_str().unwrap();
                std::fs::read_to_string(p).ok().map(|text| ImportFile { name: file_name(p), text })
            })
            .collect();
        if let Some(files) = files {
            let prepared = prepare_imported_models(&files).unwrap();
            let summary: Vec<Value> = describe_prepared(&prepared)["entries"]
                .as_array()
                .unwrap()
                .iter()
                .map(|e| {
                    json!({"filename": e["filename"], "modelKey": e["modelKey"], "speciesId": e["speciesId"], "abundance": e["abundance"], "summary": e["summary"]})
                })
                .collect();
            assert_json_eq(&json!(summary), &fx["summary"], 0.0, "three real models uploaded together");
            assert_json_eq(&json!(prepared.medium), &fx["medium"], 0.0, "their inferred medium");
            checked += 1;
        }
    }
    eprintln!("JSON models checked: {checked}");
}

#[test]
fn mat_files_read_like_javascript() {
    let Some(index) = index() else { return };
    let mut checked = 0;
    for case in index["mat"].as_array().unwrap() {
        let key = case["key"].as_str().unwrap();
        let Some(fx) = read_fixture(&format!("large/io/mat-{key}.json")) else { continue };
        let path = case["input"].as_str().unwrap();
        if !Path::new(path).exists() {
            continue;
        }
        let got = describe_mat(read_mat5_file(path));
        let expected = match fx.get("error") {
            Some(e) => json!({"error": e}),
            None => json!({"variables": fx["variables"]}),
        };
        assert_json_eq(&got, &expected, 0.0, &format!("readMat5({key})"));
        checked += 1;
    }
    eprintln!("MAT files checked: {checked}");
}

#[test]
fn workbooks_read_like_javascript() {
    let Some(index) = index() else { return };
    let mut checked = 0;
    for case in index["xlsx"].as_array().unwrap() {
        let key = case["key"].as_str().unwrap();
        let Some(fx) = read_fixture(&format!("large/io/xlsx-{key}.json")) else { continue };
        let Ok(bytes) = std::fs::read(case["input"].as_str().unwrap()) else { continue };
        let got = describe_workbook(bytes);
        let mut expected = fx.clone();
        expected.as_object_mut().unwrap().remove("input");
        assert_json_eq(&got, &expected, 0.0, &format!("workbook {key}"));
        checked += 1;
    }
    eprintln!("workbooks checked: {checked}");
}
