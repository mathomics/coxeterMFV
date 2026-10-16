//! Shared helpers of the store tests: a store over the repository's files,
//! small models, and the step dispatcher the parity fixtures are replayed
//! with (the JS store methods by name, with JSON arguments).
#![allow(dead_code)]

use std::sync::Arc;
use std::time::Duration;

use coxeter_core::js;
use coxeter_core::mapdata::{Point, Rect};
use coxeter_core::model::Model;
use coxeter_core::testkit::repo_root;
use coxeter_io::model_import::ImportFile;
use coxeter_map::editable_map::{ColorChange, DuplicateOptions};
use coxeter_map::layout_settings::RawLayoutSettings;
use coxeter_map::multi_selection::SelectionRect;
use coxeter_map::place_reaction::PlaceOptions;
use coxeter_render::theme::FluxColorStop;
use coxeter_state::map_edit::GeometryColors;
use coxeter_state::cross_feeding::ExplorerPatch;
use coxeter_state::omics::{CommunityOmicsPatch, OmicsSettingsPatch};
use coxeter_state::{DirAssets, MemoryAssets, Store, StoreError, UserNumber, ViewPatch};
use serde_json::{Value, json};

pub const WAIT: Duration = Duration::from_secs(120);

/// A store over the repository's `models/`, not yet initialised.
pub fn store() -> Store {
    Store::with_dir(repo_root())
}

/// A store over the repository, with the catalogue read (`init()`).
pub fn init_store() -> Store {
    let mut s = store();
    s.init().expect("the bundled catalogue loads");
    s
}

/// A store whose files are held in memory over the repository.
pub fn memory_store() -> (Store, Arc<MemoryAssets>) {
    let assets = Arc::new(MemoryAssets::over(DirAssets::new(repo_root())));
    (Store::new(assets.clone()), assets)
}

pub fn model(doc: Value) -> Model {
    Model::from_value(&doc).expect("a valid test model")
}

pub fn bundled_model(name: &str) -> Model {
    let text = std::fs::read_to_string(repo_root().join("models").join(format!("{name}.json"))).unwrap();
    Model::from_json_str(&text).unwrap()
}

/// A JSON value as a control holds it.
pub fn user_number(v: &Value) -> UserNumber {
    match v {
        Value::Number(n) => UserNumber::Number(n.as_f64().unwrap_or(f64::NAN)),
        Value::String(s) => UserNumber::Text(s.clone()),
        _ => UserNumber::Empty,
    }
}

fn s(v: &Value) -> String {
    js::to_string(v)
}

fn opt_s(v: Option<&Value>) -> Option<String> {
    v.filter(|x| !x.is_null()).map(js::to_string)
}

fn f(v: &Value) -> f64 {
    js::to_number(v)
}

fn rect(v: &Value) -> Option<Rect> {
    v.as_object().map(|_| Rect::new(f(&v["x"]), f(&v["y"]), f(&v["w"]), f(&v["h"])))
}

fn color(v: Option<&Value>) -> ColorChange {
    match v {
        None => ColorChange::Keep,
        Some(Value::Null) => ColorChange::Reset,
        Some(x) => ColorChange::Set(s(x)),
    }
}

/// A `setView` patch from its JSON.
pub fn view_patch(p: &Value) -> ViewPatch {
    let str_of = |k: &str| p.get(k).map(s);
    let num = |k: &str| p.get(k).map(f);
    let flag = |k: &str| p.get(k).map(js::truthy);
    ViewPatch {
        labels: str_of("labels"),
        scroll_input: str_of("scrollInput"),
        flux_color_stops: p.get("fluxColorStops").map(stops),
        reaction_color_mode: str_of("reactionColorMode"),
        width_by_flux: flag("widthByFlux"),
        dash_no_flux: flag("dashNoFlux"),
        line_hops: flag("lineHops"),
        line_color_group: str_of("lineColorGroup"),
        pathway_ribbon_mode: str_of("pathwayRibbonMode"),
        pathway_ribbon_width: num("pathwayRibbonWidth"),
        pathway_ribbon_opacity: num("pathwayRibbonOpacity"),
        show_currency: flag("showCurrency"),
        currency_labels: flag("currencyLabels"),
        show_cross_feeding: flag("showCrossFeeding"),
        cross_feeding_top: num("crossFeedingTop"),
        cross_feeding_shown: None,
        dark: flag("dark"),
        geometry_edit: flag("geometryEdit"),
        multi_select: flag("multiSelect"),
        text_tool: flag("textTool"),
    }
}

