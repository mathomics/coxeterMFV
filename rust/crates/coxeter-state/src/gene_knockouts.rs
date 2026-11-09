//! Gene knockouts (the "Gene knockouts" section of `store.js`): knocking out
//! genes closes, on top of the overrides, every reaction whose gene rule they
//! make false. Rules are read with `coxeter_io::gene_rules`, the evaluator
//! the Zuñiga case uses.

use std::sync::{Arc, Mutex};

use coxeter_core::community::namespaced;
use coxeter_core::js;
use coxeter_io::gene_rules::{gene_knockout_reactions, gene_rule_genes};
use indexmap::{IndexMap, IndexSet};
use serde_json::Value;

use crate::error::Result;
use crate::store::Store;
use crate::types::*;
use crate::util::normalize_search;

/// One gene the community's reaction rules name.
#[derive(Debug, Clone, PartialEq)]
pub struct GeneInfo {
    /// `species@gene`, or the gene id for a reaction without a species.
    pub key: String,
    pub gene: String,
    pub species_id: Option<String>,
    pub name: String,
    /// The merged reactions whose rule names it.
    pub reactions: Vec<String>,
}

/// `geneIndex()`: every gene by key, and every non-empty rule by reaction.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GeneIndex {
    pub genes: IndexMap<String, GeneInfo>,
    pub rules: IndexMap<String, String>,
}

/// Reaction id → the knocked-out genes (keys) its rule names.
pub type GeneClosures = IndexMap<String, Vec<String>>;

/// One result of `geneSearch()`.
#[derive(Debug, Clone, PartialEq)]
pub struct GeneSearchResult {
    pub gene: GeneInfo,
    pub knocked_out: bool,
    /// What knocking it out closes (see `gene_knockout_effect`).
    pub closes: Vec<String>,
}

/// One gene knockout as the Changes list shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct GeneKnockoutChange {
    pub key: String,
    pub gene: String,
    pub species_id: Option<String>,
    pub title: String,
    /// The reactions closed through this gene.
    pub reactions: Vec<String>,
}

/// Per built problem: the gene index, and the reactions the knockouts in
/// force close (the JS keeps the index in a WeakMap on the merged model).
#[derive(Default)]
pub(crate) struct GeneCaches {
    index: Mutex<Option<(u64, Arc<GeneIndex>)>>,
    closed: Mutex<Option<(u64, String, Arc<GeneClosures>)>>,
}

