//! Port of `src/core/gene-rules.js`: gene rules (GPR), whether a reaction
//! keeps an enzyme once genes are deleted, and which reactions a set of gene
//! knockouts closes. Rules are parsed, never evaluated as code.

use indexmap::IndexMap;

use crate::error::{Error, ErrorKind, Result};
use crate::jscompat::{is_js_ws, js_trim};

fn invalid(message: &str) -> Error {
    Error::new(ErrorKind::InvalidModel, message)
}

/// `String(rule).match(/\(|\)|[^\s()]+/g)`
pub(crate) fn gene_rule_tokens(rule: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let mut start: Option<usize> = None;
    for (i, c) in rule.char_indices() {
        if c == '(' || c == ')' || is_js_ws(c) {
            if let Some(s) = start.take() {
                tokens.push(&rule[s..i]);
            }
            if c == '(' || c == ')' {
                tokens.push(&rule[i..i + 1]);
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(s) = start {
        tokens.push(&rule[s..]);
    }
    tokens
}

fn is_operator(token: &str) -> bool {
    token.eq_ignore_ascii_case("and") || token.eq_ignore_ascii_case("or")
}

struct RuleParser<'a, 'd> {
    tokens: Vec<&'a str>,
    index: usize,
    deleted: &'d dyn Fn(&str) -> bool,
}

impl<'a> RuleParser<'a, '_> {
    fn next(&mut self) -> Option<&'a str> {
        let t = self.tokens.get(self.index).copied();
        self.index += 1;
        t
    }

    fn atom(&mut self) -> Result<bool> {
        let Some(token) = self.next() else { return Err(invalid("Incomplete gene rule.")) };
        if token == "(" {
            let value = self.or()?;
            if self.next() != Some(")") {
                return Err(invalid("Unbalanced gene rule."));
            }
            return Ok(value);
        }
        if token == ")" || is_operator(token) {
            return Err(invalid("Invalid gene rule."));
        }
        Ok(!(self.deleted)(token))
    }

    fn peek_is(&self, word: &str) -> bool {
        self.tokens.get(self.index).is_some_and(|t| t.eq_ignore_ascii_case(word))
    }

    fn and(&mut self) -> Result<bool> {
        let mut value = self.atom()?;
        while self.peek_is("and") {
            self.index += 1;
            let next = self.atom()?;
            value = value && next;
        }
        Ok(value)
    }

    fn or(&mut self) -> Result<bool> {
        let mut value = self.and()?;
        while self.peek_is("or") {
            self.index += 1;
            let next = self.and()?;
            value = value || next;
        }
        Ok(value)
    }
}

/// `evaluateGeneRule(rule, deleted)`: whether a reaction stays functional
/// once the genes `deleted` says are gone, so an alternate isozyme keeps it
/// on. An empty rule is always functional; a malformed one is an error.
pub fn evaluate_gene_rule(rule: &str, deleted: &dyn Fn(&str) -> bool) -> Result<bool> {
    if js_trim(rule).is_empty() {
        return Ok(true);
    }
    let mut parser = RuleParser { tokens: gene_rule_tokens(rule), index: 0, deleted };
    let result = parser.or()?;
    if parser.index != parser.tokens.len() {
        return Err(invalid("Unexpected token in gene rule."));
    }
    Ok(result)
}

/// `geneRuleGenes(rule)`: the gene ids a rule names, once each, in the order
/// they first appear.
pub fn gene_rule_genes(rule: &str) -> Vec<String> {
    let mut genes: Vec<String> = Vec::new();
    for token in gene_rule_tokens(rule) {
        if token == "(" || token == ")" || is_operator(token) || genes.iter().any(|g| g == token) {
            continue;
        }
        genes.push(token.to_string());
    }
    genes
}

/// What `geneKnockoutReactions` finds: the reactions closed, each with the
/// deleted genes its rule names, and the rules that could not be read.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GeneKnockoutEffect {
    pub closed: IndexMap<String, Vec<String>>,
    pub unreadable: Vec<String>,
}

/// `geneKnockoutReactions(rules, deleted)`: the reactions a set of deleted
/// genes closes, each reaction whose rule names one of them and is false
/// without them. A rule that cannot be read is left open and listed in
/// `unreadable`.
pub fn gene_knockout_reactions<'a>(rules: impl IntoIterator<Item = (&'a str, &'a str)>, deleted: &dyn Fn(&str) -> bool) -> GeneKnockoutEffect {
    let mut effect = GeneKnockoutEffect::default();
    for (id, rule) in rules {
        let named: Vec<String> = gene_rule_genes(rule).into_iter().filter(|g| deleted(g)).collect();
        if named.is_empty() {
            continue;
        }
        match evaluate_gene_rule(rule, deleted) {
            Ok(false) => {
                effect.closed.insert(id.to_string(), named);
            }
            Ok(true) => {}
            Err(_) => effect.unreadable.push(id.to_string()),
        }
    }
    effect
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_split_on_parentheses_and_javascript_whitespace() {
        assert_eq!(gene_rule_tokens("(a and b)or\u{FEFF}c"), ["(", "a", "and", "b", ")", "or", "c"]);
        assert_eq!(gene_rule_tokens("  "), Vec::<&str>::new());
    }

    #[test]
    fn and_or_nesting_and_isozymes() {
        let gone = |genes: &'static [&'static str]| move |g: &str| genes.contains(&g);
        assert!(evaluate_gene_rule("", &gone(&["g1"])).unwrap());
        assert!(evaluate_gene_rule("g1 or g2", &gone(&["g1"])).unwrap());
        assert!(!evaluate_gene_rule("g1 and g2", &gone(&["g2"])).unwrap());
        assert!(evaluate_gene_rule("(g1 and g4) or (g5 and g6)", &gone(&["g1"])).unwrap());
        assert!(!evaluate_gene_rule("(g1 and g4) or (g5 and g6)", &gone(&["g1", "g6"])).unwrap());
        assert!(evaluate_gene_rule("g1 OR (g2 AND (g3 or g4))", &gone(&["g1", "g3"])).unwrap());
        assert_eq!(evaluate_gene_rule("g1 and (g2", &gone(&[])).unwrap_err().message, "Unbalanced gene rule.");
        let effect = gene_knockout_reactions(
            [("A", "g1"), ("B", "g1 or g2"), ("C", "g1 and g2"), ("D", "g3"), ("E", "g1 and (")],
            &|g| g == "g1",
        );
        assert_eq!(effect.closed.into_iter().collect::<Vec<_>>(), [("A".to_string(), vec!["g1".to_string()]), ("C".to_string(), vec!["g1".to_string()])]);
        assert_eq!(effect.unreadable, ["E"]);
        assert_eq!(gene_rule_genes("(b1 and b2) or b1"), ["b1", "b2"]);
    }
}