/// Flux colour stops from JSON (`null`: automatic).
pub fn stops(v: &Value) -> Option<Vec<FluxColorStop>> {
    v.as_array().map(|list| {
        list.iter()
            .map(|x| FluxColorStop { value: x["value"].as_f64().unwrap_or(f64::NAN), color: x["color"].as_str().unwrap_or("").into() })
            .collect()
    })
}

/// Files a replay keeps between steps (`saveState`, `saveMap`).
#[derive(Default)]
pub struct Harness {
    pub state: String,
    pub map: String,
}

/// Run one step `[method, ...args]` as the fixture script runs it.
pub fn step(store: &mut Store, h: &mut Harness, step: &[Value]) -> Result<(), StoreError> {
    let method = step[0].as_str().expect("a method name");
    let a = |i: usize| step.get(i + 1).cloned().unwrap_or(Value::Null);
    match method {
        "useScenario" => store.use_scenario(&s(&a(0)))?,
        "setMediumLimit" => {
            store.set_medium_limit(&s(&a(0)), user_number(&a(1)))?;
        }
        "setReactionBounds" => {
            store.set_reaction_bounds(&s(&a(0)), user_number(&a(1)), user_number(&a(2)))?;
        }
        "resetReaction" => store.reset_reaction(&s(&a(0)))?,
        "resetAllReactions" => store.reset_all_reactions()?,
        "knockout" => store.knockout(&s(&a(0)))?,
        "solve" => {
            store.solve();
        }
        "setParsimonious" => {
            store.set_parsimonious(js::truthy(&a(0)))?;
        }
        "setPFBABiomassPercent" => {
            store.set_pfba_biomass_percent(user_number(&a(0)))?;
        }
        "setLoopless" => {
            store.set_loopless(js::truthy(&a(0)))?;
        }
        "setObjective" => store.set_objective(&s(&a(0)), opt_s(Some(&a(1))).as_deref())?,
        "setSpeciesEnabled" => store.set_species_enabled(&s(&a(0)), js::truthy(&a(1)))?,
        "setAbundance" => store.set_abundance(&s(&a(0)), user_number(&a(1)))?,
        "setView" => store.set_view(view_patch(&a(0)))?,
        "setFluxColorStops" => {
            store.set_flux_color_stops(stops(&a(0)))?;
        }
        "setCrossFeedingTop" => store.set_cross_feeding_top(f(&a(0))),
        "setLayoutPreset" => store.set_layout_preset(&s(&a(0))),
        "setLayoutSettings" => store.set_layout_settings(coxeter_state::work::raw_layout_settings(&a(0))),
        "showEmptyMap" => {
            store.show_empty_map();
        }
        "showFullMap" => {
            store.show_full_map();
        }
        "showCoreMetabolismOverview" => {
            store.show_core_metabolism_overview(true);
        }
        "relayoutMap" => {
            store.relayout_map("Relayout map");
        }
        "addMapText" => {
            store.add_map_text(&s(&a(0)), f(&a(1)), f(&a(2)), None);
        }
        "moveNodeTx" => {
            let tx = store.begin_map_edit("Move metabolite");
            store.move_node(&s(&a(0)), f(&a(1)), f(&a(2)));
            store.commit_map_edit(tx);
        }
        "moveReactionTx" => {
            let tx = store.begin_map_edit("Move reaction");
            store.move_reaction(&s(&a(0)), f(&a(1)), f(&a(2)));
            store.commit_map_edit(tx);
        }
        "rotateReactionTx" => {
            let tx = store.begin_map_edit("Rotate reaction");
            store.set_reaction_angle(&s(&a(0)), f(&a(1)));
            store.commit_map_edit(tx);
        }
        "moveTextTx" => {
            let tx = store.begin_map_edit("Move text");
            store.move_map_text(&s(&a(0)), f(&a(1)), f(&a(2)));
            store.commit_map_edit(tx);
        }
        "setMapNodePrimary" => {
            store.set_map_node_primary(&s(&a(0)), js::truthy(&a(1)));
        }
        "setMapNodeHidden" => {
            store.set_map_node_hidden(&s(&a(0)), js::truthy(&a(1)));
        }
        "showAllHiddenMetabolites" => {
            store.show_all_hidden_metabolites();
        }
        "duplicateMapMetabolite" => {
            store.duplicate_map_metabolite(&s(&a(0)), &DuplicateOptions::default());
        }
        "setMapGeometryColors" => {
            let colors = a(2);
            let c = GeometryColors { fill: color(colors.get("fill")), stroke: color(colors.get("stroke")) };
            store.set_map_geometry_colors(&s(&a(0)), opt_s(Some(&a(1))).as_deref(), &c);
        }
        "addMapReaction" => {
            let opts = a(1);
            let anchor = opts.get("anchor").map(|p| Point::new(f(&p["x"]), f(&p["y"])));
            store.add_map_reaction(&s(&a(0)), &PlaceOptions { anchor, ..PlaceOptions::default() });
        }
        "removeMapReaction" => {
            store.remove_map_reaction(&s(&a(0)));
        }
        "removeMapNode" => {
            store.remove_map_node(&s(&a(0)));
        }
        "undoMap" => {
            store.undo_map();
        }
        "redoMap" => {
            store.redo_map();
        }
        "setMapLabelSource" => {
            store.set_map_label_source(&s(&a(0)), &s(&a(1)), opt_s(Some(&a(2))).as_deref());
        }
        "selectInRect" => {
            let r = a(0);
            store.select_in_rect(SelectionRect::from_corners(f(&r["x1"]), f(&r["y1"]), f(&r["x2"]), f(&r["y2"])), true);
        }
        "toggleMultiSelection" => store.toggle_multi_selection(&s(&a(0)), &s(&a(1))),
        "clearMultiSelection" => {
            store.clear_multi_selection(true);
        }
        "removeMultiSelection" => {
            store.remove_multi_selection();
        }
        "arrangeMultiSelection" => {
            store.arrange_multi_selection(&s(&a(0)));
        }
        "createManualPathway" => {
            let p = a(0);
            let ids: Option<Vec<String>> = p.get("reactionIds").and_then(Value::as_array).map(|l| l.iter().map(s).collect());
            store.create_manual_pathway(&s(&p["name"]), opt_s(p.get("color")).as_deref(), ids.as_deref())?;
        }
        "setPathwayColor" => {
            store.set_pathway_color(&s(&a(0)), &s(&a(1)))?;
        }
        "setPathwayVisible" => {
            store.set_pathway_visible(&s(&a(0)), js::truthy(&a(1)))?;
        }
        "deleteManualPathway" => {
            store.delete_manual_pathway(&s(&a(0)));
        }
        "select" => store.select(opt_s(Some(&a(0))).as_deref()),
        "saveState" => h.state = store.state_json(),
        "loadSavedState" => {
            let text = h.state.clone();
            store.load_state(&text)?;
            store.wait_for_jobs(WAIT);
        }
        "saveMap" => h.map = store.map_json()?,
        "loadSavedMap" => {
            let text = h.map.clone();
            store.load_map_json(&text)?;
        }
        "loadSavedMapDocument" => {
            let text = h.map.clone();
            store.load_map_document(&text)?;
        }
        "markMapSaved" => store.mark_map_saved(),
        "markScenarioSaved" => store.mark_scenario_saved(),
        "markModelSaved" => store.mark_model_saved(None),
        "addMapCell" => {
            store.add_map_cell(&s(&a(0)), rect(&a(1)));
        }
        "addMapCompartment" => {
            store.add_map_compartment(&s(&a(0)), &s(&a(1)), rect(&a(2)));
        }
        "setMapCanvasSize" => {
            store.set_map_canvas_size(Some(f(&a(0))), Some(f(&a(1))), 0.0, 0.0);
        }
        "importModelFiles" => {
            let files: Vec<ImportFile> = a(0)
                .as_array()
                .unwrap()
                .iter()
                .map(|x| ImportFile {
                    name: s(&x["name"]),
                    text: std::fs::read_to_string(repo_root().join(s(&x["path"]))).unwrap(),
                })
                .collect();
            store.import_model_files(&files)?;
        }
        "usePublishedCommunity" => store.use_published_community(&s(&a(0)))?,
        "importModelsWithGeneRules" => {
            let files: Vec<ImportFile> = a(0)
                .as_array()
                .unwrap()
                .iter()
                .map(|x| ImportFile { name: s(&x["name"]), text: model_with_gene_rules(&s(&x["path"]), &s(&x["id"])) })
                .collect();
            store.import_model_files(&files)?;
        }
        "importOmicsCSV" => {
            let o = a(1);
            let filename = opt_s(o.get("filename")).unwrap_or_else(|| "omics.csv".into());
            let data_type = opt_s(o.get("dataType")).unwrap_or_else(|| "gene".into());
            store.import_omics_csv(&s(&a(0)), &filename, &data_type)?;
        }
        "setOmicsSettings" => {
            let p = a(0);
            let text = |k: &str| p.get(k).map(|v| opt_s(Some(v)));
            store.set_omics_settings(OmicsSettingsPatch {
                enabled: p.get("enabled").map(js::truthy),
                data_type: text("dataType").flatten(),
                mode: text("mode").flatten(),
                condition_a: text("conditionA"),
                condition_b: text("conditionB"),
                metric: text("metric").flatten(),
                pseudocount: p.get("pseudocount").map(f),
            });
        }
        "importMetatranscriptomics" => {
            store.import_metatranscriptomics(&s(&a(0)), &opt_s(Some(&a(1))).unwrap_or_else(|| "metatranscriptomics.csv".into()))?;
        }
        "setCommunityOmics" => store.set_community_omics(CommunityOmicsPatch::from_json(&a(0))),
        "setCrossFeedingExplorer" => {
            let p = a(0);
            store.set_cross_feeding_explorer(ExplorerPatch {
                enabled: p.get("enabled").map(js::truthy),
                query: opt_s(p.get("query")),
                metabolite: opt_s(p.get("metabolite")),
                producer: opt_s(p.get("producer")),
                consumer: opt_s(p.get("consumer")),
                min_flux: p.get("minFlux").map(f),
            });
        }
        "runCommunityGIMME" => {
            store.run_community_gimme(CommunityOmicsPatch::from_json(&a(0)))?;
        }
        "disableGIMME" => {
            store.disable_gimme();
        }
        "rotateMapReaction" => {
            store.rotate_map_reaction(&s(&a(0)), f(&a(1)));
        }
        "setMapReactionLinear" => {
            store.set_map_reaction_linear(&s(&a(0)), js::truthy(&a(1)));
        }
        "flipMapReaction" => {
            store.flip_map_reaction(&s(&a(0)));
        }
        "mirrorMapReaction" => {
            store.mirror_map_reaction(&s(&a(0)));
        }
        "tidyMapReaction" => {
            store.tidy_map_reaction(&s(&a(0)));
        }
        "alignMapReaction" => {
            store.align_map_reaction(&s(&a(0)), &s(&a(1)));
        }
        "alignMapReactionAxis" => {
            store.align_map_reaction_axis(&s(&a(0)), &s(&a(1)));
        }
        "createModelMetabolite" => {
            store.create_model_metabolite(&a(0))?;
        }
        "createModelReaction" => {
            store.create_model_reaction(&a(0), js::truthy(&a(1)))?;
        }
        "setModelEditorSpecies" => {
            store.set_model_editor_species(&s(&a(0)));
        }
        other => panic!("unknown step {other}"),
    }
    Ok(())
}

