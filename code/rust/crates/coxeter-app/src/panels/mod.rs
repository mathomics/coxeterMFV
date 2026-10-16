//! Everything around the map: the layout of `index.html` and
//! `assets/app.css`, and what `src/ui/panels.js` renders, with the handlers
//! of `onSidebarClick` / `onSidebarInput` (`src/ui/app.js`) beside the
//! controls they serve.
//!
//! | Rust module | JavaScript |
//! |---|---|
//! | [`topbar`] | `.topbar` of index.html, `paintShell`, `paintStatus`, `paintSaveState`, the File and Simulate menus |
//! | [`left`] | `.left-panel`: tabs, tool grid, `renderTaskPanel`, the panel footer |
//! | [`toolbar`] | `.canvas-toolbar`, `renderViewControls` |
//! | [`results`] | `.results-panel`: `renderResults`, `renderLogEntries`, `paintRunSummary` |
//! | [`right`] | `.inspector-panel`: its tabs and collapse |
//! | [`inspector`] | `renderInspector`, `renderLayoutSettings`, the inspectors, `reactionDrawingControls`, `mapLabelControl` |
//! | [`workspace`] | `.workspace-editor`, `renderWorkspace`, `getWorkspaceTitle` |
//! | [`scenario`] | `scenarioCard`, `objectiveControls`, published communities, `steadyComBlock`, the sweep figure, `zunigaCaseBlock` |
//! | [`model`] | `speciesCard`, `mediumCard`, `modelEditorCard`, `modelOverviewCard`, `communityOmicsCard`, `omicsOverlayCard` |
//! | [`map_options`] | `displayCard`, `mapAppearanceCard`, `fluxColorEditor`, `legendCard`, `pathwayManagerCard`, `mapStructureCard`, `mapCanvasCard`, `reactionSearchCard` |
//! | [`cross_feeding`] | `crossFeedingCard`, `crossFeedingTopControl`, `crossFeedingMetabolitePicker`, `renderCrossFeedingEvents` |
//! | [`menus`] | `metabolite-menu.js`, `compartment-menu.js`, `boundary-menu.js`, `metabolite-add-menu.js` |
//! | [`add_reaction`] | `add-reaction-panel.js` |

pub mod add_reaction;
pub mod cross_feeding;
pub mod inspector;
pub mod left;
pub mod map_options;
pub mod menus;
pub mod model;
pub mod results;
pub mod right;
pub mod scenario;
pub mod toolbar;
pub mod topbar;
pub mod workspace;

/// A card of a workspace page: what `panels.js` renders as one `<section class="card">`.
pub type CardFn = fn(&mut egui::Ui, &mut crate::app::Shell);
