//! Port of `src/communities/steadycom-ecoli.js`.
//!
//! Published community: four auxotrophic Escherichia coli strains (SteadyCom).
//!
//! > Chan SHJ, Simons MN, Maranas CD (2017) SteadyCom: Predicting microbial
//! > abundances while ensuring community stability.
//! > PLoS Comput Biol 13(5): e1005539.  doi:10.1371/journal.pcbi.1005539
//!
//! Everything here is transcribed from the authors' own COBRA Toolbox tutorial
//! (opencobra/COBRA.tutorials, analysis/steadyCom/tutorial_steadyCom.m), not
//! inferred from the paper's prose. The tutorial's MATLAB is quoted beside each
//! constraint so the two can be checked line by line.
//!
//! The community is four E. coli, each a double amino-acid auxotroph that is also
//! unable to export one amino acid it can still make. Only two auxotrophy pairs
//! exist, each used twice. What separates Ec1 from Ec4 is which amino acid it
//! cannot export. No member grows alone, so the four must feed each other.
//!
//! ```text
//! strain  auxotroph for  blocked reactions   cannot export  therefore supplies
//! Ec1     Lys + Met      DAPDC, HSST         Phe (PHEt2rpp)  Arg
//! Ec2     Arg + Phe      ARGSL, PPNDH        Met (METt3pp)   Lys
//! Ec3     Arg + Phe      ARGSL, PPNDH        Lys (LYSt3pp)   Met
//! Ec4     Lys + Met      DAPDC, HSST         Arg (ARGt3pp)   Phe
//! ```
//!
//! The tutorial names its MATLAB variables after genes while they hold reaction
//! ids: `ilvE` holds PPNDH, prephenate dehydratase. The auxotrophy is imposed on
//! PPNDH, not on a transaminase.

use std::sync::Arc;

use coxeter_core::Model;
use coxeter_core::js;
use coxeter_io::sbml::{SbmlReadOptions, parse_cobra_sbml};
use coxeter_solver::steadycom::{Member, SteadyComSpec};
use indexmap::IndexMap;
use serde_json::{Value, json};

use crate::error::{Error, ErrorKind, Result};

fn fail(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::SteadyComEcoli, message)
}

/// A paper, a procedure or a model file, as `PROVENANCE` records it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EcoliPaper {
    pub title: &'static str,
    pub authors: &'static str,
    pub year: u32,
    pub journal: &'static str,
    pub doi: &'static str,
    pub framework: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EcoliProcedure {
    pub source: &'static str,
    pub path: &'static str,
    pub note: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EcoliModelSource {
    pub id: &'static str,
    pub organism: &'static str,
    pub reference: &'static str,
    pub file: &'static str,
    pub source: &'static str,
    pub note: &'static str,
}

/// Where the inputs come from, so a reader can fetch the same files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EcoliProvenance {
    pub paper: EcoliPaper,
    pub procedure: EcoliProcedure,
    pub model: EcoliModelSource,
}

impl EcoliProvenance {
    pub fn to_json(&self) -> Value {
        let (p, r, m) = (&self.paper, &self.procedure, &self.model);
        json!({
            "paper": {"title": p.title, "authors": p.authors, "year": p.year, "journal": p.journal, "doi": p.doi, "framework": p.framework},
            "procedure": {"source": r.source, "path": r.path, "note": r.note},
            "model": {"id": m.id, "organism": m.organism, "reference": m.reference, "file": m.file, "source": m.source, "note": m.note},
        })
    }
}

pub const PROVENANCE: EcoliProvenance = EcoliProvenance {
    paper: EcoliPaper {
        title: "SteadyCom: Predicting microbial abundances while ensuring community stability",
        authors: "Chan SHJ, Simons MN, Maranas CD",
        year: 2017,
        journal: "PLoS Computational Biology",
        doi: "10.1371/journal.pcbi.1005539",
        framework: "SteadyCom",
    },
    procedure: EcoliProcedure {
        source: "opencobra/COBRA.tutorials",
        path: "analysis/steadyCom/tutorial_steadyCom.m",
        note: "The authors' own tutorial for this community; every bound below is quoted from it.",
    },
    model: EcoliModelSource {
        id: "iAF1260",
        organism: "Escherichia coli K-12 MG1655",
        reference: "Feist AM et al. (2007) Mol Syst Biol 3:121. doi:10.1038/msb4100155",
        file: "iAF1260.xml",
        source: "opencobra/m_model_collection, sbml/iAF1260.xml",
        note: "The tutorial loads iAF1260.mat from the COBRA Toolbox; this is the same \
               reconstruction as distributed in SBML by the m_model_collection.",
    },
};

/// The one reaction the tutorial adds to iAF1260.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AddedReaction {
    pub id: &'static str,
    pub tutorial: &'static str,
}

pub const ADDED_REACTION: AddedReaction = AddedReaction {
    id: "METt3pp",
    tutorial: "iAF1260 = addReaction(iAF1260,{'METt3pp',''},'met__L[c] + h[c] => met__L[p] + h[p]');",
};

