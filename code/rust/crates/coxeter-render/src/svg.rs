//! The drawing as a tree of SVG elements.
//!
//! The JavaScript renderer writes its SVG as one string of hand-written
//! markup. Here the same drawing is built as elements first, and the string
//! is one way of writing them out: [`Element::to_markup`] gives exactly the
//! characters `renderMap` returns, layout whitespace included, and
//! [`Element::to_xml`] the form `XMLSerializer` gives once a browser has
//! parsed that markup (what the SVG export writes). A native front end paints
//! the tree itself, with the styles of [`crate::css`].
//!
//! Text and attribute values are kept unescaped; escaping happens when the
//! tree is written. The layout whitespace of the JavaScript templates (the
//! newlines and indentation between attributes and between elements) is
//! kept as well, so the markup can be compared byte for byte with the
//! JavaScript output.

use std::borrow::Cow;

use crate::format::esc_into;

/// Text that is usually a literal of the renderer, sometimes computed.
pub type Str = Cow<'static, str>;

/// One attribute, with the whitespace that precedes it in the markup.
#[derive(Debug, Clone, PartialEq)]
pub struct Attr {
    /// Whitespace written before the name (`" "` almost everywhere).
    pub lead: &'static str,
    pub name: &'static str,
    /// `None` for an attribute written without a value (`data-canvas-bg`).
    pub value: Option<Str>,
}

/// A child of an element.
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Element(Element),
    /// Character data, unescaped. Layout whitespace between elements is text too.
    Text(Str),
}

/// One SVG element.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Element {
    pub name: &'static str,
    pub attrs: Vec<Attr>,
    pub children: Vec<Node>,
    /// Whitespace between the last attribute and the `>` or `/>` of the start tag.
    pub close_ws: &'static str,
    /// Written as a start and an end tag even when empty (`<g></g>`), as the
    /// renderer writes its groups and texts; otherwise an empty element is
    /// closed with `/>`.
    pub paired: bool,
}

impl Element {
    /// An element with no attributes or children, closed with `/>` when empty.
    pub fn new(name: &'static str) -> Element {
        // Most elements of the map carry six to twelve attributes.
        Element { name, attrs: Vec::with_capacity(12), children: Vec::new(), close_ws: "", paired: false }
    }

    /// An element written with an end tag even when it has no content.
    pub fn paired(name: &'static str) -> Element {
        Element { paired: true, ..Element::new(name) }
    }

    /// Add `name="value"`, preceded by one space.
    pub fn attr(&mut self, name: &'static str, value: impl Into<Str>) -> &mut Self {
        self.attrs.push(Attr { lead: " ", name, value: Some(value.into()) });
        self
    }

    /// Add `name="value"` preceded by the given whitespace.
    pub fn attr_ws(&mut self, lead: &'static str, name: &'static str, value: impl Into<Str>) -> &mut Self {
        self.attrs.push(Attr { lead, name, value: Some(value.into()) });
        self
    }

    /// Add an attribute written without a value.
    pub fn flag(&mut self, name: &'static str) -> &mut Self {
        self.attrs.push(Attr { lead: " ", name, value: None });
        self
    }

    /// Builder form of [`Element::attr`].
    pub fn with(mut self, name: &'static str, value: impl Into<Str>) -> Self {
        self.attr(name, value);
        self
    }

    /// Builder form of [`Element::attr_ws`].
    pub fn with_ws(mut self, lead: &'static str, name: &'static str, value: impl Into<Str>) -> Self {
        self.attr_ws(lead, name, value);
        self
    }

    /// Builder: the whitespace before `>` or `/>`.
    pub fn closing(mut self, ws: &'static str) -> Self {
        self.close_ws = ws;
        self
    }

    /// Append a child element.
    pub fn push(&mut self, child: Element) -> &mut Self {
        self.children.push(Node::Element(child));
        self
    }

    /// Append text (or layout whitespace).
    pub fn push_text(&mut self, text: impl Into<Str>) -> &mut Self {
        self.children.push(Node::Text(text.into()));
        self
    }

    /// Builder: append a child element.
    pub fn child(mut self, child: Element) -> Self {
        self.push(child);
        self
    }

    /// Builder: append text (or layout whitespace).
    pub fn text(mut self, text: impl Into<Str>) -> Self {
        self.push_text(text);
        self
    }

