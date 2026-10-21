//! Versioned on-disk document for Escher MCMC maps (port of
//! `src/map/map-document.js`).
//!
//! V0.6 saved raw EditableMap JSON. V0.8 wraps the same graphical payload in
//! a stable envelope that records which active biological context it belongs
//! to. Raw V0.6/V0.7 documents remain readable for backward compatibility.

use std::collections::HashSet;

use coxeter_core::js;
use coxeter_core::mapdata::{MAP_DOCUMENT_FORMAT, MAP_DOCUMENT_VERSION};
use serde_json::{Map, Value, json};

use crate::editable_map::EditableMap;

/// Errors a user can cause by opening the wrong file.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum MapDocumentError {
    #[error("{0}")]
    Invalid(String),
}

fn err<T>(msg: impl Into<String>) -> Result<T, MapDocumentError> {
    Err(MapDocumentError::Invalid(msg.into()))
}

/// One point of a user-defined flux colour scale.
#[derive(Debug, Clone, PartialEq)]
pub struct FluxColorStop {
    pub value: f64,
    pub color: String,
}

/// `normalizeFluxColorStops(stops)` (from `src/render/theme.js`): validate and
/// copy a user-defined colour scale keyed by signed flux values. `null` is
/// the automatic palette.
pub fn normalize_flux_color_stops(stops: &Value) -> Result<Option<Vec<FluxColorStop>>, MapDocumentError> {
    if stops.is_null() {
        return Ok(None);
    }
    let Some(list) = stops.as_array().filter(|l| !l.is_empty()) else {
        return err("Flux colours need at least one point, or null for the automatic palette.");
    };
    let mut seen: Vec<f64> = Vec::new();
    let mut out = Vec::new();
    for (i, stop) in list.iter().enumerate() {
        let value = match stop.get("value") {
            Some(Value::Number(n)) => n.as_f64().filter(|v| v.is_finite()),
            _ => None,
        };
        let Some(value) = value.filter(|_| js::truthy(stop)) else {
            return err(format!("Flux colour point {} needs a finite numeric value.", i + 1));
        };
        let color = stop.get("color").and_then(Value::as_str).map(|c| c.trim().to_lowercase()).unwrap_or_default();
        let ok = color.len() == 7 && color.starts_with('#') && color[1..].bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if !ok {
            return err(format!("Flux colour point {} needs a colour in #rrggbb format.", i + 1));
        }
        let value = if value == 0.0 { 0.0 } else { value };
        if seen.contains(&value) {
            return err(format!("Flux colour values must be unique; {} appears more than once.", js::number_to_string(value)));
        }
        seen.push(value);
        out.push(FluxColorStop { value, color });
    }
    out.sort_by(|a, b| a.value.partial_cmp(&b.value).unwrap_or(std::cmp::Ordering::Equal));
    Ok(Some(out))
}

fn stops_value(stops: &Option<Vec<FluxColorStop>>) -> Value {
    match stops {
        None => Value::Null,
        Some(list) => Value::Array(list.iter().map(|s| json!({ "value": s.value, "color": s.color })).collect()),
    }
}

/// One species of the active context: `{id, file?, model?, enabled?}`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ContextSpecies {
    pub id: String,
    pub file: Option<String>,
    pub model: Option<String>,
    pub enabled: Option<bool>,
}

/// The biological context a map is saved with (`createMapDocument`'s
/// `context`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MapContext {
    pub species: Vec<ContextSpecies>,
    pub scenario: Option<String>,
    pub pathway_manager: Option<Value>,
    pub flux_color_stops: Option<Value>,
    pub study: Option<Value>,
}

/// The current time as `Date#toISOString` prints it.
pub fn iso_timestamp_now() -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    iso_timestamp(now.as_millis() as i64)
}

/// Milliseconds since the epoch as `YYYY-MM-DDTHH:MM:SS.mmmZ`.
pub fn iso_timestamp(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let rem = ms.rem_euclid(86_400_000);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z",
        rem / 3_600_000,
        rem / 60_000 % 60,
        rem / 1000 % 60,
        rem % 1000
    )
}

fn context_species(context: &MapContext, model: impl Fn(&ContextSpecies) -> Value) -> Vec<(String, Value)> {
    let mut species: Vec<(String, Value)> = context
        .species
        .iter()
        .filter(|sp| sp.enabled != Some(false))
        .map(|sp| (sp.id.clone(), model(sp)))
        .collect();
    species.sort_by(|a, b| js::locale_compare(&a.0, &b.0));
    species
}

/// `createMapDocument(map, context)`: the envelope a map is saved in.
pub fn create_map_document(map: &EditableMap, context: &MapContext) -> Result<Value, MapDocumentError> {
    create_map_document_at(map, context, &iso_timestamp_now())
}

