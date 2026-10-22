/**
 * The editable map.
 *
 * This module keeps three things apart:
 *
 *   Model        everything the biology makes available. `Model` and
 *                `CommunityModel` in src/core. Every reaction that exists,
 *                whether or not anybody wants to look at it.
 *   EditableMap  what is currently drawn. A subset of the model, plus the
 *                geometry that subset is drawn with. Owns cells, compartments,
 *                boundaries, nodes, reactions, segments and the canvas.
 *   Simulation   the LP and its answer. `FluxProblem` and the solution held by
 *                the store. It reads the model, never the map.
 *
 * The point of the split is that a map is allowed to be incomplete. It may
 * hold every reaction in the community, a handful of them, or none at all, and
 * none of those is a degenerate case: an empty map is the honest starting point
 * for building one by hand, and it still draws the cells, their membranes and
 * the medium, because those come from the model's structure rather than from its
 * reactions.
 *
 * Nothing here solves, rebuilds or re-runs the automatic layout. The map is
 * geometry and membership. The numbers live elsewhere.
 *
 * Shape compatibility
 * An EditableMap exposes `width`, `height`, `environment`, `cells`, `medium`,
 * `nodes`, `reactions` and `segments` exactly as the object `layoutCommunity()`
 * returns, so the renderer and the drag helpers in layout/drag.js accept one
 * without knowing it is a different class. The extra parts (`boundaries`,
 * `canvas` and the membership methods) are additions, not replacements.
 */

import { layoutCommunity, SIZES } from '../layout/layout.js';
import { moveLayoutNode, isDraggableNode } from '../layout/drag.js';
import { placeReaction, localMetaboliteBase } from '../layout/place-reaction.js';
import {
  canMoveReaction, moveReaction, moveBranchControl, reactionControlPoints,
  canRotateReaction, rotateReaction, setReactionAngle, reactionAngle,
  canLinearizeReaction, setReactionLinear, straightenLinearReactions, straightenReaction, replaceLinearEnd,
  reactionSides, swapReactionMetabolites, flipReaction, mirrorReaction,
  reactionLineEnds, setReactionLineEnd, alignReaction, tidyReaction, keepStraightThrough,
} from '../layout/reaction-edit.js';
import {
  descendantIds, editCompartmentShape, editGeometryRect, geometryTarget, reactionInSubtree, translateWholeMap,
} from './geometry-edit.js';
import {
  alignSelection, distributeSelection, dragSelection, selectableInRect, translateSelection, textCentre,
  spaceSelectionEvenly, rotateSelection, pointSelection,
} from './multi-selection.js';
import { isSecondaryNode, metaboliteLabelAngle } from './metabolite-label.js';
import { normalizeReactionLabelOffset } from './reaction-label.js';
import { closestPerimeterPoint, compartmentCorners } from '../layout/geometry.js';
import {
  membraneStyle, compartmentName, cellCompartmentName, cellCompartmentVisualId,
} from '../core/compartments.js';
import { SHARED_COMPARTMENT } from '../core/community.js';
import { normalizeHexColor } from '../core/colors.js';
import { TEXT_SIZE, clampTextSize, normalizeTextFont } from '../core/text-style.js';
import { BOUNDARY_KIND, boundaryKey, ENVIRONMENT_KEY } from './boundaries.js';

/** Node kinds that represent a metabolite rather than map furniture. */
const METABOLITE_KINDS = new Set(['metabolite', 'medium', 'currency']);

/**
 * The least distance between two neighbours on a line made by Align vertical /
 * horizontal: a reaction marker and a metabolite either side of it, with room
 * for their labels. Two metabolites with a marker between are twice this apart.
 */
const LINE_ALIGN_GAP = 40;

export class EditableMap {
  /**
   * Prefer the named constructors. This takes the pieces as they are and does
   * not copy them, which is what `fromLayout` wants.
   */
  constructor(spec = {}) {
    /** The drawing surface. `width`/`height` read through to it. */
    this.canvas = {
      width: spec.canvas?.width ?? spec.width ?? 0,
      height: spec.canvas?.height ?? spec.height ?? 0,
    };
    this.environment = spec.environment ?? {
      rect: { x: 0, y: 0, w: this.canvas.width, h: this.canvas.height },
      id: SHARED_COMPARTMENT,
      label: compartmentName(SHARED_COMPARTMENT, { s: 'Shared medium' }),
    };
    this.cells = spec.cells ?? [];
    nameCellCompartments(this.cells);
    this.medium = spec.medium ?? { nodes: [], exchanges: [] };
    this.nodes = asMap(spec.nodes);
    migrateReactionLocalOwnership(this.nodes);
    normalizeVisualRoles(this.nodes);
    normalizeMetaboliteLabelAngles(this.nodes);
    stripSpeciesFromLabels(this.nodes);
    this.reactions = asMap(spec.reactions);
    normalizeReactionLabelOffsets(this.reactions);
    this.segments = spec.segments ?? [];
    this.texts = asMap(spec.texts);
    this.labelSettings = spec.labelSettings ? { ...spec.labelSettings } : null;
    // The colours metabolite circles take unless one sets its own.
    this.nodeStyle = spec.nodeStyle ? { ...spec.nodeStyle } : null;
    // How the secondary metabolites are drawn: 'quiet' (smaller and lighter, so
    // the pathways read first), or null for the full size every map has by default.
    this.cofactorStyle = normalizeCofactorStyle(spec.cofactorStyle);
    // Derived from the cells and the environment, never stored separately: two
    // copies of the same fact would eventually disagree.
    this.boundaries = deriveBoundaries(this);
    this.boundaryIndex = new Map(this.boundaries.map((b) => [b.id, b]));
  }

  get width() { return this.canvas.width; }
  get height() { return this.canvas.height; }

  // Constructors

  /**
   * Adopt a layout produced by `layoutCommunity()`.
   *
   * @param {object} layout
   * @param {object} [opts]
   * @param {Iterable<string>} [opts.reactions] keep only these reaction ids
   */
  static fromLayout(layout, opts = {}) {
    const map = new EditableMap({
      canvas: { width: layout.width, height: layout.height },
      environment: layout.environment,
      cells: layout.cells,
      medium: layout.medium,
      nodes: layout.nodes,
      reactions: layout.reactions,
      segments: layout.segments,
      texts: layout.texts ?? new Map(),
    });
    return opts.reactions === undefined ? map : map.subsetByReactions(opts.reactions);
  }

  /** Lay a community out and adopt the result. */
  static fromCommunity(community, opts = {}) {
    return EditableMap.fromLayout(layoutCommunity(community, opts), opts);
  }

  /**
   * The shell of a community and nothing else: every cell, its outer membrane,
   * the compartments nested inside it with their own membranes, and the shared
   * medium around them. No reaction markers, no metabolite nodes, no segments.
   *
   * The compartment packing is the same code the full layout uses, so the shell
   * a user starts from is the shell their finished map will have.
   */
  static emptyFromCommunity(community, opts = {}) {
    return EditableMap.fromLayout(layoutCommunity(community, { ...opts, contents: 'none' }));
  }

  /**
   * Truly blank drawing surface: no environment decoration, cells,
   * compartments, metabolites, reactions or segments.
   *
   * The biological community continues to exist outside this map and can be
   * used to add structure/reactions manually.
   */
  static blank(opts = {}) {
    const width = Math.max(320, Number(opts.width) || 1200);
    const height = Math.max(240, Number(opts.height) || 800);
    return new EditableMap({
      canvas: { width, height },
      environment: {
        rect: { x: 0, y: 0, w: width, h: height },
        id: SHARED_COMPARTMENT,
        label: '',
        hidden: true,
      },
      cells: [],
      medium: { nodes: [], exchanges: [] },
      nodes: new Map(),
      reactions: new Map(),
      segments: [],
      texts: new Map(),
    });
  }

  // Membership

  hasReaction(id) { return this.reactions.has(id); }
  hasNode(id) { return this.nodes.has(id); }
  hasBoundary(id) { return this.boundaryIndex.has(id); }

  reactionIds() { return [...this.reactions.keys()]; }
  nodeIds() { return [...this.nodes.keys()]; }

  boundary(id) { return this.boundaryIndex.get(id) ?? null; }

  /** Derive the boundaries again after cells or compartments have changed. */
  refreshBoundaries() {
    this.boundaries = deriveBoundaries(this);
    this.boundaryIndex = new Map(this.boundaries.map((b) => [b.id, b]));
    return this.boundaries;
  }

  geometryTarget(cellId, compartmentId) {
    return geometryTarget(this, cellId, compartmentId);
  }

  editGeometryRect(cellId, compartmentId, rect) {
    const lines = straightLines(this);
    const result = editGeometryRect(this, cellId, compartmentId, rect);
    // A compartment that moves takes one end of a straight reaction with it.
    if (result) restraighten(this, lines);
    return result;
  }

