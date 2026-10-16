//! Port of `src/omics/overlay.js`: omics overlay utilities.
//!
//! CSV rows are mapped to gene/protein identifiers used by reaction GPR rules.
//! Reaction scores use an intentionally transparent rule:
//!
//! ```text
//! AND -> minimum subunit abundance
//! OR  -> maximum isoenzyme abundance
//! ```
//!
//! Missing values in an AND branch make that branch unknown. OR can still use
//! another measured branch. This module never changes model bounds or fluxes.

use std::borrow::Borrow;

use coxeter_core::Model;
use coxeter_core::js;
use indexmap::{IndexMap, IndexSet};
use serde_json::{Map, Value, json};

use crate::community_view::CommunityView;
use crate::error::{Error, ErrorKind, Result};
use crate::jsutil::{ascending, is_js_ws, js_trim, json_number, number, split_ws};

const ID_HEADERS: [&str; 10] =
    ["id", "gene", "gene_id", "geneid", "protein", "protein_id", "proteinid", "locus", "locus_tag", "identifier"];
const SPECIES_HEADERS: [&str; 6] = ["species", "organism", "cell", "mag", "genome", "bin"];

fn fail(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::Omics, message)
}

/// One measured gene or protein: its values by condition.
#[derive(Debug, Clone, PartialEq)]
pub struct OmicsRow {
    pub id: String,
    /// The species (MAG, genome…) the row names, empty when the table has no
    /// such column, and then the row applies to every species.
    pub species: String,
    pub values: IndexMap<String, f64>,
}

impl OmicsRow {
    pub fn to_json(&self) -> Value {
        json!({"id": self.id, "species": self.species, "values": values_json(&self.values)})
    }
}

fn values_json(values: &IndexMap<String, f64>) -> Value {
    Value::Object(values.iter().map(|(k, v)| (k.clone(), json_number(*v))).collect())
}

/// A wide omics table: one row per identifier, one value per condition.
#[derive(Debug, Clone, PartialEq)]
pub struct OmicsDataset {
    pub source: String,
    /// `"gene"`, `"protein"`, `"metatranscriptomics"`…
    pub data_type: String,
    pub id_column: String,
    pub species_column: Option<String>,
    pub conditions: Vec<String>,
    /// Rows by [`omics_key`], in file order.
    pub values: IndexMap<String, OmicsRow>,
    pub row_count: usize,
}

/// Options of [`parse_omics_csv`].
#[derive(Debug, Clone)]
pub struct OmicsCsvOptions {
    pub source: String,
    pub data_type: String,
}

impl Default for OmicsCsvOptions {
    fn default() -> Self {
        OmicsCsvOptions { source: "omics.csv".into(), data_type: "gene".into() }
    }
}

/// Read a wide CSV: an identifier column, an optional species column, and
/// every other column that holds at least one number as a condition.
pub fn parse_omics_csv(text: &str, options: &OmicsCsvOptions) -> Result<OmicsDataset> {
    let rows = parse_delimited(text, ',');
    if rows.len() < 2 {
        return Err(fail("El CSV debe incluir encabezado y al menos una fila de datos."));
    }

    let header: Vec<String> = rows[0].iter().map(|x| js_trim(x).to_string()).collect();
    let normalized: Vec<String> = header.iter().map(|h| normalize_header(h)).collect();
    let id_index = normalized.iter().position(|h| ID_HEADERS.contains(&h.as_str())).unwrap_or(0);
    let species_index = normalized.iter().position(|h| SPECIES_HEADERS.contains(&h.as_str()));

    let mut condition_indices = Vec::new();
    for i in 0..header.len() {
        if i == id_index || Some(i) == species_index {
            continue;
        }
        let has_numeric = rows[1..].iter().any(|row| to_number(row.get(i)).is_finite());
        if has_numeric {
            condition_indices.push(i);
        }
    }
    if condition_indices.is_empty() {
        return Err(fail("No numeric expression/abundance columns were found."));
    }

    let conditions: Vec<String> = condition_indices
        .iter()
        .map(|i| if header[*i].is_empty() { format!("condition_{}", i + 1) } else { header[*i].clone() })
        .collect();
    let mut values = IndexMap::new();
    let mut valid_rows = 0;

    for row in &rows[1..] {
        let id = js_trim(row.get(id_index).map_or("", String::as_str)).to_string();
        if id.is_empty() {
            continue;
        }
        let species = species_index.map_or("", |i| js_trim(row.get(i).map_or("", String::as_str))).to_string();
        let mut condition_values = IndexMap::new();
        for (j, column_index) in condition_indices.iter().enumerate() {
            let value = to_number(row.get(*column_index));
            if value.is_finite() {
                condition_values.insert(conditions[j].clone(), value);
            }
        }
        if condition_values.is_empty() {
            continue;
        }
        // Conditions named like numbers come first in a JS object.
        js::js_key_order(&mut condition_values);
        values.insert(omics_key(&species, &id), OmicsRow { id, species, values: condition_values });
        valid_rows += 1;
    }

    if valid_rows == 0 {
        return Err(fail("The CSV contains no usable numeric values."));
    }

    Ok(OmicsDataset {
        source: options.source.clone(),
        data_type: options.data_type.clone(),
        id_column: header[id_index].clone(),
        species_column: species_index.map(|i| header[i].clone()),
        conditions,
        values,
        row_count: valid_rows,
    })
}

