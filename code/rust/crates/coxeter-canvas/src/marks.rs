//! `markDraggedNode()` and `markDraggedMetaboliteLabel()` of `app.js`: the
//! classes a drag puts on the drawing (`is-dragging`, `is-merge-target`),
//! applied to each tree the renderer draws so a repaint keeps them, as the
//! JavaScript re-applies them after `paintMap()`.

use coxeter_render::{Element, Node};

use crate::gestures::Marks;

/// Add a class to the first element (document order, `querySelector`) that
/// `matches`.
fn mark_first(e: &mut Element, matches: &dyn Fn(&Element) -> bool, class: &str) -> bool {
    if matches(e) {
        let classes = e.get("class").unwrap_or("").to_string();
        if !classes.split_ascii_whitespace().any(|c| c == class) {
            let next = if classes.is_empty() { class.to_string() } else { format!("{classes} {class}") };
            e.set("class", next);
        }
        return true;
    }
    for c in &mut e.children {
        if let Node::Element(child) = c
            && mark_first(child, matches, class) {
                return true;
            }
    }
    false
}

/// Put a drag's marks on a freshly drawn tree.
pub fn apply_marks(tree: &mut Element, marks: &Marks) {
    let node_circle = |id: &str| {
        let id = id.to_string();
        move |e: &Element| e.has_class("emc-node") && e.get("data-node") == Some(id.as_str())
    };
    if let Some(id) = &marks.dragging_node {
        mark_first(tree, &node_circle(id), "is-dragging");
    }
    if let Some(id) = &marks.merge_target {
        mark_first(tree, &node_circle(id), "is-merge-target");
    }
    if let Some(id) = &marks.dragging_label {
        let id = id.clone();
        mark_first(tree, &move |e: &Element| e.get("data-metabolite-label") == Some(id.as_str()), "is-dragging");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_the_first_match() {
        let mut tree = Element::paired("svg")
            .child(Element::new("circle").with("class", "emc-node").with("data-node", "a"))
            .child(Element::new("circle").with("class", "emc-node").with("data-node", "b"))
            .child(Element::paired("g").with("class", "emc-label-draggable").with("data-metabolite-label", "a"));
        apply_marks(
            &mut tree,
            &Marks { dragging_node: Some("a".into()), merge_target: Some("b".into()), dragging_label: Some("a".into()) },
        );
        let kids: Vec<&Element> = tree.elements().collect();
        assert_eq!(kids[0].get("class"), Some("emc-node is-dragging"));
        assert_eq!(kids[1].get("class"), Some("emc-node is-merge-target"));
        assert_eq!(kids[2].get("class"), Some("emc-label-draggable is-dragging"));
    }
}
