//! The editable map: automatic layout, reaction placement, editing, history,
//! reconciliation and the saved map document.
//!
//! Ported from `src/layout/` and `src/map/` of the JavaScript application.
//! [`EditableMap`] is the entry point: it owns the live
//! [`MapGraph`](coxeter_core::mapdata::MapGraph) and every editing operation
//! a user can make on it.

pub mod circularize;
pub mod compartment_reactions;
pub mod core_metabolism_overview;
pub mod drag;
pub mod editable_map;
pub mod force;
pub mod geometry_edit;
pub mod history;
pub mod layout;
pub mod layout_settings;
pub mod map_document;
pub mod multi_selection;
pub mod place_reaction;
pub mod reaction_edit;
pub mod reconcile_map;
pub mod util;
pub mod yeast9;

pub use editable_map::EditableMap;
pub use layout::{Contents, LayoutOptions, layout_community};
pub use multi_selection::Selection;
pub use place_reaction::PlaceOptions;