/// The dataset as it is saved inside a scenario file.
pub fn serialize_omics_dataset(dataset: Option<&OmicsDataset>) -> Value {
    let Some(dataset) = dataset else { return Value::Null };
    json!({
        "source": dataset.source,
        "dataType": dataset.data_type,
        "idColumn": dataset.id_column,
        "speciesColumn": dataset.species_column,
        "conditions": dataset.conditions,
        "rowCount": dataset.row_count,
        "rows": dataset.values.values().map(OmicsRow::to_json).collect::<Vec<_>>(),
    })
}

/// Read a saved dataset back, or `None` when it is not one or holds no rows.
///
/// Values that are not numbers are dropped: the JavaScript keeps them in
/// memory but never reads them (`Number.isFinite` refuses them).
pub fn deserialize_omics_dataset(raw: &Value) -> Option<OmicsDataset> {
    let obj = raw.as_object()?;
    let conditions = obj.get("conditions")?.as_array()?;
    let mut values = IndexMap::new();
    for row in obj.get("rows").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]) {
        let Some(id) = row.get("id").filter(|v| js::truthy(v)) else { continue };
        let id = js::to_string(id);
        let species = match row.get("species") {
            None | Some(Value::Null) => String::new(),
            Some(v) => js::to_string(v),
        };
        let mut row_values: IndexMap<String, f64> = IndexMap::new();
        if let Some(Value::Object(saved)) = row.get("values") {
            for (condition, value) in saved {
                if let Some(n) = value.as_f64() {
                    row_values.insert(condition.clone(), n);
                }
            }
        }
        js::js_key_order(&mut row_values);
        values.insert(omics_key(&species, &id), OmicsRow { id, species, values: row_values });
    }
    if values.is_empty() {
        return None;
    }
    let text = |key: &str, fallback: &str| match obj.get(key) {
        None | Some(Value::Null) => fallback.to_string(),
        Some(v) => js::to_string(v),
    };
    Some(OmicsDataset {
        source: text("source", "omics.csv"),
        data_type: text("dataType", "gene"),
        id_column: text("idColumn", "id"),
        species_column: match obj.get("speciesColumn") {
            None | Some(Value::Null) => None,
            Some(v) => Some(js::to_string(v)),
        },
        conditions: conditions.iter().map(js::to_string).collect(),
        row_count: values.len(),
        values,
    })
}

/// A species of the active community and the key its model is stored under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OmicsSpecies {
    pub id: String,
    pub file: String,
}

impl OmicsSpecies {
    pub fn new(id: &str, file: &str) -> OmicsSpecies {
        OmicsSpecies { id: id.into(), file: file.into() }
    }
}

