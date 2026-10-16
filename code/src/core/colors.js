/**
 * The one rule for a colour a user can choose: #rrggbb, stored lower-case.
 * Pathway colours, map cell and compartment colours and the colour pickers all
 * go through it, so they cannot disagree about what a valid colour is.
 *
 * @returns {string|null} the normalised colour, or null when it is not one
 */
export function normalizeHexColor(value) {
  const text = String(value ?? '').trim();
  return /^#[0-9a-fA-F]{6}$/.test(text) ? text.toLowerCase() : null;
}
