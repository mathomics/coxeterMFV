//! The drawn SVG as a document: every element of the tree `render_map`
//! returns, numbered in document order, with its parent, so that a pointer
//! target can be walked up the way the DOM's `element.closest(selector)` walks
//! it (`app.js` resolves every target that way, through the `data-` attributes
//! the renderer puts on its elements).
//!
//! Selectors are the small subset `app.js` passes to `closest()`: a list of
//! compounds of a tag, `.class`es and `[attr]` / `[attr="value"]` tests.

use coxeter_render::{Element, Node};

/// One element of the document. Index 0 is the `<svg>` root.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DocNode {
    /// The parent's index (`u32::MAX` for the root).
    pub parent: u32,
    /// Where it is among its parent's `children` (text included).
    pub slot: u32,
    /// One past the last element of its subtree: the subtree is `i..end`.
    pub end: u32,
    pub depth: u16,
}

/// An element tree with its elements numbered in document order.
#[derive(Debug, Clone)]
pub struct Doc {
    pub root: Element,
    pub nodes: Vec<DocNode>,
}

/// [`DocNode::parent`] of the root.
pub const NO_PARENT: u32 = u32::MAX;

impl Doc {
    pub fn new(root: Element) -> Doc {
        let mut nodes = Vec::with_capacity(1024);
        fn walk(e: &Element, parent: u32, slot: u32, depth: u16, nodes: &mut Vec<DocNode>) {
            let me = nodes.len() as u32;
            nodes.push(DocNode { parent, slot, end: 0, depth });
            for (k, c) in e.children.iter().enumerate() {
                if let Node::Element(child) = c {
                    walk(child, me, k as u32, depth + 1, nodes);
                }
            }
            let end = nodes.len() as u32;
            nodes[me as usize].end = end;
        }
        walk(&root, NO_PARENT, 0, 0, &mut nodes);
        Doc { root, nodes }
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// The element at index `i`.
    pub fn element(&self, i: usize) -> &Element {
        self.element_in(&self.root, i)
    }

    /// The element at index `i` of another tree of the same shape (the one
    /// drawn before an update, say).
    pub fn element_in<'a>(&self, root: &'a Element, i: usize) -> &'a Element {
        if i == 0 {
            return root;
        }
        let n = self.nodes[i];
        let parent = self.element_in(root, n.parent as usize);
        match &parent.children[n.slot as usize] {
            Node::Element(e) => e,
            Node::Text(_) => unreachable!("a numbered slot holds an element"),
        }
    }

    /// The element at index `i`, mutably.
    pub fn element_mut(&mut self, i: usize) -> &mut Element {
        let mut chain = Vec::new();
        let mut k = i;
        while k != 0 {
            chain.push(self.nodes[k].slot as usize);
            k = self.nodes[k].parent as usize;
        }
        let mut e = &mut self.root;
        for slot in chain.into_iter().rev() {
            e = match &mut e.children[slot] {
                Node::Element(c) => c,
                Node::Text(_) => unreachable!("a numbered slot holds an element"),
            };
        }
        e
    }

    /// The elements from the root down to `i`.
    pub fn path(&self, i: usize) -> Vec<&Element> {
        let mut idx = Vec::new();
        let mut k = i;
        loop {
            idx.push(k);
            if k == 0 {
                break;
            }
            k = self.nodes[k].parent as usize;
        }
        let mut out = Vec::with_capacity(idx.len());
        let mut e = &self.root;
        out.push(e);
        for &k in idx.iter().rev().skip(1) {
            e = match &e.children[self.nodes[k].slot as usize] {
                Node::Element(c) => c,
                Node::Text(_) => unreachable!(),
            };
            out.push(e);
        }
        out
    }

    pub fn parent(&self, i: usize) -> Option<usize> {
        let p = self.nodes[i].parent;
        (p != NO_PARENT).then_some(p as usize)
    }

    /// The indices of `i`'s child elements.
    pub fn children(&self, i: usize) -> impl Iterator<Item = usize> + '_ {
        let end = self.nodes[i].end as usize;
        let mut k = i + 1;
        std::iter::from_fn(move || {
            if k >= end {
                return None;
            }
            let at = k;
            k = self.nodes[k].end as usize;
            Some(at)
        })
    }

    /// `i` and its ancestors, nearest first.
    pub fn ancestors(&self, i: usize) -> impl Iterator<Item = usize> + '_ {
        let mut k = Some(i);
        std::iter::from_fn(move || {
            let at = k?;
            k = self.parent(at);
            Some(at)
        })
    }

    /// `element.closest(selector)`: `i` or its nearest ancestor that matches.
    pub fn closest(&self, i: usize, sel: &Selector) -> Option<usize> {
        // Walk down once and test on the way back up, nearest first.
        let path = self.path(i);
        let idx: Vec<usize> = self.ancestors(i).collect();
        for (n, e) in path.iter().rev().enumerate() {
            if sel.matches(e) {
                return Some(idx[n]);
            }
        }
        None
    }

    /// An attribute of element `i`.
    pub fn attr(&self, i: usize, name: &str) -> Option<&str> {
        self.element(i).get(name)
    }

    /// `closest(selector)?.dataset[key]`: the attribute `name` of the nearest
    /// element (from `i` up) that matches `sel`.
    pub fn closest_attr(&self, i: usize, sel: &Selector, name: &str) -> Option<String> {
        let at = self.closest(i, sel)?;
        self.attr(at, name).map(str::to_string)
    }

    /// Whether `i` lies in the subtree of `ancestor`.
    pub fn is_within(&self, i: usize, ancestor: usize) -> bool {
        i >= ancestor && i < self.nodes[ancestor].end as usize
    }
}

