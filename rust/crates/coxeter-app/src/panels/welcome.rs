//! The welcome page (`src/ui/welcome.js`), which opens over the map when the
//! app starts. It says what Coxeter-MFV is, opens one of the paper's three
//! case studies or the user's own model, links the user guide, the source
//! code and the citation, and states the license. It opens at every start
//! until the user turns that off, except when the command line names a
//! scenario or files to open, and the ? button in the header opens it again.
//! It changes neither the map nor the model: an example loads through the
//! same scenario list as the Scenario page.

use egui::{Align, Context, CornerRadius, Frame, Id, Key, Layout, Margin, RichText, Stroke, Ui};

use crate::app::Shell;
use crate::theme::{Palette, bold, size};
use crate::widgets::{self, Btn};

/// The release repository, which holds the source code, the licenses and the citation file.
pub const REPOSITORY_URL: &str = "https://github.com/mathomics/coxeterMFV";

/// The website, whose help page explains the analyses and how to read a solved map.
pub const SITE_URL: &str = "https://mathomics.github.io/coxeterMFV/";

/// The help links, in the order shown: (label, address).
pub const WELCOME_LINKS: [(&str, &str); 3] = [
    ("User guide", "https://mathomics.github.io/coxeterMFV/help.html"),
    ("Source code", "https://github.com/mathomics/coxeterMFV"),
    ("How to cite", "https://github.com/mathomics/coxeterMFV/blob/main/CITATION.cff"),
];

/// One of the paper's case studies, as a value of the scenario list.
pub struct Example {
    pub label: &'static str,
    /// `sc:` names a teaching scenario and `pub:` a published community.
    pub value: &'static str,
    pub title: &'static str,
}

/// The paper's three case studies.
pub const WELCOME_EXAMPLES: [Example; 3] = [
    Example { label: "Yeast9 compartments", value: "sc:yeast9-9-1-1", title: "Yeast9 9.1.1 drawn across six nested compartments" },
    Example {
        label: "Phenylalanine rescue",
        value: "pub:zuniga-2020-phea-rescue",
        title: "S. elongatus rescues an E. coli ΔpheA mutant (Zuñiga et al. 2020)",
    },
    Example { label: "Auxotrophic community", value: "pub:steadycom-ecoli-auxotrophs", title: "Four auxotrophic E. coli strains solved with SteadyCom" },
];

/// The license of the source code.
pub const CODE_LICENSE_URL: &str = "https://github.com/mathomics/coxeterMFV/blob/main/LICENSE";

/// The licenses of what the desktop build bundles (HiGHS and the Rust
/// crates). It has no glpk.js, so unlike the web page it is not GPL-3.0.
pub const THIRD_PARTY_URL: &str = "https://github.com/mathomics/coxeterMFV/blob/main/THIRD_PARTY_NOTICES.md";

/// The page's width inside its margins (`.welcome { width: 520px }` less the padding).
const WIDTH: f32 = 476.0;

/// `welcomeOpensAtStart`: yes unless the user turned it off, or the command
/// line names a scenario or files to open.
pub fn opens_at_start(shown_at_start: bool, opening_something: bool) -> bool {
    shown_at_start && !opening_something
}

/// Draw the page while it is open, and act on what was chosen.
pub fn show(ctx: &Context, shell: &mut Shell) {
    if !shell.ui.welcome_open {
        return;
    }
    let p = shell.palette();
    let frame = Frame::new()
        .fill(p.panel)
        .stroke(Stroke::new(1.0, p.line))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(22));
    let mut at_start = shell.ui.welcome_at_start;
    let mut chosen: Option<&'static str> = None;
    let mut import = false;
    let mut close = false;

    egui::Modal::new(Id::new("welcome")).frame(frame).show(ctx, |ui| {
        ui.set_width(WIDTH);
        ui.spacing_mut().item_spacing.y = 6.0;
        ui.label(RichText::new("Coxeter-MFV").size(18.0).family(bold()).color(p.ink));
        ui.label(RichText::new("Multicompartment metabolic maps for flux analysis of cells and microbial communities.").size(size::BODY).color(p.ink_2));
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 18.0;
            for (label, href) in WELCOME_LINKS {
                ui.hyperlink_to(RichText::new(label).size(size::BODY).color(p.accent), href);
            }
        });

        heading(ui, &p, "Try an example");
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            for example in &WELCOME_EXAMPLES {
                if Btn::new(example.label).title(example.title).show(ui).clicked() {
                    chosen = Some(example.value);
                }
            }
        });

        heading(ui, &p, "Your own model");
        if Btn::new("Import a model").title("SBML, COBRA JSON, MAT or XLSX").show(ui).clicked() {
            import = true;
        }

        ui.add_space(12.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let plain = |s: &str| RichText::new(s).size(size::HINT).color(p.ink_3);
            let link = |s: &str| RichText::new(s).size(size::HINT).color(p.accent);
            ui.label(plain("Free to use, no login. Code under the "));
            ui.hyperlink_to(link("MIT License"), CODE_LICENSE_URL);
            ui.label(plain(". "));
            ui.hyperlink_to(link("Third-party licenses"), THIRD_PARTY_URL);
            ui.label(plain("."));
        });

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let (_, changed) = widgets::checkbox(ui, at_start, "Show at startup", true, None);
            if let Some(checked) = changed {
                at_start = checked;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if Btn::new("Open the editor").primary().show(ui).clicked() {
                    close = true;
                }
            });
        });
    });

    // A <dialog> closes on Escape but not on a click beside it.
    if ctx.input(|i| i.key_pressed(Key::Escape)) {
        close = true;
    }
    if at_start != shell.ui.welcome_at_start {
        shell.ui.welcome_at_start = at_start;
        shell.save_settings();
    }
    if let Some(value) = chosen {
        shell.ui.welcome_open = false;
        shell.queue_scenario_selection(value);
        return;
    }
    if import {
        shell.ui.welcome_open = false;
        shell.import_models();
        return;
    }
    if close {
        shell.ui.welcome_open = false;
    }
}

