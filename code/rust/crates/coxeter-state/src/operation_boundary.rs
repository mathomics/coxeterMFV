//! UX/domain boundary between drawing edits and biological-model edits
//! (port of `src/ui/operation-boundary.js`).
//!
//! BUILD operations belong to MAP: they may change only the visual document
//! (geometry, visibility, instances, annotations and map history).
//!
//! MODEL operations belong to MODEL: they may change the biological model and
//! therefore may require rebuilding the community/LP.
//!
//! Keep this registry explicit. New public editing operations must be assigned
//! to one domain before they are exposed in the UI. The names are the
//! JavaScript method names, which the Rust store keeps in snake_case.

use crate::error::{Result, StoreError};

/// `OPERATION_DOMAIN`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OperationDomain {
    Map,
    Model,
}

impl OperationDomain {
    pub fn as_str(self) -> &'static str {
        match self {
            OperationDomain::Map => "map",
            OperationDomain::Model => "model",
        }
    }
}

/// The MAP (BUILD) operations, by JavaScript name.
pub const MAP_OPERATIONS: [&str; 50] = [
    "beginMapEdit",
    "commitMapEdit",
    "undoMap",
    "redoMap",
    "moveMultiSelection",
    "moveCompartmentLabel",
    "setMapLabelSource",
    "arrangeMultiSelection",
    "spaceMultiSelectionEvenly",
    "rotateMultiSelection",
    "pointMultiSelection",
    "removeMultiSelection",
    "setMap",
    "addMapReaction",
    "addMapMetabolite",
    "removeMapReaction",
    "showEmptyMap",
    "showCoreMetabolismOverview",
    "removeCoreOverviewPathways",
    "addMapCell",
    "addMapCompartment",
    "removeMapCompartment",
    "setMapCanvasSize",
    "setLayoutPreset",
    "setLayoutSettings",
    "relayoutMap",
    "moveNode",
    "mergeOverlappingMetabolite",
    "moveReaction",
    "moveBranchControl",
    "rotateReaction",
    "setReactionAngle",
    "rotateMapReaction",
    "setMapReactionLinear",
    "setMapReactionLineEnd",
    "alignMapReaction",
    "tidyMapReaction",
    "swapMapReactionMetabolites",
    "flipMapReaction",
    "mirrorMapReaction",
    "addMapText",
    "updateMapText",
    "moveMapText",
    "removeMapText",
    "duplicateMapMetabolite",
    "setMetaboliteLabelAngle",
    "setReactionLabelOffset",
    "resetReactionLabel",
    "setMapNodePrimary",
    "removeMapNode",
];

/// The MODEL operations, by JavaScript name.
pub const MODEL_OPERATIONS: [&str; 2] = ["createModelMetabolite", "createModelReaction"];

/// `operationDomain(name)`: the domain of a JavaScript-named operation.
pub fn operation_domain(name: &str) -> Option<OperationDomain> {
    if MAP_OPERATIONS.contains(&name) {
        Some(OperationDomain::Map)
    } else if MODEL_OPERATIONS.contains(&name) {
        Some(OperationDomain::Model)
    } else {
        None
    }
}

/// Whether a JavaScript-named operation only changes the drawing.
pub fn is_map_operation(name: &str) -> bool {
    operation_domain(name) == Some(OperationDomain::Map)
}

/// Whether a JavaScript-named operation changes the biological model.
pub fn is_model_operation(name: &str) -> bool {
    operation_domain(name) == Some(OperationDomain::Model)
}

/// `assertOperationDomain(name, expected)`.
pub fn assert_operation_domain(name: &str, expected: OperationDomain) -> Result<bool> {
    let Some(actual) = operation_domain(name) else {
        return Err(StoreError::msg(format!("Editing operation \"{name}\" has no MAP/MODEL domain.")));
    };
    if actual != expected {
        return Err(StoreError::msg(format!(
            "Editing operation \"{name}\" belongs to {}, not {}.",
            actual.as_str().to_uppercase(),
            expected.as_str().to_uppercase()
        )));
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domains_are_explicit() {
        assert_eq!(operation_domain("setMetaboliteLabelAngle"), Some(OperationDomain::Map));
        assert_eq!(operation_domain("createModelReaction"), Some(OperationDomain::Model));
        assert_eq!(operation_domain("solve"), None);
        assert!(assert_operation_domain("moveNode", OperationDomain::Map).unwrap());
        let wrong = assert_operation_domain("createModelMetabolite", OperationDomain::Map).unwrap_err();
        assert_eq!(wrong.to_string(), "Editing operation \"createModelMetabolite\" belongs to MODEL, not MAP.");
        let none = assert_operation_domain("nope", OperationDomain::Map).unwrap_err();
        assert_eq!(none.to_string(), "Editing operation \"nope\" has no MAP/MODEL domain.");
    }
}