impl Store {
    /// `geneIndex()`: every gene the community's reaction rules name, keyed
    /// `species@gene` (merged reactions keep their species' own rules, so two
    /// strains that share a gene id are told apart). Cached per built model.
    pub fn gene_index(&self) -> Arc<GeneIndex> {
        let Some(community) = self.community.as_ref().filter(|_| !self.is_zuniga_study()) else {
            return Arc::new(GeneIndex::default());
        };
        let mut cache = self.gene_caches.index.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, value)) = cache.as_ref().filter(|(serial, _)| *serial == self.problem_serial) {
            return value.clone();
        }
        let mut names: IndexMap<String, String> = IndexMap::new();
        for sp in &self.species {
            for gene in self.models.get(&sp.file).map(|m| m.genes.as_slice()).unwrap_or_default() {
                let id = gene.get("id").map(js::to_string).unwrap_or_default();
                let name = gene.get("name").filter(|n| js::truthy(n)).map_or_else(|| id.clone(), js::to_string);
                names.insert(namespaced(&sp.id, &id), name);
            }
        }
        let mut value = GeneIndex::default();
        for (rid, reaction) in &community.merged.reactions {
            let rule = &reaction.gene_reaction_rule;
            if crate::util::js_trim(rule).is_empty() {
                continue;
            }
            let species_id = community.reaction_owner.get(rid).and_then(|o| o.species_id.clone());
            value.rules.insert(rid.clone(), rule.clone());
            for gene in gene_rule_genes(rule) {
                let key = species_id.as_deref().map_or_else(|| gene.clone(), |sp| namespaced(sp, &gene));
                let entry = value.genes.entry(key.clone()).or_insert_with(|| GeneInfo {
                    name: names.get(&key).cloned().unwrap_or_else(|| gene.clone()),
                    key,
                    gene,
                    species_id: species_id.clone(),
                    reactions: Vec::new(),
                });
                entry.reactions.push(rid.clone());
            }
        }
        let value = Arc::new(value);
        *cache = Some((self.problem_serial, value.clone()));
        value
    }

    /// `geneClosedReactionsFor(keys)`: the reactions the genes `keys` close,
    /// each with the knocked-out genes its rule names.
    pub fn gene_closed_reactions_for<'a>(&self, keys: impl IntoIterator<Item = &'a String>) -> GeneClosures {
        let index = self.gene_index();
        // Per species: the genes deleted and the reactions that name one.
        let mut by_species: IndexMap<Option<String>, (IndexSet<String>, IndexSet<String>)> = IndexMap::new();
        for key in keys {
            let Some(gene) = index.genes.get(key) else { continue };
            let entry = by_species.entry(gene.species_id.clone()).or_default();
            entry.0.insert(gene.gene.clone());
            entry.1.extend(gene.reactions.iter().cloned());
        }
        let mut closed = GeneClosures::new();
        for (species_id, (deleted, reactions)) in by_species {
            let rules = reactions.iter().filter_map(|rid| index.rules.get(rid).map(|rule| (rid.as_str(), rule.as_str())));
            for (rid, genes) in gene_knockout_reactions(rules, &|g| deleted.contains(g)).closed {
                let keys = genes.iter().map(|g| species_id.as_deref().map_or_else(|| g.clone(), |sp| namespaced(sp, g))).collect();
                closed.insert(rid, keys);
            }
        }
        closed
    }

    /// `geneClosedReactions()`: the reactions the gene knockouts in force
    /// close. Cached until they or the model change.
    pub fn gene_closed_reactions(&self) -> Arc<GeneClosures> {
        if self.gene_knockouts.is_empty() {
            return Arc::new(GeneClosures::new());
        }
        let mut sorted: Vec<&String> = self.gene_knockouts.iter().collect();
        sorted.sort_by(|a, b| js::code_unit_compare(a, b));
        let signature = sorted.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n");
        let mut cache = self.gene_caches.closed.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, _, value)) = cache.as_ref().filter(|(serial, s, _)| *serial == self.problem_serial && *s == signature) {
            return value.clone();
        }
        let value = Arc::new(self.gene_closed_reactions_for(&self.gene_knockouts));
        *cache = Some((self.problem_serial, signature, value.clone()));
        value
    }

    /// `effectiveOverrides()`: the per-reaction bounds applied over the
    /// medium: the overrides, with every reaction a gene knockout closes held at 0.
    pub fn effective_overrides(&self) -> IndexMap<String, Bounds> {
        let mut out = self.overrides.clone();
        for rid in self.gene_closed_reactions().keys() {
            out.insert(rid.clone(), Bounds { lower: 0.0, upper: 0.0 });
        }
        out
    }

    /// `geneTitle(key)`: a gene as history entries and messages name it: its
    /// id, and its species when there are several.
    pub fn gene_title(&self, key: &str) -> String {
        let index = self.gene_index();
        let Some(gene) = index.genes.get(key) else { return key.to_string() };
        match gene.species_id.as_deref() {
            Some(sp) if self.species.len() > 1 => format!("{} ({sp})", gene.gene),
            _ => gene.gene.clone(),
        }
    }

    /// `geneKnockoutEffect(key)`: what knocking out `key` does on top of the
    /// knockouts in force: the reactions it would close or, when it is out
    /// already, the ones it closes.
    pub fn gene_knockout_effect(&self, key: &str) -> Vec<String> {
        let now = self.gene_closed_reactions();
        if self.gene_knockouts.contains(key) {
            return now.iter().filter(|(_, keys)| keys.iter().any(|k| k == key)).map(|(rid, _)| rid.clone()).collect();
        }
        let key = key.to_string();
        let with = self.gene_closed_reactions_for(self.gene_knockouts.iter().chain(std::iter::once(&key)));
        with.into_keys().filter(|rid| !now.contains_key(rid)).collect()
    }

    /// `geneSearch(query, { limit })`: genes matching a search (every word in
    /// the id, name or species), the knocked-out ones first. An empty query
    /// lists the knocked-out genes.
    pub fn gene_search(&self, query: &str, limit: usize) -> Vec<GeneSearchResult> {
        let normalized = normalize_search(query);
        let words: Vec<&str> = normalized.split_whitespace().collect();
        let index = self.gene_index();
        let mut genes: Vec<&GeneInfo> = index
            .genes
            .values()
            .filter(|g| {
                if words.is_empty() {
                    return self.gene_knockouts.contains(&g.key);
                }
                let text = normalize_search(&format!("{} {} {}", g.gene, g.name, g.species_id.as_deref().unwrap_or("")));
                words.iter().all(|w| text.contains(w))
            })
            .collect();
        genes.sort_by(|a, b| {
            let out = |g: &GeneInfo| self.gene_knockouts.contains(&g.key);
            out(b)
                .cmp(&out(a))
                .then_with(|| js::locale_compare(&a.gene, &b.gene))
                .then_with(|| js::locale_compare(a.species_id.as_deref().unwrap_or(""), b.species_id.as_deref().unwrap_or("")))
        });
        genes
            .into_iter()
            .take(limit)
            .map(|g| GeneSearchResult { gene: g.clone(), knocked_out: self.gene_knockouts.contains(&g.key), closes: self.gene_knockout_effect(&g.key) })
            .collect()
    }

    /// `setGeneKnockouts(keys, on)`: knock genes out (`on`) or restore them,
    /// as one undoable edit, and solve. Every reaction whose rule the
    /// knocked-out genes make false is held at 0; an isozyme left keeps it open.
    pub fn set_gene_knockouts(&mut self, keys: &[String], on: bool) -> Result<()> {
        self.assert_published_study_constraints()?;
        let index = self.gene_index();
        let mut targets: Vec<String> = Vec::new();
        for key in keys {
            if index.genes.contains_key(key) && self.gene_knockouts.contains(key) != on && !targets.contains(key) {
                targets.push(key.clone());
            }
        }
        if targets.is_empty() {
            return Ok(());
        }
        self.invalidate_steady_com();
        let names: Vec<String> = targets.iter().map(|k| self.gene_title(k)).collect();
        let label = format!(
            "{} gene{} {}{}",
            if on { "Knock out" } else { "Restore" },
            if targets.len() == 1 { "" } else { "s" },
            names.iter().take(3).cloned().collect::<Vec<_>>().join(", "),
            if names.len() > 3 { ", …" } else { "" }
        );
        self.change_constraints(&label, None, |s| {
            for key in &targets {
                if on {
                    s.gene_knockouts.insert(key.clone());
                } else {
                    s.gene_knockouts.shift_remove(key);
                }
            }
        });
        self.apply_overrides();
        self.solve();
        Ok(())
    }

    /// `toggleGeneKnockout(key)`: knock a gene out, or restore it when it is
    /// out, and solve.
    pub fn toggle_gene_knockout(&mut self, key: &str) -> Result<()> {
        let on = !self.gene_knockouts.contains(key);
        self.set_gene_knockouts(&[key.to_string()], on)
    }

    /// `geneKnockoutChanges()`: the gene knockouts as the Changes list shows
    /// them, by gene, with the reactions closed through each.
    pub fn gene_knockout_changes(&self) -> Vec<GeneKnockoutChange> {
        let closed = self.gene_closed_reactions();
        let index = self.gene_index();
        let mut keys: Vec<&String> = self.gene_knockouts.iter().filter(|k| index.genes.contains_key(*k)).collect();
        keys.sort_by(|a, b| js::code_unit_compare(a, b));
        keys.into_iter()
            .map(|key| {
                let gene = &index.genes[key];
                GeneKnockoutChange {
                    key: key.clone(),
                    gene: gene.gene.clone(),
                    species_id: gene.species_id.clone(),
                    title: self.gene_title(key),
                    reactions: closed.iter().filter(|(_, ks)| ks.contains(key)).map(|(rid, _)| rid.clone()).collect(),
                }
            })
            .collect()
    }

    /// The gene knockouts in force, sorted, as files write them.
    pub fn sorted_gene_knockouts(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.gene_knockouts.iter().cloned().collect();
        keys.sort_by(|a, b| js::code_unit_compare(a, b));
        keys
    }

    /// `geneKnockoutsFrom(raw)`: gene keys read from a file: the ones this
    /// community has; the others are reported and left out.
    pub(crate) fn gene_knockouts_from(&mut self, raw: Option<&Value>) -> IndexSet<String> {
        let index = self.gene_index();
        let keys: Vec<String> = raw.and_then(Value::as_array).map(|list| list.iter().map(js::to_string).collect()).unwrap_or_default();
        let known: IndexSet<String> = keys.iter().filter(|k| index.genes.contains_key(*k)).cloned().collect();
        let dropped = keys.iter().filter(|k| !index.genes.contains_key(*k)).count();
        if dropped > 0 {
            self.add_log(
                StatusState::Warn,
                &format!("{dropped} knocked-out gene(s) in the file are not in this model; they were left out."),
                LogKind::Message,
            );
        }
        known
    }
}
