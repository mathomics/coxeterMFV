import test from 'node:test';
import assert from 'node:assert/strict';
import { deflateSync } from 'node:zlib';
import { readMat5 } from '../src/import/mat5.js';

// Created from our own two-metabolite / three-reaction fixture using SciPy
// savemat(..., do_compression=True). No published model data is embedded here.
const scipyFixture = 'TUFUTEFCIDUuMCBNQVQtZmlsZSBQbGF0Zm9ybTogbnQsIENyZWF0ZWQgb246IFRodSBPY3QgIDggMjE6MDI6MjMgMjAyNgAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABSU0PAAAAhgEAAHicnVTNSsNAEJ78VYslehDxmIMPoDdviiDoQQRzEUQkTRYNtEnIj7Q3jz6Cj+DFNxAUvItHH8Gjj+Buukkm09SUTBkyM51v8n2d7ZoA8GwA9PhzlbsCMzNkrsqa8HXu9jRI71jqu2ehx0YJmLz2qFd4leAV5LO6DgOZT7jHkyCRX8GYpWVsQ2WjYRVnKEYhuCiO2X0ZeyxxYz9K/TAANo7SqawL3l+wWLdGeIv+XdSvN+jUUf8Gz44vb5xlcGoNp8LF3nLv02o4DY5Oz3Pc0z+68D676nKu2HU3nJvjHhDOkH2YH9Zl5NjKCu64XvwWYk6fu1V2/76R50GRCx4niEeP8Kf7F3O3y7k770BMzNtH89Za5umy90cTn/k99Ame7k2V/7Q2HH2vKitUv9mCG0j9Rd6kH89rOg+bgM8DwBZ3++MlSFniJ9bnq+UxK4ozNnTmddH9FIb3U/D4hm73kdCH7wt8B9H9NulbIfpEnrDbLPDCHH8I9XOvwGI9BuKH9f0BYC1C2A8AAAAuAAAAeJzjY2BgcABiNiDmAGIeBghghfIZgZgJSoPUFGem56WmIOSBoKEaSAAAWUMEAg8AAAAwAAAAeJzjY2BgcABiNiDmAGJeBghghfIZgZgJSoP4pXnFmel5qSkw9f+BgB1IAwB/PwfxDwAAADMAAAB4nONjYGBwAGI2IOYAYnYGCGCF8hmBmAlKg/hpRYnJJZn5eexQPgPDAXsGBoEDAGmFBbEPAAAANAAAAHic42NgYHAAYjYg5gBiFgYIYIXymaCYEYi5gDg5I7EoMbkktagYok4AqCMxOSkFAHH8Bjs=';

function concat(...items) {
  const result = new Uint8Array(items.reduce((size, item) => size + item.length, 0));
  let offset = 0;
  for (const item of items) { result.set(item, offset); offset += item.length; }
  return result;
}
function numeric(values, type = 5, little = true) {
  const widths = { 4: 2, 5: 4, 6: 4, 9: 8, 12: 8 };
  const methods = { 4: 'setUint16', 5: 'setInt32', 6: 'setUint32', 9: 'setFloat64', 12: 'setBigInt64' };
  const bytes = new Uint8Array(values.length * widths[type]);
  const view = new DataView(bytes.buffer);
  values.forEach((value, i) => view[methods[type]](i * widths[type], value, little));
  return bytes;
}
function tag(type, data, little = true, small = false) {
  const bytes = new Uint8Array(small ? 8 : 8 + data.length + (type === 15 ? 0 : (8 - data.length % 8) % 8));
  const view = new DataView(bytes.buffer);
  if (small) {
    assert.ok(data.length > 0 && data.length <= 4);
    view.setUint16(0, type, little); view.setUint16(2, data.length, little); bytes.set(data, 4);
  } else {
    view.setUint32(0, type, little); view.setUint32(4, data.length, little); bytes.set(data, 8);
  }
  return bytes;
}
function matrix({ name = 'v', cls = 6, shape = [1, 2], type = 9, values = [1, 2], little = true,
  raw, flags = cls, extra = new Uint8Array(), nzmax = 0 } = {}) {
  return tag(14, concat(
    tag(6, numeric([flags, nzmax], 6, little), little),
    tag(5, numeric(shape, 5, little), little),
    tag(1, new TextEncoder().encode(name), little, name.length > 0 && name.length <= 4),
    tag(type, raw ?? numeric(values, type, little), little), extra,
  ), little);
}
function file(data, little = true) {
  const header = new Uint8Array(128);
  header.set(new TextEncoder().encode('MATLAB 5.0 MAT-file, Coxeter synthetic fixture'));
  new DataView(header.buffer).setUint16(124, 0x100, little);
  header[126] = little ? 73 : 77; header[127] = little ? 77 : 73;
  return concat(header, data);
}

