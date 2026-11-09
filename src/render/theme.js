/**
 * Visual scales.
 *
 * Flux magnitude is carried by stroke width and by a sequential colour ramp,
 * and direction by the arrowhead. Compartment fills are deliberately
 * low-chroma pastels so that saturated flux lines stay readable on top of any
 * of them, in both colour schemes.
 */

import { compartmentColors } from '../core/compartments.js';

/** Sequential ramp for |flux|. Stops are (position, colour). */
const FLUX_RAMP_LIGHT = [
  [0.00, '#cbd5e1'],
  [0.12, '#38bdf8'],
  [0.35, '#2563eb'],
  [0.65, '#7c3aed'],
  [1.00, '#be123c'],
];

const FLUX_RAMP_DARK = [
  [0.00, '#475569'],
  [0.12, '#38bdf8'],
  [0.35, '#60a5fa'],
  [0.65, '#a78bfa'],
  [1.00, '#fb7185'],
];

/** Escher's absolute-value colouring: white at no flux, red at the largest. */
export function defaultAbsoluteFluxStops(max) {
  return [{ value: 0, color: '#ffffff' }, { value: max, color: '#dc2626' }];
}

/** normalizeFluxColorStops for |flux|, where no value is below zero. */
export function normalizeAbsoluteFluxColorStops(stops) {
  const normalized = normalizeFluxColorStops(stops);
  if (normalized?.some((stop) => stop.value < 0)) {
    throw new Error('Absolute flux colours need values of zero or more.');
  }
  return normalized;
}

/** The absolute palette as files keep it: its points (null for the default) and whether it is log. */
export function normalizeAbsoluteFluxColors(value) {
  return {
    stops: normalizeAbsoluteFluxColorStops(value?.stops ?? null),
    log: Boolean(value?.log),
  };
}

/**
 * The |flux| a log scale starts at: four decades below the top point, or a
 * decade below the smallest positive point when that is lower. Everything at
 * or under it takes the first colour. Null with no positive point to scale.
 */
export function absoluteLogFloor(stops) {
  const positive = stops.filter((stop) => stop.value > 0);
  if (!positive.length) return null;
  return Math.min(positive[positive.length - 1].value / 1e4, positive[0].value / 10);
}

const hexToRgb = (hex) => [
  parseInt(hex.slice(1, 3), 16),
  parseInt(hex.slice(3, 5), 16),
  parseInt(hex.slice(5, 7), 16),
];

const rgbToHex = ([r, g, b]) => `#${[r, g, b].map((v) => Math.round(v).toString(16).padStart(2, '0')).join('')}`;

/**
 * Validate and copy a user-defined colour scale keyed by signed flux values.
 * Returns the stops sorted by value, or null for the automatic ramp.
 */
export function normalizeFluxColorStops(stops) {
  if (stops === null) return null;
  if (!Array.isArray(stops) || !stops.length) {
    throw new Error('Flux colours need at least one point, or null for the automatic palette.');
  }

  const values = new Set();
  const normalized = [];
  for (let i = 0; i < stops.length; i++) {
    const stop = stops[i];
    if (!stop || !Number.isFinite(stop.value)) {
      throw new Error(`Flux colour point ${i + 1} needs a finite numeric value.`);
    }
    const color = typeof stop.color === 'string' ? stop.color.trim().toLowerCase() : '';
    if (!/^#[0-9a-f]{6}$/.test(color)) {
      throw new Error(`Flux colour point ${i + 1} needs a colour in #rrggbb format.`);
    }
    const value = stop.value === 0 ? 0 : stop.value;
    if (values.has(value)) {
      throw new Error(`Flux colour values must be unique; ${value} appears more than once.`);
    }
    values.add(value);
    normalized.push({ value, color });
  }
  return normalized.sort((a, b) => a.value - b.value);
}

/** Linear position without overflowing for finite endpoints of opposite sign. */
function signedPosition(value, lower, upper) {
  if (!(value > lower)) return 0;
  if (value >= upper) return 1;
  const span = upper - lower;
  return Number.isFinite(span)
    ? (value - lower) / span
    : (value / 2 - lower / 2) / (upper / 2 - lower / 2);
}

function sampleFluxColorStops(stops, value) {
  if (stops.length === 1 || !(value > stops[0].value)) return stops[0].color;
  const last = stops.length - 1;
  if (value >= stops[last].value) return stops[last].color;

  let lower = 0;
  let upper = last;
  while (upper - lower > 1) {
    const middle = Math.floor((lower + upper) / 2);
    if (value < stops[middle].value) upper = middle;
    else lower = middle;
  }
  const from = stops[lower];
  const to = stops[upper];
  const t = signedPosition(value, from.value, to.value);
  const a = hexToRgb(from.color);
  const b = hexToRgb(to.color);
  return rgbToHex([0, 1, 2].map((channel) => a[channel] + (b[channel] - a[channel]) * t));
}

function sampleRamp(ramp, t) {
  const p = Math.min(Math.max(t, 0), 1);
  for (let i = 1; i < ramp.length; i++) {
    if (p <= ramp[i][0]) {
      const [p0, c0] = ramp[i - 1];
      const [p1, c1] = ramp[i];
      const f = (p - p0) / (p1 - p0 || 1);
      const a = hexToRgb(c0);
      const b = hexToRgb(c1);
      return rgbToHex([0, 1, 2].map((k) => a[k] + (b[k] - a[k]) * f));
    }
  }
  return ramp[ramp.length - 1][1];
}

