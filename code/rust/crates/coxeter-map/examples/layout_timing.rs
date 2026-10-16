//! Time the automatic layout on the largest maps the bundled models make,
//! with the neighbour grid and (optionally) with the JavaScript's all-pairs
//! repulsion, and check both give the same map.
//!
//!   cargo run --release -p coxeter-map --example layout_timing [all-pairs]
//!
//! `all-pairs` also times the plain loop on every case (minutes on the
//! four-E. coli community).

use std::sync::Arc;
use std::time::Instant;

use coxeter_core::community::{CommunityModel, CommunitySpec, SpeciesSpec};
use coxeter_core::model::Model;
use coxeter_map::force::set_neighbour_grid;
use coxeter_map::layout::{LayoutOptions, layout_community};

fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..").canonicalize().unwrap()
}

fn model(rel: &str) -> Arc<Model> {
    let text = std::fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"));
    Arc::new(Model::from_json_str(&text).unwrap())
}

fn community(members: &[(&str, &str)]) -> CommunityModel {
    let n = members.len() as f64;
    CommunityModel::new(CommunitySpec {
        species: members.iter().map(|(id, file)| SpeciesSpec::new(id, model(file)).abundance(1.0 / n)).collect(),
        ..Default::default()
    })
    .unwrap()
}

fn time(c: &CommunityModel, runs: usize) -> (f64, coxeter_map::layout::Layout) {
    let mut best = f64::INFINITY;
    let mut out = None;
    for _ in 0..runs {
        let t = Instant::now();
        let layout = layout_community(c, &LayoutOptions::default());
        best = best.min(t.elapsed().as_secs_f64());
        out = Some(layout);
    }
    (best, out.unwrap())
}

fn main() {
    let all_pairs = std::env::args().any(|a| a == "all-pairs");
    let ec = |id: &str| format!("models/published/steadycom-ecoli-auxotrophs/model-{id}.json");
    let (e1, e2, e3, e4) = (ec("Ec1"), ec("Ec2"), ec("Ec3"), ec("Ec4"));
    let cases: Vec<(&str, Vec<(&str, &str)>)> = vec![
        (
            "five core models (scenario 'todas')",
            vec![
                ("lla", "models/lactobacillus_core_mc.json"),
                ("eco", "models/ecoli_core_mc.json"),
                ("sce", "models/yeast_core_mc.json"),
                ("syn", "models/cyano_core_mc.json"),
                ("mba", "models/methanogen_core_mc.json"),
            ],
        ),
        ("iAF1260 (one E. coli)", vec![("Ec1", &e1)]),
        ("4 x iAF1260 (steadycom-ecoli-auxotrophs)", vec![("Ec1", &e1), ("Ec2", &e2), ("Ec3", &e3), ("Ec4", &e4)]),
    ];
    println!("{:<44} {:>9} {:>8} {:>10} {:>12} {:>9}", "map", "reactions", "nodes", "grid (s)", "all pairs (s)", "speed-up");
    for (name, members) in cases {
        let c = community(&members);
        set_neighbour_grid(true);
        let (grid, layout) = time(&c, if members.len() > 1 && name.starts_with('4') { 1 } else { 3 });
        let (plain, same) = if all_pairs {
            set_neighbour_grid(false);
            let (t, reference) = time(&c, 1);
            set_neighbour_grid(true);
            (Some(t), Some(reference == layout))
        } else {
            (None, None)
        };
        println!(
            "{:<44} {:>9} {:>8} {:>10.3} {:>12} {:>9}{}",
            name,
            layout.reactions.len(),
            layout.nodes.len(),
            grid,
            plain.map(|t| format!("{t:.2}")).unwrap_or_else(|| "-".into()),
            plain.map(|t| format!("{:.0}x", t / grid)).unwrap_or_else(|| "-".into()),
            match same {
                Some(true) => "   identical",
                Some(false) => "   DIFFERENT",
                None => "",
            }
        );
    }
}
