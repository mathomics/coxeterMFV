/**
 * Gene rules (GPR): whether a reaction keeps an enzyme once genes are deleted,
 * and which reactions a set of gene knockouts closes. Rules are parsed, never
 * evaluated as code.
 */

const TOKENS = /\(|\)|[^\s()]+/g;
const OPERATOR = /^(and|or)$/i;

/**
 * Evaluate a GPR rule against a set of deleted gene ids. Returns true when the
 * reaction stays functional, so an alternate isozyme keeps it on after a
 * deletion. An empty rule is always functional; a malformed one throws.
 */
export function evaluateGeneRule(rule, deleted = new Set()) {
  if (!String(rule ?? '').trim()) return true;
  const tokens = String(rule).match(TOKENS) ?? [];
  let index = 0;
  const atom = () => {
    const token = tokens[index++];
    if (!token) throw new Error('Incomplete gene rule.');
    if (token === '(') {
      const value = or();
      if (tokens[index++] !== ')') throw new Error('Unbalanced gene rule.');
      return value;
    }
    if (token === ')' || OPERATOR.test(token)) throw new Error('Invalid gene rule.');
    return !deleted.has(token);
  };
  const and = () => {
    let value = atom();
    while (/^and$/i.test(tokens[index] ?? '')) { index++; const next = atom(); value = value && next; }
    return value;
  };
  const or = () => {
    let value = and();
    while (/^or$/i.test(tokens[index] ?? '')) { index++; const next = and(); value = value || next; }
    return value;
  };
  const result = or();
  if (index !== tokens.length) throw new Error('Unexpected token in gene rule.');
  return result;
}

/** The gene ids a rule names, once each, in the order they first appear. */
export function geneRuleGenes(rule) {
  const genes = [];
  for (const token of String(rule ?? '').match(TOKENS) ?? []) {
    if (token === '(' || token === ')' || OPERATOR.test(token) || genes.includes(token)) continue;
    genes.push(token);
  }
  return genes;
}

/**
 * The reactions a set of deleted genes closes: each reaction whose rule names
 * one of them and is false without them, with the deleted genes its rule
 * names. A rule that cannot be read is left open and listed in `unreadable`.
 *
 * @param {Iterable<[string, string]>} rules [reaction id, gene rule] pairs
 * @param {Set<string>} deleted gene ids
 * @returns {{closed: Map<string, string[]>, unreadable: string[]}}
 */
export function geneKnockoutReactions(rules, deleted) {
  const closed = new Map();
  const unreadable = [];
  if (!deleted?.size) return { closed, unreadable };
  for (const [id, rule] of rules) {
    const named = geneRuleGenes(rule).filter((gene) => deleted.has(gene));
    if (!named.length) continue;
    try {
      if (!evaluateGeneRule(rule, deleted)) closed.set(id, named);
    } catch {
      unreadable.push(id);
    }
  }
  return { closed, unreadable };
}