/// A bundled model with the synthetic gene rules of `modelWithGeneRules` in
/// `rust/tools/fixtures-app.mjs`: reaction i (exchanges excepted) gets
/// G(i mod 23), joined with a second gene by `and` when i is a multiple of 5
/// and by `or` when it is a multiple of 7.
pub fn model_with_gene_rules(path: &str, id: &str) -> String {
    let mut doc: Value = serde_json::from_str(&std::fs::read_to_string(repo_root().join(path)).unwrap()).unwrap();
    doc["id"] = json!(id);
    for (i, r) in doc["reactions"].as_array_mut().unwrap().iter_mut().enumerate() {
        if r["id"].as_str().unwrap_or("").starts_with("EX_") {
            continue;
        }
        let g = |k: usize| format!("G{}", k % 23);
        let rule = if i % 5 == 0 {
            format!("({} and {})", g(i), g(i + 7))
        } else if i % 7 == 0 {
            format!("{} or {}", g(i), g(i + 3))
        } else {
            g(i)
        };
        r["gene_reaction_rule"] = json!(rule);
    }
    doc.to_string()
}

/// A layout-settings patch from its JSON.
pub fn layout_patch(v: &Value) -> RawLayoutSettings {
    coxeter_state::work::raw_layout_settings(v)
}

/// `{ type, id, ...details }` of the selected entity.
pub fn entity_json(store: &Store) -> Value {
    match &store.selected_entity {
        None => Value::Null,
        Some(e) => {
            let mut o = serde_json::Map::new();
            o.insert("type".into(), json!(e.kind));
            o.insert("id".into(), json!(e.id));
            if let Some(c) = &e.cell_id {
                o.insert("cellId".into(), json!(c));
            }
            for (k, v) in &e.details {
                o.entry(k.clone()).or_insert_with(|| v.clone());
            }
            Value::Object(o)
        }
    }
}
