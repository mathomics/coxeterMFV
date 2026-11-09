/**
 * Draw reaction flux intervals with endpoint dots and an optional solution marker.
 * The plot reads analysis results without changing the model or its map.
 */

import { formatFlux } from './theme.js';

/** The linear axis includes zero, so reverse and forward flux share one reference. */
export function fluxRangeDomain(rows) {
  let min = 0;
  let max = 0;
  for (const row of rows) {
    if (Number.isFinite(row.min) && Number.isFinite(row.max)) {
      min = Math.min(min, row.min, row.max);
      max = Math.max(max, row.min, row.max);
    }
  }
  if (min === max) {
    min = -1;
    max = 1;
  }
  return { min, max };
}

/** @param {{id:string,label?:string,min:number,max:number,flux?:number}[]} rows */
export function renderFluxRanges(rows, { title = 'FVA', width = 960 } = {}) {
  const domain = fluxRangeDomain(rows);
  const left = 250;
  const right = width - 40;
  const top = 74;
  const height = top + rows.length * 30 + 76;
  const x = (value) => left + (right - left) * (value - domain.min) / (domain.max - domain.min);
  const parts = [
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${width} ${height}" role="img" aria-label="${escapeXML(title)} flux intervals" style="width:100%;height:auto;display:block">`,
    '<rect width="100%" height="100%" fill="#ffffff"/>',
    '<g font-family="Arial, sans-serif" fill="#1f2937">',
    `<text x="12" y="24" font-size="16" font-weight="700">${escapeXML(title)} flux intervals</text>`,
    '<text x="12" y="48" font-size="12">Endpoints: minimum and maximum · orange diamond: current solution</text>',
  ];
  for (let i = 0; i <= 4; i++) {
    const value = domain.min + (domain.max - domain.min) * i / 4;
    const position = x(value);
    parts.push(`<line x1="${position}" y1="${top - 12}" x2="${position}" y2="${height - 64}" stroke="#e5e7eb"/>`);
    parts.push(`<text x="${position}" y="${height - 44}" text-anchor="middle" font-size="12">${escapeXML(formatFlux(value, 3))}</text>`);
  }
  parts.push(`<line x1="${x(0)}" y1="${top - 12}" x2="${x(0)}" y2="${height - 64}" stroke="#64748b"/>`);
  for (let i = 0; i < rows.length; i++) {
    const row = rows[i];
    const y = top + i * 30;
    parts.push(`<text x="12" y="${y + 4}" font-size="12">${escapeXML(row.label ?? row.id)}</text>`);
    if (!Number.isFinite(row.min) || !Number.isFinite(row.max)) {
      parts.push(`<text x="${left}" y="${y + 4}" font-size="12">Unsolved</text>`);
      continue;
    }
    const lo = x(row.min);
    const hi = x(row.max);
    parts.push(`<g><title>${escapeXML(row.id)}: [${row.min}, ${row.max}]</title>`);
    parts.push(`<line x1="${lo}" y1="${y}" x2="${hi}" y2="${y}" stroke="#2563eb" stroke-width="4"/>`);
    parts.push(`<circle cx="${lo}" cy="${y}" r="4" fill="#2563eb"/>`);
    parts.push(`<circle cx="${hi}" cy="${y}" r="4" fill="#2563eb"/>`);
    if (Number.isFinite(row.flux) && row.flux >= domain.min && row.flux <= domain.max) {
      const value = x(row.flux);
      parts.push(`<path d="M${value},${y - 5} l5,5 l-5,5 l-5,-5 Z" fill="#c2410c"/>`);
    }
    parts.push('</g>');
  }
  parts.push(`<text x="${(left + right) / 2}" y="${height - 12}" text-anchor="middle" font-size="13">Flux (model units) · linear scale</text>`);
  parts.push('</g></svg>');
  return parts.join('');
}

function escapeXML(value) {
  return String(value).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}