  /** Corner softness and ovalness of one compartment (see compartmentShape). */
  editCompartmentShape(cellId, compartmentId, shape) {
    const lines = straightLines(this);
    const result = editCompartmentShape(this, cellId, compartmentId, shape);
    if (result) restraighten(this, lines);
    return result;
  }
  boundariesOf(cellId) { return this.boundaries.filter((b) => b.cellId === cellId); }

  /** Segments belonging to one reaction. */
  segmentsOf(reactionId) {
    return this.segments.filter((seg) => seg.reactionId === reactionId);
  }

  /** Segments whose metabolite end is this node. */
  segmentsAt(nodeId) {
    return this.segments.filter((seg) => seg.nodeId === nodeId);
  }

  /** Nothing drawn inside the shell: no reactions and no metabolites. */
  isEmpty() {
    if (this.reactions.size > 0) return false;
    for (const node of this.nodes.values()) {
      if (METABOLITE_KINDS.has(node.kind)) return false;
    }
    return this.segments.length === 0;
  }

  /** A count of everything, for status lines and tests. */
  stats() {
    let metabolites = 0;
    let currency = 0;
    let medium = 0;
    for (const node of this.nodes.values()) {
      if (node.kind === 'metabolite') metabolites += 1;
      else if (node.kind === 'currency') currency += 1;
      else if (node.kind === 'medium') medium += 1;
    }
    return {
      cells: this.cells.length,
      compartments: this.cells.reduce((sum, cell) => sum + cell.compartments.length, 0),
      boundaries: this.boundaries.length,
      reactions: this.reactions.size,
      nodes: this.nodes.size,
      metabolites,
      currency,
      medium,
      texts: this.texts.size,
      segments: this.segments.length,
    };
  }

  // Edits

  /**
   * Draw one species/cell boundary on the map.
   *
   * The cell boundary is also the outer/root extracellular compartment of that
   * species: its outside is the shared medium and its inside is the species'
   * extracellular space. This is map geometry only; the biological model is
   * never modified.
   */
  addCell(spec = {}) {
    const speciesId = String(spec.speciesId ?? '').trim();
    const outerCompartmentId = String(spec.outerCompartmentId ?? 'e').trim();
    if (!speciesId || !outerCompartmentId) return null;
    if (this.cells.some((cell) => cell.id === speciesId)) return null;

    const requested = normalizeRect(spec.rect);
    const rect = requested
      ? clampCellRectToCanvas(this, requested)
      : findFreeCellRect(this);
    if (!rect) return null;

    const outerVisualId = cellCompartmentVisualId(outerCompartmentId, { [outerCompartmentId]: spec.outerLabel });
    const compartment = {
      id: outerCompartmentId,
      rect: { ...rect },
      depth: 0,
      parentId: null,
      label: cellCompartmentName(outerCompartmentId, { [outerCompartmentId]: spec.outerLabel }),
      membrane: spec.outerMembrane || membraneStyle(outerVisualId),
      visualId: outerVisualId,
    };
    const cell = {
      id: speciesId,
      name: spec.cellName || speciesId,
      organism: spec.organism || '',
      note: spec.note || '',
      color: spec.color || '#64748b',
      abundance: Number.isFinite(Number(spec.abundance)) ? Number(spec.abundance) : 1,
      rect: { ...rect },
      compartments: [compartment],
      outerCompartment: outerCompartmentId,
      manuallyDrawn: true,
    };

    this.cells.push(cell);
    this.refreshBoundaries();
    return { cell, compartment, boundary: this.boundary(boundaryKey(speciesId, outerCompartmentId)) };
  }

  /**
   * Add one biological compartment shell manually.
   *
   * If the species has no drawn cell yet, adding its first root compartment
   * creates the cell shell as well. Children can only be added after their
   * biological parent is already drawn.
   */
  addCompartment(spec = {}) {
    const speciesId = String(spec.speciesId ?? '').trim();
    const compartmentId = String(spec.compartmentId ?? '').trim();
    if (!speciesId || !compartmentId) return null;

    let cell = this.cells.find((c) => c.id === speciesId);
    if (cell?.compartments?.some((c) => c.id === compartmentId)) return null;

    const parentId = spec.parentId ?? null;
    if (parentId && !cell?.compartments?.some((c) => c.id === parentId)) return null;

    if (!cell) {
      const rect = findFreeCellRect(this, spec.rect);
      cell = {
        id: speciesId,
        name: spec.cellName || speciesId,
        organism: spec.organism || '',
        note: spec.note || '',
        color: spec.color || '#64748b',
        abundance: Number.isFinite(Number(spec.abundance)) ? Number(spec.abundance) : 1,
        rect: { ...rect },
        compartments: [],
        outerCompartment: compartmentId,
      };
      this.cells.push(cell);
    }

    const depth = parentId
      ? (cell.compartments.find((c) => c.id === parentId)?.depth ?? -1) + 1
      : 0;
    const rect = parentId
      ? findFreeChildRect(cell, parentId, spec.rect)
      : (cell.compartments.length === 0 ? { ...cell.rect } : findFreeRootRect(cell, spec.rect));
    if (!rect) return null;

    const visualId = cellCompartmentVisualId(compartmentId, { [compartmentId]: spec.label });
    const compartment = {
      id: compartmentId,
      rect,
      depth,
      parentId,
      label: cellCompartmentName(compartmentId, { [compartmentId]: spec.label }),
      membrane: spec.membrane || membraneStyle(visualId),
      visualId,
    };
    cell.compartments.push(compartment);

    if (parentId === null) {
      if (!cell.outerCompartment) cell.outerCompartment = compartmentId;
      cell.rect = unionRects(cell.compartments.filter((c) => c.parentId === null).map((c) => c.rect))
        ?? { ...compartment.rect };
    }

    this.refreshBoundaries();
    return { cell, compartment };
  }

  /**
   * Resize the white drawing canvas without relayout, optionally moving
   * everything on it. Width/height cannot be reduced past the content.
   *
   * Growing from the right or the bottom only changes the paper. Growing from
   * the top or the left moves the edge the drawing is measured from, so the
   * drawing has to move with it or it would slide off the side the user is
   * pulling. The caller passes the shift it wants applied; the clamped size
   * comes back so the caller can see what it actually got.
   *
   * @param {{shiftX?: number, shiftY?: number}} [move] translation to apply first
   */
  setCanvasSize(width, height, move = {}) {
    const shiftX = Number(move.shiftX) || 0;
    const shiftY = Number(move.shiftY) || 0;
    if (shiftX || shiftY) translateWholeMap(this, shiftX, shiftY);

    const min = this.minimumCanvasSize();
    const w = Math.max(min.width, Number(width) || this.canvas.width);
    const h = Math.max(min.height, Number(height) || this.canvas.height);
    this.canvas.width = w;
    this.canvas.height = h;
    if (this.environment?.hidden) {
      this.environment.rect = { x: 0, y: 0, w, h };
    }
    return { width: w, height: h };
  }

  /** The smallest canvas that holds everything drawn plus `padding`, never under 320 x 240. */
  minimumCanvasSize(padding = 40) {
    let maxX = 320 - padding;
    let maxY = 240 - padding;
    for (const cell of this.cells) {
      maxX = Math.max(maxX, cell.rect.x + cell.rect.w);
      maxY = Math.max(maxY, cell.rect.y + cell.rect.h);
    }
    for (const node of this.nodes.values()) {
      maxX = Math.max(maxX, node.x + Number(node.r ?? 0));
      maxY = Math.max(maxY, node.y + Number(node.r ?? 0));
    }
    for (const rxn of this.reactions.values()) {
      maxX = Math.max(maxX, rxn.x + 25);
      maxY = Math.max(maxY, rxn.y + 25);
    }
    for (const item of this.texts.values()) {
      maxX = Math.max(maxX, item.x + 20);
      maxY = Math.max(maxY, item.y + 20);
    }
    return {
      width: Math.max(320, Math.ceil(maxX + padding)),
      height: Math.max(240, Math.ceil(maxY + padding)),
    };
  }

  /**
   * Add one biological reaction to this map without rebuilding or solving.
   * Every metabolite is created as a reaction-local visual instance. Existing
   * copies are never re-used automatically. The user may merge copies by drag.
   */
  addReaction(community, reactionId, opts = {}) {
    return placeReaction(this, community, reactionId, opts);
  }

