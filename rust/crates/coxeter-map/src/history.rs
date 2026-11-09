//! Undo/redo history for map documents (port of `src/map/history.js`).
//!
//! History stores plain JSON map documents, never model or FBA state. Each
//! entry is the state *before* an edit, which makes it cheap to record an
//! arbitrary mutation after it succeeds.
//!
//! Every state the map passes through gets a version number, and undo/redo
//! go back to the version they restore. That is what lets "Saved" mean the
//! map on screen is the one last written to a file, including after undoing
//! back to it.

use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
struct Entry {
    label: String,
    /// The document as `JSON.stringify` writes it.
    doc: String,
    context: Option<String>,
    version: u64,
}

/// What `undo`/`redo` hand back: the edit's label and the document to show.
#[derive(Debug, Clone, PartialEq)]
pub struct Restored {
    pub label: String,
    pub doc: Value,
    pub context: Option<Value>,
}

/// `MapHistory`.
#[derive(Debug, Clone, PartialEq)]
pub struct MapHistory {
    limit: usize,
    undo_stack: Vec<Entry>,
    redo_stack: Vec<Entry>,
    next_version: u64,
    version: u64,
    saved_version: u64,
    generation: u64,
}

impl Default for MapHistory {
    fn default() -> Self {
        MapHistory::new(100)
    }
}

fn serialise(doc: &Value) -> String {
    coxeter_core::js::stringify(doc, None)
}

/// Keep only the presentation fields this history entry changes.
fn serialise_context(current: Option<&Value>, recorded: Option<&str>) -> Option<String> {
    let recorded: Value = serde_json::from_str(recorded?).expect("history holds JSON");
    let mut context = serde_json::Map::new();
    for key in recorded.as_object().expect("history presentation is an object").keys() {
        let value = match current.and_then(|state| state.get(key)) {
            Some(value) => value.clone(),
            None => Value::Null,
        };
        context.insert(key.clone(), value);
    }
    Some(serialise(&Value::Object(context)))
}

impl MapHistory {
    /// `new MapHistory(limit)`; a limit of 0 is taken as the default 100.
    pub fn new(limit: usize) -> MapHistory {
        MapHistory {
            limit: if limit == 0 { 100 } else { limit },
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            next_version: 1,
            version: 0,
            saved_version: 0,
            generation: 0,
        }
    }

    pub fn limit(&self) -> usize {
        self.limit
    }
    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }
    pub fn undo_len(&self) -> usize {
        self.undo_stack.len()
    }
    pub fn redo_len(&self) -> usize {
        self.redo_stack.len()
    }
    /// The version number of the state on screen.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Which drawing history is active after the last clear.
    pub fn generation(&self) -> u64 {
        self.generation
    }
    /// The labels of the edits that can be undone, oldest first.
    pub fn undo_labels(&self) -> Vec<&str> {
        self.undo_stack.iter().map(|e| e.label.as_str()).collect()
    }

    /// True when the map differs from the last one saved, loaded or freshly drawn.
    pub fn dirty(&self) -> bool {
        self.version != self.saved_version
    }

    /// The state on screen has just been written to a file.
    pub fn mark_saved(&mut self) {
        self.mark_version_saved(self.version, self.generation);
    }

    /// Record the version written before later edits or another history arrived.
    pub fn mark_version_saved(&mut self, version: u64, generation: u64) {
        if generation != self.generation {
            return;
        }
        self.saved_version = version;
    }

    /// Start over with a new map, which has nothing in it to save yet.
    pub fn clear(&mut self) {
        self.generation += 1;
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.version = self.next_version;
        self.next_version += 1;
        self.saved_version = self.version;
    }

    /// Record the state that existed before a successful edit. Duplicate
    /// before/after states are ignored (`false`).
    pub fn record(&mut self, before: &Value, after: &Value, label: &str) -> bool {
        self.record_with_context(before, after, label, None, None)
    }

    /// `recordWithContext`: presentation that changes alongside the drawing.
    pub fn record_with_context(
        &mut self, before: &Value, after: &Value, label: &str,
        before_context: Option<&Value>, after_context: Option<&Value>,
    ) -> bool {
        let before = serialise(before);
        let after = serialise(after);
        let context = before_context.map(serialise);
        let next_context = after_context.map(serialise);
        if before == after && context == next_context {
            return false;
        }
        self.undo_stack.push(Entry { label: label.to_string(), doc: before, context, version: self.version });
        if self.undo_stack.len() > self.limit {
            self.undo_stack.remove(0);
        }
        self.redo_stack.clear();
        self.version = self.next_version;
        self.next_version += 1;
        true
    }

    /// Step back one edit. `current` is the document on screen, kept for redo.
    /// `None` when there is nothing to undo.
    pub fn undo(&mut self, current: &Value) -> Option<Restored> {
        self.undo_with_context(current, None)
    }

    /// `undoWithContext`: undo presentation only for an entry that recorded it.
    pub fn undo_with_context(&mut self, current: &Value, current_context: Option<&Value>) -> Option<Restored> {
        let entry = self.undo_stack.pop()?;
        let context = serialise_context(current_context, entry.context.as_deref());
        self.redo_stack.push(Entry { label: entry.label.clone(), doc: serialise(current), context, version: self.version });
        self.version = entry.version;
        Some(Restored {
            label: entry.label,
            doc: serde_json::from_str(&entry.doc).expect("history holds JSON"),
            context: entry.context.map(|value| serde_json::from_str(&value).expect("history holds JSON")),
        })
    }

    /// Step forward one undone edit. `current` is the document on screen, kept
    /// for undo. `None` when there is nothing to redo.
    pub fn redo(&mut self, current: &Value) -> Option<Restored> {
        self.redo_with_context(current, None)
    }

    /// `redoWithContext`: redo presentation only for an entry that recorded it.
    pub fn redo_with_context(&mut self, current: &Value, current_context: Option<&Value>) -> Option<Restored> {
        let entry = self.redo_stack.pop()?;
        let context = serialise_context(current_context, entry.context.as_deref());
        self.undo_stack.push(Entry { label: entry.label.clone(), doc: serialise(current), context, version: self.version });
        self.version = entry.version;
        Some(Restored {
            label: entry.label,
            doc: serde_json::from_str(&entry.doc).expect("history holds JSON"),
            context: entry.context.map(|value| serde_json::from_str(&value).expect("history holds JSON")),
        })
    }
}

/// The JavaScript's default label of an edit.
pub const DEFAULT_LABEL: &str = "Edit map";
