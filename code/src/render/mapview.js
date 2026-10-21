/**
 * SVG renderer.
 *
 * Produces a complete SVG document as a string, which keeps it testable outside
 * a browser and makes redrawing cheap: the app swaps innerHTML and re-binds
 * event handlers through delegation on stable data- attributes.
 *
 * Drawing order matters and mirrors the containment hierarchy: medium, then
 * cells outside-in, then flux, then nodes, then text on top of everything.
 */

import { compartmentStyle, formatFlux } from './theme.js';
import { DEFAULT_TEXT_FONT, textFontStack } from '../core/text-style.js';
import { readableName } from '../core/label-sources.js';
import { LINK_COMPARTMENT } from '../core/community.js';
import {
  closestPerimeterPoint, compartmentCorners, compartmentShape, isRoundCompartment,
} from '../layout/geometry.js';
import { shapeHandlePoint } from '../map/geometry-edit.js';
import { boundaryKey, ENVIRONMENT_KEY } from '../map/boundaries.js';
import { SIZES } from '../layout/layout.js';
import { compartmentKey, textBox } from '../map/multi-selection.js';
import { isSecondaryNode, metaboliteLabelPosition } from '../map/metabolite-label.js';
import { reactionEndpointGeometry } from './reaction-path.js';
import { findLineHops, hoppedPath } from './line-hops.js';
import { crossFeedingLineWidth, isDrawnCrossFeedingBase, roundedPath, routeAnchor, routeCrossFeeding, routeGrid } from './crossfeed-routes.js';
import { crossFeedingColor, crossFeedingColors } from './crossfeed-colors.js';

