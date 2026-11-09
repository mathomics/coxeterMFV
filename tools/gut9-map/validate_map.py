"""Structural and geometric checks for a gut9 Coxeter map.

Usage: python validate_map.py MAP [REFERENCE_MAP]
With a reference map, also checks that reaction ids, equations, bounds and
node ids are unchanged (geometry-only edits).
"""
import json
import math
import statistics
import sys
from collections import Counter


def load(path):
    return json.load(open(path, encoding='utf-8'))['map']


def inside(r, x, y, pad=0):
    return r['x'] + pad <= x <= r['x'] + r['w'] - pad and r['y'] + pad <= y <= r['y'] + r['h'] - pad


def where(m, x, y):
    best = None
    for c in m['cells']:
        for k in c['compartments']:
            if inside(k['rect'], x, y) and (best is None or k['depth'] > best[2]):
                best = (c['id'], k['id'], k['depth'])
    return (best[0], best[1]) if best else (None, 's')


def main(path, ref=None):
    m = load(path)
    N = {n['id']: n for n in m['nodes']}
    R = {r['id']: r for r in m['reactions']}
    errors = []

    # unique ids
    for kind, items in (('node', m['nodes']), ('reaction', m['reactions'])):
        dup = [k for k, v in Counter(i['id'] for i in items).items() if v > 1]
        if dup:
            errors.append(f'duplicate {kind} ids: {dup[:5]}')

    # every node in its own compartment (nodes need their radius inside too)
    for n in m['nodes']:
        want = (n.get('cellId'), n['compartment']) if n.get('cellId') else (None, 's')
        got = where(m, n['x'], n['y'])
        if got != want:
            errors.append(f'node {n["id"]} in {got}, belongs to {want}')
            continue
        if n.get('cellId'):
            comp = next(k for k in next(c for c in m['cells'] if c['id'] == n['cellId'])['compartments']
                        if k['id'] == n['compartment'])
            if not inside(comp['rect'], n['x'], n['y'], n['r']):
                errors.append(f'node {n["id"]} touches the edge of {n["compartment"]}')
    # inner compartments must not swallow nodes of the outer one
    for r in m['reactions']:
        if not r.get('onMembrane') and len(r['compartments']) == 1:
            if where(m, r['x'], r['y']) != (r['cellId'], r['compartments'][0]):
                errors.append(f'reaction {r["id"]} centre outside its compartment')

    # segments: reaction and node exist, the node end sits on the node
    for s in m['segments']:
        if s['reactionId'] not in R:
            errors.append(f'segment of missing reaction {s["reactionId"]}')
        if 'nodeId' in s:
            n = N.get(s['nodeId'])
            if not n:
                errors.append(f'segment to missing node {s["nodeId"]}')
                continue
            end = s['from'] if s['side'] == 'substrate' else s['to']
            if math.hypot(end['x'] - n['x'], end['y'] - n['y']) > 0.5:
                errors.append(f'segment of {s["reactionId"]} does not reach {s["nodeId"]}')

    # collisions between distinct nodes
    pts = sorted(m['nodes'], key=lambda n: n['x'])
    close = []
    for i, a in enumerate(pts):
        for b in pts[i + 1:]:
            if b['x'] - a['x'] > 40:
                break
            d = math.hypot(a['x'] - b['x'], a['y'] - b['y'])
            if d < a['r'] + b['r'] + 6:
                close.append((a['id'], b['id'], round(d, 1)))
    for c in close:
        errors.append(f'nodes overlap: {c}')

    # lengths
    stats = {'primary': [], 'currency': [], 'exchange': []}
    for s in m['segments']:
        if 'nodeId' not in s or s['nodeId'] not in N:
            continue
        n, r = N[s['nodeId']], R[s['reactionId']]
        k = 'exchange' if r['kind'] == 'transfer' else ('currency' if n.get('visualRole') == 'secondary' else 'primary')
        stats[k].append(math.hypot(n['x'] - r['x'], n['y'] - r['y']))
    axes = [math.hypot(r['axis']['a1']['x'] - r['axis']['a2']['x'], r['axis']['a1']['y'] - r['axis']['a2']['y']) / 2
            for r in m['reactions']]

    if ref:
        q = load(ref)
        if {r['id'] for r in q['reactions']} != set(R):
            errors.append('reaction ids differ from the reference')
        if {n['id'] for n in q['nodes']} != set(N):
            errors.append('node ids differ from the reference')
        for r in q['reactions']:
            if r['id'] in R and (r['equation'] != R[r['id']]['equation'] or r['bounds'] != R[r['id']]['bounds']):
                errors.append(f'equation or bounds changed: {r["id"]}')
        a = Counter((s['reactionId'], s.get('nodeId'), s.get('coefficient')) for s in q['segments'])
        b = Counter((s['reactionId'], s.get('nodeId'), s.get('coefficient')) for s in m['segments'])
        if a != b:
            errors.append(f'segment connectivity differs from the reference: {list((a - b).items())[:3]} / {list((b - a).items())[:3]}')

    print(f'{path}: {len(m["reactions"])} reactions, {len(m["nodes"])} nodes, {len(m["segments"])} segments')
    for k, v in stats.items():
        if v:
            print(f'  {k:9} node-reaction distance median {statistics.median(v):7.1f}  max {max(v):7.1f}  (n={len(v)})')
    print(f'  axis half-length median {statistics.median(axes):.1f}')
    print(f'  errors: {len(errors)}')
    for e in errors[:40]:
        print('   -', e)
    return len(errors)


if __name__ == '__main__':
    sys.exit(1 if main(*sys.argv[1:3]) else 0)
