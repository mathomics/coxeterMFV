//! The rule the parity tests compare maps by: exact, except that `null` and
//! an absent field are the same, as they are to every reader of a map, and
//! that `false` is the same as absent for the two segment flags JavaScript
//! only ever writes as `true`. This checks the rule still sees every real
//! difference.

mod common;

use common::*;
use serde_json::json;

#[test]
fn map_diff_is_strict_about_everything_but_nulls() {
    let a = json!({ "x": 1.0, "n": null, "seg": { "backboneAnchor": false, "to": { "x": 2 } }, "list": [1, 2] });
    assert_eq!(map_diff(&a, &json!({ "x": 1, "seg": { "to": { "x": 2.0 }, "b1": null }, "list": [1, 2] }), 0.0), None);
    assert!(map_diff(&a, &json!({ "x": 1.0000000000000002, "seg": { "to": { "x": 2 } }, "list": [1, 2] }), 0.0).is_some());
    assert!(map_diff(&a, &json!({ "x": 1, "seg": { "to": { "x": 2 } }, "list": [1, 2, 3] }), 0.0).is_some());
    assert!(map_diff(&a, &json!({ "x": 1, "seg": { "to": { "x": 2 }, "extra": 0 }, "list": [1, 2] }), 0.0).is_some());
    assert!(map_diff(&a, &json!({ "seg": { "to": { "x": 2 } }, "list": [1, 2] }), 0.0).is_some());
    assert!(map_diff(&a, &json!({ "x": 1, "seg": { "backboneAnchor": true, "to": { "x": 2 } }, "list": [1, 2] }), 0.0).is_some());
    assert!(map_diff(&json!({ "a": false }), &json!({}), 0.0).is_some());
    assert!(map_diff(&json!({ "a": "1" }), &json!({ "a": 1 }), 0.0).is_some());
    assert_eq!(canonical(&a), json!({ "x": 1.0, "seg": { "to": { "x": 2 } }, "list": [1, 2] }));
}
