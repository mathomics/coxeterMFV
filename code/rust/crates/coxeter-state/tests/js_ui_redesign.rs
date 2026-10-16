//! Port of the Store tests of `test/ui-redesign.test.mjs`.
//!
//! Ported (they drive a real `new Store`): "published community without
//! bundled models clears the previous active map" and "contextual selection
//! keeps scientific state separate from visual entity state".
//!
//! Not ported — GUI, for the app crate: "application shell exposes the
//! mockup-aligned work areas and analysis menu" (reads `index.html`), "Edit
//! Map Metabolite tool stays in Edit Map and does not open the biological Edit
//! Model workspace", "Scenario return waits for the selected community
//! rebuild and repaints the map", "Shared Medium boundary starts a geometry
//! resize drag instead of panning" and "Select moves and resizes compartments
//! from their edge, not their body" (all four read the source of
//! `src/ui/app.js`), "metabolite inspector reports available model and map
//! properties" (`renderInspector` over a plain-object store), "Add Reaction
//! panel shows reaction stoichiometry before eight in-panel directions and
//! reuses the selected metabolite" (`MetaboliteMenuController` over a
//! plain-object store), "the Add Reaction panel is never taller than the
//! stage, so its direction wheel is always in reach" and "the Add Reaction
//! panel can be dragged by its header and stays where it is put"
//! (`src/ui/add-reaction-panel.js`), "compartment Add Reaction panel uses the
//! same reaction preview and directions without linking an existing
//! metabolite" (`CompartmentMenuController` over a plain-object store),
//! "results panel uses real solver output and leaves unsupported values
//! disabled" (`renderResults` over a plain-object store).
//!
//! The JS stubs the previous session with placeholder objects
//! (`store.community = { name: 'Old community' }`, `store.problem = { old:
//! true }`…); the Rust gives it real ones of the right types, since only
//! their being cleared is asserted. `store.publishedManifest = async () =>
//! null` is the store's manifest cache holding "none" for that community.
//! `store.subscribe` is `take_events()`: the last reason emitted.

mod common;

use std::sync::Arc;

use coxeter_core::community::{CommunityModel, CommunitySpec, SpeciesSpec};
use coxeter_map::EditableMap;
use coxeter_render::theme::FluxScale;
use coxeter_solver::fba::FluxProblem;
use coxeter_solver::lp::LpStatus;
use coxeter_state::{SelectedEntity, Solution};
use regex::RegexBuilder;
use serde_json::{Map, json};

use common::{bundled_model, store};

/// JS: 'published community without bundled models clears the previous active map'
#[test]
fn published_community_without_bundled_models_clears_the_previous_active_map() {
    let mut store = store();
    store.published_communities = vec![json!({
        "id": "missing-models",
        "name": "Missing models community",
        "description": "No bundled model files",
    })];
    // `store.publishedManifest = async () => null`.
    store.published_manifests.insert("missing-models".into(), None);
    let model = bundled_model("ecoli_core_mc");
    let community = CommunityModel::new(CommunitySpec {
        species: vec![SpeciesSpec::new("eco", Arc::new(model.clone())).abundance(1.0)],
        ..Default::default()
    })
    .unwrap();
    store.community = Some(Arc::new(community));
    store.problem = Some(Arc::new(FluxProblem::new(&model)));
    store.map = Some(EditableMap::blank(Some(1000.0), Some(700.0)));
    store.solution = Some(Arc::new(Solution {
        status: LpStatus::Optimal,
        objective_value: 0.0,
        fluxes: Default::default(),
        message: String::new(),
        solver: String::new(),
        result: None,
        steady_com: None,
        extra: None,
        zuniga: None,
    }));
    store.scale = Some(FluxScale::automatic([], false));

    store.use_published_community("missing-models").expect("the community is selected");

    assert_eq!(store.scenario_id().as_deref(), Some("published:missing-models"));
    assert!(store.community.is_none());
    assert!(store.problem.is_none());
    assert!(store.map.is_none());
    assert!(store.solution.is_none());
    assert!(store.scale.is_none());
    let message = &store.status().message;
    assert!(
        RegexBuilder::new("no model files are bundled").case_insensitive(true).build().unwrap().is_match(message),
        "{message:?} does not match /no model files are bundled/i"
    );
}

/// JS: 'contextual selection keeps scientific state separate from visual entity state'
#[test]
fn contextual_selection_keeps_scientific_state_separate_from_visual_entity_state() {
    let mut store = store();
    let reason = |store: &mut coxeter_state::Store| store.take_events().last().cloned();

    store.select_entity("metabolite", "node-1", Map::new());
    assert_eq!(store.selected_entity, Some(SelectedEntity::new("metabolite", "node-1")));
    assert_eq!(store.selected_reaction, None);
    assert_eq!(reason(&mut store).as_deref(), Some("selection"));

    store.select(Some("reaction-1"));
    assert_eq!(store.selected_entity, Some(SelectedEntity::new("reaction", "reaction-1")));
    assert_eq!(store.selected_reaction.as_deref(), Some("reaction-1"));

    store.select(None);
    assert_eq!(store.selected_entity, None);
}