  /**
   * Remove one reaction from the drawing only.
   *
   * The biological model and the LP are untouched. Reaction segments disappear,
   * reaction-specific currency nodes disappear, and any primary/medium metabolite
   * node left with no surviving segment is pruned from the map.
   *
   * @returns {{reaction:object,nodes:string[]}|null}
   */
  removeReaction(reactionId) {
    const reaction = this.reactions.get(reactionId);
    if (!reaction) return null;

    this.reactions.delete(reactionId);

    // Only nodes touched by the removed reaction are candidates for pruning.
    // This protects any unrelated hand-placed/orphan node that may already be
    // present on an edited map.
    const removedSegments = this.segments.filter((seg) => seg.reactionId === reactionId);
    const candidates = new Set(removedSegments.map((seg) => seg.nodeId).filter(Boolean));
    this.segments = this.segments.filter((seg) => seg.reactionId !== reactionId);

    const referenced = new Set();
    for (const seg of this.segments) {
      if (seg.nodeId) referenced.add(seg.nodeId);
    }

    const removedNodes = [];
    for (const [id, node] of this.nodes) {
      const primaryPlacedForRemoved = node.reactionLocalFor === reactionId;
      const satelliteOwnedByRemoved = node.reactionId === reactionId;
      const ownedByRemoved = primaryPlacedForRemoved || satelliteOwnedByRemoved;
      const reactionOwned = ownedByRemoved && !referenced.has(id);
      const becameOrphan = candidates.has(id)
        && METABOLITE_KINDS.has(node.kind)
        && !node.manualDuplicate
        && !node.sharedInstance
        && !referenced.has(id);
      if (reactionOwned || becameOrphan) {
        this.nodes.delete(id);
        removedNodes.push(id);
      } else if (ownedByRemoved && referenced.has(id)) {
        // The node has become genuinely shared through an explicit merge.
        // Clear stale placement ownership so re-adding the removed reaction gets
        // a fresh visual instance instead of accidentally reusing this node.
        if (primaryPlacedForRemoved) delete node.reactionLocalFor;
        if (satelliteOwnedByRemoved) delete node.reactionId;
        node.reactionLocal = false;
        node.sharedInstance = true;
      }
    }

    if (this.medium) {
      this.medium.nodes = (this.medium.nodes ?? []).filter((id) => this.nodes.has(id));
      this.medium.exchanges = (this.medium.exchanges ?? [])
        .filter((id) => id !== reactionId && this.reactions.has(id));
    }

    return { reaction, nodes: removedNodes };
  }

  /**
   * Move a metabolite node. Straight through to moveLayoutNode (layout/drag.js),
   * the one implementation of dragging: clamping to the compartment and
   * dragging the attached segment endpoints along both happen there.
   *
   * @returns {object|null} the node that moved, or null if it is not movable
   */
  moveNode(nodeId, x, y) {
    const node = moveLayoutNode(this, nodeId, x, y);
    // A straight reaction has no place of its own: it goes where its two
    // metabolites go.
    if (node) straightenLinearReactions(this, [nodeId]);
    return node;
  }

  /**
   * Size a metabolite by radius, for callers that think in radii. Coxeter has
   * only two metabolite sizes, so a requested radius snaps to the nearest
   * visual role.
   */
  setNodeRadius(nodeId, radius) {
    const node = this.nodes.get(nodeId);
    if (!node || !['metabolite', 'medium'].includes(node.kind)) return null;
    const value = Number(radius);
    const primary = Number.isFinite(value)
      ? Math.abs(value - SIZES.primaryRadius) <= Math.abs(value - SIZES.currencyRadius)
      : node.primary !== false;
    return this.setNodePrimary(nodeId, primary);
  }

  /**
   * All visual instances representing one biological metabolite.
   * Multiple nodes may deliberately share the same mergedId.
   */
  nodesForMetabolite(mergedId) {
    if (!mergedId) return [];
    return [...this.nodes.values()].filter((node) =>
      ['metabolite', 'medium'].includes(node.kind) && node.mergedId === mergedId
    );
  }

  /**
   * Find another visual instance of the same biological metabolite whose circle
   * overlaps the dragged node. Different metabolites can overlap without being
   * merged.
   */
  overlappingMetaboliteTarget(nodeId) {
    const source = this.nodes.get(nodeId);
    if (!source || !METABOLITE_KINDS.has(source.kind) || !source.mergedId) return null;

    let best = null;
    for (const target of this.nodes.values()) {
      if (target.id === source.id || target.hidden) continue;
      if (!METABOLITE_KINDS.has(target.kind) || target.mergedId !== source.mergedId) continue;
      const distance = Math.hypot(target.x - source.x, target.y - source.y);
      const threshold = Math.max(8, Number(source.r ?? 0) + Number(target.r ?? 0));
      if (distance > threshold) continue;
      if (!best || distance < best.distance) best = { target, distance };
    }
    return best?.target ?? null;
  }

  /**
   * Merge two visual instances of the same biological metabolite.
   *
   * All branches that ended at the source are rewired to the target, then the
   * source visual node is removed. The biological model is never modified.
   */
  mergeMetaboliteNodes(sourceId, targetId) {
    const source = this.nodes.get(sourceId);
    const target = this.nodes.get(targetId);
    if (!source || !target || sourceId === targetId) return null;
    if (!METABOLITE_KINDS.has(source.kind) || !METABOLITE_KINDS.has(target.kind)) return null;
    if (!source.mergedId || source.mergedId !== target.mergedId) return null;

    const dx = target.x - source.x;
    const dy = target.y - source.y;
    let rewired = 0;
    for (const seg of this.segments) {
      if (seg.nodeId !== sourceId) continue;
      seg.nodeId = targetId;

      let key = null;
      if (seg.side === 'substrate') key = 'from';
      else if (seg.side === 'product') key = 'to';
      else {
        const fromDistance = Math.hypot(seg.from.x - source.x, seg.from.y - source.y);
        const toDistance = Math.hypot(seg.to.x - source.x, seg.to.y - source.y);
        key = fromDistance <= toDistance ? 'from' : 'to';
      }
      seg[key] = { x: target.x, y: target.y };

      // b1 is the metabolite-side Escher control point.
      if (seg.b1) seg.b1 = { x: seg.b1.x + dx, y: seg.b1.y + dy };
      rewired += 1;
    }

    this.nodes.delete(sourceId);
    if (this.medium) {
      this.medium.nodes = (this.medium.nodes ?? []).filter((id) => id !== sourceId);
      if (target.kind === 'medium' && !this.medium.nodes.includes(targetId)) {
        this.medium.nodes.push(targetId);
      }
    }
    target.hidden = false;
    target.sharedInstance = true;
    replaceLinearEnd(this, sourceId, targetId);
    straightenLinearReactions(this, [targetId]);

    return { source, target, rewired };
  }

  /**
   * Place a free-standing copy of one cellular metabolite (merged id) at
   * (opts.x, opts.y), drawn as primary and joined to no reaction. Returns the
   * node, or null for an unknown or shared-medium metabolite or a bad position.
   */
  addMetabolite(community, mergedId, opts={}) {
    const metabolite=community?.merged?.metabolites?.get?.(mergedId),owner=community?.metaboliteOwner?.get?.(mergedId);
    if(!metabolite||!owner?.speciesId)return null;
    const base=`met:${mergedId}:manual`;let n=1,id=`${base}:${n}`;while(this.nodes.has(id))id=`${base}:${++n}`;
    const node={id,kind:'metabolite',mergedId,localId:owner.originalId??metabolite.id,base:localMetaboliteBase(community,mergedId,metabolite),name:metabolite.name,formula:metabolite.formula,compartment:owner.compartment,visualCompartment:owner.compartment,cellId:owner.speciesId,x:Number(opts.x),y:Number(opts.y),r:SIZES.primaryRadius,primary:true,manualDuplicate:true,reactionLocal:false,sharedInstance:false};
    if(!Number.isFinite(node.x)||!Number.isFinite(node.y))return null;this.nodes.set(id,node);return node;
  }

  /** Merge a dragged metabolite if it currently overlaps an identical copy. */
  mergeOverlappingMetabolite(nodeId) {
    const target = this.overlappingMetaboliteTarget(nodeId);
    return target ? this.mergeMetaboliteNodes(nodeId, target.id) : null;
  }

