//! What every interface test needs: the real application (the Store over
//! the repository's `models/`, the whole window drawn by `Shell::frame`)
//! driven offscreen by `egui_kittest`, with file dialogs answered from a
//! script.
//!
//! ```ignore
//! mod common;
//! let mut h = common::harness(None);          // the first scenario, solved
//! common::click(&mut h, "Edit Model");         // by accessible label
//! assert_eq!(h.state().ui.left_tab, LeftTab::Model);
//! ```
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use coxeter_app::Shell;
use coxeter_app::files::{FileDialogs, ScriptedDialogs, SharedDialogs};
use coxeter_state::Store;
use egui_kittest::Harness;
use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT, Queryable};

/// The repository root (it holds `models/`).
pub fn repo_root() -> PathBuf {
    coxeter_core::testkit::repo_root()
}

/// A fresh folder under the system's temp directory.
pub fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("coxeter-app-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// A store over the repository's bundled files (not yet `init()`-ed).
pub fn store() -> Store {
    Store::with_dir(repo_root())
}

/// A booted shell (`init()` and the scenario, or the first one), with
/// `dialogs` answering the file dialogs.
pub fn shell_with(scenario: Option<&str>, dialogs: Box<dyn FileDialogs>) -> Shell {
    let mut shell = Shell::new(store(), dialogs);
    shell.boot(scenario);
    assert!(shell.boot_error.is_none(), "boot: {:?}", shell.boot_error);
    shell
}

/// A booted shell whose dialogs open nothing and save nowhere.
pub fn shell(scenario: Option<&str>) -> Shell {
    shell_with(scenario, Box::new(ScriptedDialogs::new()))
}

/// The window at a laptop's size, drawn by the real interface.
pub fn harness_of(shell: Shell) -> Harness<'static, Shell> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .with_max_steps(40)
        .build_eframe(move |_cc| shell);
    settle(&mut h);
    h
}

/// The same, drawn through wgpu so it can be rendered to an image.
pub fn rendering_harness_of(shell: Shell) -> Harness<'static, Shell> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .with_max_steps(40)
        .wgpu()
        .build_eframe(move |_cc| shell);
    settle(&mut h);
    h
}

/// The first scenario (or `scenario`), in the window.
pub fn harness(scenario: Option<&str>) -> Harness<'static, Shell> {
    harness_of(shell(scenario))
}

/// A harness whose saves land in a fresh folder, with a handle on what the
/// app asked of the dialogs and the folder.
pub fn harness_saving(name: &str, scenario: Option<&str>) -> (Harness<'static, Shell>, SharedDialogs, PathBuf) {
    let dir = temp_dir(name);
    let dialogs = SharedDialogs::new(ScriptedDialogs::saving_to(&dir));
    let h = harness_of(shell_with(scenario, Box::new(dialogs.clone())));
    (h, dialogs, dir)
}

/// Let the interface catch up: frames until nothing is pending, background
/// work finished, debounced calls run.
pub fn settle(h: &mut Harness<'static, Shell>) {
    let _ = h.try_run();
    for _ in 0..4 {
        let busy = h.state().store.has_running_jobs() || !h.state().deferred.is_empty() || h.state().pending_solve;
        if !busy {
            break;
        }
        h.state_mut().store.wait_for_jobs(Duration::from_secs(120));
        h.state_mut().flush_deferred();
        let _ = h.try_run();
    }
    let _ = h.try_run();
}

/// Whether a node is a control (not a text label that happens to say the same).
fn is_control(node: &egui_kittest::kittest::AccessKitNode<'_>) -> bool {
    !matches!(node.role(), Role::Label | Role::Unknown | Role::GenericContainer)
}

/// Every control with this accessible label, in drawing order.
pub fn controls<'h>(h: &'h Harness<'static, Shell>, label: &'h str) -> Vec<egui_kittest::Node<'h>> {
    h.query_all_by(move |n| is_control(n) && n.label().as_deref() == Some(label)).collect()
}

