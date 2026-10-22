//! The map's stylesheet (`assets/map.css`), applied to the element tree.
//!
//! The SVG the renderer draws is only half the picture: the browser styles it
//! with `map.css`, which gives label sizes scaled by `--label-scale`, name
//! plates scaled by `--frame-label-scale` and `--fit`, cofactors drawn lighter,
//! selection outlines, and the small type hidden when zoomed far out. A native
//! front end has no browser, so this module does the browser's part for the
//! map: it parses the stylesheet, matches its selectors against the tree, runs
//! the cascade (presentation attributes, rules by specificity, inline `style`,
//! `!important`), inherits, substitutes `var()` and evaluates `calc()`,
//! `min()`, `max()` and `clamp()`. The result is the computed paint style of
//! every element ([`ComputedStyle`]).
//!
//! What is supported is what map.css (and the `.emc-` rules of app.css) use:
//! type, class, id and attribute selectors, `*`, `:hover` (for the element the
//! caller says is hovered), `:not()`/`:is()`/`:where()` of compound selectors,
//! `:root`, and the descendant, child, `+` and `~` combinators. Other pseudo
//! classes never match; `@media` blocks are kept for the export but do not
//! apply.

use std::collections::HashMap;
use std::sync::LazyLock;

use coxeter_core::js;
use indexmap::IndexMap;

use crate::svg::{Element, Node};

/// `assets/map.css`, as the app ships it.
pub const MAP_CSS: &str = include_str!("../../../../assets/map.css");
/// `assets/app.css`: the app's own stylesheet; a few of its rules style the map.
pub const APP_CSS: &str = include_str!("../../../../assets/app.css");

static MAP_SHEET: LazyLock<StyleSheet> = LazyLock::new(|| StyleSheet::parse(MAP_CSS));

/// The parsed `map.css`.
pub fn map_stylesheet() -> &'static StyleSheet {
    &MAP_SHEET
}

// Parsing

/// One `name: value` of a rule or a `style` attribute.
#[derive(Debug, Clone, PartialEq)]
pub struct Declaration {
    pub name: String,
    /// The value as written, whitespace collapsed, without `!important`.
    pub value: String,
    pub important: bool,
}

/// A rule of the sheet.
#[derive(Debug, Clone, PartialEq)]
pub enum Rule {
    Style(StyleRule),
    /// `@media`, `@keyframes`, … with its nested rules (or its raw body).
    At(AtRule),
}

/// A style rule: its selectors and the declarations they apply.
#[derive(Debug, Clone, PartialEq)]
pub struct StyleRule {
    /// The selector list as written, whitespace collapsed.
    pub selector_text: String,
    pub selectors: Vec<Selector>,
    pub declarations: Vec<Declaration>,
}

/// An at-rule: `@name prelude`, then a block or a `;`.
#[derive(Debug, Clone, PartialEq)]
pub struct AtRule {
    pub name: String,
    pub prelude: String,
    /// A block at-rule (`@media … { }`), not a statement (`@import …;`).
    pub block: bool,
    /// Nested rules of a grouping rule (`@media`, `@supports`).
    pub rules: Vec<Rule>,
    /// The body of any other block (`@keyframes`, `@font-face`), as written.
    pub body: Option<String>,
}

/// How a compound selector relates to the one on its left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Combinator {
    /// The leftmost compound.
    None,
    Descendant,
    Child,
    Adjacent,
    Sibling,
}

/// The test of an attribute selector: `[a]`, `[a=v]`, `[a~=v]`, `[a|=v]`,
/// `[a^=v]`, `[a$=v]` or `[a*=v]`.
#[derive(Debug, Clone, PartialEq)]
pub enum AttrOp {
    Exists,
    Equals(String),
    Includes(String),
    DashMatch(String),
    Prefix(String),
    Suffix(String),
    Substring(String),
}

/// A pseudo-class in a compound selector.
#[derive(Debug, Clone, PartialEq)]
pub enum Pseudo {
    Hover,
    Root,
    /// `:not(…)`: matches when none of the compounds does.
    Not(Vec<Compound>),
    /// `:is(…)` / `:where(…)`.
    Is(Vec<Compound>, bool),
    /// A pseudo-class or pseudo-element this resolver never matches.
    Never(String),
}

/// A compound selector: `rect.emc-node.is-multi-selected:hover`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Compound {
    pub tag: Option<String>,
    pub ids: Vec<String>,
    pub classes: Vec<String>,
    pub attrs: Vec<(String, AttrOp)>,
    pub pseudos: Vec<Pseudo>,
}

/// A complex selector, left to right, each compound with its combinator.
#[derive(Debug, Clone, PartialEq)]
pub struct Selector {
    pub parts: Vec<(Combinator, Compound)>,
    /// (ids, classes/attributes/pseudo-classes, types)
    pub specificity: (u32, u32, u32),
}

/// Remove `/* comments */`, leaving strings alone.
fn strip_comments(css: &str) -> String {
    let b = css.as_bytes();
    let mut out = String::with_capacity(css.len());
    let mut i = 0;
    let mut quote: Option<u8> = None;
    let mut start = 0;
    while i < b.len() {
        let c = b[i];
        if let Some(q) = quote {
            if c == b'\\' {
                i += 2;
                continue;
            }
            if c == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        if c == b'"' || c == b'\'' {
            quote = Some(c);
            i += 1;
            continue;
        }
        if c == b'/' && b.get(i + 1) == Some(&b'*') {
            out.push_str(&css[start..i]);
            let end = css[i + 2..].find("*/").map_or(b.len(), |e| i + 2 + e + 2);
            out.push(' ');
            i = end;
            start = end;
            continue;
        }
        i += 1;
    }
    out.push_str(&css[start.min(css.len())..]);
    out
}

/// Collapse runs of whitespace to one space and trim.
fn collapse(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for c in s.trim().chars() {
        if c.is_whitespace() {
            space = true;
        } else {
            if space && !out.is_empty() {
                out.push(' ');
            }
            space = false;
            out.push(c);
        }
    }
    out
}

/// Index of the `}` that closes the block opened just before `from`.
fn block_end(b: &[u8], from: usize) -> usize {
    let mut depth = 1;
    let mut i = from;
    let mut quote: Option<u8> = None;
    while i < b.len() {
        let c = b[i];
        if let Some(q) = quote {
            if c == b'\\' {
                i += 2;
                continue;
            }
            if c == q {
                quote = None;
            }
        } else if c == b'"' || c == b'\'' {
            quote = Some(c);
        } else if c == b'{' {
            depth += 1;
        } else if c == b'}' {
            depth -= 1;
            if depth == 0 {
                return i;
            }
        }
        i += 1;
    }
    b.len()
}

/// Split at a separator outside parentheses, brackets and strings.
fn split_top(s: &str, sep: u8) -> Vec<&str> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut quote: Option<u8> = None;
    let mut start = 0;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if let Some(q) = quote {
            if c == b'\\' {
                i += 2;
                continue;
            }
            if c == q {
                quote = None;
            }
        } else {
            match c {
                b'"' | b'\'' => quote = Some(c),
                b'(' | b'[' => depth += 1,
                b')' | b']' => depth -= 1,
                _ if c == sep && depth == 0 => {
                    out.push(&s[start..i]);
                    start = i + 1;
                }
                _ => {}
            }
        }
        i += 1;
    }
    out.push(&s[start..]);
    out
}

/// Parse the declarations of a block or a `style` attribute.
pub fn parse_declarations(text: &str) -> Vec<Declaration> {
    let mut out = Vec::new();
    for part in split_top(text, b';') {
        let Some(colon) = part.find(':') else { continue };
        let name = part[..colon].trim();
        if name.is_empty() {
            continue;
        }
        let mut value = collapse(&part[colon + 1..]);
        let mut important = false;
        if let Some(bang) = value.rfind('!').filter(|b| value[b + 1..].trim().eq_ignore_ascii_case("important")) {
            important = true;
            value = value[..bang].trim_end().to_string();
        }
        let name = if name.starts_with("--") { name.to_string() } else { name.to_ascii_lowercase() };
        out.push(Declaration { name, value, important });
    }
    out
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '-' || c == '_' || !c.is_ascii()
}

