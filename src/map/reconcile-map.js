/**
 * Bringing a saved map up to date with the model it is opened over.
 *
 * A map is a drawing of a model at the moment it was saved. Two things move on
 * after that. The model does: a reconstruction that is still being worked on
 * loses a reaction here and renames a metabolite there between one export and
 * the next. And this program does: a map remembers how its compartments were
 * nested when it was drawn, and that nesting comes from the model's compartment
 * tree, which later builds read differently (maps from earlier builds draw the
 * intermembrane space, written `i`, as a box beside the cell, though it belongs
 * around the matrix).
 *
 * Neither is a reason to turn a map away. What is drawn and still exists is
 * kept exactly where it was put; what the model no longer has is taken off the
 * map; and compartments are moved to where the model nests them, taking their
 * contents along. Everything done is reported, so nothing changes silently.
 *
 * Pure geometry and membership: the model and the LP are never touched.
 */

import {
  cellCompartmentName, cellCompartmentVisualId, membraneStyle,
} from '../core/compartments.js';
import { LINK_COMPARTMENT } from '../core/community.js';
import { clampNodePosition, updateConnectedSegments } from '../layout/drag.js';
import { closestBoundaryPoint } from '../layout/geometry.js';
import { straightenLinearReactions } from '../layout/reaction-edit.js';
import { reactionBoundaryKeys } from './boundaries.js';
import { editGeometryRect } from './geometry-edit.js';

const METABOLITE_KINDS = new Set(['metabolite', 'medium', 'currency']);

/**
 * The band a compartment is given around the one it encloses. A metabolite has
 * to fit in it clear of both membranes, which takes 42 units; a little over
 * that leaves room to put it down without touching either.
 */
const WRAP_SHELL = 52;
const MIN_RING = 44;
const CHILD_PAD = 14;

// Contents

/**
 * Take off the map what the model no longer has.
 *
 * A reaction that is gone is removed, with the nodes only it used. A reaction
 * that is still there but was drawn with a metabolite the model no longer knows
 * has changed its stoichiometry; it is drawn afresh in the same place, because
 * a curve to a metabolite that does not exist cannot be kept. A reaction whose
 * drawn metabolites merely differ from the model's is left as it is and named.
 *
 * Species in `keepSpecies` are switched off, not gone: what they have drawn is
 * missing from the community only while they are out, and is kept. That is
 * their own `sp@…` reactions and metabolites, and also the shared-medium nodes
 * those reactions are drawn to and the community exchanges on those nodes,
 * which leave the community with them when nothing else uses the metabolite.
 *
 * @returns {{dropped: string[], redrawn: string[], changed: string[], nodes: number}}
 */
export function pruneMapToModel(map, community, { keepSpecies = new Set() } = {}) {
  const report = { dropped: [], redrawn: [], changed: [], nodes: 0 };
  const reactions = community.merged.reactions;
  const metabolites = community.merged.metabolites;
  const kept = (id) => keepSpecies.size > 0 && keepSpecies.has(String(id ?? '').split('@')[0]);
  const keptNodes = new Set();
  const keptReactions = new Set();
  if (keepSpecies.size > 0) {
    for (const seg of map.segments) {
      if (seg.nodeId && kept(seg.reactionId)) keptNodes.add(seg.nodeId);
    }
    for (const seg of map.segments) {
      // Community-level only (`EX_x_s`): a missing `sp@…` reaction of a species
      // that is still on is really gone, even when it shares the node.
      if (seg.nodeId && keptNodes.has(seg.nodeId) && !reactions.has(seg.reactionId)
        && !String(seg.reactionId).includes('@')) keptReactions.add(seg.reactionId);
    }
  }

  for (const id of [...map.reactions.keys()]) {
    if (reactions.has(id) || kept(id) || keptReactions.has(id)) continue;
    map.removeReaction(id);
    report.dropped.push(id);
  }

  const drawnWith = new Map();   // reaction id -> merged ids of its metabolite nodes
  for (const seg of map.segments) {
    if (!seg.nodeId) continue;
    const node = map.nodes.get(seg.nodeId);
    if (!node?.mergedId) continue;
    if (!drawnWith.has(seg.reactionId)) drawnWith.set(seg.reactionId, new Set());
    drawnWith.get(seg.reactionId).add(node.mergedId);
  }

  for (const [id, drawn] of drawnWith) {
    const rxn = map.reactions.get(id);
    if (!rxn || !reactions.has(id)) continue;
    const broken = [...drawn].some((mid) => !metabolites.has(mid));
    if (broken) {
      const at = { x: rxn.x, y: rxn.y };
      map.removeReaction(id);
      if (map.addReaction(community, id, { anchor: at })) report.redrawn.push(id);
      else report.dropped.push(id);
      continue;
    }
    const wanted = Object.keys(reactions.get(id).metabolites)
      .filter((mid) => metabolites.get(mid)?.compartment !== LINK_COMPARTMENT);
    if (wanted.length !== drawn.size || wanted.some((mid) => !drawn.has(mid))) report.changed.push(id);
  }

  // What is left are metabolites standing on their own that the model has lost.
  for (const [id, node] of [...map.nodes]) {
    if (!METABOLITE_KINDS.has(node.kind) || !node.mergedId || metabolites.has(node.mergedId)
      || kept(node.mergedId) || keptNodes.has(id)) continue;
    map.nodes.delete(id);
    map.segments = map.segments.filter((seg) => seg.nodeId !== id);
    report.nodes += 1;
  }
  if (map.medium) {
    map.medium.nodes = (map.medium.nodes ?? []).filter((id) => map.nodes.has(id));
    map.medium.exchanges = (map.medium.exchanges ?? []).filter((id) => map.reactions.has(id));
  }
  return report;
}