  /**
   * Duplicate one metabolite graphically. The biological Model is untouched.
   *
   * The copy receives its own node id, position, radius and visibility while
   * retaining the same mergedId/localId identity used for reaction discovery.
   */
  duplicateMetaboliteNode(nodeId, opts = {}) {
    const source = this.nodes.get(nodeId);
    if (!source || !['metabolite', 'medium'].includes(source.kind)) return null;

    const id = opts.id ?? nextDuplicateNodeId(this.nodes, source);
    if (this.nodes.has(id)) return null;

    const offset = Number.isFinite(Number(opts.offset)) ? Number(opts.offset) : 42;
    const requestedX = Number.isFinite(Number(opts.x)) ? Number(opts.x) : source.x + offset;
    const requestedY = Number.isFinite(Number(opts.y)) ? Number(opts.y) : source.y + offset;

    const copy = {
      ...source,
      id,
      x: source.x,
      y: source.y,
      hidden: false,
      manualDuplicate: true,
      duplicateOf: source.duplicateOf ?? source.id,
    };
    delete copy.reactionId;
    delete copy.reactionLocalFor;
    copy.reactionLocal = false;
    copy.sharedInstance = false;

    this.nodes.set(id, copy);
    moveLayoutNode(this, id, requestedX, requestedY);

    if (copy.kind === 'medium' && this.medium) {
      if (!Array.isArray(this.medium.nodes)) this.medium.nodes = [];
      if (!this.medium.nodes.includes(id)) this.medium.nodes.push(id);
    }
    return copy;
  }

  /**
   * Mark one visual metabolite instance as primary or secondary.
   *
   * This is map-only metadata. The biological metabolite is unchanged.
   * The role also fixes its radius: primary = 13 px, secondary = 7 px.
   */
  setNodePrimary(nodeId, primary = true, opts = {}) {
    const node = this.nodes.get(nodeId);
    if (!node || !['metabolite', 'medium'].includes(node.kind)) return null;

    node.primary = Boolean(primary);
    node.visualRole = node.primary ? 'primary' : 'secondary';
    node.r = node.primary ? SIZES.primaryRadius : SIZES.currencyRadius;
    return node;
  }

  setNodeVisualRole(nodeId, role, opts = {}) {
    if (!['primary', 'secondary'].includes(role)) return null;
    return this.setNodePrimary(nodeId, role === 'primary', opts);
  }

  /**
   * Map-only colours of a cell. `fill` is the colour of the cell body, which is
   * its outer compartment, so it is stored there and the two cannot disagree;
   * `stroke` draws the cell boundary. A colour given as null goes back to the
   * theme; one left out, or not a #rrggbb colour, is left as it is.
   */
  setCellColors(cellId, { fill, stroke } = {}) {
    const cell = this.cells.find((item) => item.id === cellId);
    if (!cell) return null;
    const body = cell.compartments.find((item) => item.id === cell.outerCompartment)
      ?? cell.compartments.find((item) => item.parentId === null);
    if (body) applyMapColor(body, 'fill', fill);
    applyMapColor(cell, 'stroke', stroke);
    return cell;
  }

  /** Map-only colours of one compartment: `fill` for its space, `stroke` for its membrane. */
  setCompartmentColors(cellId, compartmentId, { fill, stroke } = {}) {
    const cell = this.cells.find((item) => item.id === cellId);
    const compartment = cell?.compartments.find((item) => item.id === compartmentId);
    if (!compartment) return null;
    applyMapColor(compartment, 'fill', fill);
    applyMapColor(compartment, 'stroke', stroke);
    return compartment;
  }

  /**
   * The colours every metabolite circle is drawn with unless it has its own:
   * `{ fill, stroke }`, a colour each, null to give one back to the theme.
   */
  setNodeStyle({ fill, stroke } = {}) {
    const style = { ...(this.nodeStyle ?? {}) };
    applyMapColor(style, 'fill', fill);
    applyMapColor(style, 'stroke', stroke);
    this.nodeStyle = Object.keys(style).length ? style : null;
    return this.nodeStyle;
  }

  /** How secondary metabolites are drawn: 'quiet', or null for full size. Returns the style kept. */
  setCofactorStyle(style) {
    this.cofactorStyle = normalizeCofactorStyle(style);
    return this.cofactorStyle;
  }

  /** Colour some metabolite circles: `{ fill, stroke }`, null to return one to the map's. */
  setNodeColors(nodeIds, { fill, stroke } = {}) {
    const changed = [];
    for (const id of nodeIds) {
      const node = this.nodes.get(id);
      if (!node || !['metabolite', 'medium', 'currency'].includes(node.kind)) continue;
      applyMapColor(node, 'fill', fill);
      applyMapColor(node, 'stroke', stroke);
      changed.push(node);
    }
    return changed;
  }

  /**
   * The colour some reactions are drawn in when lines are not coloured by
   * flux, over their pathway's; null gives them back their pathway's.
   */
  setReactionColor(reactionIds, color) {
    const changed = [];
    for (const id of reactionIds) {
      const reaction = this.reactions.get(id);
      if (!reaction) continue;
      applyMapColor(reaction, 'color', color);
      changed.push(reaction);
    }
    return changed;
  }

  /**
   * Pin a compartment's (or the environment's) label to the point of its
   * outline nearest (x, y). Stored as fractions of its rect, so it follows
   * moves and resizes.
   */
  setCompartmentLabelPosition(cellId, compartmentId, x, y) {
    const target = this.geometryTarget(cellId, compartmentId);
    if (!target || !Number.isFinite(x) || !Number.isFinite(y)) {
      return null;
    }
    const compartment = target.compartment;
    const rect = compartment.rect;
    const radius = target.environment ? 26 : compartmentCorners(compartment);
    const point = closestPerimeterPoint(rect, { x, y }, radius);
    compartment.labelPosition = {
      x: (point.x - rect.x) / rect.w,
      y: (point.y - rect.y) / rect.h,
    };
    return compartment.labelPosition;
  }

  /** Change one label's orbit without moving its circle or attached branches. */
  setMetaboliteLabelAngle(nodeId, angle) {
    const node = this.nodes.get(nodeId);
    if (!node || !METABOLITE_KINDS.has(node.kind) || !Number.isFinite(angle)) return null;
    node.labelAngle = metaboliteLabelAngle({ labelAngle: angle });
    return node;
  }

  /**
   * Write a reaction's name at `offset` from its marker (map units) instead of
   * where the map would put it. Nothing else of the reaction changes.
   */
  setReactionLabelOffset(reactionId, offset) {
    const rxn = this.reactions.get(reactionId);
    const value = normalizeReactionLabelOffset(offset);
    if (!rxn || !value) return null;
    rxn.labelOffset = value;
    return rxn;
  }

  /** Put a moved reaction name back in its default place. Null when it was never moved. */
  resetReactionLabel(reactionId) {
    const rxn = this.reactions.get(reactionId);
    if (!rxn || !Object.hasOwn(rxn, 'labelOffset')) return null;
    delete rxn.labelOffset;
    return rxn;
  }

  /** Hide/show one metabolite graphically. Its model membership is untouched. */
  setNodeHidden(nodeId, hidden = true) {
    const node = this.nodes.get(nodeId);
    if (!node || !['metabolite', 'medium'].includes(node.kind)) return null;
    node.hidden = Boolean(hidden);
    return node;
  }

  hiddenNodeCount() {
    let count = 0;
    for (const node of this.nodes.values()) {
      if (node.hidden && ['metabolite', 'medium'].includes(node.kind)) count += 1;
    }
    return count;
  }

  showAllHiddenNodes() {
    const changed = [];
    for (const node of this.nodes.values()) {
      if (!node.hidden || !['metabolite', 'medium'].includes(node.kind)) continue;
      node.hidden = false;
      changed.push(node.id);
    }
    return changed;
  }

  /** Is this node one the user is allowed to move? */
  canMoveNode(nodeId) {
    return isDraggableNode(this.nodes.get(nodeId));
  }

  /** Reaction centre / transporter movement. */
  canMoveReaction(reactionId) {
    return canMoveReaction(this, reactionId);
  }

  moveReaction(reactionId, x, y) {
    return moveReaction(this, reactionId, x, y);
  }

  /** Turning the backbone of an internal reaction about its centre. */
  canRotateReaction(reactionId) {
    return canRotateReaction(this, reactionId);
  }

  reactionAngle(reactionId) {
    return reactionAngle(this, reactionId);
  }

  rotateReaction(reactionId, angle) {
    return rotateReaction(this, reactionId, angle);
  }

  setReactionAngle(reactionId, angle) {
    return setReactionAngle(this, reactionId, angle);
  }

  /**
   * Keep a reaction straight between two of its metabolites, so that it follows
   * them when they are moved. `false` gives it back its own position.
   */
  canLinearizeReaction(reactionId) {
    return canLinearizeReaction(this, reactionId);
  }

  setReactionLinear(reactionId, on = true, ends = {}) {
    return setReactionLinear(this, reactionId, on, ends);
  }

  /** The two metabolites a reaction's line runs through, or would if made straight. */
  reactionLineEnds(reactionId) {
    return reactionLineEnds(this, reactionId);
  }

  setReactionLineEnd(reactionId, nodeId) {
    return setReactionLineEnd(this, reactionId, nodeId);
  }

  /**
   * Lay a reaction out along a compass direction ('N', 'NE', ... 'NW'),
   * substrate to product, through its marker, and keep it straight.
   */
  alignReaction(reactionId, direction, ends = {}) {
    return alignReaction(this, reactionId, direction, ends);
  }

