//! Port of `src/omics/community-omics.js`: the community metatranscriptomics
//! layer.
//!
//! Accepts long tables (species/MAG, gene, condition, expression) and converts
//! them to the common wide omics dataset consumed by Coxeter's GPR mapper.
//! Also reports mapping coverage by species and condition.

use std::borrow::Borrow;

use coxeter_core::Model;
use coxeter_core::js;
use indexmap::{IndexMap, IndexSet};
use serde_json::{Value, json};

use crate::community_view::CommunityView;
use crate::error::{Error, ErrorKind, Result};
use crate::jsutil::{js_trim, number, split_ws};
use crate::overlay::{
    OmicsCsvOptions, OmicsDataset, OmicsRow, OmicsSpecies, normalize_header, omics_key, parse_delimited, parse_omics_csv,
    reaction_omics_scores,
};

const SPECIES: [&str; 6] = ["species", "organism", "cell", "mag", "genome", "bin"];
const IDS: [&str; 7] = ["gene", "gene_id", "geneid", "locus", "locus_tag", "identifier", "id"];
const CONDITIONS: [&str; 5] = ["condition", "sample", "state", "group", "treatment"];
const VALUES: [&str; 9] = ["expression", "abundance", "value", "count", "counts", "tpm", "fpkm", "rpkm", "cpm"];

fn fail(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::Omics, message)
}

/// How the table was laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableFormat {
    /// One row per species, gene and condition.
    Long,
    /// One row per gene, one column per condition.
    Wide,
}

impl TableFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            TableFormat::Long => "long",
            TableFormat::Wide => "wide",
        }
    }
}

/// A metatranscriptomics table read into the common dataset.
#[derive(Debug, Clone, PartialEq)]
pub struct Metatranscriptomics {
    pub dataset: OmicsDataset,
    pub format: TableFormat,
}

/// Read a long CSV or TSV (species, gene, condition, expression). A table with
/// no condition or value column is taken to be wide already.
pub fn parse_metatranscriptomics(text: &str, source: Option<&str>) -> Result<Metatranscriptomics> {
    let source = source.unwrap_or("metatranscriptomics.csv");
    let delimiter = detect_delimiter(text);
    let rows = parse_delimited(text, delimiter);
    if rows.len() < 2 {
        return Err(fail("The metatranscriptomics table needs a header row and data."));
    }
    let header: Vec<String> = rows[0].iter().map(|x| js_trim(x).to_string()).collect();
    let norm: Vec<String> = header.iter().map(|h| normalize_header(h)).collect();
    let find = |set: &[&str]| norm.iter().position(|x| set.contains(&x.as_str()));
    let (si, gi, ci, vi) = (find(&SPECIES), find(&IDS), find(&CONDITIONS), find(&VALUES));
    let Some(si) = si else { return Err(fail("Falta una columna species/MAG/organism.")) };
    let Some(gi) = gi else { return Err(fail("Falta una columna gene/gene_id/locus_tag.")) };
    let (Some(ci), Some(vi)) = (ci, vi) else {
        // Already-wide community omics is accepted as a convenience.
        let options = OmicsCsvOptions { source: source.to_string(), data_type: "metatranscriptomics".into() };
        return Ok(Metatranscriptomics { dataset: parse_omics_csv(text, &options)?, format: TableFormat::Wide });
    };

    let mut conditions: IndexSet<String> = IndexSet::new();
    let mut records: IndexMap<String, OmicsRow> = IndexMap::new();
    for row in &rows[1..] {
        let cell = |i: usize| js_trim(row.get(i).map_or("", String::as_str));
        let (species, gene, condition) = (cell(si), cell(gi), cell(ci));
        // `Number('')` is 0: a blank expression cell counts as zero here.
        let value = number(cell(vi));
        if species.is_empty() || gene.is_empty() || condition.is_empty() || !value.is_finite() {
            continue;
        }
        conditions.insert(condition.to_string());
        let record = records.entry(omics_key(species, gene)).or_insert_with(|| OmicsRow {
            id: gene.to_string(),
            species: species.to_string(),
            values: IndexMap::new(),
        });
        record.values.insert(condition.to_string(), value);
        js::js_key_order(&mut record.values);
    }
    if records.is_empty() {
        return Err(fail("No usable metatranscriptomics values were found."));
    }
    Ok(Metatranscriptomics {
        dataset: OmicsDataset {
            source: source.to_string(),
            data_type: "metatranscriptomics".into(),
            id_column: header[gi].clone(),
            species_column: Some(header[si].clone()),
            conditions: conditions.into_iter().collect(),
            row_count: records.len(),
            values: records,
        },
        format: TableFormat::Long,
    })
}

/// How well one species' model is covered by the table.
#[derive(Debug, Clone, PartialEq)]
pub struct SpeciesMapping {
    pub species: String,
    /// Genes the model lists or its rules name.
    pub genes: usize,
    pub measured_genes: usize,
    /// Reactions with a gene rule.
    pub gpr_reactions: usize,
    /// The most reactions any one condition could score.
    pub mapped_reactions: usize,
    /// Scored reactions per condition.
    pub conditions: IndexMap<String, usize>,
}