// Compartments

/**
 * Nest the drawn compartments the way the model nests them, and put back
 * inside its compartment whatever is drawn outside it.
 *
 * Three things can be wrong with a compartment, and each has its own remedy:
 *
 *   it has to go around one that is already standing (the intermembrane space
 *     around the matrix): it takes that one's box, and the inner one is drawn
 *     in a little, so nothing next to them has to move;
 *   it is drawn in the wrong place altogether (the eyespot beside the cell):
 *     it moves into its parent as it is, contents and all, to a free spot;
 *   its contents are not in it (earlier builds drew a cell's `s` reactions out
 *     in the shared medium): they are drawn afresh inside it.
 *
 * What is already where it belongs is not touched: a map that needs nothing
 * comes out exactly as it went in.
 *
 * @returns {{wrapped: object[], moved: object[], left: object[], reseated: string[], pulled: number}}
 */
export function reconcileCompartments(map, community) {
  const report = { wrapped: [], moved: [], left: [], reseated: [], pulled: 0 };
  const touched = new Set(mendNodeOwnership(map, community));   // cells to look over

  for (const sp of community.activeSpecies ?? []) {
    const cell = map.cells.find((c) => c.id === sp.id);
    if (!cell) continue;
    const tree = sp.model.compartmentTree();
    const names = sp.model.compartmentNames;
    const drawn = () => new Map(cell.compartments.map((c) => [c.id, c]));
    const pending = [];   // reactions taken off to be drawn afresh: { id, at }

    const takeOff = (ids) => {
      for (const id of ids) {
        const rxn = map.reactions.get(id);
        if (!rxn) continue;
        pending.push({ id, at: { x: rxn.x, y: rxn.y }, onMembrane: Boolean(rxn.onMembrane) });
        map.removeReaction(id);
      }
    };

    /** Put `outer` around `inner`, which stands in `parent`. */
    const wrap = (outer, inner, parent) => {
      // What reaches into the outer compartment was drawn to wherever it used
      // to be, or to nowhere; it is drawn again once the two are in place.
      takeOff(reactionsTouching(map, cell.id, outer.id));
      const box = { ...inner.rect };
      outer.rect = { ...box };
      outer.parentId = parent.id;
      inner.parentId = outer.id;
      setDepths(cell);
      map.refreshBoundaries?.();
      // The inner one gives up a band along its edge, as far as what is drawn
      // in it allows.
      editGeometryRect(map, cell.id, inner.id, inset(box, WRAP_SHELL));
      // Where its contents would not let it, the outer one grows instead, if
      // its neighbours leave it the room.
      const siblings = cell.compartments.filter((c) =>
        (c.parentId ?? null) === parent.id && c !== outer);
      const limit = inset(parent.rect, 6);
      for (const side of ['left', 'right', 'top', 'bottom']) {
        const short = MIN_RING - ringWidth(outer.rect, inner.rect, side);
        if (short <= 0) continue;
        const grown = grow(outer.rect, side, short);
        if (contains(limit, grown) && !siblings.some((c) => overlaps(grown, c.rect, 4))) outer.rect = grown;
      }
      map.refreshBoundaries?.();
      touched.add(cell.id);
    };

    // Outside in, so a compartment's parent is settled before it is.
    const order = [...tree.byId.values()].sort((a, b) => a.depth - b.depth || a.id.localeCompare(b.id));
    for (const node of order) {
      const comps = drawn();
      const comp = comps.get(node.id);
      const want = node.parent ?? null;
      if (!comp) continue;
      if ((comp.parentId ?? null) === want) continue;

      const parent = want ? comps.get(want) : null;

      // The model has put a compartment between this one and the one it was
      // drawn in, and that compartment is not drawn at all. Draw it here:
      // nothing else could ever get it around what it has to contain.
      if (want && !parent) {
        const between = tree.byId.get(want);
        const outerParent = between ? comps.get(between.parent ?? '') : null;
        if (between && outerParent && (comp.parentId ?? null) === (between.parent ?? null)) {
          const visualId = cellCompartmentVisualId(want, names);
          const created = {
            id: want,
            rect: { ...comp.rect },
            depth: 0,
            parentId: outerParent.id,
            label: cellCompartmentName(want, names),
            membrane: membraneStyle(visualId),
            visualId,
          };
          cell.compartments.push(created);
          wrap(created, comp, outerParent);
          report.wrapped.push({ cell: cell.id, id: want, around: [comp.id], created: true });
        } else {
          report.left.push({ cell: cell.id, id: comp.id, reason: `its parent ${want} is not drawn` });
        }
        continue;
      }

      if (!want) {
        // Was nested, is now a compartment of its own beside the cell. It keeps
        // its place; only what it is said to be inside changes.
        comp.parentId = null;
        touched.add(cell.id);
        report.moved.push({ cell: cell.id, id: comp.id, into: null });
        continue;
      }

      // Drawn elsewhere, and it has to go around a compartment that is already
      // standing in its parent.
      const inside = cell.compartments.filter((c) =>
        tree.byId.get(c.id)?.parent === comp.id && (c.parentId ?? null) === want);
      const ownChildren = cell.compartments.filter((c) => (c.parentId ?? null) === comp.id);
      if (inside.length === 1 && !ownChildren.length) {
        wrap(comp, inside[0], parent);
        report.wrapped.push({ cell: cell.id, id: comp.id, around: [inside[0].id] });
        continue;
      }

      // Otherwise it moves into its parent as it is, with everything in it.
      const spot = findSpot(map, cell, comp, parent);
      if (!spot) {
        report.left.push({ cell: cell.id, id: comp.id, reason: `there is no room for it in ${want}` });
        continue;
      }
      const was = { parentId: comp.parentId ?? null, rect: { ...comp.rect } };
      comp.parentId = want;
      if (spot.resized) comp.rect = { ...comp.rect, w: spot.w, h: spot.h };
      setDepths(cell);
      map.refreshBoundaries?.();
      if (!editGeometryRect(map, cell.id, comp.id, { x: spot.x, y: spot.y, w: comp.rect.w, h: comp.rect.h })) {
        comp.parentId = was.parentId;
        comp.rect = was.rect;
        setDepths(cell);
        map.refreshBoundaries?.();
        report.left.push({ cell: cell.id, id: comp.id, reason: `it could not be moved into ${want}` });
        continue;
      }
      touched.add(cell.id);
      report.moved.push({ cell: cell.id, id: comp.id, into: want });
    }

    if (!touched.has(cell.id)) continue;

    setDepths(cell);
    const roots = cell.compartments.filter((c) => (c.parentId ?? null) === null);
    if (roots.length) cell.rect = union(roots.map((c) => c.rect));
    if (!roots.some((c) => c.id === cell.outerCompartment)) {
      cell.outerCompartment = (roots.find((c) => c.id === 'e') ?? roots[0])?.id ?? cell.outerCompartment;
    }
    map.refreshBoundaries?.();
    const boxes = drawn();

    // A transporter has to be on a membrane it crosses, which for the same
    // reaction is a different one once the nesting changes. And a reaction
    // inside a compartment has to be inside it.
    const cache = new Map();
    const misplaced = [];
    const strays = new Map();   // compartment id -> reactions drawn outside it
    for (const [id, rxn] of map.reactions) {
      if (rxn.cellId !== cell.id) continue;
      if (rxn.onMembrane && rxn.boundaryId) {
        if (!reactionBoundaryKeys(community, id, cache).includes(rxn.boundaryId)) misplaced.push(id);
        continue;
      }
      const home = (rxn.compartments ?? []).map((cid) => boxes.get(cid)).find(Boolean);
      if (home && !holds(home.rect, rxn)) {
        if (!strays.has(home.id)) strays.set(home.id, []);
        strays.get(home.id).push(id);
      }
    }
    takeOff(misplaced);
    // Reactions found outside their compartment all want the same nearest
    // corner of it; they are shared out over the box instead.
    const places = new Map();
    for (const [compId, ids] of strays) {
      const r = boxes.get(compId).rect;
      const cols = Math.max(1, Math.ceil(Math.sqrt((ids.length * r.w) / Math.max(r.h, 1))));
      const rows = Math.max(1, Math.ceil(ids.length / cols));
      ids.forEach((id, k) => places.set(id, {
        x: r.x + (((k % cols) + 0.5) / cols) * r.w,
        y: r.y + ((Math.floor(k / cols) + 0.5) / rows) * r.h,
      }));
      takeOff(ids);
    }

    for (const { id, at } of pending) {
      if (map.reactions.has(id)) continue;
      if (map.addReaction(community, id, { anchor: places.get(id) ?? at })) report.reseated.push(id);
    }

    // Transporters still on the right membrane, whose membrane has moved.
    for (const [id, rxn] of map.reactions) {
      if (rxn.cellId !== cell.id || !rxn.onMembrane || !rxn.boundaryId) continue;
      const boundary = map.boundary?.(rxn.boundaryId);
      if (!boundary?.rect) continue;
      const p = closestBoundaryPoint(map, boundary, rxn);
      if (Math.hypot(p.x - rxn.x, p.y - rxn.y) > 0.5) map.moveReaction(id, rxn.x, rxn.y);
    }

    // Last, any metabolite still outside the compartment it belongs to.
    for (const node of map.nodes.values()) {
      if (node.cellId !== cell.id || node.kind === 'medium' || !METABOLITE_KINDS.has(node.kind)) continue;
      const home = boxes.get(node.compartment);
      if (!home || holds(home.rect, node)) continue;
      const p = clampNodePosition(map, node, node.x, node.y);
      const dx = p.x - node.x;
      const dy = p.y - node.y;
      if (Math.abs(dx) + Math.abs(dy) < 0.01) continue;
      node.x = p.x;
      node.y = p.y;
      updateConnectedSegments(map, node.id, { x: dx, y: dy });
      report.pulled += 1;
    }
  }

  if (touched.size) {
    map.refreshBoundaries?.();
    straightenLinearReactions(map);
  }
  return report;
}

