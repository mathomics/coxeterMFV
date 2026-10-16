/**
 * Browser-only reader for the documented MATLAB Level 5 interchange format.
 * https://www.mathworks.com/help/pdf_doc/matlab/matfile_format.pdf
 *
 * Cells and structure arrays retain MATLAB column-major order. Numeric arrays
 * are flat typed arrays with a `shape` property. A scalar is a one-item array.
 * Scalar structures become ordinary objects, and character rows become strings.
 * Sparse arrays retain zero-based CSC storage: { shape, ir, jc, values }.
 * This deliberately rejects objects, complex arrays and Level 7.3/HDF5 files.
 * No model data or third-party parser code is included here.
 */

const MI = Object.freeze({
  INT8: 1, UINT8: 2, INT16: 3, UINT16: 4, INT32: 5, UINT32: 6,
  SINGLE: 7, DOUBLE: 9, INT64: 12, UINT64: 13, MATRIX: 14,
  COMPRESSED: 15, UTF8: 16, UTF16: 17, UTF32: 18,
});
const KNOWN_TYPES = new Set(Object.values(MI));
const MAX_BYTES = 128 * 1024 * 1024;
const MAX_ELEMENTS = 20_000_000;
const MAX_CONTAINERS = 1_000_000;
const MAX_DEPTH = 128;
const decoder = new TextDecoder('utf-8', { fatal: true });

function fail(message) { throw new Error(`MAT Level 5: ${message}`); }
function own(object, name, value) {
  Object.defineProperty(object, name, { value, enumerable: true, writable: true, configurable: true });
}
function shaped(values, shape) {
  Object.defineProperty(values, 'shape', { value: shape.slice(), enumerable: false });
  return values;
}

class Cursor {
  constructor(bytes, little) {
    this.bytes = bytes;
    this.view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    this.little = little;
    this.offset = 0;
  }

  tag() {
    const start = this.offset;
    if (this.bytes.length - start < 8) fail('truncated data-element tag.');
    const word = this.view.getUint32(start, this.little);
    let type, size, payload, next;
    if (word > 0xffff) {
      type = this.view.getUint16(start, this.little);
      size = this.view.getUint16(start + 2, this.little);
      if (size < 1 || size > 4) fail('invalid small data-element length.');
      payload = start + 4;
      next = start + 8;
    } else {
      type = word;
      size = this.view.getUint32(start + 4, this.little);
      payload = start + 8;
      // miCOMPRESSED is not padded. SciPy and MATLAB concatenate it directly.
      next = payload + size + (type === MI.COMPRESSED ? 0 : (8 - size % 8) % 8);
    }
    if (!KNOWN_TYPES.has(type)) fail(`unsupported data-element type ${type}.`);
    if (size > MAX_BYTES || next > this.bytes.length) fail('data element exceeds its container.');
    this.offset = next;
    return { type, bytes: this.bytes.subarray(payload, payload + size) };
  }

  done() {
    if (this.offset !== this.bytes.length) fail('unexpected trailing matrix data.');
  }
}

function numbers(element, little) {
  const { type, bytes } = element;
  const specs = {
    [MI.INT8]: [1, Int8Array, 'getInt8'], [MI.UINT8]: [1, Uint8Array, 'getUint8'],
    [MI.INT16]: [2, Int16Array, 'getInt16'], [MI.UINT16]: [2, Uint16Array, 'getUint16'],
    [MI.INT32]: [4, Int32Array, 'getInt32'], [MI.UINT32]: [4, Uint32Array, 'getUint32'],
    [MI.SINGLE]: [4, Float32Array, 'getFloat32'], [MI.DOUBLE]: [8, Float64Array, 'getFloat64'],
    [MI.INT64]: [8, Float64Array, 'getBigInt64'], [MI.UINT64]: [8, Float64Array, 'getBigUint64'],
  };
  const spec = specs[type];
  if (!spec) fail(`expected numeric data, found type ${type}.`);
  const [width, ArrayType, getter] = spec;
  if (bytes.length % width) fail('numeric data length is not a whole number of values.');
  const count = bytes.length / width;
  if (count > MAX_ELEMENTS) fail('numeric array exceeds the element limit.');
  const values = new ArrayType(count);
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  for (let i = 0; i < count; i++) {
    const raw = view[getter](i * width, little);
    if (typeof raw === 'bigint') {
      if (raw > BigInt(Number.MAX_SAFE_INTEGER) || raw < BigInt(Number.MIN_SAFE_INTEGER)) {
        fail('64-bit integer cannot be represented exactly as a JavaScript number.');
      }
      values[i] = Number(raw);
    } else values[i] = raw;
  }
  return values;
}

