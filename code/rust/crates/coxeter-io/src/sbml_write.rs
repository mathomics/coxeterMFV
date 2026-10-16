//! Port of `src/core/sbml-write.js`: the COBRA SBML writer, Level 3 Version 1
//! with the FBC package, Version 2.
//!
//! The counterpart of [`crate::sbml`]. That module reads what the field
//! publishes, and this one writes what the field reads: cobrapy and the COBRA
//! Toolbox both load L3V1 + fbc-v2, so a model built or edited here can leave
//! for a real analysis stack instead of staying in this app's JSON.
//!
//! # What travels and what does not
//! The metabolic model travels in full: compartments, species with their
//! formula and charge, stoichiometry, flux bounds, the objective, gene products
//! and the gene-protein-reaction rules, plus each reaction's subsystem in its
//! notes.
//!
//! This project's own extensions do not: `cell`, `compartment_tree`,
//! per-reaction `layout`, `primary` and `pathway` describe how a model is
//! *drawn*, have no home in FBC, and are already carried by the map document.
//! An SBML file written here is the model, not the picture.
//!
//! SteadyCom is a further step out of reach: its coupling rows (`LB·X ≤ V ≤
//! UB·X` and `Σ X = 1`) are not expressible in fbc-v2 at all, which stops at
//! bounds, an objective and GPR. Members export one file each, and the community
//! LP does not export. The place that would hold it is fbc **v3**'s
//! UserDefinedConstraint.
//!
//! # Identifiers
//! SBML SIds are `(letter|_)(letter|digit|_)*`, and BiGG ids are not:
//! `12ppd-R_e` opens with a digit, `EX_glc__D(e)` and `12ethd[c_Bt]` carry
//! punctuation. Two escaping conventions are in use: the COBRA Toolbox writes
//! `_DASH_`, cobrapy writes `__45__` (the character code). This writer emits
//! cobrapy's, because cobrapy is what most readers will open the file with, and
//! the reader decodes both so anything written here reads back unchanged.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::LazyLock;

use indexmap::IndexSet;
use regex::Regex;
use serde_json::{Map, Value, json};

use coxeter_core::js;
use coxeter_core::model::Model;

use crate::error::{Error, ErrorKind, Result};
use crate::jscompat;

/// A valid SBML SId.
static SID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*$").expect("pattern"));

/// What fbc-v2 accepts as a chemical formula: letters and digits, opening with
/// a letter. Verified against libSBML, which accepts `Ferredoxin` and `Xy5` but
/// rejects anything carrying a `.` or a `_`.
static FBC_FORMULA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z][A-Za-z0-9]*$").expect("pattern"));

/// Is `id` a valid SBML SId?
pub fn is_sid(id: &str) -> bool {
    SID.is_match(id)
}

/// Escape an identifier into an SId, cobrapy style: every character outside
/// `[A-Za-z0-9_]` becomes `__<code>__` (one escape per UTF-16 code unit, as
/// `charCodeAt` gives them). The type prefix (`R_`, `M_`, `G_`) also rescues
/// the ids that merely start with a digit.
pub fn mangle_id(id: &str, prefix: &str) -> Result<String> {
    let mut out = String::from(prefix);
    for c in id.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else {
            let mut units = [0u16; 2];
            for unit in c.encode_utf16(&mut units) {
                let _ = write!(out, "__{unit}__");
            }
        }
    }
    if !SID.is_match(&out) {
        return Err(Error::new(
            ErrorKind::SbmlWrite,
            format!("Cannot write \"{id}\" as an SBML identifier (got \"{out}\")"),
        ));
    }
    Ok(out)
}