/**
 * Say in one sentence what bringing a map up to date changed, or '' if nothing.
 * `contents` is the report of pruneMapToModel and `shell` that of
 * reconcileCompartments; either may be null. A non-empty sentence starts with a
 * space, so it can be appended to another message.
 */
export function describeReconciliation(contents, shell) {
  const parts = [];
  const list = (ids, strip = true) => {
    const names = ids.slice(0, 4).map((id) => (strip ? String(id).replace(/^[^@]*@/, '') : id));
    return names.join(', ') + (ids.length > 4 ? ` and ${ids.length - 4} more` : '');
  };
  if (contents?.dropped.length) {
    parts.push(`${contents.dropped.length} reaction${contents.dropped.length === 1 ? '' : 's'} the model no longer has ${contents.dropped.length === 1 ? 'was' : 'were'} left out (${list(contents.dropped)})`);
  }
  if (contents?.redrawn.length) {
    parts.push(`${contents.redrawn.length} drawn afresh because the model has changed their metabolites (${list(contents.redrawn)})`);
  }
  if (contents?.changed.length) {
    parts.push(`${contents.changed.length} still drawn with metabolites the model has since changed (${list(contents.changed)})`);
  }
  if (contents?.nodes) {
    parts.push(`${contents.nodes} metabolite${contents.nodes === 1 ? '' : 's'} the model no longer has removed`);
  }
  const placed = [
    ...(shell?.wrapped ?? []).map((w) => `${w.id} around ${w.around.join(', ')}`),
    ...(shell?.moved ?? []).map((m) => (m.into ? `${m.id} into ${m.into}` : `${m.id} out beside the cell`)),
  ];
  if (placed.length) parts.push(`compartments put where the model nests them (${placed.join('; ')})`);
  if (shell?.reseated.length) {
    parts.push(`${shell.reseated.length} reaction${shell.reseated.length === 1 ? '' : 's'} there drawn afresh (${list(shell.reseated)})`);
  }
  if (shell?.left.length) {
    parts.push(`${shell.left.map((l) => `${l.id} left where it was: ${l.reason}`).join('; ')}`);
  }
  return parts.length ? ` Brought up to date with the model: ${parts.join('; ')}.` : '';
}

