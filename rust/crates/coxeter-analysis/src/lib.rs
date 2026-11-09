//! Published communities, omics overlays, GIMME and the cross-feeding explorer.
//!
//! Ported from the JavaScript application, which stays the specification:
//!
//! | Rust module | JavaScript |
//! |---|---|
//! | [`overlay`] | `src/omics/overlay.js` |
//! | [`community_omics`] | `src/omics/community-omics.js` |
//! | [`gimme`] | `src/omics/gimme.js` |
//! | [`cross_feeding_explorer`] | `src/analysis/cross-feeding-explorer.js` |
//! | [`published`] | `src/communities/published.js` |
//! | [`stolyar`] | `src/communities/stolyar.js` |
//! | [`steadycom_ecoli`] | `src/communities/steadycom-ecoli.js` |
//! | [`gut9`] | `src/communities/gut9.js` |
//! | [`zuniga`] | `src/communities/zuniga.js` |
//!
//! Not here, because other crates port them: `gut9-figure4.js` and
//! `steadycom-runtime.js` (`coxeter_solver::gut9_figure4`,
//! `coxeter_solver::steadycom_runtime`), `src/core/steadycom.js`
//! (`coxeter_solver::steadycom`). The SteadyCom client and worker become
//! threads in the app.
//!
//! [`community_view::CommunityView`] is what the explorer and the omics
//! mapper read from a community. In the JS that is whatever object the active
//! scenario built. Here it is implemented by `coxeter_core::CommunityModel`
//! and by the Zuñiga case's [`zuniga::ZunigaCommunity`].
//!
//! A GUI typically needs only the re-exports below:
//!
//! ```no_run
//! use coxeter_analysis::{OmicsSpecies, parse_metatranscriptomics, reaction_omics_scores, run_gimme, GimmeOptions};
//! # fn demo(community: &coxeter_core::CommunityModel,
//! #         models: &indexmap::IndexMap<String, std::sync::Arc<coxeter_core::Model>>,
//! #         problem: &coxeter_solver::fba::FluxProblem) -> coxeter_analysis::Result<()> {
//! let parsed = parse_metatranscriptomics("MAG,gene,condition,TPM\nEc,b0001,A,4\n", None)?;
//! let species = [OmicsSpecies::new("Ec", "ecoli_core_mc")];
//! let scores = reaction_omics_scores(community, &species, models, Some(&parsed.dataset), "A");
//! let gimme = run_gimme(problem, &scores, &GimmeOptions::default())?;
//! # Ok(()) }
//! ```

// The workspace declares Rust 1.85: let-chains (1.88), which this lint asks
// for, are not available to it.
#![allow(clippy::collapsible_if)]

pub mod community_omics;
pub mod community_view;
pub mod cross_feeding_explorer;
pub mod error;
pub mod gimme;
pub(crate) mod jsutil;
pub mod overlay;
pub mod published;
pub mod steadycom_ecoli;
pub mod stolyar;
pub mod gut9;
pub mod zuniga;

pub use error::{Error, ErrorKind, Result};

/// What the analysis functions read from a community.
pub use community_view::{CommunityView, TransferRef};

/// Omics tables, reaction scores, comparisons and their colour scales.
pub use overlay::{
    CompareMetric, CompareOptions, GprNode, OmicsCsvOptions, OmicsDataset, OmicsRow, OmicsScale, OmicsSpecies, ScaleKind,
    compare_reaction_scores, deserialize_omics_dataset, diverging_omics_colors, evaluate_gpr, omics_key, parse_gpr,
    parse_omics_csv, reaction_omics_scores, scores_to_json, sequential_omics_colors, serialize_omics_dataset,
};

/// Community metatranscriptomics and its mapping report.
pub use community_omics::{MappingReport, Metatranscriptomics, SpeciesMapping, TableFormat, community_omics_mapping_report, parse_metatranscriptomics};

/// GIMME on a flux problem.
pub use gimme::{DirectionalReducedCost, GimmeOptions, GimmeResult, run_gimme};

/// The cross-feeding explorer.
pub use cross_feeding_explorer::{
    CROSS_FEEDING_GROUPS, CrossFeedingEvent, CrossFeedingFilter, CrossFeedingIndex, CrossFeedingOptions, CrossFeedingPair,
    ExchangedMetabolite, build_cross_feeding_index, exchanged_metabolites, filter_cross_feeding_index,
};

/// The registry of published communities.
pub use published::{
    FRAMEWORKS, PUBLISHED_COMMUNITIES, PublishedCommunity, ReproductionStatus, blocked_communities, find_community,
    published_communities, reproduced_communities,
};

/// The Stolyar 2007 joint model (CellNetAnalyzer files).
pub use stolyar::{StolyarFiles, StolyarModel, import_stolyar, to_joint_cobra_json, to_species_cobra_json, validate_stolyar};

/// The four-auxotroph E. coli community (SteadyCom tutorial).
pub use steadycom_ecoli::{EcoliCommunity, build_community};

/// The nine-species gut community (model9.xlsx).
pub use gut9::{Gut9Community, Gut9ImportOptions, Gut9SpecOptions, import_gut9, to_steady_com_spec};

/// The Zuñiga 2020 ΔpheA rescue.
pub use zuniga::{
    DirectoryCache, FetchResponse, Fetcher, SourceCache, ZUNIGA_SCENARIOS, ZunigaCommunity, ZunigaLoadOptions, ZunigaPrepared,
    ZunigaSolution, ZunigaStudy, create_zuniga_scenario, load_zuniga_study, restore_zuniga_scenario, solve_zuniga_scenario,
};
#[cfg(feature = "download")]
pub use zuniga::HttpsFetcher;