/// Score every reaction of the merged community whose own model gives it a
/// gene rule the dataset can evaluate under `condition`.
pub fn reaction_omics_scores<M: Borrow<Model>>(
    community: &impl CommunityView,
    species_config: &[OmicsSpecies],
    models: &IndexMap<String, M>,
    dataset: Option<&OmicsDataset>,
    condition: &str,
) -> IndexMap<String, f64> {
    let mut out = IndexMap::new();
    let Some(dataset) = dataset else { return out };
    if condition.is_empty() {
        return out;
    }

    // A later entry with the same id replaces an earlier one, as `new Map` does.
    let species_by_id: IndexMap<&str, &OmicsSpecies> = species_config.iter().map(|sp| (sp.id.as_str(), sp)).collect();
    for rid in community.merged_reaction_ids() {
        let Some(owner) = community.reaction_owner(rid) else { continue };
        let Some(species_id) = owner.species_id.as_deref().filter(|s| !s.is_empty()) else { continue };
        if owner.original_id.is_empty() {
            continue;
        }
        let Some(model) = species_by_id.get(species_id).and_then(|sp| models.get(&sp.file)) else { continue };
        let Some(reaction) = model.borrow().reactions.get(&owner.original_id) else { continue };
        let rule = js_trim(&reaction.gene_reaction_rule);
        if rule.is_empty() {
            continue;
        }

        let Some(ast) = parse_gpr(rule) else { continue };
        let value = evaluate_gpr(Some(&ast), &|gene_id| lookup_value(dataset, species_id, gene_id, condition));
        if let Some(value) = value.filter(|v| v.is_finite()) {
            out.insert(rid.to_string(), value);
        }
    }
    out
}

/// How two conditions are compared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CompareMetric {
    /// `log2((b + pseudocount) / (a + pseudocount))`.
    #[default]
    Log2Fc,
    /// `b - a`.
    Difference,
}

impl CompareMetric {
    pub fn as_str(self) -> &'static str {
        match self {
            CompareMetric::Log2Fc => "log2fc",
            CompareMetric::Difference => "difference",
        }
    }

    /// The JS tests the metric against `'difference'` only: anything else is
    /// the fold change.
    pub fn parse(s: &str) -> CompareMetric {
        if s == "difference" { CompareMetric::Difference } else { CompareMetric::Log2Fc }
    }
}

/// Options of [`compare_reaction_scores`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompareOptions {
    pub metric: CompareMetric,
    pub pseudocount: f64,
}

impl Default for CompareOptions {
    fn default() -> Self {
        CompareOptions { metric: CompareMetric::Log2Fc, pseudocount: 1.0 }
    }
}

/// Compare condition B with condition A, reaction by reaction. Reactions
/// scored in only one of them, or whose fold change is undefined, are left out.
pub fn compare_reaction_scores(
    a_scores: &IndexMap<String, f64>,
    b_scores: &IndexMap<String, f64>,
    options: &CompareOptions,
) -> IndexMap<String, f64> {
    let mut out = IndexMap::new();
    // `Math.max(0, Number(pseudocount) || 0)`
    let pc = if options.pseudocount.is_nan() { 0.0 } else { js::max(0.0, options.pseudocount) };

    let ids: IndexSet<&String> = a_scores.keys().chain(b_scores.keys()).collect();
    for rid in ids {
        let (Some(a), Some(b)) = (a_scores.get(rid).copied(), b_scores.get(rid).copied()) else { continue };
        if !a.is_finite() || !b.is_finite() {
            continue;
        }
        let value = match options.metric {
            CompareMetric::Difference => b - a,
            CompareMetric::Log2Fc => {
                let aa = a + pc;
                let bb = b + pc;
                // `!(aa > 0)`: NaN is refused too, as in the JS.
                #[allow(clippy::neg_cmp_op_on_partial_ord)]
                let refused = !(aa > 0.0) || !(bb > 0.0);
                if refused {
                    continue;
                }
                js::log2(bb / aa)
            }
        };
        if value.is_finite() {
            out.insert(rid.clone(), value);
        }
    }
    out
}

/// Which kind of colour scale a legend draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScaleKind {
    Sequential,
    Diverging,
}