// Helpers

/**
 * A metabolite node says where it lives through `kind` and `cellId`, and maps
 * saved by earlier builds have two things wrong there. A metabolite in a cell's
 * own `s` compartment was taken for one in the shared medium, which shares the
 * letter, and held out in the medium. And a real medium node was filed under
 * the cell of the reaction that placed it, so that moving that cell's
 * compartments would now carry it off with them.
 */
function mendNodeOwnership(map, community) {
  const cells = new Set();
  for (const node of map.nodes.values()) {
    if (!METABOLITE_KINDS.has(node.kind)) continue;
    const owner = community.metaboliteOwner?.get(node.mergedId);
    if (!owner) continue;
    if (owner.speciesId === null || owner.speciesId === undefined) {
      if (node.kind === 'medium') node.cellId = null;
      continue;
    }
    if (node.kind !== 'medium') continue;
    node.kind = 'metabolite';
    node.cellId = owner.speciesId;
    node.compartment = owner.compartment;
    if (map.medium) map.medium.nodes = (map.medium.nodes ?? []).filter((id) => id !== node.id);
    cells.add(owner.speciesId);
  }
  return cells;
}

/** Reactions drawn with a metabolite in one compartment of one cell. */
function reactionsTouching(map, cellId, compartmentId) {
  const out = new Set();
  for (const seg of map.segments) {
    const node = seg.nodeId ? map.nodes.get(seg.nodeId) : null;
    if (node && node.cellId === cellId && node.compartment === compartmentId) out.add(seg.reactionId);
  }
  return out;
}

