/**
 * Small branch-and-bound MILP layer on top of the bounded-variable simplex.
 *
 * It is intentionally minimal: all constraints are equalities plus variable
 * bounds, exactly the form solveLP already accepts. Integer variables are
 * supplied by column index and branched by tightening their bounds to 0/1.
 */

import {
  solveLP, LP_OPTIMAL, LP_INFEASIBLE, LP_UNBOUNDED, LP_ITERATION_LIMIT,
} from './simplex.js';

/**
 * Solve `problem` (as solveLP takes it) with the columns in `binaryIndices`
 * restricted to 0 or 1, by depth-first branch and bound. Options: nodeLimit
 * (20000), timeLimitMs (8000), integralityTolerance, objectiveTolerance, and
 * lpOptions passed on to solveLP.
 *
 * Returns solveLP's result for the best integer point, plus `nodes` explored
 * and `provenOptimal`. When a limit stops the search the status is
 * LP_ITERATION_LIMIT, with the best point found so far if there is one.
 */
export function solveBinaryMILP(problem, binaryIndices, options = {}) {
  const {
    nodeLimit = 20000,
    timeLimitMs = 8000,
    integralityTolerance = 1e-7,
    objectiveTolerance = 1e-8,
  } = options;

  const maximize = problem.maximize !== false;
  const started = now();
  let explored = 0;
  let incumbent = null;
  let limitHit = false;

  const rootLb = Float64Array.from(problem.lb);
  const rootUb = Float64Array.from(problem.ub);
  const stack = [{ lb: rootLb, ub: rootUb }];

  while (stack.length) {
    if (explored >= nodeLimit || now() - started >= timeLimitMs) {
      limitHit = true;
      break;
    }

    const node = stack.pop();
    explored += 1;
    const relaxation = solveLP({
      A: problem.A,
      b: problem.b,
      c: problem.c,
      lb: node.lb,
      ub: node.ub,
      maximize,
    }, options.lpOptions ?? {});

    if (relaxation.status === LP_INFEASIBLE) continue;
    if (relaxation.status === LP_UNBOUNDED) {
      return {
        ...relaxation,
        nodes: explored,
        provenOptimal: false,
        message: 'The LP relaxation of the MILP is unbounded.',
      };
    }
    if (relaxation.status !== LP_OPTIMAL) {
      limitHit = true;
      continue;
    }

    if (incumbent && cannotBeat(relaxation.objective, incumbent.objective, maximize, objectiveTolerance)) {
      continue;
    }

    const branch = chooseFractional(relaxation.x, binaryIndices, integralityTolerance);
    if (!branch) {
      incumbent = relaxation;
      continue;
    }

    const { index, value } = branch;

    // Depth-first search, nearest integer first to obtain incumbents early.
    const lowNode = cloneNode(node);
    lowNode.ub[index] = Math.min(lowNode.ub[index], 0);
    const highNode = cloneNode(node);
    highNode.lb[index] = Math.max(highNode.lb[index], 1);

    if (value >= 0.5) {
      if (lowNode.lb[index] <= lowNode.ub[index]) stack.push(lowNode);
      if (highNode.lb[index] <= highNode.ub[index]) stack.push(highNode);
    } else {
      if (highNode.lb[index] <= highNode.ub[index]) stack.push(highNode);
      if (lowNode.lb[index] <= lowNode.ub[index]) stack.push(lowNode);
    }
  }

  if (!incumbent) {
    return {
      status: limitHit ? LP_ITERATION_LIMIT : LP_INFEASIBLE,
      objective: NaN,
      x: null,
      duals: null,
      reducedCosts: null,
      iterations: 0,
      nodes: explored,
      provenOptimal: !limitHit,
      message: limitHit
        ? `Loopless MILP hit the search limit (${explored} nodes).`
        : 'Loopless MILP has no feasible solution.',
    };
  }

  return {
    ...incumbent,
    status: limitHit ? LP_ITERATION_LIMIT : LP_OPTIMAL,
    nodes: explored,
    provenOptimal: !limitHit,
    incumbentObjective: incumbent.objective,
    message: limitHit
      ? `A loopless solution was found, but optimality was not proven before the limit (${explored} nodes).`
      : '',
  };
}

function chooseFractional(x, binaryIndices, tolerance) {
  let best = null;
  let bestScore = -1;
  for (const index of binaryIndices) {
    const value = x[index];
    const nearest = Math.round(value);
    if (Math.abs(value - nearest) <= tolerance) continue;
    const score = 0.5 - Math.abs(value - 0.5);
    if (score > bestScore) {
      bestScore = score;
      best = { index, value };
    }
  }
  return best;
}

function cannotBeat(bound, incumbent, maximize, tolerance) {
  return maximize
    ? bound <= incumbent + tolerance
    : bound >= incumbent - tolerance;
}

function cloneNode(node) {
  return {
    lb: Float64Array.from(node.lb),
    ub: Float64Array.from(node.ub),
  };
}

function now() {
  return typeof performance !== 'undefined' && performance.now
    ? performance.now()
    : Date.now();
}

export {
  LP_OPTIMAL, LP_INFEASIBLE, LP_UNBOUNDED, LP_ITERATION_LIMIT,
};
