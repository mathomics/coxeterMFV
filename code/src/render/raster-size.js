/**
 * How large a PNG of the map a browser can actually make.
 *
 * The PNG is the SVG drawn into a canvas, and a canvas has a ceiling: about
 * 268 million pixels in Chromium, about 124 million in Firefox, 16.7 million in
 * Safari, and 32767 pixels a side. A hand-built genome-scale map is easily
 * 10000 × 10000, which at the 2× the export asks for is 400 million pixels,
 * more than any of those canvases holds, so a fixed 2× would fail on the very
 * maps that most need a picture. Each entry here is the largest scale, up to the
 * 2× that is wanted, that fits one browser's ceiling. The export tries them in
 * order and keeps the first one the browser accepts.
 */

export const PREFERRED_SCALE = 2;

const CEILINGS = [
  { area: 268_435_456, side: 32_767 }, // Chromium
  { area: 124_000_000, side: 32_767 }, // Firefox
  { area: 16_777_216, side: 16_384 },  // Safari, iOS
];

/** Scales to try for a map of `width` × `height` SVG pixels, largest first, each at most `preferred`. */
export function rasterScales(width, height, preferred = PREFERRED_SCALE) {
  const w = Math.max(1, Number(width) || 1);
  const h = Math.max(1, Number(height) || 1);
  const scales = [];
  for (const { area, side } of CEILINGS) {
    const fit = Math.min(preferred, side / w, side / h, Math.sqrt(area / (w * h)));
    // Rounded down, so the rounded pixel size never lands just over the ceiling.
    const scale = Math.floor(fit * 100) / 100;
    if (scale > 0 && !scales.includes(scale)) scales.push(scale);
  }
  return scales;
}
