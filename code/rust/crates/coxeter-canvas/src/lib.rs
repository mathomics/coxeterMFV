//! The map widget: painting, camera, hit-testing and gestures of
//! `src/ui/app.js` (with `src/ui/camera-zoom.js` and
//! `src/ui/wheel-navigation.js`), on egui.
//!
//! - [`scene`]: the SVG element tree of `coxeter_render::render_map`, styled by
//!   `assets/map.css` ([`style`]), tessellated once into meshes in map
//!   coordinates ([`tess`], [`geom`], [`text`]).
//! - [`painter`]: draws the scene through the camera ([`camera`], [`wheel`]).
//! - [`hit`]: what is under the pointer, in the DOM's order; [`targets`]:
//!   the `*At(target)` functions that read it (`closest()` over `data-*`).
//! - [`gestures`]: the stage's pointer plumbing and every gesture of
//!   `app.js`, acting on a [`MapHost`]; [`marks`]: what a drag marks on the
//!   drawing.
//! - [`canvas`]: [`MapCanvas`], the stage without a window (camera,
//!   drawing, gestures) fed with plain events (what the tests drive).
//! - [`widget`]: [`MapView`], the egui widget: input, painting, overlays,
//!   the in-place text editor; [`settle`]: the sharp raster of the view it
//!   shows once the camera is still.
//! - [`test_host`]: a [`MapHost`] over a bare `EditableMap` and
//!   `MapHistory`, with the store's semantics.

// `!(x > 0.0)` is how the JavaScript's `!(x > 0)` reads NaN as false; the
// ported tests keep that form on purpose.
#![allow(clippy::neg_cmp_op_on_partial_ord)]
// The font book's caches are keyed by what makes a glyph mesh.
#![allow(clippy::type_complexity)]

pub mod camera;
pub mod canvas;
pub mod doc;
pub mod geom;
pub mod gestures;
pub mod hit;
pub mod host;
pub mod marks;
pub mod painter;
pub mod scene;
pub mod settle;
pub mod style;
pub mod targets;
pub mod tess;
pub mod test_host;
pub mod text;
pub mod wheel;
pub mod widget;

pub use canvas::MapCanvas;
pub use gestures::{CanvasEvent, MapMenu, MenuRequest, Tool};
pub use host::MapHost;
pub use widget::{MapView, MapViewOutput};