impl ScaleKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ScaleKind::Sequential => "sequential",
            ScaleKind::Diverging => "diverging",
        }
    }
}

/// Reaction colours with the numbers a legend needs.
#[derive(Debug, Clone, PartialEq)]
pub struct OmicsScale {
    pub kind: ScaleKind,
    /// Reaction id → `#rrggbb`.
    pub colors: IndexMap<String, String>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// The values the two ends of the scale stand for (5th and 95th
    /// percentile, so one outlier does not flatten everything else).
    pub domain_min: Option<f64>,
    pub domain_max: Option<f64>,
    pub center: Option<f64>,
}

impl OmicsScale {
    pub fn to_json(&self) -> Value {
        json!({
            "kind": self.kind.as_str(), "colors": self.colors, "min": self.min, "max": self.max,
            "domainMin": self.domain_min, "domainMax": self.domain_max, "center": self.center,
        })
    }
}

/// One condition: pale to red.
pub fn sequential_omics_colors(scores: &IndexMap<String, f64>, dark: bool) -> OmicsScale {
    let finite: Vec<f64> = scores.values().copied().filter(|v| v.is_finite()).collect();
    if finite.is_empty() {
        return empty_scale(ScaleKind::Sequential);
    }
    let lo = quantile(&finite, 0.05);
    let hi0 = quantile(&finite, 0.95);
    let hi = if hi0 > lo { hi0 } else { lo + 1.0 };
    let mut colors = IndexMap::new();
    for (rid, value) in scores {
        let t = clamp01((value - lo) / (hi - lo));
        colors.insert(rid.clone(), interpolate_hex(if dark { "#334155" } else { "#e2e8f0" }, if dark { "#fb7185" } else { "#b91c1c" }, t));
    }
    OmicsScale {
        kind: ScaleKind::Sequential,
        colors,
        min: Some(js::min_n(finite.iter().copied())),
        max: Some(js::max_n(finite.iter().copied())),
        domain_min: Some(lo),
        domain_max: Some(hi),
        center: None,
    }
}

/// A comparison: blue for less, red for more, pale at zero.
pub fn diverging_omics_colors(scores: &IndexMap<String, f64>, dark: bool) -> OmicsScale {
    let finite: Vec<f64> = scores.values().copied().filter(|v| v.is_finite()).collect();
    if finite.is_empty() {
        return empty_scale(ScaleKind::Diverging);
    }
    let abs: Vec<f64> = finite.iter().map(|v| v.abs()).collect();
    let max_abs = js::max(quantile(&abs, 0.95), 1e-12);
    let mut colors = IndexMap::new();
    let low = if dark { "#60a5fa" } else { "#2563eb" };
    let mid = if dark { "#e2e8f0" } else { "#f8fafc" };
    let high = if dark { "#fb7185" } else { "#dc2626" };
    for (rid, value) in scores {
        let x = js::max(-max_abs, js::min(max_abs, *value));
        let color = if x < 0.0 { interpolate_hex(low, mid, (x + max_abs) / max_abs) } else { interpolate_hex(mid, high, x / max_abs) };
        colors.insert(rid.clone(), color);
    }
    OmicsScale {
        kind: ScaleKind::Diverging,
        colors,
        min: Some(js::min_n(finite.iter().copied())),
        max: Some(js::max_n(finite.iter().copied())),
        domain_min: Some(-max_abs),
        domain_max: Some(max_abs),
        center: Some(0.0),
    }
}

/// A gene rule as a tree. The parser is forgiving, as the JS one is: a rule
/// that starts with an operator keeps an empty left side.
#[derive(Debug, Clone, PartialEq)]
pub enum GprNode {
    Gene(String),
    And(Option<Box<GprNode>>, Box<GprNode>),
    Or(Option<Box<GprNode>>, Box<GprNode>),
}

