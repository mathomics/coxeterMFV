#!/usr/bin/env node
/**
 * Drive the Figure 4 sweep one point per process, with a time budget each.
 *
 *   node tools/sweep-figure4.mjs --dir=<model9 folder> --out=sweep.json \
 *       [--points=1,2,3] [--budget=900] [--parallel=3]
 *
 * Why a process per point. Some points of this sweep are tractable in three
 * minutes and others are not tractable at all: at certain maximum fiber uptake
 * rates the LP near the community's maximum growth rate is degenerate enough
 * that GLPK's simplex runs for hours, and glpk.js exposes no way to interrupt it
 * from inside the process. Measured: FUR 6 took 163 s and FUR 17 took 238 s,
 * while FUR 7, 10 and 29 each ran past 100 minutes without finishing.
 *
 * In one process a point like that blocks every point after it. Three
 * two-hour runs returned seven, one and two points respectively, and what
 * stopped them was always a single slow point rather than any accumulated
 * state. Given its own process, a point can be given a wall-clock budget and
 * killed when it exceeds it, and the sweep carries on. A point that was killed
 * is recorded as not computed, with its budget, so the gap is visible as a gap
 * rather than absent from the file.
 */
import { spawn } from 'node:child_process';
import { readFile, writeFile, unlink } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import { tmpdir } from 'node:os';
import { FIBER_SWEEP } from '../src/communities/gut9-figure4.js';

const here = dirname(fileURLToPath(import.meta.url));
const args = Object.fromEntries(process.argv.slice(2).map((a) => {
  const [k, v = 'true'] = a.replace(/^--/, '').split('=');
  return [k, v];
}));

const dir = args.dir ?? '.';
const out = args.out ?? 'gut9-figure4.json';
const budget = Number(args.budget ?? 900);
const parallel = Math.max(1, Number(args.parallel ?? 3));
const points = args.points && args.points !== 'true'
  ? args.points.split(',').map(Number)
  : FIBER_SWEEP.bacteroides;

const results = [];
const skipped = [];

/** Run one point in its own process, and give up on it after `budget` seconds. */
function runPoint(point) {
  return new Promise((resolve) => {
    const scratch = join(tmpdir(), `fig4-point-${point}-${process.pid}.json`);
    const child = spawn(process.execPath, [
      join(here, 'run-gut9-figure4.mjs'),
      `--dir=${dir}`, `--points=${point}`, `--out=${scratch}`,
    ], { stdio: ['ignore', 'ignore', 'inherit'] });

    const started = Date.now();
    const timer = setTimeout(() => {
      child.kill('SIGKILL');
    }, budget * 1000);

    child.on('exit', async (code, signal) => {
      clearTimeout(timer);
      const seconds = (Date.now() - started) / 1000;
      let row = null;
      try {
        row = JSON.parse(await readFile(scratch, 'utf8')).results?.[0] ?? null;
      } catch { /* the point never wrote anything */ }
      await unlink(scratch).catch(() => {});

      if (row && !row.status) {
        results.push(row);
        console.log(`FUR ${String(point).padStart(2)}: mu = ${row.growthRate.toFixed(5)} /h`
          + `  fiber ${row.fiberUptake.toFixed(3)}  FDS ${row.fdsExport.toFixed(3)}`
          + `  (${seconds.toFixed(0)} s)`);
      } else {
        skipped.push({ point, seconds: Math.round(seconds), budget, signal, code });
        console.log(`FUR ${String(point).padStart(2)}: not computed`
          + ` — ${signal === 'SIGKILL' ? `exceeded its ${budget} s budget` : `exited ${code}`}`);
      }
      await save();
      resolve();
    });
  });
}

async function save() {
  const ordered = [...results].sort((a, b) => a.bacteroides - b.bacteroides);
  const computed = ordered.map((r) => r.bacteroides);
  await writeFile(out, `${JSON.stringify({
    source: 'simFigure4.m on model9.xlsx + model9_rxns_params.txt',
    sweep: FIBER_SWEEP,
    budgetSeconds: budget,
    computedPoints: computed,
    missingPoints: FIBER_SWEEP.bacteroides.filter((x) => !computed.includes(x)),
    notComputed: [...skipped].sort((a, b) => a.point - b.point),
    results: ordered,
  }, null, 2)}\n`);
}

// A small pool, so a slow point holds up only its own slot.
const queue = [...points];
await Promise.all(Array.from({ length: Math.min(parallel, queue.length) }, async () => {
  while (queue.length) await runPoint(queue.shift());
}));

await save();
console.log(`\n${out}: ${results.length} computed, ${skipped.length} over budget`);