test('SciPy compressed synthetic COBRA structures preserve fields, CSC and numeric precision', async () => {
  const variables = await readMat5(Uint8Array.from(Buffer.from(scipyFixture, 'base64')));
  const [model, second] = variables.SyntheticModels;
  assert.deepEqual(model.rxns, ['EX_a', 'R1', 'BIO']);
  assert.deepEqual(model.mets, ['a[e]', 'a[c]']);
  assert.equal(model.description, 'Síntesis μ de prueba');
  assert.deepEqual(model.S.shape, [2, 3]);
  assert.deepEqual(Array.from(model.S.ir), [0, 0, 1, 1]);
  assert.deepEqual(Array.from(model.S.jc), [0, 1, 3, 4]);
  assert.deepEqual(Array.from(model.S.values), [-1, -1, 1, -1]);
  assert.deepEqual(Array.from(model.lb), [-10, 0, 0]);
  assert.deepEqual(model.lb.shape, [3, 1]);
  assert.ok(model.ub instanceof Uint16Array);
  assert.deepEqual(Array.from(model.ub), [1000, 1000, 1000]);
  assert.deepEqual(Array.from(model.b), [0, 0]);
  assert.deepEqual(Array.from(model.c), [0, 0, 1]);
  assert.deepEqual(Array.from(model.rev), [1, 0, 0]);
  assert.equal(model.empty.length, 0);
  assert.equal(second.description, 'segundo');
  assert.deepEqual(second.S.shape, [0, 0]);
  assert.equal(second.S.values.length, 0);
  assert.deepEqual(Array.from(variables.signed), [-2147483648, 123]);
  assert.deepEqual(Array.from(variables.unsigned), [4294967295, 7]);
  assert.deepEqual(Array.from(variables.fraction), [1.5, -2.25]);
  assert.deepEqual(variables.characters, ['ab', 'cd']);
});

test('little and big endian, compact names, ArrayBuffer and offset Uint8Array inputs', async () => {
  for (const little of [true, false]) {
    const bytes = file(matrix({ little, values: [Math.PI, -4.5] }), little);
    const padded = concat(new Uint8Array(3), bytes, new Uint8Array(7));
    const slice = padded.subarray(3, padded.length - 7);
    assert.deepEqual(Array.from((await readMat5(slice)).v), [Math.PI, -4.5]);
    assert.deepEqual(Array.from((await readMat5(bytes.buffer)).v), [Math.PI, -4.5]);
  }
});

test('UTF-16 in either endian and UTF-32 decode without rotating character rows', async () => {
  for (const little of [true, false]) {
    const char = matrix({ little, cls: 4, shape: [1, 2], type: 17, raw: numeric([0x3bc, 0x3b1], 4, little) });
    assert.equal((await readMat5(file(char, little))).v, 'μα');
  }
  const char = matrix({ cls: 4, shape: [1, 2], type: 18, raw: numeric([0x3bc, 0x1f9ea], 6) });
  assert.equal((await readMat5(file(char))).v, 'μ🧪');
});

test('uncompressed variables may follow compressed elements without alignment padding', async () => {
  const compressed = tag(15, deflateSync(matrix({ name: 'one', values: [3, 7] })));
  const doc = await readMat5(file(concat(compressed, matrix({ name: 'two', values: [4, 9] }))));
  assert.deepEqual(Array.from(doc.one), [3, 7]);
  assert.deepEqual(Array.from(doc.two), [4, 9]);
});