    /// The value of an attribute (`Some("")` for one written without a value).
    pub fn get(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|a| a.name == name).map(|a| a.value.as_deref().unwrap_or(""))
    }

    /// Set or replace an attribute's value, keeping its place.
    pub fn set(&mut self, name: &'static str, value: impl Into<Str>) {
        let value = Some(value.into());
        match self.attrs.iter_mut().find(|a| a.name == name) {
            Some(a) => a.value = value,
            None => self.attrs.push(Attr { lead: " ", name, value }),
        }
    }

    /// Remove an attribute.
    pub fn remove(&mut self, name: &str) {
        self.attrs.retain(|a| a.name != name);
    }

    /// The class names of the element.
    pub fn classes(&self) -> impl Iterator<Item = &str> {
        self.get("class").unwrap_or("").split_ascii_whitespace()
    }

    pub fn has_class(&self, class: &str) -> bool {
        self.classes().any(|c| c == class)
    }

    /// The child elements, without the text between them.
    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|c| match c {
            Node::Element(e) => Some(e),
            Node::Text(_) => None,
        })
    }

    /// The text directly inside the element.
    pub fn text_content(&self) -> String {
        let mut out = String::new();
        for c in &self.children {
            match c {
                Node::Text(t) => out.push_str(t),
                Node::Element(e) => out.push_str(&e.text_content()),
            }
        }
        out
    }

    /// Every element of the tree, depth first, this one included.
    pub fn descendants(&self) -> Vec<&Element> {
        fn walk<'a>(e: &'a Element, out: &mut Vec<&'a Element>) {
            out.push(e);
            for c in e.elements() {
                walk(c, out);
            }
        }
        let mut out = Vec::new();
        walk(self, &mut out);
        out
    }

    /// The markup `renderMap` writes: whitespace as the templates have it,
    /// valueless attributes bare, elements without content closed with `/>`.
    pub fn to_markup(&self) -> String {
        let mut out = String::with_capacity(1024);
        self.write_markup(&mut out);
        out
    }

    /// [`Element::to_markup`] appended to a buffer.
    pub fn write_markup(&self, out: &mut String) {
        out.push('<');
        out.push_str(self.name);
        for a in &self.attrs {
            out.push_str(a.lead);
            out.push_str(a.name);
            if let Some(v) = &a.value {
                out.push_str("=\"");
                esc_into(out, v);
                out.push('"');
            }
        }
        out.push_str(self.close_ws);
        if self.children.is_empty() && !self.paired {
            out.push_str("/>");
            return;
        }
        out.push('>');
        for c in &self.children {
            match c {
                Node::Element(e) => e.write_markup(out),
                Node::Text(t) => esc_into(out, t),
            }
        }
        out.push_str("</");
        out.push_str(self.name);
        out.push('>');
    }

    /// What `XMLSerializer` writes for this markup once a browser has parsed
    /// it as HTML: one space between attributes, every attribute with a
    /// value, text escaped as XML text (`"` stays), and an element with no
    /// content closed with `/>`.
    pub fn to_xml(&self) -> String {
        let mut out = String::with_capacity(1024);
        self.write_xml(&mut out);
        out
    }

    /// [`Element::to_xml`] appended to a buffer.
    pub fn write_xml(&self, out: &mut String) {
        out.push('<');
        out.push_str(self.name);
        for a in &self.attrs {
            out.push(' ');
            out.push_str(a.name);
            out.push_str("=\"");
            if let Some(v) = &a.value {
                esc_into(out, v);
            }
            out.push('"');
        }
        // An empty text node is no node at all once parsed.
        let empty = self.children.iter().all(|c| matches!(c, Node::Text(t) if t.is_empty()));
        if empty {
            out.push_str("/>");
            return;
        }
        out.push('>');
        for c in &self.children {
            match c {
                Node::Element(e) => e.write_xml(out),
                Node::Text(t) => xml_text_into(out, t),
            }
        }
        out.push_str("</");
        out.push_str(self.name);
        out.push('>');
    }
}

/// Text as `XMLSerializer` escapes it: `&`, `<` and `>`.
fn xml_text_into(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markup_keeps_layout_whitespace_and_bare_attributes() {
        let e = Element::new("g")
            .with("class", "a")
            .text("\n    ")
            .child(Element::new("rect").with("x", "1").with_ws("\n      ", "fill", "#fff").closing("\n    "))
            .child(Element::new("text").text("a \"b\" & <c>"));
        let mut rect = Element::new("rect");
        rect.flag("data-canvas-bg");
        let e = e.child(rect);
        assert_eq!(
            e.to_markup(),
            "<g class=\"a\">\n    <rect x=\"1\"\n      fill=\"#fff\"\n    /><text>a &quot;b&quot; &amp; &lt;c&gt;</text><rect data-canvas-bg/></g>"
        );
        assert_eq!(
            e.to_xml(),
            "<g class=\"a\">\n    <rect x=\"1\" fill=\"#fff\"/><text>a \"b\" &amp; &lt;c&gt;</text><rect data-canvas-bg=\"\"/></g>"
        );
    }

    #[test]
    fn empty_text_is_an_empty_element_in_xml_only() {
        let e = Element::paired("text").text("");
        assert_eq!(e.to_markup(), "<text></text>");
        assert_eq!(e.to_xml(), "<text/>");
        assert_eq!(Element::paired("g").to_markup(), "<g></g>");
        assert_eq!(Element::paired("g").to_xml(), "<g/>");
    }

    #[test]
    fn descendants_are_in_document_order() {
        let e = Element::new("a")
            .child(Element::new("b").child(Element::new("c")))
            .child(Element::new("d"));
        let names: Vec<&str> = e.descendants().iter().map(|e| e.name).collect();
        assert_eq!(names, ["a", "b", "c", "d"]);
    }
}
