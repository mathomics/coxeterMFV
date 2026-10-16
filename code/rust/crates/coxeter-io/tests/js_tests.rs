//! The JavaScript unit tests of the ported modules, ported:
//! `test/model-import.test.mjs`, `test/sbml-write.test.mjs`,
//! `test/model-edit.test.mjs`, `test/mat5.test.mjs`,
//! `test/provenance.test.mjs`, the xlsx reader tests of `test/gut9.test.mjs`,
//! and the parts of `test/label-sources.test.mjs` and `test/models.test.mjs`
//! that read model files. (`test/reproducibility.test.mjs` and the file-name
//! tests of `test/ui-audit-fixes.test.mjs` are ported next to their modules.)
//!
//! Tests that drive the `Store` (an upload drawing a map, the Store's model
//! export) belong to the GUI crate and are not here.

mod common;

use base64::Engine;
use coxeter_core::model::Model;
use coxeter_io::mat5::{MatValue, TypedArrayKind, read_mat5};
use coxeter_io::model_edit::{add_metabolite_to_model, add_reaction_to_model, gene_ids_from_gpr, parse_stoichiometry};
use coxeter_io::model_import::{ImportFile, infer_community_medium, parse_model_file, parse_model_json, parse_model_sbml, prepare_imported_models};
use coxeter_io::sbml::{SbmlReadOptions, UnmangleOptions, parse_cobra_sbml, unmangle_id};
use coxeter_io::sbml_write::{SbmlWriteOptions, mangle_id, parse_gpr, to_sbml};
use coxeter_io::xlsx::read_workbook;
use regex::Regex;
use serde_json::{Value, json};

use common::repo;

fn model(doc: Value) -> Model {
    Model::from_value(&doc).unwrap()
}

