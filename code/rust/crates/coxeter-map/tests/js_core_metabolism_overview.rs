//! Port of `test/core-metabolism-overview.test.mjs`: the glycolysis-only
//! overview drawn the same way in every cell.
//!
//! Every JS test is ported.

mod common;

use std::sync::Arc;

use common::load_model;
use coxeter_core::community::{CommunityModel, CommunitySpec, Objective, SpeciesSpec, namespaced};
use coxeter_map::core_metabolism_overview::{
    CORE_OVERVIEW_GROUPS, CoreOverview, build_core_metabolism_overview, core_overview_support,
};
use coxeter_map::layout_settings::{RawLayoutSettings, layout_preset};
use indexmap::IndexMap;

const CELLS: [&str; 4] = ["Ec1", "Ec2", "Ec3", "Ec4"];

/// `steadyComCommunity()`: the four preview models of the SteadyCom E. coli
/// auxotrophs, a quarter each.
fn steady_com_community() -> CommunityModel {
    let species = CELLS
        .iter()
        .map(|id| SpeciesSpec {
            id: Some(id.to_string()),
            model: Arc::new(load_model(&format!("published/steadycom-ecoli-auxotrophs/preview-{id}"))),
            abundance: Some(0.25),
            enabled: Some(true),
            color: None,
        })
        .collect();
    CommunityModel::new(CommunitySpec {
        name: Some("SteadyCom glycolysis fixture".into()),
        objective: Some(Objective::parse("equal-growth").unwrap()),
        species,
        medium: IndexMap::new(),
        product_target: None,
    })
    .unwrap()
}

/// `buildCoreMetabolismOverview(community, layoutPreset('spacious'))`.
fn build(community: &CommunityModel) -> CoreOverview {
    build_core_metabolism_overview(community, &RawLayoutSettings::from(&layout_preset(Some("spacious"))))
}

/// JS: "core overview contains glycolysis only"
#[test]
fn core_overview_contains_glycolysis_only() {
    assert_eq!(CORE_OVERVIEW_GROUPS.len(), 1);
    assert_eq!(CORE_OVERVIEW_GROUPS[0].id, "glycolysis");
    assert_eq!(CORE_OVERVIEW_GROUPS[0].reactions.to_vec(), ["PGI", "PFK", "FBA", "TPI", "GAPD", "PGK", "PGM", "ENO", "PYK"]);

    let community = steady_com_community();
    let support = core_overview_support(&community);
    assert!(support.supported);
    assert_eq!(support.coverage, 1.0);
}

/// JS: "glycolysis is the only reaction set drawn in every cell"
#[test]
fn glycolysis_is_the_only_reaction_set_drawn_in_every_cell() {
    let community = steady_com_community();
    let built = build(&community);

    assert_eq!(built.per_cell.len(), 4);
    assert_eq!(built.pathway_groups.len(), 4);
    assert_eq!(built.placed, 36);

    let allowed = CORE_OVERVIEW_GROUPS[0].reactions;
    for cell in built.map.cells() {
        let reactions: Vec<_> =
            built.map.reactions().values().filter(|rxn| rxn.cell_id.as_deref() == Some(cell.id.as_str())).collect();
        assert_eq!(reactions.len(), 9, "{} should show 9 glycolysis reactions", cell.id);

        for rxn in reactions {
            let local_id = &rxn.id[cell.id.len() + 1..];
            assert!(allowed.contains(&local_id), "unexpected reaction {}", rxn.id);
        }
    }
}

