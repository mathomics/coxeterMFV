//! Port of `src/core/sbml.js`: the COBRA SBML reader.
//!
//! Published genome-scale models are distributed as SBML, so this is what lets
//! a Published Community be built from the authors' own file rather than from
//! a network retyped out of the paper. It reads the two dialects those files
//! actually come in:
//!
//! ```text
//! SBML Level 2 + COBRA conventions
//!   bounds and objective live in each reaction's <kineticLaw> as parameters
//!   named LOWER_BOUND / UPPER_BOUND / OBJECTIVE_COEFFICIENT
//! SBML Level 3 + the FBC package
//!   bounds are named parameters referenced by fbc:lowerFluxBound, and the
//!   objective is an fbc:listOfObjectives
//! ```
//!
//! Identifiers are un-mangled back to the ids the papers use: SBML forbids a
//! leading digit and most punctuation, so exporters write
//! `R_EX_glc__D_LPAREN_e_RPAREN_` for `EX_glc__D_e`. Nothing else about the
//! model is altered (no reactions are added, removed or rescaled), so what
//! comes out can be checked against the source file reaction by reaction.
//!
//! Like the JS, this reads the text with patterns rather than an XML parser.
//! The patterns are the JS ones, so a file reads the same in both builds,
//! quirks included (a `speciesReference` without `stoichiometry` counts 0).

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value, json};

use coxeter_core::js;
use coxeter_core::model::Model;

use crate::error::{Error, ErrorKind, Result};
use crate::jscompat::{self, B, W, js_order};

static ESCAPED_CODE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"__([0-9]+)__").expect("pattern"));
static PAREN_SUFFIX: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"\(({W}+)\)$")).expect("pattern"));
static BRACKET_SUFFIX: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"\[({W}+)\]$")).expect("pattern"));

/// `String.fromCharCode(Number(code))`: the code taken modulo 2¹⁶.
fn char_code_unit(digits: &str) -> u16 {
    let n = js::string_to_number(digits);
    if !n.is_finite() { 0 } else { n.trunc().rem_euclid(65536.0) as u16 }
}

/// Options of [`unmangle_id`].
#[derive(Debug, Clone, Copy)]
pub struct UnmangleOptions {
    /// Rewrite a trailing `(e)` or `[e]` to `_e`, so the three spellings of one
    /// metabolite become one id. On by default because it is what makes a BiGG
    /// file line up with this app's ids. But it is **not invertible**:
    /// `12ethd[c_Bt]` comes back as `12ethd_c_Bt`. Turn it off to read back
    /// exactly what was written.
    pub normalise_compartment: bool,
}

impl Default for UnmangleOptions {
    fn default() -> Self {
        UnmangleOptions { normalise_compartment: true }
    }
}

/// Undo the character escaping SBML exporters apply to BiGG identifiers.
///
/// Two conventions are in the wild: the COBRA Toolbox writes `_DASH_`, cobrapy
/// writes `__45__` (the character code). Both are decoded, because files of
/// both kinds are published and this project writes the second.
pub fn unmangle_id(id: &str, prefix: &str, opts: UnmangleOptions) -> String {
    let rest = if !prefix.is_empty() { id.strip_prefix(prefix).unwrap_or(id) } else { id };
    // Character codes are UTF-16 code units: a pair of escapes is one character.
    let mut units: Vec<u16> = Vec::with_capacity(rest.len());
    let mut last = 0;
    for c in ESCAPED_CODE.captures_iter(rest) {
        let m = c.get(0).expect("whole match");
        units.extend(rest[last..m.start()].encode_utf16());
        units.push(char_code_unit(&c[1]));
        last = m.end();
    }
    units.extend(rest[last..].encode_utf16());
    let mut out = String::from_utf16_lossy(&units);
    for (from, to) in [
        ("_LPAREN_", "("),
        ("_RPAREN_", ")"),
        ("_LSQBKT_", "["),
        ("_RSQBKT_", "]"),
        ("_DASH_", "-"),
        ("_FSLASH_", "/"),
        ("_BSLASH_", "\\"),
        ("_COMMA_", ","),
        ("_PERIOD_", "."),
        ("_APOS_", "'"),
    ] {
        if out.contains(from) {
            out = out.replace(from, to);
        }
    }
    // `EX_glc__D(e)` and `glc__D[e]` are both spelled `..._e` in this project.
    if opts.normalise_compartment {
        out = PAREN_SUFFIX.replace(&out, "_$1").into_owned();
        out = BRACKET_SUFFIX.replace(&out, "_$1").into_owned();
    }
    out
}