impl GprNode {
    /// `{type: 'gene', id}` or `{type: 'and'|'or', left, right}`.
    pub fn to_json(&self) -> Value {
        let branch = |kind: &str, left: &Option<Box<GprNode>>, right: &GprNode| {
            json!({"type": kind, "left": left.as_ref().map(|n| n.to_json()), "right": right.to_json()})
        };
        match self {
            GprNode::Gene(id) => json!({"type": "gene", "id": id}),
            GprNode::And(left, right) => branch("and", left, right),
            GprNode::Or(left, right) => branch("or", left, right),
        }
    }
}

struct GprParser<'a> {
    tokens: Vec<&'a str>,
    pos: usize,
}

impl<'a> GprParser<'a> {
    fn peek(&self) -> Option<&'a str> {
        self.tokens.get(self.pos).copied()
    }

    fn primary(&mut self) -> Option<GprNode> {
        let token = self.peek();
        if token == Some("(") {
            self.pos += 1;
            let node = self.or_expr();
            if self.peek() == Some(")") {
                self.pos += 1;
            }
            return node;
        }
        match token {
            None | Some(")") | Some("and") | Some("or") => None,
            Some(id) => {
                self.pos += 1;
                Some(GprNode::Gene(id.to_string()))
            }
        }
    }

    fn and_expr(&mut self) -> Option<GprNode> {
        let mut node = self.primary();
        while self.peek() == Some("and") {
            self.pos += 1;
            let Some(right) = self.primary() else { break };
            node = Some(GprNode::And(node.map(Box::new), Box::new(right)));
        }
        node
    }

    fn or_expr(&mut self) -> Option<GprNode> {
        let mut node = self.and_expr();
        while self.peek() == Some("or") {
            self.pos += 1;
            let Some(right) = self.and_expr() else { break };
            node = Some(GprNode::Or(node.map(Box::new), Box::new(right)));
        }
        node
    }
}

/// Parse a gene rule (`(g1 and g2) or g3`), or `None` when it names no gene.
pub fn parse_gpr(rule: &str) -> Option<GprNode> {
    let spaced = rule.replace('(', " ( ").replace(')', " ) ");
    let lowered: Vec<String> = split_ws(js_trim(&spaced))
        .into_iter()
        .filter(|t| !t.is_empty())
        .map(|token| {
            let lower = token.to_lowercase();
            if lower == "and" || lower == "or" { lower } else { token.to_string() }
        })
        .collect();
    if lowered.is_empty() {
        return None;
    }
    let mut parser = GprParser { tokens: lowered.iter().map(String::as_str).collect(), pos: 0 };
    parser.or_expr()
}

/// The abundance a rule implies: the least of an AND, the most of an OR that
/// has a measured side. `get_value` gives a gene's value, if it has one.
pub fn evaluate_gpr(node: Option<&GprNode>, get_value: &dyn Fn(&str) -> Option<f64>) -> Option<f64> {
    let finite = |v: Option<f64>| v.filter(|x| x.is_finite());
    match node? {
        GprNode::Gene(id) => finite(get_value(id)),
        GprNode::And(left, right) => {
            let left = finite(evaluate_gpr(left.as_deref(), get_value));
            let right = finite(evaluate_gpr(Some(right), get_value));
            Some(js::min(left?, right?))
        }
        GprNode::Or(left, right) => {
            let left = finite(evaluate_gpr(left.as_deref(), get_value));
            let right = finite(evaluate_gpr(Some(right), get_value));
            match (left, right) {
                (Some(l), Some(r)) => Some(js::max(l, r)),
                (Some(l), None) => Some(l),
                (None, r) => r,
            }
        }
    }
}

/// A species' own row wins over a row with no species. A row that exists but
/// lacks the condition is not rescued by the other one.
fn lookup_value(dataset: &OmicsDataset, species_id: &str, gene_id: &str, condition: &str) -> Option<f64> {
    let row = dataset.values.get(&omics_key(species_id, gene_id)).or_else(|| dataset.values.get(&omics_key("", gene_id)))?;
    row.values.get(condition).copied().filter(|v| v.is_finite())
}

/// The key a row is stored under: species and identifier, both trimmed.
pub fn omics_key(species: &str, id: &str) -> String {
    format!("{}\u{0}{}", js_trim(species), js_trim(id))
}

