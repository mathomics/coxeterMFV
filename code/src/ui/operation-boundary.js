/**
 * UX/domain boundary between drawing edits and biological-model edits.
 *
 * BUILD operations belong to MAP: they may change only the visual document
 * (geometry, visibility, instances, annotations and map history).
 *
 * MODEL operations belong to MODEL: they may change the biological model and
 * therefore may require rebuilding the community/LP.
 *
 * Keep this registry explicit. New public editing operations must be assigned
 * to one domain before they are exposed in the UI.
 */

export const OPERATION_DOMAIN = Object.freeze({
  MAP: 'map',
  MODEL: 'model',
});

const MAP_OPERATIONS = [
  'beginMapEdit',
  'commitMapEdit',
  'undoMap',
  'redoMap',
  'moveMultiSelection',
  'moveCompartmentLabel',
  'setMapLabelSource',
  'arrangeMultiSelection',
  'spaceMultiSelectionEvenly',
  'rotateMultiSelection',
  'pointMultiSelection',
  'removeMultiSelection',
  'setMap',
  'addMapReaction',
  'addMapMetabolite',
  'removeMapReaction',
  'showEmptyMap',
  'showCoreMetabolismOverview',
  'removeCoreOverviewPathways',
  'addMapCell',
  'addMapCompartment',
  'removeMapCompartment',
  'setMapCanvasSize',
  'setLayoutPreset',
  'setLayoutSettings',
  'relayoutMap',
  'moveNode',
  'mergeOverlappingMetabolite',
  'moveReaction',
  'moveBranchControl',
  'rotateReaction',
  'setReactionAngle',
  'rotateMapReaction',
  'setMapReactionLinear',
  'setMapReactionLineEnd',
  'alignMapReaction',
  'tidyMapReaction',
  'swapMapReactionMetabolites',
  'flipMapReaction',
  'mirrorMapReaction',
  'addMapText',
  'updateMapText',
  'moveMapText',
  'removeMapText',
  'duplicateMapMetabolite',
  'setMetaboliteLabelAngle',
  'setReactionLabelOffset',
  'resetReactionLabel',
  'setMapNodePrimary',
  'removeMapNode',
];

const MODEL_OPERATIONS = [
  'createModelMetabolite',
  'createModelReaction',
];

export const EDIT_OPERATION_DOMAIN = Object.freeze({
  ...Object.fromEntries(MAP_OPERATIONS.map((name) => [name, OPERATION_DOMAIN.MAP])),
  ...Object.fromEntries(MODEL_OPERATIONS.map((name) => [name, OPERATION_DOMAIN.MODEL])),
});

/** The domain ('map' or 'model') of a Store editing method by name, or null when unregistered. */
export function operationDomain(name) {
  return EDIT_OPERATION_DOMAIN[name] ?? null;
}

export function isMapOperation(name) {
  return operationDomain(name) === OPERATION_DOMAIN.MAP;
}

export function isModelOperation(name) {
  return operationDomain(name) === OPERATION_DOMAIN.MODEL;
}

/** Throw unless operation `name` is registered under the `expected` domain. Returns true. */
export function assertOperationDomain(name, expected) {
  const actual = operationDomain(name);
  if (!actual) throw new Error(`Editing operation "${name}" has no MAP/MODEL domain.`);
  if (actual !== expected) {
    throw new Error(`Editing operation "${name}" belongs to ${actual.toUpperCase()}, not ${expected.toUpperCase()}.`);
  }
  return true;
}
