/**
 * The XLSX reader of xlsx-core.js bound to Node's zlib, so a workbook opens
 * synchronously. The browser reads workbooks with readWorkbookAsync instead.
 */

import { inflateRawSync } from 'node:zlib';
import { openWorkbook } from './xlsx-core.js';

/**
 * Open a workbook.
 * @param {Buffer|Uint8Array} bytes  the .xlsx file
 * @returns {{sheetNames: string[], rows: (name: string) => Generator<Object<string,string>>}}
 */
export function readWorkbook(bytes) {
  return openWorkbook(bytes, (data) => inflateRawSync(data));
}

/** Read a whole .xlsx from disk. */
export async function readWorkbookFile(path) {
  const { readFile } = await import('node:fs/promises');
  return readWorkbook(await readFile(path));
}