/// `attr(fragment, name)`: the value of the first `name="…"` in a tag.
fn attr<'a>(fragment: &'a str, name: &str) -> Option<&'a str> {
    let re = jscompat::regex(&format!(r#"{B}{}="([^"]*)""#, regex::escape(name)));
    re.captures(fragment).map(|c| c.get(1).expect("group").as_str())
}

/// `num(value, fallback)`: `Number(value)` when finite. `Number(null)` is 0,
/// so an absent attribute reads as 0, not as the fallback.
fn num(value: Option<&str>, fallback: Option<f64>) -> Option<f64> {
    let n = match value {
        None => 0.0,
        Some(v) => js::string_to_number(v),
    };
    if n.is_finite() { Some(n) } else { fallback }
}

/// Decode the handful of XML entities that appear in model names.
fn decode(s: Option<&str>) -> String {
    s.unwrap_or("")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// `s.slice(0, i)` where `i` came from `indexOf` and may be -1 (everything but
/// the last character).
fn slice_to(s: &str, index: Option<usize>) -> &str {
    match index {
        Some(i) => &s[..i],
        None => match s.char_indices().next_back() {
            Some((i, _)) => &s[..i],
            None => "",
        },
    }
}

/// Options of [`parse_cobra_sbml`].
#[derive(Debug, Clone)]
pub struct SbmlReadOptions {
    /// Model id to use when the file does not carry one.
    pub id: Option<String>,
    /// Magnitude used when a bound is missing (1000).
    pub default_bound: f64,
    /// Bounds at least this large in magnitude are the file's way of writing
    /// "unbounded" and are rewritten to `default_bound`. Off (infinite) unless
    /// asked for: it changes the model, so the caller has to say so and justify
    /// it. The Feist 2007 iAF1260 supplement writes 999999 where COBRA writes
    /// 1000, and a coefficient of 1e6 in a coupling row is enough on its own to
    /// make a community LP unsolvable.
    pub sentinel_bound: f64,
    /// See [`UnmangleOptions::normalise_compartment`].
    pub normalise_compartment: bool,
}

impl Default for SbmlReadOptions {
    fn default() -> Self {
        SbmlReadOptions { id: None, default_bound: 1000.0, sentinel_bound: f64::INFINITY, normalise_compartment: true }
    }
}

static MODEL_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"<model{B}([^>]*)>")).expect("pattern"));
static COMPARTMENT_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"<compartment{B}([^>]*)/?>")).expect("pattern"));
static PARAM_SECTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<listOfParameters>((?s:.)*?)</listOfParameters>").expect("pattern"));
static PARAM_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"<parameter{B}([^>]*)/?>")).expect("pattern"));
static GENE_PRODUCT_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"<fbc:geneProduct{B}([^>]*)/?>")).expect("pattern"));
static SPECIES_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"<species{B}([^>]*)/?>")).expect("pattern"));
static SPECIES_SPLIT: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"<species{B}")).expect("pattern"));
static REACTION_SPLIT: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"<reaction{B}")).expect("pattern"));
static SPECIES_REF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"<speciesReference{B}([^>]*)/?>")).expect("pattern"));
static FLUX_OBJECTIVE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"<fbc:fluxObjective{B}([^>]*)/?>")).expect("pattern"));
static COMPARTMENT_SUFFIX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"_([a-z][a-z0-9]?)$").expect("pattern"));
static FORMULA_NOTE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"FORMULA:{}*([^<{}]*)", *jscompat::S, jscompat::WS_CHARS)).expect("pattern"));
static GPA_SECTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"<fbc:geneProductAssociation{B}[^>]*>((?s:.)*?)</fbc:geneProductAssociation>")).expect("pattern")
});
static GPA_TAGS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"<(/?)fbc:(and|or|geneProductRef){B}([^>]*?)(/?)>")).expect("pattern"));