/// Compartment ids are usually already valid (`c`, `e`, `p`), so only the rest are fixed.
fn compartment_sid(id: &str) -> Result<String> {
    if SID.is_match(id) { Ok(id.to_string()) } else { mangle_id(id, "C_") }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Numbers as SBML doubles: no thousands separators, no stray precision.
fn numeric(n: f64) -> String {
    if !n.is_finite() {
        return if n > 0.0 { "INF".into() } else { "-INF".into() };
    }
    js::number_to_string(n)
}

/// ` key="value"` for every pair whose value is present and not blank.
fn attrs(pairs: &[(&str, Option<String>)]) -> String {
    let mut out = String::new();
    for (k, v) in pairs {
        if let Some(v) = v.as_deref().filter(|v| !v.is_empty()) {
            let _ = write!(out, " {k}=\"{}\"", esc(v));
        }
    }
    out
}

fn some(s: &str) -> Option<String> {
    Some(s.to_string())
}

// GPR

/// `and` / `or`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GprOp {
    And,
    Or,
}

impl GprOp {
    pub fn as_str(self) -> &'static str {
        match self {
            GprOp::And => "and",
            GprOp::Or => "or",
        }
    }
}

/// A parsed gene-protein-reaction rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Gpr {
    /// A gene, or `None` where an operator has nothing after it (`a and`), which
    /// the JS parser keeps as `{gene: undefined}`.
    Gene(Option<String>),
    Op { op: GprOp, args: Vec<Gpr> },
}

impl Gpr {
    /// The tree as the JS returns it: `{gene}` or `{op, args}`.
    pub fn to_json(&self) -> Value {
        match self {
            Gpr::Gene(Some(g)) => json!({"gene": g}),
            Gpr::Gene(None) => Value::Object(Map::new()),
            Gpr::Op { op, args } => json!({"op": op.as_str(), "args": args.iter().map(Gpr::to_json).collect::<Vec<_>>()}),
        }
    }
}

static GPR_TOKENS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"\(|\)|[^{}()]+", jscompat::WS_CHARS)).expect("pattern"));

/// Parse a gene-protein-reaction rule into a tree.
///
/// Grammar, with `or` binding loosest, which is how COBRA writes and reads them:
///
/// ```text
/// expr   := term ( 'or' term )*
/// term   := factor ( 'and' factor )*
/// factor := '(' expr ')' | gene
/// ```
///
/// `None` for no rule.
pub fn parse_gpr(rule: &str) -> Option<Gpr> {
    let text = jscompat::js_trim(rule);
    if text.is_empty() {
        return None;
    }
    // `&&` and `|` turn up in hand-written rules. COBRA's own files use the words.
    let tokens: Vec<String> = GPR_TOKENS
        .find_iter(text)
        .map(|m| match m.as_str() {
            "&&" | "&" => "and".to_string(),
            "||" | "|" => "or".to_string(),
            t => t.to_string(),
        })
        .collect();

    struct Parser {
        tokens: Vec<String>,
        at: usize,
    }
    impl Parser {
        fn peek(&self) -> Option<&str> {
            self.tokens.get(self.at).map(String::as_str)
        }
        fn keyword(&self, word: &str) -> bool {
            self.peek().is_some_and(|t| t.to_lowercase() == word)
        }
        fn expr(&mut self) -> Gpr {
            let mut args = vec![self.term()];
            while self.keyword("or") {
                self.at += 1;
                args.push(self.term());
            }
            if args.len() == 1 { args.pop().expect("one") } else { Gpr::Op { op: GprOp::Or, args } }
        }
        fn term(&mut self) -> Gpr {
            let mut args = vec![self.factor()];
            while self.keyword("and") {
                self.at += 1;
                args.push(self.factor());
            }
            if args.len() == 1 { args.pop().expect("one") } else { Gpr::Op { op: GprOp::And, args } }
        }
        fn factor(&mut self) -> Gpr {
            let t = self.peek().map(str::to_string);
            if t.as_deref() == Some("(") {
                self.at += 1;
                let inner = self.expr();
                if self.peek() == Some(")") {
                    self.at += 1;
                }
                return inner;
            }
            self.at += 1;
            Gpr::Gene(t)
        }
    }

    let tree = Parser { tokens, at: 0 }.expr();
    // A rule that parsed to nothing usable (stray punctuation) is no rule at all.
    match tree {
        Gpr::Gene(None) => None,
        other => Some(other),
    }
}