/// A quoted-field reader for CSV and TSV: `""` inside quotes is a quote, a
/// row with nothing but blanks is skipped, a leading byte-order mark is not
/// part of the first header.
pub(crate) fn parse_delimited(text: &str, delimiter: char) -> Vec<Vec<String>> {
    fn end_row(rows: &mut Vec<Vec<String>>, row: &mut Vec<String>, field: &mut String) {
        if field.ends_with('\r') {
            field.pop();
        }
        row.push(std::mem::take(field));
        if row.iter().any(|x| !js_trim(x).is_empty()) {
            rows.push(std::mem::take(row));
        } else {
            row.clear();
        }
    }

    let mut rows = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let input = text.strip_prefix('\u{FEFF}').unwrap_or(text);

    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if quoted {
            if ch == '"' && chars.peek() == Some(&'"') {
                field.push('"');
                chars.next();
            } else if ch == '"' {
                quoted = false;
            } else {
                field.push(ch);
            }
        } else if ch == '"' {
            quoted = true;
        } else if ch == delimiter {
            row.push(std::mem::take(&mut field));
        } else if ch == '\n' {
            end_row(&mut rows, &mut row, &mut field);
        } else {
            field.push(ch);
        }
    }
    end_row(&mut rows, &mut row, &mut field);
    rows
}

/// A cell as a number, or NaN: blank is not zero here.
fn to_number(value: Option<&String>) -> f64 {
    let text = js_trim(value.map_or("", String::as_str));
    if text.is_empty() {
        return f64::NAN;
    }
    let n = number(text);
    if n.is_finite() { n } else { f64::NAN }
}

/// `Sample.Name 1` → `sample_name_1`.
pub(crate) fn normalize_header(value: &str) -> String {
    let lower = js_trim(value).to_lowercase();
    let mut out = String::with_capacity(lower.len());
    let mut in_run = false;
    for c in lower.chars() {
        if is_js_ws(c) || c == '.' || c == '-' {
            if !in_run {
                out.push('_');
                in_run = true;
            }
        } else {
            out.push(c);
            in_run = false;
        }
    }
    out
}

fn empty_scale(kind: ScaleKind) -> OmicsScale {
    OmicsScale {
        kind,
        colors: IndexMap::new(),
        min: None,
        max: None,
        domain_min: None,
        domain_max: None,
        center: if kind == ScaleKind::Diverging { Some(0.0) } else { None },
    }
}

fn quantile(values: &[f64], q: f64) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(ascending);
    if sorted.is_empty() {
        return f64::NAN;
    }
    let index = (sorted.len() - 1) as f64 * q;
    let lo = index.floor();
    let hi = index.ceil();
    if lo == hi {
        return sorted[lo as usize];
    }
    let t = index - lo;
    sorted[lo as usize] * (1.0 - t) + sorted[hi as usize] * t
}

fn interpolate_hex(a: &str, b: &str, t: f64) -> String {
    let aa = hex_to_rgb(a);
    let bb = hex_to_rgb(b);
    let mut out = String::from("#");
    for i in 0..3 {
        let v = js::round(aa[i] + (bb[i] - aa[i]) * clamp01(t));
        // `NaN.toString(16)` is "NaN": a score that is not a number shows.
        if v.is_nan() {
            out.push_str("NaN");
        } else {
            out.push_str(&format!("{:02x}", v as i64));
        }
    }
    out
}

fn hex_to_rgb(hex: &str) -> [f64; 3] {
    let clean = hex.trim_start_matches('#');
    [0, 2, 4].map(|i| i64::from_str_radix(&clean[i..i + 2], 16).map_or(f64::NAN, |v| v as f64))
}

fn clamp01(x: f64) -> f64 {
    js::min(1.0, js::max(0.0, x))
}

/// A map of scores as a JSON object (reaction id → number).
pub fn scores_to_json(scores: &IndexMap<String, f64>) -> Value {
    Value::Object(scores.iter().map(|(k, v)| (k.clone(), json_number(*v))).collect::<Map<_, _>>())
}
