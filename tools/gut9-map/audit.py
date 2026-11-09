"""Whole-map audit (R55, R56): crossings and crowding, cell by cell.

Every branch is taken as the curve Coxeter draws. Two branches of one
reaction, or two that meet at a node they share, do not count as crossing.
A node closer than MIN_CLEAR (plus its radius) to a branch that is not its own
counts as crowded.

Usage: python audit.py MAP [MIN_CLEAR]
Exit status 1 if any crossing or crowded node is found.
"""
import sys
from collections import Counter, defaultdict
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from coxmap import CoxMap, crosses, load, pdist, seg_pieces  # noqa: E402


def audit(cm, min_clear=22):
    M = cm.M
    owner = {n['id']: set() for n in M['nodes']}
    for s in M['segments']:
        if s.get('nodeId'):
            owner.setdefault(s['nodeId'], set()).add(s['reactionId'])
    cell_of = {r['id']: r['cellId'] for r in M['reactions']}
    found = defaultdict(set)
    for s in M['segments']:
        if s['kind'] in ('axis', 'transport-lane') or not s.get('nodeId'):
            continue
        for a, b in seg_pieces(s):
            for p, q, rid, nid in cm.grid.near_pieces(a[0], a[1], b[0], b[1], 2):
                if rid == s['reactionId'] or (nid and nid == s['nodeId']):
                    continue
                if crosses(a, b, p, q):
                    key = tuple(sorted([(s['reactionId'], s['nodeId']), (rid, nid or '')]))
                    found[cell_of.get(s['reactionId'])].add(key)
    crowded = defaultdict(set)
    for n in M['nodes']:
        p = (n['x'], n['y'])
        for a, b, rid, nid in cm.grid.near_pieces(p[0], p[1], p[0], p[1], min_clear + 20):
            if rid in owner.get(n['id'], ()) or nid == n['id']:
                continue
            if pdist(p, a, b) < min_clear + n.get('r', 13):
                crowded[n.get('cellId') or 'medium'].add((n['id'], rid))
    return found, crowded


def main(path, min_clear=22):
    cm = CoxMap(load(path))
    found, crowded = audit(cm, float(min_clear))
    kinds = Counter((r['cellId'], r['kind']) for r in cm.M['reactions'])
    cells = sorted({c for c, _ in kinds})
    print(f'{path}: {len(cm.M["reactions"])} reactions')
    bad = 0
    for c in cells:
        n_ex = kinds[(c, 'transfer')]
        n_in = sum(v for (cc, k), v in kinds.items() if cc == c and k != 'transfer')
        print(f'  {c}: {n_ex} exchanges, {n_in} other reactions; crossings {len(found.get(c, ()))}, crowded {len(crowded.get(c, ()))}')
        for f in sorted(found.get(c, ()))[:5]:
            print('     cross', f)
        for f in sorted(crowded.get(c, ()))[:5]:
            print('     crowded', f)
        bad += len(found.get(c, ())) + len(crowded.get(c, ()))
    if crowded.get('medium'):
        print(f'  medium: crowded {len(crowded["medium"])}', sorted(crowded['medium'])[:3])
        bad += len(crowded['medium'])
    return bad


if __name__ == '__main__':
    sys.exit(1 if main(*sys.argv[1:3]) else 0)