/// Parse one compound selector from `s` starting at `*i`.
fn parse_compound(s: &[char], i: &mut usize) -> Option<Compound> {
    let mut c = Compound::default();
    let ident = |s: &[char], i: &mut usize| -> String {
        let start = *i;
        while *i < s.len() && (is_ident_char(s[*i]) || (s[*i] == '\\' && *i + 1 < s.len())) {
            if s[*i] == '\\' {
                *i += 1;
            }
            *i += 1;
        }
        s[start..*i].iter().filter(|c| **c != '\\').collect()
    };
    let start = *i;
    if *i < s.len() && s[*i] == '*' {
        *i += 1;
    } else if *i < s.len() && is_ident_char(s[*i]) {
        c.tag = Some(ident(s, i));
    }
    loop {
        if *i >= s.len() {
            break;
        }
        match s[*i] {
            '.' => {
                *i += 1;
                c.classes.push(ident(s, i));
            }
            '#' => {
                *i += 1;
                c.ids.push(ident(s, i));
            }
            '[' => {
                *i += 1;
                let close = s[*i..].iter().position(|ch| *ch == ']').map_or(s.len(), |p| *i + p);
                let inner: String = s[*i..close].iter().collect();
                *i = (close + 1).min(s.len());
                let (name, op) = match inner.find('=') {
                    None => (inner.trim().to_string(), AttrOp::Exists),
                    Some(eq) => {
                        let (lhs, rhs) = inner.split_at(eq);
                        let value = rhs[1..].trim().trim_matches(|q| q == '"' || q == '\'').to_string();
                        let (name, kind) = match lhs.chars().last() {
                            Some(k @ ('~' | '|' | '^' | '$' | '*')) => (&lhs[..lhs.len() - 1], Some(k)),
                            _ => (lhs, None),
                        };
                        let op = match kind {
                            Some('~') => AttrOp::Includes(value),
                            Some('|') => AttrOp::DashMatch(value),
                            Some('^') => AttrOp::Prefix(value),
                            Some('$') => AttrOp::Suffix(value),
                            Some('*') => AttrOp::Substring(value),
                            _ => AttrOp::Equals(value),
                        };
                        (name.trim().to_string(), op)
                    }
                };
                c.attrs.push((name, op));
            }
            ':' => {
                *i += 1;
                let element = *i < s.len() && s[*i] == ':';
                if element {
                    *i += 1;
                }
                let name = ident(s, i).to_ascii_lowercase();
                let mut args = None;
                if *i < s.len() && s[*i] == '(' {
                    let mut depth = 0;
                    let open = *i;
                    while *i < s.len() {
                        if s[*i] == '(' {
                            depth += 1;
                        } else if s[*i] == ')' {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        *i += 1;
                    }
                    args = Some(s[open + 1..*i].iter().collect::<String>());
                    *i = (*i + 1).min(s.len());
                }
                let compounds = |a: &str| -> Vec<Compound> {
                    split_top(a, b',')
                        .into_iter()
                        .filter_map(|part| {
                            let chars: Vec<char> = part.trim().chars().collect();
                            let mut k = 0;
                            parse_compound(&chars, &mut k)
                        })
                        .collect()
                };
                c.pseudos.push(match (element, name.as_str(), args) {
                    (false, "hover", None) => Pseudo::Hover,
                    (false, "root", None) => Pseudo::Root,
                    (false, "not", Some(a)) => Pseudo::Not(compounds(&a)),
                    (false, "is" | "matches", Some(a)) => Pseudo::Is(compounds(&a), true),
                    (false, "where", Some(a)) => Pseudo::Is(compounds(&a), false),
                    (el, n, _) => Pseudo::Never(format!("{}{n}", if el { "::" } else { ":" })),
                });
            }
            _ => break,
        }
    }
    (*i > start).then_some(c)
}

fn compound_specificity(c: &Compound) -> (u32, u32, u32) {
    let mut a = c.ids.len() as u32;
    let mut b = (c.classes.len() + c.attrs.len()) as u32;
    let mut t = u32::from(c.tag.is_some());
    for p in &c.pseudos {
        match p {
            Pseudo::Hover | Pseudo::Root => b += 1,
            Pseudo::Not(list) | Pseudo::Is(list, true) => {
                let best = list.iter().map(compound_specificity).max().unwrap_or((0, 0, 0));
                a += best.0;
                b += best.1;
                t += best.2;
            }
            Pseudo::Is(_, false) => {}
            Pseudo::Never(name) => {
                if name.starts_with("::") {
                    t += 1;
                } else {
                    b += 1;
                }
            }
        }
    }
    (a, b, t)
}

/// Parse one complex selector (`.emc-map.is-far .emc-label-currency`).
pub fn parse_selector(text: &str) -> Option<Selector> {
    let s: Vec<char> = text.trim().chars().collect();
    let mut i = 0;
    let mut parts = Vec::new();
    let mut comb = Combinator::None;
    loop {
        let compound = parse_compound(&s, &mut i)?;
        parts.push((comb, compound));
        let mut saw_space = false;
        while i < s.len() && s[i].is_whitespace() {
            saw_space = true;
            i += 1;
        }
        if i >= s.len() {
            break;
        }
        comb = match s[i] {
            '>' => Combinator::Child,
            '+' => Combinator::Adjacent,
            '~' => Combinator::Sibling,
            _ if saw_space => Combinator::Descendant,
            _ => return None,
        };
        if comb != Combinator::Descendant {
            i += 1;
            while i < s.len() && s[i].is_whitespace() {
                i += 1;
            }
        }
    }
    let specificity = parts.iter().fold((0, 0, 0), |acc, (_, c)| {
        let s = compound_specificity(c);
        (acc.0 + s.0, acc.1 + s.1, acc.2 + s.2)
    });
    Some(Selector { parts, specificity })
}

fn parse_rules(css: &str) -> Vec<Rule> {
    let b = css.as_bytes();
    let mut rules = Vec::new();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && (b[i] as char).is_whitespace() {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        // The prelude runs to the next `{` or, for a statement at-rule, `;`.
        let at = b[i] == b'@';
        let mut j = i;
        let mut quote: Option<u8> = None;
        while j < b.len() {
            let c = b[j];
            if let Some(q) = quote {
                if c == q {
                    quote = None;
                }
            } else if c == b'"' || c == b'\'' {
                quote = Some(c);
            } else if c == b'{' || (at && c == b';') {
                break;
            } else if c == b'}' {
                break; // stray close brace: skip it
            }
            j += 1;
        }
        if j >= b.len() {
            break;
        }
        let prelude = &css[i..j];
        if b[j] == b'}' {
            i = j + 1;
            continue;
        }
        if b[j] == b';' {
            let (name, rest) = split_at_name(&prelude[1..]);
            rules.push(Rule::At(AtRule { name, prelude: collapse(rest), block: false, rules: vec![], body: None }));
            i = j + 1;
            continue;
        }
        let end = block_end(b, j + 1);
        let body = &css[j + 1..end.min(css.len())];
        if at {
            let (name, rest) = split_at_name(&prelude[1..]);
            let grouping = matches!(name.as_str(), "media" | "supports" | "layer" | "container" | "document");
            rules.push(Rule::At(AtRule {
                prelude: collapse(rest),
                block: true,
                rules: if grouping { parse_rules(body) } else { vec![] },
                body: (!grouping).then(|| body.to_string()),
                name,
            }));
        } else {
            let selector_text = collapse(prelude);
            let selectors = split_top(&selector_text, b',').into_iter().filter_map(parse_selector).collect();
            rules.push(Rule::Style(StyleRule { selector_text, selectors, declarations: parse_declarations(body) }));
        }
        i = end + 1;
    }
    rules
}

fn split_at_name(s: &str) -> (String, &str) {
    let end = s.find(|c: char| !(c.is_alphanumeric() || c == '-')).unwrap_or(s.len());
    (s[..end].to_ascii_lowercase(), &s[end..])
}

// Sheet

/// An element on the way from the root, with its index among its parent's children.
type Frame<'a> = (&'a Element, usize);

/// Where a declaration stands in the cascade: (origin and importance,
/// specificity, order in the sheet). The highest wins.
type Rank = (u8, (u32, u32, u32), usize);

/// A parsed stylesheet, indexed for matching.
#[derive(Debug, Clone)]
pub struct StyleSheet {
    pub rules: Vec<Rule>,
    /// Every applicable style rule, flattened in sheet order.
    flat: Vec<StyleRule>,
    /// (rule, selector) pairs by the class of their rightmost compound.
    by_class: HashMap<String, Vec<(usize, usize)>>,
    /// … by the tag of their rightmost compound, when it has no class.
    by_tag: HashMap<String, Vec<(usize, usize)>>,
    /// … that can match any element.
    universal: Vec<(usize, usize)>,
}

impl StyleSheet {
    /// Parse a stylesheet's text (comments are dropped) and index it.
    pub fn parse(css: &str) -> StyleSheet {
        let rules = parse_rules(&strip_comments(css));
        StyleSheet::from_rules(rules)
    }

    /// Several sheets, in document order (as `document.styleSheets` lists them).
    pub fn concat(sheets: &[&StyleSheet]) -> StyleSheet {
        StyleSheet::from_rules(sheets.iter().flat_map(|s| s.rules.iter().cloned()).collect())
    }

    fn from_rules(rules: Vec<Rule>) -> StyleSheet {
        // Only top-level style rules apply: a native view has no media to query.
        let flat: Vec<StyleRule> = rules
            .iter()
            .filter_map(|r| match r {
                Rule::Style(s) => Some(s.clone()),
                Rule::At(_) => None,
            })
            .collect();
        let mut by_class: HashMap<String, Vec<(usize, usize)>> = HashMap::new();
        let mut by_tag: HashMap<String, Vec<(usize, usize)>> = HashMap::new();
        let mut universal = Vec::new();
        for (ri, rule) in flat.iter().enumerate() {
            for (si, sel) in rule.selectors.iter().enumerate() {
                let last = &sel.parts[sel.parts.len() - 1].1;
                if let Some(class) = last.classes.first() {
                    by_class.entry(class.clone()).or_default().push((ri, si));
                } else if let Some(tag) = &last.tag {
                    by_tag.entry(tag.clone()).or_default().push((ri, si));
                } else {
                    universal.push((ri, si));
                }
            }
        }
        StyleSheet { rules, flat, by_class, by_tag, universal }
    }