// iAF1260 spells amino acids `arg-L`, not the newer `arg__L`, while its exchange
// reactions use `EX_arg_L_e` with an underscore. Metabolite and exchange ids are
// therefore kept apart rather than derived from one another.
const AA_MET: &str = "met-L";

/// The amino-acid exchange each strain uses, for reporting cross-feeding
/// (`AMINO_ACID_EXCHANGES`, in the JS object's order).
pub const AMINO_ACID_EXCHANGES: [(&str, &str); 4] =
    [("arg", "EX_arg_L_e"), ("lys", "EX_lys_L_e"), ("met", "EX_met_L_e"), ("phe", "EX_phe_L_e")];

/// `argH`, `lysA`, `metA`, `ilvE` in the tutorial.
const AUXOTROPHY: [(&str, &str); 4] = [("arg", "ARGSL"), ("lys", "DAPDC"), ("met", "HSST"), ("phe", "PPNDH")];
/// `argO`, `lysO`, `yjeH`, `yddG` in the tutorial.
const EXPORTER: [(&str, &str); 4] = [("arg", "ARGt3pp"), ("lys", "LYSt3pp"), ("met", "METt3pp"), ("phe", "PHEt2rpp")];

fn lookup(table: &[(&'static str, &'static str)], aa: &str) -> &'static str {
    table.iter().find(|(k, _)| *k == aa).map(|(_, v)| *v).expect("an amino acid of the tables")
}

/// The exchange reaction of an amino acid (`arg` → `EX_arg_L_e`).
pub fn amino_acid_exchange(aa: &str) -> Option<&'static str> {
    AMINO_ACID_EXCHANGES.iter().find(|(k, _)| *k == aa).map(|(_, v)| *v)
}

/// One strain of the community.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Strain {
    pub id: &'static str,
    pub auxotroph_for: [&'static str; 2],
    pub cannot_export: &'static str,
    pub supplies: &'static str,
}

impl Strain {
    pub fn to_json(&self) -> Value {
        json!({"id": self.id, "auxotrophFor": self.auxotroph_for, "cannotExport": self.cannot_export, "supplies": self.supplies})
    }
}

pub const STRAINS: [Strain; 4] = [
    Strain { id: "Ec1", auxotroph_for: ["lys", "met"], cannot_export: "phe", supplies: "arg" },
    Strain { id: "Ec2", auxotroph_for: ["arg", "phe"], cannot_export: "met", supplies: "lys" },
    Strain { id: "Ec3", auxotroph_for: ["arg", "phe"], cannot_export: "lys", supplies: "met" },
    Strain { id: "Ec4", auxotroph_for: ["lys", "met"], cannot_export: "arg", supplies: "phe" },
];

/// `exRate = 1`: the most of a cross-fed amino acid any member may take up.
pub const EX_RATE: f64 = 1.0;

/// `EcCom.ub(EcCom.indCom.EXsp(:)) = 1000;`: every member may secrete anything up
/// to 1000, which is also COBRA's default flux bound.
pub const EXCHANGE_UPPER_BOUND: f64 = 1000.0;

/// The growth rate the tutorial reports for this community, and therefore the
/// number a reproduction has to land on.
pub const EXPECTED_GROWTH_RATE: f64 = 0.73599;

/// Documented departure from the source file.
///
/// The Feist 2007 SBML writes 999999 where COBRA writes 1000 to mean "no limit",
/// and SteadyCom turns every bound into a coefficient: `V - UB*X <= 0`. A
/// coefficient of 1e6 beside a growth rate of 1e-6 spans twelve orders of
/// magnitude, which is enough on its own to make GLPK abandon the LP (GLP_UNDEF).
/// These bounds are therefore read as the infinity they stand for and rewritten to
/// COBRA's 1000.
///
/// This is checked, not assumed, by test/steadycom.test.mjs: a single strain grows
/// at exactly the same rate either way, and so does the community at a fixed
/// growth rate, because no reaction carrying mass ever approaches 1000 when
/// glucose is capped at 8 mmol/gDW/h. The only fluxes that reached 999999 were
/// futile cycles, which carry no net mass.
pub const SENTINEL_BOUND: f64 = 1e5;

/// One strain, built.
#[derive(Debug, Clone)]
pub struct EcoliMember {
    pub id: String,
    pub strain: Strain,
    pub model: Arc<Model>,
    pub biomass_reaction: Option<String>,
    /// Exchanged metabolite (`glc`) → the strain's exchange (`EX_glc_e`).
    pub exchange_for: IndexMap<String, String>,
}

/// The SteadyCom problem for this community (`buildCommunity`'s result).
#[derive(Debug, Clone)]
pub struct EcoliCommunity {
    pub members: Vec<EcoliMember>,
    /// What the community may take up, per exchanged metabolite (positive).
    pub community_uptake: IndexMap<String, f64>,
    pub total_biomass: f64,
    /// iAF1260 as read, with METt3pp added, before any strain's changes.
    pub document: Value,
}