function utf8(bytes) {
  try { return decoder.decode(bytes); }
  catch { fail('invalid UTF-8 text.'); }
}

function textUnits(element, little) {
  const { type, bytes } = element;
  if (type === MI.UTF8 || type === MI.INT8 || type === MI.UINT8) return Array.from(utf8(bytes));
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  if (type === MI.UTF16 || type === MI.UINT16 || type === MI.INT16) {
    if (bytes.length % 2) fail('invalid UTF-16 byte length.');
    return Array.from({ length: bytes.length / 2 }, (_, i) => String.fromCharCode(view.getUint16(i * 2, little)));
  }
  if (type === MI.UTF32 || type === MI.UINT32) {
    if (bytes.length % 4) fail('invalid UTF-32 byte length.');
    return Array.from({ length: bytes.length / 4 }, (_, i) => {
      const code = view.getUint32(i * 4, little);
      if (code > 0x10ffff || (code >= 0xd800 && code <= 0xdfff)) fail('invalid UTF-32 code point.');
      return String.fromCodePoint(code);
    });
  }
  fail(`unsupported character storage type ${type}.`);
}

function matrix(element, little, depth) {
  if (element.type !== MI.MATRIX) fail('expected a matrix data element.');
  if (depth > MAX_DEPTH) fail('matrix nesting exceeds the depth limit.');
  const cursor = new Cursor(element.bytes, little);
  const flagsTag = cursor.tag();
  if (flagsTag.type !== MI.UINT32 || flagsTag.bytes.length !== 8) fail('invalid array flags.');
  const flags = numbers(flagsTag, little);
  const arrayClass = flags[0] & 0xff;
  if (flags[0] & 0x800) fail('complex arrays are not supported.');
  const dimsTag = cursor.tag();
  if (dimsTag.type !== MI.INT32) fail('invalid array dimensions type.');
  const shape = Array.from(numbers(dimsTag, little));
  if (shape.length < 2 || shape.length > 32 || shape.some(x => x < 0)) fail('invalid array dimensions.');
  let count = 1;
  for (const size of shape) {
    count *= size;
    if (!Number.isSafeInteger(count) || (arrayClass !== 5 && count > MAX_ELEMENTS) || size > MAX_ELEMENTS) {
      fail('array dimensions exceed the element limit.');
    }
  }
  const nameTag = cursor.tag();
  if (![MI.INT8, MI.UINT8, MI.UTF8].includes(nameTag.type)) fail('invalid array name type.');
  const name = utf8(nameTag.bytes);
  let value;
  if (arrayClass === 1) {
    if (count > MAX_CONTAINERS || count > (cursor.bytes.length - cursor.offset) / 8) fail('cell array exceeds its container or element limit.');
    value = Array.from({ length: count }, () => matrix(cursor.tag(), little, depth + 1).value);
  } else if (arrayClass === 2) {
    const lengthTag = cursor.tag();
    if (lengthTag.type !== MI.INT32) fail('invalid structure field-name length type.');
    const lengths = numbers(lengthTag, little);
    const width = lengths[0];
    if (lengths.length !== 1 || width < 1 || width > 1024) fail('invalid structure field-name length.');
    const namesTag = cursor.tag();
    if (namesTag.type !== MI.INT8 || namesTag.bytes.length % width) fail('invalid structure field names.');
    const fields = [];
    for (let start = 0; start < namesTag.bytes.length; start += width) {
      const slot = namesTag.bytes.subarray(start, start + width);
      const end = slot.indexOf(0);
      const field = utf8(slot.subarray(0, end < 0 ? slot.length : end));
      if (!field || fields.includes(field)) fail('empty or duplicate structure field name.');
      fields.push(field);
    }
    if (count > MAX_CONTAINERS || count * fields.length > (cursor.bytes.length - cursor.offset) / 8) {
      fail('structure array exceeds its container or element limit.');
    }
    const structures = Array.from({ length: count }, () => {
      const object = {};
      for (const field of fields) own(object, field, matrix(cursor.tag(), little, depth + 1).value);
      return object;
    });
    value = count === 1 ? structures[0] : structures;
  } else if (arrayClass === 4) {
    if (shape.length > 2) fail('character arrays with more than two dimensions are not supported.');
    const units = textUnits(cursor.tag(), little);
    if (units.length !== count) fail('character data does not match its dimensions.');
    if (shape[0] <= 1) value = units.join('');
    else value = Array.from({ length: shape[0] }, (_, row) => {
      let result = '';
      for (let col = 0; col < shape[1]; col++) result += units[row + col * shape[0]];
      return result;
    });
  } else if (arrayClass === 5) {
    if (shape.length !== 2) fail('sparse arrays must have two dimensions.');
    const irTag = cursor.tag(), jcTag = cursor.tag();
    if (![MI.INT32, MI.UINT32].includes(irTag.type) || ![MI.INT32, MI.UINT32].includes(jcTag.type)) {
      fail('invalid sparse index storage type.');
    }
    const ir = numbers(irTag, little), jc = numbers(jcTag, little), values = numbers(cursor.tag(), little);
    if (jc.length !== shape[1] + 1 || jc[0] !== 0) fail('invalid sparse column pointers.');
    const nnz = jc[jc.length - 1];
    if (nnz < 0 || nnz > ir.length || nnz > values.length || nnz > flags[1]) fail('sparse entries exceed allocated storage.');
    for (let col = 1; col < jc.length; col++) {
      if (jc[col] < jc[col - 1] || jc[col] > nnz) fail('sparse column pointers are not monotone.');
    }
    for (let i = 0; i < nnz; i++) if (ir[i] < 0 || ir[i] >= shape[0]) fail('sparse row index exceeds its dimensions.');
    value = { shape, ir: ir.slice(0, nnz), jc, values: values.slice(0, nnz) };
  } else if (arrayClass >= 6 && arrayClass <= 15) {
    value = numbers(cursor.tag(), little);
    if (value.length !== count) fail('numeric data does not match its dimensions.');
    shaped(value, shape);
  } else fail(`unsupported MATLAB array class ${arrayClass}.`);
  cursor.done();
  return { name, value };
}