    /// The top-level rules whose text mentions `needle`, written as a
    /// browser's CSSOM writes `rule.cssText` (whitespace normalised).
    pub fn rules_text_containing(&self, needle: &str) -> Vec<String> {
        self.rules.iter().map(rule_text).filter(|t| t.contains(needle)).collect()
    }

    /// Computed styles of every element of a tree, in document order
    /// (the order of [`Element::descendants`]).
    pub fn resolve(&self, root: &Element, opts: &ResolveOptions) -> Vec<ComputedStyle> {
        let ctx = MatchCtx::new(root, opts);
        let mut out = Vec::new();
        let mut path: Vec<(&Element, usize)> = vec![(root, 0)];
        self.resolve_into(&mut path, None, &ctx, &mut out);
        out.into_iter().map(|s| s.expect("every element styled")).collect()
    }

    fn resolve_into(
        &self,
        path: &mut Vec<Frame<'_>>,
        parent: Option<&ComputedStyle>,
        ctx: &MatchCtx,
        out: &mut Vec<Option<ComputedStyle>>,
    ) {
        let style = self.cascade(path, parent, ctx);
        let el = path[path.len() - 1].0;
        let at = out.len();
        out.push(None);
        for (idx, child) in el.children.iter().enumerate() {
            if let Node::Element(c) = child {
                path.push((c, idx));
                self.resolve_into(path, Some(&style), ctx, out);
                path.pop();
            }
        }
        out[at] = Some(style);
    }

    /// The computed style of the last element of `path` (the root first),
    /// with every ancestor's style worked out on the way.
    pub fn computed_style(&self, path: &[&Element], opts: &ResolveOptions) -> ComputedStyle {
        let ctx = MatchCtx::new(path[0], opts);
        let mut frames: Vec<(&Element, usize)> = Vec::with_capacity(path.len());
        let mut style: Option<ComputedStyle> = None;
        for (k, el) in path.iter().enumerate() {
            let idx = if k == 0 {
                0
            } else {
                path[k - 1]
                    .children
                    .iter()
                    .position(|c| matches!(c, Node::Element(e) if std::ptr::eq(e, *el)))
                    .expect("each element of the path is a child of the one before it")
            };
            frames.push((el, idx));
            style = Some(self.cascade(&frames, style.as_ref(), &ctx));
        }
        style.expect("a non-empty path")
    }

    fn cascade(&self, path: &[(&Element, usize)], parent: Option<&ComputedStyle>, ctx: &MatchCtx) -> ComputedStyle {
        let opts = ctx.opts;
        let el = path[path.len() - 1].0;
        let is_root = path.len() == 1;
        let mut found: Vec<(Rank, &Declaration)> = Vec::new();
        let mut candidates: Vec<(usize, usize)> = Vec::new();
        let extra_class = is_root && opts.is_far;
        for class in el.classes().chain(extra_class.then_some("is-far")) {
            if let Some(list) = self.by_class.get(class) {
                candidates.extend(list);
            }
        }
        if let Some(list) = self.by_tag.get(el.name) {
            candidates.extend(list);
        }
        candidates.extend(&self.universal);
        candidates.sort_unstable();
        candidates.dedup();
        for (ri, si) in candidates {
            let rule = &self.flat[ri];
            let sel = &rule.selectors[si];
            if !ctx.matches(sel, path) {
                continue;
            }
            for (di, d) in rule.declarations.iter().enumerate() {
                let order = ri * 1000 + di;
                found.push(((if d.important { 3 } else { 1 }, sel.specificity, order), d));
            }
        }
        // Presentation attributes: below every rule. A pattern's or a gradient's
        // own transform attribute is its `transform`, as browsers map it.
        let attr_decls: Vec<Declaration> = el
            .attrs
            .iter()
            .filter_map(|a| {
                let name = match (el.name, a.name) {
                    ("pattern", "patternTransform") | ("linearGradient" | "radialGradient", "gradientTransform") => "transform",
                    (_, n) if PROPERTIES.iter().any(|p| p.name == n && p.attribute) => n,
                    _ => return None,
                };
                Some(Declaration { name: name.to_string(), value: collapse(a.value.as_deref().unwrap_or("")), important: false })
            })
            .collect();
        for (k, d) in attr_decls.iter().enumerate() {
            found.push(((0, (0, 0, 0), k), d));
        }
        // Inline style, and the root's custom properties set by the app.
        let mut inline: Vec<Declaration> = el.get("style").map(parse_declarations).unwrap_or_default();
        if is_root {
            if let Some(v) = opts.label_scale {
                inline.push(Declaration { name: "--label-scale".into(), value: js::number_to_string(v), important: false });
            }
            if let Some(v) = opts.frame_label_scale {
                inline.push(Declaration { name: "--frame-label-scale".into(), value: js::number_to_string(v), important: false });
            }
        }
        for (k, d) in inline.iter().enumerate() {
            found.push(((if d.important { 4 } else { 2 }, (0, 0, 0), k), d));
        }
        found.sort_by_key(|f| f.0);
        let mut winners: IndexMap<&str, &Declaration> = IndexMap::new();
        for (_, d) in &found {
            winners.insert(d.name.as_str(), d);
        }

        // Custom properties first: everything else may refer to them.
        let mut custom: IndexMap<String, String> = match parent {
            Some(p) => p.custom.clone(),
            None => opts.custom.iter().cloned().collect(),
        };
        for (name, d) in &winners {
            if name.starts_with("--") {
                let parent_custom = parent.map(|p| &p.custom);
                match substitute_vars(&d.value, &custom, parent_custom) {
                    Some(v) => {
                        custom.insert(name.to_string(), v);
                    }
                    None => {
                        custom.shift_remove(*name);
                    }
                }
            }
        }

        let parent_font = parent.map_or(opts.root_font_size, |p| p.font_size);
        let mut values: Vec<String> = Vec::with_capacity(PROPERTIES.len());
        let mut font_size = parent_font;
        let mut color = parent.map_or_else(|| opts.root_color.clone(), |p| p.values[COLOR].clone());
        // `color` and `font-size` first: other values refer to them.
        let order = std::iter::once(COLOR).chain(std::iter::once(FONT_SIZE)).chain((0..PROPERTIES.len()).filter(|i| *i != COLOR && *i != FONT_SIZE));
        let mut computed: Vec<Option<String>> = vec![None; PROPERTIES.len()];
        for i in order {
            let p = &PROPERTIES[i];
            let inherited_value = || parent.map(|ps| ps.values[i].clone());
            let declared = winners.get(p.name).map(|d| d.value.as_str());
            let mut value = match declared {
                None => None,
                Some(v) if v.eq_ignore_ascii_case("inherit") => Some(inherited_value().unwrap_or_else(|| p.initial.to_string())),
                Some(v) if v.eq_ignore_ascii_case("initial") => Some(p.initial.to_string()),
                Some(v) if v.eq_ignore_ascii_case("unset") => None,
                Some(v) => substitute_vars(v, &custom, None).and_then(|v| {
                    let em = if i == FONT_SIZE { parent_font } else { font_size };
                    eval_math(&v, em)
                }),
            };
            if value.as_deref().is_some_and(|v| v.eq_ignore_ascii_case("currentcolor")) {
                value = Some(color.clone());
            }
            let v = match value {
                Some(v) => v,
                // The map's own font size, when it sets none, is the page's.
                None if i == FONT_SIZE && parent.is_none() => format!("{}px", js::number_to_string(opts.root_font_size)),
                None if p.inherited => inherited_value().unwrap_or_else(|| p.initial.to_string()),
                None => p.initial.to_string(),
            };
            if i == FONT_SIZE {
                font_size = font_size_px(&v, parent_font);
                computed[i] = Some(format!("{}px", js::number_to_string(font_size)));
            } else {
                if i == COLOR {
                    color = v.clone();
                }
                computed[i] = Some(canonical(p.name, v, font_size));
            }
        }
        values.extend(computed.into_iter().map(|v| v.expect("every property computed")));
        ComputedStyle { values, custom, font_size }
    }
}

/// One CSS number as the CSSOM prints it (`.10` → `0.1`).
fn cssom_numbers(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    let mut out = String::with_capacity(value.len());
    let mut i = 0;
    let mut quote: Option<char> = None;
    while i < chars.len() {
        let c = chars[i];
        if let Some(q) = quote {
            out.push(c);
            if c == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        if c == '"' || c == '\'' {
            quote = Some(c);
            out.push(c);
            i += 1;
            continue;
        }
        let starts = c.is_ascii_digit() || (c == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit));
        let free = i == 0 || !(is_ident_char(chars[i - 1]) || chars[i - 1] == '#' || chars[i - 1] == '.');
        if starts && free {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            let literal: String = chars[start..i].iter().collect();
            match literal.parse::<f64>() {
                Ok(n) => out.push_str(&js::number_to_string(n)),
                Err(_) => out.push_str(&literal),
            }
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

static HEX_COLOR: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"#[0-9a-fA-F]{3,8}\b").unwrap());
static URL_BARE: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r#"url\(\s*([^"')\s][^)]*?)\s*\)"#).unwrap());
static CURRENT_COLOR: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"(?i)\bcurrentcolor\b").unwrap());