  /** Put a reaction's cofactors and curves back in their base shape. */
  tidyReaction(reactionId) {
    return tidyReaction(this, reactionId);
  }

  /** The metabolites drawn on each side of a reaction. Map-only, like the rest. */
  reactionSides(reactionId) {
    return reactionSides(this, reactionId);
  }

  swapReactionMetabolites(reactionId, nodeIdA, nodeIdB) {
    return swapReactionMetabolites(this, reactionId, nodeIdA, nodeIdB);
  }

  /** Exchange the substrate side and the product side of a reaction's drawing. */
  flipReaction(reactionId) {
    return flipReaction(this, reactionId);
  }

  /** Mirror a reaction's drawing in its own backbone. */
  mirrorReaction(reactionId) {
    return mirrorReaction(this, reactionId);
  }

  /** Escher-style cubic Bézier control handles. */
  moveBranchControl(reactionId, nodeId, handle, x, y) {
    return moveBranchControl(this, reactionId, nodeId, handle, x, y);
  }

  reactionControlPoints(reactionId) {
    return reactionControlPoints(this, reactionId);
  }

  selectableInRect(rect, options = {}) {
    return selectableInRect(this, rect, options);
  }

  translateSelection(selection, dx, dy, opts = {}) {
    return translateSelection(this, selection, dx, dy, opts);
  }

  /** Move a selection as the map's drag does, its compartments with it. */
  dragSelection(selection, dx, dy) {
    return dragSelection(this, selection, dx, dy);
  }

  alignSelection(selection, mode) {
    return alignSelection(this, selection, mode);
  }

  distributeSelection(selection, axis) {
    return distributeSelection(this, selection, axis);
  }

  /** Space a selection evenly along its own line, in any direction. */
  spaceSelectionEvenly(selection) {
    return spaceSelectionEvenly(this, selection);
  }

  /** Turn a selection as one rigid piece (radians, clockwise on screen), or say why not. */
  rotateSelection(selection, angle, pivot = null) {
    return rotateSelection(this, selection, angle, pivot);
  }

  /** Turn a selection so its main line points a compass direction ('N' ... 'NW'), the way it runs. */
  pointSelection(selection, direction) {
    return pointSelection(this, selection, direction);
  }

  /**
   * Escher's Align vertical / Align horizontal for a selection with reactions
   * in it: the reactions come with their metabolites, and everything ends on
   * one line.
   *
   * Each selected reaction brings the two metabolites its line runs through.
   * Those, the other selected primary metabolites and the selected reaction
   * markers are set on one vertical (same x) or horizontal (same y) line, at
   * the mean of where the metabolites and markers were, each keeping its place
   * along it, so a pathway keeps its order. Every selected reaction is then kept
   * straight between its two metabolites, and its cofactors put back in order
   * around the new line, rather than left where the line used to be.
   *
   * Transporters stay on their membranes. Secondary metabolites, a reaction's
   * cofactor helpers among them, are placed by their reaction, never stacked on
   * the line: a reaction whose line would end on one brings only its marker.
   * A metabolite stays inside its compartment, so one the line or the spacing
   * would take through a membrane stops at it and is reported in `held`.
   * Reactions drawn to a moved metabolite that were not selected have their
   * curves and cofactors put back in order where they stand, rather than left
   * stretched across to the line. Selected texts are centred on the line.
   *
   * @param {'horizontal'|'vertical'} mode
   * @returns {{nodes: string[], reactions: string[], texts: string[], held: string[]}|null}
   */
  alignSelectionInLine(selection, mode) {
    if (mode !== 'horizontal' && mode !== 'vertical') return null;
    const vertical = mode === 'vertical';
    const reactionIds = [...(selection?.reactions ?? [])]
      .filter((id) => this.reactions.has(id) && !this.reactions.get(id).onMembrane);
    const primary = (id) => this.nodes.has(id) && !isSecondaryNode(this.nodes.get(id));
    const nodeIds = new Set([...(selection?.nodes ?? [])].filter(primary));
    const ends = new Map();
    for (const id of reactionIds) {
      const { from, to } = reactionLineEnds(this, id);
      for (const end of [from, to]) if (end && primary(end)) nodeIds.add(end);
      if (from && to && from !== to && primary(from) && primary(to)) ends.set(id, { from, to });
    }
    const textIds = [...(selection?.texts ?? [])].filter((id) => this.texts.has(id));
    // The line runs through the drawing; a selected title somewhere else joins
    // it rather than pulling it away.
    const anchors = [
      ...[...nodeIds].map((id) => this.nodes.get(id)),
      ...reactionIds.map((id) => this.reactions.get(id)),
    ];
    if (!anchors.length || anchors.length + textIds.length < 2) return null;
    const target = anchors.reduce((sum, p) => sum + (vertical ? p.x : p.y), 0) / anchors.length;

    // The metabolites keep their places along the line, in order; two that
    // would now sit on top of each other (a substrate and product that were
    // side by side, for a vertical line) are spread to leave room for the
    // marker between them, the later ones moving on. Spacing follows where each
    // one really landed, since a membrane can stop it short.
    const along = (p) => (vertical ? p.y : p.x);
    const across = (p) => (vertical ? p.x : p.y);
    const onLine = (at) => (vertical ? { x: target, y: at } : { x: at, y: target });
    const held = [];
    let previous = -Infinity;
    for (const id of lineOrder(this, nodeIds, ends, along)) {
      const at = Math.max(along(this.nodes.get(id)), previous + 2 * LINE_ALIGN_GAP);
      const point = onLine(at);
      this.moveNode(id, point.x, point.y);
      const landed = this.nodes.get(id);
      if (Math.abs(across(landed) - target) > 0.5 || Math.abs(along(landed) - at) > 0.5) held.push(id);
      previous = Math.max(previous, along(landed));
    }

    // A reaction with both its metabolites on the line is kept straight between
    // them, its marker half way, as Escher draws it; one with a single metabolite
    // there brings its marker onto the line where it stands along it.
    for (const id of reactionIds) {
      const pair = ends.get(id);
      if (pair) {
        const a = this.nodes.get(pair.from);
        const b = this.nodes.get(pair.to);
        keepStraightThrough(this, id, pair.from, pair.to, { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 });
      } else {
        const point = onLine(along(this.reactions.get(id)));
        this.moveReaction(id, point.x, point.y);
        tidyReaction(this, id);
      }
    }
    straightenLinearReactions(this, [...nodeIds]);

    // Neighbours not selected keep their place, but not curves stretched to
    // where their metabolite used to be.
    const selected = new Set(reactionIds);
    const neighbours = new Set(this.segments
      .filter((seg) => nodeIds.has(seg.nodeId) && !selected.has(seg.reactionId))
      .map((seg) => seg.reactionId));
    for (const id of neighbours) {
      const rxn = this.reactions.get(id);
      if (rxn && !rxn.onMembrane && !rxn.linear) tidyReaction(this, id);
    }

    for (const id of textIds) {
      const text = this.texts.get(id);
      const centre = textCentre(text);
      const at = vertical ? { x: target, y: centre.y } : { x: centre.x, y: target };
      this.moveText(id, text.x + (at.x - centre.x), text.y + (at.y - centre.y));
    }
    return { nodes: [...nodeIds], reactions: reactionIds, texts: textIds, held };
  }

  /** Whether a selection has a reaction that Align vertical / horizontal can lay on a line. */
  selectionHasLineReactions(selection) {
    return [...(selection?.reactions ?? [])]
      .some((id) => this.reactions.has(id) && !this.reactions.get(id).onMembrane);
  }

  // Free map text

  /**
   * Add a free text whose baseline starts at (x, y). Options: `id`, `fontSize`
   * (px) and `fontFamily`. Returns the annotation, or null for empty text, a
   * bad position or an id already taken.
   */
  addText(text, x, y, opts = {}) {
    const value = String(text ?? '').trim();
    if (!value || !Number.isFinite(x) || !Number.isFinite(y)) return null;
    const id = opts.id ?? nextTextId(this.texts);
    if (this.texts.has(id)) return null;
    const annotation = {
      id,
      text: value,
      x,
      y,
      fontSize: clampTextSize(opts.fontSize ?? TEXT_SIZE.default),
      fontFamily: normalizeTextFont(opts.fontFamily),
    };
    this.texts.set(id, annotation);
    return annotation;
  }

  updateText(id, text) {
    const annotation = this.texts.get(id);
    const value = String(text ?? '').trim();
    if (!annotation || !value) return null;
    annotation.text = value;
    return annotation;
  }

