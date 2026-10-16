//! The biology every other Coxeter crate reads: metabolic models, the
//! multi-species community built from them, the compartment registry and the
//! sources a label can be drawn from.
//!
//! Ported from `src/core/` of the JavaScript application. [`js`] holds the
//! handful of JavaScript behaviours (rounding, number printing, collation)
//! the port has to reproduce for maps and exports to come out the same.

pub mod boundaries;
pub mod colors;
pub mod community;
pub mod compartments;
pub mod geometry;
pub mod js;
pub mod label_sources;
pub mod mapdata;
pub mod metabolite_label;
pub mod model;
pub mod testkit;
pub mod text_style;

pub use community::{CommunityModel, CommunitySpec, Objective, SpeciesSpec};
pub use model::{Metabolite, Model, ModelError, Reaction, ReactionKind};