/// The mapping report shown after loading community omics.
#[derive(Debug, Clone, PartialEq)]
pub struct MappingReport {
    pub input_rows: usize,
    pub input_species: usize,
    pub model_species: usize,
    pub genes_in_models: usize,
    pub measured_genes: usize,
    pub gpr_reactions: usize,
    pub mapped_reactions: usize,
    pub gene_coverage: f64,
    pub reaction_coverage: f64,
    /// Species the table names that the community does not have, sorted.
    pub unmatched_species: Vec<String>,
    pub species: Vec<SpeciesMapping>,
}

impl MappingReport {
    pub fn to_json(&self) -> Value {
        json!({
            "inputRows": self.input_rows, "inputSpecies": self.input_species, "modelSpecies": self.model_species,
            "genesInModels": self.genes_in_models, "measuredGenes": self.measured_genes,
            "gprReactions": self.gpr_reactions, "mappedReactions": self.mapped_reactions,
            "geneCoverage": self.gene_coverage, "reactionCoverage": self.reaction_coverage,
            "unmatchedSpecies": self.unmatched_species,
            "species": self.species.iter().map(|s| json!({
                "species": s.species, "genes": s.genes, "measuredGenes": s.measured_genes,
                "gprReactions": s.gpr_reactions, "mappedReactions": s.mapped_reactions, "conditions": s.conditions,
            })).collect::<Vec<_>>(),
        })
    }
}

/// Count, per species and per condition, what the table could be mapped to.
pub fn community_omics_mapping_report<M: Borrow<Model>>(
    community: &impl CommunityView,
    species_config: &[OmicsSpecies],
    models: &IndexMap<String, M>,
    dataset: &OmicsDataset,
) -> MappingReport {
    let mut species_rows = Vec::new();
    let (mut genes_in_models, mut measured_genes, mut mapped_reactions, mut gpr_reactions) = (0, 0, 0, 0);
    // The JS scores every condition again for each species. The scores do not
    // depend on the species, so they are computed once (same result).
    let scored: Vec<(String, IndexMap<String, f64>)> = dataset
        .conditions
        .iter()
        .map(|c| (c.clone(), reaction_omics_scores(community, species_config, models, Some(dataset), c)))
        .collect();
    for sp in species_config {
        let Some(model) = models.get(&sp.file).map(Borrow::borrow) else { continue };
        let mut gene_ids: IndexSet<String> = model
            .genes
            .iter()
            .map(|g| match g.get("id") {
                Some(id) if !id.is_null() => js::to_string(id),
                _ => js::to_string(g),
            })
            .collect();
        // GPR-only genes are common in imperfect reconstructions, so include them.
        for r in model.reactions.values() {
            gene_ids.extend(genes_from_gpr(&r.gene_reaction_rule));
        }
        let measured = gene_ids
            .iter()
            .filter(|g| dataset.values.contains_key(&omics_key(&sp.id, g)) || dataset.values.contains_key(&omics_key("", g)))
            .count();
        let local_gpr = model.reactions.values().filter(|r| !js_trim(&r.gene_reaction_rule).is_empty()).count();
        let mut per_condition: IndexMap<String, usize> = IndexMap::new();
        for (condition, scores) in &scored {
            let count = scores
                .keys()
                .filter(|rid| community.reaction_owner(rid).and_then(|o| o.species_id.as_deref()) == Some(sp.id.as_str()))
                .count();
            per_condition.insert(condition.clone(), count);
        }
        js::js_key_order(&mut per_condition);
        let mapped = per_condition.values().copied().max().unwrap_or(0);
        genes_in_models += gene_ids.len();
        measured_genes += measured;
        gpr_reactions += local_gpr;
        mapped_reactions += mapped;
        species_rows.push(SpeciesMapping {
            species: sp.id.clone(),
            genes: gene_ids.len(),
            measured_genes: measured,
            gpr_reactions: local_gpr,
            mapped_reactions: mapped,
            conditions: per_condition,
        });
    }
    let input_species: IndexSet<&str> = dataset.values.values().map(|r| r.species.as_str()).filter(|s| !s.is_empty()).collect();
    let mut unmatched_species: Vec<String> =
        input_species.iter().filter(|s| !species_config.iter().any(|sp| sp.id == **s)).map(|s| s.to_string()).collect();
    unmatched_species.sort_by(|a, b| js::code_unit_compare(a, b));
    MappingReport {
        input_rows: dataset.row_count,
        input_species: input_species.len(),
        model_species: species_rows.len(),
        genes_in_models,
        measured_genes,
        gpr_reactions,
        mapped_reactions,
        gene_coverage: if genes_in_models > 0 { measured_genes as f64 / genes_in_models as f64 } else { 0.0 },
        reaction_coverage: if gpr_reactions > 0 { mapped_reactions as f64 / gpr_reactions as f64 } else { 0.0 },
        unmatched_species,
        species: species_rows,
    }
}

/// Every word of a gene rule that is not `and`, `or` or a parenthesis.
fn genes_from_gpr(rule: &str) -> Vec<String> {
    let spaced = rule.replace(['(', ')'], " ");
    split_ws(&spaced)
        .into_iter()
        .filter(|x| !x.is_empty() && !x.eq_ignore_ascii_case("and") && !x.eq_ignore_ascii_case("or"))
        .map(str::to_string)
        .collect()
}

/// A tab-separated first line has more tabs than commas.
fn detect_delimiter(text: &str) -> char {
    let first = text.split('\n').next().unwrap_or("");
    let first = first.strip_suffix('\r').unwrap_or(first);
    if first.matches('\t').count() > first.matches(',').count() { '\t' } else { ',' }
}