/// Read a COBRA SBML document (Level 2 or Level 3 + FBC) into a COBRA JSON
/// document, ready for [`Model::from_value`].
pub fn parse_cobra_sbml(xml: &str, opts: &SbmlReadOptions) -> Value {
    let default_bound = opts.default_bound;
    let sentinel_bound = opts.sentinel_bound;
    let id_opts = UnmangleOptions { normalise_compartment: opts.normalise_compartment };
    let unmangle = |id: &str, prefix: &str| unmangle_id(id, prefix, id_opts);

    let model_tag = MODEL_TAG.captures(xml).map_or("", |c| c.get(1).expect("group").as_str());
    let model_id = match &opts.id {
        Some(id) => id.clone(),
        None => unmangle(attr(model_tag, "id").unwrap_or("model"), "M_"),
    };
    let model_name = {
        let n = decode(attr(model_tag, "name"));
        if n.is_empty() { model_id.clone() } else { n }
    };

    // Compartments
    let mut compartments = Map::new();
    for m in COMPARTMENT_TAG.captures_iter(xml) {
        let Some(id) = attr(&m[1], "id").filter(|s| !s.is_empty()) else { continue };
        let name = decode(attr(&m[1], "name"));
        compartments.insert(unmangle(id, "C_"), json!(if name.is_empty() { id.to_string() } else { name }));
    }

    // Global parameters, for the FBC dialect
    let mut global_params: HashMap<String, Option<f64>> = HashMap::new();
    if let Some(section) = PARAM_SECTION.captures(xml) {
        for p in PARAM_TAG.captures_iter(&section[1]) {
            if let Some(id) = attr(&p[1], "id").filter(|s| !s.is_empty()) {
                global_params.insert(id.to_string(), num(attr(&p[1], "value"), None));
            }
        }
    }

    // Gene products, for the FBC dialect
    // Level 2 files keep the rule in a note. Level 3 + FBC keeps it as a nested
    // and/or tree over gene products declared once at the end of the model, and
    // reading only the note would drop every GPR in a modern BiGG file.
    let mut gene_labels: HashMap<String, String> = HashMap::new();
    let mut genes = Vec::new();
    for g in GENE_PRODUCT_TAG.captures_iter(xml) {
        let Some(gid) = attr(&g[1], "fbc:id").filter(|s| !s.is_empty()) else { continue };
        let label = {
            let l = decode(attr(&g[1], "fbc:label"));
            if l.is_empty() { unmangle(gid, "G_") } else { l }
        };
        gene_labels.insert(gid.to_string(), label.clone());
        let name = decode(attr(&g[1], "fbc:name"));
        genes.push(json!({"id": label, "name": if name.is_empty() { label.clone() } else { name }}));
    }

    // Metabolites
    let mut metabolites = Vec::new();
    let mut met_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut formula_index: Option<HashMap<String, String>> = None;
    for m in SPECIES_TAG.captures_iter(xml) {
        let fragment = &m[1];
        let Some(raw_id) = attr(fragment, "id").filter(|s| !s.is_empty()) else { continue };
        // Boundary species are the "outside" ends of exchange reactions. COBRA
        // models carry the balance on the real metabolite, so they are dropped.
        if attr(fragment, "boundaryCondition") == Some("true") {
            continue;
        }
        let id = unmangle(raw_id, "M_");
        met_ids.insert(raw_id.to_string());
        let compartment = {
            let c = unmangle(attr(fragment, "compartment").unwrap_or(""), "C_");
            if !c.is_empty() {
                c
            } else if let Some(s) = COMPARTMENT_SUFFIX.captures(&id).map(|c| c[1].to_string()).filter(|s| !s.is_empty()) {
                s
            } else {
                "c".to_string()
            }
        };
        let formula = match attr(fragment, "fbc:chemicalFormula") {
            Some(f) => f.to_string(),
            None => formula_from_notes(xml, raw_id, &mut formula_index).unwrap_or_default(),
        };
        let name = decode(attr(fragment, "name"));
        let charge = num(attr(fragment, "fbc:charge").or_else(|| attr(fragment, "charge")), Some(0.0)).unwrap_or(0.0);
        metabolites.push(json!({
            "id": id,
            "name": if name.is_empty() { id.clone() } else { name },
            "compartment": compartment,
            "formula": formula,
            "charge": charge,
        }));
    }

    // Reactions
    let mut reactions = Vec::new();
    let mut objective = Map::new();
    for block in REACTION_SPLIT.split(xml).skip(1) {
        let header = slice_to(block, block.find('>'));
        let Some(raw_id) = attr(header, "id").filter(|s| !s.is_empty()) else { continue };
        let body = match block.find("</reaction>") {
            Some(i) => &block[..i + 1],
            None => "",
        };
        let id = unmangle(raw_id, "R_");

        let mut stoich: Map<String, Value> = Map::new();
        let mut collect = |tag: &str, sign: f64| {
            let section = jscompat::regex(&format!(r"<listOf{tag}>((?s:.)*?)</listOf{tag}>"));
            let Some(section) = section.captures(body) else { return };
            for r in SPECIES_REF.captures_iter(&section[1]) {
                let Some(species) = attr(&r[1], "species").filter(|s| !s.is_empty()) else { continue };
                if !met_ids.contains(species) {
                    continue;
                }
                let mid = unmangle(species, "M_");
                let coef = sign * num(attr(&r[1], "stoichiometry"), Some(1.0)).unwrap_or(1.0);
                let previous = stoich.get(&mid).and_then(Value::as_f64).unwrap_or(0.0);
                stoich.insert(mid, json!(previous + coef));
            }
        };
        collect("Reactants", -1.0);
        collect("Products", 1.0);
        let mut stoich = js_order(stoich);
        stoich.retain(|_, v| v.as_f64() != Some(0.0));

        // Bounds: FBC first, then the Level 2 kineticLaw parameters.
        let mut lower: Option<f64> = None;
        let mut upper: Option<f64> = None;
        if let Some(r) = attr(header, "fbc:lowerFluxBound").filter(|s| !s.is_empty()) {
            if let Some(v) = global_params.get(r) {
                lower = *v;
            }
        }
        if let Some(r) = attr(header, "fbc:upperFluxBound").filter(|s| !s.is_empty()) {
            if let Some(v) = global_params.get(r) {
                upper = *v;
            }
        }
        if lower.is_none() || upper.is_none() {
            for p in PARAM_TAG.captures_iter(body) {
                let pid = attr(&p[1], "id");
                let Some(value) = num(attr(&p[1], "value"), None) else { continue };
                if pid == Some("LOWER_BOUND") && lower.is_none() {
                    lower = Some(value);
                } else if pid == Some("UPPER_BOUND") && upper.is_none() {
                    upper = Some(value);
                } else if pid == Some("OBJECTIVE_COEFFICIENT") && value != 0.0 {
                    objective.insert(id.clone(), json!(value));
                }
            }
        }
        let mut lower = lower.unwrap_or(if attr(header, "reversible") == Some("false") { 0.0 } else { -default_bound });
        let mut upper = upper.unwrap_or(default_bound);
        if lower <= -sentinel_bound {
            lower = -default_bound;
        }
        if upper >= sentinel_bound {
            upper = default_bound;
        }

        let name = decode(attr(header, "name"));
        let rule = gpr_from_fbc(body, &gene_labels).or_else(|| gene_rule_from_notes(body)).unwrap_or_default();
        reactions.push(json!({
            "id": id,
            "name": if name.is_empty() { id.clone() } else { name },
            "metabolites": stoich,
            "lower_bound": lower,
            "upper_bound": upper,
            "gene_reaction_rule": rule,
            "subsystem": note_value(body, "SUBSYSTEM").unwrap_or_default(),
        }));
    }

    // FBC objective
    for m in FLUX_OBJECTIVE.captures_iter(xml) {
        let rid = attr(&m[1], "fbc:reaction");
        let coef = num(attr(&m[1], "fbc:coefficient"), Some(1.0)).unwrap_or(1.0);
        if let Some(rid) = rid.filter(|r| !r.is_empty()) {
            if coef != 0.0 {
                objective.insert(unmangle(rid, "R_"), json!(coef));
            }
        }
    }

    for met in &metabolites {
        let c = met["compartment"].as_str().unwrap_or_default().to_string();
        let named = compartments.get(&c).is_some_and(js::truthy);
        if !named {
            compartments.insert(c.clone(), json!(c));
        }
    }

    let objective = js_order(objective);
    json!({
        "id": model_id,
        "name": model_name,
        "compartments": js_order(compartments),
        "metabolites": metabolites,
        "reactions": reactions,
        "genes": genes,
        "objective": if objective.is_empty() { Value::Null } else { Value::Object(objective) },
    })
}

