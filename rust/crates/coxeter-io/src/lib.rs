//! Models in and out: SBML, COBRA JSON, MATLAB v5 and Excel readers, the SBML
//! writer, model editing, provenance links and reproducibility bundles.
//!
//! Ported from the JavaScript application, which stays the specification:
//!
//! | Rust module | JavaScript |
//! |---|---|
//! | [`model_import`] | `src/core/model-import.js` |
//! | [`sbml`] | `src/core/sbml.js` |
//! | [`sbml_write`] | `src/core/sbml-write.js` |
//! | [`model_edit`] | `src/core/model-edit.js` |
//! | [`provenance`] | `src/core/provenance.js` |
//! | [`reproducibility`] | `src/core/reproducibility.js` |
//! | [`xlsx`] | `src/core/xlsx.js` |
//! | [`mat5`] | `src/import/mat5.js` |
//! | [`file_names`] | `src/ui/file-names.js` |
//!
//! Every reader returns the same model, byte for byte once serialised, as the
//! JS reader given the same file. The parity tests under `tests/` prove it on
//! the bundled and published models and on the user's genome-scale files.
//!
//! A GUI typically needs only the re-exports below:
//!
//! ```no_run
//! use coxeter_io::{ImportFile, prepare_imported_models, to_sbml, SbmlWriteOptions};
//!
//! let text = std::fs::read_to_string("yeast-GEM.xml").unwrap();
//! let prepared = prepare_imported_models(&[ImportFile { name: "yeast-GEM.xml".into(), text }]).unwrap();
//! let model = &prepared.entries[0].model;
//! let xml = to_sbml(model, &SbmlWriteOptions::default()).unwrap();
//! ```

// The workspace declares Rust 1.85: let-chains (1.88) and `is_multiple_of`
// (1.87), which these lints ask for, are not available to it.
#![allow(clippy::collapsible_if, clippy::manual_is_multiple_of)]

pub mod error;
pub mod file_names;
pub mod gene_rules;
mod jscompat;
pub mod mat5;
pub mod model_edit;
pub mod model_import;
pub mod provenance;
pub mod reproducibility;
pub mod sbml;
pub mod sbml_write;
pub mod xlsx;

pub use error::{Error, ErrorKind, Result};

/// Read one user file (COBRA JSON or SBML, told apart by name and content).
pub use model_import::parse_model_file;
/// Read a COBRA JSON model.
pub use model_import::parse_model_json;
/// Read an SBML Level 3 + FBC model (the upload path).
pub use model_import::parse_model_sbml;
/// Read several uploads at once into distinct species with an inferred medium;
/// MATLAB and Excel files come as bytes.
pub use model_import::{
    ImportFile, ImportSummary, ImportedEntry, ModelFileKind, PreparedImport, UploadFile, infer_community_medium, is_binary_model_file,
    model_file_kind, parse_model_mat, parse_model_upload, parse_model_xlsx, prepare_imported_models, prepare_model_uploads,
};

/// Read a published COBRA SBML file (Level 2 or 3) into a COBRA JSON document.
pub use sbml::{SbmlReadOptions, UnmangleOptions, load_sbml_model, parse_cobra_sbml, unmangle_id};

/// Write a model as SBML L3V1 + fbc-v2.
pub use sbml_write::{Gpr, GprOp, SbmlWriteOptions, genes_in_gpr, mangle_id, parse_gpr, to_sbml};

/// Add metabolites and reactions to a model from the editor's forms.
pub use model_edit::{MetaboliteSpec, ReactionSpec, add_metabolite_to_model, add_reaction_to_model, gene_ids_from_gpr, parse_stoichiometry};

/// Database links and identifiers.org annotations.
pub use provenance::{AnnotationLink, annotation_links, extract_identifiers_org, normalize_annotations};

/// Reproducibility bundles and the methods report.
pub use reproducibility::{
    BundleContext, ReplayComparison, ReproStore, Tolerances, analysis_provenance, canonical_json, compare_replayed_solution,
    create_reproducibility_bundle, create_reproducibility_bundle_with, methods_provenance_report, sha256, software_identity,
    verify_reproducibility_bundle,
};

/// Excel workbooks.
pub use xlsx::{Row, Rows, Workbook, read_workbook, read_workbook_file};

/// MATLAB Level 5 MAT files.
pub use mat5::{MatValue, MatVariables, read_mat5, read_mat5_file};

/// Export file names.
pub use file_names::{
    community_data_file_name, export_file_name, file_stem, member_model_file_name, timestamped_file_name,
};
