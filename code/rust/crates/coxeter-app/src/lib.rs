//! Coxeter Multicell Flux Visualizer: the native application, on eframe.
//!
//! It replaces the interface half of `src/ui/` of the JavaScript app (see
//! `DESIGN.md`): `coxeter-state` holds the Store, `coxeter-canvas` the map
//! widget, and this crate everything around them.
//!
//! | Rust module | JavaScript |
//! |---|---|
//! | [`app`] | `src/ui/app.js`: boot, the store subscription, the window keys, `debounce`, `scheduleSolve`, `paintStatus` |
//! | [`actions`] | `src/ui/app.js`: `openWorkspace`, `activateTool`, `setTheme`, the camera, imports and exports |
//! | [`panels`] | `index.html`, `src/ui/panels.js`, the menu controllers |
//! | [`host`] | the global `store` the gestures of `app.js` call |
//! | [`ui_state`] | the `ui` object of `app.js` and the classes of `.app` |
//! | [`files`] | the file inputs of `index.html` and `download()` |
//! | [`theme`], [`fonts`] | `assets/app.css` |
//! | [`widgets`] | the form controls of `app.css`, with `input` and `change` |
//! | [`assets`] | `fetch(…/models/…)` and the standalone's embedded data |

// The workspace declares Rust 1.85: let-chains (1.88), which this lint asks
// for, are not available to it.
#![allow(clippy::collapsible_if)]

pub mod actions;
pub mod app;
pub mod assets;
pub mod files;
pub mod fonts;
pub mod host;
pub mod panels;
pub mod solvers;
pub mod theme;
pub mod ui_state;
pub mod widgets;

pub use app::Shell;