const esc = (s) => String(s ?? '')
  .replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
  .replace(/"/g, '&quot;');

const n = (v) => (Math.round(v * 100) / 100).toString();

/**
 * Render the whole map as one SVG document string.
 *
 * @param {object} layout  result of layoutCommunity()
 * @param {object} state
 * @param {Map<string, number>} state.fluxes
 * @param {Map<string, number>} [state.solvedFluxes] actual flux directions when fluxes contains differences
 * @param {'constant'|'flux'} [state.reactionWidthMode] opt-in scaling by the displayed flux magnitude
 * @param {Set<string>} [state.disabledReactionIds] map-visible zero-bound reactions
 * @param {Set<string>} [state.modifiedReactionIds] reactions whose bounds differ from the model's, badged
 * @param {Set<string>} [state.inactiveSpeciesIds] context cells kept visible outside the current simulation
 * @param {import('./theme.js').FluxScale} state.scale
 * @param {boolean} [state.dark]
 * @param {string|null} [state.selectedReaction]
 * @param {string|null} [state.selectedNode]
 * @param {boolean} [state.reactionEdit] show control points for every drawn reaction
 * @param {'auto'|'all'|'active'|'none'} [state.labels] `none` hides every label
 * @param {boolean} [state.showCurrency]
 * @param {Map<string,number>} [state.growth]  species id -> growth rate
 * @param {boolean} [state.forExport] the picture only: no selection, handles,
 *   pointer targets or canvas resize grips
 * @param {boolean} [state.offStyle] draw reactions that carry no flux in the
 *   solution grey and dashed; pass it only when there is a solution
 * @param {boolean} [state.lineHops] a line drawn under another is cut where they cross
 * @param {number} [state.crossFeedingWidth] cross-feeding line width, as a
 *   multiple of the default (see crossFeedingLineWidth)
 * @param {{reaction:(id:string)=>string, node:(id:string)=>string}} [state.tooltips]
 *   hover text for reactions and metabolites, written into the SVG
 */
export function renderMap(layout, state) {
  // An exported figure is the map, not the editor around it.
  if (state.forExport) {
    state = {
      ...state,
      selectedReaction: null, selectedGeometry: null, geometryEdit: false,
      reactionEdit: false, multiSelection: null, minHitRadius: 0,
      // A badge for edited bounds is a note to the editor, not part of the figure.
      modifiedReactionIds: new Set(),
      // The event picked in the cross-feeding explorer is a selection too.
      crossFeedingExplorer: state.crossFeedingExplorer
        ? { ...state.crossFeedingExplorer, selectedEvent: null }
        : null,
    };
  }
  const forExport = Boolean(state.forExport);
  const {
    fluxes = new Map(), scale, dark = false,
    solvedFluxes = fluxes, reactionWidthMode = 'constant',
    disabledReactionIds = new Set(), inactiveSpeciesIds = new Set(), inactiveSpeciesLabel = 'inactive context',
    emphasizedReactionIds = new Set(), modifiedReactionIds = new Set(),
    reactionColorMode = 'flux', reactionColors = new Map(),
    pathwayRibbonMode = 'off', pathwayRibbonColors = new Map(), pathwayRibbonGroups = new Map(),
    pathwayRibbonWidth = 56, pathwayRibbonOpacity = 0.18,
    selectedReaction = null, selectedGeometry = null, geometryEdit = false,
    reactionEdit = false, multiSelection = null,
    fvaEnabled = false, fvaRanges = new Map(), fvaProgress = null,
    labels = 'auto', showCurrency = true,
    currencyLabels = true,
    metaboliteLabels = new Map(), reactionLabels = new Map(),
    growth = new Map(), crossFeeding = null, showCrossFeeding = false, crossFeedingExplorer = null,
    crossFeedingBases = null, crossFeedingWidth = 1,
    omicsEnabled = false, omicsColors = new Map(), omicsFallbackColor = dark ? '#475569' : '#cbd5e1',
    minHitRadius = 0,
    offStyle = false, lineHops = false, tooltips = null,
  } = state;

  const parts = [];
  parts.push(
    `<svg xmlns="http://www.w3.org/2000/svg" class="emc-map${dark ? ' emc-dark' : ''}" `
    + `viewBox="0 0 ${n(layout.width)} ${n(layout.height)}" `
    + `width="${n(layout.width)}" height="${n(layout.height)}" role="img" `
    + `aria-label="Multicell community metabolic map">`,
  );
  parts.push(defs(dark));
  parts.push(renderCanvasFrame(layout, dark));

  parts.push(renderEnvironment(layout, dark, minHitRadius));
  // Routed lines run through the medium, so they go over the cells' fills and
  // under the exchanges they end at. A link that cannot be routed is drawn as
  // an arc, before the cells so it passes behind them instead of cutting
  // across the pathways it connects.
  const crossFeedingLayers = showCrossFeeding && crossFeeding
    ? renderCrossFeeding(layout, crossFeeding, scale, dark, crossFeedingExplorer, crossFeedingBases, crossFeedingWidth)
    : null;
  if (crossFeedingLayers) parts.push(crossFeedingLayers.behind);
  parts.push(renderCells(layout, dark, growth, minHitRadius, inactiveSpeciesIds, inactiveSpeciesLabel));
  if (crossFeedingLayers) parts.push(crossFeedingLayers.routed);
  parts.push(renderPathwayRibbons(
    layout,
    pathwayRibbonMode,
    pathwayRibbonColors,
    pathwayRibbonGroups,
    pathwayRibbonWidth,
    pathwayRibbonOpacity,
    showCurrency,
  ));
  const crossFeedingReactions = activeCrossFeedingReactionIds(crossFeedingExplorer, showCrossFeeding);
  parts.push(renderSegments(
    layout, fluxes, scale, selectedReaction, showCurrency, multiSelection,
    reactionColorMode, reactionColors, omicsEnabled, omicsColors, omicsFallbackColor,
    crossFeedingReactions, showCrossFeeding,
    {
      solvedFluxes, reactionWidthMode, disabledReactionIds, inactiveSpeciesIds,
      offStyle, lineHops, dark, tooltip: tooltips?.reaction ?? null,
    },
  ));
  // The multi-selection area goes under the marks, so it reads as the area they
  // sit on rather than a box drawn over them.
  const selectionBounds = multiSelectionBounds(layout, multiSelection);
  parts.push(renderMultiSelectionArea(selectionBounds, selectedCompartmentRects(layout, multiSelection)));
  // Invisible pointer targets keep a glyph clickable at any zoom without
  // changing how it is drawn. They go between the segments and the markers,
  // which is the hit-test order the tests expect.
  if (!forExport) parts.push(renderHitTargets(layout, showCurrency, minHitRadius));
  parts.push(renderMarkers(
    layout, fluxes, scale, selectedReaction, dark, multiSelection,
    reactionColorMode, reactionColors, omicsEnabled, omicsColors, omicsFallbackColor,
    { solvedFluxes, disabledReactionIds, modifiedReactionIds, inactiveSpeciesIds, offStyle, tooltip: tooltips?.reaction ?? null },
  ));
  parts.push(renderNodes(layout, dark, showCurrency, multiSelection, inactiveSpeciesIds, tooltips?.node ?? null));
  parts.push(renderEditHandles(layout, dark, reactionEdit, minHitRadius));
  parts.push(renderText(
    layout, fluxes, scale, dark, labels, showCurrency, currencyLabels,
    fvaEnabled, fvaRanges, fvaProgress,
    { solvedFluxes, disabledReactionIds, inactiveSpeciesIds, emphasizedReactionIds, metaboliteLabels, reactionLabels, offStyle },
  ));
  parts.push(renderMapAnnotations(layout, dark, multiSelection));
  if (crossFeedingLayers?.tag) parts.push(crossFeedingLayers.tag);
  if (!forExport) parts.push(renderCanvasResizeHandles(layout, dark));
  parts.push(renderCompartmentLabels(layout, dark, inactiveSpeciesIds));
  // The handles (corners, sides, the centre to move by and the two shape
  // diamonds) belong to any compartment that is selected, not only to the
  // geometry-editing tools. Select sets the same geometry selection, so
  // choosing a compartment there shows them and they can be dragged straight
  // away. They go over the name plates, because a plate astride the top edge of
  // a round compartment would cover the handles running along it.
  if (geometryEdit || selectedGeometry) {
    parts.push(renderGeometryHandles(layout, selectedGeometry, dark, minHitRadius));
  }
  parts.push(renderMultiSelectionBorder(selectionBounds, minHitRadius));

  parts.push('</svg>');
  return parts.join('');
}

// Geometry editor

/** The side of a corner handle, in map units. The shape handles are spaced by it. */
export function geometryHandleSide(minHitRadius = 0) {
  return Math.max(12, (minHitRadius || 0) * 2);
}

function renderGeometryHandles(layout, selection, dark, minHitRadius = 0) {
  if (!selection) return '';
  const isEnvironment = (selection.cellId === null || selection.cellId === undefined)
    && selection.compartmentId === layout.environment?.id;
  const cell = isEnvironment ? null : layout.cells.find((c) => c.id === selection.cellId);
  const comp = isEnvironment
    ? layout.environment
    : cell?.compartments.find((c) => c.id === selection.compartmentId);
  if (!comp?.rect) return '';

  const r = comp.rect;
  const stroke = dark ? '#f8fafc' : '#0f172a';
  const fill = dark ? '#111827' : '#ffffff';
  const base = `data-geometry-cell="${isEnvironment ? '' : esc(cell.id)}" data-geometry-comp="${esc(comp.id)}"`;
  const corners = [
    ['nw', r.x, r.y],
    ['ne', r.x + r.w, r.y],
    ['se', r.x + r.w, r.y + r.h],
    ['sw', r.x, r.y + r.h],
  ];
  // The middle of each side moves that side alone: width or height only.
  const sides = [
    ['n', r.x + r.w / 2, r.y],
    ['e', r.x + r.w, r.y + r.h / 2],
    ['s', r.x + r.w / 2, r.y + r.h],
    ['w', r.x, r.y + r.h / 2],
  ];

  const out = [
    '<g class="emc-layer emc-layer-geometry-editor">',
    `<rect class="emc-geometry-selection" x="${n(r.x)}" y="${n(r.y)}" width="${n(r.w)}" height="${n(r.h)}"
      fill="none" stroke="${stroke}" stroke-width="2" stroke-dasharray="7 5" pointer-events="none"/>`,
  ];

  // Each handle says what it does, on the handle. Moving a compartment takes two
  // steps (select it, then drag its middle), and a tooltip where the pointer
  // already is beats explaining it anywhere else.
  // Sized in screen terms, like the reaction halos: a 12-unit square is a
  // pixel and a half once a large map is fitted to the stage, which is neither
  // visible nor clickable.
  // minHitRadius is MIN_HIT_PX in map units, and MIN_HIT_PX is a radius: twice
  // it is the 24-pixel target the halos already use, so a handle ends up the
  // same size on screen as everything else that can be grabbed.
  const side = geometryHandleSide(minHitRadius);
  const half = side / 2;
  const moveRadius = Math.max(8, minHitRadius || 0);

  // A bar lying along its side, so it reads as "this side" and not as one more
  // corner. Its short dimension is the corner's, which keeps the hit target.
  // Drawn before the corners, so a corner stays on top where they meet.
  const long = side * 1.75;
  for (const [handle, x, y] of sides) {
    const across = handle === 'n' || handle === 's';
    const w = across ? long : side;
    const h = across ? side : long;
    out.push(`<rect class="emc-geometry-handle emc-geometry-${handle}" ${base}
      data-geometry-handle="${handle}" x="${n(x - w / 2)}" y="${n(y - h / 2)}"
      width="${n(w)}" height="${n(h)}"
      rx="${n(side / 2)}" fill="${fill}" stroke="${stroke}" stroke-width="2"
      ><title>Drag to change this compartment's ${across ? 'height' : 'width'}</title></rect>`);
  }

  for (const [handle, x, y] of corners) {
    out.push(`<rect class="emc-geometry-handle emc-geometry-${handle}" ${base}
      data-geometry-handle="${handle}" x="${n(x - half)}" y="${n(y - half)}"
      width="${n(side)}" height="${n(side)}"
      rx="2" fill="${fill}" stroke="${stroke}" stroke-width="2"
      ><title>Drag to resize this compartment</title></rect>`);
  }

  out.push(`<circle class="emc-geometry-handle emc-geometry-move" ${base}
    data-geometry-handle="move" cx="${n(r.x + r.w / 2)}" cy="${n(r.y + r.h / 2)}" r="${n(moveRadius)}"
    fill="${fill}" stroke="${stroke}" stroke-width="2"
    ><title>Drag to move this compartment and everything in it</title></circle>`);

  // The shape diamonds, a cell's compartments only: the orange one along the
  // top softens the corners (and, on an oval, makes it rounder or boxier), the
  // yellow one down the left side rounds the outline into an oval. Where each
  // stands on its edge is how far it goes.
  if (!isEnvironment) {
    const shape = compartmentShape(comp);
    const reach = side * 0.62;
    for (const [handle, amount, colour, edge, title] of [
      ['corner', shape.corner, SHAPE_HANDLE_COLOURS.corner, dark ? '#fed7aa' : '#9a3412',
        'Drag along the edge to soften the corners (on an oval: rounder or boxier)'],
      ['oval', shape.oval, SHAPE_HANDLE_COLOURS.oval, dark ? '#fef08a' : '#854d0e',
        'Drag along the edge to make this compartment oval'],
    ]) {
      const p = shapeHandlePoint(r, handle, amount, side);
      out.push(`<path class="emc-geometry-handle emc-geometry-shape emc-geometry-${handle}" ${base}
        data-geometry-handle="${handle}" data-shape-amount="${n(amount)}"
        d="M${n(p.x)} ${n(p.y - reach)}L${n(p.x + reach)} ${n(p.y)}L${n(p.x)} ${n(p.y + reach)}L${n(p.x - reach)} ${n(p.y)}Z"
        fill="${colour}" stroke="${edge}" stroke-width="2" stroke-linejoin="round"
        ><title>${title}</title></path>`);
    }
  }
  out.push('</g>');
  return out.join('');
}

// Orange and sunflower yellow, the same in both themes, so the inspector can
// name each by its colour.
const SHAPE_HANDLE_COLOURS = { corner: '#f97316', oval: '#ffc512' };

// Defs

function defs(dark) {
  const wall = dark ? '#78716c' : '#a8a29e';
  return `<defs>
    <pattern id="emc-wall" width="9" height="9" patternUnits="userSpaceOnUse" patternTransform="rotate(45)">
      <line x1="0" y1="0" x2="0" y2="9" stroke="${wall}" stroke-width="3" opacity="0.5"/>
    </pattern>
    <filter id="emc-cell-shadow" x="-12%" y="-12%" width="124%" height="124%">
      <feDropShadow dx="0" dy="6" stdDeviation="12" flood-opacity="${dark ? 0.55 : 0.16}"/>
    </filter>
    <filter id="emc-glow" x="-40%" y="-40%" width="180%" height="180%">
      <feGaussianBlur stdDeviation="4" result="b"/>
      <feMerge><feMergeNode in="b"/><feMergeNode in="SourceGraphic"/></feMerge>
    </filter>
    <filter id="emc-ribbon-soft" x="-30%" y="-30%" width="160%" height="160%">
      <feGaussianBlur stdDeviation="5.5"/>
    </filter>
  </defs>`;
}

// Canvas

function renderCanvasFrame(layout, dark) {
  const fill = dark ? '#0f172a' : '#ffffff';
  const stroke = dark ? '#475569' : '#cbd5e1';
  return `<g class="emc-layer emc-layer-canvas">
    <rect class="emc-canvas-frame" data-canvas-bg
      x="0.75" y="0.75" width="${n(Math.max(0, layout.width - 1.5))}" height="${n(Math.max(0, layout.height - 1.5))}"
      fill="${fill}" stroke="${stroke}" stroke-width="1.5"/>
  </g>`;
}

function renderCanvasResizeHandles(layout, dark) {
  const stroke = dark ? '#60a5fa' : '#2563eb';
  const w = layout.width;
  const h = layout.height;
  const x2 = n(Math.max(8, w - 8));
  const y2 = n(Math.max(8, h - 8));

  // All four edges, so the canvas can grow upwards or to the left without
  // moving everything on it by hand first. Dragging the top or the left edge
  // grows the canvas there and carries the drawing with it, which is what keeps
  // the picture still while the paper around it changes.
  const edge = (name, x1, yy1, xx2, yy2) => `
    <line class="emc-canvas-resize-hit emc-canvas-resize-${name}"
      data-canvas-resize="${name}" x1="${x1}" y1="${yy1}" x2="${xx2}" y2="${yy2}"
      stroke="transparent" stroke-width="16" pointer-events="stroke"/>
    <line class="emc-canvas-resize-guide emc-canvas-resize-${name}"
      x1="${x1}" y1="${yy1}" x2="${xx2}" y2="${yy2}"
      stroke="${stroke}" stroke-width="1.5" opacity="0.28" pointer-events="none"/>`;

  return `<g class="emc-layer emc-layer-canvas-resize">
    ${edge('right', n(w - 1), 8, n(w - 1), y2)}
    ${edge('bottom', 8, n(h - 1), x2, n(h - 1))}
    ${edge('left', 1, 8, 1, y2)}
    ${edge('top', 8, 1, x2, 1)}
    <rect class="emc-canvas-resize-corner" data-canvas-resize="corner"
      x="${n(w - 12)}" y="${n(h - 12)}" width="12" height="12" rx="2"
      fill="${stroke}" opacity="0.48"/>
  </g>`;
}

// Environment

/**
 * How wide a grab band has to be, in map units, to stay a real target.
 *
 * Everything on this map is drawn in map units and scaled by the camera, so a
 * stroke of 18 is 18 units, about two screen pixels at the fitted view of a
 * large map. Like the reaction and metabolite halos, the band compensates
 * through minHitRadius (MIN_HIT_PX / scale), so a membrane is a target a hand
 * can aim at and not only one a test can dispatch an event on.
 *
 * Capped against the shape itself, so a band never swallows a small compartment.
 */
function grabBand(rect, minHitRadius, base = 18) {
  const wanted = Math.max(base, (minHitRadius || 0) * 2);
  const smallest = Math.min(rect?.w ?? Infinity, rect?.h ?? Infinity);
  return Math.max(base, Math.min(wanted, smallest / 3));
}

function renderEnvironment(layout, dark, minHitRadius = 0) {
  if (layout.environment?.hidden) return '';
  const s = compartmentStyle('s', dark);
  const r = layout.environment.rect;
  const attrs = `x="${n(r.x)}" y="${n(r.y)}" width="${n(r.w)}" height="${n(r.h)}" rx="26"`;
  return `<g class="emc-layer emc-layer-env">
    <rect ${attrs} fill="${s.fill}" stroke="${s.stroke}" stroke-width="2" stroke-dasharray="10 7"/>
    <rect class="emc-boundary-hit emc-environment-boundary" data-boundary="${esc(ENVIRONMENT_KEY)}"
      ${attrs} fill="none" stroke="transparent" stroke-width="${n(grabBand(r, minHitRadius))}"
      pointer-events="stroke"/>
    <rect class="emc-boundary-glow" ${attrs} fill="none" stroke="transparent" stroke-width="5"
      pointer-events="none"/>
  </g>`;
}

// Cells

function renderCells(layout, dark, growth, minHitRadius = 0, inactiveSpeciesIds = new Set(), inactiveLabel = 'inactive context') {
  const out = ['<g class="emc-layer emc-layer-cells">'];

  for (const cell of layout.cells) {
    const inactive = inactiveSpeciesIds.has(cell.id);
    out.push(`<g class="emc-cell${inactive ? ' is-inactive-species' : ''}" data-cell="${esc(cell.id)}"${inactive ? ' opacity="0.32"' : ''}>`);

    // Compartments, outermost first, so nesting reads as containment. Shapes and
    // labels go in two passes: a nested compartment's opaque label pill would
    // otherwise be painted over by the compartment drawn after it.
    const ordered = [...cell.compartments].sort((a, b) => a.depth - b.depth);
    for (const comp of ordered) {
      out.push(renderCompartmentShape(comp, dark, cell, comp.depth === 0, minHitRadius));
    }

    const outer = ordered.find((comp) => comp.id === cell.outerCompartment)
      ?? ordered.find((comp) => comp.parentId === null);
    if (outer) out.push(renderCellBoundary(cell, outer, dark));

    // Cell identity plate, standing on the outer membrane's top-left corner. It
    // is as wide as its text and grows with it, up to the width of the cell.
    const r = cell.rect;
    const mu = inactive ? undefined : growth.get(cell.id);
    const muText = mu === undefined ? '' : `  ·  µ = ${formatFlux(mu, 4)} h⁻¹`;
    const plateText = `${cell.name ?? ''}${inactive ? ` · ${inactiveLabel}` : muText}`;
    const textWidth = plateText.length * PLATE_CHAR;
    const plateWidth = textWidth + 44;
    out.push(`<g class="emc-cell-plate" data-cell-plate="${esc(cell.id)}" transform="translate(${n(r.x)} ${n(r.y - 10)})">
      <g class="emc-frame-label" style="--fit:${fitScale(cellPlateFit(cell, layout.environment?.rect, plateText))}">
        <rect x="0" y="-30" width="${n(plateWidth)}" height="30" rx="15"
          fill="${cell.color}" opacity="0.14"/>
        <circle cx="17" cy="-15" r="6" fill="${cell.color}"/>
        <text class="emc-cell-name" x="30" y="-10" textLength="${n(textWidth)}" lengthAdjust="spacingAndGlyphs"
          fill="${cell.color}">${esc(plateText)}</text>
      </g>
    </g>`);
    out.push('</g>');
  }

  out.push('</g>');
  return out.join('');
}

function renderCellBoundary(cell, outer, dark) {
  const r = outer.rect;
  const corners = compartmentCorners(outer);
  const boundaryId = boundaryKey(cell.id, outer.id);
  // A border chosen in the inspector wins over the species colour.
  const stroke = cell.stroke || cell.color || (dark ? '#60a5fa' : '#2563eb');
  return `<rect class="emc-cell-boundary" data-cell-boundary="${esc(cell.id)}"
    data-boundary="${esc(boundaryId)}"
    x="${n(r.x)}" y="${n(r.y)}" width="${n(r.w)}" height="${n(r.h)}" ${cornerAttrs(corners)}
    fill="none" stroke="${stroke}" stroke-width="3" opacity="0.72"
    pointer-events="stroke"/>`;
}

/* A rect's rx and ry: a compartment's corners may be elliptical. */
function cornerAttrs({ rx, ry }, inset = 0) {
  return `rx="${n(Math.max(0, rx - inset))}" ry="${n(Math.max(0, ry - inset))}"`;
}

function renderCompartmentShape(comp, dark, cell, isOuter, minHitRadius = 0) {
  const theme = compartmentStyle(comp.visualId ?? comp.id, dark);
  // Colours chosen in the inspector are map-only and win over the theme.
  const s = { fill: comp.fill || theme.fill, stroke: comp.stroke || theme.stroke };
  const r = comp.rect;
  const corners = compartmentCorners(comp);
  const attrs = `x="${n(r.x)}" y="${n(r.y)}" width="${n(r.w)}" height="${n(r.h)}" ${cornerAttrs(corners)}`;
  const boundaryId = boundaryKey(cell.id, comp.id);
  const out = [`<g class="emc-compartment emc-comp-${esc(comp.id)}" data-compartment="${esc(comp.id)}">`];

  const shadow = isOuter ? ' filter="url(#emc-cell-shadow)"' : '';
  out.push(`<rect class="emc-compartment-space" data-cell-space="${esc(cell.id)}" data-compartment-space="${esc(comp.id)}" ${attrs} fill="${s.fill}"${shadow}/>`);

  if (comp.membrane === 'bilayer') {
    // Two concentric strokes read as the two leaflets of a lipid bilayer.
    out.push(`<rect ${attrs} fill="none" stroke="${s.stroke}" stroke-width="4.5"/>`);
    out.push(`<rect x="${n(r.x + 5)}" y="${n(r.y + 5)}" width="${n(Math.max(0, r.w - 10))}" `
      + `height="${n(Math.max(0, r.h - 10))}" ${cornerAttrs(corners, 5)} fill="none" `
      + `stroke="${s.stroke}" stroke-width="1.4" opacity="0.55"/>`);
  } else if (comp.membrane === 'wall') {
    out.push(`<rect ${attrs} fill="url(#emc-wall)" opacity="0.7"/>`);
    out.push(`<rect ${attrs} fill="none" stroke="${s.stroke}" stroke-width="5"/>`);
  } else {
    out.push(`<rect ${attrs} fill="none" stroke="${s.stroke}" stroke-width="1.8" stroke-dasharray="7 5"/>`);
  }

  // Wide transparent stroke: easy membrane hit-testing without making the
  // compartment interior clickable.
  out.push(`<rect class="emc-boundary-hit" data-boundary="${esc(boundaryId)}" ${attrs} `
    + `fill="none" stroke="transparent" stroke-width="${n(grabBand(comp.rect, minHitRadius))}" `
    + 'pointer-events="stroke"/>'
  );
  // Paints the hover; takes no pointer events, so it cannot move the band that does.
  out.push(`<rect class="emc-boundary-glow" ${attrs} fill="none" stroke="transparent" `
    + 'stroke-width="5" pointer-events="none"/>');

  out.push('</g>');
  return out.join('');
}

/*
 * Name plates of compartments, cells and the medium.
 *
 * A plate is a pill with its text, drawn once at its base size and then scaled
 * as a whole by the stylesheet: up as the map is zoomed out, so that a name
 * stays the same size on screen, and never past what its own compartment has
 * room for (--fit). Scaled together, the pill and the text cannot disagree about
 * how big the text is. Enlarging the text alone would, at the fitted view of a
 * large map, run it out of both ends of a pill three times too short for it,
 * across the membrane it is supposed to sit inside.
 *
 * The width of a piece of text is not knowable here, so it is not left to
 * chance: textLength pins it to the estimate the pill is cut to.
 */
const LABEL_CHAR = 8.6;      // one capital of a compartment name, at base size
const LABEL_HEIGHT = 19;
const PLATE_CHAR = 9.4;      // one character of a cell name, at base size
const FRAME_SCALE_MAX = 16;
const FRAME_SCALE_MIN = 0.2;
const FRAME_SCALE_GOOD = 6;  // a plate that can grow this far is not cramped

/** How much the plates are enlarged at the current zoom, 1 once zoomed in to full size. */
export function frameLabelScale(minHitRadius = 0) {
  // minHitRadius is 12 px in map units, so this is 1 / zoom.
  return Math.min(FRAME_SCALE_MAX, Math.max(1, (minHitRadius || 0) / 12));
}

/**
 * How far a cell's name plate may grow: as wide as the cell, and no taller
 * than the strip of medium above the cell it stands in.
 */
function cellPlateFit(cell, envRect, text = cell.name ?? '') {
  const width = String(text).length * PLATE_CHAR + 44;
  const above = envRect ? cell.rect.y - 10 - envRect.y : Infinity;
  return Math.max(1, Math.min(cell.rect.w / width, above / 32, FRAME_SCALE_MAX));
}

/** A fit factor as the stylesheet wants it. */
function fitScale(value) {
  return Number.isFinite(value) ? Math.max(FRAME_SCALE_MIN, value).toFixed(3) : '1000';
}

/**
 * The largest scale, up to `most`, at which a plate anchored at (x, y) stays
 * clear of every rectangle in `taken`. A plate only ever grows away from its
 * anchor, so once it touches something it keeps touching it: a bisection finds
 * the edge. Zero when it is not clear even at its smallest.
 */
function clearScale(x, y, box, w, h, taken, most) {
  const hits = (scale) => {
    const left = x + box.x * scale;
    const top = y + box.y * scale;
    return taken.some((t) => left < t.x + t.w && t.x < left + w * scale
      && top < t.y + t.h && t.y < top + h * scale);
  };
  if (!hits(most)) return most;
  let low = FRAME_SCALE_MIN;
  if (hits(low)) return 0;
  let high = most;
  for (let i = 0; i < 14; i += 1) {
    const mid = (low + high) / 2;
    if (hits(mid)) high = mid;
    else low = mid;
  }
  return low;
}

/**
 * One name plate in a corner of `rect`, or centred on its top edge.
 *
 * Where it goes and how large it may get are settled together, and for every
 * zoom at once: the corner is the first that lets the plate reach a good size
 * without running into anything in `taken` (the compartments nested inside,
 * mostly), and that size is its limit. Chosen that way, a plate neither jumps
 * from corner to corner as the map is zoomed nor grows over its neighbours.
 *
 * @param {object[]} [taken] rectangles the plate has to keep out of
 * @param {number} [limit]  a further cap on its scale, for the caller's reasons
 */
function frameLabel({
  id, label, rect: r, style: s, margin = 14, centred = false, taken = [], limit = Infinity, cls = '', position = null,
}) {
  const text = String(label ?? '').trim();
  if (!text) return '';
  const textWidth = text.length * LABEL_CHAR;
  const w = textWidth + 18;
  const h = LABEL_HEIGHT;
  // As large as the others, unless the compartment is too small to hold that.
  const room = Math.min((r.w - 2 * margin) / w, (r.h * 0.34) / h, limit, FRAME_SCALE_MAX);
  const most = Math.max(FRAME_SCALE_MIN, room);

  let x;
  let y;
  let box;
  let fit = most;
  if (position) {
    x = position.x;
    y = position.y;
    box = { x: -w / 2, y: -h / 2 };
    centred = true;
  } else if (centred) {
    // Astride the top edge, in the middle of it.
    x = r.x + r.w / 2;
    y = r.y;
    box = { x: -w / 2, y: -h / 2 };
    const clear = clearScale(x, y, box, w, h, taken, most);
    if (clear > 0) fit = clear;
  } else {
    const inset = 15;
    const corners = [
      { x: r.x + margin, y: r.y + inset, box: { x: 0, y: 0 } },
      { x: r.x + r.w - margin, y: r.y + inset, box: { x: -w, y: 0 } },
      { x: r.x + margin, y: r.y + r.h - inset, box: { x: 0, y: -h } },
      { x: r.x + r.w - margin, y: r.y + r.h - inset, box: { x: -w, y: -h } },
    ].map((c) => ({ ...c, clear: clearScale(c.x, c.y, c.box, w, h, taken, most) }));
    // The top-left corner is where a name is looked for, so it keeps the plate
    // as long as it lets it grow to a good size, and another corner takes it only
    // where that one is cramped and it is not.
    const best = Math.max(...corners.map((c) => c.clear));
    const enough = Math.min(best, FRAME_SCALE_GOOD);
    const chosen = corners.find((c) => c.clear >= enough) ?? corners[0];
    ({ x, y, box } = chosen);
    if (chosen.clear > 0) fit = chosen.clear;
  }

  return `<g class="emc-comp-label${cls ? ` ${cls}` : ''}" data-compartment-label="${esc(id)}" transform="translate(${n(x)} ${n(y)})">
    <g class="emc-frame-label" style="--fit:${fitScale(fit)}">
      <rect x="${n(box.x)}" y="${n(box.y)}" width="${n(w)}" height="${h}" rx="${h / 2}"
        fill="${s.fill}" stroke="${s.stroke}" stroke-width="${centred ? 1.2 : 0}" opacity="${centred ? 1 : 0.18}"/>
      <text x="${n(box.x + w / 2)}" y="${n(box.y + 13.5)}" text-anchor="middle" textLength="${n(textWidth)}"
        lengthAdjust="spacingAndGlyphs" fill="${s.text}" class="emc-comp-text">${esc(text)}</text>
    </g>
  </g>`;
}

function compartmentLabelPosition(compartment, radius) {
  const position = compartment.labelPosition;
  if (!Number.isFinite(position?.x) || !Number.isFinite(position?.y)) {
    return null;
  }
  const rect = compartment.rect;
  return closestPerimeterPoint(rect, {
    x: rect.x + position.x * rect.w,
    y: rect.y + position.y * rect.h,
  }, radius);
}

function renderCompartmentLabels(layout, dark, inactiveSpeciesIds) {
  const out = ['<g class="emc-layer emc-layer-compartment-labels">'];
  const environment = layout.environment;
  if (environment?.rect && !environment.hidden) {
    const taken = (layout.cells ?? []).map(cell => {
      const plate = 40 * cellPlateFit(cell, environment.rect);
      return { x: cell.rect.x, y: cell.rect.y - plate, w: cell.rect.w, h: cell.rect.h + plate };
    });
    out.push(frameLabel({
      id: environment.id ?? 's', label: environment.label, rect: environment.rect,
      style: compartmentStyle('s', dark), margin: 22, taken, cls: 'emc-env-label',
      position: compartmentLabelPosition(environment, 26),
    }));
  }
  for (const cell of layout.cells) {
    const inactive = inactiveSpeciesIds.has(cell.id);
    out.push(`<g data-cell="${esc(cell.id)}"${inactive ? ' opacity="0.32"' : ''}>`);
    const ordered = [...cell.compartments].sort((a, b) => a.depth - b.depth);
    for (const compartment of ordered) {
      out.push(renderCompartmentLabel(compartment, dark, cell));
    }
    out.push('</g>');
  }
  out.push('</g>');
  return out.join('');
}

/**
 * Compartment name plate. Rounded organelles get it straddling the top edge;
 * boxy compartments keep it in a corner, out of the pathway and clear of the
 * compartments nested inside them.
 */
function renderCompartmentLabel(comp, dark, cell) {
  const visualId = comp.visualId ?? comp.id;
  const corners = compartmentCorners(comp);
  const centred = isRoundCompartment(comp);
  const children = cell.compartments.filter((o) => o.parentId === comp.id);
  let limit = Infinity;
  if (centred) {
    // A plate astride a top edge reaches half its height into whatever is
    // above. Where two such compartments are nested closely (the matrix in
    // the intermembrane space), each plate gets half of the band between their
    // edges, or they are written over one another.
    const parent = cell.compartments.find((o) => o.id === comp.parentId);
    if (parent) limit = Math.min(limit, Math.max(0, comp.rect.y - parent.rect.y) / LABEL_HEIGHT);
    for (const child of children) {
      if (isRoundCompartment(child)) {
        limit = Math.min(limit, Math.max(0, child.rect.y - comp.rect.y) / LABEL_HEIGHT);
      }
    }
  }
  const theme = compartmentStyle(visualId, dark);
  return frameLabel({
    id: comp.id,
    label: comp.label,
    rect: comp.rect,
    // The name plate keeps the colours the compartment is drawn in.
    style: { ...theme, fill: comp.fill || theme.fill, stroke: comp.stroke || theme.stroke },
    margin: Math.max(14, Math.max(corners.rx, corners.ry) * 0.45),
    centred,
    taken: children.map((o) => o.rect),
    limit,
    position: compartmentLabelPosition(comp, corners),
  });
}

// Pathway ribbons

/**
 * Omix-style pathway ribbons. They reuse the exact edited reaction geometry,
 * but are drawn as broad translucent strokes underneath the real reaction
 * layer. This keeps flux colours, metabolite colours, arrowheads and editing
 * semantics untouched.
 */
function renderPathwayRibbons(layout, mode, colors, groups, width = 56, opacity = 0.18, showCurrency = true) {
  if (mode === 'off' || !groups?.size) return '';

  const ribbonWidth = Math.min(120, Math.max(20, Number(width) || 56));
  const alpha = Math.min(0.45, Math.max(0.04, Number(opacity) || 0.18));
  const outerWidth = ribbonWidth * 1.34;
  const innerOpacity = alpha;
  const outerOpacity = alpha * 0.34;
  const out = ['<g class="emc-layer emc-layer-pathway-ribbons" pointer-events="none">'];

  for (const seg of layout.segments) {
    if (seg.kind === 'currency') continue;
    const rxn = layout.reactions.get(seg.reactionId);
    if (!rxn) continue;
    const group = groups.get(seg.reactionId);
    if (!group || /^Sin (subsystem|pathway)$/i.test(group)) continue;
    const targetNode = seg.nodeId ? layout.nodes.get(seg.nodeId) : null;
    if (targetNode?.hidden) continue;
    if (!showCurrency && isSecondaryNode(targetNode)) continue;

    const color = colors.get(seg.reactionId);
    if (!color) continue;
    const d = curvePath(segmentCurve(seg));
    const attrs = `data-ribbon-rxn="${esc(seg.reactionId)}" data-ribbon-group="${esc(group)}"`;

    out.push(`<path class="emc-pathway-ribbon emc-pathway-ribbon-outer" ${attrs}
      d="${d}" fill="none" stroke="${color}" stroke-width="${n(outerWidth)}"
      stroke-linecap="round" stroke-linejoin="round" opacity="${n(outerOpacity)}"
      filter="url(#emc-ribbon-soft)"/>`);
    out.push(`<path class="emc-pathway-ribbon emc-pathway-ribbon-core" ${attrs}
      d="${d}" fill="none" stroke="${color}" stroke-width="${n(ribbonWidth)}"
      stroke-linecap="round" stroke-linejoin="round" opacity="${n(innerOpacity)}"/>`);
  }

  out.push('</g>');
  return out.join('');
}


function activeCrossFeedingReactionIds(explorer, enabled) {
  const ids = new Set();
  if (!enabled) return ids;
  for (const event of explorer?.visibleEvents ?? []) {
    if (event.producerTransfer) ids.add(event.producerTransfer);
    if (event.consumerTransfer) ids.add(event.consumerTransfer);
  }
  return ids;
}

// Segments

/**
 * A reaction's colour. An omics overlay wins, then flux colouring when lines
 * are coloured by flux, otherwise the colour set on the reaction itself, then
 * its pathway's or subsystem's.
 */
function reactionDisplayColor(
  reactionId, flux, scale, mode, reactionColors,
  omicsEnabled = false, omicsColors = new Map(), omicsFallbackColor = null, ownColor = null,
) {
  if (omicsEnabled) {
    return omicsColors?.get?.(reactionId)
      ?? omicsFallbackColor
      ?? scale.color(flux);
  }
  return mode === 'flux'
    ? scale.color(flux)
    : (ownColor || (reactionColors?.get?.(reactionId) ?? scale.color(flux)));
}

const ESCHER_REACTION_STROKE_WIDTH = 7.5;

/*
 * A reaction that carries no flux in the solution, drawn the way a switched-off
 * reaction is in a topology figure: grey and dashed, so what the solution uses
 * stands out from what it does not.
 */
const OFF_COLOR = { light: '#9b9da5', dark: '#64748b' };
const offColor = (dark) => (dark ? OFF_COLOR.dark : OFF_COLOR.light);
const offDash = (width) => `${n(Math.max(6, width * 1.5))} ${n(Math.max(5, width * 1.15))}`;

/** Whether a reaction is drawn off: a solution exists and it carries nothing in it. */
function isOffReaction(rid, { offStyle = false, solvedFluxes = new Map(), disabledReactionIds = new Set(), inactive = false }) {
  if (!offStyle || inactive || disabledReactionIds.has(rid)) return false;
  return Math.abs(solvedFluxes.get(rid) ?? 0) <= 1e-9;
}

function renderSegments(
  layout, fluxes, scale, selected, showCurrency, multiSelection,
  reactionColorMode, reactionColors, omicsEnabled, omicsColors, omicsFallbackColor,
  crossFeedingReactions = new Set(), highlightCrossFeeding = false,
  {
    solvedFluxes = fluxes, reactionWidthMode = 'constant', disabledReactionIds = new Set(), inactiveSpeciesIds = new Set(),
    offStyle = false, lineHops = false, dark = false, tooltip = null,
  } = {},
) {
  const out = ['<g class="emc-layer emc-layer-segments">'];
  const arrows = [];
  // Segments are collected before they are written: a line that hops over
  // another needs to know where the others run.
  const drawn = [];

  for (const seg of layout.segments) {
    const rxn = layout.reactions.get(seg.reactionId);
    if (!rxn) continue;
    const targetNode = seg.nodeId ? layout.nodes.get(seg.nodeId) : null;
    if (!showCurrency && (seg.kind === 'currency' || isSecondaryNode(targetNode))) continue;
    // A lane is the stretch of a transported metabolite's path that lies in the
    // transporter, and the transporter's own glyph draws it, as an arrow. Drawn
    // here as well it would be a stub of full line width poking out of both
    // sides of the glyph, doubling up with the end of the curve that meets it.
    if (seg.kind === 'transport-lane' && (rxn.transportLanes?.length ?? 0) >= 2) continue;
    if (targetNode?.hidden) continue;

    const v = fluxes.get(seg.reactionId) ?? 0;
    const disabled = disabledReactionIds.has(seg.reactionId);
    const inactive = inactiveSpeciesIds.has(rxn.cellId);
    const solved = disabled || inactive ? 0 : (solvedFluxes.get(seg.reactionId) ?? 0);
    const width = reactionWidthMode === 'flux' ? scale.width(v) : ESCHER_REACTION_STROKE_WIDTH;
    const off = isOffReaction(seg.reactionId, { offStyle, solvedFluxes, disabledReactionIds, inactive });
    const color = disabled ? '#db2777' : off ? offColor(dark) : reactionDisplayColor(
      seg.reactionId, v, scale, reactionColorMode, reactionColors,
      omicsEnabled, omicsColors, omicsFallbackColor, rxn.color,
    );
    const active = Math.abs(solved) > 1e-9;
    const isSel = selected === seg.reactionId;
    const isMulti = multiSelection?.reactions?.has?.(seg.reactionId) ?? false;

    const isCrossFeed = crossFeedingReactions.has(seg.reactionId);
    const crossFeedOpacity = inactive ? 0.22 : (highlightCrossFeeding && !isSel && !isMulti ? (isCrossFeed ? 1 : 0.14) : 1);
    const cls = `emc-seg emc-seg-${seg.kind}${disabled ? ' is-disabled-reaction' : ''}${off ? ' is-off' : ''}${inactive ? ' is-inactive-species' : ''}${active ? ' is-active' : ''}${isCrossFeed ? ' is-crossfeeding' : ''}${isSel ? ' is-selected' : ''}${isMulti ? ' is-multi-selected' : ''}`;
    // Arrowheads follow the sign of the flux, so a reversed reaction reads as
    // reversed on the map instead of only in the numbers.
    const forward = solved >= 0;
    const wantArrow = seg.side === (forward ? 'product' : 'substrate')
      || (seg.kind === 'boundary' && forward);
    const geometry = segmentGeometry(seg, rxn, targetNode, !disabled && !inactive && wantArrow && (active || seg.kind === 'boundary'), width);
    if (geometry.d) {
      const dash = disabled ? '8 5' : off ? offDash(width) : null;
      drawn.push({
        seg, d: geometry.d, curve: geometry.curve, reactionId: seg.reactionId, width,
        // Cofactor spurs are short and many; a hop on one, or over one, says nothing.
        hops: seg.kind !== 'currency' && !isSecondaryNode(targetNode),
        attrs: `class="${cls}" fill="none" stroke="${color}" `
          + `stroke-width="${n(width)}" stroke-linecap="${targetNode || geometry.arrow ? 'butt' : 'round'}" opacity="${n(crossFeedOpacity)}" `
          + `${dash ? `stroke-dasharray="${dash}" ` : ''}data-rxn="${esc(seg.reactionId)}"${seg.nodeId ? ` data-node="${esc(seg.nodeId)}"` : ''}`,
      });
    }
    if (geometry.arrow) {
      const points = geometry.arrow.points.map((p) => `${n(p.x)},${n(p.y)}`).join(' ');
      arrows.push(`<polygon class="emc-arrow" points="${points}" fill="${color}" opacity="${n(crossFeedOpacity)}" `
        + `data-rxn="${esc(seg.reactionId)}"${seg.nodeId ? ` data-node="${esc(seg.nodeId)}"` : ''}/>`);
    }
  }

  const hopping = lineHops ? drawn.filter((item) => item.hops) : [];
  const hops = lineHops ? findLineHops(hopping) : new Map();
  const hopsOf = new Map([...hops].map(([index, list]) => [hopping[index], list]));
  drawn.forEach((item) => {
    const d = hopsOf.has(item) ? hoppedPath(item.curve, hopsOf.get(item)) : item.d;
    const title = tooltip ? `<title>${esc(tooltip(item.reactionId))}</title>` : '';
    out.push(title
      ? `<path ${item.attrs} d="${d}">${title}</path>`
      : `<path ${item.attrs} d="${d}"/>`);
  });

  out.push(arrows.join(''));
  out.push('</g>');
  return out.join('');
}

function segmentGeometry(seg, rxn, node, arrow, width) {
  if (node && seg.kind !== 'boundary' && seg.kind !== 'transport-lane' && seg.kind !== 'axis') {
    const { b1, b2 } = branchControls(seg);
    const metabolite = seg.side === 'substrate' ? seg.from : seg.to;
    const reaction = seg.side === 'substrate' ? seg.to : seg.from;
    return reactionEndpointGeometry([reaction, b2, b1, metabolite], { node, arrow, width });
  }
  if (arrow || node) {
    return reactionEndpointGeometry([seg.from, seg.to], { node, arrow, width });
  }
  const curve = segmentCurve(seg);
  return { d: curvePath(curve), arrow: null, curve };
}

/** The line or cubic a segment without a node or arrowhead is drawn as. */
function segmentCurve(seg) {
  const { from, to } = seg;
  if (seg.kind === 'axis' || seg.kind === 'boundary' || seg.kind === 'transport-lane') return [from, to];

  const { b1, b2 } = branchControls(seg);
  // metabolite -> reaction backbone
  if (seg.side === 'substrate') return [from, b1, b2, to];
  // reaction backbone -> metabolite; semantic b2 is reaction-side, b1 metabolite-side.
  return [from, b2, b1, to];
}

function curvePath(curve) {
  const [from] = curve;
  const to = curve.at(-1);
  if (curve.length === 2) return `M ${n(from.x)} ${n(from.y)} L ${n(to.x)} ${n(to.y)}`;
  const [, c1, c2] = curve;
  return `M ${n(from.x)} ${n(from.y)} C ${n(c1.x)} ${n(c1.y)}, ${n(c2.x)} ${n(c2.y)}, ${n(to.x)} ${n(to.y)}`;
}

function branchControls(seg) {
  const metabolite = seg.side === 'substrate' ? seg.from : seg.to;
  const reaction = seg.side === 'substrate' ? seg.to : seg.from;

  if (seg.b1 && seg.b2) return { b1: seg.b1, b2: seg.b2 };

  if (seg.control) {
    const q = seg.control;
    return {
      b1: {
        x: metabolite.x + (2 / 3) * (q.x - metabolite.x),
        y: metabolite.y + (2 / 3) * (q.y - metabolite.y),
      },
      b2: {
        x: reaction.x + (2 / 3) * (q.x - reaction.x),
        y: reaction.y + (2 / 3) * (q.y - reaction.y),
      },
    };
  }

  return {
    b1: {
      x: metabolite.x + (reaction.x - metabolite.x) * 0.34,
      y: metabolite.y + (reaction.y - metabolite.y) * 0.34,
    },
    b2: {
      x: metabolite.x + (reaction.x - metabolite.x) * 0.72,
      y: metabolite.y + (reaction.y - metabolite.y) * 0.72,
    },
  };
}

/**
 * The area a multi-selection covers, shaded.
 *
 * Outlining each selected mark says which ones are in, but not what region
 * they make up, which is what matters when the next action moves,
 * aligns or deletes them as one. The shading is the group, and it is drawn
 * beneath everything so no mark is tinted by it.
 */
function multiSelectionBounds(layout, multiSelection) {
  const xs = [];
  const ys = [];
  for (const id of multiSelection?.nodes ?? []) {
    const node = layout.nodes.get(id);
    if (node) {
      xs.push(node.x);
      ys.push(node.y);
    }
  }
  for (const id of multiSelection?.reactions ?? []) {
    const rxn = layout.reactions.get(id);
    if (rxn) {
      xs.push(rxn.x);
      ys.push(rxn.y);
    }
  }
  for (const id of multiSelection?.texts ?? []) {
    const annotation = layout.texts?.get(id);
    if (!annotation) {
      continue;
    }
    const box = textBox(annotation);
    xs.push(box.left, box.right);
    ys.push(box.top, box.bottom);
  }
  for (const r of selectedCompartmentRects(layout, multiSelection)) {
    xs.push(r.x, r.x + r.w);
    ys.push(r.y, r.y + r.h);
  }
  // One object is already shown as selected. A region around it says nothing.
  if (xs.length < 2) {
    return null;
  }

  const pad = 30;
  const x = Math.min(...xs) - pad;
  const y = Math.min(...ys) - pad;
  const w = (Math.max(...xs) - Math.min(...xs)) + pad * 2;
  const h = (Math.max(...ys) - Math.min(...ys)) + pad * 2;
  return { x, y, w, h };
}

/* Where the compartments in a multi-selection are, in map order. */
function selectedCompartmentRects(layout, multiSelection) {
  const chosen = multiSelection?.compartments;
  if (!chosen?.size) return [];
  const out = [];
  for (const cell of layout.cells ?? []) {
    for (const comp of cell.compartments ?? []) {
      if (comp.rect && chosen.has(compartmentKey(cell.id, comp.id))) out.push(comp.rect);
    }
  }
  return out;
}

/*
 * The shaded area, and a dashed outline round each selected compartment: those
 * move with the selection, membrane and all, and should look it.
 */
function renderMultiSelectionArea(bounds, compartments = []) {
  if (!bounds) {
    return '';
  }
  const { x, y, w, h } = bounds;
  const outlines = compartments.map((r) => `
    <rect class="emc-multi-compartment" x="${n(r.x - 4)}" y="${n(r.y - 4)}" width="${n(r.w + 8)}" height="${n(r.h + 8)}" rx="18" fill="none" stroke="#2563eb" stroke-width="3" stroke-dasharray="10 6"/>`).join('');
  return `<g class="emc-layer emc-layer-multi-area" pointer-events="none">
    <rect class="emc-multi-area" x="${n(x)}" y="${n(y)}" width="${n(w)}" height="${n(h)}" rx="18"/>${outlines}
  </g>`;
}

function renderMultiSelectionBorder(bounds, minHitRadius = 0) {
  if (!bounds) {
    return '';
  }
  const { x, y, w, h } = bounds;
  const strokeWidth = Math.max(12, minHitRadius);
  return `<g class="emc-layer emc-layer-multi-border" data-selection-drag>
    <title>Drag the border to move the selection</title>
    <rect class="emc-multi-drag-hit" x="${n(x)}" y="${n(y)}" width="${n(w)}" height="${n(h)}" rx="18" stroke-width="${n(strokeWidth)}"/>
    <rect class="emc-multi-drag-outline" x="${n(x)}" y="${n(y)}" width="${n(w)}" height="${n(h)}" rx="18"/>
  </g>`;
}

// Markers

function renderMarkers(
  layout, fluxes, scale, selected, dark, multiSelection,
  reactionColorMode, reactionColors, omicsEnabled, omicsColors, omicsFallbackColor,
  {
    solvedFluxes = fluxes, disabledReactionIds = new Set(), modifiedReactionIds = new Set(),
    inactiveSpeciesIds = new Set(), offStyle = false, tooltip = null,
  } = {},
) {
  const out = ['<g class="emc-layer emc-layer-markers">'];

  // A hand-placed transporter records the compartment whose membrane it is on,
  // not what that compartment is drawn as. The cell knows (`i` is drawn as the
  // intermembrane space, a cell's own `s` as the eyespot).
  const drawnAs = new Map();
  for (const cell of layout.cells ?? []) {
    for (const comp of cell.compartments ?? []) {
      drawnAs.set(`${cell.id}/${comp.id}`, comp.visualId ?? comp.id);
    }
  }

  for (const [rid, rxn] of layout.reactions) {
    const v = fluxes.get(rid) ?? 0;
    const disabled = disabledReactionIds.has(rid);
    const inactive = inactiveSpeciesIds.has(rxn.cellId);
    const solved = disabled || inactive ? 0 : (solvedFluxes.get(rid) ?? 0);
    const off = isOffReaction(rid, { offStyle, solvedFluxes, disabledReactionIds, inactive });
    const color = disabled ? '#db2777' : off ? offColor(dark) : reactionDisplayColor(
      rid, v, scale, reactionColorMode, reactionColors,
      omicsEnabled, omicsColors, omicsFallbackColor, rxn.color,
    );
    const isSel = selected === rid;
    const isMulti = multiSelection?.reactions?.has?.(rid) ?? false;
    const angle = rxn.axis ? (Math.atan2(rxn.axis.dir.y, rxn.axis.dir.x) * 180) / Math.PI : 0;
    const title = tooltip ? `<title>${esc(tooltip(rid))}</title>` : '';
    const cls = `emc-marker${disabled ? ' is-disabled-reaction' : ''}${off ? ' is-off' : ''}${inactive ? ' is-inactive-species' : ''}${isSel ? ' is-selected' : ''}${isMulti ? ' is-multi-selected' : ''}${Math.abs(solved) > 1e-9 ? ' is-active' : ''}`;
    if (inactive) out.push('<g opacity="0.22">');

    if (rxn.onMembrane) {
      // Multi-solute transporters get one visible lane per transported base.
      // Symport lanes point together, antiport lanes in opposite directions.
      const s = compartmentStyle(
        rxn.membraneVisualId ?? drawnAs.get(`${rxn.cellId}/${rxn.membrane}`) ?? rxn.membrane ?? 'c', dark,
      );
      if ((rxn.transportLanes?.length ?? 0) >= 2) {
        /*
         * One arrow per transported metabolite, drawn where its lane is.
         *
         * The glyph is laid out in the frame of the membrane, not of the
         * reaction's axis: +x points out of the compartment the transporter
         * sits on, which is what a lane's direction is recorded against. The
         * axis may point either way across the membrane, and read against it a
         * symporter drawn the other way round would show both arrows backwards.
         */
        const lanes = rxn.transportLanes;
        const out_ = rxn.normal ?? rxn.axis?.dir ?? { x: 1, y: 0 };
        const frame = (Math.atan2(out_.y, out_.x) * 180) / Math.PI;
        const offsets = lanes.map((lane, i) => (Number.isFinite(lane.offset)
          ? lane.offset
          : (i - (lanes.length - 1) / 2) * 12));
        const h = Math.max(...offsets) - Math.min(...offsets) + 22;
        const laneSvg = lanes.map((lane, i) =>
          laneArrow(offsets[i], disabled || inactive ? 0 : (solved >= 0 ? 1 : -1) * (lane.direction || 0), color)).join('');
        out.push(`<g class="${cls} emc-transporter emc-transporter-${esc(rxn.transportMode)}" data-rxn="${esc(rid)}" `
          + `transform="translate(${n(rxn.x)} ${n(rxn.y)}) rotate(${n(frame)})">
          ${title}<rect x="-13" y="${n(-h / 2)}" width="26" height="${n(h)}" rx="8" fill="${s.fill}" stroke="${s.stroke}" stroke-width="2.4"/>
          ${laneSvg}
        </g>`);
      } else {
        out.push(`<g class="${cls} emc-transporter" data-rxn="${esc(rid)}" `
          + `transform="translate(${n(rxn.x)} ${n(rxn.y)}) rotate(${n(angle)})">
          ${title}<rect x="-13" y="-11" width="26" height="22" rx="7" fill="${s.fill}" stroke="${s.stroke}" stroke-width="2.4"/>
          <rect x="-4.5" y="-11" width="9" height="22" rx="4" fill="${color}" opacity="0.85"/>
        </g>`);
      }
    } else {
      out.push(`<g class="${cls}" data-rxn="${esc(rid)}" `
        + `transform="translate(${n(rxn.x)} ${n(rxn.y)}) rotate(${n(angle + 90)})">
        ${title}<rect x="-9" y="-3" width="18" height="6" rx="3" fill="${color}"/>
        <rect x="-9" y="-3" width="18" height="6" rx="3" fill="none" stroke="${dark ? '#0f172a' : '#ffffff'}" stroke-width="1.1" opacity="0.8"/>
      </g>`);
    }
    if (disabled) {
      out.push(`<g class="emc-ko-mark" data-disabled-reaction="${esc(rid)}" transform="translate(${n(rxn.x)} ${n(rxn.y)})" stroke="#db2777" stroke-width="3" pointer-events="none">
        <title>Disabled reaction: both bounds fixed to zero</title><path d="M -9 -9 L 9 9 M -9 9 L 9 -9"/></g>`);
    } else if (modifiedReactionIds.has(rid)) {
      // A small amber dot off the marker's corner: the bounds are not the model's.
      out.push(`<g class="emc-modified-badge" data-modified-reaction="${esc(rid)}" transform="translate(${n(rxn.x + 13)} ${n(rxn.y - 13)})" pointer-events="none">
        <title>Bounds changed from the model's</title><circle r="5.5" fill="${dark ? '#fbbf24' : '#d97706'}" stroke="${dark ? '#0f172a' : '#ffffff'}" stroke-width="1.6"/></g>`);
    }
    if (inactive) out.push('</g>');
    if (isSel || isMulti) {
      out.push(`<circle class="emc-select-ring${isMulti ? ' is-multi' : ''}" cx="${n(rxn.x)}" cy="${n(rxn.y)}" r="21" `
        + `fill="none" stroke="${isMulti ? '#2563eb' : color}" stroke-width="2.5" stroke-dasharray="4 3"/>`);
    }
  }

  out.push('</g>');
  return out.join('');
}

/**
 * One lane of a transporter: an ordinary arrow, a thin shaft with a head clearly
 * wider than it. A thick bar with a head barely wider than itself reads as a rod
 * with a blunt tip rather than as a direction.
 *
 * @param {number} y     where the lane lies across the glyph
 * @param {number} sign  +1 out of the compartment, -1 into it, 0 if not known
 */
function laneArrow(y, sign, color) {
  const half = 9.5;
  if (!sign) {
    return `<line class="emc-lane" x1="${-half}" y1="${n(y)}" x2="${half}" y2="${n(y)}" `
      + `stroke="${color}" stroke-width="2.6" stroke-linecap="round"/>`;
  }
  const tip = half * sign;
  const neck = tip - 7.5 * sign;
  return `<line class="emc-lane" x1="${n(-half * sign)}" y1="${n(y)}" x2="${n(neck + sign)}" y2="${n(y)}" `
    + `stroke="${color}" stroke-width="2.6" stroke-linecap="round"/>`
    + `<polygon class="emc-lane-head" data-lane-direction="${sign > 0 ? 'out' : 'in'}" `
    + `points="${n(tip)},${n(y)} ${n(neck)},${n(y - 4.6)} ${n(neck)},${n(y + 4.6)}" fill="${color}"/>`;
}

// Hit targets

/**
 * Invisible, constant-size pointer targets.
 *
 * A reaction marker is about 26×22 map units across and a metabolite 26. At the
 * zoom the map opens on (28% on a large screen, 19% on a laptop) that is 3 to
 * 7 screen pixels, far under the ~24 px a pointer can reliably hit, so the first
 * thing a new user tries (click a reaction) fails silently. These circles sit
 * *under* the artwork, so the glyphs keep their own hover styling, and they
 * carry the same data- attributes, so a click near a node or marker still
 * resolves to it.
 *
 * `data-hit-base` is the glyph's own radius and acts as a floor. `applyCamera()`
 * in the app rewrites `r` from the live zoom so the halo stays a constant size
 * on screen: wide enough to hit when zoomed out, never smaller than the glyph
 * when zoomed in.
 */
function renderHitTargets(layout, showCurrency, minHitRadius = 0) {
  const out = ['<g class="emc-layer emc-layer-hit" aria-hidden="true">'];

  for (const [rid, rxn] of layout.reactions) {
    const base = rxn.onMembrane ? 15 : 11;
    out.push(`<circle class="emc-hit emc-hit-reaction" data-rxn="${esc(rid)}" `
      + `data-hit-base="${base}" cx="${n(rxn.x)}" cy="${n(rxn.y)}" r="${n(Math.max(base, minHitRadius))}"/>`);
  }

  for (const node of layout.nodes.values()) {
    if (node.hidden) continue;
    if (!showCurrency && isSecondaryNode(node)) continue;
    if (node.compartment === LINK_COMPARTMENT) continue;
    const base = Math.max(node.r, 6);
    out.push(`<circle class="emc-hit emc-hit-node" data-node="${esc(node.id)}" `
      + `data-met="${esc(node.mergedId)}" data-hit-base="${n(base)}" `
      + `cx="${n(node.x)}" cy="${n(node.y)}" r="${n(Math.max(base, minHitRadius))}"/>`);
  }

  out.push('</g>');
  return out.join('');
}

// Nodes

const METABOLITE_LIKE = new Set(['metabolite', 'medium', 'currency']);

// The Cofactors toggles act on every secondary node (isSecondaryNode): a
// hand-built map draws its cofactors as instances set to Secondary, so a toggle
// that knew only the layout's own helper nodes would leave all of them on screen.

function renderNodes(layout, dark, showCurrency, multiSelection, inactiveSpeciesIds = new Set(), tooltip = null) {
  const out = ['<g class="emc-layer emc-layer-nodes">'];
  // A colour chosen for the map, or for one metabolite, holds in both themes.
  const orange = layout.nodeStyle?.fill || (dark ? '#f59e0b' : '#e67e4c');
  const orangeBorder = layout.nodeStyle?.stroke || (dark ? '#7c2d12' : '#a64613');
  const fallbackStroke = dark ? '#0f172a' : '#ffffff';

  for (const node of layout.nodes.values()) {
    if (node.hidden) continue;
    if (!showCurrency && isSecondaryNode(node)) continue;
    if (node.compartment === LINK_COMPARTMENT) continue;

    const metaboliteLike = METABOLITE_LIKE.has(node.kind);
    const secondary = isSecondaryNode(node);
    const isMulti = multiSelection?.nodes?.has?.(node.id) ?? false;
    const roleClass = metaboliteLike
      ? (secondary ? ' emc-node-secondary' : ' emc-node-primary')
      : '';
    const inactive = inactiveSpeciesIds.has(node.cellId);
    const cls = `emc-node emc-node-${node.kind}${roleClass}${inactive ? ' is-inactive-species' : ''}${isMulti ? ' is-multi-selected' : ''}`;
    if (inactive) out.push('<g opacity="0.28">');

    if (metaboliteLike) {
      const radius = secondary ? SIZES.currencyRadius : SIZES.primaryRadius;
      const title = tooltip ? `<title>${esc(tooltip(node.id))}</title>` : '';
      out.push(`<circle class="${cls}" data-node="${esc(node.id)}" data-met="${esc(node.mergedId)}" `
        + `cx="${n(node.x)}" cy="${n(node.y)}" r="${n(radius)}" fill="${esc(node.fill || orange)}" `
        + `stroke="${esc(node.stroke || orangeBorder)}" stroke-width="1.8"${title ? `>${title}</circle>` : '/>'}`);
    } else {
      const style = compartmentStyle(node.visualCompartment ?? node.compartment, dark);
      out.push(`<circle class="${cls}" data-node="${esc(node.id)}" data-met="${esc(node.mergedId)}" `
        + `cx="${n(node.x)}" cy="${n(node.y)}" r="${n(node.r)}" fill="${style.stroke}" `
        + `stroke="${fallbackStroke}" stroke-width="2.4"/>`);
    }
    if (inactive) out.push('</g>');
  }

  out.push('</g>');
  return out.join('');
}

// Edit handles

/**
 * How far from a reaction's centre its rotation handle stands, in map units.
 *
 * Far enough to be told apart from the centre handle it turns about, at any
 * zoom: the handles are kept a constant size on screen, so the gap between two
 * of them has to be kept in screen terms as well. The app recomputes it when
 * the zoom changes without a repaint.
 */
export function rotationArm(minHitRadius = 0) {
  return Math.min(170, Math.max(30, (minHitRadius || 0) * 2.7));
}

/**
 * The handles of the Edit Control Points tool, for every drawn reaction.
 *
 * They belong to that tool alone. Putting them up on selection as well would
 * cover the marker with targets at exactly the moment someone is reaching for
 * it, to drag it or just to read it in the inspector.
 *
 * Per reaction: the centre, to move it; one lever to turn its backbone; and
 * per branch the two Bézier handles, blue at the metabolite end and red at the
 * reaction end. The points where the branches meet the backbone are not
 * handles: they stay on the backbone and turn with it.
 */
function renderEditHandles(layout, dark, editAll = false, minHitRadius = 0) {
  if (!editAll || !layout.reactions.size) return '';

  const centreStroke = dark ? '#f8fafc' : '#0f172a';
  const centreFill = dark ? '#0f172a' : '#ffffff';
  const blue = '#2563eb';
  const red = '#dc2626';
  const green = '#16a34a';
  const arm = rotationArm(minHitRadius);
  const out = ['<g class="emc-layer emc-layer-edit-handles">'];

  const branches = new Map();
  for (const seg of layout.segments) {
    if (!seg.nodeId) continue;
    if (seg.kind === 'axis' || seg.kind === 'boundary' || seg.kind === 'transport-lane') continue;
    if (!branches.has(seg.reactionId)) branches.set(seg.reactionId, []);
    branches.get(seg.reactionId).push(seg);
  }

  for (const [reactionId, rxn] of layout.reactions) {
    out.push(`<circle class="emc-reaction-handle" data-reaction-handle="${esc(reactionId)}"
      data-edit-handle-base="7.5"
      cx="${n(rxn.x)}" cy="${n(rxn.y)}" r="7.5" fill="${centreFill}" stroke="${centreStroke}" stroke-width="2"/>`);

    // A straight reaction lies along the line between two metabolites, so
    // neither its direction nor the two branches on that line are free.
    const straight = rxn.linear && layout.nodes.has(rxn.linear.from) && layout.nodes.has(rxn.linear.to)
      ? rxn.linear : null;

    if (rxn.axis && !rxn.onMembrane && !straight) {
      // The lever stands across the backbone, where no branch leaves it.
      const ux = -rxn.axis.dir.y;
      const uy = rxn.axis.dir.x;
      const hx = rxn.x + ux * arm;
      const hy = rxn.y + uy * arm;
      const attrs = `data-rotate-ox="${n(rxn.x)}" data-rotate-oy="${n(rxn.y)}" `
        + `data-rotate-ux="${ux.toFixed(5)}" data-rotate-uy="${uy.toFixed(5)}"`;
      out.push(`<line class="emc-control-guide emc-rotate-guide" data-rotate-guide="${esc(reactionId)}" ${attrs}
        x1="${n(rxn.x)}" y1="${n(rxn.y)}" x2="${n(hx)}" y2="${n(hy)}" stroke="${green}"/>`);
      out.push(`<circle class="emc-control-handle emc-rotate-handle" data-rotate-handle="${esc(reactionId)}" ${attrs}
        data-edit-handle-base="5.5" cx="${n(hx)}" cy="${n(hy)}" r="5.5"
        fill="${centreFill}" stroke="${green}" stroke-width="1.8"
        ><title>Drag to rotate the reaction marker</title></circle>`);
    }

    for (const seg of branches.get(reactionId) ?? []) {
      if (layout.nodes.get(seg.nodeId)?.hidden) continue;
      if (straight && (seg.nodeId === straight.from || seg.nodeId === straight.to)) continue;

      const { b1, b2 } = branchControls(seg);
      const metabolite = seg.side === 'substrate' ? seg.from : seg.to;
      const reactionAnchor = seg.side === 'substrate' ? seg.to : seg.from;
      const hitR = 12;

      out.push(`<line class="emc-control-guide emc-control-guide-metabolite"
        x1="${n(metabolite.x)}" y1="${n(metabolite.y)}"
        x2="${n(b1.x)}" y2="${n(b1.y)}" stroke="${blue}"/>`);
      out.push(`<circle class="emc-control-hit"
        data-control-rxn="${esc(reactionId)}" data-control-node="${esc(seg.nodeId)}"
        data-control-handle="b1" cx="${n(b1.x)}" cy="${n(b1.y)}" r="${hitR}"/>`);
      out.push(`<circle class="emc-control-handle emc-control-b1"
        data-control-rxn="${esc(reactionId)}" data-control-node="${esc(seg.nodeId)}"
        data-control-handle="b1" data-edit-handle-base="5.5"
        cx="${n(b1.x)}" cy="${n(b1.y)}" r="5.5"
        fill="${centreFill}" stroke="${blue}" stroke-width="1.8"/>`);

      out.push(`<line class="emc-control-guide emc-control-guide-reaction"
        x1="${n(reactionAnchor.x)}" y1="${n(reactionAnchor.y)}"
        x2="${n(b2.x)}" y2="${n(b2.y)}" stroke="${red}"/>`);
      out.push(`<circle class="emc-control-hit"
        data-control-rxn="${esc(reactionId)}" data-control-node="${esc(seg.nodeId)}"
        data-control-handle="b2" cx="${n(b2.x)}" cy="${n(b2.y)}" r="${hitR}"/>`);
      out.push(`<circle class="emc-control-handle emc-control-b2"
        data-control-rxn="${esc(reactionId)}" data-control-node="${esc(seg.nodeId)}"
        data-control-handle="b2" data-edit-handle-base="5.5"
        cx="${n(b2.x)}" cy="${n(b2.y)}" r="5.5"
        fill="${centreFill}" stroke="${red}" stroke-width="1.8"/>`);
    }
  }

  out.push('</g>');
  return out.join('');
}

// Map annotations

function renderMapAnnotations(layout, dark, multiSelection = null) {
  if (!layout.texts?.size) return '';
  const out = ['<g class="emc-layer emc-layer-map-text">'];
  const halo = dark ? '#0b1220' : '#ffffff';
  const fill = dark ? '#f8fafc' : '#111827';

  for (const annotation of layout.texts.values()) {
    // A selected annotation has to read as selected, the way a selected node
    // does, or a rectangle that caught it gives no sign it did.
    const isMulti = multiSelection?.texts?.has?.(annotation.id) ?? false;
    if (isMulti) {
      const box = textBox(annotation);
      out.push(`<rect class="emc-map-text-selected" x="${n(box.left - 4)}" y="${n(box.top - 2)}"
        width="${n(box.right - box.left + 8)}" height="${n(box.bottom - box.top + 6)}" rx="4"/>`);
    }
    // A chosen font goes in the element's own style: the stylesheet sets the
    // default font on .emc-map-text, and would win over a font-family attribute.
    const font = annotation.fontFamily && annotation.fontFamily !== DEFAULT_TEXT_FONT
      ? ` style="font-family: ${esc(textFontStack(annotation.fontFamily))}"`
      : '';
    out.push(`<text class="emc-map-text${isMulti ? ' is-multi' : ''}" data-map-text="${esc(annotation.id)}"${font}
      x="${n(annotation.x)}" y="${n(annotation.y)}"
      font-size="${n(annotation.fontSize ?? 22)}"
      text-anchor="start" dominant-baseline="alphabetic"
      stroke="${halo}" stroke-width="3.5" stroke-linejoin="round" paint-order="stroke"
      fill="${fill}">${esc(annotation.text)}</text>`);
  }

  out.push('</g>');
  return out.join('');
}

// Text

/** Metabolite nodes drawn on at least one reaction that carries flux. */
function activeNodeIds(layout, solvedFluxes, disabledReactionIds, inactiveSpeciesIds) {
  const ids = new Set();
  for (const seg of layout.segments) {
    if (!seg.nodeId || ids.has(seg.nodeId)) continue;
    const rxn = layout.reactions.get(seg.reactionId);
    if (!rxn || disabledReactionIds.has(seg.reactionId) || inactiveSpeciesIds.has(rxn.cellId)) continue;
    if (Math.abs(solvedFluxes.get(seg.reactionId) ?? 0) > 1e-9) ids.add(seg.nodeId);
  }
  return ids;
}

function renderText(
  layout, fluxes, scale, dark, labels, showCurrency, currencyLabels,
  fvaEnabled = false, fvaRanges = new Map(), fvaProgress = null,
  { solvedFluxes = fluxes, disabledReactionIds = new Set(), inactiveSpeciesIds = new Set(), emphasizedReactionIds = new Set(), metaboliteLabels = new Map(), reactionLabels = new Map(), offStyle = false } = {},
) {
  const out = ['<g class="emc-layer emc-layer-text">'];
  const halo = dark ? '#0b1220' : '#ffffff';

  // "None" means none: metabolite names go too, not only reaction labels.
  // "All active" names what carries flux: a metabolite is named when one of
  // the reactions drawn to it is active, as a reaction is named when it is.
  const activeNodes = labels === 'active' && !fvaEnabled
    ? activeNodeIds(layout, solvedFluxes, disabledReactionIds, inactiveSpeciesIds)
    : null;
  for (const node of labels === 'none' ? [] : layout.nodes.values()) {
    if (node.hidden) continue;
    if (node.compartment === LINK_COMPARTMENT) continue;
    if (activeNodes && !activeNodes.has(node.id)) continue;
    const ghostClass = inactiveSpeciesIds.has(node.cellId) ? ' is-inactive-species' : '';
    if (isSecondaryNode(node) && (!showCurrency || !currencyLabels)) continue;
    if (node.kind === 'currency') {
      out.push(metaboliteLabel(node, `emc-label-currency${ghostClass}`, halo, dark, metaboliteLabels.get(node.id)));
    } else {
      const role = node.primary === false ? ' emc-label-secondary' : ' emc-label-primary';
      const cls = (node.kind === 'medium' ? 'emc-label-medium' : 'emc-label-met') + role;
      out.push(metaboliteLabel(node, cls + ghostClass, halo, dark, metaboliteLabels.get(node.id)));
    }
  }

  if (labels !== 'none' || fvaEnabled) {
    let nodesOf = null;
    for (const [rid, rxn] of layout.reactions) {
      const v = fluxes.get(rid) ?? 0;
      const disabled = disabledReactionIds.has(rid);
      const inactive = inactiveSpeciesIds.has(rxn.cellId);
      const solved = disabled || inactive ? 0 : (solvedFluxes.get(rid) ?? 0);
      if (!fvaEnabled && !disabled) {
        if (labels === 'active' && Math.abs(solved) < 1e-9) continue;
        // `auto` keeps the overview readable by naming only the reactions that
        // actually carry the map: everything else appears on hover or on zoom.
        if (labels === 'auto' && Math.abs(v) < scale.reference * 0.18
          && !(emphasizedReactionIds.has(rid) && Math.abs(solved) > 1e-9)) continue;
      }

      // Place the ID and its value beside the marker, leaving a gap even
      // for rotated transporters with several lanes.
      const angle = rxn.axis ? Math.atan2(rxn.axis.dir.y, rxn.axis.dir.x) : 0;
      const halfWidth = rxn.onMembrane ? 13 : 3;
      const halfHeight = rxn.onMembrane
        ? ((rxn.transportLanes?.length ?? 0) >= 2 ? (14 + rxn.transportLanes.length * 10) / 2 : 11)
        : 9;
      const markerRight = halfWidth * Math.abs(Math.cos(angle)) + halfHeight * Math.abs(Math.sin(angle));
      let tx = rxn.x + markerRight + 8;
      let ty = rxn.y;
      let placement = 'emc-label-rxn-adjacent';
      // A reaction kept straight and lying on its side has its own arm where
      // the name would go, so there the name stands over the bar, on the side
      // with fewer of its metabolites.
      if (rxn.linear && rxn.axis && !rxn.onMembrane && Math.abs(Math.sin(angle)) < 0.5) {
        const markerTop = halfWidth * Math.abs(Math.sin(angle)) + halfHeight * Math.abs(Math.cos(angle));
        nodesOf ??= reactionNodes(layout);
        const below = emptierSide(nodesOf.get(rid), rxn) > 0;
        tx = rxn.x;
        ty = rxn.y + (markerTop + 5) * (below ? 1 : -1);
        placement = below ? 'emc-label-rxn-below' : 'emc-label-rxn-above';
      }
      const localId = String(rxn.localId ?? '').trim();
      const cellPrefix = rxn.cellId ? `${rxn.cellId}@` : '';
      const reactionId = reactionLabels.get(rid) ?? (localId || (cellPrefix && rid.startsWith(cellPrefix)
        ? rid.slice(cellPrefix.length) || rid
        : rid));

      let text;
      if (fvaEnabled) {
        const range = fvaRanges.get(rid);
        text = range && Number.isFinite(range.min) && Number.isFinite(range.max)
          ? `${reactionId}  [${formatFlux(range.min, 3)}, ${formatFlux(range.max, 3)}]`
          : `${reactionId}  [calculando…]`;
      } else {
        text = `${reactionId}${disabled ? ' [KO]' : ''}  ${formatFlux(v, 3)}`;
      }
      const off = !fvaEnabled && isOffReaction(rid, { offStyle, solvedFluxes, disabledReactionIds, inactive });
      out.push(label(tx, ty, text, `emc-label-rxn${disabled ? ' is-disabled-reaction' : ''}${off ? ' is-off' : ''}${inactive ? ' is-inactive-species' : ''}${fvaEnabled ? ' emc-label-fva' : ''} ${placement}`, halo, dark, rid,
        placement === 'emc-label-rxn-adjacent' ? 'start' : 'middle', inactive ? ' opacity="0.28"' : ''));
    }
  }

  out.push('</g>');
  return out.join('');
}

/** The metabolite nodes each reaction reaches. */
function reactionNodes(layout) {
  const byReaction = new Map();
  for (const seg of layout.segments) {
    const node = seg.nodeId ? layout.nodes.get(seg.nodeId) : null;
    if (!node || !seg.reactionId) continue;
    if (!byReaction.has(seg.reactionId)) byReaction.set(seg.reactionId, []);
    byReaction.get(seg.reactionId).push(node);
  }
  return byReaction;
}

/** Above (-1) or below (+1) a reaction lying on its side: where fewer of its metabolites stand. */
function emptierSide(nodes = [], rxn) {
  let above = 0;
  let below = 0;
  for (const node of nodes) {
    if (node.y < rxn.y - 4) above += 1;
    else if (node.y > rxn.y + 4) below += 1;
  }
  return below < above ? 1 : -1;
}

function metaboliteLabel(node, cls, halo, dark, text = node.base) {
  if (!['metabolite', 'medium', 'currency'].includes(node.kind)) {
    const gap = node.primary === false ? 3.5 : 5;
    return label(node.x, node.y - node.r - gap, node.base, cls, halo, dark);
  }
  const { x, y, anchor, baseline } = metaboliteLabelPosition(node);
  return label(x, y, text, `${cls} emc-label-draggable`, halo, dark, null, anchor,
    ` data-metabolite-label="${esc(node.id)}" dominant-baseline="${baseline}"${cls.includes('is-inactive-species') ? ' opacity="0.28"' : ''}`);
}

function label(x, y, text, cls, halo, dark, rid, anchor = 'middle', extraAttrs = '') {
  const attrs = rid ? ` data-rxn="${esc(rid)}"` : '';
  return `<text class="emc-label ${cls}" x="${n(x)}" y="${n(y)}" text-anchor="${anchor}"`
    + ` stroke="${halo}" stroke-width="3.2" stroke-linejoin="round" paint-order="stroke"`
    + ` fill="${dark ? '#e2e8f0' : '#1e293b'}"${attrs}${extraAttrs}>${esc(text)}</text>`;
}

// Cross-feeding

/**
 * One line per metabolite a species passes to another, from the producer's
 * medium node to the consumer's, ending in an arrow, in the metabolite's colour
 * (see crossFeedingColors), so every line of one metabolite looks the same.
 * Protons and water get none. The routed lines all have one width,
 * `widthScale` times the default, and the flux is in their hover text. Returns
 * the routed lines and, separately, the arcs for links that could not be routed.
 */
function renderCrossFeeding(layout, crossFeeding, scale, dark, explorer = null, bases = null, widthScale = 1) {
  const out = ['<g class="emc-layer emc-layer-crossfeed">'];
  const routedOut = ['<g class="emc-layer emc-layer-crossfeed emc-layer-crossfeed-routed">'];
  const cellById = new Map(layout.cells.map((c) => [c.id, c]));

  const filtered = explorer?.enabled ? new Set((explorer.visibleEvents ?? []).map((e) => e.from + '|' + e.base + '|' + e.to)) : null;
  const selected = explorer?.selectedEvent;
  const isPicked = (link) => Boolean(selected && selected.from === link.from && selected.to === link.to && selected.base === link.base);
  // Only the metabolites chosen to be shown (and the picked line). The routes
  // are worked out for those alone, so fewer lines also run straighter.
  const links = crossFeeding.links.filter((link) => link.from && link.to && isDrawnCrossFeedingBase(link.base)
    && (!bases || bases.has(link.base) || isPicked(link)));
  // Colours are shared out among the metabolites shown, the ones the panel's
  // key lists; a picked line of another metabolite keeps its own colour.
  const colors = crossFeedingColors(links.filter((link) => !bases || bases.has(link.base)).map((link) => link.base), dark);
  const colorOf = (base) => colors.get(base) ?? crossFeedingColor(base, dark);
  const lineWidth = crossFeedingLineWidth(layout, widthScale);
  // The lanes are spaced for this width, so lines side by side never overlap.
  const routes = routeCrossFeeding(layout, links, { lineWidth });
  const grid = routeGrid(layout);
  // Smallest flux first and the picked line last, over all of them.
  const ordered = [...links].sort((p, q) => (isPicked(p) - isPicked(q)) || (p.flux - q.flux));
  let tag = '';
  for (const link of ordered) {
    const key = link.from + '|' + link.base + '|' + link.to;
    if (filtered && !filtered.has(key)) continue;
    const a = cellById.get(link.from);
    const b = cellById.get(link.to);
    if (!a || !b) continue;
    const isSelected = selected && selected.from === link.from && selected.to === link.to && selected.base === link.base;
    const color = colorOf(link.base);
    const route = routes.get(key);
    if (route) {
      const w = isSelected ? lineWidth * 1.4 : lineWidth;
      const opacity = isSelected ? 1 : (explorer?.enabled && selected ? 0.3 : 0.8);
      const pts = route.points;
      // stop short of the node so the arrow tip touches its rim
      const end = route.end;
      const tip = { x: end.x + end.nx * (end.r + 2), y: end.y + end.ny * (end.r + 2) };
      const head = Math.max(w * 1.6, grid * 0.38);
      const neck = { x: tip.x + end.nx * head, y: tip.y + end.ny * head };
      const start = route.start;
      const from = { x: start.x + start.nx * start.r, y: start.y + start.ny * start.r };
      const line = [from, ...pts.slice(1, -1), neck];
      const attrs = `data-crossfeed-route="" data-crossfeed="${esc(link.base)}" data-crossfeed-from="${esc(link.from)}" data-crossfeed-to="${esc(link.to)}"`;
      const d = roundedPath(line, grid * 2.2);
      const what = crossFeedingText(link, route.start.name ?? route.end.name);
      // --w lets the stylesheet thicken the line under the pointer. The wide
      // transparent stroke is what the pointer finds, so a thin line is not a
      // one-pixel target.
      routedOut.push(`<g class="emc-crossfeed-route${isSelected ? ' is-picked' : ''}" opacity="${n(opacity)}" style="--w: ${n(w)}" ${attrs}>`
        + `<title>${esc(`${what} mmol·gDW⁻¹·h⁻¹ of the whole community, split in proportion`)}</title>`
        + `<path class="emc-crossfeed" d="${d}" fill="none" stroke="${color}" `
        + `stroke-width="${n(w)}" stroke-linecap="round" stroke-linejoin="round"/>`
        + `<polygon class="emc-crossfeed-arrow" points="${arrowHead(tip, end, head, Math.max(w * 1.2, grid * 0.3))}" fill="${color}"/>`
        + `<path class="emc-crossfeed-hit" d="${d}" fill="none" stroke="transparent" stroke-width="${n(Math.max(grid * 0.55, w))}" pointer-events="stroke"/>`
        + '</g>');
      if (isSelected) tag = crossFeedingTag(routeAnchor(line, explorer?.selectedAt ?? null), what, color, dark);
      continue;
    }
    const ca = { x: a.rect.x + a.rect.w / 2, y: a.rect.y + a.rect.h / 2 };
    const cb = { x: b.rect.x + b.rect.w / 2, y: b.rect.y + b.rect.h / 2 };
    const p1 = closestPerimeterPoint(a.rect, cb);
    const p2 = closestPerimeterPoint(b.rect, ca);
    const mx = (p1.x + p2.x) / 2;
    const my = (p1.y + p2.y) / 2;
    const dx = p2.x - p1.x;
    const dy = p2.y - p1.y;
    const len = Math.hypot(dx, dy) || 1;
    const bow = 0.16 * len;
    const cx = mx - (dy / len) * bow;
    const cy = my + (dx / len) * bow;
    const w = Math.max(isSelected ? 5 : 2, scale.width(link.flux) * (isSelected ? 1.5 : 0.8));
    out.push(`<path class="emc-crossfeed" d="M ${n(p1.x)} ${n(p1.y)} Q ${n(cx)} ${n(cy)}, ${n(p2.x)} ${n(p2.y)}" `
      + `fill="none" stroke="${color}" stroke-width="${n(w)}" stroke-linecap="round" `
      + `opacity="${isSelected ? 0.9 : (explorer?.enabled ? 0.5 : 0.22)}" data-crossfeed="${esc(link.base)}" data-crossfeed-from="${esc(link.from)}" data-crossfeed-to="${esc(link.to)}"/>`);
  }

  out.push('</g>');
  routedOut.push('</g>');
  return { behind: out.join(''), routed: routedOut.join(''), tag };
}

/** What a cross-feeding line carries: `Acetate (ac) · St → Fp · 0.611`, one name of the metabolite's synonyms. */
function crossFeedingText(link, raw) {
  const name = raw ? readableName(raw, [link.base]) : raw;
  const what = name && name !== link.base ? `${name} (${link.base})` : link.base;
  return `${what} · ${link.from} → ${link.to} · ${formatFlux(link.flux)}`;
}

/**
 * The name tag of the picked line, pinned where it was clicked. It keeps one
 * size on screen at any zoom, like the cells' name plates: the group's origin
 * is the pin, and the stylesheet scales it about that corner.
 */
function crossFeedingTag(at, text, color, dark) {
  const width = Math.round(String(text).length * 7.4 + 22);
  const bg = dark ? '#0b1220' : '#ffffff';
  const fg = dark ? '#f8fafc' : '#111827';
  return `<g class="emc-layer emc-layer-crossfeed-tag" transform="translate(${n(at.x)} ${n(at.y)})">`
    + '<g class="emc-frame-label emc-crossfeed-tag">'
    + `<circle r="4.5" fill="${color}" stroke="${bg}" stroke-width="1.5"/>`
    + `<rect x="10" y="-34" width="${width}" height="26" rx="7" fill="${bg}" fill-opacity="0.94" stroke="${color}" stroke-width="2"/>`
    + `<text x="21" y="-16.5" font-size="13" font-weight="700" fill="${fg}" textLength="${width - 22}" lengthAdjust="spacingAndGlyphs">${esc(text)}</text>`
    + '</g></g>';
}

/** Triangle with its tip at `tip`, pointing into the node along the end's normal. */
function arrowHead(tip, end, length, width) {
  const bx = tip.x + end.nx * length;
  const by = tip.y + end.ny * length;
  const px = -end.ny * width / 2;
  const py = end.nx * width / 2;
  return `${n(tip.x)},${n(tip.y)} ${n(bx + px)},${n(by + py)} ${n(bx - px)},${n(by - py)}`;
}

export { esc };
