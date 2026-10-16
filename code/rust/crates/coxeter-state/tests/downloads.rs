//! Downloads through the store's `Fetcher`: the official Yeast9 SBML, on the
//! caller's thread (`use_yeast9_scenario`, the JS `await`) and as a
//! background job (`start_yeast9_scenario`). Not a port of a JS test file:
//! the JS tests never download. A fake fetcher answers with the bundled yeast
//! core model written as SBML, so nothing goes to the network.

mod common;

use std::sync::Arc;
use std::time::Duration;

use coxeter_io::sbml_write::{SbmlWriteOptions, to_sbml};
use coxeter_map::yeast9::{YEAST9_MODEL_KEY, YEAST9_SOURCE};
use coxeter_state::{FetchResponse, Fetcher, JobKind, Store, StoreError};

/// Answers every URL with one status and body, and counts the requests.
struct Fake {
    status: u16,
    body: Vec<u8>,
    calls: std::sync::atomic::AtomicUsize,
}

impl Fetcher for Fake {
    fn fetch(&self, url: &str) -> coxeter_state::Result<FetchResponse> {
        assert_eq!(url, YEAST9_SOURCE.url);
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(FetchResponse { status: self.status, body: self.body.clone() })
    }
}

fn yeast_sbml() -> Vec<u8> {
    let model = common::bundled_model("yeast_core_mc");
    let options = SbmlWriteOptions { objective_id: "obj".into(), objective_type: "maximize".into() };
    to_sbml(&model, &options).unwrap().into_bytes()
}

fn store_with(status: u16, body: Vec<u8>) -> (Store, Arc<Fake>) {
    let fake = Arc::new(Fake { status, body, calls: Default::default() });
    let store = common::init_store().with_fetcher(fake.clone());
    (store, fake)
}

#[test]
fn yeast9_downloads_through_the_fetcher_and_opens_without_solving() {
    let (mut store, fake) = store_with(200, yeast_sbml());
    let report = store.import_yeast9().unwrap();
    assert_eq!(fake.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(store.species.len(), 1);
    assert_eq!(store.species[0].file, YEAST9_MODEL_KEY);
    assert!(store.solution.is_none(), "the genome-scale LP is not solved on arrival");
    assert!(store.status().message.starts_with("Loaded Yeast9 9.1.1:"), "{}", store.status().message);
    assert!(report.preview.is_some());
    let unsaved = store.unsaved_work();
    assert!(!unsaved.map && !unsaved.scenario, "an open reads Saved: {unsaved:?}");
}

#[test]
fn yeast9_in_the_background_lands_as_the_direct_download_does() {
    let (mut direct, _) = store_with(200, yeast_sbml());
    direct.import_yeast9().unwrap();

    let (mut store, _) = store_with(200, yeast_sbml());
    let scenario = store.yeast9_scenario();
    store.start_yeast9_scenario(scenario);
    assert_eq!(store.status().state.as_str(), "busy");
    assert!(store.status().message.starts_with("Downloading official Yeast9"));
    let outcomes = store.wait_for_kind(JobKind::Download, Duration::from_secs(120));
    assert!(outcomes.iter().any(|o| o.kind == JobKind::Download && o.result.is_ok()), "{outcomes:?}");
    assert_eq!(store.species[0].file, YEAST9_MODEL_KEY);
    assert_eq!(store.status().message, direct.status().message);
    assert_eq!(store.map.as_ref().unwrap().to_json(), direct.map.as_ref().unwrap().to_json());
}

#[test]
fn a_failed_yeast9_download_says_what_to_do_instead() {
    let (mut store, _) = store_with(404, Vec::new());
    let before = store.scenario.clone();
    let error = store.import_yeast9().unwrap_err();
    assert_eq!(
        error,
        StoreError::msg(
            "Yeast9 9.1.1 download failed (HTTP 404). You can still download yeast-GEM.xml manually and use Load model(s)."
        )
    );
    assert_eq!(store.scenario, before, "nothing was opened");

    let scenario = store.yeast9_scenario();
    store.start_yeast9_scenario(scenario);
    let outcomes = store.wait_for_kind(JobKind::Download, Duration::from_secs(60));
    let outcome = outcomes.iter().find(|o| o.kind == JobKind::Download).unwrap();
    assert_eq!(outcome.result.as_ref().unwrap_err(), &error);
    assert_eq!(store.status().state.as_str(), "error");
    assert_eq!(store.status().message, error.to_string());
}
