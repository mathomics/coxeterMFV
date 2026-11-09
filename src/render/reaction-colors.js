/**
 * Categorical reaction colouring for subsystem/pathway views.
 *
 * Flux remains encoded by stroke width. These helpers only choose the colour,
 * keeping visual grouping completely separate from the biological model and
 * the solver.
 */

const LIGHT = [
  '#2563eb', '#dc2626', '#059669', '#7c3aed', '#d97706', '#0891b2',
  '#db2777', '#65a30d', '#4f46e5', '#c2410c', '#0f766e', '#9333ea',
  '#0284c7', '#be123c', '#15803d', '#a16207',
];

const DARK = [
  '#60a5fa', '#f87171', '#34d399', '#c4b5fd', '#fbbf24', '#22d3ee',
  '#f472b6', '#a3e635', '#818cf8', '#fb923c', '#2dd4bf', '#d8b4fe',
  '#38bdf8', '#fb7185', '#4ade80', '#facc15',
];

export const REACTION_COLOR_MODES = new Set(['flux', 'absflux', 'subsystem', 'pathway']);
export const PATHWAY_RIBBON_MODES = new Set(['off', 'subsystem', 'pathway']);

/** A known colour mode, or 'flux' for anything else. */
export function normalizeReactionColorMode(mode) {
  return REACTION_COLOR_MODES.has(mode) ? mode : 'flux';
}

/** Whether the mode colours lines by their flux ('flux', or 'absflux' by |flux| alone). */
export function isFluxColorMode(mode) {
  return mode === 'flux' || mode === 'absflux';
}

/** A known pathway ribbon mode, or 'off' for anything else. */
export function normalizePathwayRibbonMode(mode) {
  return PATHWAY_RIBBON_MODES.has(mode) ? mode : 'off';
}

/**
 * The reaction's subsystem or pathway label for this mode, a "Sin ..." label
 * when it has none, or null in a flux mode.
 */
export function reactionCategory(reaction, mode) {
  const normalized = normalizeReactionColorMode(mode);
  if (isFluxColorMode(normalized)) return null;
  if (normalized === 'subsystem') {
    return cleanText(reaction?.subsystem) || 'Sin subsystem';
  }
  return cleanText(reaction?.pathway) || 'Sin pathway';
}

/** Palette colour picked by a hash of the label, so it is the same on every map. Missing labels are grey. */
export function categoryColor(label, dark = false) {
  const text = cleanText(label);
  if (!text || /^Sin (subsystem|pathway)$/i.test(text)) {
    return dark ? '#94a3b8' : '#64748b';
  }
  const palette = dark ? DARK : LIGHT;
  return palette[stableHash(text) % palette.length];
}

/**
 * Build per-reaction colours and a legend. `opts.reactionIds` limits the legend
 * counts, but colours stay stable because they depend on the category name, not
 * on order. `opts.dark` selects the dark palette.
 * @returns {{mode:string, colors:Map, groups:Map, legend:object[]}}
 */
export function buildReactionColoring(model, mode = 'flux', opts = {}) {
  const normalized = normalizeReactionColorMode(mode);
  const colors = new Map();
  const groups = new Map();
  const counts = new Map();

  if (!model?.reactions || isFluxColorMode(normalized)) {
    return { mode: normalized, colors, groups, legend: [] };
  }

  const allowed = opts.reactionIds == null ? null : new Set(opts.reactionIds);
  for (const [rid, reaction] of model.reactions) {
    const group = reactionCategory(reaction, normalized);
    groups.set(rid, group);
    colors.set(rid, categoryColor(group, Boolean(opts.dark)));
    if (!allowed || allowed.has(rid)) counts.set(group, (counts.get(group) || 0) + 1);
  }

  const legend = [...counts]
    .map(([label, count]) => ({
      label,
      count,
      color: categoryColor(label, Boolean(opts.dark)),
    }))
    .sort((a, b) => {
      const aMissing = /^Sin /.test(a.label);
      const bMissing = /^Sin /.test(b.label);
      if (aMissing !== bMissing) return aMissing ? 1 : -1;
      return a.label.localeCompare(b.label);
    });

  return { mode: normalized, colors, groups, legend };
}

function cleanText(value) {
  if (Array.isArray(value)) {
    const item = value.find((x) => typeof x === 'string' && x.trim());
    return item ? item.trim() : '';
  }
  return typeof value === 'string' ? value.trim() : '';
}

function stableHash(text) {
  let hash = 2166136261;
  for (let i = 0; i < text.length; i++) {
    hash ^= text.charCodeAt(i);
    hash = Math.imul(hash, 16777619);
  }
  return hash >>> 0;
}
