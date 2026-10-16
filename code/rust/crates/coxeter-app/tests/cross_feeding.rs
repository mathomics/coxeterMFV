//! The cross-feeding explorer through the real interface: its card
//! (`crossFeedingCard`, `renderCrossFeedingEvents`), the metabolite picker
//! (`crossFeedingMetabolitePicker`) and the View menu's count
//! (`crossFeedingTopControl`) — one test per `data-action` they wire
//! (`onSidebarClick` / `onSidebarInput` of `src/ui/app.js`), checked on the
//! Store as the JS changes it.
//!
//! The first scenario (the anaerobic consortium) is solved at boot, and its
//! species feed each other.

mod common;

use coxeter_app::Shell;
use coxeter_app::panels::cross_feeding::exchange_names;
use coxeter_app::ui_state::Workspace;
use egui_kittest::Harness;

type H = Harness<'static, Shell>;

fn open(h: &mut H, ws: Workspace) {
    h.state_mut().open_workspace(ws);
    common::settle(h);
}

/// `nameOf(id)`: the species' cell name.
fn name_of(h: &H, id: &str) -> String {
    let store = &h.state().store;
    let sp = store.species.iter().find(|s| s.id == id).unwrap();
    store.models.get(&sp.file).unwrap().cell.name.clone()
}

/// The explorer, opened through its card.
fn explorer(h: &mut H) {
    open(h, Workspace::Crossfeeding);
    common::click(h, "Explore cross-feeding");
}

#[test]
fn open_crossfeeding_explorer() {
    let mut h = common::harness(None);
    open(&mut h, Workspace::Crossfeeding);
    let n = h.state().store.cross_feeding.links.iter().filter(|l| l.from.is_some() && l.to.is_some()).count();
    assert!(n > 0, "the consortium cross-feeds");
    assert!(common::shows(&h, &format!("{n} active producer→consumer event(s) detected in the current flux solution.")));
    assert!(!h.state().store.view.show_cross_feeding);
    h.state_mut().close_workspace();
    common::settle(&mut h);

    explorer(&mut h);
    let store = &h.state().store;
    assert_eq!(h.state().ui.workspace, Some(Workspace::Crossfeeding));
    assert!(store.cross_feeding_explorer.enabled);
    assert!(store.view.show_cross_feeding, "the lines are drawn");
    let total = store.cross_feeding_explorer.index.as_ref().unwrap().events.len();
    assert!(total > 0);
    let visible = store.cross_feeding_explorer_events().len();
    assert!(common::shows(&h, &format!(" of {total} active events · flux solution evidence.")));
    assert!(common::shows(&h, &visible.to_string()));
    assert!(common::shows(&h, "mmol·gDW community⁻¹·h⁻¹"));
}

#[test]
fn crossfeeding_filters_and_clear() {
    let mut h = common::harness(None);
    explorer(&mut h);
    let event = h.state().store.cross_feeding_explorer_events()[0].clone();

    // crossfeeding-query: live.
    common::type_into(&mut h, "Search metabolite or species", &event.base);
    assert_eq!(h.state().store.cross_feeding_explorer.query, event.base);
    let visible = h.state().store.cross_feeding_explorer_events();
    assert!(!visible.is_empty() && visible.len() < h.state().store.cross_feeding_explorer.index.as_ref().unwrap().events.len());
    common::type_into(&mut h, "Search metabolite or species", "");
    // crossfeeding-metabolite
    common::choose(&mut h, "Metabolite filter", &event.base);
    assert_eq!(h.state().store.cross_feeding_explorer.metabolite, event.base);
    assert!(h.state().store.cross_feeding_explorer_events().iter().all(|e| e.base == event.base));
    // crossfeeding-producer
    let producer = name_of(&h, &event.from);
    common::choose(&mut h, "Producer filter", &producer);
    assert_eq!(h.state().store.cross_feeding_explorer.producer, event.from);
    // crossfeeding-consumer
    let consumer = name_of(&h, &event.to);
    common::choose(&mut h, "Consumer filter", &consumer);
    assert_eq!(h.state().store.cross_feeding_explorer.consumer, event.to);
    assert!(h.state().store.cross_feeding_explorer_events().iter().any(|e| e.id == event.id));
    // crossfeeding-min-flux: `Number(el.value) || 0`.
    common::type_into(&mut h, "Min flux", "1e9");
    assert_eq!(h.state().store.cross_feeding_explorer.min_flux, 1e9);
    assert!(h.state().store.cross_feeding_explorer_events().is_empty());
    assert!(common::shows(&h, "No events match these filters."));
    common::type_into(&mut h, "Min flux", "abc");
    assert_eq!(h.state().store.cross_feeding_explorer.min_flux, 0.0);

    // clear-crossfeeding-filters
    common::type_into(&mut h, "Search metabolite or species", "x");
    h.state_mut().store.select_cross_feeding_event(&event.id);
    common::settle(&mut h);
    common::click(&mut h, "Clear filters");
    let x = &h.state().store.cross_feeding_explorer;
    assert_eq!((x.query.as_str(), x.metabolite.as_str(), x.producer.as_str(), x.consumer.as_str(), x.min_flux), ("", "", "", "", 0.0));
    assert_eq!(x.selected_id, None);
    assert!(x.enabled, "the explorer stays open");
}

