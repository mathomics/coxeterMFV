/**
 * Versioned on-disk document for Escher MCMC maps.
 *
 * A map is saved as its EditableMap payload inside a stable envelope that
 * records which active biological context it belongs to. Raw EditableMap JSON,
 * as written by V0.6/V0.7, is still read for backward compatibility.
 */

import { normalizeAbsoluteFluxColors, normalizeFluxColorStops } from '../render/theme.js';

export const MAP_DOCUMENT_FORMAT = 'EscherMCMCMap';
export const MAP_DOCUMENT_VERSION = 3;

/**
 * Wrap a map in the versioned envelope, ready for JSON.stringify.
 *
 * @param {import('./editable-map.js').EditableMap} map
 * @param {object} [context] `species` (enabled ones are recorded with their
 *   model reference), `scenario`, `pathwayManager`, `fluxColorStops`,
 *   `absoluteFluxColors`, `study`
 * @returns {object} the document
 */
export function createMapDocument(map, context = {}) {
  if (!map) throw new Error('No map to serialise.');

  const species = (context.species ?? [])
    .filter((sp) => sp.enabled !== false)
    .map((sp) => ({
      id: sp.id,
      model: sp.file ?? sp.model ?? null,
    }))
    .sort((a, b) => a.id.localeCompare(b.id));

  return {
    format: MAP_DOCUMENT_FORMAT,
    version: MAP_DOCUMENT_VERSION,
    metadata: {
      createdAt: new Date().toISOString(),
      scenario: context.scenario ?? null,
      species,
      reactionCount: map.reactions?.size ?? 0,
      nodeCount: map.nodes?.size ?? 0,
      segmentCount: map.segments?.length ?? 0,
      textCount: map.texts?.size ?? 0,
    },
    pathwayManager: context.pathwayManager ?? null,
    fluxColorStops: normalizeFluxColorStops(context.fluxColorStops ?? null),
    absoluteFluxColors: normalizeAbsoluteFluxColors(context.absoluteFluxColors),
    ...(context.study ? { study: JSON.parse(JSON.stringify(context.study)) } : {}),
    map: map.toJSON(),
  };
}

/**
 * Read a map document from a JSON string or an already parsed object.
 * Accepts the envelope (up to MAP_DOCUMENT_VERSION) and raw legacy exports,
 * which come back with `legacy: true`. Throws a readable Error otherwise.
 *
 * @returns {{format, version, metadata, pathwayManager, fluxColorStops,
 *   absoluteFluxColors, map, legacy: boolean, study?: object|null}}
 */
export function parseMapDocument(input) {
  const doc = typeof input === 'string' ? JSON.parse(input) : input;
  if (!doc || typeof doc !== 'object') {
    throw new Error('The file does not contain a valid JSON document.');
  }

  // Backward compatibility with V0.6/V0.7 raw EditableMap exports.
  if (Array.isArray(doc.reactions) && Array.isArray(doc.nodes) && Array.isArray(doc.segments)) {
    return {
      format: 'EditableMap',
      version: Number(doc.version ?? 1),
      metadata: null,
      pathwayManager: null,
      fluxColorStops: normalizeFluxColorStops(doc.fluxColorStops ?? null),
      absoluteFluxColors: normalizeAbsoluteFluxColors(null),
      map: doc,
      legacy: true,
    };
  }

  if (doc.format !== MAP_DOCUMENT_FORMAT) {
    // The commonest wrong file here is a COBRA model, because both are "the
    // JSON" the app offers to download. Say which door it goes through.
    if (Array.isArray(doc.reactions) && Array.isArray(doc.metabolites)) {
      throw new Error('This is a metabolic model, not a map. Load it from Edit Model ▸ Import model.');
    }
    throw new Error('The file is not a recognised Escher MCMC map.');
  }
  if (!Number.isInteger(doc.version) || doc.version < 1) {
    throw new Error('The map document declares no valid version.');
  }
  if (doc.version > MAP_DOCUMENT_VERSION) {
    throw new Error(
      `This map uses version ${doc.version}, but this application only understands up to version ${MAP_DOCUMENT_VERSION}.`,
    );
  }
  if (!doc.map || typeof doc.map !== 'object') {
    throw new Error('The document has no map section.');
  }

  return {
    format: doc.format,
    version: doc.version,
    metadata: doc.metadata ?? null,
    pathwayManager: doc.pathwayManager ?? null,
    fluxColorStops: normalizeFluxColorStops(doc.fluxColorStops ?? null),
    absoluteFluxColors: normalizeAbsoluteFluxColors(doc.absoluteFluxColors),
    study: doc.study ?? null,
    map: doc.map,
    legacy: false,
  };
}

/**
 * Reduce a model reference to the thing it identifies.
 *
 * A published community's member is served from two files: the full model and
 * the map excerpt. Which one the runtime loads depends on the build (the
 * single-file one embeds only the excerpts), so the same drawing of the same
 * community records `preview-Ec1` when it is saved there and `model-Ec1` when
 * it is saved from a server. They are the same member, and a map saved in one
 * has to open in the other.
 */
