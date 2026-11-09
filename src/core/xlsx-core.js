/**
 * Minimal XLSX reader, for the browser and Node alike.
 *
 * Published community models sometimes arrive as a workbook rather than SBML
 * (the nine-species gut microbiota model of the SteadyCom paper is distributed
 * that way, and the COBRA Toolbox writes one with writeCbModel), so this reads
 * one without pulling in a dependency, the same way sbml.js reads a COBRA SBML
 * file. src/core/xlsx.js binds it to Node's zlib.
 *
 * An .xlsx is a ZIP of XML. Two things about the format matter for correctness
 * and are easy to get wrong:
 *
 *   Empty cells are omitted entirely. A row is a sparse list, so reading cells
 *   by their position in the row silently shifts every value after the first
 *   gap into the wrong column. Cells are therefore keyed by the column letter
 *   from their `r` reference, never by order.
 *
 *   Text usually lives in a shared-string table and the cell holds an index
 *   into it (`t="s"`). A cell with `t="str"` or `t="inlineStr"` holds its own
 *   text instead, and a cell with no `t` is a number. All four appear in real
 *   files.
 *
 * Rows are yielded one at a time rather than collected, because these sheets run
 * to tens of thousands of rows.
 */

// ZIP

const EOCD = 0x06054b50;
const CENTRAL = 0x02014b50;
const utf8 = new TextDecoder('utf-8');

/**
 * Index a ZIP archive by reading its central directory.
 * @param {Uint8Array} bytes
 * @returns {Map<string, {method:number, offset:number, size:number}>}
 */
function readZip(bytes) {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  // The end-of-central-directory record is last, but a trailing comment may
  // follow it, so scan back for the signature rather than assuming the offset.
  let eocd = -1;
  for (let i = bytes.length - 22; i >= 0 && i >= bytes.length - 22 - 65535; i--) {
    if (view.getUint32(i, true) === EOCD) { eocd = i; break; }
  }
  if (eocd < 0) throw new Error('not a ZIP archive: no end-of-central-directory record');

  const count = view.getUint16(eocd + 10, true);
  let at = view.getUint32(eocd + 16, true);
  const entries = new Map();

  for (let i = 0; i < count; i++) {
    if (at + 46 > bytes.length || view.getUint32(at, true) !== CENTRAL) throw new Error(`corrupt central directory at ${at}`);
    const method = view.getUint16(at + 10, true);
    // The compressed size: a stored entry has the same one, and a deflated
    // stream must be cut at its end for a streaming inflater.
    const size = view.getUint32(at + 20, true);
    const nameLength = view.getUint16(at + 28, true);
    const extraLength = view.getUint16(at + 30, true);
    const commentLength = view.getUint16(at + 32, true);
    const offset = view.getUint32(at + 42, true);
    const name = utf8.decode(bytes.subarray(at + 46, at + 46 + nameLength));
    entries.set(name, { method, offset, size });
    at += 46 + nameLength + extraLength + commentLength;
  }
  return entries;
}

/** The stored bytes of one entry. The local header's lengths are the ones to trust. */
function entryData(bytes, entry) {
  // The central directory's name and extra lengths can differ from the local
  // header's, so the data offset is computed from the local header.
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  if (entry.offset + 30 > bytes.length) throw new Error('corrupt local header');
  const nameLength = view.getUint16(entry.offset + 26, true);
  const extraLength = view.getUint16(entry.offset + 28, true);
  const start = entry.offset + 30 + nameLength + extraLength;
  if (entry.method !== 0 && entry.method !== 8) throw new Error(`unsupported ZIP compression method ${entry.method}`);
  return bytes.subarray(start, start + entry.size);
}

function asBytes(input) {
  if (input instanceof Uint8Array) return input;
  if (input instanceof ArrayBuffer) return new Uint8Array(input);
  if (ArrayBuffer.isView(input)) return new Uint8Array(input.buffer, input.byteOffset, input.byteLength);
  throw new Error('expected the bytes of an .xlsx file');
}

// XML