/// [`create_map_document`] with the creation time given.
pub fn create_map_document_at(map: &EditableMap, context: &MapContext, created_at: &str) -> Result<Value, MapDocumentError> {
    let species: Vec<Value> = context_species(context, |sp| {
        sp.file.clone().or_else(|| sp.model.clone()).map(Value::String).unwrap_or(Value::Null)
    })
    .into_iter()
    .map(|(id, model)| json!({ "id": id, "model": model }))
    .collect();
    let stops = normalize_flux_color_stops(context.flux_color_stops.as_ref().unwrap_or(&Value::Null))?;
    let mut out = Map::new();
    out.insert("format".into(), json!(MAP_DOCUMENT_FORMAT));
    out.insert("version".into(), json!(MAP_DOCUMENT_VERSION));
    out.insert(
        "metadata".into(),
        json!({
            "createdAt": created_at,
            "scenario": context.scenario,
            "species": species,
            "reactionCount": map.reactions().len(),
            "nodeCount": map.nodes().len(),
            "segmentCount": map.segments().len(),
            "textCount": map.texts().len(),
        }),
    );
    out.insert("pathwayManager".into(), context.pathway_manager.clone().unwrap_or(Value::Null));
    out.insert("fluxColorStops".into(), stops_value(&stops));
    if let Some(study) = context.study.as_ref().filter(|s| js::truthy(s)) {
        out.insert("study".into(), study.clone());
    }
    out.insert("map".into(), map.to_json());
    Ok(Value::Object(out))
}

/// What `parseMapDocument` reads out of a file.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedMapDocument {
    pub format: String,
    pub version: f64,
    pub metadata: Option<Value>,
    pub pathway_manager: Option<Value>,
    pub flux_color_stops: Option<Vec<FluxColorStop>>,
    /// Absent for a legacy document (the JavaScript object has no such key).
    pub study: Option<Value>,
    pub map: Value,
    pub legacy: bool,
}

impl ParsedMapDocument {
    /// The JSON the JavaScript object would print.
    pub fn to_json(&self) -> Value {
        let mut out = Map::new();
        out.insert("format".into(), json!(self.format));
        out.insert("version".into(), crate::util::json_num(self.version));
        out.insert("metadata".into(), self.metadata.clone().unwrap_or(Value::Null));
        out.insert("pathwayManager".into(), self.pathway_manager.clone().unwrap_or(Value::Null));
        out.insert("fluxColorStops".into(), stops_value(&self.flux_color_stops));
        if !self.legacy {
            out.insert("study".into(), self.study.clone().unwrap_or(Value::Null));
        }
        out.insert("map".into(), self.map.clone());
        out.insert("legacy".into(), json!(self.legacy));
        Value::Object(out)
    }

    /// Build the editable map it holds.
    pub fn to_map(&self) -> Result<EditableMap, crate::editable_map::MapError> {
        EditableMap::from_json(&self.map)
    }
}

fn non_null(v: Option<&Value>) -> Option<Value> {
    v.filter(|v| !v.is_null()).cloned()
}

/// `parseMapDocument(text)`.
pub fn parse_map_document_str(text: &str) -> Result<ParsedMapDocument, MapDocumentError> {
    let doc: Value = serde_json::from_str(text).map_err(|e| MapDocumentError::Invalid(format!("The file is not valid JSON: {e}")))?;
    parse_map_document(&doc)
}

/// `parseMapDocument(doc)`: read a saved map, current or legacy, or say
/// clearly why the file is not one.
pub fn parse_map_document(doc: &Value) -> Result<ParsedMapDocument, MapDocumentError> {
    if !matches!(doc, Value::Object(_) | Value::Array(_)) {
        return err("The file does not contain a valid JSON document.");
    }
    let is_array = |k: &str| doc.get(k).is_some_and(Value::is_array);
    // Backward compatibility with V0.6/V0.7 raw EditableMap exports.
    if is_array("reactions") && is_array("nodes") && is_array("segments") {
        let version = doc.get("version").filter(|v| !v.is_null()).map(js::to_number).unwrap_or(1.0);
        return Ok(ParsedMapDocument {
            format: "EditableMap".into(),
            version,
            metadata: None,
            pathway_manager: None,
            flux_color_stops: normalize_flux_color_stops(doc.get("fluxColorStops").unwrap_or(&Value::Null))?,
            study: None,
            map: doc.clone(),
            legacy: true,
        });
    }
    if doc.get("format").and_then(Value::as_str) != Some(MAP_DOCUMENT_FORMAT) {
        // The commonest wrong file here is a COBRA model, because both are
        // "the JSON" the app offers to download. Say which door it goes
        // through.
        if is_array("reactions") && is_array("metabolites") {
            return err("This is a metabolic model, not a map. Load it from Edit Model ▸ Import model.");
        }
        return err("The file is not a recognised Escher MCMC map.");
    }
    let version = doc.get("version").and_then(Value::as_f64).filter(|v| v.fract() == 0.0 && v.is_finite());
    let Some(version) = version.filter(|v| *v >= 1.0) else {
        return err("The map document declares no valid version.");
    };
    if version > MAP_DOCUMENT_VERSION as f64 {
        return err(format!(
            "This map uses version {}, but this application only understands up to version {MAP_DOCUMENT_VERSION}.",
            js::number_to_string(version)
        ));
    }
    let Some(map) = doc.get("map").filter(|m| matches!(m, Value::Object(_) | Value::Array(_))) else {
        return err("The document has no map section.");
    };
    Ok(ParsedMapDocument {
        format: MAP_DOCUMENT_FORMAT.into(),
        version,
        metadata: non_null(doc.get("metadata")),
        pathway_manager: non_null(doc.get("pathwayManager")),
        flux_color_stops: normalize_flux_color_stops(doc.get("fluxColorStops").unwrap_or(&Value::Null))?,
        study: non_null(doc.get("study")),
        map: map.clone(),
        legacy: false,
    })
}