/// The one control with this accessible label.
pub fn control<'h>(h: &'h Harness<'static, Shell>, label: &'h str) -> egui_kittest::Node<'h> {
    let mut nodes = controls(h, label);
    assert_eq!(nodes.len(), 1, "controls labelled {label:?}");
    nodes.remove(0)
}

/// Click the control with this accessible label (the only one), then settle.
pub fn click(h: &mut Harness<'static, Shell>, label: &str) {
    control(h, label).click_accesskit();
    settle(h);
}

/// Click the `n`-th control with this label (in drawing order).
pub fn click_nth(h: &mut Harness<'static, Shell>, label: &str, n: usize) {
    let nodes = controls(h, label);
    assert!(nodes.len() > n, "only {} controls labelled {label:?}", nodes.len());
    nodes[n].click_accesskit();
    settle(h);
}

/// Click with the pointer (for popups and menus, which want a real press).
pub fn press(h: &mut Harness<'static, Shell>, label: &str) {
    control(h, label).click();
    settle(h);
}

/// Whether some control has this label.
pub fn has(h: &Harness<'static, Shell>, label: &str) -> bool {
    !controls(h, label).is_empty()
}

/// Whether some text on screen (a label or a control) contains `text`.
pub fn shows(h: &Harness<'static, Shell>, text: &str) -> bool {
    h.query_all_by(|n| n.label().is_some_and(|l| l.contains(text)) || n.value().is_some_and(|v| v.contains(text))).next().is_some()
}

/// Type into the field with this label and commit it (Enter), as a user would.
pub fn type_into(h: &mut Harness<'static, Shell>, label: &str, text: &str) {
    control(h, label).focus();
    h.run_steps(1);
    // Select what is there, so the typed text replaces it.
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    control(h, label).type_text(text);
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    settle(h);
}

/// Set a slider (or a number) through accessibility, as assistive technology would.
pub fn set_value(h: &mut Harness<'static, Shell>, label: &str, value: f64) {
    let (target_node, target_tree) = control(h, label).accesskit_node().locate();
    h.event(egui::Event::AccessKitActionRequest(egui::accesskit::ActionRequest {
        action: egui::accesskit::Action::SetValue,
        target_node,
        target_tree,
        data: Some(egui::accesskit::ActionData::NumericValue(value)),
    }));
    settle(h);
}

/// Pick `option` in the select labelled `label` (open it, click the option).
pub fn choose(h: &mut Harness<'static, Shell>, label: &str, option: &str) {
    control(h, label).click();
    let _ = h.try_run();
    // The option is a selectable label in the popup.
    let option_node = h.query_all_by(|n| n.role() == Role::Button && n.label().as_deref() == Some(option)).last().expect("the option");
    option_node.click();
    settle(h);
}

/// Whether the toggle with this label is pressed (`aria-pressed`).
pub fn pressed(h: &Harness<'static, Shell>, label: &str) -> bool {
    control(h, label).accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)
}

/// Whether the control with this label is disabled.
pub fn disabled(h: &Harness<'static, Shell>, label: &str) -> bool {
    control(h, label).accesskit_node().is_disabled()
}

/// Render the window and keep it when `COXETER_SHOT_DIR` is set (the
/// harness must come from [`rendering_harness_of`]).
pub fn shot(h: &mut Harness<'static, Shell>, name: &str) {
    let Ok(dir) = std::env::var("COXETER_SHOT_DIR") else { return };
    let image = h.render().expect("offscreen rendering through wgpu");
    let path = Path::new(&dir).join(format!("{name}.png"));
    image.save(&path).expect("write the screenshot");
}

/// The text of every file saved so far, by name.
pub fn saved(dialogs: &SharedDialogs) -> Vec<(String, Vec<u8>)> {
    dialogs.with(|d| d.saved.clone()).into_iter().map(|p| (p.file_name().unwrap().to_string_lossy().into_owned(), std::fs::read(&p).unwrap_or_default())).collect()
}
