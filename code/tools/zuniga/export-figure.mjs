/**
 * Draw the Zuñiga et al. (2020) ΔpheA rescue figure from the validated numbers
 * in results.json, without the source models. Writes an SVG and, through
 * Playwright, a PNG at twice the resolution.
 *
 *   node tools/zuniga/export-figure.mjs [--output=<dir>] [--svg-only]
 *       [--playwright-module=<path>] [--browser=<chrome binary>] [--copy-output=<dir>]
 */
import { readFile, mkdir, writeFile, copyFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const options = Object.fromEntries(process.argv.slice(2).map(value => {
  const match = /^--([^=]+)(?:=(.*))?$/.exec(value);
  if (!match) throw new Error(`Unknown argument: ${value}`);
  return [match[1], match[2] ?? true];
}));
const folder = path.join(root, 'models/published/zuniga-2020-phea-rescue');
const results = JSON.parse(await readFile(path.join(folder, 'results.json'), 'utf8'));
const manifest = JSON.parse(await readFile(path.join(folder, 'manifest.json'), 'utf8'));
if (!results.publishedPredictionsMatched) throw new Error('Validated numerical results are required.');
for (const source of manifest.sources) {
  if (results.sourceHashes[source.file] !== source.sha256) throw new Error(`Source hash mismatch: ${source.file}`);
}
const scenarios = ['wt-monoculture', 'mutant-monoculture', 'mutant-coculture'].map(id => {
  const scenario = results.scenarios.find(item => item.id === id);
  if (!scenario || scenario.status !== 'optimal') throw new Error(`Missing optimal scenario: ${id}`);
  return scenario;
});
const co = scenarios[2];
const transfer = co.fluxes['Ec@PHEtex'];
if (!(transfer > 0) || Math.abs(transfer + co.fluxes['Se@PHELtex']) > results.tolerance ||
    co.fluxes['Ec@PPNDH'] !== 0 || !co.knockouts.includes('Ec@PPNDH') || co.fluxes['Ec@CHORM'] <= 0) {
  throw new Error('The selected rescue mechanism does not match the validated solution.');
}
const colors = { Ec: '#2469ad', Se: '#159477', KO: '#b12662', ink: '#19283a', muted: '#596579', grid: '#dce2e9', inactive: '#aab4c0' };
const esc = value => String(value).replace(/[&<>"']/g, ch => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&apos;' }[ch]));
const chunks = [];
const add = value => chunks.push(value);
const text = (x, y, value, { size = 26, fill = colors.ink, weight = 400, anchor = 'start', italic = false } = {}) =>
  add(`<text x="${x}" y="${y}" font-size="${size}" fill="${fill}" font-weight="${weight}" text-anchor="${anchor}"${italic ? ' font-style="italic"' : ''}>${esc(value)}</text>`);
const line = (x1, y1, x2, y2, { stroke = colors.grid, width = 2, dash = '', arrow = '' } = {}) =>
  add(`<line x1="${x1}" y1="${y1}" x2="${x2}" y2="${y2}" stroke="${stroke}" stroke-width="${width}"${dash ? ` stroke-dasharray="${dash}"` : ''}${arrow ? ` marker-end="url(#arrow-${arrow})"` : ''}/>`);
const rect = (x, y, w, h, fill, stroke = 'none', rx = 0) =>
  add(`<rect x="${x}" y="${y}" width="${w}" height="${h}" rx="${rx}" fill="${fill}" stroke="${stroke}" stroke-width="2"/>`);
const metabolite = (x, y, label, inactive = false) => {
  rect(x - 170, y - 23, 340, 46, inactive ? '#f4f5f7' : '#fff', inactive ? '#c9d0d9' : '#a9b8c6', 14);
  text(x, y + 9, label, { size: 25, anchor: 'middle', fill: inactive ? colors.muted : colors.ink });
};
const rate = value => value === 0 ? '0' : value.toFixed(6);

add(`<svg xmlns="http://www.w3.org/2000/svg" width="1800" height="1440" viewBox="0 0 1800 1440" role="img" aria-labelledby="title description">
<title id="title">Coxeter reproduces the stationary ΔpheA rescue prediction</title>
<desc id="description">Three calculated growth scenarios and the selected phenylalanine rescue pathway for S. elongatus and E. coli. Mutant predictions match Supplementary Data 9 of Zuñiga et al. 2020. Wild-type monoculture is a newly calculated reference. Monoculture and coculture retain different source media.</desc>
<metadata>${esc(JSON.stringify({ study: results.id, doi: manifest.provenance.doi, sourceHashes: results.sourceHashes, method: results.method, growth: scenarios.map(({ id, growth }) => ({ id, growth })), phenylalanineTransfer: transfer, fluxUnits: manifest.fluxUnits, reference: manifest.provenance.reference, tolerance: results.tolerance, schematic: true }))}</metadata>
<defs>${Object.entries(colors).filter(([key]) => ['Ec', 'Se', 'inactive'].includes(key)).map(([key, color]) => `<marker id="arrow-${key}" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="8" markerHeight="8" orient="auto-start-reverse"><path d="M 0 0 L 10 5 L 0 10 z" fill="${color}"/></marker>`).join('')}</defs>
<g font-family="Arial, Helvetica, sans-serif">`);
rect(0, 0, 1800, 1440, '#fff');
text(70, 65, 'Coxeter reproduces the stationary ΔpheA rescue prediction', { size: 38, weight: 700 });
text(70, 108, 'Zuñiga et al. (2020) · Complete source models · Supplementary Data 9', { size: 25, fill: colors.muted });

text(70, 177, 'A', { size: 32, weight: 700 });
text(115, 177, 'Calculated growth rates', { size: 29, weight: 600 });
rect(1210, 153, 24, 24, colors.Ec, colors.Ec, 3);
text(1245, 176, 'E. coli (Ec)', { size: 25 });
rect(1450, 153, 24, 24, colors.Se, colors.Se, 3);
text(1485, 176, 'S. elongatus (Se)', { size: 25 });
const chart = { left: 225, right: 1670, top: 225, bottom: 515, max: 0.10 };
for (let tick = 0; tick <= 5; tick++) {
  const value = tick * .02;
  const y = chart.bottom - value / chart.max * (chart.bottom - chart.top);
  line(chart.left, y, chart.right, y, { stroke: tick === 0 ? '#758292' : '#e0e5eb', width: tick === 0 ? 2.5 : 1.5 });
  text(chart.left - 18, y + 9, value.toFixed(2), { size: 25, anchor: 'end', fill: colors.muted });
}
text(84, 203, 'μ (h⁻¹)', { size: 27, weight: 600 });
const centers = [450, 925, 1430];
for (let index = 0; index < scenarios.length; index++) {
  const scenario = scenarios[index];
  const center = centers[index];
  for (const [species, offset] of [['Ec', -67], ['Se', 67]]) {
    const x = center + offset - 47;
    const value = scenario.growth[species];
    if (value === undefined) {
      line(x + 7, chart.bottom, x + 87, chart.bottom, { stroke: colors.inactive, width: 3, dash: '7 6' });
      text(x + 47, chart.bottom - 16, 'absent', { size: 22, anchor: 'middle', fill: colors.muted });
      continue;
    }
    const height = value / chart.max * (chart.bottom - chart.top);
    if (height > 0) rect(x, chart.bottom - height, 94, height, colors[species], colors[species], 3);
    else line(x, chart.bottom, x + 94, chart.bottom, { stroke: colors[species], width: 5 });
    text(x + 47, chart.bottom - height - 16, rate(value), { size: 27, weight: 600, anchor: 'middle', fill: colors[species] });
  }
}
text(centers[0], 568, 'WT monoculture', { size: 27, weight: 600, anchor: 'middle' });
text(centers[0], 602, 'new reference calculation', { size: 23, anchor: 'middle', fill: colors.muted });
text(centers[1], 568, 'ΔpheA monoculture', { size: 27, weight: 600, anchor: 'middle' });
text(centers[1], 602, 'Data 9 prediction matched', { size: 23, anchor: 'middle', fill: colors.muted });
text(centers[2], 568, 'ΔpheA coculture', { size: 27, weight: 600, anchor: 'middle' });
text(centers[2], 602, 'Data 9 prediction matched', { size: 23, anchor: 'middle', fill: colors.muted });

text(70, 675, 'B', { size: 32, weight: 700 });
text(115, 675, 'Selected rescue pathway in mutant coculture', { size: 29, weight: 600 });
text(115, 708, 'Qualitative schematic: arrow width is constant; cofactors and other routes are omitted.', { size: 23, fill: colors.muted });
rect(70, 741, 720, 485, '#f0faf6', '#bddccf', 24);
rect(1010, 741, 720, 485, '#f1f6fc', '#c4d4e5', 24);
text(105, 789, 'S. elongatus PCC7942 cscB⁺', { size: 28, weight: 600, fill: colors.Se });
text(1045, 789, 'E. coli K-12 MG1655 ΔpheA', { size: 28, weight: 600, fill: colors.Ec });
const positions = { Se: 350, Ec: 1450 };
for (const species of ['Se', 'Ec']) {
  const x = positions[species];
  const ec = species === 'Ec';
  metabolite(x, 836, 'Chorismate');
  metabolite(x, 936, 'Prephenate');
  metabolite(x, 1036, 'Phenylpyruvate', ec);
  metabolite(x, 1136, 'L-Phenylalanine');
  line(x, 862, x, 908, { stroke: colors[species], width: 5, arrow: species });
  const labelX = ec ? 1140 : 600;
  text(labelX, 884, ec ? 'CHORM_D' : 'CHORM_A', { size: 25, weight: 600, anchor: 'middle', fill: colors[species] });
  if (ec) text(labelX, 913, 'b2599 or b2600', { size: 22, anchor: 'middle', fill: colors.muted });
  if (ec) {
    line(x, 962, x, 1008, { stroke: colors.KO, width: 5, dash: '7 5' });
    line(x - 13, 972, x + 13, 998, { stroke: colors.KO, width: 6 });
    line(x - 13, 998, x + 13, 972, { stroke: colors.KO, width: 6 });
    text(labelX, 979, 'PPNDH_D = 0', { size: 25, weight: 600, anchor: 'middle', fill: colors.KO });
    text(labelX, 1007, 'ΔpheA / b2599', { size: 22, anchor: 'middle', fill: colors.KO });
    line(x, 1062, x, 1108, { stroke: colors.inactive, width: 3, dash: '7 5' });
  } else {
    line(x, 962, x, 1008, { stroke: colors.Se, width: 5, arrow: 'Se' });
    text(labelX, 984, 'PPNDH_A', { size: 25, weight: 600, anchor: 'middle', fill: colors.Se });
    line(x, 1062, x, 1108, { stroke: colors.Se, width: 5, arrow: 'Se' });
  }
  text(ec ? 1637 : 140, 1091, ec ? 'PHETA1_D' : 'PHETA1_A', { size: 22, anchor: 'middle', fill: ec ? colors.muted : colors.Se });
  text(x, 1202, `μ = ${rate(co.growth[species])} h⁻¹`, { size: 26, weight: 600, anchor: 'middle', fill: colors[species] });
}
add('<circle cx="900" cy="1136" r="64" fill="#fff" stroke="#8798aa" stroke-width="2.5"/>');
text(900, 1146, 'Phe', { size: 29, weight: 600, anchor: 'middle' });
text(900, 1044, 'Shared pool', { size: 25, weight: 600, anchor: 'middle' });
line(524, 1136, 828, 1136, { stroke: colors.Se, width: 6, arrow: 'Se' });
line(972, 1136, 1272, 1136, { stroke: colors.Ec, width: 6, arrow: 'Ec' });
text(678, 1080, 'PHELtex_A', { size: 25, weight: 600, anchor: 'middle', fill: colors.Se });
text(678, 1110, `v = −${transfer.toFixed(6)}`, { size: 22, anchor: 'middle', fill: colors.muted });
text(1122, 1080, 'PHEtex_D', { size: 25, weight: 600, anchor: 'middle', fill: colors.Ec });
text(1122, 1110, `v = +${transfer.toFixed(6)}`, { size: 22, anchor: 'middle', fill: colors.muted });
text(900, 1274, `Net Phe transfer: Se → Ec = ${transfer.toFixed(9)} mmol gDW⁻¹ h⁻¹ (original model basis)`, { size: 26, weight: 600, anchor: 'middle' });
text(70, 1330, 'Saved source bounds: glucose uptake −1 (mono), −0.1 (co); citrate uptake −0.02 in both.', { size: 23, fill: colors.muted });
text(70, 1366, 'WT is a new calculation. Mutant growth and Phe exchange match Supplementary Data 9 within 10⁻⁸.', { size: 23, fill: colors.muted });
text(70, 1404, 'Reference: Zuñiga et al., Nature Communications 11, 3803 (2020). DOI: 10.1038/s41467-020-17612-8.', { size: 22, fill: colors.muted });
add('</g></svg>');

const out = path.resolve(options.output ?? path.join(root, 'figures/zuniga-2020-phea-rescue'));
await mkdir(out, { recursive: true });
const svg = path.join(out, 'coxeter-zuniga-figura.svg');
const png = path.join(out, 'coxeter-zuniga-figura.png');
await writeFile(svg, chunks.join('\n') + '\n');
if (!options['svg-only']) {
  const modulePath = options['playwright-module'] ?? process.env.COXETER_PLAYWRIGHT_MODULE ?? 'playwright';
  const moduleName = path.isAbsolute(modulePath) ? pathToFileURL(modulePath).href : modulePath;
  const { chromium } = await import(moduleName);
  const browser = await chromium.launch({ headless: true, ...(options.browser ? { executablePath: path.resolve(options.browser) } : {}) });
  try {
    const page = await browser.newPage({ viewport: { width: 1800, height: 1440 }, deviceScaleFactor: 2 });
    await page.goto(pathToFileURL(svg).href);
    await page.screenshot({ path: png });
  } finally { await browser.close(); }
}
if (options['copy-output']) {
  const target = path.resolve(options['copy-output']);
  await mkdir(target, { recursive: true });
  await copyFile(svg, path.join(target, path.basename(svg)));
  if (!options['svg-only']) await copyFile(png, path.join(target, path.basename(png)));
}
console.log(JSON.stringify({ svg, png: options['svg-only'] ? null : png, dimensions: '1800 × 1440 SVG; 3600 × 2880 PNG', source: 'Validated numerical results; no third-party models embedded.' }));