async function inflate(bytes, budget) {
  if (typeof DecompressionStream !== 'function' || typeof Response !== 'function') {
    fail('this browser does not provide zlib decompression (DecompressionStream).');
  }
  const chunks = [];
  let size = 0;
  try {
    const reader = new Response(bytes).body.pipeThrough(new DecompressionStream('deflate')).getReader();
    try {
      while (true) {
        const item = await reader.read();
        if (item.done) break;
        size += item.value.length;
        if (size > budget.remaining) {
          await reader.cancel();
          fail('decompressed data exceeds the byte limit.');
        }
        chunks.push(item.value);
      }
    } finally { reader.releaseLock(); }
  } catch (error) {
    if (error.message.startsWith('MAT Level 5:')) throw error;
    fail('invalid compressed zlib data.');
  }
  budget.remaining -= size;
  const expanded = new Uint8Array(size);
  let offset = 0;
  for (const chunk of chunks) { expanded.set(chunk, offset); offset += chunk.length; }
  return expanded;
}

/** Read an ArrayBuffer or Uint8Array into a dictionary of named MATLAB variables. */
export async function readMat5(input) {
  const bytes = input instanceof Uint8Array ? input : input instanceof ArrayBuffer ? new Uint8Array(input) : null;
  if (!bytes) fail('expected an ArrayBuffer or Uint8Array.');
  if (bytes.length < 128) fail('truncated Level 5 header.');
  if (bytes.length > MAX_BYTES) fail('file exceeds the byte limit.');
  const marker = String.fromCharCode(bytes[126], bytes[127]);
  if (marker !== 'IM' && marker !== 'MI') fail('invalid endian indicator; Level 7.3/HDF5 is not supported.');
  const little = marker === 'IM';
  const version = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength).getUint16(124, little);
  if (version !== 0x100) fail(`unsupported MAT-file version ${version}.`);
  const variables = {};
  const budget = { remaining: MAX_BYTES };
  async function readElements(data, compressed = false) {
    const cursor = new Cursor(data, little);
    while (cursor.offset < data.length) {
      const element = cursor.tag();
      if (element.type === MI.COMPRESSED) {
        if (compressed) fail('nested compressed data elements are not supported.');
        await readElements(await inflate(element.bytes, budget), true);
      } else {
        const { name, value } = matrix(element, little, 0);
        if (!name || Object.hasOwn(variables, name)) fail('empty or duplicate variable name.');
        own(variables, name, value);
      }
    }
  }
  await readElements(bytes.subarray(128));
  return variables;
}