#[test]
fn select_and_close_the_explorer() {
    let mut h = common::harness(None);
    explorer(&mut h);
    let event = h.state().store.cross_feeding_explorer_events()[0].clone();
    let label = format!("{} → {} → {}", name_of(&h, &event.from), event.base, name_of(&h, &event.to));
    h.state_mut().store.cross_feeding_explorer.selected_at = Some(coxeter_core::mapdata::Point { x: 1.0, y: 2.0 });

    // select-crossfeeding-event: picked from the list, not pinned to a map point.
    common::click(&mut h, &label);
    let store = &h.state().store;
    assert_eq!(store.cross_feeding_explorer.selected_id.as_deref(), Some(event.id.as_str()));
    assert_eq!(store.cross_feeding_explorer.selected_at, None);
    if let Some(transfer) = &event.producer_transfer {
        assert_eq!(store.selected_reaction.as_deref(), Some(transfer.as_str()));
    }
    assert!(common::shows(&h, "Producer transfer: "), "the picked event is described");
    assert!(common::shows(&h, &label));

    // close-crossfeeding-explorer
    common::click(&mut h, "Close cross-feeding explorer");
    let x = &h.state().store.cross_feeding_explorer;
    assert!(!x.enabled);
    assert_eq!(x.selected_id, None);
    assert!(common::has(&h, "Explore cross-feeding"));
}

#[test]
fn metabolite_picker() {
    let mut h = common::harness(None);
    open(&mut h, Workspace::Crossfeeding);
    let all = h.state().store.cross_feeding_metabolites();
    assert!(all.len() >= 2, "{} exchanged metabolites", all.len());
    assert!(common::shows(&h, "Metabolites on the map"));
    let top = all.len().min(5);
    assert!(common::shows(&h, &format!("{top} of {}", all.len())), "the five most important by default");
    assert!(common::shows(&h, &all[0].group.to_uppercase()), "grouped");

    // crossfeed-show-all
    common::click(&mut h, "All");
    assert_eq!(h.state().store.view.cross_feeding_top, 0.0);
    assert_eq!(h.state().store.view.cross_feeding_shown, None);
    assert_eq!(h.state().store.cross_feeding_visible_bases().len(), all.len());
    // crossfeed-show-none
    common::click(&mut h, "None");
    assert_eq!(h.state().store.view.cross_feeding_shown, Some(Vec::new()));
    assert!(common::shows(&h, &format!("0 of {}", all.len())));
    // crossfeed-show-top
    common::click(&mut h, "Top 5");
    assert_eq!(h.state().store.view.cross_feeding_top, 5.0);
    assert_eq!(h.state().store.view.cross_feeding_shown, None);

    // crossfeed-metabolite: the list becomes the user's own.
    let names = exchange_names(&h.state().store);
    let first = &all[0];
    let label = match names.get(&first.base) {
        Some(name) if name != &first.base => format!("{name} ({})", first.base),
        _ => first.base.clone(),
    };
    common::click(&mut h, &label);
    let shown = h.state().store.view.cross_feeding_shown.clone().expect("picked by hand");
    assert_eq!(shown.len(), top - 1);
    assert!(!shown.contains(&first.base));
    common::click(&mut h, &label);
    assert!(h.state().store.view.cross_feeding_shown.as_ref().unwrap().contains(&first.base));
    let lines = format!("{} line{} · ", first.lines, if first.lines == 1 { "" } else { "s" });
    assert!(common::shows(&h, &lines));
}

#[test]
fn view_menu_top_control_and_choose() {
    let mut h = common::harness(None);
    let all = h.state().store.cross_feeding_metabolites().len();
    common::press(&mut h, "View menu");
    assert_eq!(common::control(&h, "Number of exchanged metabolites shown").value().as_deref(), Some("5"));
    assert!(common::shows(&h, &format!("of {all}")));

    // crossfeed-top: applied once the number is settled.
    common::type_into(&mut h, "Number of exchanged metabolites shown", "1");
    assert_eq!(h.state().store.view.cross_feeding_top, 1.0);
    assert_eq!(h.state().store.cross_feeding_visible_bases().len(), 1);

    // A hand-picked list shows its size and says so.
    let base = h.state().store.cross_feeding_metabolites()[1].base.clone();
    h.state_mut().store.toggle_cross_feeding_metabolite(&base, true);
    common::settle(&mut h);
    if !common::has(&h, "Number of exchanged metabolites shown") {
        common::press(&mut h, "View menu");
    }
    assert_eq!(common::control(&h, "Number of exchanged metabolites shown").value().as_deref(), Some("2"));
    assert!(common::shows(&h, &format!("of {all} · picked by hand")));

    // crossfeed-choose
    common::click(&mut h, "Choose…");
    assert_eq!(h.state().ui.workspace, Some(Workspace::Crossfeeding));
    assert!(common::shows(&h, "Metabolites on the map"));
}

#[test]
fn no_top_control_without_cross_feeding() {
    let mut h = common::harness(Some("solo-ecoli"));
    assert!(h.state().store.cross_feeding_metabolites().is_empty());
    common::press(&mut h, "View menu");
    assert!(common::has(&h, "Cofactors"), "the menu is open");
    assert!(!common::has(&h, "Number of exchanged metabolites shown"));
    open(&mut h, Workspace::Crossfeeding);
    assert!(common::shows(&h, "0 active producer→consumer event(s)"));
    assert!(!common::shows(&h, "Metabolites on the map"));
}