/// `modelIdentity`: reduce a model reference to the thing it identifies. A
/// published community's member is served from two files (the full model
/// and the map excerpt); a map saved in one has to open in the other.
fn model_identity(reference: Option<&str>) -> Option<String> {
    let r = reference.filter(|r| !r.is_empty())?;
    if let Some(rest) = r.strip_prefix("published/") {
        if let Some(slash) = rest.find('/') {
            let (group, tail) = rest.split_at(slash);
            let tail = &tail[1..];
            for kind in ["model-", "preview-"] {
                if let Some(name) = tail.strip_prefix(kind) {
                    if !name.is_empty() {
                        return Some(format!("published/{group}/{name}"));
                    }
                }
            }
        }
    }
    Some(r.to_string())
}

fn is_upload_reference(r: Option<&str>) -> bool {
    r.is_some_and(|r| r.starts_with("upload:"))
}

/// `validateMapContext(parsed, current)`: refuse a map saved for a different
/// set of species, or for a different model of one of them.
pub fn validate_map_context(parsed: &ParsedMapDocument, current: &MapContext) -> Result<bool, MapDocumentError> {
    let Some(listed) = parsed.metadata.as_ref().and_then(|m| m.get("species")).and_then(Value::as_array).filter(|l| !l.is_empty())
    else {
        return Ok(true);
    };
    let expected = context_species(current, |sp| {
        model_identity(sp.file.as_deref().or(sp.model.as_deref())).map(Value::String).unwrap_or(Value::Null)
    });
    let mut incoming: Vec<(String, Value)> = listed
        .iter()
        .map(|sp| {
            let id = sp.get("id").map(js::to_string).unwrap_or_default();
            let model = sp.get("model").filter(|m| js::truthy(m)).map(js::to_string);
            (id, model_identity(model.as_deref()).map(Value::String).unwrap_or(Value::Null))
        })
        .collect();
    incoming.sort_by(|a, b| js::locale_compare(&a.0, &b.0));
    if expected.len() != incoming.len() {
        return err("The map was saved with a different number of active species.");
    }
    for (e, i) in expected.iter().zip(&incoming) {
        if e.0 != i.0 {
            return err("This map was saved for a different set of species.");
        }
        // An uploaded model is known only by the name of the file it came
        // from, and a file gets renamed. Whether the map fits is settled by
        // what it draws, when the map is applied.
        let (em, im) = (e.1.as_str(), i.1.as_str());
        if is_upload_reference(em) && is_upload_reference(im) {
            continue;
        }
        if let (Some(em), Some(im)) = (em, im) {
            if em != im {
                return err(format!("Species {} uses a different model in the map file.", e.0));
            }
        }
    }
    Ok(true)
}

/// `mapSpeciesIds(parsed)`: the species a map was drawn over, from its
/// envelope, or from its cells.
pub fn map_species_ids(parsed: &ParsedMapDocument) -> Vec<String> {
    let mut seen = HashSet::new();
    let listed: Vec<String> = parsed
        .metadata
        .as_ref()
        .and_then(|m| m.get("species"))
        .and_then(Value::as_array)
        .map(|l| l.iter().filter_map(|sp| sp.get("id").filter(|v| js::truthy(v)).map(js::to_string)).collect())
        .unwrap_or_default();
    if !listed.is_empty() {
        return listed.into_iter().filter(|id| seen.insert(id.clone())).collect();
    }
    parsed
        .map
        .get("cells")
        .and_then(Value::as_array)
        .map(|cells| {
            cells
                .iter()
                .filter_map(|c| c.get("id").filter(|v| js::truthy(v)).map(js::to_string))
                .filter(|id| seen.insert(id.clone()))
                .collect()
        })
        .unwrap_or_default()
}