/// `.welcome h3`: a small capitalized section title.
fn heading(ui: &mut Ui, p: &Palette, text: &str) {
    ui.add_space(10.0);
    ui.label(RichText::new(text.to_uppercase()).size(size::CARD_TITLE).family(bold()).color(p.ink_2));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::ScriptedDialogs;
    use coxeter_state::Store;
    use serde_json::Value;

    fn make_shell() -> Shell {
        let mut store = Store::with_dir(coxeter_core::testkit::repo_root());
        store.init().expect("bundled models");
        Shell::new(store, Box::new(ScriptedDialogs::new()))
    }

    #[test]
    fn the_welcome_page_offers_the_three_case_studies_and_each_is_a_scenario_the_app_can_load() {
        let shell = make_shell();
        let mut values = Vec::new();
        for scenario in &shell.store.scenarios {
            values.push(format!("sc:{}", scenario.get("id").and_then(Value::as_str).unwrap_or_default()));
        }
        for community in &shell.store.published_communities {
            values.push(format!("pub:{}", community.get("id").and_then(Value::as_str).unwrap_or_default()));
        }
        let offered: Vec<&str> = WELCOME_EXAMPLES.iter().map(|e| e.value).collect();
        assert_eq!(offered, ["sc:yeast9-9-1-1", "pub:zuniga-2020-phea-rescue", "pub:steadycom-ecoli-auxotrophs"]);
        for example in &WELCOME_EXAMPLES {
            assert!(values.iter().any(|v| v == example.value), "{} is not in models/scenarios.json", example.value);
        }
    }

    #[test]
    fn the_welcome_page_shows_the_license_and_links_the_user_guide_the_source_code_and_the_citation() {
        let labels: Vec<&str> = WELCOME_LINKS.iter().map(|(label, _)| *label).collect();
        assert_eq!(labels, ["User guide", "Source code", "How to cite"]);
        assert_eq!(WELCOME_LINKS[0].1, format!("{SITE_URL}help.html"));
        assert!(WELCOME_LINKS[1].1.starts_with(REPOSITORY_URL));
        assert!(WELCOME_LINKS[2].1.starts_with(REPOSITORY_URL));
        assert_eq!(CODE_LICENSE_URL, format!("{REPOSITORY_URL}/blob/main/LICENSE"));
        assert_eq!(THIRD_PARTY_URL, format!("{REPOSITORY_URL}/blob/main/THIRD_PARTY_NOTICES.md"));
    }

    #[test]
    fn the_welcome_page_opens_at_start_until_the_user_turns_it_off_and_not_when_the_address_names_a_scenario() {
        assert!(opens_at_start(true, false));
        assert!(!opens_at_start(true, true));
        assert!(!opens_at_start(false, false));

        let path = std::env::temp_dir().join(format!("coxeter-welcome-test-{}", std::process::id())).join("settings.json");
        let mut shell = make_shell();
        shell.settings_path = Some(path.clone());
        shell.ui.welcome_at_start = false;
        shell.save_settings();
        assert!(!Shell::saved_welcome(&path));
        shell.ui.welcome_at_start = true;
        shell.save_settings();
        assert!(Shell::saved_welcome(&path));
        let _ = std::fs::remove_dir_all(path.parent().expect("temporary folder"));
    }

    #[test]
    fn storage_the_browser_refuses_leaves_the_welcome_page_opening_at_start() {
        // The desktop's storage is its settings file: one that is missing or
        // unreadable counts as never turned off.
        let missing = std::env::temp_dir().join("coxeter-welcome-test-missing").join("settings.json");
        assert!(Shell::saved_welcome(&missing));
    }
}
