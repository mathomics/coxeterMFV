//! The store around `coxeter-analysis`: the cross-feeding explorer's picks,
//! and the omics imports' refusals. Not ports of a JS test file (the JS tests
//! of these modules exercise the modules alone, ported in coxeter-analysis);
//! the numbers themselves are compared with the JS store in `parity_app.rs`
//! (case `analysis`).

mod common;

use coxeter_render::RenderState;
use coxeter_state::cross_feeding::ExplorerPatch;
use coxeter_state::omics::CommunityOmicsPatch;

#[test]
fn a_cross_feeding_line_picks_its_event_and_picking_it_again_lets_go() {
    let mut store = common::init_store();
    store.use_scenario("consorcio-anaerobio").unwrap();
    store.set_cross_feeding_explorer(ExplorerPatch { enabled: Some(true), ..ExplorerPatch::default() });
    assert!(store.view.show_cross_feeding, "opening the explorer shows the lines");
    let events = store.cross_feeding_explorer_events();
    assert!(!events.is_empty(), "the consortium feeds itself");
    // Largest first, every one between two different species.
    assert!(events.windows(2).all(|w| w[0].flux >= w[1].flux));
    assert!(events.iter().all(|e| e.from != e.to));

    let first = events[0].clone();
    let picked = store.toggle_cross_feeding_link(&first.from, &first.base, &first.to, None).expect("the line's event");
    assert_eq!(picked.id, first.id);
    assert_eq!(store.cross_feeding_explorer.selected_id.as_deref(), Some(first.id.as_str()));
    if let Some(t) = &first.producer_transfer {
        assert_eq!(store.selected_reaction.as_deref(), Some(t.as_str()), "the producer's transfer is selected");
    }
    let state: RenderState = store.map_render_state(Default::default());
    let shown = state.cross_feeding_explorer.as_ref().and_then(|x| x.selected_event.as_ref()).expect("the picked event is drawn");
    assert_eq!((shown.from.as_str(), shown.base.as_str(), shown.to.as_str()), (first.from.as_str(), first.base.as_str(), first.to.as_str()));

    assert!(store.toggle_cross_feeding_link(&first.from, &first.base, &first.to, None).is_none());
    assert!(store.cross_feeding_explorer.selected_id.is_none());

    // A filter that hides the picked event drops the pick on the next refresh.
    store.select_cross_feeding_event(&first.id);
    store.set_cross_feeding_explorer(ExplorerPatch { metabolite: Some("no-such-metabolite".into()), enabled: Some(true), ..Default::default() });
    assert!(store.cross_feeding_explorer_events().is_empty());
    assert!(store.cross_feeding_explorer.selected_id.is_none());
}

#[test]
fn a_failed_solve_has_no_explorer_index() {
    let mut store = common::init_store();
    store.use_scenario("consorcio-anaerobio").unwrap();
    store.refresh_cross_feeding_explorer();
    assert!(store.cross_feeding_explorer.index.is_some());
    for rid in store.community.as_ref().unwrap().biomass_ids.values().cloned().collect::<Vec<_>>() {
        store.set_reaction_bounds(&rid, 5000.0, 5000.0).unwrap();
    }
    store.solve();
    assert!(!store.solution.as_ref().unwrap().is_optimal());
    assert!(store.cross_feeding_explorer.index.is_none());
    assert!(store.cross_feeding.links.is_empty());
}

#[test]
fn omics_imports_refuse_what_they_cannot_read_and_change_nothing() {
    let mut store = common::init_store();
    store.use_scenario("solo-ecoli").unwrap();
    let error = store.import_omics_csv("gene\n", "omics.csv", "gene").unwrap_err();
    assert_eq!(error.to_string(), "El CSV debe incluir encabezado y al menos una fila de datos.");
    assert!(store.omics_dataset.is_none());
    let error = store.import_omics_csv("gene,A\nb0001,x\n", "omics.csv", "gene").unwrap_err();
    assert_eq!(error.to_string(), "No numeric expression/abundance columns were found.");
    let error = store.import_metatranscriptomics("gene,TPM\nb1,2\n", "meta.csv").unwrap_err();
    assert_eq!(error.to_string(), "Falta una columna species/MAG/organism.");
    assert!(store.omics.dataset.is_none());
    let error = store.run_community_gimme(CommunityOmicsPatch::default()).unwrap_err();
    assert_eq!(error.to_string(), "Load metatranscriptomics data first.");
    let overlay = store.omics_overlay();
    assert!(!overlay.enabled);
    assert_eq!(overlay.total_drawn, store.map.as_ref().unwrap().reaction_ids().len());
}

/// Two bundled models with the synthetic gene rules of the parity fixtures,
/// and a long metatranscriptomics table for both.
fn store_with_expression() -> coxeter_state::Store {
    use coxeter_io::model_import::ImportFile;
    let mut store = common::init_store();
    let files = [("eco-gpr.json", "models/ecoli_core_mc.json", "EcoG"), ("sce-gpr.json", "models/yeast_core_mc.json", "SceG")]
        .map(|(name, path, id)| ImportFile { name: name.into(), text: common::model_with_gene_rules(path, id) });
    store.import_model_files(&files).unwrap();
    let mut table = String::from("MAG,gene,condition,TPM\n");
    for (s, sp) in ["EcoG", "SceG"].iter().enumerate() {
        for i in 0..23 {
            table.push_str(&format!("{sp},G{i},A,{}\n{sp},G{i},B,{}\n", (i * 3 + s) % 17 + 2, (i * 11 + 2 * s) % 19 + 1));
        }
    }
    store.import_metatranscriptomics(&table, "meta.csv").unwrap();
    store
}

#[test]
fn gimme_in_the_background_gives_what_the_direct_run_gives() {
    let mut direct = store_with_expression();
    let expected = direct.run_community_gimme(CommunityOmicsPatch::default()).unwrap();

    let mut store = store_with_expression();
    store.start_community_gimme(CommunityOmicsPatch::default()).unwrap();
    assert_eq!(store.status().state.as_str(), "busy");
    let outcomes = store.wait_for_kind(coxeter_state::JobKind::Gimme, std::time::Duration::from_secs(120));
    assert!(outcomes.iter().any(|o| o.kind == coxeter_state::JobKind::Gimme && o.result.is_ok()), "{outcomes:?}");
    assert!(store.omics.gimme_enabled);
    let got = store.omics.gimme_result.clone().unwrap();
    assert_eq!(got.inconsistency_score, expected.inconsistency_score);
    assert_eq!(store.status().message, direct.status().message);
    assert!(store.status().message.starts_with("GIMME A: objective 90%"), "{}", store.status().message);
    assert_eq!(store.solution.as_ref().unwrap().extra.as_ref().unwrap()["gimme"], true);
}

#[test]
fn a_solve_started_after_gimme_supersedes_it() {
    let mut store = store_with_expression();
    store.start_community_gimme(CommunityOmicsPatch::default()).unwrap();
    store.solve();
    let ordinary = store.solution.clone().unwrap();
    let outcomes = store.wait_for_kind(coxeter_state::JobKind::Gimme, std::time::Duration::from_secs(120));
    let outcome = outcomes.iter().find(|o| o.kind == coxeter_state::JobKind::Gimme).expect("GIMME finished");
    assert!(matches!(&outcome.result, Err(e) if e.is_abort()), "{:?}", outcome.result);
    assert!(std::sync::Arc::ptr_eq(store.solution.as_ref().unwrap(), &ordinary));
    assert!(!store.omics.gimme_enabled);
}