  /**
   * Change how a text is set: `fontSize` in px (8–144) and/or `fontFamily`, a
   * key of TEXT_FONTS. A field left out is left as it is.
   */
  styleText(id, { fontSize, fontFamily } = {}) {
    const annotation = this.texts.get(id);
    if (!annotation) return null;
    if (fontSize !== undefined && fontSize !== null && fontSize !== '') annotation.fontSize = clampTextSize(fontSize);
    if (fontFamily !== undefined && fontFamily !== null) annotation.fontFamily = normalizeTextFont(fontFamily);
    return annotation;
  }

  moveText(id, x, y) {
    const annotation = this.texts.get(id);
    if (!annotation || !Number.isFinite(x) || !Number.isFinite(y)) return null;
    annotation.x = x;
    annotation.y = y;
    return annotation;
  }

  removeText(id) {
    const annotation = this.texts.get(id);
    if (!annotation) return null;
    this.texts.delete(id);
    return annotation;
  }

  /** A primary/medium metabolite may be removed only when no segment uses it. */
  canRemoveNode(nodeId) {
    const node = this.nodes.get(nodeId);
    if (!node || !['metabolite', 'medium'].includes(node.kind)) return false;
    return this.segmentsAt(nodeId).length === 0;
  }

  /**
   * Take a compartment of a cell off the drawing, with everything that cannot
   * be drawn without it: the compartments nested in it, the reactions that run
   * in any of them or cross their membranes, the metabolites drawn there and
   * the texts written inside it.
   * A cell's outermost compartment is the cell, so the whole cell goes. The
   * biological model is untouched, and the compartment can be drawn again.
   *
   * @returns {{cellId:string, compartmentIds:string[], cellRemoved:boolean,
   *   label:string, reactions:string[], nodes:string[], texts:string[]}|null}
   */
  removeCompartment(cellId, compartmentId) {
    const cell = this.cells.find((item) => item.id === cellId);
    const compartment = cell?.compartments.find((item) => item.id === compartmentId);
    if (!compartment) return null;
    const cellRemoved = !compartment.parentId;
    const subtree = descendantIds(cell, compartmentId);

    const reactions = [];
    const nodes = [];
    for (const rxn of [...this.reactions.values()]) {
      if (!reactionInSubtree(rxn, cellId, subtree)) continue;
      const result = this.removeReaction(rxn.id);
      if (!result) continue;
      reactions.push(rxn.id);
      nodes.push(...result.nodes);
    }
    // What is left inside: metabolites drawn on their own, or kept as shared.
    for (const [id, node] of [...this.nodes]) {
      if (node.cellId !== cellId || !subtree.has(node.compartment)) continue;
      this.nodes.delete(id);
      nodes.push(id);
    }
    this.segments = this.segments.filter((seg) => !seg.nodeId || this.nodes.has(seg.nodeId));
    if (this.medium) this.medium.nodes = (this.medium.nodes ?? []).filter((id) => this.nodes.has(id));
    // The texts written inside it are its pathways' names: left behind, they
    // would label an empty space.
    const area = compartment.rect;
    const texts = [];
    for (const [id, text] of [...(this.texts ?? new Map())]) {
      if (!Number.isFinite(text.x) || !Number.isFinite(text.y)) continue;
      if (text.x < area.x || text.x > area.x + area.w || text.y < area.y || text.y > area.y + area.h) continue;
      if (this.removeText(id)) texts.push(id);
    }

    if (cellRemoved) this.cells = this.cells.filter((item) => item !== cell);
    else cell.compartments = cell.compartments.filter((item) => !subtree.has(item.id));
    this.refreshBoundaries();

    return {
      cellId,
      compartmentIds: [...subtree],
      cellRemoved,
      label: cellRemoved ? (cell.name ?? cellId) : (compartment.label ?? compartmentId),
      reactions,
      nodes,
      texts,
    };
  }

  /** Remove one orphan metabolite from the drawing only. */
  removeNode(nodeId) {
    if (!this.canRemoveNode(nodeId)) return null;
    const node = this.nodes.get(nodeId);
    this.nodes.delete(nodeId);
    if (this.medium) {
      this.medium.nodes = (this.medium.nodes ?? []).filter((id) => id !== nodeId);
    }
    return node;
  }

  /**
   * A new map drawing only the listed reactions.
   *
   * Segments follow their reaction. A node is kept when a surviving segment
   * still ends at it (a metabolite nobody reacts with has nothing to say on a
   * map), so the result is always internally consistent, including for the
   * empty selection. The geometry is copied, so the two maps can be edited
   * independently.
   */
  subsetByReactions(reactionIds) {
    const keep = new Set(reactionIds);
    const reactions = new Map();
    for (const [id, rxn] of this.reactions) {
      if (keep.has(id)) reactions.set(id, rxn);
    }
    const segments = this.segments.filter((seg) => reactions.has(seg.reactionId));

    const referenced = new Set();
    for (const seg of segments) {
      if (seg.nodeId) referenced.add(seg.nodeId);
    }
    const nodes = new Map();
    for (const [id, node] of this.nodes) {
      if (referenced.has(id)) nodes.set(id, node);
    }

    const copied = cloneGraphics(reactions, segments);
    return new EditableMap({
      canvas: { ...this.canvas },
      environment: this.environment,
      cells: this.cells,
      medium: {
        nodes: (this.medium?.nodes ?? []).filter((id) => nodes.has(id)),
        exchanges: (this.medium?.exchanges ?? []).filter((id) => reactions.has(id)),
      },
      nodes: new Map([...nodes].map(([id, node]) => [id, { ...node }])),
      reactions: copied.reactions,
      segments: copied.segments,
      texts: new Map([...this.texts].map(([id, item]) => [id, { ...item }])),
    });
  }

  /** A copy with nothing drawn in it, keeping the shell. */
  cleared() {
    return this.subsetByReactions([]);
  }

  // I/O

  /**
   * Serialise the graphical state.
   *
   * Boundaries are left out on purpose: they are derived from the cells and the
   * environment, and storing them would let a file contradict itself.
   *
   * The result is a deep copy, detached from the map. The undo history takes
   * its "before" document ahead of an edit and only serialises it afterwards,
   * so with the live nodes, reactions and segments every edit made in place (a
   * drag, a control point, a resize, a role change) would read back as
   * "nothing changed" and never be recorded, and one undo would skip all of
   * them.
   */
  toJSON() {
    return JSON.parse(JSON.stringify({
      version: 1,
      canvas: { ...this.canvas },
      environment: this.environment,
      cells: this.cells,
      medium: this.medium,
      nodes: [...this.nodes.values()],
      reactions: [...this.reactions.values()],
      segments: this.segments,
      texts: [...this.texts.values()],
      ...(this.labelSettings ? { labelSettings: this.labelSettings } : {}),
      ...(this.nodeStyle ? { nodeStyle: this.nodeStyle } : {}),
      ...(this.cofactorStyle ? { cofactorStyle: this.cofactorStyle } : {}),
    }));
  }

  /**
   * Rebuild a map from `toJSON()` output.
   *
   * JSON has no object identity, so the endpoints a segment shares with its
   * reaction's axis arrive as separate copies. They are re-linked here, which
   * keeps the invariant the geometry relies on: the axis is one object, and
   * moving it moves everything pinned to it.
   */
  static fromJSON(doc) {
    const reactions = new Map((doc.reactions ?? []).map((rxn) => [rxn.id, rxn]));
    const segments = (doc.segments ?? []).map((seg) => ({ ...seg }));
    relinkAxisEndpoints(reactions, segments);
    return new EditableMap({
      canvas: doc.canvas ?? { width: doc.width, height: doc.height },
      environment: doc.environment,
      cells: doc.cells ?? [],
      medium: doc.medium,
      nodes: new Map((doc.nodes ?? []).map((node) => [node.id, node])),
      reactions,
      segments,
      texts: new Map((doc.texts ?? []).map((item) => [item.id, item])),
      labelSettings: doc.labelSettings,
      nodeStyle: doc.nodeStyle,
      cofactorStyle: doc.cofactorStyle,
    });
  }
}

// Helpers

/** The secondary-metabolite styles a map can carry; anything else is the default. */
const COFACTOR_STYLES = new Set(['quiet']);

function normalizeCofactorStyle(style) {
  return COFACTOR_STYLES.has(style) ? style : null;
}

/*
 * Where each straight reaction and its two ends stand, before a geometry edit.
 * Straightening redraws a line's branches from scratch, so after the edit only
 * the lines it pulled out of true are straightened: one end moved and the
 * reaction did not move with it. Straightening every line on the map instead
 * would reset the bends drawn into lines nowhere near the compartment being
 * edited.
 */
