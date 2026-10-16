//! Port of `test/yeast9-layout.test.mjs`: the Yeast9 compartment profile, its
//! containment tree and the layout of its organelles.
//!
//! All three JavaScript tests are ported.

use std::collections::HashMap;
use std::sync::Arc;

use coxeter_core::community::{CommunityModel, CommunitySpec, SpeciesSpec};
use coxeter_core::compartments::{Names, compartment_layout_profile, is_yeast9_compartment_set};
use coxeter_core::mapdata::MapCompartment;
use coxeter_core::model::Model;
use coxeter_map::layout::{Contents, LayoutOptions, layout_community};
use indexmap::IndexMap;
use serde_json::{Value, json};

/// The JS `compartments` object, in its declared order.
const COMPARTMENTS: [(&str, &str); 14] = [
    ("c", "cytoplasm"),
    ("ce", "cell envelope"),
    ("e", "extracellular"),
    ("er", "endoplasmic reticulum"),
    ("erm", "endoplasmic reticulum membrane"),
    ("g", "Golgi"),
    ("gm", "Golgi membrane"),
    ("lp", "lipid particle"),
    ("m", "mitochondrion"),
    ("mm", "mitochondrial membrane"),
    ("n", "nucleus"),
    ("p", "peroxisome"),
    ("v", "vacuole"),
    ("vm", "vacuolar membrane"),
];

fn compartment_names() -> Names {
    COMPARTMENTS.iter().map(|(id, name)| (id.to_string(), name.to_string())).collect()
}

/// `Object.keys(compartments)`.
fn compartment_ids() -> Vec<&'static str> {
    COMPARTMENTS.iter().map(|(id, _)| *id).collect()
}

fn yeast9_mini_model() -> Model {
    let compartments: serde_json::Map<String, Value> =
        COMPARTMENTS.iter().map(|(id, name)| (id.to_string(), json!(name))).collect();
    let metabolites: Vec<Value> = COMPARTMENTS
        .iter()
        .map(|(id, _)| {
            json!({
                "id": format!("marker_{id}"),
                "name": format!("marker {id}"),
                "compartment": id,
                "formula": "",
                "charge": 0,
                "primary": true,
            })
        })
        .collect();
    Model::from_value(&json!({
        "id": "yeast9-mini",
        "name": "Yeast9 topology fixture",
        "compartments": compartments,
        "cell": {
            "name": "S. cerevisiae",
            "organism": "Saccharomyces cerevisiae",
            "shape": "round",
        },
        "metabolites": metabolites,
        "reactions": [],
    }))
    .unwrap()
}

/// JS: `Yeast9 is detected as a dedicated compartment layout profile`
#[test]
fn yeast9_is_detected_as_a_dedicated_compartment_layout_profile() {
    let ids = compartment_ids();
    let names = compartment_names();
    assert!(is_yeast9_compartment_set(&ids, &names));
    assert_eq!(compartment_layout_profile(&ids, &names), "yeast9");
}

/// JS: `Yeast9 uses its biological 14-compartment containment tree`
#[test]
fn yeast9_uses_its_biological_14_compartment_containment_tree() {
    let model = yeast9_mini_model();
    let tree = model.compartment_tree();
    let by_id = &tree.by_id;

    assert_eq!(by_id.len(), 14);
    let expected_parents: [(&str, Option<&str>); 14] = [
        ("e", None),
        ("ce", Some("e")),
        ("c", Some("ce")),
        ("erm", Some("c")),
        ("er", Some("erm")),
        ("gm", Some("c")),
        ("g", Some("gm")),
        ("lp", Some("c")),
        ("mm", Some("c")),
        ("m", Some("mm")),
        ("n", Some("c")),
        ("p", Some("c")),
        ("vm", Some("c")),
        ("v", Some("vm")),
    ];

    for (id, parent) in expected_parents {
        // `byId.get(id)?.parent ?? null`
        let actual = by_id.get(id).and_then(|node| node.parent.as_deref());
        assert_eq!(actual, parent, "{id} should be inside {}", parent.unwrap_or("the cell"));
    }

    // The easiest mistake in a yeast layout: in Yeast9 p means peroxisome, not
    // bacterial periplasm.
    assert_eq!(by_id.get("p").unwrap().parent.as_deref(), Some("c"));
}

/// JS: `Yeast9 cytosolic organelles are rendered as separate non-overlapping islands`
#[test]
fn yeast9_cytosolic_organelles_are_rendered_as_separate_non_overlapping_islands() {
    let model = yeast9_mini_model();
    let community = CommunityModel::new(CommunitySpec {
        species: vec![SpeciesSpec {
            id: Some("sce".into()),
            model: Arc::new(model),
            abundance: Some(1.0),
            enabled: Some(true),
            color: None,
        }],
        medium: IndexMap::new(),
        ..Default::default()
    })
    .unwrap();
    let layout = layout_community(&community, &LayoutOptions { contents: Contents::None, ..Default::default() });
    let cell = &layout.cells[0];

    assert_eq!(cell.compartments.len(), 14);
    let by_id: HashMap<&str, &MapCompartment> =
        cell.compartments.iter().map(|compartment| (compartment.id.as_str(), compartment)).collect();
    let cytosol = by_id["c"].rect;
    let organelles: Vec<&MapCompartment> = ["erm", "gm", "lp", "mm", "n", "p", "vm"]
        .iter()
        .map(|id| *by_id.get(id).expect("expected organelle compartment"))
        .collect();

    for compartment in &organelles {
        let r = compartment.rect;
        assert!(r.x >= cytosol.x && r.y >= cytosol.y, "{} starts outside cytosol", compartment.id);
        assert!(r.x + r.w <= cytosol.x + cytosol.w + 1e-6, "{} exceeds cytosol width", compartment.id);
        assert!(r.y + r.h <= cytosol.y + cytosol.h + 1e-6, "{} exceeds cytosol height", compartment.id);
    }

    for i in 0..organelles.len() {
        for j in (i + 1)..organelles.len() {
            let a = organelles[i].rect;
            let b = organelles[j].rect;
            let overlap = a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;
            assert!(!overlap, "{} overlaps {}", organelles[i].id, organelles[j].id);
        }
    }
}