function holds(rect, point) {
  return point.x >= rect.x && point.x <= rect.x + rect.w
    && point.y >= rect.y && point.y <= rect.y + rect.h;
}

/** How wide the band between an enclosing box and the enclosed one is on one side. */
function ringWidth(outer, inner, side) {
  if (side === 'left') return inner.x - outer.x;
  if (side === 'right') return (outer.x + outer.w) - (inner.x + inner.w);
  if (side === 'top') return inner.y - outer.y;
  return (outer.y + outer.h) - (inner.y + inner.h);
}

function grow(r, side, by) {
  if (side === 'left') return { x: r.x - by, y: r.y, w: r.w + by, h: r.h };
  if (side === 'right') return { x: r.x, y: r.y, w: r.w + by, h: r.h };
  if (side === 'top') return { x: r.x, y: r.y - by, w: r.w, h: r.h + by };
  return { x: r.x, y: r.y, w: r.w, h: r.h + by };
}

function setDepths(cell) {
  const byId = new Map(cell.compartments.map((c) => [c.id, c]));
  for (const comp of cell.compartments) {
    let depth = 0;
    let at = comp;
    const seen = new Set([comp.id]);
    while (at.parentId && byId.has(at.parentId) && !seen.has(at.parentId)) {
      at = byId.get(at.parentId);
      seen.add(at.id);
      depth += 1;
    }
    comp.depth = depth;
  }
}

/**
 * Where in `parent` a compartment drawn somewhere else can be set down: clear
 * of the other compartments in there, and on as little of what is already
 * drawn as possible. An empty compartment is cut down to fit; one with
 * something in it keeps its size, since its contents keep their arrangement.
 */