/// A declaration's value as a browser's CSSOM writes it back (Gecko's
/// serialisation, for the kinds of value the map's stylesheets use).
///
/// A value that refers to a custom property is kept as it was written, as the
/// browser keeps it; otherwise numbers lose their shorthand (`.10` → `0.1`),
/// `#rrggbb` becomes `rgb(r, g, b)`, `url(#id)` gets its quotes, the SVG
/// lengths that may be written bare get their `px`, and a transition drops
/// the timing function that is the default anyway.
fn cssom_value(name: &str, value: &str) -> String {
    if name.starts_with("--") || value.contains("var(") {
        return value.to_string();
    }
    // url(#id) first, so that an id is never taken for a colour.
    let mut urls: Vec<String> = Vec::new();
    let v = URL_BARE.replace_all(value, |c: &regex::Captures| {
        urls.push(format!("url(\"{}\")", &c[1]));
        format!("\u{1}{}\u{1}", urls.len() - 1)
    });
    let v = HEX_COLOR.replace_all(&v, |c: &regex::Captures| match parse_color(&c[0]) {
        Some(rgba) if rgba.a >= 1.0 => format!("rgb({}, {}, {})", rgba.r, rgba.g, rgba.b),
        Some(rgba) => format!("rgba({}, {}, {}, {})", rgba.r, rgba.g, rgba.b, js::number_to_string(js::round(rgba.a as f64 * 1000.0) / 1000.0)),
        None => c[0].to_string(),
    });
    let v = CURRENT_COLOR.replace_all(&v, "currentcolor");
    let mut v = cssom_numbers(&v);
    let bare_px = |part: &str| if part.parse::<f64>().is_ok() { format!("{part}px") } else { part.to_string() };
    match name {
        "stroke-width" => v = bare_px(&v),
        "stroke-dasharray" if v != "none" => {
            v = v.split(|c: char| c == ',' || c.is_whitespace()).filter(|p| !p.is_empty()).map(bare_px).collect::<Vec<_>>().join(", ");
        }
        "transition" => {
            v = split_top(&v, b',')
                .into_iter()
                .map(|t| t.split_ascii_whitespace().filter(|w| *w != "ease").collect::<Vec<_>>().join(" "))
                .collect::<Vec<_>>()
                .join(", ");
        }
        "transform-origin" if v == "center" => v = "center center 0px".into(),
        _ => {}
    }
    for (k, url) in urls.iter().enumerate() {
        v = v.replace(&format!("\u{1}{k}\u{1}"), url);
    }
    v
}

/// The declarations of a rule as `cssText` lists them.
fn cssom_declarations(declarations: &[Declaration]) -> Vec<String> {
    let has = |name: &str| declarations.iter().find(|d| d.name == name);
    let mut out = Vec::new();
    for d in declarations {
        let mut value = cssom_value(&d.name, &d.value);
        match d.name.as_str() {
            // An alias of a property the rule also sets: written once.
            "-webkit-user-select" if has("user-select").is_some_and(|u| u.value == d.value) => continue,
            // A longhand the shorthand before it can carry.
            "text-decoration-thickness" if has("text-decoration").is_some() => continue,
            "text-decoration" => {
                if let Some(t) = has("text-decoration-thickness") {
                    value = format!("{value} {}", cssom_value(&t.name, &t.value));
                }
            }
            _ => {}
        }
        out.push(format!("{}: {value}{};", d.name, if d.important { " !important" } else { "" }));
    }
    out
}

/// `rule.cssText`, written the way a browser normalises it.
fn rule_text(rule: &Rule) -> String {
    match rule {
        Rule::Style(s) => {
            let selectors = split_top(&s.selector_text, b',').into_iter().map(str::trim).collect::<Vec<_>>().join(", ");
            let decls = cssom_declarations(&s.declarations);
            if decls.is_empty() { format!("{selectors} {{ }}") } else { format!("{selectors} {{ {} }}", decls.join(" ")) }
        }
        Rule::At(a) => {
            let head = if a.prelude.is_empty() { format!("@{}", a.name) } else { format!("@{} {}", a.name, a.prelude) };
            match &a.body {
                Some(body) => format!("{head} {{ {} }}", collapse(body)),
                None if !a.block => format!("{head};"),
                None => {
                    let inner: Vec<String> = a.rules.iter().map(|r| format!("  {}", rule_text(r))).collect();
                    format!("{head} {{\n{}\n}}", inner.join("\n"))
                }
            }
        }
    }
}

// Matching

/// What the resolver is told about the view, beyond the tree itself.
#[derive(Debug, Clone)]
pub struct ResolveOptions<'a> {
    /// `--label-scale` set on the map (`None`: the stylesheet's 1).
    pub label_scale: Option<f64>,
    /// `--frame-label-scale` set on the map (`None`: the stylesheet's 1).
    pub frame_label_scale: Option<f64>,
    /// Zoomed far out: the app adds `is-far` to the map.
    pub is_far: bool,
    /// The element under the pointer, for `:hover` (it and its ancestors match).
    pub hovered: Option<&'a Element>,
    /// The `color` the map inherits (`currentColor` in the stylesheet).
    pub root_color: String,
    /// The font size the map inherits, in px (16 on a page that sets none).
    pub root_font_size: f64,
    /// Custom properties the map inherits from the page around it, such as
    /// the app's `--accent` and `--ink`; the stylesheet has fallbacks for
    /// when there are none (an exported SVG opened on its own).
    pub custom: Vec<(String, String)>,
}

impl Default for ResolveOptions<'_> {
    fn default() -> Self {
        ResolveOptions {
            label_scale: None,
            frame_label_scale: None,
            is_far: false,
            hovered: None,
            root_color: "#000000".into(),
            root_font_size: 16.0,
            custom: Vec::new(),
        }
    }
}

struct MatchCtx<'o, 'a> {
    opts: &'o ResolveOptions<'a>,
    /// The hovered element and its ancestors (by address): `:hover` holds for all.
    hover: Vec<*const Element>,
}

impl<'o, 'a> MatchCtx<'o, 'a> {
    fn new(root: &Element, opts: &'o ResolveOptions<'a>) -> MatchCtx<'o, 'a> {
        fn chain(e: &Element, target: *const Element, out: &mut Vec<*const Element>) -> bool {
            out.push(e as *const Element);
            if std::ptr::eq(e, target) || e.elements().any(|c| chain(c, target, out)) {
                return true;
            }
            out.pop();
            false
        }
        let mut hover = Vec::new();
        if let Some(h) = opts.hovered {
            chain(root, h as *const Element, &mut hover);
        }
        MatchCtx { opts, hover }
    }

    fn matches(&self, sel: &Selector, path: &[(&Element, usize)]) -> bool {
        self.match_from(sel, sel.parts.len() - 1, path)
    }

    fn match_from(&self, sel: &Selector, ci: usize, path: &[(&Element, usize)]) -> bool {
        if !self.compound(&sel.parts[ci].1, path) {
            return false;
        }
        if ci == 0 {
            return true;
        }
        // The combinator between parts[ci - 1] and parts[ci] is stored on parts[ci].
        match sel.parts[ci].0 {
            Combinator::None => true,
            Combinator::Descendant => (1..path.len()).rev().any(|k| self.match_from(sel, ci - 1, &path[..k])),
            Combinator::Child => path.len() > 1 && self.match_from(sel, ci - 1, &path[..path.len() - 1]),
            Combinator::Adjacent | Combinator::Sibling => {
                if path.len() < 2 {
                    return false;
                }
                let parent = path[path.len() - 2].0;
                let idx = path[path.len() - 1].1;
                let mut siblings = parent.children[..idx]
                    .iter()
                    .enumerate()
                    .rev()
                    .filter_map(|(k, c)| if let Node::Element(e) = c { Some((e, k)) } else { None });
                let try_sibling = |e: &Element, k: usize| {
                    let mut p: Vec<(&Element, usize)> = path[..path.len() - 1].to_vec();
                    p.push((e, k));
                    self.match_from(sel, ci - 1, &p)
                };
                if sel.parts[ci].0 == Combinator::Adjacent {
                    siblings.next().is_some_and(|(e, k)| try_sibling(e, k))
                } else {
                    siblings.any(|(e, k)| try_sibling(e, k))
                }
            }
        }
    }

    fn compound(&self, c: &Compound, path: &[(&Element, usize)]) -> bool {
        let el = path[path.len() - 1].0;
        let is_root = path.len() == 1;
        if c.tag.as_deref().is_some_and(|t| t != el.name) {
            return false;
        }
        for id in &c.ids {
            if el.get("id") != Some(id.as_str()) {
                return false;
            }
        }
        for class in &c.classes {
            if !(el.has_class(class) || (is_root && self.opts.is_far && class == "is-far")) {
                return false;
            }
        }
        for (name, op) in &c.attrs {
            let Some(v) = el.get(name) else { return false };
            let ok = match op {
                AttrOp::Exists => true,
                AttrOp::Equals(x) => v == x,
                AttrOp::Includes(x) => v.split_ascii_whitespace().any(|w| w == x),
                AttrOp::DashMatch(x) => v == x || v.starts_with(&format!("{x}-")),
                AttrOp::Prefix(x) => !x.is_empty() && v.starts_with(x.as_str()),
                AttrOp::Suffix(x) => !x.is_empty() && v.ends_with(x.as_str()),
                AttrOp::Substring(x) => !x.is_empty() && v.contains(x.as_str()),
            };
            if !ok {
                return false;
            }
        }
        for p in &c.pseudos {
            let ok = match p {
                Pseudo::Hover => self.hovered(path),
                Pseudo::Root => is_root,
                Pseudo::Not(list) => !list.iter().any(|x| self.compound(x, path)),
                Pseudo::Is(list, _) => list.iter().any(|x| self.compound(x, path)),
                Pseudo::Never(_) => false,
            };
            if !ok {
                return false;
            }
        }
        true
    }