fn catalogue() -> Vec<String> {
    let mut files: Vec<String> = std::fs::read_dir(repo("models"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|f| f.ends_with(".json") && f != "index.json" && f != "scenarios.json")
        .collect();
    files.sort();
    files
}

fn read_model(rel: &str) -> Model {
    Model::from_json_str(&std::fs::read_to_string(repo(rel)).unwrap()).unwrap()
}

// model-import.test

fn model_doc(id: &str) -> Value {
    json!({
        "id": id,
        "compartments": {"c": "Cytosol", "e": "Extracellular"},
        "metabolites": [
            {"id": "a_c", "name": "A", "compartment": "c", "formula": "C", "charge": 0},
            {"id": "a_e", "name": "A", "compartment": "e", "formula": "C", "charge": 0},
        ],
        "reactions": [
            {"id": "EX_a_e", "name": "A exchange", "metabolites": {"a_e": -1}, "lower_bound": -10, "upper_bound": 1000},
            {"id": "At", "name": "A transport", "metabolites": {"a_e": -1, "a_c": 1}, "lower_bound": 0, "upper_bound": 1000},
            {"id": "BIOMASS", "metabolites": {"a_c": -1}, "lower_bound": 0, "upper_bound": 1000, "objective_coefficient": 1},
        ],
    })
}

#[test]
fn a_normal_cobra_json_model_imports() {
    let m = parse_model_json(&model_doc("eco").to_string(), "eco.json").unwrap();
    assert_eq!(m.id, "eco");
    assert_eq!(m.reactions.len(), 3);
    assert_eq!(m.metabolites.len(), 2);
    assert_eq!(m.biomass_reaction_id().as_deref(), Some("BIOMASS"));
}

#[test]
fn an_exchange_uptake_lower_bound_becomes_inferred_community_medium() {
    let m = parse_model_json(&model_doc("eco").to_string(), "eco.json").unwrap();
    assert_eq!(json!(infer_community_medium(&[&m])), json!({"a": 10.0}));
}

#[test]
fn multiple_uploaded_json_files_become_distinct_species() {
    let prepared = prepare_imported_models(&[
        ImportFile { name: "first.json".into(), text: model_doc("same").to_string() },
        ImportFile { name: "second.json".into(), text: model_doc("same").to_string() },
    ])
    .unwrap();
    assert_eq!(prepared.entries.len(), 2);
    assert_eq!(prepared.entries.iter().map(|e| e.species_id.as_str()).collect::<Vec<_>>(), ["same", "same_2"]);
    assert_eq!(prepared.entries[0].abundance, 0.5);
    assert_eq!(prepared.entries[1].abundance, 0.5);
}

#[test]
fn a_reaction_referencing_an_unknown_metabolite_is_rejected_clearly() {
    let mut raw = model_doc("eco");
    raw["reactions"].as_array_mut().unwrap().push(json!({"id": "broken", "metabolites": {"missing_c": -1}, "lower_bound": 0, "upper_bound": 1000}));
    let err = parse_model_json(&raw.to_string(), "broken.json").unwrap_err();
    assert!(err.message.contains("unknown metabolite"), "{err}");
}

#[test]
fn duplicate_reaction_ids_are_rejected_before_model_construction() {
    let mut raw = model_doc("eco");
    let first = raw["reactions"][0].clone();
    raw["reactions"].as_array_mut().unwrap().push(first);
    let err = parse_model_json(&raw.to_string(), "dup.json").unwrap_err();
    assert!(err.message.contains("duplicate reaction id"), "{err}");
}

#[test]
fn invalid_json_names_the_offending_file() {
    let err = parse_model_json("{not-json", "bad.json").unwrap_err();
    assert!(err.message.starts_with("bad.json: invalid JSON"), "{err}");
}

const SBML_FIXTURE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<sbml xmlns="http://www.sbml.org/sbml/level3/version1/core"
 xmlns:fbc="http://www.sbml.org/sbml/level3/version1/fbc/version2"
 level="3" version="1" fbc:required="false">
 <model id="eco_sbml" name="SBML test model" fbc:strict="true">
  <listOfCompartments>
   <compartment id="c" name="Cytosol" constant="true"/>
   <compartment id="e" name="Extracellular" constant="true"/>
  </listOfCompartments>
  <listOfSpecies>
   <species id="a_e" name="A external" compartment="e" fbc:chemicalFormula="C" fbc:charge="0"/>
   <species id="a_c" name="A" compartment="c" fbc:chemicalFormula="C" fbc:charge="0"/>
  </listOfSpecies>
  <listOfParameters>
   <parameter id="LB_EX" value="-10" constant="true"/>
   <parameter id="LB_ZERO" value="0" constant="true"/>
   <parameter id="UB" value="1000" constant="true"/>
  </listOfParameters>
  <fbc:listOfGeneProducts>
   <fbc:geneProduct fbc:id="g1" fbc:label="gene one"/>
   <fbc:geneProduct fbc:id="g2" fbc:label="gene two"/>
  </fbc:listOfGeneProducts>
  <listOfReactions>
   <reaction id="EX_a_e" reversible="true" fbc:lowerFluxBound="LB_EX" fbc:upperFluxBound="UB">
    <listOfReactants><speciesReference species="a_e" stoichiometry="1"/></listOfReactants>
   </reaction>
   <reaction id="At" reversible="false" fbc:lowerFluxBound="LB_ZERO" fbc:upperFluxBound="UB">
    <listOfReactants><speciesReference species="a_e" stoichiometry="1"/></listOfReactants>
    <listOfProducts><speciesReference species="a_c" stoichiometry="1"/></listOfProducts>
    <fbc:geneProductAssociation><fbc:and>
      <fbc:geneProductRef fbc:geneProduct="g1"/>
      <fbc:geneProductRef fbc:geneProduct="g2"/>
    </fbc:and></fbc:geneProductAssociation>
   </reaction>
   <reaction id="BIOMASS" reversible="false" fbc:lowerFluxBound="LB_ZERO" fbc:upperFluxBound="UB">
    <listOfReactants><speciesReference species="a_c" stoichiometry="1"/></listOfReactants>
   </reaction>
  </listOfReactions>
  <fbc:listOfObjectives fbc:activeObjective="obj">
   <fbc:objective fbc:id="obj" fbc:type="maximize">
    <fbc:listOfFluxObjectives><fbc:fluxObjective fbc:reaction="BIOMASS" fbc:coefficient="1"/></fbc:listOfFluxObjectives>
   </fbc:objective>
  </fbc:listOfObjectives>
 </model>
</sbml>"#;

#[test]
fn sbml_level3_fbc_imports_compartments_bounds_objective_and_gpr() {
    let m = parse_model_sbml(SBML_FIXTURE, "eco.xml").unwrap();
    assert_eq!(m.id, "eco_sbml");
    assert_eq!(m.compartment_names["c"], "Cytosol");
    assert_eq!(m.metabolites["a_e"].formula, "C");
    assert_eq!(m.reactions["EX_a_e"].lower_bound, -10.0);
    assert_eq!(m.reactions["At"].upper_bound, 1000.0);
    assert_eq!(m.reactions["At"].gene_reaction_rule, "g1 and g2");
    assert_eq!(m.biomass_reaction_id().as_deref(), Some("BIOMASS"));
    assert_eq!(json!(infer_community_medium(&[&m])), json!({"a": 10.0}));
}

#[test]
fn generic_model_parser_detects_sbml_by_extension_or_xml_content() {
    assert_eq!(parse_model_file(SBML_FIXTURE, "eco.sbml").unwrap().id, "eco_sbml");
    assert_eq!(parse_model_file(SBML_FIXTURE, "eco.xml").unwrap().reactions.len(), 3);
}

#[test]
fn paired_sbml_species_keep_notes_after_the_genome_scale_import_optimisation() {
    let paired = SBML_FIXTURE
        .replace(
            r#"<species id="a_e" name="A external" compartment="e" fbc:chemicalFormula="C" fbc:charge="0"/>"#,
            r#"<species id="a_e" name="A external" compartment="e" fbc:chemicalFormula="C" fbc:charge="0"></species>"#,
        )
        .replace(
            r#"<species id="a_c" name="A" compartment="c" fbc:chemicalFormula="C" fbc:charge="0"/>"#,
            r#"<species id="a_c" name="A" compartment="c" fbc:chemicalFormula="C" fbc:charge="0"><notes><p>YEAST9_NOTE</p></notes></species>"#,
        );
    let m = parse_model_sbml(&paired, "paired.xml").unwrap();
    assert!(m.metabolites["a_c"].notes["text"].as_str().unwrap().contains("YEAST9_NOTE"));
}

#[test]
fn json_and_sbml_files_can_be_imported_together_as_a_community() {
    let prepared = prepare_imported_models(&[
        ImportFile { name: "first.json".into(), text: model_doc("json_model").to_string() },
        ImportFile { name: "second.xml".into(), text: SBML_FIXTURE.into() },
    ])
    .unwrap();
    assert_eq!(prepared.entries.len(), 2);
    assert_eq!(prepared.entries[1].model.id, "eco_sbml");
    assert_eq!(prepared.entries[0].abundance, 0.5);
    assert_eq!(prepared.entries[1].abundance, 0.5);
}

#[test]
fn invalid_xml_is_rejected_with_a_clear_sbml_error() {
    let err = parse_model_sbml("<xml/>", "bad.xml").unwrap_err();
    assert!(err.message.contains("valid SBML document"), "{err}");
}

// sbml-write.test

fn same_stoichiometry(a: &Model, b: &Model, rid: &str) -> bool {
    let (x, y) = (&a.reactions[rid].metabolites, &b.reactions[rid].metabolites);
    x.len() == y.len() && x.iter().all(|(k, v)| y.get(k) == Some(v))
}

fn round_trip(doc: &Value) -> (Model, Model, String) {
    let before = Model::from_value(doc).unwrap();
    let xml = to_sbml(&before, &SbmlWriteOptions::default()).unwrap();
    // Faithful ids: the reader's default rewrites a trailing `[c_Ba]` to `_c_Ba`,
    // which is a deliberate normalisation and not invertible.
    let opts = SbmlReadOptions { normalise_compartment: false, ..SbmlReadOptions::default() };
    let after = Model::from_value(&parse_cobra_sbml(&xml, &opts)).unwrap();
    (before, after, xml)
}

fn assert_same_model(before: &Model, after: &Model, label: &str) {
    assert_eq!(after.reactions.len(), before.reactions.len(), "{label}: reaction count");
    assert_eq!(after.metabolites.len(), before.metabolites.len(), "{label}: metabolite count");
    for rxn in before.reactions.values() {
        let back = after.reactions.get(&rxn.id).unwrap_or_else(|| panic!("{label}: reaction {} is missing", rxn.id));
        assert!(same_stoichiometry(before, after, &rxn.id), "{label}: {} stoichiometry", rxn.id);
        assert_eq!(back.lower_bound, rxn.lower_bound, "{label}: {} lower bound", rxn.id);
        assert_eq!(back.upper_bound, rxn.upper_bound, "{label}: {} upper bound", rxn.id);
        assert_eq!(back.name, rxn.name, "{label}: {} name", rxn.id);
    }
    for met in before.metabolites.values() {
        let back = after.metabolites.get(&met.id).unwrap_or_else(|| panic!("{label}: metabolite {} is missing", met.id));
        assert_eq!(back.formula, met.formula, "{label}: {} formula", met.id);
        assert_eq!(back.charge, met.charge, "{label}: {} charge", met.id);
        assert_eq!(back.compartment, met.compartment, "{label}: {} compartment", met.id);
    }
    assert_eq!(after.objective_reactions(), before.objective_reactions(), "{label}: objective");
}

#[test]
fn identifiers_survive_the_trip_into_sbml_and_back() {
    let sid = Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*$").unwrap();
    for id in ["12ethd[c_Ba]", "EX_12ppd-R[u]tr_Bt", "PFK(ppi)_Bt", "12ppd-R_e", "EX_glc__D_e", "BIOMASS_Ec_iAF1260_core_59p81M", "3'5'-cyclic"] {
        let written = mangle_id(id, "M_").unwrap();
        assert!(sid.is_match(&written), "{id} must become a valid SId");
        assert_eq!(unmangle_id(&written, "M_", UnmangleOptions { normalise_compartment: false }), id);
    }
}

#[test]
fn gene_rules_survive_the_trip_through_fbc_gene_product_association() {
    let doc = json!({
        "id": "gpr", "name": "GPR probe", "compartments": {"c": "Cytosol"},
        "metabolites": [
            {"id": "a_c", "name": "A", "compartment": "c", "formula": "C", "charge": 0},
            {"id": "b_c", "name": "B", "compartment": "c", "formula": "C", "charge": 0},
        ],
        "reactions": [
            {"id": "R1", "name": "R one", "metabolites": {"a_c": -1, "b_c": 1}, "lower_bound": 0, "upper_bound": 1000, "gene_reaction_rule": "(b0001 and b0002) or b0003"},
            {"id": "R2", "name": "R two", "metabolites": {"b_c": -1}, "lower_bound": -1000, "upper_bound": 1000, "gene_reaction_rule": "b0004"},
        ],
        "genes": [{"id": "b0001", "name": "gene one"}],
        "objective": {"R1": 1},
    });
    let (before, after, xml) = round_trip(&doc);
    assert!(xml.contains("<fbc:geneProductAssociation>"));
    assert!(xml.contains("<fbc:listOfGeneProducts>"));
    assert_same_model(&before, &after, "gpr");
    assert_eq!(parse_gpr(&after.reactions["R1"].gene_reaction_rule), parse_gpr("(b0001 and b0002) or b0003"));
    assert_eq!(after.reactions["R2"].gene_reaction_rule, "b0004");
    let mut ids: Vec<String> = after.genes.iter().map(|g| g["id"].as_str().unwrap().to_string()).collect();
    ids.sort();
    assert_eq!(ids, ["b0001", "b0002", "b0003", "b0004"]);
    assert_eq!(after.genes.iter().find(|g| g["id"] == "b0001").unwrap()["name"], "gene one");
}

#[test]
fn the_document_is_level3_version1_with_fbc_v2_and_strict() {
    let model = read_model("models/ecoli_core_mc.json");
    let xml = to_sbml(&model, &SbmlWriteOptions::default()).unwrap();
    assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
    assert!(xml.contains("xmlns=\"http://www.sbml.org/sbml/level3/version1/core\""));
    assert!(xml.contains("xmlns:fbc=\"http://www.sbml.org/sbml/level3/version1/fbc/version2\""));
    assert!(xml.contains("level=\"3\" version=\"1\" fbc:required=\"false\""));
    assert!(xml.contains("fbc:strict=\"true\""));
    for required in ["hasOnlySubstanceUnits=", "boundaryCondition=", "constant="] {
        assert!(xml.contains(required), "species must carry {required}");
    }
    assert!(Regex::new(r#"<speciesReference[^>]*constant="true""#).unwrap().is_match(&xml));
    assert!(Regex::new(r#"<reaction[^>]*fast="false""#).unwrap().is_match(&xml));
    assert!(Regex::new(r#"<parameter[^>]*id="cobra_default_lb"[^>]*value="-1000""#).unwrap().is_match(&xml));
    assert!(Regex::new(r#"<parameter[^>]*sboTerm="SBO:0000626""#).unwrap().is_match(&xml));
    let parameters = Regex::new(r"<parameter\b").unwrap().find_iter(&xml).count();
    assert!(parameters < model.reactions.len(), "expected pooled bounds, got {parameters} parameters");
    assert!(xml.contains("<fbc:listOfObjectives fbc:activeObjective=\"obj\">"));
    assert!(xml.contains("<fbc:objective fbc:id=\"obj\" fbc:type=\"maximize\">"));
    assert!(Regex::new(r#"<fbc:fluxObjective[^>]*fbc:coefficient="1""#).unwrap().is_match(&xml));
}

#[test]
fn names_and_formulas_are_xml_escaped() {
    let (before, after, xml) = round_trip(&json!({
        "id": "esc", "compartments": {"c": "C"},
        "metabolites": [{"id": "m_c", "name": "a < b & \"c\"", "compartment": "c", "formula": "CHO", "charge": 0}],
        "reactions": [{"id": "R", "name": "x > y & z", "metabolites": {"m_c": -1}, "lower_bound": 0, "upper_bound": 1}],
        "objective": {"R": 1},
    }));
    let stripped = Regex::new("&(amp|lt|gt|quot);").unwrap().replace_all(&xml, "");
    assert!(!Regex::new(r#"name="[^"]*[<&]"#).unwrap().is_match(&stripped), "no raw markup may survive inside an attribute");
    assert_same_model(&before, &after, "escaping");
    assert_eq!(after.metabolites["m_c"].name, "a < b & \"c\"");
}

#[test]
fn a_formula_fbc_cannot_hold_is_kept_in_the_notes_instead() {
    let (before, after, xml) = round_trip(&json!({
        "id": "formulas", "compartments": {"c": "Cytosol"},
        "metabolites": [
            {"id": "bio_c", "name": "Biomass", "compartment": "c", "charge": 0, "formula": "C37.024041172583H67.803828393317O20.609573521004"},
            {"id": "acp_c", "name": "ACP", "compartment": "c", "charge": -1, "formula": "C11H20O6N2PSApo_acp"},
            {"id": "glc_c", "name": "Glucose", "compartment": "c", "charge": 0, "formula": "C6H12O6"},
        ],
        "reactions": [{"id": "R", "name": "R", "metabolites": {"bio_c": -1, "acp_c": -1, "glc_c": 1}, "lower_bound": 0, "upper_bound": 1000}],
        "objective": {"R": 1},
    }));
    assert!(xml.contains("fbc:chemicalFormula=\"C6H12O6\""), "a conformant formula stays an attribute");
    assert!(!xml.contains("fbc:chemicalFormula=\"C11H20O6N2PSApo_acp\""), "an underscore may not reach fbc:chemicalFormula");
    assert!(xml.contains("FORMULA: C11H20O6N2PSApo_acp"));
    assert_same_model(&before, &after, "formulas");
}

#[test]
fn every_catalogue_model_survives_the_round_trip_through_sbml() {
    for file in catalogue() {
        let doc: Value = serde_json::from_str(&std::fs::read_to_string(repo(&format!("models/{file}"))).unwrap()).unwrap();
        let (before, after, _) = round_trip(&doc);
        assert_same_model(&before, &after, &file);
    }
}

#[test]
fn every_published_member_survives_the_round_trip_through_sbml() {
    let dir = repo("models/published");
    let Ok(entries) = std::fs::read_dir(&dir) else { return };
    let mut checked = 0;
    for community in entries {
        let community = community.unwrap().path();
        for f in std::fs::read_dir(&community).unwrap() {
            let path = f.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if !name.starts_with("model-") {
                continue;
            }
            let doc: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            let (before, after, _) = round_trip(&doc);
            assert_same_model(&before, &after, &path.display().to_string());
            checked += 1;
        }
    }
    assert!(checked > 0);
}

#[test]
fn an_sbml_file_this_app_writes_can_be_uploaded_back_into_it() {
    let before = read_model("models/ecoli_core_mc.json");
    let uploaded = parse_model_file(&to_sbml(&before, &SbmlWriteOptions::default()).unwrap(), "uploaded.xml").unwrap();
    assert_eq!(uploaded.reactions.len(), before.reactions.len());
    assert_eq!(uploaded.metabolites.len(), before.metabolites.len());
    for rxn in before.reactions.values() {
        let back = uploaded.reactions.get(&rxn.id).unwrap_or_else(|| panic!("reaction {} arrived under a different id", rxn.id));
        assert!(same_stoichiometry(&before, &uploaded, &rxn.id), "{} stoichiometry", rxn.id);
        assert_eq!(back.lower_bound, rxn.lower_bound);
        assert_eq!(back.upper_bound, rxn.upper_bound);
    }
    for met in before.metabolites.values() {
        let back = uploaded.metabolites.get(&met.id).unwrap_or_else(|| panic!("metabolite {} arrived under a different id", met.id));
        assert_eq!(back.formula, met.formula);
        assert_eq!(back.charge, met.charge);
        assert_eq!(back.compartment, met.compartment);
    }
    assert_eq!(uploaded.objective_reactions(), before.objective_reactions());
}

#[test]
fn uploading_carries_gene_rules_and_odd_formulas_across() {
    let doc = json!({
        "id": "upload_probe", "compartments": {"c": "Cytosol"},
        "metabolites": [
            {"id": "a_c", "name": "A", "compartment": "c", "formula": "C11H20O6N2PSApo_acp", "charge": -1},
            {"id": "b_c", "name": "B", "compartment": "c", "formula": "C6H12O6", "charge": 0},
        ],
        "reactions": [
            {"id": "12ppd-R_t", "name": "Transport", "metabolites": {"a_c": -1, "b_c": 1}, "lower_bound": -10, "upper_bound": 1000, "gene_reaction_rule": "(b0001 and b0002) or b0003"},
            {"id": "EX_b(e)", "name": "B exchange", "metabolites": {"b_c": -1}, "lower_bound": -10, "upper_bound": 1000},
        ],
        "objective": {"12ppd-R_t": 1},
    });
    let uploaded = parse_model_file(&to_sbml(&model(doc), &SbmlWriteOptions::default()).unwrap(), "probe.xml").unwrap();
    let rxn = uploaded.reactions.get("12ppd-R_t").expect("the punctuated reaction id must survive the upload");
    assert_eq!(parse_gpr(&rxn.gene_reaction_rule), parse_gpr("(b0001 and b0002) or b0003"));
    assert_eq!(uploaded.metabolites["a_c"].formula, "C11H20O6N2PSApo_acp");
    assert_eq!(uploaded.objective_reactions(), [("12ppd-R_t".to_string(), 1.0)]);
    // Pinned: an upload normalises the compartment suffix, so `EX_b(e)` arrives as `EX_b_e`.
    assert!(uploaded.reactions.contains_key("EX_b_e"));
    assert!(!uploaded.reactions.contains_key("EX_b(e)"));
}

#[test]
fn writing_is_idempotent() {
    let once = to_sbml(&read_model("models/methanogen_core_mc.json"), &SbmlWriteOptions::default()).unwrap();
    let opts = SbmlReadOptions { normalise_compartment: false, ..SbmlReadOptions::default() };
    let twice = to_sbml(&Model::from_value(&parse_cobra_sbml(&once, &opts)).unwrap(), &SbmlWriteOptions::default()).unwrap();
    assert_eq!(twice, once);
}

// model-edit.test

fn edit_model() -> Model {
    model(json!({
        "id": "demo", "name": "Demo", "compartments": {"c": "cytosol", "e": "extracellular"},
        "metabolites": [{"id": "a_c", "name": "A", "compartment": "c"}, {"id": "b_c", "name": "B", "compartment": "c"}],
        "reactions": [], "genes": [],
    }))
}

#[test]
fn stoichiometry_accepts_comma_line_syntax_and_json_objects() {
    let pairs = |t: &str| parse_stoichiometry(t).unwrap().into_iter().collect::<Vec<_>>();
    let expected = vec![("a_c".to_string(), -2.0), ("b_c".to_string(), 1.0)];
    assert_eq!(pairs("a_c:-2, b_c:1"), expected);
    assert_eq!(pairs("a_c -1\nb_c 1"), vec![("a_c".to_string(), -1.0), ("b_c".to_string(), 1.0)]);
    assert_eq!(pairs("{\"a_c\":-1,\"b_c\":1}"), vec![("a_c".to_string(), -1.0), ("b_c".to_string(), 1.0)]);
}

#[test]
fn new_metabolite_is_added_to_the_biological_model_and_exported() {
    let mut m = edit_model();
    add_metabolite_to_model(&mut m, &json!({"id": "c_c", "name": "C", "compartment": "c"})).unwrap();
    assert!(m.metabolites.contains_key("c_c"));
    assert!(m.to_json()["metabolites"].as_array().unwrap().iter().any(|x| x["id"] == "c_c"));
}

#[test]
fn new_reaction_validates_stoichiometry_and_exports_bounds_and_gpr() {
    let mut m = edit_model();
    add_reaction_to_model(&mut m, &json!({
        "id": "R_NEW", "name": "New reaction", "metabolites": "a_c:-1, b_c:1",
        "lowerBound": -10, "upperBound": 25, "geneReactionRule": "geneA and (geneB or geneC)",
    }))
    .unwrap();
    let r = &m.reactions["R_NEW"];
    assert_eq!(r.metabolites.iter().map(|(k, v)| (k.as_str(), *v)).collect::<Vec<_>>(), [("a_c", -1.0), ("b_c", 1.0)]);
    assert_eq!(r.lower_bound, -10.0);
    assert_eq!(r.upper_bound, 25.0);
    assert_eq!(r.gene_reaction_rule, "geneA and (geneB or geneC)");
    let exported = m.to_json()["reactions"].as_array().unwrap().iter().find(|x| x["id"] == "R_NEW").unwrap().clone();
    assert_eq!(exported["lower_bound"], -10.0);
    assert_eq!(exported["upper_bound"], 25.0);
    assert_eq!(exported["gene_reaction_rule"], "geneA and (geneB or geneC)");
}

#[test]
fn genes_mentioned_in_a_new_gpr_are_added_to_model_genes() {
    let mut m = edit_model();
    add_reaction_to_model(&mut m, &json!({"id": "R_GPR", "metabolites": {"a_c": -1, "b_c": 1}, "lowerBound": 0, "upperBound": 1000, "gpr": "g1 and (g2 or g3)"})).unwrap();
    assert_eq!(gene_ids_from_gpr("g1 and (g2 or g3)"), ["g1", "g2", "g3"]);
    let ids: Vec<&str> = m.genes.iter().map(|g| g["id"].as_str().unwrap()).collect();
    assert!(ids.contains(&"g1") && ids.contains(&"g2") && ids.contains(&"g3"));
}

#[test]
fn duplicate_ids_bad_compartments_unknown_metabolites_and_bad_bounds_are_rejected() {
    let mut m = edit_model();
    let msg = |r: coxeter_io::Result<()>| r.unwrap_err().message;
    assert!(msg(add_metabolite_to_model(&mut m, &json!({"id": "a_c", "compartment": "c"})).map(|_| ())).contains("already exists"));
    assert!(msg(add_metabolite_to_model(&mut m, &json!({"id": "x_z", "name": "X", "compartment": "z"})).map(|_| ())).contains("Invalid compartment"));
    assert!(msg(add_metabolite_to_model(&mut m, &json!({"id": "x_c", "name": "", "compartment": "c"})).map(|_| ())).contains("metabolite name is required"));
    assert!(msg(add_reaction_to_model(&mut m, &json!({"id": "R_BAD", "metabolites": "missing_c:-1", "lowerBound": 0, "upperBound": 1000})).map(|_| ())).contains("does not exist"));
    assert!(msg(add_reaction_to_model(&mut m, &json!({"id": "R_BAD2", "metabolites": "a_c:-1,b_c:1", "lowerBound": 10, "upperBound": 0})).map(|_| ())).contains("lower bound"));
}

#[test]
fn model_export_contains_user_created_metabolites_and_reactions() {
    // The JS test reads this back through the Store's export; the model's own
    // JSON is what that export writes for a single species.
    let mut m = edit_model();
    add_metabolite_to_model(&mut m, &json!({"id": "c_c", "name": "C", "compartment": "c"})).unwrap();
    add_reaction_to_model(&mut m, &json!({"id": "R_EXPORT", "name": "Exported edit", "metabolites": "a_c:-1,c_c:1", "lowerBound": -5, "upperBound": 10, "gpr": "gx1 or gx2"})).unwrap();
    let doc: Value = serde_json::from_str(&coxeter_core::js::stringify(&m.to_json(), None)).unwrap();
    assert!(doc["metabolites"].as_array().unwrap().iter().any(|x| x["id"] == "c_c"));
    let reaction = doc["reactions"].as_array().unwrap().iter().find(|x| x["id"] == "R_EXPORT").unwrap().clone();
    assert_eq!(reaction["metabolites"], json!({"a_c": -1, "c_c": 1}));
    assert_eq!(reaction["lower_bound"], -5);
    assert_eq!(reaction["upper_bound"], 10);
    assert_eq!(reaction["gene_reaction_rule"], "gx1 or gx2");
}

// mat5.test

const SCIPY_FIXTURE: &str = "TUFUTEFCIDUuMCBNQVQtZmlsZSBQbGF0Zm9ybTogbnQsIENyZWF0ZWQgb246IFRodSBPY3QgIDggMjE6MDI6MjMgMjAyNgAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABSU0PAAAAhgEAAHicnVTNSsNAEJ78VYslehDxmIMPoDdviiDoQQRzEUQkTRYNtEnIj7Q3jz6Cj+DFNxAUvItHH8Gjj+Buukkm09SUTBkyM51v8n2d7ZoA8GwA9PhzlbsCMzNkrsqa8HXu9jRI71jqu2ehx0YJmLz2qFd4leAV5LO6DgOZT7jHkyCRX8GYpWVsQ2WjYRVnKEYhuCiO2X0ZeyxxYz9K/TAANo7SqawL3l+wWLdGeIv+XdSvN+jUUf8Gz44vb5xlcGoNp8LF3nLv02o4DY5Oz3Pc0z+68D676nKu2HU3nJvjHhDOkH2YH9Zl5NjKCu64XvwWYk6fu1V2/76R50GRCx4niEeP8Kf7F3O3y7k770BMzNtH89Za5umy90cTn/k99Ame7k2V/7Q2HH2vKitUv9mCG0j9Rd6kH89rOg+bgM8DwBZ3++MlSFniJ9bnq+UxK4ozNnTmddH9FIb3U/D4hm73kdCH7wt8B9H9NulbIfpEnrDbLPDCHH8I9XOvwGI9BuKH9f0BYC1C2A8AAAAuAAAAeJzjY2BgcABiNiDmAGIeBghghfIZgZgJSoPUFGem56WmIOSBoKEaSAAAWUMEAg8AAAAwAAAAeJzjY2BgcABiNiDmAGJeBghghfIZgZgJSoP4pXnFmel5qSkw9f+BgB1IAwB/PwfxDwAAADMAAAB4nONjYGBwAGI2IOYAYnYGCGCF8hmBmAlKg/hpRYnJJZn5eexQPgPDAXsGBoEDAGmFBbEPAAAANAAAAHic42NgYHAAYjYg5gBiFgYIYIXymaCYEYi5gDg5I7EoMbkktagYok4AqCMxOSkFAHH8Bjs=";

fn concat(items: &[&[u8]]) -> Vec<u8> {
    items.concat()
}

/// `numeric(values, type, little)`: 4 → u16, 5 → i32, 6 → u32, 9 → f64, 12 → i64.
fn numeric(values: &[f64], kind: u32, little: bool) -> Vec<u8> {
    let mut out = Vec::new();
    for v in values {
        let bytes: Vec<u8> = match (kind, little) {
            (4, true) => (*v as u16).to_le_bytes().to_vec(),
            (4, false) => (*v as u16).to_be_bytes().to_vec(),
            (5, true) => (*v as i32).to_le_bytes().to_vec(),
            (5, false) => (*v as i32).to_be_bytes().to_vec(),
            (6, true) => (*v as u32).to_le_bytes().to_vec(),
            (6, false) => (*v as u32).to_be_bytes().to_vec(),
            (9, true) => v.to_le_bytes().to_vec(),
            (9, false) => v.to_be_bytes().to_vec(),
            (12, true) => (*v as i64).to_le_bytes().to_vec(),
            (12, false) => (*v as i64).to_be_bytes().to_vec(),
            _ => unreachable!("type {kind}"),
        };
        out.extend(bytes);
    }
    out
}

fn tag(kind: u32, data: &[u8], little: bool, small: bool) -> Vec<u8> {
    let u16b = |x: u16| if little { x.to_le_bytes() } else { x.to_be_bytes() };
    let u32b = |x: u32| if little { x.to_le_bytes() } else { x.to_be_bytes() };
    if small {
        assert!(!data.is_empty() && data.len() <= 4);
        let mut out = Vec::new();
        out.extend(u16b(kind as u16));
        out.extend(u16b(data.len() as u16));
        out.extend(data);
        out.resize(8, 0);
        return out;
    }
    let pad = if kind == 15 { 0 } else { (8 - data.len() % 8) % 8 };
    let mut out = Vec::new();
    out.extend(u32b(kind));
    out.extend(u32b(data.len() as u32));
    out.extend(data);
    out.extend(vec![0; pad]);
    out
}

struct Matrix<'a> {
    name: &'a str,
    cls: u32,
    shape: Vec<f64>,
    kind: u32,
    values: Vec<f64>,
    little: bool,
    raw: Option<Vec<u8>>,
    flags: Option<u32>,
    extra: Vec<u8>,
    nzmax: f64,
}

impl Default for Matrix<'_> {
    fn default() -> Self {
        Matrix { name: "v", cls: 6, shape: vec![1.0, 2.0], kind: 9, values: vec![1.0, 2.0], little: true, raw: None, flags: None, extra: vec![], nzmax: 0.0 }
    }
}

fn matrix(m: Matrix) -> Vec<u8> {
    let flags = m.flags.unwrap_or(m.cls);
    let data = m.raw.clone().unwrap_or_else(|| numeric(&m.values, m.kind, m.little));
    tag(
        14,
        &concat(&[
            &tag(6, &numeric(&[flags as f64, m.nzmax], 6, m.little), m.little, false),
            &tag(5, &numeric(&m.shape, 5, m.little), m.little, false),
            &tag(1, m.name.as_bytes(), m.little, !m.name.is_empty() && m.name.len() <= 4),
            &tag(m.kind, &data, m.little, false),
            &m.extra,
        ]),
        m.little,
        false,
    )
}

fn file(data: &[u8], little: bool) -> Vec<u8> {
    let mut header = vec![0u8; 128];
    let text = b"MATLAB 5.0 MAT-file, Coxeter synthetic fixture";
    header[..text.len()].copy_from_slice(text);
    let v = if little { 0x100u16.to_le_bytes() } else { 0x100u16.to_be_bytes() };
    header[124..126].copy_from_slice(&v);
    header[126] = if little { 73 } else { 77 };
    header[127] = if little { 77 } else { 73 };
    concat(&[&header, data])
}

fn deflate(bytes: &[u8]) -> Vec<u8> {
    miniz_oxide_compress(bytes)
}

/// `zlib.deflateSync`: a zlib stream.
fn miniz_oxide_compress(bytes: &[u8]) -> Vec<u8> {
    // A stored (uncompressed) zlib stream is still a valid zlib stream; the
    // reader inflates it like any other.
    let mut out = vec![0x78, 0x01];
    let chunks: Vec<&[u8]> = if bytes.is_empty() { vec![&[]] } else { bytes.chunks(65535).collect() };
    for (i, chunk) in chunks.iter().enumerate() {
        out.push(u8::from(i + 1 == chunks.len()));
        let len = chunk.len() as u16;
        out.extend(len.to_le_bytes());
        out.extend((!len).to_le_bytes());
        out.extend(*chunk);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for byte in bytes {
        a = (a + *byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    out.extend(((b << 16) | a).to_be_bytes());
    out
}

fn numbers(v: &MatValue) -> Vec<f64> {
    v.numbers().unwrap().to_vec()
}

#[test]
fn scipy_compressed_synthetic_cobra_structures_preserve_fields_csc_and_numeric_precision() {
    let bytes = base64::engine::general_purpose::STANDARD.decode(SCIPY_FIXTURE).unwrap();
    let vars = read_mat5(&bytes).unwrap();
    // A cell array of two structures (their fields differ, so not a structure array).
    let MatValue::Cell(models) = &vars["SyntheticModels"] else { panic!("expected a cell array") };
    let (MatValue::Struct(model), MatValue::Struct(second)) = (&models[0], &models[1]) else { panic!("expected two structures") };
    assert_eq!(model["rxns"], MatValue::Cell(vec![MatValue::Text("EX_a".into()), MatValue::Text("R1".into()), MatValue::Text("BIO".into())]));
    assert_eq!(model["mets"], MatValue::Cell(vec![MatValue::Text("a[e]".into()), MatValue::Text("a[c]".into())]));
    assert_eq!(model["description"], MatValue::Text("Síntesis μ de prueba".into()));
    let MatValue::Sparse(s) = &model["S"] else { panic!("S is sparse") };
    assert_eq!(s.shape, [2, 3]);
    assert_eq!(s.ir.values, [0.0, 0.0, 1.0, 1.0]);
    assert_eq!(s.jc.values, [0.0, 1.0, 3.0, 4.0]);
    assert_eq!(s.values.values, [-1.0, -1.0, 1.0, -1.0]);
    assert_eq!(numbers(&model["lb"]), [-10.0, 0.0, 0.0]);
    let MatValue::Numeric { shape, data } = &model["lb"] else { panic!() };
    assert_eq!(shape, &[3, 1]);
    assert_eq!(data.kind, TypedArrayKind::Float64);
    let MatValue::Numeric { data: ub, .. } = &model["ub"] else { panic!() };
    assert_eq!(ub.kind, TypedArrayKind::Uint16, "ub is stored as uint16");
    assert_eq!(ub.values, [1000.0, 1000.0, 1000.0]);
    assert_eq!(numbers(&model["b"]), [0.0, 0.0]);
    assert_eq!(numbers(&model["c"]), [0.0, 0.0, 1.0]);
    assert_eq!(numbers(&model["rev"]), [1.0, 0.0, 0.0]);
    assert!(numbers(&model["empty"]).is_empty());
    assert_eq!(second["description"], MatValue::Text("segundo".into()));
    let MatValue::Sparse(s2) = &second["S"] else { panic!() };
    assert_eq!(s2.shape, [0, 0]);
    assert!(s2.values.values.is_empty());
    assert_eq!(numbers(&vars["signed"]), [-2147483648.0, 123.0]);
    assert_eq!(numbers(&vars["unsigned"]), [4294967295.0, 7.0]);
    assert_eq!(numbers(&vars["fraction"]), [1.5, -2.25]);
    assert_eq!(vars["characters"], MatValue::TextRows(vec!["ab".into(), "cd".into()]));
}

#[test]
fn little_and_big_endian_and_compact_names() {
    for little in [true, false] {
        let bytes = file(&matrix(Matrix { little, values: vec![std::f64::consts::PI, -4.5], ..Matrix::default() }), little);
        assert_eq!(numbers(&read_mat5(&bytes).unwrap()["v"]), [std::f64::consts::PI, -4.5]);
        // An offset slice reads the same (the JS test's padded Uint8Array).
        let padded = concat(&[&[0, 0, 0], &bytes, &[0; 7]]);
        assert_eq!(numbers(&read_mat5(&padded[3..padded.len() - 7]).unwrap()["v"]), [std::f64::consts::PI, -4.5]);
    }
}

#[test]
fn utf16_in_either_endian_and_utf32_decode_without_rotating_character_rows() {
    for little in [true, false] {
        let char = matrix(Matrix { little, cls: 4, shape: vec![1.0, 2.0], kind: 17, raw: Some(numeric(&[0x3bc as f64, 0x3b1 as f64], 4, little)), ..Matrix::default() });
        assert_eq!(read_mat5(&file(&char, little)).unwrap()["v"], MatValue::Text("μα".into()));
    }
    let char = matrix(Matrix { cls: 4, shape: vec![1.0, 2.0], kind: 18, raw: Some(numeric(&[0x3bc as f64, 0x1f9ea as f64], 6, true)), ..Matrix::default() });
    assert_eq!(read_mat5(&file(&char, true)).unwrap()["v"], MatValue::Text("μ🧪".into()));
}

#[test]
fn uncompressed_variables_may_follow_compressed_elements_without_alignment_padding() {
    let compressed = tag(15, &deflate(&matrix(Matrix { name: "one", values: vec![3.0, 7.0], ..Matrix::default() })), true, false);
    let doc = read_mat5(&file(&concat(&[&compressed, &matrix(Matrix { name: "two", values: vec![4.0, 9.0], ..Matrix::default() })]), true)).unwrap();
    assert_eq!(numbers(&doc["one"]), [3.0, 7.0]);
    assert_eq!(numbers(&doc["two"]), [4.0, 9.0]);
}

fn mat_error(bytes: &[u8]) -> String {
    read_mat5(bytes).unwrap_err().message
}

#[test]
fn rejects_truncated_headers_invalid_endian_versions_tags_and_bounded_matrix_payloads() {
    assert!(mat_error(&[0u8; 127]).contains("truncated Level 5 header"));
    let mut header = file(&[], true);
    header[126] = 0;
    assert!(mat_error(&header).contains("invalid endian indicator"));
    let mut version = file(&[], true);
    version[124] = 2;
    assert!(mat_error(&version).contains("unsupported MAT-file version"));
    assert!(mat_error(&file(&[0; 4], true)).contains("truncated data-element tag"));
    let clipped = file(&matrix(Matrix::default()), true);
    assert!(mat_error(&clipped[..clipped.len() - 1]).contains("exceeds its container"));
    assert!(mat_error(&file(&tag(8, &[], true, false), true)).contains("unsupported data-element type 8"));
    let extra = tag(9, &numeric(&[1.0], 9, true), true, false);
    assert!(mat_error(&file(&matrix(Matrix { extra, ..Matrix::default() }), true)).contains("unexpected trailing matrix data"));
}

#[test]
fn rejects_invalid_dimensions_data_counts_unsupported_arrays_and_lossy_64bit_integers() {
    let err = |m: Matrix| mat_error(&file(&matrix(m), true));
    assert!(err(Matrix { shape: vec![-1.0, 2.0], ..Matrix::default() }).contains("invalid array dimensions"));
    assert!(err(Matrix { shape: vec![20000001.0, 1.0], ..Matrix::default() }).contains("element limit"));
    assert!(err(Matrix { shape: vec![1.0, 3.0], ..Matrix::default() }).contains("does not match its dimensions"));
    assert!(err(Matrix { flags: Some(6 | 0x800), ..Matrix::default() }).contains("complex arrays"));
    assert!(err(Matrix { cls: 3, ..Matrix::default() }).contains("unsupported MATLAB array class 3"));
    let mut lossy = numeric(&[0.0, 0.0], 12, true);
    lossy[..8].copy_from_slice(&9007199254740992i64.to_le_bytes());
    assert!(err(Matrix { kind: 12, raw: Some(lossy), ..Matrix::default() }).contains("cannot be represented exactly"));
    assert!(err(Matrix { cls: 4, kind: 16, raw: Some(vec![255, 255]), ..Matrix::default() }).contains("invalid UTF-8"));
}

#[test]
fn rejects_invalid_compression_nested_compressed_streams_and_duplicate_variable_names() {
    assert!(mat_error(&file(&tag(15, &[1, 2, 3, 4], true, false), true)).contains("invalid compressed zlib data"));
    let nested = tag(15, &deflate(&tag(15, &deflate(&matrix(Matrix::default())), true, false)), true, false);
    assert!(mat_error(&file(&nested, true)).contains("nested compressed"));
    assert!(mat_error(&file(&concat(&[&matrix(Matrix::default()), &matrix(Matrix::default())]), true)).contains("duplicate variable name"));
}

#[test]
fn validates_sparse_csc_pointers_and_row_bounds_before_exposing_a_matrix() {
    fn sparse(ir: &[f64], jc: &[f64], values: &[f64]) -> Vec<u8> {
        let data = concat(&[&tag(5, &numeric(jc, 5, true), true, false), &tag(9, &numeric(values, 9, true), true, false)]);
        file(
            &matrix(Matrix { cls: 5, shape: vec![2.0, 2.0], kind: 5, raw: Some(numeric(ir, 5, true)), extra: data, nzmax: values.len() as f64, ..Matrix::default() }),
            true,
        )
    }
    let vars = read_mat5(&sparse(&[0.0, 1.0], &[0.0, 1.0, 2.0], &[2.0, -3.0])).unwrap();
    let MatValue::Sparse(value) = &vars["v"] else { panic!() };
    assert_eq!(value.shape, [2, 2]);
    assert_eq!(value.values.values, [2.0, -3.0]);
    assert!(mat_error(&sparse(&[0.0, 1.0], &[1.0, 1.0, 2.0], &[2.0, -3.0])).contains("invalid sparse column pointers"));
    assert!(mat_error(&sparse(&[0.0, 1.0], &[0.0, 2.0, 1.0], &[2.0, -3.0])).contains("not monotone"));
    assert!(mat_error(&sparse(&[0.0, 2.0], &[0.0, 1.0, 2.0], &[2.0, -3.0])).contains("row index exceeds"));
    assert!(mat_error(&sparse(&[0.0], &[0.0, 1.0, 2.0], &[2.0, -3.0])).contains("exceed allocated storage"));
}

// provenance.test

#[test]
fn cobra_annotations_survive_model_round_trip() {
    let raw = json!({
        "id": "p", "compartments": {"c": "c"}, "genes": [{"id": "g1", "annotation": {"uniprot": "P12345"}}],
        "metabolites": [{"id": "a_c", "compartment": "c", "annotation": {"chebi": "CHEBI:15377"}, "notes": {"source": "curated"}}],
        "reactions": [{"id": "R", "metabolites": {"a_c": -1}, "annotation": {"kegg.reaction": "R00001"}, "notes": {"pmid": "123"}}],
    });
    let m = parse_model_json(&raw.to_string(), "p.json").unwrap();
    let json = m.to_json();
    assert_eq!(m.metabolites["a_c"].annotation["chebi"], "CHEBI:15377");
    assert_eq!(m.reactions["R"].annotation["kegg.reaction"], "R00001");
    assert_eq!(json["metabolites"][0]["annotation"]["chebi"], "CHEBI:15377");
}

#[test]
fn sbml_provenance_is_imported_for_species_reaction_and_gene_product() {
    let xml = r#"<sbml level="3" version="1" xmlns:fbc="x"><model id="m"><listOfCompartments><compartment id="c"/></listOfCompartments>
 <listOfSpecies><species id="a_c" compartment="c"><annotation><rdf:li rdf:resource="https://identifiers.org/chebi:15377"/></annotation></species></listOfSpecies>
 <fbc:listOfGeneProducts><fbc:geneProduct fbc:id="g1"><annotation><rdf:li rdf:resource="https://identifiers.org/uniprot:P12345"/></annotation></fbc:geneProduct></fbc:listOfGeneProducts>
 <listOfReactions><reaction id="R" reversible="false"><listOfReactants><speciesReference species="a_c"/></listOfReactants><annotation><rdf:li rdf:resource="https://identifiers.org/rhea:10000"/></annotation></reaction></listOfReactions>
 </model></sbml>"#;
    let m = parse_model_sbml(xml, "p.xml").unwrap();
    assert_eq!(m.metabolites["a_c"].annotation["chebi"], json!(["15377"]));
    assert_eq!(m.reactions["R"].annotation["rhea"], json!(["10000"]));
    assert_eq!(m.genes[0]["annotation"]["uniprot"], json!(["P12345"]));
}

// gut9.test: xlsx reader

/// A workbook built here, so the reader is tested on something whose every
/// value is known. Entries are stored rather than deflated.
fn mini_workbook(sheet_xml: &str, shared_strings: &[&str]) -> Vec<u8> {
    let shared = format!("<sst>{}</sst>", shared_strings.iter().map(|s| format!("<si><t>{s}</t></si>")).collect::<String>());
    let sheet = format!("<worksheet><sheetData>{sheet_xml}</sheetData></worksheet>");
    let files: Vec<(&str, String)> = vec![
        ("xl/workbook.xml", r#"<workbook><sheets><sheet name="data" sheetId="1" r:id="rId1"/></sheets></workbook>"#.into()),
        ("xl/_rels/workbook.xml.rels", r#"<Relationships><Relationship Id="rId1" Type="x/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#.into()),
        ("xl/sharedStrings.xml", shared),
        ("xl/worksheets/sheet1.xml", sheet),
    ];
    let (mut body, mut directory) = (Vec::new(), Vec::new());
    for (name, text) in &files {
        let offset = body.len() as u32;
        let mut local = vec![0u8; 30];
        local[..4].copy_from_slice(&0x04034b50u32.to_le_bytes());
        local[18..22].copy_from_slice(&(text.len() as u32).to_le_bytes());
        local[22..26].copy_from_slice(&(text.len() as u32).to_le_bytes());
        local[26..28].copy_from_slice(&(name.len() as u16).to_le_bytes());
        body.extend(local);
        body.extend(name.as_bytes());
        body.extend(text.as_bytes());
        let mut dir = vec![0u8; 46];
        dir[..4].copy_from_slice(&0x02014b50u32.to_le_bytes());
        dir[20..24].copy_from_slice(&(text.len() as u32).to_le_bytes());
        dir[24..28].copy_from_slice(&(text.len() as u32).to_le_bytes());
        dir[28..30].copy_from_slice(&(name.len() as u16).to_le_bytes());
        dir[42..46].copy_from_slice(&offset.to_le_bytes());
        directory.extend(dir);
        directory.extend(name.as_bytes());
    }
    let mut eocd = vec![0u8; 22];
    eocd[..4].copy_from_slice(&0x06054b50u32.to_le_bytes());
    eocd[8..10].copy_from_slice(&(files.len() as u16).to_le_bytes());
    eocd[10..12].copy_from_slice(&(files.len() as u16).to_le_bytes());
    eocd[12..16].copy_from_slice(&(directory.len() as u32).to_le_bytes());
    eocd[16..20].copy_from_slice(&(body.len() as u32).to_le_bytes());
    concat(&[&body, &directory, &eocd])
}

fn rows_of(bytes: Vec<u8>, sheet: &str) -> Vec<Value> {
    read_workbook(bytes).unwrap().rows(sheet).unwrap().map(|r| r.unwrap().to_json()).collect()
}

#[test]
fn the_xlsx_reader_keys_cells_by_column_not_by_position() {
    let xml = concat!(
        r#"<row r="1"><c r="A1" t="s"><v>0</v></c><c r="D1" t="s"><v>1</v></c></row>"#,
        r#"<row r="2"><c r="A2" t="s"><v>2</v></c><c r="D2" t="s"><v>3</v></c></row>"#
    );
    let rows = rows_of(mini_workbook(xml, &["head", "tail", "left", "right"]), "data");
    assert_eq!(rows[0], json!({"__row": 1.0, "A": "head", "D": "tail"}));
    assert_eq!(rows[1], json!({"__row": 2.0, "A": "left", "D": "right"}));
    assert!(rows[1].get("B").is_none(), "a missing cell must stay missing");
}

#[test]
fn the_xlsx_reader_handles_numbers_inline_strings_and_entities() {
    let xml = concat!(
        r#"<row r="1"><c r="A1"><v>42.5</v></c>"#,
        r#"<c r="B1" t="inlineStr"><is><t>in</t><t>line</t></is></c>"#,
        r#"<c r="C1" t="s"><v>0</v></c><c r="D1"/></row>"#
    );
    let rows = rows_of(mini_workbook(xml, &["shared &amp; escaped"]), "data");
    assert_eq!(rows[0]["A"], "42.5");
    assert_eq!(rows[0]["B"], "inline", "text runs are joined");
    assert_eq!(rows[0]["C"], "shared & escaped", "entities are decoded");
    assert!(rows[0].get("D").is_none(), "an empty cell yields nothing");
}

#[test]
fn the_xlsx_reader_reports_a_missing_sheet_by_name() {
    let wb = read_workbook(mini_workbook(r#"<row r="1"><c r="A1"><v>1</v></c></row>"#, &[])).unwrap();
    assert_eq!(wb.sheet_names(), ["data"]);
    assert!(wb.rows("nope").unwrap_err().message.contains("no sheet named \"nope\""));
}

// label-sources.test / models.test

#[test]
fn model_vectors_and_database_annotations_survive_an_upload_and_round_trip() {
    let doc = json!({
        "id": "labels", "compartments": {"c": "Cytosol"},
        "metabolites": [
            {"id": "a_c", "name": "Original A", "compartment": "c", "annotation": {"bigg.metabolite": ["a"], "kegg.compound": ["C00001"], "metanetx.chemical": ["MNXM1"]}},
            {"id": "b_c", "name": "Original B", "compartment": "c", "annotation": {"bigg.metabolite": ["b"], "metacyc.compound": ["B-META"], "seed.compound": ["cpd00002"]}},
        ],
        "metNames": ["Alternative A", "Alternative B"], "metModelSEEDID": ["cpd00001", "cpd00002"], "metFormulas": ["H2O", "CO2"],
        "reactions": [{"id": "R", "name": "Original reaction", "metabolites": {"a_c": -1, "b_c": 1}, "lower_bound": 0, "upper_bound": 1000,
            "annotation": {"bigg.reaction": ["RXN-A"], "kegg.reaction": ["R00001"], "kegg.pathway": ["path:map00010"]}}],
        "rxnNames": ["Alternative reaction"], "rxnMetaNetXID": ["MNXR1"], "rxnMetaCycID": ["RXN-META"], "rxnModelSEEDID": ["rxn00001"],
    });
    let m = parse_model_json(&doc.to_string(), "labels.json").unwrap();
    let restored = parse_model_json(&m.to_json().to_string(), "labels.json").unwrap();
    assert_eq!(restored.metabolites["a_c"].label_sources["name"], ["Alternative A"]);
    assert_eq!(restored.metabolites["a_c"].label_sources["modelseed"], ["cpd00001"]);
    assert_eq!(restored.reactions["R"].label_sources["name"], ["Alternative reaction"]);
    assert_eq!(m.reactions["R"].label_sources["metacyc"], ["RXN-META"]);
    assert_eq!(m.metabolites["a_c"].label_sources["kegg"], ["C00001"]);
    assert!(!m.reactions["R"].label_sources["kegg"].contains(&"path:map00010".to_string()));
    assert!(!m.metabolites["a_c"].label_sources.contains_key("metFormulas"));
}

#[test]
fn the_catalogue_is_not_empty_and_every_file_in_it_is_a_model() {
    let files = catalogue();
    assert!(files.len() >= 5, "found {} models", files.len());
    for f in &files {
        let text = std::fs::read_to_string(repo(&format!("models/{f}"))).unwrap();
        let m = parse_model_file(&text, f).unwrap_or_else(|e| panic!("{f}: {e}"));
        let tree = m.compartment_tree();
        assert!(!tree.roots.is_empty());
        // Every metabolite lives in a declared compartment.
        for met in m.metabolites.values() {
            assert!(tree.by_id.contains_key(&met.compartment), "{} in undeclared {}", met.id, met.compartment);
        }
    }
}