fn incomplete_rule(rule: &str) -> Error {
    // The JS crashes on these with a TypeError. The message here says why.
    Error::new(ErrorKind::SbmlWrite, format!("The gene rule \"{rule}\" is incomplete: an operator has nothing after it."))
}

/// Every gene id a rule mentions, in order of first appearance. Fails on a
/// rule with an operator left dangling (`a and`), as the JS does.
pub fn genes_in_gpr(rule: &str) -> Result<Vec<String>> {
    fn walk(node: &Gpr, out: &mut IndexSet<String>) -> bool {
        match node {
            Gpr::Gene(Some(g)) => {
                out.insert(g.clone());
                true
            }
            Gpr::Gene(None) => false,
            Gpr::Op { args, .. } => args.iter().all(|a| walk(a, out)),
        }
    }
    let mut out = IndexSet::new();
    if let Some(tree) = parse_gpr(rule) {
        if !walk(&tree, &mut out) {
            return Err(incomplete_rule(rule));
        }
    }
    Ok(out.into_iter().collect())
}

fn gpr_xml(node: &Gpr, indent: &str, rule: &str) -> Result<String> {
    match node {
        Gpr::Gene(Some(g)) => Ok(format!("{indent}<fbc:geneProductRef fbc:geneProduct=\"{}\"/>", esc(&mangle_id(g, "G_")?))),
        Gpr::Gene(None) => Err(incomplete_rule(rule)),
        Gpr::Op { op, args } => {
            let deeper = format!("{indent}  ");
            let inner = args.iter().map(|a| gpr_xml(a, &deeper, rule)).collect::<Result<Vec<_>>>()?.join("\n");
            let op = op.as_str();
            Ok(format!("{indent}<fbc:{op}>\n{inner}\n{indent}</fbc:{op}>"))
        }
    }
}

// Bounds

struct BoundParameters {
    parameters: Vec<(String, f64)>,
    per_reaction: HashMap<String, (String, String)>,
}

/// Flux bounds are parameters in fbc-v2, not attributes, and `fbc:strict`
/// wants every one of them constant and defined. Pooling the values keeps the
/// file the size of the model rather than the size of twice its reaction
/// count: a genome-scale model uses a handful of distinct bounds for thousands
/// of reactions. The three shared names are cobrapy's, so its files and ours
/// read alike.
fn bound_parameters(model: &Model) -> Result<BoundParameters> {
    // Keyed as a JS Map keys numbers: -0 and 0 are one key.
    let key = |v: f64| if v == 0.0 { 0u64 } else if v.is_nan() { f64::NAN.to_bits() } else { v.to_bits() };
    let mut pool: HashMap<u64, String> = HashMap::new();
    let mut parameters = Vec::new();
    let mut name_for = |value: f64, reaction_id: &str, side: &str| -> Result<String> {
        if let Some(id) = pool.get(&key(value)) {
            return Ok(id.clone());
        }
        let id = if value == -1000.0 {
            "cobra_default_lb".to_string()
        } else if value == 1000.0 {
            "cobra_default_ub".to_string()
        } else if value == 0.0 {
            "cobra_0_bound".to_string()
        } else {
            mangle_id(&format!("{reaction_id}_{side}_bound"), "R_")?
        };
        pool.insert(key(value), id.clone());
        parameters.push((id.clone(), value));
        Ok(id)
    };
    let mut per_reaction = HashMap::new();
    for rxn in model.reactions.values() {
        let lower = name_for(rxn.lower_bound, &rxn.id, "lower")?;
        let upper = name_for(rxn.upper_bound, &rxn.id, "upper")?;
        per_reaction.insert(rxn.id.clone(), (lower, upper));
    }
    Ok(BoundParameters { parameters, per_reaction })
}

// Writing

const NS_CORE: &str = "http://www.sbml.org/sbml/level3/version1/core";
const NS_FBC: &str = "http://www.sbml.org/sbml/level3/version1/fbc/version2";
const NS_HTML: &str = "http://www.w3.org/1999/xhtml";

/// Options of [`to_sbml`].
#[derive(Debug, Clone)]
pub struct SbmlWriteOptions {
    /// Name of the objective element (`obj`).
    pub objective_id: String,
    /// `maximize` or `minimize`.
    pub objective_type: String,
}