    /// `:hover` holds for the hovered element and every ancestor of it.
    fn hovered(&self, path: &[(&Element, usize)]) -> bool {
        let el = path[path.len() - 1].0 as *const Element;
        self.hover.contains(&el)
    }
}

// Values

/// A property the resolver computes.
pub struct Property {
    pub name: &'static str,
    pub inherited: bool,
    pub initial: &'static str,
    /// Also settable as an SVG presentation attribute.
    pub attribute: bool,
}

const fn prop(name: &'static str, inherited: bool, initial: &'static str, attribute: bool) -> Property {
    Property { name, inherited, initial, attribute }
}

/// The properties of a computed style, in this order.
pub const PROPERTIES: [Property; 26] = [
    prop("color", true, "#000000", true),
    prop("font-size", true, "16px", true),
    prop("fill", true, "black", true),
    prop("fill-opacity", true, "1", true),
    prop("stroke", true, "none", true),
    prop("stroke-width", true, "1", true),
    prop("stroke-opacity", true, "1", true),
    prop("stroke-dasharray", true, "none", true),
    prop("stroke-linecap", true, "butt", true),
    prop("stroke-linejoin", true, "miter", true),
    prop("opacity", false, "1", true),
    prop("font-weight", true, "400", true),
    prop("font-family", true, "serif", true),
    prop("font-variant-numeric", true, "normal", false),
    prop("letter-spacing", true, "normal", true),
    prop("display", false, "inline", true),
    prop("visibility", true, "visible", true),
    prop("text-transform", true, "none", false),
    prop("text-anchor", true, "start", true),
    prop("dominant-baseline", true, "auto", true),
    prop("paint-order", true, "normal", true),
    prop("filter", false, "none", true),
    prop("transform", false, "none", true),
    prop("vector-effect", false, "none", true),
    prop("pointer-events", true, "auto", true),
    prop("text-decoration", false, "none", true),
];

const COLOR: usize = 0;
const FONT_SIZE: usize = 1;

fn index_of(name: &str) -> Option<usize> {
    PROPERTIES.iter().position(|p| p.name == name)
}

/// Replace every `var(--x[, fallback])`; `None` when one has neither a value
/// nor a fallback (the declaration is then invalid at computed-value time).
fn substitute_vars(value: &str, custom: &IndexMap<String, String>, parent: Option<&IndexMap<String, String>>) -> Option<String> {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(at) = find_function(rest, "var") {
        out.push_str(&rest[..at]);
        let open = at + 3;
        let close = matching_paren(rest, open)?;
        let inner = &rest[open + 1..close];
        let parts = split_top(inner, b',');
        let name = parts[0].trim();
        let fallback = (parts.len() > 1).then(|| inner[parts[0].len() + 1..].trim());
        let found = custom.get(name).or_else(|| parent.and_then(|p| p.get(name)));
        let replacement = match (found, fallback) {
            (Some(v), _) => v.clone(),
            (None, Some(f)) => substitute_vars(f, custom, parent)?,
            (None, None) => return None,
        };
        out.push_str(&replacement);
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    Some(out)
}

/// Where `name(` starts in `s` as a function (not inside a longer identifier).
fn find_function(s: &str, name: &str) -> Option<usize> {
    let lower = s.to_ascii_lowercase();
    let pattern = format!("{name}(");
    let mut from = 0;
    while let Some(k) = lower[from..].find(&pattern) {
        let at = from + k;
        let before = lower[..at].chars().last();
        if before.is_none_or(|c| !is_ident_char(c)) {
            return Some(at);
        }
        from = at + 1;
    }
    None
}

/// The `)` that closes the `(` at byte `open`.
fn matching_paren(s: &str, open: usize) -> Option<usize> {
    let mut depth = 0;
    for (k, c) in s[open..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + k);
                }
            }
            _ => {}
        }
    }
    None
}

/// A number with its unit, as math functions compute.
#[derive(Debug, Clone, PartialEq)]
struct Dim {
    v: f64,
    unit: String,
}

impl Dim {
    fn text(&self) -> String {
        format!("{}{}", js::number_to_string(self.v), self.unit)
    }
}

/// Evaluate `calc()`, `min()`, `max()` and `clamp()` wherever they occur;
/// `em` is the font size `em` units refer to. `None` when one cannot be
/// computed (mixed units, division by a length).
fn eval_math(value: &str, em: f64) -> Option<String> {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    loop {
        let next = ["calc", "min", "max", "clamp"]
            .iter()
            .filter_map(|f| find_function(rest, f).map(|at| (at, f.len())))
            .min_by_key(|(at, _)| *at);
        let Some((at, len)) = next else { break };
        out.push_str(&rest[..at]);
        let close = matching_paren(rest, at + len)?;
        let expr: Vec<char> = rest[at..=close].chars().collect();
        let mut k = 0;
        let d = MathParser { s: &expr, em }.factor(&mut k)?;
        out.push_str(&d.text());
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    Some(out)
}

struct MathParser<'a> {
    s: &'a [char],
    em: f64,
}

impl MathParser<'_> {
    fn skip(&self, k: &mut usize) {
        while *k < self.s.len() && self.s[*k].is_whitespace() {
            *k += 1;
        }
    }

    fn sum(&self, k: &mut usize) -> Option<Dim> {
        let mut a = self.product(k)?;
        loop {
            self.skip(k);
            let Some(&op) = self.s.get(*k) else { break };
            if op != '+' && op != '-' {
                break;
            }
            *k += 1;
            let b = self.product(k)?;
            let (a2, b2) = unify(a, b)?;
            a = Dim { v: if op == '+' { a2.v + b2.v } else { a2.v - b2.v }, unit: a2.unit };
        }
        Some(a)
    }

    fn product(&self, k: &mut usize) -> Option<Dim> {
        let mut a = self.factor(k)?;
        loop {
            self.skip(k);
            let Some(&op) = self.s.get(*k) else { break };
            if op != '*' && op != '/' {
                break;
            }
            *k += 1;
            let b = self.factor(k)?;
            a = if op == '*' {
                match (a.unit.is_empty(), b.unit.is_empty()) {
                    (true, _) => Dim { v: a.v * b.v, unit: b.unit },
                    (_, true) => Dim { v: a.v * b.v, unit: a.unit },
                    _ => return None,
                }
            } else {
                if !b.unit.is_empty() {
                    return None;
                }
                Dim { v: a.v / b.v, unit: a.unit }
            };
        }
        Some(a)
    }

    fn factor(&self, k: &mut usize) -> Option<Dim> {
        self.skip(k);
        let c = *self.s.get(*k)?;
        if c == '(' {
            *k += 1;
            let d = self.sum(k)?;
            self.skip(k);
            if self.s.get(*k) != Some(&')') {
                return None;
            }
            *k += 1;
            return Some(d);
        }
        if c.is_ascii_alphabetic() {
            let start = *k;
            while *k < self.s.len() && (self.s[*k].is_ascii_alphanumeric() || self.s[*k] == '-') {
                *k += 1;
            }
            let name: String = self.s[start..*k].iter().collect::<String>().to_ascii_lowercase();
            if self.s.get(*k) != Some(&'(') {
                return None;
            }
            *k += 1;
            let mut args = Vec::new();
            loop {
                args.push(self.sum(k)?);
                self.skip(k);
                match self.s.get(*k) {
                    Some(',') => *k += 1,
                    Some(')') => {
                        *k += 1;
                        break;
                    }
                    _ => return None,
                }
            }
            return match name.as_str() {
                "calc" if args.len() == 1 => args.pop(),
                "min" | "max" => {
                    let mut acc = args[0].clone();
                    for b in &args[1..] {
                        let (x, y) = unify(acc, b.clone())?;
                        acc = Dim { v: if name == "min" { js::min(x.v, y.v) } else { js::max(x.v, y.v) }, unit: x.unit };
                    }
                    Some(acc)
                }
                "clamp" if args.len() == 3 => {
                    let (lo, v) = unify(args[0].clone(), args[1].clone())?;
                    let (v, hi) = unify(v, args[2].clone())?;
                    Some(Dim { v: js::max(lo.v, js::min(v.v, hi.v)), unit: v.unit })
                }
                _ => None,
            };
        }
        // A number, with its sign and unit.
        let start = *k;
        if c == '+' || c == '-' {
            *k += 1;
        }
        while *k < self.s.len() && (self.s[*k].is_ascii_digit() || self.s[*k] == '.') {
            *k += 1;
        }
        if *k < self.s.len() && (self.s[*k] == 'e' || self.s[*k] == 'E') {
            let save = *k;
            *k += 1;
            if *k < self.s.len() && (self.s[*k] == '+' || self.s[*k] == '-') {
                *k += 1;
            }
            if *k < self.s.len() && self.s[*k].is_ascii_digit() {
                while *k < self.s.len() && self.s[*k].is_ascii_digit() {
                    *k += 1;
                }
            } else {
                *k = save;
            }
        }
        let number: String = self.s[start..*k].iter().collect();
        let v: f64 = number.parse().ok()?;
        let ustart = *k;
        while *k < self.s.len() && (self.s[*k].is_ascii_alphabetic() || self.s[*k] == '%') {
            *k += 1;
        }
        let unit: String = self.s[ustart..*k].iter().collect::<String>().to_ascii_lowercase();
        Some(match unit.as_str() {
            "em" => Dim { v: v * self.em, unit: "px".into() },
            "rem" => Dim { v: v * 16.0, unit: "px".into() },
            _ => Dim { v, unit },
        })
    }
}