function findSpot(map, cell, comp, parent) {
  const area = inset(parent.rect, CHILD_PAD);
  const subtree = new Set([comp.id]);
  let grew = true;
  while (grew) {
    grew = false;
    for (const c of cell.compartments) {
      if (c.parentId && subtree.has(c.parentId) && !subtree.has(c.id)) { subtree.add(c.id); grew = true; }
    }
  }
  // Only what is actually drawn in the box travels with it. Contents that are
  // filed under the compartment but drawn somewhere else are put right later.
  const occupied = subtree.size > 1
    || [...map.nodes.values()].some((n) => n.cellId === cell.id && subtree.has(n.compartment)
      && holds(comp.rect, n))
    || [...map.reactions.values()].some((r) => r.cellId === cell.id && holds(comp.rect, r)
      && ((r.compartments ?? []).some((id) => subtree.has(id)) || subtree.has(r.membrane)));

  const siblings = cell.compartments.filter((c) => (c.parentId ?? null) === parent.id && c !== comp);
  const marks = [];
  for (const node of map.nodes.values()) if (!node.hidden) marks.push(node);
  for (const rxn of map.reactions.values()) marks.push(rxn);
  for (const text of map.texts?.values?.() ?? []) marks.push(text);
  const from = { x: comp.rect.x + comp.rect.w / 2, y: comp.rect.y + comp.rect.h / 2 };

  const search = (w, h, resized) => {
    if (w > area.w || h > area.h) return null;
    const step = Math.max(24, Math.min(w, h) / 3, Math.max(area.w - w, area.h - h) / 60);
    let best = null;
    for (let y = area.y; y <= area.y + area.h - h + 0.5; y += step) {
      for (let x = area.x; x <= area.x + area.w - w + 0.5; x += step) {
        const r = { x, y, w, h };
        if (siblings.some((s) => overlaps(r, s.rect, CHILD_PAD))) continue;
        const grown = inset(r, -24);
        let covered = 0;
        for (const m of marks) {
          if (m.x >= grown.x && m.x <= grown.x + grown.w && m.y >= grown.y && m.y <= grown.y + grown.h) covered += 1;
        }
        const away = Math.hypot(x + w / 2 - from.x, y + h / 2 - from.y);
        if (!best || covered < best.covered || (covered === best.covered && away < best.away)) {
          best = { x, y, w, h, covered, away, resized };
        }
      }
    }
    return best;
  };

  if (occupied) return search(comp.rect.w, comp.rect.h, false);

  // Nothing in it to preserve: try it at a fair size, then smaller, until there
  // is a place between its new neighbours that it fits.
  for (const share of [0.22, 0.16, 0.12, 0.09, 0.06, 0.04, 0]) {
    // The last try is the smallest box a compartment can usefully be.
    const least = share ? { w: 160, h: 110 } : { w: 96, h: 70 };
    const w = Math.max(least.w, Math.min(comp.rect.w, area.w * share));
    const h = Math.max(least.h, Math.min(comp.rect.h, area.h * share));
    const spot = search(w, h, w !== comp.rect.w || h !== comp.rect.h);
    if (spot) return spot;
  }
  return null;
}

function union(rects) {
  const x1 = Math.min(...rects.map((r) => r.x));
  const y1 = Math.min(...rects.map((r) => r.y));
  const x2 = Math.max(...rects.map((r) => r.x + r.w));
  const y2 = Math.max(...rects.map((r) => r.y + r.h));
  return { x: x1, y: y1, w: x2 - x1, h: y2 - y1 };
}

function inset(r, d) {
  return { x: r.x + d, y: r.y + d, w: r.w - 2 * d, h: r.h - 2 * d };
}

function contains(outer, inner) {
  return inner.x >= outer.x && inner.y >= outer.y
    && inner.x + inner.w <= outer.x + outer.w && inner.y + inner.h <= outer.y + outer.h;
}

function overlaps(a, b, gap = 0) {
  return !(a.x + a.w + gap <= b.x || b.x + b.w + gap <= a.x
    || a.y + a.h + gap <= b.y || b.y + b.h + gap <= a.y);
}