/**
 * Scale for a whole flux map. Uses a log-ish compression so that a map with one
 * huge flux does not render everything else as a hairline.
 */
export class FluxScale {
  /**
   * @param {Iterable<number>} values
   * @param {object} [opts] `colorStops` (see normalizeFluxColorStops), the
   *   absolute palette's `absoluteStops` (null for the default) and
   *   `absoluteLog`, `dark`, and the stroke widths `minWidth` (1.6),
   *   `maxWidth` (15), `zeroWidth` (1.1)
   */
  constructor(values, opts = {}) {
    this.colorStops = normalizeFluxColorStops(opts.colorStops ?? null);
    this.absoluteStops = normalizeAbsoluteFluxColorStops(opts.absoluteStops ?? null);
    this.absoluteLog = Boolean(opts.absoluteLog);
    const magnitudes = [];
    for (const v of values) {
      const m = Math.abs(v);
      if (Number.isFinite(m) && m > 1e-9) magnitudes.push(m);
    }
    magnitudes.sort((a, b) => a - b);
    this.count = magnitudes.length;
    this.min = magnitudes.length ? magnitudes[0] : 0;
    this.max = magnitudes.length ? magnitudes[magnitudes.length - 1] : 1;
    // A high percentile rather than the maximum: one runaway water flux should
    // not flatten the rest of the map.
    this.reference = magnitudes.length
      ? magnitudes[Math.min(magnitudes.length - 1, Math.floor(magnitudes.length * 0.93))]
      : 1;
    this.dark = Boolean(opts.dark);
    this.minWidth = opts.minWidth ?? 1.6;
    this.maxWidth = opts.maxWidth ?? 15;
    this.zeroWidth = opts.zeroWidth ?? 1.1;
  }

  /** Normalised position of |v| on the ramp, 0..1. */
  position(v) {
    const m = Math.abs(v);
    if (!(m > 1e-9)) return 0;
    const ref = Math.max(this.reference, 1e-9);
    return Math.min(1, Math.log1p(m / ref * 9) / Math.log(10));
  }

  width(v) {
    if (!(Math.abs(v) > 1e-9)) return this.zeroWidth;
    return this.minWidth + this.position(v) * (this.maxWidth - this.minWidth);
  }

  color(v) {
    if (this.colorStops) return sampleFluxColorStops(this.colorStops, v);
    const ramp = this.dark ? FLUX_RAMP_DARK : FLUX_RAMP_LIGHT;
    if (!(Math.abs(v) > 1e-9)) return ramp[0][1];
    return sampleRamp(ramp, this.position(v));
  }

  /** The |flux| colour points in use: the chosen ones, or white at 0 to red at the largest |flux|. */
  absolutePalette() {
    return this.absoluteStops ?? defaultAbsoluteFluxStops(this.max);
  }

  /** How |flux| is placed along the absolute palette: as it is, or as log10 above the floor. */
  absoluteAxis() {
    const stops = this.absolutePalette();
    const floor = this.absoluteLog ? absoluteLogFloor(stops) : null;
    return { stops, floor, at: (m) => (floor === null ? m : Math.log10(Math.max(m, floor))) };
  }

  /**
   * Colour by |flux| alone, along the absolute palette, linearly or on a log
   * scale. With no flux anywhere there is nothing to scale against, and lines
   * keep the automatic ramp's grey rather than turning white.
   */
  absoluteColor(v) {
    if (!this.count) return (this.dark ? FLUX_RAMP_DARK : FLUX_RAMP_LIGHT)[0][1];
    const { stops, floor, at } = this.absoluteAxis();
    const placed = floor === null ? stops : stops.map(({ value, color }) => ({ value: at(value), color }));
    return sampleFluxColorStops(placed, at(Math.abs(v)));
  }

  /**
   * The absolute palette's points for the legend, placed (`t`) as the scale
   * places them. On a log scale a point below the floor shows the floor.
   */
  absoluteLegendStops() {
    const { stops, floor, at } = this.absoluteAxis();
    const first = at(stops[0].value);
    const last = at(stops[stops.length - 1].value);
    return stops.map(({ value, color }) => ({
      t: stops.length === 1 ? 0 : signedPosition(at(value), first, last),
      value: floor === null ? value : Math.max(value, floor),
      color,
      width: this.width(value),
    }));
  }

  /** Swatches for the legend. */
  legendStops(n = 6) {
    if (this.colorStops) {
      const first = this.colorStops[0].value;
      const last = this.colorStops[this.colorStops.length - 1].value;
      return this.colorStops.map(({ value, color }) => ({
        t: this.colorStops.length === 1 ? 0 : signedPosition(value, first, last),
        value,
        color,
        width: this.width(value),
      }));
    }
    const out = [];
    for (let i = 0; i < n; i++) {
      const t = i / (n - 1);
      const value = t === 0 ? 0 : (Math.expm1(t * Math.log(10)) / 9) * this.reference;
      out.push({ t, value, color: this.color(value || 0), width: this.width(value) });
    }
    return out;
  }
}

/** Colours for a compartment in the active scheme. */
export function compartmentStyle(id, dark) {
  const c = compartmentColors(id, dark);
  return {
    fill: c.fill,
    stroke: c.stroke,
    text: c.text,
  };
}

/** Format a flux for display. */
export function formatFlux(v, digits = 3) {
  if (v === null || v === undefined || Number.isNaN(v)) return '—';
  if (Math.abs(v) < 1e-9) return '0';
  if (Math.abs(v) >= 1000) return v.toExponential(2);
  return Number(v.toFixed(digits)).toString();
}

export { sampleRamp };