/// Two values in the same unit; a bare 0 takes the other's unit.
fn unify(a: Dim, b: Dim) -> Option<(Dim, Dim)> {
    if a.unit == b.unit {
        return Some((a, b));
    }
    if a.unit.is_empty() && a.v == 0.0 {
        return Some((Dim { v: 0.0, unit: b.unit.clone() }, b));
    }
    if b.unit.is_empty() && b.v == 0.0 {
        let unit = a.unit.clone();
        return Some((a, Dim { v: 0.0, unit }));
    }
    None
}

/// A computed value as a browser reports it: opacities as plain numbers in
/// 0..=1, lengths in px, weights as numbers.
fn canonical(name: &str, value: String, em: f64) -> String {
    match name {
        "opacity" | "fill-opacity" | "stroke-opacity" => match split_number(&value) {
            (Some(n), "%") => js::number_to_string((n / 100.0).clamp(0.0, 1.0)),
            (Some(n), "") => js::number_to_string(n.clamp(0.0, 1.0)),
            _ => value,
        },
        "stroke-width" | "letter-spacing" => {
            let px = length_px(&value, em);
            if px.is_nan() { value } else { format!("{}px", js::number_to_string(px)) }
        }
        "font-weight" => match value.as_str() {
            "normal" => "400".into(),
            "bold" => "700".into(),
            _ => value,
        },
        _ => value,
    }
}

/// A computed `font-size` in px.
fn font_size_px(value: &str, parent: f64) -> f64 {
    let v = value.trim().to_ascii_lowercase();
    let keyword = match v.as_str() {
        "xx-small" => Some(9.0),
        "x-small" => Some(10.0),
        "small" => Some(13.0),
        "medium" => Some(16.0),
        "large" => Some(18.0),
        "x-large" => Some(24.0),
        "xx-large" => Some(32.0),
        "smaller" => Some(parent / 1.2),
        "larger" => Some(parent * 1.2),
        _ => None,
    };
    if let Some(k) = keyword {
        return k;
    }
    let (num, unit) = split_number(&v);
    match (num, unit) {
        (Some(n), "px" | "") => n,
        (Some(n), "em") => n * parent,
        (Some(n), "rem") => n * 16.0,
        (Some(n), "%") => n / 100.0 * parent,
        (Some(n), "pt") => n * 4.0 / 3.0,
        _ => parent,
    }
}

/// A leading number and the unit after it.
fn split_number(s: &str) -> (Option<f64>, &str) {
    let s = s.trim();
    let end = s
        .char_indices()
        .find(|(k, c)| !(c.is_ascii_digit() || *c == '.' || ((*c == '-' || *c == '+') && *k == 0)))
        .map_or(s.len(), |(k, _)| k);
    (s[..end].parse().ok(), s[end..].trim())
}

// Computed style

/// A colour with straight alpha, channels 0..=255 and alpha 0..=1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: f32,
}

/// How a shape is filled or stroked.
#[derive(Debug, Clone, PartialEq)]
pub enum Paint {
    None,
    Color(Rgba),
    /// A paint server (`url(#emc-wall)`).
    Url(String),
}

/// The computed style of one element: every property of [`PROPERTIES`] as
/// CSS text, plus the custom properties in scope.
#[derive(Debug, Clone, PartialEq)]
pub struct ComputedStyle {
    values: Vec<String>,
    pub custom: IndexMap<String, String>,
    font_size: f64,
}

impl ComputedStyle {
    /// A property's computed value as CSS text (`""` for one not computed).
    pub fn get(&self, name: &str) -> &str {
        index_of(name).map_or("", |i| self.values[i].as_str())
    }

    /// Every computed property, by name.
    pub fn iter(&self) -> impl Iterator<Item = (&'static str, &str)> {
        PROPERTIES.iter().zip(&self.values).map(|(p, v)| (p.name, v.as_str()))
    }

    pub fn fill(&self) -> Paint {
        parse_paint(self.get("fill"))
    }
    pub fn stroke(&self) -> Paint {
        parse_paint(self.get("stroke"))
    }
    /// In px. NaN when it is not a length this module reads.
    pub fn stroke_width(&self) -> f64 {
        length_px(self.get("stroke-width"), self.font_size)
    }
    /// `None` for `none`.
    pub fn stroke_dasharray(&self) -> Option<Vec<f64>> {
        let v = self.get("stroke-dasharray");
        if v == "none" {
            return None;
        }
        Some(v.split(|c: char| c == ',' || c.is_whitespace()).filter(|s| !s.is_empty()).map(|s| length_px(s, self.font_size)).collect())
    }
    /// As a fraction (`50%` is 0.5); 1 when not a number.
    pub fn opacity(&self) -> f64 {
        number_or_percent(self.get("opacity"))
    }
    pub fn fill_opacity(&self) -> f64 {
        number_or_percent(self.get("fill-opacity"))
    }
    pub fn stroke_opacity(&self) -> f64 {
        number_or_percent(self.get("stroke-opacity"))
    }
    /// In px.
    pub fn font_size(&self) -> f64 {
        self.font_size
    }
    /// 100–900.
    pub fn font_weight(&self) -> u16 {
        match self.get("font-weight") {
            "normal" => 400,
            "bold" | "bolder" => 700,
            "lighter" => 300,
            v => v.parse::<f64>().map_or(400, |w| w.round().clamp(1.0, 1000.0) as u16),
        }
    }
    pub fn font_family(&self) -> &str {
        self.get("font-family")
    }
    /// `display: none` takes the element and its subtree out of the picture.
    pub fn is_displayed(&self) -> bool {
        self.get("display") != "none"
    }
    pub fn is_visible(&self) -> bool {
        self.get("visibility") == "visible"
    }
    pub fn text_transform(&self) -> &str {
        self.get("text-transform")
    }
    pub fn text_anchor(&self) -> &str {
        self.get("text-anchor")
    }
    pub fn dominant_baseline(&self) -> &str {
        self.get("dominant-baseline")
    }
    /// The CSS `transform`, or `None`.
    pub fn transform(&self) -> Option<&str> {
        Some(self.get("transform")).filter(|t| *t != "none")
    }
    /// The transform as a matrix `[a, b, c, d, e, f]` (identity for none);
    /// `None` when it is not a transform this module reads.
    pub fn transform_matrix(&self) -> Option<[f64; 6]> {
        parse_transform(self.get("transform"))
    }
    /// A uniform `scale(s)` transform's factor (name plates), if that is what it is.
    pub fn transform_scale(&self) -> Option<f64> {
        let t = self.transform()?.trim();
        let inner = t.strip_prefix("scale(")?.strip_suffix(')')?;
        let parts: Vec<f64> = inner.split(',').map(|p| p.trim().parse().ok()).collect::<Option<_>>()?;
        match parts.as_slice() {
            [s] => Some(*s),
            [x, y] if x == y => Some(*x),
            _ => None,
        }
    }
}

fn number_or_percent(v: &str) -> f64 {
    let (n, unit) = split_number(v);
    match (n, unit) {
        (Some(n), "%") => n / 100.0,
        (Some(n), _) => n,
        _ => 1.0,
    }
}

/// A length in px (`px`, `em`, `pt` or a bare number); NaN for anything else.
pub fn parse_length(v: &str, em: f64) -> f64 {
    length_px(v, em)
}

fn length_px(v: &str, em: f64) -> f64 {
    let (n, unit) = split_number(v);
    match (n, unit) {
        (Some(n), "" | "px") => n,
        (Some(n), "em") => n * em,
        (Some(n), "pt") => n * 4.0 / 3.0,
        _ => f64::NAN,
    }
}

/// Parse a paint: `none`, a colour, or `url(#id)`.
pub fn parse_paint(v: &str) -> Paint {
    let t = v.trim();
    if t.eq_ignore_ascii_case("none") {
        return Paint::None;
    }
    if let Some(rest) = t.strip_prefix("url(") {
        let id = rest.split(')').next().unwrap_or("").trim().trim_matches(|c| c == '"' || c == '\'');
        return Paint::Url(id.to_string());
    }
    parse_color(t).map_or(Paint::None, Paint::Color)
}

