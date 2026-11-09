import test from 'node:test';
import assert from 'node:assert/strict';
import { MapHistory } from '../src/map/history.js';

const doc = (x) => ({ version: 1, nodes: [{ id: 'n', x }] });

test('map history undoes and redoes map documents', () => {
  const h = new MapHistory();
  const a = doc(1);
  const b = doc(2);

  assert.equal(h.record(a, b, 'move'), true);
  assert.equal(h.canUndo, true);
  assert.equal(h.canRedo, false);

  const undone = h.undo(b);
  assert.equal(undone.label, 'move');
  assert.deepEqual(undone.doc, a);
  assert.equal(h.canRedo, true);

  const redone = h.redo(a);
  assert.deepEqual(redone.doc, b);
  assert.equal(h.canUndo, true);
});

test('a new edit clears redo history', () => {
  const h = new MapHistory();
  const a = doc(1);
  const b = doc(2);
  const c = doc(3);

  h.record(a, b, 'first');
  h.undo(b);
  assert.equal(h.canRedo, true);

  h.record(a, c, 'second');
  assert.equal(h.canRedo, false);
});

test('history ignores no-op edits', () => {
  const h = new MapHistory();
  const a = doc(1);
  assert.equal(h.record(a, JSON.parse(JSON.stringify(a)), 'noop'), false);
  assert.equal(h.canUndo, false);
});

test('history enforces its maximum size', () => {
  const h = new MapHistory(2);
  h.record(doc(0), doc(1), 'one');
  h.record(doc(1), doc(2), 'two');
  h.record(doc(2), doc(3), 'three');

  assert.deepEqual(h.undo(doc(3)).doc, doc(2));
  assert.deepEqual(h.undo(doc(2)).doc, doc(1));
  assert.equal(h.undo(doc(1)), null);
});