/// Pull `KEY: value` out of a COBRA `<notes>` block.
fn note_value(fragment: &str, key: &str) -> Option<String> {
    let re = jscompat::regex(&format!(r"{}:{}*([^<]*)", regex::escape(key), *jscompat::S));
    let caps = re.captures(fragment)?;
    let value = jscompat::js_trim(caps.get(1).map_or("", |m| m.as_str()));
    if value.is_empty() { None } else { Some(decode(Some(value))) }
}

/// Read `<fbc:geneProductAssociation>` back into a COBRA rule string.
///
/// The nesting is arbitrary, so this walks the tags with a stack rather than
/// matching a fixed shape. Parentheses go back in only where a child's operator
/// differs from its parent's, which is what makes the string re-parse to the
/// same tree.
fn gpr_from_fbc(body: &str, labels: &HashMap<String, String>) -> Option<String> {
    #[derive(Default)]
    struct Node {
        op: Option<String>,
        gene: Option<String>,
        args: Vec<Node>,
    }
    let section = GPA_SECTION.captures(body)?;
    let mut stack: Vec<Node> = vec![Node::default()];
    for m in GPA_TAGS.captures_iter(&section[1]) {
        let closing = !m[1].is_empty();
        let kind = &m[2];
        let attr_text = &m[3];
        let self_closing = !m[4].is_empty();
        if kind == "geneProductRef" {
            if let Some(r) = attr(attr_text, "fbc:geneProduct").filter(|s| !s.is_empty()) {
                let gene = labels.get(r).cloned().unwrap_or_else(|| unmangle_id(r, "G_", UnmangleOptions::default()));
                stack.last_mut().expect("root").args.push(Node { gene: Some(gene), ..Node::default() });
            }
        } else if closing {
            if stack.len() > 1 {
                let node = stack.pop().expect("non-empty");
                stack.last_mut().expect("root").args.push(node);
            }
        } else if !self_closing {
            stack.push(Node { op: Some(kind.to_string()), ..Node::default() });
        }
    }

    fn render(node: &Node) -> String {
        if let Some(g) = node.gene.as_deref().filter(|g| !g.is_empty()) {
            return g.to_string();
        }
        match &node.op {
            None => {
                if node.args.len() == 1 {
                    render(&node.args[0])
                } else {
                    node.args.iter().map(render).collect::<Vec<_>>().join(" and ")
                }
            }
            Some(op) => node
                .args
                .iter()
                .map(|a| match &a.op {
                    Some(child) if a.gene.is_none() && child != op => format!("({})", render(a)),
                    _ => render(a),
                })
                .collect::<Vec<_>>()
                .join(&format!(" {op} ")),
        }
    }
    let root = stack.into_iter().next().expect("root");
    let rule = jscompat::js_trim(&render(&root)).to_string();
    if rule.is_empty() { None } else { Some(rule) }
}