/// One attribute test of a compound selector.
#[derive(Debug, Clone, PartialEq)]
pub struct AttrTest {
    pub name: String,
    pub value: Option<String>,
}

/// `tag.class1.class2[attr][attr="v"]`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Compound {
    pub tag: Option<String>,
    pub classes: Vec<String>,
    pub attrs: Vec<AttrTest>,
}

impl Compound {
    pub fn matches(&self, e: &Element) -> bool {
        if self.tag.as_deref().is_some_and(|t| t != e.name) {
            return false;
        }
        if !self.classes.iter().all(|c| e.has_class(c)) {
            return false;
        }
        self.attrs.iter().all(|a| match (e.get(&a.name), &a.value) {
            (None, _) => false,
            (Some(_), None) => true,
            (Some(v), Some(want)) => v == want,
        })
    }
}

/// A selector list as `closest()` takes it: `a, b, c`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Selector {
    pub any: Vec<Compound>,
}

impl Selector {
    /// Parse the selectors `app.js` uses. Panics on anything else: the
    /// selectors are literals of this crate.
    pub fn parse(text: &str) -> Selector {
        let mut any = Vec::new();
        for part in text.split(',') {
            let s = part.trim();
            if s.is_empty() {
                continue;
            }
            let mut c = Compound::default();
            let b = s.as_bytes();
            let mut i = 0;
            let ident = |i: &mut usize| {
                let start = *i;
                while *i < b.len() && (b[*i].is_ascii_alphanumeric() || b[*i] == b'-' || b[*i] == b'_') {
                    *i += 1;
                }
                s[start..*i].to_string()
            };
            while i < b.len() {
                match b[i] {
                    b'.' => {
                        i += 1;
                        c.classes.push(ident(&mut i));
                    }
                    b'[' => {
                        i += 1;
                        let name = ident(&mut i);
                        let value = if b[i] == b'=' {
                            i += 1;
                            let quote = b[i];
                            assert!(quote == b'"' || quote == b'\'', "selector {s}: quoted values only");
                            i += 1;
                            let start = i;
                            while b[i] != quote {
                                i += 1;
                            }
                            let v = s[start..i].to_string();
                            i += 1;
                            Some(v)
                        } else {
                            None
                        };
                        assert_eq!(b[i], b']', "selector {s}");
                        i += 1;
                        c.attrs.push(AttrTest { name, value });
                    }
                    c2 if c2.is_ascii_alphabetic() => {
                        c.tag = Some(ident(&mut i));
                    }
                    _ => panic!("selector {s}: unsupported syntax at {i}"),
                }
            }
            any.push(c);
        }
        Selector { any }
    }

    pub fn matches(&self, e: &Element) -> bool {
        self.any.iter().any(|c| c.matches(e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closest_walks_up() {
        let tree = Element::paired("svg")
            .with("class", "emc-map")
            .text("\n")
            .child(
                Element::paired("g")
                    .with("class", "emc-layer")
                    .child(Element::paired("g").with("data-rxn", "R1").child(Element::new("rect").with("x", "1"))),
            )
            .child(Element::new("circle").with("class", "emc-node emc-node-metabolite").with("data-node", "n1"));
        let doc = Doc::new(tree);
        assert_eq!(doc.len(), 5);
        assert_eq!(doc.element(3).name, "rect");
        assert_eq!(doc.element(4).get("data-node"), Some("n1"));
        assert_eq!(doc.closest_attr(3, &Selector::parse("[data-rxn]"), "data-rxn").as_deref(), Some("R1"));
        assert_eq!(doc.closest(4, &Selector::parse(".emc-node[data-node], .emc-hit-node[data-node]")), Some(4));
        assert_eq!(doc.closest(3, &Selector::parse(".emc-node[data-node]")), None);
        assert_eq!(doc.children(0).collect::<Vec<_>>(), vec![1, 4]);
        assert_eq!(doc.ancestors(3).collect::<Vec<_>>(), vec![3, 2, 1, 0]);
        assert!(Selector::parse(r#"g[data-rxn="R1"]"#).matches(doc.element(2)));
        assert_eq!(doc.path(3).len(), 4);
        doc.clone().element_mut(3).set("x", "2");
    }
}
