//! Drawing a map: the SVG the JavaScript renderer draws, its styles, flux
//! colours and cross-feeding routes; SVG and PNG export.
//!
//! Ported from `src/render/` of the JavaScript application, with the export
//! half of `src/ui/app.js`:
//!
//! - [`render_map`] draws a [`MapGraph`](coxeter_core::mapdata::MapGraph) and a
//!   [`RenderState`] as an [`Element`] tree; [`render_map_svg`] writes it as
//!   the exact string the JavaScript `renderMap` returns.
//! - [`theme`] holds the flux scale ([`FluxScale`]) and compartment colours,
//!   [`reaction_colors`] the categorical colours, [`reaction_path`] where a
//!   line stops short of its metabolite, and [`crossfeed_routes`] the metro-map
//!   routing of cross-feeding lines.
//! - [`css`] reads `assets/map.css` and works out the computed paint style of
//!   any element of the tree, in light and dark, for a native front end.
//! - [`export`] makes the SVG file the app exports ([`exportable_svg`]) and the
//!   PNG ([`export_png`]), at the scales [`raster_size`] allows.

pub mod crossfeed_routes;
pub mod css;
pub mod export;
pub mod format;
pub mod line_hops;
pub mod mapview;
pub mod raster_size;
pub mod reaction_colors;
pub mod reaction_path;
pub mod state;
pub mod svg;
pub mod theme;

pub use crossfeed_routes::{Route, Routes, route_cross_feeding};
pub use css::{ComputedStyle, Paint, ResolveOptions, Rgba, StyleSheet, map_stylesheet};
pub use export::{ExportError, ExportOptions, InlineOptions, PngExport, export_png, exportable_svg, exportable_tree, inline_styles};
pub use format::esc;
pub use mapview::{frame_label_scale, geometry_handle_side, render_map, render_map_svg, rotation_arm};
pub use raster_size::{PREFERRED_SCALE, raster_scales};
pub use reaction_colors::{PathwayRibbonMode, ReactionColorMode, ReactionColoring, build_reaction_coloring};
pub use state::{
    CrossFeedingEvent, CrossFeedingExplorerView, FvaRange, GeometrySelection, LabelMode, MultiSelection, ReactionWidthMode,
    RenderState, Tooltips,
};
pub use svg::{Attr, Element, Node};
pub use theme::{FluxColorStop, FluxScale, FluxScaleOptions, Scale, compartment_style, format_flux};