fn gene_rule_from_notes(body: &str) -> Option<String> {
    note_value(body, "GENE_ASSOCIATION").or_else(|| note_value(body, "GENE ASSOCIATION"))
}

/// Level 2 files keep the elemental formula in the species' notes rather than
/// in an attribute. Scanning the whole document per species would be
/// quadratic, so the notes are indexed once on first use.
fn formula_from_notes(xml: &str, raw_id: &str, index: &mut Option<HashMap<String, String>>) -> Option<String> {
    let index = index.get_or_insert_with(|| {
        let mut index = HashMap::new();
        for block in SPECIES_SPLIT.split(xml).skip(1) {
            let header = slice_to(block, block.find('>'));
            let Some(id) = attr(header, "id").filter(|s| !s.is_empty()) else { continue };
            let notes_end = block.find("</species>").unwrap_or(0);
            let notes = &block[..notes_end + block[notes_end..].chars().next().map_or(0, char::len_utf8)];
            if let Some(f) = FORMULA_NOTE.captures(notes) {
                if !f[1].is_empty() {
                    index.insert(id.to_string(), f[1].to_string());
                }
            }
        }
        index
    });
    index.get(raw_id).cloned()
}

/// Convenience: read a model straight into a [`Model`].
pub fn load_sbml_model(xml: &str, opts: &SbmlReadOptions) -> Result<Model> {
    Model::from_value(&parse_cobra_sbml(xml, opts)).map_err(|e| Error::new(ErrorKind::InvalidModel, e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAITHFUL: UnmangleOptions = UnmangleOptions { normalise_compartment: false };

    #[test]
    fn the_reader_also_decodes_the_cobra_toolbox_spelling() {
        assert_eq!(unmangle_id("M_12ppd_DASH_R_LSQBKT_u_RSQBKT_", "M_", FAITHFUL), "12ppd-R[u]");
        assert_eq!(unmangle_id("R_EX_glc_LPAREN_e_RPAREN_", "R_", FAITHFUL), "EX_glc(e)");
    }

    #[test]
    fn normalising_the_compartment_suffix_is_documented_as_one_way() {
        assert_eq!(unmangle_id("M_12ethd__91__c_Ba__93__", "M_", UnmangleOptions::default()), "12ethd_c_Ba");
        assert_eq!(unmangle_id("M_12ethd__91__c_Ba__93__", "M_", FAITHFUL), "12ethd[c_Ba]");
        assert_eq!(unmangle_id("M_a__55357____56832__", "M_", FAITHFUL), "a😀");
        assert_eq!(unmangle_id("x__65601__", "", FAITHFUL), "xA");
    }

    #[test]
    fn slices_like_index_of() {
        assert_eq!(slice_to("abc", None), "ab");
        assert_eq!(slice_to("ab>c", Some(2)), "ab");
    }
}
