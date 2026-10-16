/**
 * Minimal XLSX reader.
 *
 * Published community models sometimes arrive as a workbook rather than SBML
 * (the nine-species gut microbiota model of the SteadyCom paper is distributed
 * that way), so this reads one without pulling in a dependency, the same way
 * sbml.js reads a COBRA SBML file.
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

import { inflateRawSync } from 'node:zlib';

// ZIP

const EOCD = 0x06054b50;
const CENTRAL = 0x02014b50;

/**
 * Index a ZIP archive by reading its central directory.
 * @param {Buffer} buf
 * @returns {Map<string, {method:number, offset:number, size:number}>}
 */
function readZip(buf) {
  // The end-of-central-directory record is last, but a trailing comment may
  // follow it, so scan back for the signature rather than assuming the offset.
  let eocd = -1;
  for (let i = buf.length - 22; i >= 0 && i >= buf.length - 22 - 65535; i--) {
    if (buf.readUInt32LE(i) === EOCD) { eocd = i; break; }
  }
  if (eocd < 0) throw new Error('not a ZIP archive: no end-of-central-directory record');

  const count = buf.readUInt16LE(eocd + 10);
  let at = buf.readUInt32LE(eocd + 16);
  const entries = new Map();

  for (let i = 0; i < count; i++) {
    if (buf.readUInt32LE(at) !== CENTRAL) throw new Error(`corrupt central directory at ${at}`);
    const method = buf.readUInt16LE(at + 10);
    const size = buf.readUInt32LE(at + 24);
    const nameLength = buf.readUInt16LE(at + 28);
    const extraLength = buf.readUInt16LE(at + 30);
    const commentLength = buf.readUInt16LE(at + 32);
    const offset = buf.readUInt32LE(at + 42);
    const name = buf.toString('utf8', at + 46, at + 46 + nameLength);
    entries.set(name, { method, offset, size });
    at += 46 + nameLength + extraLength + commentLength;
  }
  return entries;
}

/** Decompress one entry. The local header's lengths are the ones to trust. */
function readEntry(buf, entry) {
  // The central directory's name and extra lengths can differ from the local
  // header's, so the data offset is computed from the local header.
  const nameLength = buf.readUInt16LE(entry.offset + 26);
  const extraLength = buf.readUInt16LE(entry.offset + 28);
  const start = entry.offset + 30 + nameLength + extraLength;

  if (entry.method === 0) return buf.subarray(start, start + entry.size);
  if (entry.method === 8) return inflateRawSync(buf.subarray(start));
  throw new Error(`unsupported ZIP compression method ${entry.method}`);
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
 * Open a workbook.
 * @param {Buffer|Uint8Array} bytes  the .xlsx file
 * @returns {{sheetNames: string[], rows: (name: string) => Generator<Object<string,string>>}}
 */
export function readWorkbook(bytes) {
  const buf = Buffer.isBuffer(bytes) ? bytes : Buffer.from(bytes);
  const zip = readZip(buf);

  const part = (name) => {
    const entry = zip.get(name);
    if (!entry) throw new Error(`${name} is missing from the workbook`);
    return readEntry(buf, entry).toString('utf8');
  };

  // Shared strings, in order: a cell with t="s" holds an index into this.
  const shared = [];
  if (zip.has('xl/sharedStrings.xml')) {
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

/** Read a whole .xlsx from disk. */
export async function readWorkbookFile(path) {
  const { readFile } = await import('node:fs/promises');
  return readWorkbook(await readFile(path));
}