/// JS: "glycolysis metabolite landmarks run vertically from G6P to pyruvate"
#[test]
fn glycolysis_metabolite_landmarks_run_vertically_from_g6p_to_pyruvate() {
    let community = steady_com_community();
    let built = build(&community);

    let order = ["g6p_c", "f6p_c", "fdp_c", "g3p_c", "13dpg_c", "3pg_c", "2pg_c", "pep_c", "pyr_c"];

    for cell_id in CELLS {
        let cell = built.map.cells().iter().find(|item| item.id == cell_id).unwrap();
        let cytosol = cell.compartments.iter().find(|comp| comp.id == "c").unwrap().rect;

        let points: Vec<(f64, f64)> = order
            .iter()
            .map(|local_id| {
                let nodes = built.map.nodes_for_metabolite(&namespaced(cell_id, local_id));
                let node = nodes.first();
                assert!(node.is_some(), "{cell_id} missing {local_id}");
                let node = node.unwrap();
                ((node.x - cytosol.x) / cytosol.w, (node.y - cytosol.y) / cytosol.h)
            })
            .collect();

        for i in 1..points.len() {
            assert!(points[i].1 > points[i - 1].1, "{cell_id} glycolysis should progress downward at step {i}");
        }

        for point in &points {
            assert!((point.0 - 0.50).abs() < 0.001, "{cell_id} main glycolysis backbone should be centered vertically");
        }
    }
}

/// JS: "DHAP branches left of the vertical backbone and rejoins through TPI"
#[test]
fn dhap_branches_left_of_the_vertical_backbone_and_rejoins_through_tpi() {
    let community = steady_com_community();
    let built = build(&community);

    let cell = built.map.cells().iter().find(|item| item.id == "Ec1").unwrap();
    let cytosol = cell.compartments.iter().find(|comp| comp.id == "c").unwrap().rect;
    let dhap = built.map.nodes_for_metabolite(&namespaced("Ec1", "dhap_c")).first().copied();
    let g3p = built.map.nodes_for_metabolite(&namespaced("Ec1", "g3p_c")).first().copied();

    assert!(dhap.is_some() && g3p.is_some());
    let (dhap, g3p) = (dhap.unwrap(), g3p.unwrap());
    assert!(dhap.x < g3p.x - cytosol.w * 0.08, "DHAP should branch clearly to the left");
    assert!((dhap.y - g3p.y).abs() < cytosol.h * 0.08, "DHAP/G3P should occupy the same branch level");

    let tpi = built.map.reaction(&namespaced("Ec1", "TPI"));
    let axis = tpi.and_then(|r| r.axis);
    assert!(axis.is_some(), "TPI should have an axis");
    let axis = axis.unwrap();
    assert!(axis.dir.x.abs() > axis.dir.y.abs(), "TPI should be predominantly horizontal from DHAP toward G3P");
}

/// JS: "linear glycolysis reactions point predominantly downward"
#[test]
fn linear_glycolysis_reactions_point_predominantly_downward() {
    let community = steady_com_community();
    let built = build(&community);

    for local_id in ["PGI", "PFK", "GAPD", "PGK", "PGM", "ENO", "PYK"] {
        let reaction = built.map.reaction(&namespaced("Ec1", local_id));
        let axis = reaction.and_then(|r| r.axis);
        assert!(axis.is_some(), "missing axis for {local_id}");
        let axis = axis.unwrap();
        assert!(axis.dir.y.abs() > axis.dir.x.abs(), "{local_id} should be predominantly vertical");
        assert!(axis.dir.y > 0.0, "{local_id} should point downward");
    }
}

/// JS: "overview labels contain glycolysis metabolites only"
#[test]
fn overview_labels_contain_glycolysis_metabolites_only() {
    let community = steady_com_community();
    let built = build(&community);
    let texts: Vec<&str> = built.map.texts().values().map(|item| item.text.as_str()).collect();

    assert!(texts.contains(&"GLYCOLYSIS"));
    for label in ["G6P", "F6P", "FBP", "DHAP", "G3P", "1,3-BPG", "3PG", "2PG", "PEP", "PYR"] {
        assert!(texts.contains(&label), "missing label {label}");
    }

    for forbidden in ["TCA CYCLE", "PENTOSE PHOSPHATE PATHWAY", "SER / GLY FAMILY ← 3PG"] {
        assert!(!texts.contains(&forbidden), "forbidden overview label {forbidden}");
    }
}