function straightLines(map) {
  const out = new Map();
  for (const rxn of map.reactions.values()) {
    if (!rxn.linear) continue;
    const from = map.nodes.get(rxn.linear.from);
    const to = map.nodes.get(rxn.linear.to);
    if (!from || !to) continue;
    out.set(rxn.id, { rxn: { x: rxn.x, y: rxn.y }, from: { x: from.x, y: from.y }, to: { x: to.x, y: to.y } });
  }
  return out;
}

function restraighten(map, lines) {
  const shift = (now, before) => ({ x: now.x - before.x, y: now.y - before.y });
  const same = (a, b) => Math.abs(a.x - b.x) < 1e-9 && Math.abs(a.y - b.y) < 1e-9;
  for (const [id, before] of lines) {
    const rxn = map.reactions.get(id);
    const from = map.nodes.get(rxn?.linear?.from);
    const to = map.nodes.get(rxn?.linear?.to);
    if (!rxn || !from || !to) continue;
    const moved = shift(rxn, before.rxn);
    if (!same(shift(from, before.from), moved) || !same(shift(to, before.to), moved)) {
      straightenReaction(map, id);
    }
  }
}

/**
 * The order metabolites take on a line made by Align vertical / horizontal.
 *
 * When the selected reactions join their metabolites into one unbranched chain
 * (g6p → f6p → fdp), the chain's own order is used, from the end that stands
 * first along the line: projecting a pathway drawn across the line would fold
 * it back on itself wherever two neighbours had swapped sides. Anything else
 * (branches, separate pieces, lone metabolites) keeps the order of where each
 * stands now, as Escher does.
 */
function lineOrder(map, nodeIds, ends, along) {
  const byPlace = [...nodeIds].sort((a, b) => along(map.nodes.get(a)) - along(map.nodes.get(b)));
  if (ends.size === 0) return byPlace;
  const next = new Map([...nodeIds].map((id) => [id, []]));
  for (const { from, to } of ends.values()) {
    next.get(from)?.push(to);
    next.get(to)?.push(from);
  }
  const chain = [...next.values()].every((links) => links.length <= 2 && new Set(links).size === links.length)
    && ends.size === nodeIds.size - 1;
  const tips = [...next].filter(([, links]) => links.length === 1).map(([id]) => id);
  if (!chain || tips.length !== 2) return byPlace;
  const start = along(map.nodes.get(tips[0])) <= along(map.nodes.get(tips[1])) ? tips[0] : tips[1];
  const order = [start];
  while (order.length < nodeIds.size) {
    const step = next.get(order.at(-1)).find((id) => !order.includes(id));
    if (!step) return byPlace;
    order.push(step);
  }
  return order;
}

function applyMapColor(target, key, value) {
  if (value === undefined) return;
  if (value === null) {
    delete target[key];
    return;
  }
  const color = normalizeHexColor(value);
  if (color) target[key] = color;
}

function migrateReactionLocalOwnership(nodes) {
  for (const node of nodes.values()) {
    if (!['metabolite', 'medium'].includes(node?.kind)) continue;
    if (node.reactionLocal !== true || !node.reactionId || node.reactionLocalFor) continue;
    node.reactionLocalFor = node.reactionId;
    delete node.reactionId;
  }
}

/**
 * Give every drawn compartment a readable name and a visual identity.
 *
 * A map saved from a model whose compartments are named after their own ids
 * (`"c": "c"`, which is what cobrapy writes for an SBML compartment with no
 * name) has "c" and "h" stored as its labels, and its `i` and `s` drawn as
 * unknown or as the shared medium. Nothing in a file should have to be edited
 * for the cytosol to say Cytosol.
 */
function nameCellCompartments(cells) {
  for (const cell of cells ?? []) {
    for (const comp of cell?.compartments ?? []) {
      if (!comp?.id) continue;
      const names = { [comp.id]: comp.label };
      const resolved = cellCompartmentVisualId(comp.id, names);
      if (!comp.visualId || comp.visualId === comp.id) {
        // The one membrane a stale id gets wrong: a cell's own `s` was stroked
        // like the open shared medium.
        if (resolved === 'es' && comp.membrane === membraneStyle('s')) comp.membrane = membraneStyle(resolved);
        comp.visualId = resolved;
      }
      if (!comp.label || comp.label === comp.id) comp.label = cellCompartmentName(comp.id, names);
    }
  }
}

/**
 * Maps saved by earlier builds carry the species in front of every
 * hand-placed metabolite: `base` is the label, and it was read off the merged id,
 * so a node of the model `iDemo` is named `iDemo@g6p`. Mend them on the way in,
 * so an old file does not have to be edited by hand to be readable.
 */
function stripSpeciesFromLabels(nodes) {
  for (const node of nodes.values()) {
    if (!METABOLITE_KINDS.has(node?.kind) || typeof node.base !== 'string') continue;
    // The cell for a cellular metabolite; `M` for one in the shared medium.
    const merged = typeof node.mergedId === 'string' ? node.mergedId : '';
    const owners = [node.cellId, merged.includes('@') ? merged.slice(0, merged.indexOf('@')) : null];
    const owner = owners.find((id) => id && node.base.startsWith(`${id}@`));
    if (owner) node.base = node.base.slice(owner.length + 1);
  }
}

function normalizeVisualRoles(nodes) {
  for (const node of nodes.values()) {
    if (!METABOLITE_KINDS.has(node?.kind)) continue;

    if (node.kind === 'currency') {
      node.primary = false;
      node.visualRole = 'secondary';
      node.r = SIZES.currencyRadius;
      continue;
    }

    if (node.visualRole === 'secondary') node.primary = false;
    else if (node.visualRole === 'primary') node.primary = true;
    else if (node.primary === false) node.visualRole = 'secondary';
    else {
      node.primary = true;
      node.visualRole = 'primary';
    }
    node.r = node.primary ? SIZES.primaryRadius : SIZES.currencyRadius;
  }
}

function normalizeMetaboliteLabelAngles(nodes) {
  for (const node of nodes.values()) {
    if (!METABOLITE_KINDS.has(node?.kind) || !Object.hasOwn(node, 'labelAngle')) continue;
    if (Number.isFinite(node.labelAngle)) node.labelAngle = metaboliteLabelAngle(node);
    else delete node.labelAngle;
  }
}

/* A moved reaction name keeps a usable offset; anything else puts it back in its default place. */
function normalizeReactionLabelOffsets(reactions) {
  for (const rxn of reactions.values()) {
    if (!rxn || !Object.hasOwn(rxn, 'labelOffset')) continue;
    const offset = normalizeReactionLabelOffset(rxn.labelOffset);
    if (offset) rxn.labelOffset = offset;
    else delete rxn.labelOffset;
  }
}

function normalizeRect(rect) {
  if (!rect) return null;
  const x1 = Number(rect.x);
  const y1 = Number(rect.y);
  const w = Number(rect.w);
  const h = Number(rect.h);
  if (![x1, y1, w, h].every(Number.isFinite)) return null;
  const x2 = x1 + w;
  const y2 = y1 + h;
  return {
    x: Math.min(x1, x2),
    y: Math.min(y1, y2),
    w: Math.abs(w),
    h: Math.abs(h),
  };
}

function clampCellRectToCanvas(map, requested) {
  const pad = 18;
  const minW = 180;
  const minH = 120;
  const env = map.environment?.rect ?? { x: 0, y: 0, w: map.width, h: map.height };
  const maxW = Math.max(minW, env.w - pad * 2);
  const maxH = Math.max(minH, env.h - pad * 2);
  const w = Math.min(maxW, Math.max(minW, requested.w));
  const h = Math.min(maxH, Math.max(minH, requested.h));
  const x = Math.min(
    Math.max(requested.x, env.x + pad),
    env.x + env.w - pad - w,
  );
  const y = Math.min(
    Math.max(requested.y, env.y + pad),
    env.y + env.h - pad - h,
  );
  return { x, y, w, h };
}

function findFreeCellRect(map, requested = null) {
  if (requested && [requested.x, requested.y, requested.w, requested.h].every(Number.isFinite)) {
    return { ...requested };
  }
  const w = Math.min(520, Math.max(260, map.width * 0.42));
  const h = Math.min(380, Math.max(200, map.height * 0.42));
  const gap = 36;
  for (let y = 50; y + h <= map.height - 30; y += 50) {
    for (let x = 50; x + w <= map.width - 30; x += 50) {
      const candidate = { x, y, w, h };
      if (map.cells.every((cell) => !rectsOverlap(candidate, cell.rect, gap))) return candidate;
    }
  }
  // If no free slot fits, grow the canvas to the right and use the new area.
  const x = map.cells.reduce((m, cell) => Math.max(m, cell.rect.x + cell.rect.w), 40) + gap;
  map.canvas.width = Math.max(map.canvas.width, x + w + 50);
  if (map.environment?.hidden) map.environment.rect.w = map.canvas.width;
  return { x, y: 50, w, h };
}