/// Parse a CSS colour: `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`, `rgb()`/`rgba()`,
/// `transparent` and the basic named colours.
pub fn parse_color(v: &str) -> Option<Rgba> {
    let t = v.trim().to_ascii_lowercase();
    if let Some(hex) = t.strip_prefix('#') {
        let digits: Vec<u8> = hex.chars().map(|c| c.to_digit(16).map(|d| d as u8)).collect::<Option<_>>()?;
        let expand = |d: u8| d * 17;
        return match digits.len() {
            3 => Some(Rgba { r: expand(digits[0]), g: expand(digits[1]), b: expand(digits[2]), a: 1.0 }),
            4 => Some(Rgba { r: expand(digits[0]), g: expand(digits[1]), b: expand(digits[2]), a: expand(digits[3]) as f32 / 255.0 }),
            6 => Some(Rgba { r: digits[0] * 16 + digits[1], g: digits[2] * 16 + digits[3], b: digits[4] * 16 + digits[5], a: 1.0 }),
            8 => Some(Rgba {
                r: digits[0] * 16 + digits[1],
                g: digits[2] * 16 + digits[3],
                b: digits[4] * 16 + digits[5],
                a: (digits[6] * 16 + digits[7]) as f32 / 255.0,
            }),
            _ => None,
        };
    }
    if let Some(args) = t.strip_prefix("rgba(").or_else(|| t.strip_prefix("rgb(")) {
        let args = args.strip_suffix(')')?;
        let parts: Vec<&str> = args.split(|c: char| c == ',' || c == '/' || c.is_whitespace()).filter(|s| !s.is_empty()).collect();
        if parts.len() < 3 {
            return None;
        }
        let channel = |p: &str| -> Option<u8> {
            let (n, unit) = split_number(p);
            let n = n?;
            Some(if unit == "%" { (n * 2.55).round().clamp(0.0, 255.0) as u8 } else { n.round().clamp(0.0, 255.0) as u8 })
        };
        let a = parts.get(3).map_or(Some(1.0), |p| Some(number_or_percent(p) as f32))?;
        return Some(Rgba { r: channel(parts[0])?, g: channel(parts[1])?, b: channel(parts[2])?, a });
    }
    let named = |r, g, b| Some(Rgba { r, g, b, a: 1.0 });
    match t.as_str() {
        "transparent" => Some(Rgba { r: 0, g: 0, b: 0, a: 0.0 }),
        "black" => named(0, 0, 0),
        "white" => named(255, 255, 255),
        "red" => named(255, 0, 0),
        "green" => named(0, 128, 0),
        "blue" => named(0, 0, 255),
        "gray" | "grey" => named(128, 128, 128),
        "orange" => named(255, 165, 0),
        "yellow" => named(255, 255, 0),
        _ => None,
    }
}