function modelIdentity(reference) {
  if (!reference) return null;
  const published = /^(published\/[^/]+\/)(?:model|preview)-(.+)$/.exec(String(reference));
  return published ? `${published[1]}${published[2]}` : String(reference);
}

/**
 * Check that a parsed map was saved for the active species and models.
 * Maps without species metadata pass. Throws an Error on a mismatch.
 *
 * @param {object} parsed   result of parseMapDocument
 * @param {object} [current] `{species}` as passed to createMapDocument
 * @returns {true}
 */
export function validateMapContext(parsed, current = {}) {
  const metadata = parsed?.metadata;
  if (!metadata?.species?.length) return true;

  const expected = (current.species ?? [])
    .filter((sp) => sp.enabled !== false)
    .map((sp) => ({ id: sp.id, model: modelIdentity(sp.file ?? sp.model) }))
    .sort((a, b) => a.id.localeCompare(b.id));

  const incoming = [...metadata.species]
    .map((sp) => ({ id: sp.id, model: modelIdentity(sp.model) }))
    .sort((a, b) => a.id.localeCompare(b.id));

  if (expected.length !== incoming.length) {
    throw new Error('The map was saved with a different number of active species.');
  }

  for (let i = 0; i < expected.length; i += 1) {
    if (expected[i].id !== incoming[i].id) {
      throw new Error('This map was saved for a different set of species.');
    }
    // An uploaded model is known only by the name of the file it came from, and
    // a file gets renamed, for instance every time a model is re-exported under
    // a new version. That is not a different model. Whether the map fits is
    // settled by what it draws: a reaction or metabolite the active model does
    // not have is refused, by name, when the map is applied.
    if (isUploadReference(expected[i].model) && isUploadReference(incoming[i].model)) continue;
    if (expected[i].model && incoming[i].model && expected[i].model !== incoming[i].model) {
      throw new Error(
        `Species ${expected[i].id} uses a different model in the map file.`,
      );
    }
  }

  return true;
}

function isUploadReference(reference) {
  return typeof reference === 'string' && reference.startsWith('upload:');
}

/** The species a map was drawn over: from its envelope, or from its cells. */
export function mapSpeciesIds(parsed) {
  const listed = (parsed?.metadata?.species ?? []).map((sp) => sp.id).filter(Boolean);
  if (listed.length) return [...new Set(listed)];
  return [...new Set((parsed?.map?.cells ?? []).map((cell) => cell.id).filter(Boolean))];
}

/*
 * Fields that are prose rather than identifiers. Everything else in a map that
 * is a string is, or contains, an id.
 */
const PROSE_FIELDS = new Set([
  'name', 'label', 'text', 'formula', 'organism', 'note', 'subsystem', 'color',
  'fill', 'stroke', 'kind', 'membrane', 'side', 'visualRole', 'transportMode',
  'fontFamily',
]);

/**
 * Read a map drawn over one species as a map of another.
 *
 * The species id is in front of every identifier a map holds (`iDemo@PGI`,
 * `met:iDemo@PGI:iDemo@g6p_c`, the boundary `iDemo/h`), and for an uploaded model
 * that id is whatever the file declares as its model id. Export the next
 * version of a model under a new id and a drawing that took days no longer
 * names anything. The drawing is still good: the reactions are the same
 * reactions. This rewrites the id wherever it stands in front of something,
 * and leaves names, labels and free text alone.
 *
 * It does not decide whether the result fits. Applying the map still checks
 * every reaction and metabolite against the active model.
 *
 * Returns a new parsed document (the input is not changed). Its metadata lists
 * the single species `toId` with the given `model` reference.
 */
export function renameMapSpecies(parsed, fromId, toId, { model = null } = {}) {
  if (!parsed?.map || !fromId || !toId || fromId === toId) return parsed;
  const escaped = fromId.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  // The id, where it starts an identifier: at the very start, or after a
  // character that cannot be part of one.
  const inFront = new RegExp(`(^|[^A-Za-z0-9_.-])${escaped}(?=[@/])`, 'g');
  const rename = (value) => (value === fromId ? toId : value.replace(inFront, `$1${toId}`));

  const walk = (value, key) => {
    if (typeof value === 'string') return PROSE_FIELDS.has(key) ? value : rename(value);
    if (Array.isArray(value)) return value.map((item) => walk(item, key));
    if (value && typeof value === 'object') {
      const out = {};
      for (const [k, v] of Object.entries(value)) out[k] = walk(v, k);
      return out;
    }
    return value;
  };

  const map = { ...walk({ ...parsed.map, texts: undefined }, null), texts: parsed.map.texts ?? [] };
  return {
    ...parsed,
    metadata: parsed.metadata
      ? { ...parsed.metadata, species: [{ id: toId, model }] }
      : parsed.metadata,
    pathwayManager: parsed.pathwayManager ? walk(parsed.pathwayManager, null) : parsed.pathwayManager,
    map,
  };
}
