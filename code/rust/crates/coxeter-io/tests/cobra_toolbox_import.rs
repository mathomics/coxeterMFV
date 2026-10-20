//! Port of `test/cobra-toolbox-import.test.mjs`: one five-reaction model in
//! the COBRA Toolbox layouts (test/fixtures/cobra-toolbox, written by its
//! make.py) reads the same from .mat and .xlsx.

use coxeter_core::model::Model;
use coxeter_core::testkit::repo_root;
use coxeter_io::model_import::{UploadFile, parse_model_upload, prepare_model_uploads};

fn fixture(name: &str) -> UploadFile {
    let bytes = std::fs::read(repo_root().join("test/fixtures/cobra-toolbox").join(name)).expect("fixture");
    UploadFile { name: name.into(), bytes }
}

fn assert_mini_model(model: &Model) {
    let keys = |it: Vec<&String>| it.into_iter().cloned().collect::<Vec<_>>();
    assert_eq!(keys(model.reactions.keys().collect()), ["EX_glc__D_e", "GLCt", "HEX1", "ATPS", "BIOMASS"]);
    // The [c] suffixes are written as the Toolbox SBML export writes them.
    assert_eq!(keys(model.metabolites.keys().collect()), ["glc__D_e", "glc__D_c", "g6p_c", "atp_c", "adp_c"]);
    let atps = &model.reactions["ATPS"];
    assert_eq!((atps.metabolites["adp_c"], atps.metabolites["atp_c"]), (-2.0, 2.0));
    assert_eq!(model.reactions["EX_glc__D_e"].lower_bound, -10.0);
    assert_eq!(model.reactions["GLCt"].gene_reaction_rule, "(g1 and g2) or g3");
    assert_eq!(model.reactions["HEX1"].subsystem, "Glycolysis");
    assert_eq!(model.reactions["HEX1"].label_sources["ec"], ["2.7.1.1"]);
    assert_eq!(model.biomass_reaction_id().as_deref(), Some("BIOMASS"));
    let g6p = &model.metabolites["g6p_c"];
    assert_eq!((g6p.compartment.as_str(), g6p.base.as_str(), g6p.name.as_str(), g6p.formula.as_str(), g6p.charge), ("c", "g6p", "D-Glucose 6-phosphate", "C6H11O9P", -2.0));
    assert_eq!(g6p.label_sources["kegg"], ["C00092"]);
    assert_eq!(model.compartment_names["e"], "Extracellular");
    assert_eq!(model.genes.iter().map(|g| g["id"].as_str().unwrap()).collect::<Vec<_>>(), ["g1", "g2", "g3", "g4", "g5", "g6"]);
}

#[test]
fn a_cobra_toolbox_mat_model_imports_and_a_v7_3_file_is_refused_with_a_way_out() {
    let prepared = prepare_model_uploads(&[fixture("mini.mat")]).unwrap();
    let entry = &prepared.entries[0];
    assert_mini_model(&entry.model);
    assert_eq!(entry.model.name, "Mini COBRA Toolbox model");
    assert_eq!(entry.model_key, "upload:mini");
    assert_eq!(prepared.medium.iter().map(|(k, v)| (k.as_str(), *v)).collect::<Vec<_>>(), [("glc__D", 10.0)]);

    let mut hdf5 = vec![0u8; 1024];
    let header = b"MATLAB 7.3 MAT-file, Platform: GLNXA64, HDF5 schema 1.00";
    hdf5[..header.len()].copy_from_slice(header);
    hdf5[512..520].copy_from_slice(&[0x89, 0x48, 0x44, 0x46, 0x0d, 0x0a, 0x1a, 0x0a]);
    let error = parse_model_upload(&UploadFile { name: "big.mat".into(), bytes: hdf5 }, 0).unwrap_err();
    assert!(error.message.starts_with("big.mat: this is a MATLAB v7.3 (HDF5) file"), "{}", error.message);
    assert!(error.message.contains("-v7"));
}

#[test]
fn a_cobra_toolbox_excel_workbook_imports_with_the_same_model() {
    let model = parse_model_upload(&fixture("mini.xlsx"), 0).unwrap();
    assert_mini_model(&model);
    // The Compartment column names the bracketed id.
    assert_eq!(model.compartment_names["c"], "Cytosol");
}
