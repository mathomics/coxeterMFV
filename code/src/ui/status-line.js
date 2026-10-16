/**
 * The one line of status in the header.
 *
 * Busy, error and warning states say what is going on until something replaces
 * them. A plain `ok` message ("SteadyCom calculation cancelled", "Map loaded",
 * the permitted µ range, why a layout preset kept the drawing) is shown for
 * NOTICE_MS and then gives way to the standing summary. Without that delay the
 * summary would replace it on the very paint that should have shown it.
 */

export const NOTICE_MS = 6000;

/**
 * What the status line shows for the store's current status.
 *
 * @param {object} store
 * @param {number} [noticeAge] milliseconds since this status was first painted
 * @returns {{ text: string, title: string, state: string, expiresIn: number }}
 *   expiresIn > 0 when the line has to be repainted after that many ms
 */
export function statusLine(store, noticeAge = Infinity) {
  const s = store.status ?? {};
  if (s.state === 'busy') return { text: s.message || 'Working…', title: '', state: 'busy', expiresIn: 0 };
  if (s.state === 'error') return { text: s.message ?? '', title: s.message ?? '', state: 'error', expiresIn: 0 };
  if (s.state === 'warn' && s.message) return { text: s.message, title: s.message, state: 'warn', expiresIn: 0 };
  if (s.state === 'ok' && s.message && noticeAge < NOTICE_MS) {
    return { text: s.message, title: s.message, state: 'notice', expiresIn: NOTICE_MS - noticeAge };
  }
  return { text: statusSummary(store), title: '', state: 'summary', expiresIn: 0 };
}

/** The standing summary: active species, merged reactions, objective value and LP time. */
export function statusSummary(store) {
  const mu = store.solution?.objectiveValue;
  const n = store.community ? store.community.merged.reactions.size : 0;
  return `${store.community?.activeSpecies.length ?? 0} species · ${n} reactions · `
    + `objective ${Number.isFinite(mu) ? mu.toFixed(5) : '—'} · LP ${store.status?.solveMs ?? 0} ms`;
}
