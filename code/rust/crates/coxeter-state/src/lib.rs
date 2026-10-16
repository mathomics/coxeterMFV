//! The application's state and every action it can take: a port of `Store`
//! in `src/ui/store.js`, with no GUI. It never draws and never opens a
//! dialog (see `../coxeter-app/DESIGN.md`).
//!
//! ```no_run
//! use coxeter_state::Store;
//! let mut store = Store::with_dir(".");          // the folder holding models/
//! store.init().unwrap();
//! store.use_scenario("aerobio-mixto").unwrap();
//! for reason in store.take_events() { /* rebuild what `reason` touched */ }
//! store.run_analysis("fva").unwrap();             // background work…
//! let finished = store.poll_jobs();               // …applied once per frame
//! let state = store.map_render_state(Default::default());
//! ```
//!
//! | Rust module | JavaScript |
//! |---|---|
//! | [`store`] | `Store`: fields, construction, events, status, log |
//! | [`scenarios`] | `useScenario`, uploads, Yeast9, published communities, `rebuild` |
//! | [`simulation`] | `solve`, `runAnalysis`, pFBA, loopless, FVA, bounds, medium |
//! | [`steadycom`] | the SteadyCom section |
//! | [`map_edit`] | every map edit, selection, labels, layouts |
//! | [`pathways`] | the pathway manager and reaction colouring |
//! | [`documents`] | map documents, scenario state, models, bundles |
//! | [`view`] | `setView`, the flux scale |
//! | [`work`] | saved / unsaved work |
//! | [`omics`], [`cross_feeding`], [`zuniga`] | omics overlays and GIMME, the cross-feeding explorer, the Zuñiga case (on `coxeter-analysis`) |
//! | [`model_editor`] | the biological model editor |
//! | [`render_state`] | `mapRenderState()` of `src/ui/app.js` |
//! | [`status_line`] | `src/ui/status-line.js` |
//! | [`operation_boundary`] | `src/ui/operation-boundary.js` |
//! | [`jobs`] | Workers and timers, as background threads |
//! | [`assets`] | `fetch`, as files and an injectable fetcher |

// The workspace declares Rust 1.85: let-chains (1.88), which this lint asks
// for, are not available to it.
#![allow(clippy::collapsible_if)]

pub mod assets;
pub mod cross_feeding;
pub mod documents;
pub mod error;
pub mod jobs;
pub mod map_edit;
pub mod model_editor;
pub mod omics;
pub mod operation_boundary;
pub mod pathways;
pub mod render_state;
pub mod scenarios;
pub mod simulation;
pub mod status_line;
pub mod steadycom;
pub mod store;
pub mod types;
pub mod util;
pub mod view;
pub mod work;
pub mod zuniga;

pub use assets::{Assets, CachedFetcher, DirAssets, EmbeddedAssets, FetchResponse, Fetcher, MemoryAssets, NoFetcher};
pub use error::{Result, StoreError};
pub use jobs::{JobInfo, JobKind, JobOutcome};
pub use render_state::RenderOptions;
pub use steadycom::{SteadyComRequest, SteadyComRunner, SteadyComTask, SteadyComTaskResult, TaskControls};
pub use store::Store;
pub use types::*;
