/**
 * Undo/redo history for EditableMap documents.
 *
 * History stores plain JSON-compatible map documents, never Model or FBA state.
 * Each entry is the state *before* an edit, which makes it cheap to record an
 * arbitrary mutation after it succeeds.
 *
 * Every state the map passes through gets a version number, and undo/redo go
 * back to the version they restore. That is what lets "Saved" mean the map on
 * screen is the one last written to a file, including after undoing back to it.
 */
export class MapHistory {
  constructor(limit = 100) {
    this.limit = Math.max(1, Number(limit) || 100);
    this.undoStack = [];
    this.redoStack = [];
    this.nextVersion = 1;
    this.version = 0;
    this.savedVersion = 0;
    this.generation = 0;
  }

  get canUndo() { return this.undoStack.length > 0; }
  get canRedo() { return this.redoStack.length > 0; }

  /** True when the map differs from the last one saved, loaded or freshly drawn. */
  get dirty() { return this.version !== this.savedVersion; }

  markSaved(version = this.version, generation = this.generation) {
    if (generation !== this.generation) {
      return;
    }
    this.savedVersion = version;
  }

  /** Start over with a new map, which has nothing in it to save yet. */
  clear() {
    this.generation += 1;
    this.undoStack.length = 0;
    this.redoStack.length = 0;
    this.version = this.nextVersion++;
    this.savedVersion = this.version;
  }

  /**
   * Record the state that existed before a successful edit.
   * Duplicate before/after states are ignored.
   */
  record(beforeDoc, afterDoc, label = 'Edit map') {
    return this.recordWithContext(beforeDoc, afterDoc, label);
  }

  /** Record drawing presentation with an edit when it changes alongside the map. */
  recordWithContext(beforeDoc, afterDoc, label, beforeContext = null, afterContext = null) {
    const before = serialise(beforeDoc);
    const after = serialise(afterDoc);
    const context = beforeContext === null ? null : serialise(beforeContext);
    const nextContext = afterContext === null ? null : serialise(afterContext);
    if (before === after && context === nextContext) {
      return false;
    }

    this.undoStack.push({ label, doc: before, context, version: this.version });
    if (this.undoStack.length > this.limit) this.undoStack.shift();
    this.redoStack.length = 0;
    this.version = this.nextVersion++;
    return true;
  }

  /**
   * Step back one edit. `currentDoc` goes onto the redo stack.
   * @returns {{label: string, doc: object}|null} the restored document, or null
   */
  undo(currentDoc) {
    return this.undoWithContext(currentDoc);
  }

  /** Undo presentation only for an entry that recorded it. */
  undoWithContext(currentDoc, currentContext = null) {
    if (!this.canUndo) return null;
    const entry = this.undoStack.pop();
    const context = serialiseContext(currentContext, entry.context);
    this.redoStack.push({ label: entry.label, doc: serialise(currentDoc), context, version: this.version });
    this.version = entry.version;
    const restored = { label: entry.label, doc: parse(entry.doc) };
    if (entry.context !== null) {
      restored.context = parse(entry.context);
    }
    return restored;
  }

  /**
   * Step forward one undone edit. `currentDoc` goes back onto the undo stack.
   * @returns {{label: string, doc: object}|null} the restored document, or null
   */
  redo(currentDoc) {
    return this.redoWithContext(currentDoc);
  }

  /** Redo presentation only for an entry that recorded it. */
  redoWithContext(currentDoc, currentContext = null) {
    if (!this.canRedo) return null;
    const entry = this.redoStack.pop();
    const context = serialiseContext(currentContext, entry.context);
    this.undoStack.push({ label: entry.label, doc: serialise(currentDoc), context, version: this.version });
    this.version = entry.version;
    const restored = { label: entry.label, doc: parse(entry.doc) };
    if (entry.context !== null) {
      restored.context = parse(entry.context);
    }
    return restored;
  }
}

function serialise(doc) {
  return JSON.stringify(doc);
}

function parse(text) {
  return JSON.parse(text);
}

/** Keep only the presentation fields this history entry changes. */
function serialiseContext(current, recorded) {
  if (recorded === null) {
    return null;
  }
  const context = {};
  for (const key of Object.keys(parse(recorded))) {
    context[key] = current?.[key] ?? null;
  }
  return serialise(context);
}
