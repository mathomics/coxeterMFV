//! A COBRA Toolbox .mat model loaded through Load model(s), and gene
//! knockouts from the Reactions page's Genes control and the Inspector's gene
//! list. The store's half is tested in `coxeter-state/tests/gene_knockouts.rs`.

mod common;

use coxeter_app::ui_state::Workspace;

#[test]
fn a_mat_model_loads_and_its_genes_are_knocked_out_from_the_reactions_page_and_the_inspector() {
    let (mut h, dialogs, _dir) = common::harness_saving("gene-knockouts", None);
    h.state_mut().open_workspace(Workspace::Community);
    common::settle(&mut h);
    dialogs.with(|d| d.opens.push_back(vec![common::repo_root().join("test/fixtures/cobra-toolbox/mini.mat")]));
    common::click(&mut h, "Load model(s)");
    let store = &h.state().store;
    assert_eq!(store.species.len(), 1, "{}", store.status().message);
    let sp = store.species[0].id.clone();
    let growth = store.solution.as_ref().map_or(f64::NAN, |s| s.objective_value);
    assert!(growth > 0.0, "the imported model grows: {growth}");

    // The Genes control: search, see what a gene would close, knock it out.
    h.state_mut().open_workspace(Workspace::Reactions);
    common::settle(&mut h);
    common::type_into(&mut h, "Search genes to knock out", "g4");
    common::settle(&mut h);
    assert!(common::shows(&h, "would close HEX1"), "the preview names the reaction");
    common::click(&mut h, "Knock out gene g4");
    let hex = format!("{sp}@HEX1");
    assert_eq!(h.state().store.current_reaction_bounds(&hex).upper, 0.0);
    assert!(h.state().store.solution.as_ref().unwrap().objective_value.abs() < 1e-9, "no hexokinase, no growth");
    // Its chip restores it.
    common::click(&mut h, "g4 knocked out: restore it");
    assert!(h.state().store.gene_knockouts.is_empty());

    // The Inspector: a KO toggle beside each gene of the selected reaction.
    h.state_mut().close_workspace();
    h.state_mut().store.select(Some(&format!("{sp}@GLCt")));
    common::settle(&mut h);
    common::click(&mut h, "Knock out gene g3");
    assert!(!h.state().store.reaction_knocked_out(&format!("{sp}@GLCt")), "g1 and g2 still make the transporter");
    common::click(&mut h, "Knock out gene g1");
    assert!(h.state().store.reaction_knocked_out(&format!("{sp}@GLCt")));
    assert_eq!(h.state().store.gene_knockout_changes().len(), 2);
    h.state_mut().open_workspace(Workspace::Reactions);
    common::settle(&mut h);
    common::click(&mut h, "Undo constraint edit");
    assert!(!h.state().store.reaction_knocked_out(&format!("{sp}@GLCt")), "undo restores g1");
}