function findFreeChildRect(cell, parentId, requested = null) {
  const parent = cell.compartments.find((c) => c.id === parentId);
  if (!parent) return null;
  const p = parent.rect;
  const children = cell.compartments.filter((c) => c.parentId === parentId);

  // Every new child is inset from its biological parent. This guarantees a
  // visible parent space around it (periplasm around cytosol, mitochondrial
  // intermembrane space around matrix, cytosol around organelles, etc.).
  const shellPad = Math.max(10, Math.min(28, Math.min(p.w, p.h) * 0.055));
  if (requested && [requested.x, requested.y, requested.w, requested.h].every(Number.isFinite)) {
    return clampChildRect(requested, p, shellPad);
  }

  const availableW = Math.max(90, p.w - shellPad * 2);
  const availableH = Math.max(64, p.h - shellPad * 2);
  const w = Math.max(90, Math.min(availableW, p.w * (children.length ? 0.42 : 0.72)));
  const h = Math.max(64, Math.min(availableH, p.h * (children.length ? 0.42 : 0.72)));
  const candidates = [
    { x: p.x + (p.w - w) / 2, y: p.y + (p.h - h) / 2, w, h },
    { x: p.x + shellPad, y: p.y + shellPad, w, h },
    { x: p.x + p.w - shellPad - w, y: p.y + shellPad, w, h },
    { x: p.x + shellPad, y: p.y + p.h - shellPad - h, w, h },
    { x: p.x + p.w - shellPad - w, y: p.y + p.h - shellPad - h, w, h },
  ].map((r) => clampChildRect(r, p, shellPad));

  return candidates.find((r) => children.every((c) => !rectsOverlap(r, c.rect, 10)))
    ?? candidates[children.length % candidates.length]
    ?? null;
}

function clampChildRect(requested, parent, pad = 12) {
  const normalized = normalizeRect(requested);
  if (!normalized) return null;

  const minW = Math.min(90, Math.max(24, parent.w - pad * 2));
  const minH = Math.min(64, Math.max(20, parent.h - pad * 2));
  const maxW = Math.max(minW, parent.w - pad * 2);
  const maxH = Math.max(minH, parent.h - pad * 2);
  const w = Math.min(maxW, Math.max(minW, normalized.w));
  const h = Math.min(maxH, Math.max(minH, normalized.h));
  const x = Math.min(
    Math.max(normalized.x, parent.x + pad),
    parent.x + parent.w - pad - w,
  );
  const y = Math.min(
    Math.max(normalized.y, parent.y + pad),
    parent.y + parent.h - pad - h,
  );
  return { x, y, w, h };
}

function findFreeRootRect(cell, requested = null) {
  if (requested && [requested.x, requested.y, requested.w, requested.h].every(Number.isFinite)) {
    return { ...requested };
  }
  const roots = cell.compartments.filter((c) => c.parentId === null);
  const p = cell.rect;
  const gap = 14;
  const w = Math.max(90, (p.w - gap * (roots.length + 2)) / Math.max(2, roots.length + 1));
  const h = Math.max(64, p.h - gap * 2);
  return {
    x: p.x + gap + roots.length * (w + gap),
    y: p.y + gap,
    w,
    h,
  };
}

function unionRects(rects) {
  if (!rects.length) return null;
  const x1 = Math.min(...rects.map((r) => r.x));
  const y1 = Math.min(...rects.map((r) => r.y));
  const x2 = Math.max(...rects.map((r) => r.x + r.w));
  const y2 = Math.max(...rects.map((r) => r.y + r.h));
  return { x: x1, y: y1, w: x2 - x1, h: y2 - y1 };
}

function rectsOverlap(a, b, gap = 0) {
  return !(a.x + a.w + gap <= b.x
    || b.x + b.w + gap <= a.x
    || a.y + a.h + gap <= b.y
    || b.y + b.h + gap <= a.y);
}

function nextDuplicateNodeId(nodes, source) {
  const root = source.duplicateOf ?? source.id;
  let index = 2;
  let id = `${root}#${index}`;
  while (nodes.has(id)) {
    index += 1;
    id = `${root}#${index}`;
  }
  return id;
}

function nextTextId(texts) {
  let i = 1;
  while (texts.has(`text-${i}`)) i += 1;
  return `text-${i}`;
}

function asMap(value) {
  if (value instanceof Map) return value;
  if (!value) return new Map();
  if (Array.isArray(value)) return new Map(value);
  return new Map(Object.entries(value));
}

/**
 * Every membrane in the drawing, as a first-class object.
 *
 * One per compartment (the surface of a compartment is what separates it from
 * whatever it sits in), plus one for the shared medium, whose outside is the
 * world beyond the system. Each carries who it belongs to, which two
 * compartments it lies between, and the rectangle it is drawn as, which is what
 * hit-testing a click on a membrane needs.
 */
function deriveBoundaries(map) {
  const env = map.environment;
  const out = [{
    id: ENVIRONMENT_KEY,
    kind: BOUNDARY_KIND.ENVIRONMENT,
    cellId: null,
    inside: env?.id ?? SHARED_COMPARTMENT,
    outside: null,
    compartmentId: env?.id ?? SHARED_COMPARTMENT,
    parentId: null,
    depth: -1,
    membrane: membraneStyle(env?.id ?? SHARED_COMPARTMENT),
    rect: env?.rect ?? null,
    label: env?.label ?? '',
  }];

  for (const cell of map.cells) {
    for (const comp of cell.compartments) {
      out.push({
        id: boundaryKey(cell.id, comp.id),
        kind: comp.parentId === null ? BOUNDARY_KIND.CELL : BOUNDARY_KIND.MEMBRANE,
        cellId: cell.id,
        inside: comp.id,
        // The outermost membrane of a cell faces the shared medium.
        outside: comp.parentId ?? SHARED_COMPARTMENT,
        compartmentId: comp.id,
        parentId: comp.parentId,
        depth: comp.depth,
        membrane: comp.membrane ?? membraneStyle(comp.id),
        rect: comp.rect,
        label: comp.label,
      });
    }
  }
  return out;
}

/**
 * Deep-copy reactions and segments while preserving the one piece of shared
 * identity in the geometry: a reaction's axis endpoints are the same objects
 * its segments are anchored to.
 */
function cloneGraphics(reactions, segments) {
  const copies = new Map();
  const axes = new Map();
  for (const [id, rxn] of reactions) {
    const copy = { ...rxn };
    if (rxn.axis) {
      copy.axis = {
        a1: { ...rxn.axis.a1 },
        a2: { ...rxn.axis.a2 },
        dir: { ...rxn.axis.dir },
      };
      axes.set(id, { from: rxn.axis, to: copy.axis });
    }
    if (rxn.normal) copy.normal = { ...rxn.normal };
    if (rxn.boundary) copy.boundary = { ...rxn.boundary };
    if (rxn.linear) copy.linear = { ...rxn.linear };
    if (rxn.lineEnds) copy.lineEnds = { ...rxn.lineEnds };
    if (rxn.labelOffset) copy.labelOffset = { ...rxn.labelOffset };
    copies.set(id, copy);
  }

  const cloned = segments.map((seg) => {
    const axis = axes.get(seg.reactionId);
    const at = (point) => {
      if (axis) {
        if (point === axis.from.a1) return axis.to.a1;
        if (point === axis.from.a2) return axis.to.a2;
      }
      return { x: point.x, y: point.y };
    };
    return {
      ...seg,
      from: at(seg.from),
      to: at(seg.to),
      b1: seg.b1 ? { ...seg.b1 } : undefined,
      b2: seg.b2 ? { ...seg.b2 } : undefined,
      control: seg.control ? { ...seg.control } : undefined,
    };
  });

  return { reactions: copies, segments: cloned };
}

/**
 * Point every segment endpoint that belongs to a reaction axis back at the axis
 * object, following the rules `buildSegments()` draws with: the axis segment
 * runs a1 -> a2, a substrate arrives at a1, a product leaves from a2, and a
 * boundary stub leaves from a2.
 */
function relinkAxisEndpoints(reactions, segments) {
  for (const seg of segments) {
    const axis = reactions.get(seg.reactionId)?.axis;
    if (!axis) continue;
    if (seg.kind === 'axis') {
      seg.from = axis.a1;
      seg.to = axis.a2;
    } else if (seg.kind === 'boundary') {
      seg.from = axis.a2;
    } else if (seg.transportLane || seg.backboneAnchor) {
      // Transport lanes and Escher-style backbone branches keep their own
      // reaction-side attachment point instead of collapsing to a1/a2.
      continue;
    } else if (seg.side === 'substrate') {
      seg.to = axis.a1;
    } else if (seg.side === 'product') {
      seg.from = axis.a2;
    }
  }
}

export { deriveBoundaries };