impl EcoliCommunity {
    /// The spec `steady_com` takes.
    pub fn steady_com_spec(&self) -> Result<SteadyComSpec> {
        let mut members = Vec::new();
        for m in &self.members {
            let Some(biomass) = m.biomass_reaction.clone() else {
                return Err(fail(format!("{}: iAF1260 has no biomass reaction", m.id)));
            };
            members.push(Member {
                id: m.id.clone(),
                name: None,
                biomass_reaction: biomass,
                atp_maintenance: None,
                model: Some(m.model.clone()),
                exchange_for: m.exchange_for.clone(),
            });
        }
        Ok(SteadyComSpec {
            members,
            total_biomass: self.total_biomass,
            biomass_cap: None,
            community_uptake: self.community_uptake.clone(),
            community_exchange_bounds: IndexMap::new(),
            joint: None,
        })
    }
}

/// `EX_glc_e` -> `glc`: the metabolite an exchange reaction trades.
pub fn exchange_base(rid: &str) -> String {
    coxeter_solver::steadycom_runtime::exchange_base(rid)
}

fn exchange_index(model: &Model) -> IndexMap<String, String> {
    let mut out = IndexMap::new();
    for rid in model.reactions.keys() {
        if rid.starts_with("EX_") {
            out.insert(exchange_base(rid), rid.clone());
        }
    }
    js::js_key_order(&mut out);
    out
}

/// Build the SteadyCom problem for this community from the contents of
/// iAF1260.xml.
pub fn build_community(xml: &str) -> Result<EcoliCommunity> {
    let mut doc = parse_cobra_sbml(xml, &SbmlReadOptions { sentinel_bound: SENTINEL_BOUND, ..Default::default() });
    let Some(reactions) = doc.get_mut("reactions").and_then(Value::as_array_mut) else {
        return Err(fail("iAF1260.xml holds no reactions"));
    };
    reactions.push(json!({
        "id": ADDED_REACTION.id,
        "name": "L-methionine export via proton antiport (added by the SteadyCom tutorial)",
        "metabolites": {format!("{AA_MET}_c"): -1, "h_c": -1, format!("{AA_MET}_p"): 1, "h_p": 1},
        "lower_bound": 0,
        "upper_bound": EXCHANGE_UPPER_BOUND,
    }));

    let mut members = Vec::new();
    for strain in &STRAINS {
        let mut copy = doc.clone();
        let reactions = copy["reactions"].as_array_mut().expect("checked above");
        // `new Map(copy.reactions.map((r) => [r.id, r]))`: the last of a repeated id wins.
        let mut by_id: IndexMap<String, usize> = IndexMap::new();
        for (i, r) in reactions.iter().enumerate() {
            by_id.insert(js::to_string(r.get("id").unwrap_or(&Value::Null)), i);
        }
        let block = |reactions: &mut Vec<Value>, rid: &str| -> Result<()> {
            let Some(&i) = by_id.get(rid) else {
                return Err(fail(format!("{}: reaction {rid} is not in iAF1260", strain.id)));
            };
            reactions[i]["lower_bound"] = json!(0);
            reactions[i]["upper_bound"] = json!(0);
            Ok(())
        };

        // changeRxnBounds(Ec1, [lysA; metA; yddG], 0, 'b') and its three siblings.
        for aa in strain.auxotroph_for {
            block(reactions, lookup(&AUXOTROPHY, aa))?;
        }
        block(reactions, lookup(&EXPORTER, strain.cannot_export))?;

        // EcCom.ub(EXsp(:)) = 1000: secretion of anything, capped at 1000.
        for r in reactions.iter_mut() {
            if r.get("id").and_then(Value::as_str).is_some_and(|id| id.starts_with("EX_")) {
                r["upper_bound"] = json!(EXCHANGE_UPPER_BOUND);
            }
        }

        // Only the amino acids a strain lacks may be taken up, and only at exRate.
        for (aa, ex_id) in AMINO_ACID_EXCHANGES {
            let Some(&i) = by_id.get(ex_id) else {
                return Err(fail(format!("{}: no exchange reaction {ex_id}", strain.id)));
            };
            reactions[i]["lower_bound"] = json!(if strain.auxotroph_for.contains(&aa) { -EX_RATE } else { 0.0 });
        }

        let model = Model::from_value(&copy)?;
        members.push(EcoliMember {
            id: strain.id.to_string(),
            strain: *strain,
            biomass_reaction: model.biomass_reaction_id(),
            exchange_for: exchange_index(&model),
            model: Arc::new(model),
        });
    }

    // EcCom.lb(EXcom(:,1)) = lbEx: the community takes up what iAF1260 itself
    // allows, so the medium is the model's own and nothing is invented here.
    let mut community_uptake = IndexMap::new();
    for r in doc["reactions"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
        let id = r.get("id").and_then(Value::as_str).unwrap_or("");
        let lower = r.get("lower_bound").map_or(f64::NAN, js::to_number);
        if id.starts_with("EX_") && lower < 0.0 {
            community_uptake.insert(exchange_base(id), -lower);
        }
    }
    js::js_key_order(&mut community_uptake);

    Ok(EcoliCommunity { members, community_uptake, total_biomass: 1.0, document: doc })
}