/// Fields that are prose rather than identifiers. Everything else in a map
/// that is a string is, or contains, an id.
const PROSE_FIELDS: [&str; 16] = [
    "name", "label", "text", "formula", "organism", "note", "subsystem", "color", "fill", "stroke", "kind", "membrane", "side",
    "visualRole", "transportMode", "fontFamily",
];

fn is_id_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-'
}

/// `value.replace(/(^|[^A-Za-z0-9_.-])<from>(?=[@/])/g, '$1<to>')`.
fn rename_in_front(value: &str, from: &str, to: &str) -> String {
    if value == from {
        return to.to_string();
    }
    let mut out = String::with_capacity(value.len());
    let mut i = 0;
    let bytes = value.as_bytes();
    while i < value.len() {
        if value[i..].starts_with(from) {
            let before_ok = i == 0 || value[..i].chars().next_back().is_some_and(|c| !is_id_char(c));
            let after = i + from.len();
            let after_ok = after < value.len() && (bytes[after] == b'@' || bytes[after] == b'/');
            if before_ok && after_ok {
                out.push_str(to);
                i = after;
                continue;
            }
        }
        let ch = value[i..].chars().next().expect("in bounds");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn walk(value: &Value, key: Option<&str>, from: &str, to: &str) -> Value {
    match value {
        Value::String(s) => {
            if key.is_some_and(|k| PROSE_FIELDS.contains(&k)) {
                value.clone()
            } else {
                Value::String(rename_in_front(s, from, to))
            }
        }
        Value::Array(a) => Value::Array(a.iter().map(|v| walk(v, key, from, to)).collect()),
        Value::Object(m) => Value::Object(m.iter().map(|(k, v)| (k.clone(), walk(v, Some(k), from, to))).collect()),
        other => other.clone(),
    }
}

/// `renameMapSpecies(parsed, fromId, toId, {model})`: read a map drawn over
/// one species as a map of another.
///
/// The species id is in front of every identifier a map holds (`iDemo@PGI`,
/// `met:iDemo@PGI:iDemo@g6p_c`, the boundary `iDemo/h`), and for an uploaded
/// model that id is whatever the file declares. This rewrites the id wherever
/// it stands in front of something, and leaves names, labels and free text
/// alone. It does not decide whether the result fits.
pub fn rename_map_species(parsed: &ParsedMapDocument, from_id: &str, to_id: &str, model: Option<&str>) -> ParsedMapDocument {
    if from_id.is_empty() || to_id.is_empty() || from_id == to_id {
        return parsed.clone();
    }
    let mut map = match &parsed.map {
        Value::Object(m) => {
            let without: Map<String, Value> = m.iter().filter(|(k, _)| k.as_str() != "texts").map(|(k, v)| (k.clone(), v.clone())).collect();
            match walk(&Value::Object(without), None, from_id, to_id) {
                Value::Object(o) => o,
                _ => Map::new(),
            }
        }
        other => match walk(other, None, from_id, to_id) {
            Value::Object(o) => o,
            _ => Map::new(),
        },
    };
    map.insert("texts".into(), parsed.map.get("texts").filter(|t| !t.is_null()).cloned().unwrap_or(json!([])));
    let mut out = parsed.clone();
    out.map = Value::Object(map);
    if let Some(Value::Object(meta)) = &parsed.metadata {
        let mut meta = meta.clone();
        meta.insert("species".into(), json!([{ "id": to_id, "model": model }]));
        out.metadata = Some(Value::Object(meta));
    }
    if let Some(pm) = parsed.pathway_manager.as_ref().filter(|p| js::truthy(p)) {
        out.pathway_manager = Some(walk(pm, None, from_id, to_id));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_timestamps_print_like_javascript() {
        assert_eq!(iso_timestamp(0), "1970-01-01T00:00:00.000Z");
        // new Date(1760371200123).toISOString()
        assert_eq!(iso_timestamp(1_760_371_200_123), "2025-10-13T16:00:00.123Z");
    }

    #[test]
    fn rename_only_in_front_of_identifiers() {
        assert_eq!(rename_in_front("iDemo@PGI", "iDemo", "x"), "x@PGI");
        assert_eq!(rename_in_front("met:iDemo@PGI:iDemo@g6p_c", "iDemo", "x"), "met:x@PGI:x@g6p_c");
        assert_eq!(rename_in_front("iDemo/h", "iDemo", "x"), "x/h");
        assert_eq!(rename_in_front("aiDemo@PGI", "iDemo", "x"), "aiDemo@PGI");
        assert_eq!(rename_in_front("iDemo", "iDemo", "x"), "x");
        assert_eq!(rename_in_front("iDemo2@a", "iDemo", "x"), "iDemo2@a");
    }
}
