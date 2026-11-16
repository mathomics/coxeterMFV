/**
 * Flux data files: a solution written as reaction identifiers and flux values,
 * and read back onto a model. The JSON form is Escher's reaction data, one
 * object such as `{"PGI": 4.86, "PFK": 7.48}`, so files move between Escher,
 * COBRApy and Coxeter-MFV. A CSV with a reaction column and a flux column (the
 * Flux table export, for instance) is read too. This module only reads, writes
 * and matches identifiers: it never changes a model or solves anything.
 */

import { splitId } from './community.js';

/** Header names of the reaction identifier column of a flux CSV, compared in lower case. */
const ID_HEADERS = ['reaction', 'reaction_id', 'rxn', 'id'];

/** Header names of the flux column of a flux CSV, compared in lower case. */
const FLUX_HEADERS = ['flux', 'fluxes', 'value'];

/**
 * Read a flux data file: Escher reaction data JSON, or a CSV (or tab-separated
 * file) with a reaction column and a flux column. Values that are not finite
 * numbers are skipped.
 *
 * @param {string} text
 * @returns {Map<string, number>} flux by reaction id, in file order
 */
export function parseFluxData(text) {
  const trimmed = String(text ?? '').replace(/^﻿/, '').trim();
  if (!trimmed) throw new Error('The file is empty.');
  let data;
  if (trimmed.startsWith('{') || trimmed.startsWith('[')) {
    data = fluxDataFromJSON(trimmed);
  } else {
    data = fluxDataFromCSV(trimmed);
  }
  if (!data.size) throw new Error('No reaction fluxes were found in this file.');
  return data;
}

/** Escher reaction data: one object of fluxes, or a list holding one. */
function fluxDataFromJSON(text) {
  let parsed;
  try {
    parsed = JSON.parse(text);
  } catch (error) {
    throw new Error(`Not a readable JSON file (${error.message}).`);
  }
  if (Array.isArray(parsed)) {
    if (parsed.length > 1) {
      throw new Error(`This file holds ${parsed.length} data sets, as Escher uses for a comparison. Import one set at a time.`);
    }
    parsed = parsed[0];
  }
  if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) {
    throw new Error('Flux data JSON must be an object of reaction ids and fluxes, such as {"PGI": 4.86}.');
  }
  const data = new Map();
  for (const [id, value] of Object.entries(parsed)) {
    const number = toNumber(value);
    if (Number.isFinite(number)) {
      data.set(id, number);
    }
  }
  return data;
}

/** A table with a reaction column and a flux column, found by header or else the first two columns. */
function fluxDataFromCSV(text) {
  const rows = parseDelimitedRows(text);
  const header = rows[0].map((cell) => cell.trim().toLowerCase());
  let idIndex = header.findIndex((name) => ID_HEADERS.includes(name));
  let fluxIndex = header.findIndex((name) => FLUX_HEADERS.includes(name));
  let first = 1;
  if (idIndex < 0 || fluxIndex < 0) {
    // No header: the first column names the reaction and the second holds its flux.
    if (rows[0].length < 2 || !Number.isFinite(toNumber(rows[0][1]))) {
      throw new Error('The table needs a reaction column and a flux column.');
    }
    idIndex = 0;
    fluxIndex = 1;
    first = 0;
  }
  const data = new Map();
  for (const row of rows.slice(first)) {
    const id = String(row[idIndex] ?? '').trim();
    const number = toNumber(row[fluxIndex]);
    if (id && Number.isFinite(number)) {
      data.set(id, number);
    }
  }
  return data;
}

/**
 * Split delimited text into rows of cells. The delimiter is a tab when the
 * first line has one, else a semicolon when it has a semicolon and no comma,
 * else a comma. Double quotes enclose cells that hold the delimiter, and two
 * double quotes inside them stand for one.
 *
 * @param {string} text
 * @returns {string[][]}
 */
export function parseDelimitedRows(text) {
  const firstLine = text.split(/\r?\n/, 1)[0];
  let delimiter = ',';
  if (firstLine.includes('\t')) {
    delimiter = '\t';
  } else if (firstLine.includes(';') && !firstLine.includes(',')) {
    delimiter = ';';
  }
  const rows = [];
  let row = [];
  let cell = '';
  let quoted = false;
  for (let i = 0; i < text.length; i += 1) {
    const char = text[i];
    if (quoted) {
      if (char === '"' && text[i + 1] === '"') {
        cell += '"';
        i += 1;
      } else if (char === '"') {
        quoted = false;
      } else {
        cell += char;
      }
      continue;
    }
    if (char === '"') {
      quoted = true;
    } else if (char === delimiter) {
      row.push(cell);
      cell = '';
    } else if (char === '\n' || char === '\r') {
      if (char === '\r' && text[i + 1] === '\n') i += 1;
      row.push(cell);
      cell = '';
      if (row.some((value) => value.trim() !== '')) rows.push(row);
      row = [];
    } else {
      cell += char;
    }
  }
  row.push(cell);
  if (row.some((value) => value.trim() !== '')) rows.push(row);
  return rows;
}

/** A number from a JSON value or a table cell, or NaN. Empty text is not zero. */
function toNumber(value) {
  if (typeof value === 'number') return value;
  if (typeof value !== 'string' || value.trim() === '') return NaN;
  return Number(value.trim());
}

/**
 * Which model reaction each value belongs to. An identifier matches the model
 * reaction with the same id, or, in a community, a species' copy of it
 * (`PGI` matches `Ec1@PGI`) when exactly one species has that reaction.
 *
 * @param {Map<string, number>} data flux by reaction id, as parseFluxData returns it
 * @param {Iterable<string>} reactionIds the model's reaction ids
 * @returns {{ fluxes: Map<string, number>, unmatched: string[] }}
 */
export function matchFluxData(data, reactionIds) {
  const ids = new Set(reactionIds);
  const copies = new Map();
  for (const rid of ids) {
    const { species, id } = splitId(rid);
    if (!species) continue;
    if (!copies.has(id)) {
      copies.set(id, []);
    }
    copies.get(id).push(rid);
  }
  const fluxes = new Map();
  const unmatched = [];
  for (const [id, value] of data) {
    if (ids.has(id)) {
      fluxes.set(id, value);
      continue;
    }
    const candidates = copies.get(id);
    if (candidates?.length === 1) {
      fluxes.set(candidates[0], value);
    } else {
      unmatched.push(id);
    }
  }
  return { fluxes, unmatched };
}

/**
 * Fluxes as Escher reaction data JSON, keyed by the model's reaction ids.
 *
 * @param {Map<string, number>} fluxes
 * @returns {string}
 */
export function fluxDataJSON(fluxes) {
  return JSON.stringify(Object.fromEntries(fluxes), null, 2);
}
