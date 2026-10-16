//! The neighbour grid of `relax` changes how close pairs are found, not what
//! is computed: every layout comes out bit for bit as with the JavaScript's
//! all-pairs loop. (The parity fixtures prove the grid against the JavaScript
//! itself; this proves it against the plain loop in the same build, on maps
//! large enough for the grid to be in use everywhere.)
//!
//! Its own test binary, because the switch is process-wide.

mod common;

use coxeter_map::force::set_neighbour_grid;
use coxeter_map::layout::{LayoutOptions, layout_community};
use coxeter_map::layout_settings::{RawLayoutSettings, layout_engine_options, layout_preset};
use common::*;

#[test]
fn layouts_are_bit_identical_with_and_without_the_grid() {
    for scenario in ["todas", "consorcio-anaerobio"] {
        let community = scenario_community(scenario);
        for preset in [None, Some("compact"), Some("spacious")] {
            let opts = match preset {
                None => LayoutOptions::default(),
                Some(p) => layout_engine_options(&RawLayoutSettings::from(&layout_preset(Some(p)))),
            };
            set_neighbour_grid(true);
            let with = layout_community(&community, &opts);
            set_neighbour_grid(false);
            let without = layout_community(&community, &opts);
            set_neighbour_grid(true);
            assert!(with == without, "{scenario} / {preset:?}: the grid changed the layout");
            assert!(with.nodes.len() > 300);
        }
    }
}