impl Default for SbmlWriteOptions {
    fn default() -> Self {
        SbmlWriteOptions { objective_id: "obj".into(), objective_type: "maximize".into() }
    }
}

/// The id and name of a model gene entry (`{id, name, …}` or a bare string).
fn gene_id_name(g: &Value) -> (String, Option<Value>) {
    match g {
        Value::String(s) => (s.clone(), None),
        Value::Object(o) => (
            o.get("id").map_or_else(|| "undefined".to_string(), js::to_string),
            o.get("name").cloned(),
        ),
        other => (js::to_string(other), None),
    }
}

/// Serialize a model as SBML L3V1 + fbc-v2, byte for byte what the JS writes.
pub fn to_sbml(model: &Model, opts: &SbmlWriteOptions) -> Result<String> {
    let BoundParameters { parameters, per_reaction } = bound_parameters(model)?;

    let mut compartments = Vec::new();
    for id in model.compartment_ids() {
        let name = model.compartment_names.get(&id).filter(|n| !n.is_empty() && **n != id).cloned();
        compartments.push(format!(
            "    <compartment{}/>",
            attrs(&[("id", Some(compartment_sid(&id)?)), ("name", name), ("constant", some("true"))])
        ));
    }
    let compartments = compartments.join("\n");

    let mut species = Vec::new();
    for met in model.metabolites.values() {
        // fbc:chemicalFormula takes letters and digits only. Published models
        // carry formulas that do not qualify: a biomass pseudo-metabolite with
        // fractional counts (`C37.024041172583H67.8…`) and macromolecule
        // placeholders with an underscore (`Apo_acp`). libSBML rejects those
        // outright, so they go into the species' notes instead, under COBRA's
        // own `FORMULA:` key, where Level 2 files keep every formula and where
        // the reader already looks. Nothing is lost and the file stays conformant.
        let conformant = FBC_FORMULA.is_match(&met.formula);
        let head = format!(
            "    <species{}",
            attrs(&[
                ("id", Some(mangle_id(&met.id, "M_")?)),
                ("name", (!met.name.is_empty() && met.name != met.id).then(|| met.name.clone())),
                ("compartment", Some(compartment_sid(&met.compartment)?)),
                ("hasOnlySubstanceUnits", some("false")),
                ("boundaryCondition", some("false")),
                ("constant", some("false")),
                ("fbc:charge", met.charge.is_finite().then(|| js::number_to_string(met.charge))),
                ("fbc:chemicalFormula", conformant.then(|| met.formula.clone())),
            ])
        );
        if conformant || met.formula.is_empty() {
            species.push(format!("{head}/>"));
        } else {
            species.push(format!(
                "{head}>\n      <notes>\n        <html:p xmlns:html=\"{NS_HTML}\">FORMULA: {}</html:p>\n      </notes>\n    </species>",
                esc(&met.formula)
            ));
        }
    }
    let species = species.join("\n");

    // SBO:0000626 is "flux bound". Published FBC files carry it on these, and it
    // is what tells a reader these parameters are bounds rather than kinetics.
    let parameter_xml = parameters
        .iter()
        .map(|(id, value)| {
            format!(
                "    <parameter{}/>",
                attrs(&[
                    ("id", Some(id.clone())),
                    ("value", Some(numeric(*value))),
                    ("constant", some("true")),
                    ("sboTerm", some("SBO:0000626")),
                    ("units", some("mmol_per_gDW_per_hr")),
                ])
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    // Gene products: everything the rules mention, plus anything the model
    // lists on its own. An id appearing twice is one product.
    let mut gene_names: HashMap<String, Option<Value>> = HashMap::new();
    let mut gene_ids: IndexSet<String> = IndexSet::new();
    for g in &model.genes {
        let (id, name) = gene_id_name(g);
        gene_names.insert(id.clone(), name);
        gene_ids.insert(id);
    }
    for rxn in model.reactions.values() {
        for gene in genes_in_gpr(&rxn.gene_reaction_rule)? {
            gene_ids.insert(gene);
        }
    }

    let mut reaction_xml = Vec::new();
    for rxn in model.reactions.values() {
        let side = |entries: Vec<(&String, f64)>, tag: &str| -> Result<String> {
            if entries.is_empty() {
                return Ok(String::new());
            }
            let mut refs = Vec::new();
            for (mid, coef) in entries {
                refs.push(format!(
                    "        <speciesReference{}/>",
                    attrs(&[
                        ("species", Some(mangle_id(mid, "M_")?)),
                        ("stoichiometry", Some(numeric(coef.abs()))),
                        ("constant", some("true")),
                    ])
                ));
            }
            Ok(format!("\n      <listOf{tag}>\n{}\n      </listOf{tag}>", refs.join("\n")))
        };

        let (lower, upper) = &per_reaction[&rxn.id];
        let gpr_block = match parse_gpr(&rxn.gene_reaction_rule) {
            Some(tree) => format!(
                "\n      <fbc:geneProductAssociation>\n{}\n      </fbc:geneProductAssociation>",
                gpr_xml(&tree, "        ", &rxn.gene_reaction_rule)?
            ),
            None => String::new(),
        };
        // The subsystem has no fbc home. COBRA's own Level 2 convention is a
        // note, and the reader reads it back from there.
        let notes = if rxn.subsystem.is_empty() {
            String::new()
        } else {
            format!(
                "\n      <notes>\n        <html:p xmlns:html=\"{NS_HTML}\">SUBSYSTEM: {}</html:p>\n      </notes>",
                esc(&rxn.subsystem)
            )
        };
        let body = format!(
            "{notes}{}{}{gpr_block}",
            side(rxn.substrates().collect(), "Reactants")?,
            side(rxn.products().collect(), "Products")?
        );
        let head = format!(
            "    <reaction{}>",
            attrs(&[
                ("id", Some(mangle_id(&rxn.id, "R_")?)),
                ("name", (!rxn.name.is_empty() && rxn.name != rxn.id).then(|| rxn.name.clone())),
                ("reversible", some(if rxn.lower_bound < 0.0 { "true" } else { "false" })),
                ("fast", some("false")),
                ("fbc:lowerFluxBound", Some(lower.clone())),
                ("fbc:upperFluxBound", Some(upper.clone())),
            ])
        );
        reaction_xml.push(format!("{head}{body}\n    </reaction>"));
    }
    let reaction_xml = reaction_xml.join("\n");

    let mut flux_objectives = Vec::new();
    for (id, coefficient) in model.objective_reactions() {
        flux_objectives.push(format!(
            "        <fbc:fluxObjective{}/>",
            attrs(&[("fbc:reaction", Some(mangle_id(&id, "R_")?)), ("fbc:coefficient", Some(numeric(coefficient)))])
        ));
    }
    let flux_objectives = flux_objectives.join("\n");

    let mut gene_products = Vec::new();
    for id in &gene_ids {
        let name = match gene_names.get(id) {
            Some(Some(Value::String(n))) if n == id => None,
            Some(Some(n)) if js::truthy(n) => Some(js::to_string(n)),
            _ => None,
        };
        gene_products.push(format!(
            "    <fbc:geneProduct{}/>",
            attrs(&[("fbc:id", Some(mangle_id(id, "G_")?)), ("fbc:label", Some(id.clone())), ("fbc:name", name)])
        ));
    }
    let gene_products = gene_products.join("\n");

    let block = |open: &str, inner: &str, close: &str| {
        if inner.is_empty() { String::new() } else { format!("\n  {open}\n{inner}\n  {close}") }
    };

    // Published files write the model's id plain (`iAF1260`), so only the ones
    // SBML would reject get the prefix.
    let model_id = if SID.is_match(&model.id) { model.id.clone() } else { mangle_id(&model.id, "M_")? };
    let model_attrs = attrs(&[
        ("id", Some(model_id)),
        ("name", (!model.name.is_empty() && model.name != model.id).then(|| model.name.clone())),
        ("fbc:strict", some("true")),
    ]);
    let objective_id = esc(&opts.objective_id);
    let objective_type = esc(&opts.objective_type);
    let flux_list = if flux_objectives.is_empty() {
        String::new()
    } else {
        format!("        <fbc:listOfFluxObjectives>\n{flux_objectives}\n        </fbc:listOfFluxObjectives>\n")
    };

    let mut out = String::with_capacity(reaction_xml.len() + species.len() + 4096);
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    let _ = writeln!(out, "<sbml xmlns=\"{NS_CORE}\" xmlns:fbc=\"{NS_FBC}\" level=\"3\" version=\"1\" fbc:required=\"false\">");
    let _ = writeln!(out, "  <model{model_attrs}>");
    out.push_str(concat!(
        "    <listOfUnitDefinitions>\n",
        "      <unitDefinition id=\"mmol_per_gDW_per_hr\">\n",
        "        <listOfUnits>\n",
        "          <unit kind=\"mole\" exponent=\"1\" scale=\"-3\" multiplier=\"1\"/>\n",
        "          <unit kind=\"gram\" exponent=\"-1\" scale=\"0\" multiplier=\"1\"/>\n",
        "          <unit kind=\"second\" exponent=\"-1\" scale=\"0\" multiplier=\"3600\"/>\n",
        "        </listOfUnits>\n",
        "      </unitDefinition>\n",
        "    </listOfUnitDefinitions>",
    ));
    out.push_str(&block("<listOfCompartments>", &compartments, "</listOfCompartments>"));
    out.push_str(&block("<listOfSpecies>", &species, "</listOfSpecies>"));
    out.push_str(&block("<listOfParameters>", &parameter_xml, "</listOfParameters>"));
    out.push_str(&block("<listOfReactions>", &reaction_xml, "</listOfReactions>"));
    let _ = write!(out, "\n    <fbc:listOfObjectives fbc:activeObjective=\"{objective_id}\">\n");
    let _ = writeln!(out, "      <fbc:objective fbc:id=\"{objective_id}\" fbc:type=\"{objective_type}\">");
    out.push_str(&flux_list);
    out.push_str("      </fbc:objective>\n");
    out.push_str("    </fbc:listOfObjectives>");
    out.push_str(&block("<fbc:listOfGeneProducts>", &gene_products, "</fbc:listOfGeneProducts>"));
    out.push_str("\n  </model>\n</sbml>\n");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gene_rules_parse_with_or_binding_loosest() {
        let p = |s: &str| parse_gpr(s).map(|t| t.to_json());
        assert_eq!(p("b0001"), Some(json!({"gene": "b0001"})));
        assert_eq!(
            p("a and b or c"),
            Some(json!({"op": "or", "args": [{"op": "and", "args": [{"gene": "a"}, {"gene": "b"}]}, {"gene": "c"}]}))
        );
        assert_eq!(
            p("a or b and c"),
            Some(json!({"op": "or", "args": [{"gene": "a"}, {"op": "and", "args": [{"gene": "b"}, {"gene": "c"}]}]}))
        );
        assert_eq!(
            p("(a or b) and c"),
            Some(json!({"op": "and", "args": [{"op": "or", "args": [{"gene": "a"}, {"gene": "b"}]}, {"gene": "c"}]}))
        );
        assert_eq!(parse_gpr(""), None);
        assert_eq!(parse_gpr("   "), None);
        assert_eq!(genes_in_gpr("(a and b) or a").unwrap(), ["a", "b"]);
        assert!(genes_in_gpr("a and").is_err());
    }

    #[test]
    fn identifiers_become_valid_sids() {
        for id in ["12ethd[c_Ba]", "EX_12ppd-R[u]tr_Bt", "PFK(ppi)_Bt", "12ppd-R_e", "EX_glc__D_e", "BIOMASS_Ec_iAF1260_core_59p81M", "3'5'-cyclic"] {
            assert!(is_sid(&mangle_id(id, "M_").unwrap()), "{id}");
        }
        assert_eq!(mangle_id("a😀", "M_").unwrap(), "M_a__55357____56832__");
        assert!(mangle_id("", "").is_err());
    }
}