test('an empty array MATLAB writes as a matrix element without bytes reads as an empty array', async () => {
  const cell = matrix({ cls: 1, shape: [2, 1], type: 14, raw: new Uint8Array(), extra: matrix({ name: '', shape: [1, 1], values: [5] }) });
  const { v } = await readMat5(file(cell));
  assert.deepEqual([v[0].length, v[0].shape], [0, [0, 0]]);
  assert.deepEqual(Array.from(v[1]), [5]);
});

test('rejects truncated headers, invalid endian, versions, tags and bounded matrix payloads', async () => {
  await assert.rejects(readMat5('not binary'), /expected an ArrayBuffer/);
  await assert.rejects(readMat5(new Uint8Array(127)), /truncated Level 5 header/);
  const header = file(new Uint8Array()); header[126] = 0;
  await assert.rejects(readMat5(header), /invalid endian indicator/);
  const version = file(new Uint8Array()); version[124] = 2;
  await assert.rejects(readMat5(version), /unsupported MAT-file version/);
  await assert.rejects(readMat5(file(new Uint8Array(4))), /truncated data-element tag/);
  const clipped = file(matrix()).slice(0, -1);
  await assert.rejects(readMat5(clipped), /exceeds its container/);
  await assert.rejects(readMat5(file(tag(8, new Uint8Array()))), /unsupported data-element type 8/);
  await assert.rejects(readMat5(file(matrix({ extra: tag(9, numeric([1], 9)) }))), /unexpected trailing matrix data/);
});

test('rejects invalid dimensions, data counts, unsupported arrays and lossy 64-bit integers', async () => {
  await assert.rejects(readMat5(file(matrix({ shape: [-1, 2] }))), /invalid array dimensions/);
  await assert.rejects(readMat5(file(matrix({ shape: [20000001, 1] }))), /element limit/);
  await assert.rejects(readMat5(file(matrix({ shape: [1, 3] }))), /does not match its dimensions/);
  await assert.rejects(readMat5(file(matrix({ flags: 6 | 0x800 }))), /complex arrays/);
  await assert.rejects(readMat5(file(matrix({ cls: 3 }))), /unsupported MATLAB array class 3/);
  await assert.rejects(readMat5(file(matrix({ type: 12, values: [9007199254740992n, 0n] }))), /cannot be represented exactly/);
  await assert.rejects(readMat5(file(matrix({ cls: 4, type: 16, raw: Uint8Array.of(255, 255) }))), /invalid UTF-8/);
});

test('rejects invalid compression, nested compressed streams and duplicate variable names', async () => {
  await assert.rejects(readMat5(file(tag(15, Uint8Array.of(1, 2, 3, 4)))), /invalid compressed zlib data/);
  const nested = tag(15, deflateSync(tag(15, deflateSync(matrix()))));
  await assert.rejects(readMat5(file(nested)), /nested compressed/);
  await assert.rejects(readMat5(file(concat(matrix(), matrix()))), /duplicate variable name/);
});

test('validates sparse CSC pointers and row bounds before exposing a matrix', async () => {
  function sparse(ir, jc, values, shape = [2, 2], nzmax = values.length) {
    const data = concat(tag(5, numeric(jc)), tag(9, numeric(values, 9)));
    return file(matrix({ cls: 5, shape, type: 5, raw: numeric(ir), extra: data, nzmax }));
  }
  const value = (await readMat5(sparse([0, 1], [0, 1, 2], [2, -3]))).v;
  assert.deepEqual(value.shape, [2, 2]);
  assert.deepEqual(Array.from(value.values), [2, -3]);
  await assert.rejects(readMat5(sparse([0, 1], [1, 1, 2], [2, -3])), /invalid sparse column pointers/);
  await assert.rejects(readMat5(sparse([0, 1], [0, 2, 1], [2, -3])), /not monotone/);
  await assert.rejects(readMat5(sparse([0, 2], [0, 1, 2], [2, -3])), /row index exceeds/);
  await assert.rejects(readMat5(sparse([0], [0, 1, 2], [2, -3])), /exceed allocated storage/);
});