/// A transform list as the matrix `[a, b, c, d, e, f]` (x' = a·x + c·y + e,
/// y' = b·x + d·y + f). Reads the SVG attribute syntax (`translate(10 20)
/// rotate(45)`) and the CSS one (`scale(1.5)`, `rotate(45deg)`,
/// `translate(10px, 5px)`); `none` is the identity.
pub fn parse_transform(text: &str) -> Option<[f64; 6]> {
    let mut m = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    let t = text.trim();
    if t.is_empty() || t == "none" {
        return Some(m);
    }
    let mut rest = t;
    while !rest.trim().is_empty() {
        let open = rest.find('(')?;
        let close = rest.find(')')?;
        let name = rest[..open].trim().trim_start_matches(',').trim().to_ascii_lowercase();
        let mut args = Vec::new();
        let mut angle_scale = 1.0;
        for part in rest[open + 1..close].split(|c: char| c == ',' || c.is_whitespace()).filter(|p| !p.is_empty()) {
            let (n, unit) = split_number(part);
            let n = n?;
            match unit {
                "" | "px" | "deg" => args.push(n),
                "rad" => {
                    angle_scale = 180.0 / std::f64::consts::PI;
                    args.push(n);
                }
                "turn" => {
                    angle_scale = 360.0;
                    args.push(n);
                }
                _ => return None,
            }
        }
        let arg = |k: usize| args.get(k).copied();
        let step = match name.as_str() {
            "matrix" if args.len() == 6 => [args[0], args[1], args[2], args[3], args[4], args[5]],
            "translate" => [1.0, 0.0, 0.0, 1.0, arg(0)?, arg(1).unwrap_or(0.0)],
            "translatex" => [1.0, 0.0, 0.0, 1.0, arg(0)?, 0.0],
            "translatey" => [1.0, 0.0, 0.0, 1.0, 0.0, arg(0)?],
            "scale" => [arg(0)?, 0.0, 0.0, arg(1).unwrap_or(arg(0)?), 0.0, 0.0],
            "scalex" => [arg(0)?, 0.0, 0.0, 1.0, 0.0, 0.0],
            "scaley" => [1.0, 0.0, 0.0, arg(0)?, 0.0, 0.0],
            "rotate" => {
                let a = (arg(0)? * angle_scale).to_radians();
                let (sin, cos) = (js::sin(a), js::cos(a));
                let (cx, cy) = (arg(1).unwrap_or(0.0), arg(2).unwrap_or(0.0));
                [cos, sin, -sin, cos, cx - cos * cx + sin * cy, cy - sin * cx - cos * cy]
            }
            "skewx" => [1.0, 0.0, js::tan((arg(0)? * angle_scale).to_radians()), 1.0, 0.0, 0.0],
            "skewy" => [1.0, js::tan((arg(0)? * angle_scale).to_radians()), 0.0, 1.0, 0.0, 0.0],
            _ => return None,
        };
        m = [
            m[0] * step[0] + m[2] * step[1],
            m[1] * step[0] + m[3] * step[1],
            m[0] * step[2] + m[2] * step[3],
            m[1] * step[2] + m[3] * step[3],
            m[0] * step[4] + m[2] * step[5] + m[4],
            m[1] * step[4] + m[3] * step[5] + m[5],
        ];
        rest = &rest[close + 1..];
    }
    Some(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transforms_multiply_left_to_right() {
        assert_eq!(parse_transform("none"), Some([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]));
        assert_eq!(parse_transform("translate(10 20) scale(2)"), Some([2.0, 0.0, 0.0, 2.0, 10.0, 20.0]));
        assert_eq!(parse_transform("scale(0.5)"), Some([0.5, 0.0, 0.0, 0.5, 0.0, 0.0]));
        let m = parse_transform("translate(100 50) rotate(90)").unwrap();
        let near = |a: f64, b: f64| (a - b).abs() < 1e-12;
        assert!(near(m[0], 0.0) && near(m[1], 1.0) && near(m[2], -1.0) && near(m[3], 0.0) && m[4] == 100.0 && m[5] == 50.0);
        assert_eq!(parse_transform("translate(10px, 5px) rotate(0deg)"), Some([1.0, 0.0, 0.0, 1.0, 10.0, 5.0]));
        assert_eq!(parse_transform("frobnicate(1)"), None);
    }

    fn tree() -> Element {
        tree_of("emc-map")
    }

    /// A miniature of what render_map draws, its root of class `map_class`.
    fn tree_of(map_class: &'static str) -> Element {
        let label = |cls: &str| Element::paired("text").with("class", format!("emc-label {cls}")).with("fill", "#1e293b").text("x");
        Element::paired("svg")
            .with("class", map_class)
            .child(
                Element::paired("g")
                    .with("class", "emc-layer emc-layer-text")
                    .child(label("emc-label-met emc-label-primary emc-label-draggable"))
                    .child(label("emc-label-currency"))
                    .child(label("emc-label-rxn emc-label-rxn-adjacent")),
            )
            .child(
                Element::paired("g").with("class", "emc-layer emc-layer-nodes").child(
                    Element::new("circle")
                        .with("class", "emc-node emc-node-currency emc-node-secondary is-multi-selected")
                        .with("r", "7")
                        .with("fill", "#e67e4c")
                        .with("stroke", "#a64613")
                        .with("stroke-width", "1.8"),
                ),
            )
            .child(
                Element::paired("g").with("class", "emc-comp-label").child(
                    Element::paired("g")
                        .with("class", "emc-frame-label")
                        .with("style", "--fit:0.500")
                        .child(Element::paired("text").with("class", "emc-comp-text").text("Cytosol")),
                ),
            )
            .child(
                Element::paired("g")
                    .with("class", "emc-boundary-pair")
                    .child(Element::new("rect").with("class", "emc-boundary-hit").with("stroke", "transparent"))
                    .child(Element::new("rect").with("class", "emc-boundary-glow").with("stroke", "transparent")),
            )
    }

    fn styles(svg: &Element, opts: &ResolveOptions) -> Vec<(String, ComputedStyle)> {
        let all = map_stylesheet().resolve(svg, opts);
        svg.descendants().iter().map(|e| e.get("class").unwrap_or(e.name).to_string()).zip(all).collect()
    }

    fn find<'a>(s: &'a [(String, ComputedStyle)], class: &str) -> &'a ComputedStyle {
        &s.iter().find(|(c, _)| c.contains(class)).unwrap_or_else(|| panic!("{class}")).1
    }

    #[test]
    fn selectors_parse_with_specificity() {
        let s = parse_selector(".emc-map.is-far .emc-label-currency").unwrap();
        assert_eq!(s.parts.len(), 2);
        assert_eq!(s.parts[1].0, Combinator::Descendant);
        assert_eq!(s.specificity, (0, 3, 0));
        let s = parse_selector(".emc-boundary-hit:hover + .emc-boundary-glow").unwrap();
        assert_eq!(s.parts[1].0, Combinator::Adjacent);
        assert_eq!(s.specificity, (0, 3, 0));
        let s = parse_selector(".emc-marker:hover rect").unwrap();
        assert_eq!(s.specificity, (0, 2, 1));
        let s = parse_selector(r#".stage[data-tool="cell"] .emc-node:not(.x)"#).unwrap();
        assert_eq!(s.specificity, (0, 4, 0));
        assert_eq!(s.parts[0].1.attrs, vec![("data-tool".to_string(), AttrOp::Equals("cell".into()))]);
    }

    #[test]
    fn the_whole_map_stylesheet_parses() {
        let sheet = map_stylesheet();
        let count = sheet.rules.len();
        assert!(count > 80, "{count} rules");
        for rule in &sheet.rules {
            if let Rule::Style(s) = rule {
                assert_eq!(s.selectors.len(), split_top(&s.selector_text, b',').len(), "{}", s.selector_text);
                assert!(!s.declarations.is_empty(), "{}", s.selector_text);
            }
        }
        let important = sheet.flat.iter().flat_map(|r| &r.declarations).filter(|d| d.important).count();
        assert_eq!(important, 11);
    }

    #[test]
    fn a_quiet_map_draws_its_cofactors_lighter_with_smaller_names() {
        let full = styles(&tree(), &ResolveOptions::default());
        let light = styles(&tree_of("emc-map emc-quiet-cofactors"), &ResolveOptions::default());
        assert_eq!(find(&full, "emc-label-currency").font_size(), 9.5);
        assert_eq!(find(&light, "emc-label-currency").font_size(), 7.5);
        assert!(find(&light, "emc-node-secondary").opacity() < find(&full, "emc-node-secondary").opacity());
        assert_eq!(find(&light, "emc-label-met").font_size(), 12.5, "primary names keep their size");
    }

    #[test]
    fn labels_scale_with_the_zoom() {
        let t = tree();
        let s = styles(&t, &ResolveOptions::default());
        let met = find(&s, "emc-label-met");
        assert_eq!(met.font_size(), 12.5);
        assert_eq!(met.font_weight(), 600);
        assert_eq!(met.get("font-family"), r#""Inter", "Segoe UI", system-ui, -apple-system, sans-serif"#);
        let cur = find(&s, "emc-label-currency");
        assert_eq!((cur.font_size(), cur.opacity()), (9.5, 0.72));
        let rxn = find(&s, "emc-label-rxn");
        assert_eq!((rxn.font_size(), rxn.font_weight(), rxn.dominant_baseline()), (11.0, 700, "central"));
        assert_eq!(rxn.fill(), Paint::Color(Rgba { r: 0x1e, g: 0x29, b: 0x3b, a: 1.0 }), "a presentation attribute");

        let far = styles(&t, &ResolveOptions { label_scale: Some(2.0), is_far: true, ..Default::default() });
        assert_eq!(find(&far, "emc-label-met").font_size(), 25.0);
        assert!(!find(&far, "emc-label-currency").is_displayed(), "small type goes when far out");
        assert!(!find(&far, "emc-label-rxn").is_displayed());
        assert!(find(&far, "emc-label-met").is_displayed());
    }

    #[test]
    fn name_plates_scale_by_the_zoom_up_to_their_fit() {
        let t = tree();
        let at = |frame: Option<f64>| {
            let s = styles(&t, &ResolveOptions { frame_label_scale: frame, ..Default::default() });
            (find(&s, "emc-frame-label").transform_scale(), find(&s, "emc-comp-text").clone())
        };
        let (scale, text) = at(None);
        assert_eq!(scale, Some(0.5), "min(1, --fit)");
        assert_eq!((text.font_size(), text.font_weight(), text.text_transform()), (11.5, 700, "uppercase"));
        assert_eq!(at(Some(0.25)).0, Some(0.25));
        assert_eq!(at(Some(4.0)).0, Some(0.5));
    }

    #[test]
    fn important_rules_beat_attributes_and_cofactors_are_lighter() {
        let t = tree();
        let s = styles(&t, &ResolveOptions::default());
        let node = find(&s, "emc-node-currency");
        // .emc-node.is-multi-selected { stroke: var(--accent, #2563eb) !important; stroke-width: 4px !important }
        // beats .emc-node-secondary { stroke-width: 1.8px !important } on specificity.
        assert_eq!(node.stroke(), Paint::Color(Rgba { r: 0x25, g: 0x63, b: 0xeb, a: 1.0 }));
        assert_eq!(node.stroke_width(), 4.0);
        assert_eq!(node.opacity(), 0.78);
        assert_eq!(node.get("filter"), "url(#emc-glow)");
        assert_eq!(node.fill(), Paint::Color(Rgba { r: 0xe6, g: 0x7e, b: 0x4c, a: 1.0 }));
    }

    #[test]
    fn hover_reaches_the_sibling_glow_in_light_and_dark() {
        let t = tree();
        let pair = t.elements().nth(3).unwrap();
        let hit = pair.elements().next().unwrap();
        let s = styles(&t, &ResolveOptions { hovered: Some(hit), ..Default::default() });
        assert_eq!(find(&s, "emc-boundary-glow").stroke(), Paint::Color(Rgba { r: 37, g: 99, b: 235, a: 0.55 }));
        let mut dark = t.clone();
        dark.set("class", "emc-map emc-dark");
        let hit = dark.elements().nth(3).unwrap().elements().next().unwrap();
        let s = styles(&dark, &ResolveOptions { hovered: Some(hit), ..Default::default() });
        assert_eq!(find(&s, "emc-boundary-glow").stroke(), Paint::Color(Rgba { r: 96, g: 165, b: 250, a: 0.65 }));
        let s = styles(&t, &ResolveOptions::default());
        assert_eq!(find(&s, "emc-boundary-glow").stroke(), Paint::Color(Rgba { r: 0, g: 0, b: 0, a: 0.0 }));
    }

    #[test]
    fn the_page_around_the_map_supplies_its_accent_and_font_size() {
        let mut t = tree();
        t.push(Element::paired("g").child(Element::new("rect").with("class", "emc-multi-area")).child(Element::paired("text").text("x")));
        let alone = styles(&t, &ResolveOptions::default());
        assert_eq!(find(&alone, "emc-multi-area").fill(), Paint::Color(Rgba { r: 0x25, g: 0x63, b: 0xeb, a: 1.0 }), "the fallback");
        assert_eq!(find(&alone, "emc-multi-area").fill_opacity(), 0.1);
        assert_eq!(alone.last().unwrap().1.font_size(), 16.0);
        let opts = ResolveOptions { custom: vec![("--accent".into(), "#ff00aa".into())], root_font_size: 13.0, ..Default::default() };
        let paged = styles(&t, &opts);
        assert_eq!(find(&paged, "emc-multi-area").fill(), Paint::Color(Rgba { r: 0xff, g: 0x00, b: 0xaa, a: 1.0 }));
        assert_eq!(find(&paged, "emc-multi-area").stroke_dasharray(), Some(vec![8.0, 6.0]));
        assert_eq!(paged.last().unwrap().1.font_size(), 13.0, "text with no size of its own");
        assert_eq!(find(&paged, "emc-label-met").font_size(), 12.5, "labels have their own");
    }

    #[test]
    fn one_element_by_its_path() {
        let t = tree();
        let layer = t.elements().next().unwrap();
        let currency = layer.elements().nth(1).unwrap();
        let style = map_stylesheet().computed_style(&[&t, layer, currency], &ResolveOptions { label_scale: Some(1.5), ..Default::default() });
        assert_eq!(style.font_size(), 14.25);
        assert_eq!(style.font_weight(), 500);
    }

    #[test]
    fn math_and_vars() {
        assert_eq!(eval_math("calc(12.5px * 2)", 16.0).as_deref(), Some("25px"));
        assert_eq!(eval_math("max(calc(4 * 2.2px), calc(5px * 1))", 16.0).as_deref(), Some("8.8px"));
        assert_eq!(eval_math("scale(min(1, 1000))", 16.0).as_deref(), Some("scale(1)"));
        assert_eq!(eval_math("calc(0.5em + 2px)", 10.0).as_deref(), Some("7px"));
        assert_eq!(eval_math("calc(1px + 1%)", 10.0), None);
        let mut custom = IndexMap::new();
        custom.insert("--a".to_string(), "3".to_string());
        assert_eq!(substitute_vars("var(--a) var(--b, var(--a))", &custom, None).as_deref(), Some("3 3"));
        assert_eq!(substitute_vars("var(--missing)", &custom, None), None);
    }

    #[test]
    fn colours_parse() {
        assert_eq!(parse_color("#abc"), Some(Rgba { r: 0xaa, g: 0xbb, b: 0xcc, a: 1.0 }));
        assert_eq!(parse_color("rgba(37, 99, 235, 0.55)"), Some(Rgba { r: 37, g: 99, b: 235, a: 0.55 }));
        assert_eq!(parse_paint("url(#emc-wall)"), Paint::Url("#emc-wall".into()));
        assert_eq!(parse_paint("none"), Paint::None);
    }

    #[test]
    fn rule_text_is_normalised() {
        let sheet = StyleSheet::parse(".a,\n.b { fill: red;  stroke : blue !important }\n@media (max-width: 9px) { .emc-x { color: red } }");
        assert_eq!(sheet.rules_text_containing(".a"), [".a, .b { fill: red; stroke: blue !important; }"]);
        assert_eq!(sheet.rules_text_containing(".emc-"), ["@media (max-width: 9px) {\n  .emc-x { color: red; }\n}"]);
    }
}