const decode = (s) => (s ?? '')
  .replace(/&lt;/g, '<').replace(/&gt;/g, '>')
  .replace(/&quot;/g, '"').replace(/&apos;/g, "'")
  .replace(/&#(\d+);/g, (_, n) => String.fromCodePoint(Number(n)))
  .replace(/&#x([0-9a-fA-F]+);/g, (_, n) => String.fromCodePoint(parseInt(n, 16)))
  .replace(/&amp;/g, '&');

/** All the text of an element, with its runs joined: `<t>a</t><t>b</t>` -> "ab". */
const textOf = (xml) => {
  let out = '';
  for (const m of xml.matchAll(/<t(?:\s[^>]*)?>([\s\S]*?)<\/t>|<t\s*\/>/g)) out += decode(m[1] ?? '');
  return out;
};

// Workbook

/**
 * Open a workbook whose parts `part(name)` returns as text.
 * @returns {{sheetNames: string[], rows: (name: string) => Generator<Object<string,string>>}}
 */
function workbookFromParts(has, part) {
  // Shared strings, in order: a cell with t="s" holds an index into this.
  const shared = [];
  if (has('xl/sharedStrings.xml')) {
    for (const m of part('xl/sharedStrings.xml').matchAll(/<si>([\s\S]*?)<\/si>/g)) {
      shared.push(textOf(m[1]));
    }
  }

  // Sheet name -> part name, through the relationship id the workbook cites.
  const rels = new Map();
  for (const m of part('xl/_rels/workbook.xml.rels').matchAll(/<Relationship\b([^>]*)\/>/g)) {
    const id = /\bId="([^"]*)"/.exec(m[1])?.[1];
    const targetAttr = /\bTarget="([^"]*)"/.exec(m[1])?.[1];
    if (!id || !targetAttr) continue;
    const target = targetAttr.replace(/^\//, '').replace(/^xl\//, '');
    rels.set(id, `xl/${target}`);
  }

  const sheets = new Map();
  for (const m of part('xl/workbook.xml').matchAll(/<sheet\b([^>]*)\/>/g)) {
    const name = decode(/\bname="([^"]*)"/.exec(m[1])?.[1] ?? '');
    const rid = /\br:id="([^"]*)"/.exec(m[1])?.[1];
    if (name && rid && rels.has(rid)) sheets.set(name, rels.get(rid));
  }

  /**
   * Yield each row of a sheet as { A: value, C: value, ... }. Columns with no
   * cell are simply absent, which is what the file says and what callers must
   * handle. The row number comes along as `__row`.
   */
  function* rows(sheetName) {
    const target = sheets.get(sheetName);
    if (!target) throw new Error(`no sheet named "${sheetName}" (have: ${[...sheets.keys()].join(', ')})`);
    const xml = part(target);

    for (const rowMatch of xml.matchAll(/<row\b([^>]*)>([\s\S]*?)<\/row>/g)) {
      const row = { __row: Number(/\br="(\d+)"/.exec(rowMatch[1])?.[1] ?? 0) };
      for (const cell of rowMatch[2].matchAll(/<c\b([^>]*?)(?:\/>|>([\s\S]*?)<\/c>)/g)) {
        const attrs = cell[1];
        const body = cell[2] ?? '';
        const ref = /\br="([A-Z]+)\d+"/.exec(attrs)?.[1];
        if (!ref) continue;
        const type = /\bt="([^"]*)"/.exec(attrs)?.[1] ?? 'n';

        let value;
        if (type === 's') {
          const index = Number(/<v>([\s\S]*?)<\/v>/.exec(body)?.[1]);
          value = shared[index] ?? '';
        } else if (type === 'inlineStr') {
          value = textOf(body);
        } else {
          const raw = /<v>([\s\S]*?)<\/v>/.exec(body)?.[1];
          value = raw === undefined ? undefined : decode(raw);
        }
        if (value !== undefined && value !== '') row[ref] = value;
      }
      yield row;
    }
  }

  return { sheetNames: [...sheets.keys()], rows };
}

/**
 * Open a workbook, inflating deflated parts with `inflateRaw` (raw DEFLATE
 * bytes in, bytes out), which only a workbook with deflated parts needs.
 * @param {Uint8Array|ArrayBuffer} input the .xlsx file
 * @param {(data: Uint8Array) => Uint8Array} [inflateRaw]
 */
export function openWorkbook(input, inflateRaw = null) {
  const bytes = asBytes(input);
  const zip = readZip(bytes);
  const part = (name) => {
    const entry = zip.get(name);
    if (!entry) throw new Error(`${name} is missing from the workbook`);
    const data = entryData(bytes, entry);
    if (entry.method === 0) return utf8.decode(data);
    if (!inflateRaw) throw new Error(`${name} is compressed and no inflater was given`);
    return utf8.decode(inflateRaw(data));
  };
  return workbookFromParts((name) => zip.has(name), part);
}

/**
 * Open a workbook in any modern browser or Node: its XML parts are inflated
 * first with DecompressionStream('deflate-raw'), then read as openWorkbook does.
 * @param {Uint8Array|ArrayBuffer} input the .xlsx file
 */
export async function readWorkbookAsync(input) {
  const bytes = asBytes(input);
  const zip = readZip(bytes);
  const parts = new Map();
  for (const [name, entry] of zip) {
    if (!/\.(xml|rels)$/i.test(name)) continue;
    const data = entryData(bytes, entry);
    parts.set(name, entry.method === 0 ? utf8.decode(data) : utf8.decode(await inflateRawStream(data)));
  }
  return workbookFromParts((name) => parts.has(name), (name) => {
    if (!parts.has(name)) throw new Error(`${name} is missing from the workbook`);
    return parts.get(name);
  });
}

async function inflateRawStream(data) {
  if (typeof DecompressionStream !== 'function' || typeof Response !== 'function') {
    throw new Error('this browser does not provide DEFLATE decompression (DecompressionStream)');
  }
  try {
    const stream = new Response(data).body.pipeThrough(new DecompressionStream('deflate-raw'));
    return new Uint8Array(await new Response(stream).arrayBuffer());
  } catch {
    throw new Error('invalid compressed data in the workbook');
  }
}
