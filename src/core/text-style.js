/**
 * Styles a free map text can have: a size and a font. Fonts are kept to a short
 * list that every system draws the same way, and stored by key in the map file,
 * so a map opened on another machine looks as it did where it was drawn.
 */

/*
 * `advance` is the average width of one character, in em, at the weight map
 * text is drawn in. Selecting, moving and aligning a text use a box estimated
 * from it, since measuring glyphs needs a DOM. It errs a little wide, which only
 * makes a label easier to catch.
 */
export const TEXT_FONTS = [
  { id: 'sans', label: 'Sans serif', stack: '"Inter", "Segoe UI", system-ui, -apple-system, sans-serif', advance: 0.58 },
  { id: 'serif', label: 'Serif', stack: 'Georgia, "Times New Roman", Times, serif', advance: 0.56 },
  { id: 'mono', label: 'Monospace', stack: '"JetBrains Mono", Consolas, "Courier New", monospace', advance: 0.64 },
  { id: 'arial', label: 'Arial', stack: 'Arial, Helvetica, sans-serif', advance: 0.57 },
  { id: 'times', label: 'Times New Roman', stack: '"Times New Roman", Times, serif', advance: 0.52 },
  { id: 'courier', label: 'Courier New', stack: '"Courier New", Courier, monospace', advance: 0.64 },
];

export const DEFAULT_TEXT_FONT = 'sans';
export const TEXT_SIZE = Object.freeze({ min: 8, max: 144, default: 22 });

/** A known font key, or the default font's key for anything else. */
export function normalizeTextFont(id) {
  return TEXT_FONTS.some((font) => font.id === id) ? id : DEFAULT_TEXT_FONT;
}

/** The CSS font-family stack of a font key (the first font's for an unknown key). */
export function textFontStack(id) {
  return (TEXT_FONTS.find((font) => font.id === id) ?? TEXT_FONTS[0]).stack;
}

/** Average character width of a font, in em. */
export function textFontAdvance(id) {
  return (TEXT_FONTS.find((font) => font.id === id) ?? TEXT_FONTS[0]).advance;
}

/** A size in px, rounded and clamped to TEXT_SIZE.min..max. A value that is not a number gives the default. */
export function clampTextSize(value) {
  const size = Number(value);
  if (!Number.isFinite(size)) return TEXT_SIZE.default;
  return Math.min(TEXT_SIZE.max, Math.max(TEXT_SIZE.min, Math.round(size)));
}
