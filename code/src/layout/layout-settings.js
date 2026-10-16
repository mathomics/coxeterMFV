/**
 * User-facing automatic-layout presets.
 *
 * These values are deliberately expressed in map pixels / dimensionless
 * multipliers so they can be shown directly in the UI and passed unchanged to
 * the layout engine.
 */

export const LAYOUT_PRESETS = Object.freeze({
  compact: Object.freeze({
    preset: 'compact',
    cellSpacing: 120,
    minimumCompartmentSize: 38,
    nodeSpacing: 0.82,
    cellAreaMultiplier: 1,
  }),
  normal: Object.freeze({
    preset: 'normal',
    cellSpacing: 180,
    minimumCompartmentSize: 48,
    nodeSpacing: 1.08,
    cellAreaMultiplier: 3,
  }),
  spacious: Object.freeze({
    preset: 'spacious',
    cellSpacing: 240,
    minimumCompartmentSize: 60,
    nodeSpacing: 1.42,
    cellAreaMultiplier: 6,
  }),
});

export const DEFAULT_LAYOUT_PRESET = 'spacious';

/** A copy of the named preset, or of the default one when the name is unknown. */
export function layoutPreset(name = DEFAULT_LAYOUT_PRESET) {
  const preset = LAYOUT_PRESETS[name] ?? LAYOUT_PRESETS[DEFAULT_LAYOUT_PRESET];
  return { ...preset };
}

/**
 * Clamp user layout settings to their allowed ranges, filling missing or
 * invalid values from the preset named by `raw.preset` (the default one when
 * it is unknown). `preset` in the result is the preset the values match
 * exactly, or 'custom'.
 */
export function normalizeLayoutSettings(raw = {}) {
  const base = layoutPreset(raw.preset);
  const next = {
    preset: base.preset,
    cellSpacing: clamp(raw.cellSpacing, 80, 420, base.cellSpacing),
    minimumCompartmentSize: clamp(raw.minimumCompartmentSize, 28, 110, base.minimumCompartmentSize),
    nodeSpacing: clamp(raw.nodeSpacing, 0.65, 1.9, base.nodeSpacing),
    cellAreaMultiplier: clamp(raw.cellAreaMultiplier, 1, 8, base.cellAreaMultiplier),
  };

  const matched = Object.entries(LAYOUT_PRESETS).find(([, preset]) =>
    Math.abs(next.cellSpacing - preset.cellSpacing) < 1e-9
    && Math.abs(next.minimumCompartmentSize - preset.minimumCompartmentSize) < 1e-9
    && Math.abs(next.nodeSpacing - preset.nodeSpacing) < 1e-9
    && Math.abs(next.cellAreaMultiplier - preset.cellAreaMultiplier) < 1e-9
  );
  next.preset = matched?.[0] ?? 'custom';
  return next;
}

/** Normalised settings, renamed to the options layoutCommunity() takes. */
export function layoutEngineOptions(settings = {}) {
  const normalized = normalizeLayoutSettings(settings);
  return {
    cellGap: normalized.cellSpacing,
    minShell: normalized.minimumCompartmentSize,
    nodeSpacing: normalized.nodeSpacing,
    cellAreaMultiplier: normalized.cellAreaMultiplier,
  };
}

function clamp(value, min, max, fallback) {
  const n = Number(value);
  if (!Number.isFinite(n)) return fallback;
  return Math.min(max, Math.max(min, n));
}
