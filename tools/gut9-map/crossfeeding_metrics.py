"""Length and crossings of the cross-feeding links of a gut9 map.

A link joins the medium node of a donor's exchange to the medium node of a
receiver's exchange of the same metabolite, weighted by the flow allocated to
that pair (see build_phase2b.py). This is how the map would read if those
links were drawn; it scores the placement of cells and exchanges.

Usage: python crossfeeding_metrics.py MAP PAIRS_JSON [MAP ...]
"""
import json
import math
import sys

sys.path.insert(0, __file__.rsplit('\\', 1)[0].rsplit('/', 1)[0])
from build_phase2b import seg_cross, seg_hits_rect  # noqa: E402


def metrics(path, pairs):
    m = json.load(open(path, encoding='utf-8'))['map']
    N = {n['id']: n for n in m['nodes']}
    seg = {}
    for s in m['segments']:
        n = N.get(s.get('nodeId'))
        if n and n['kind'] == 'medium':
            seg[s['reactionId']] = (n['x'], n['y'])
    cells = {c['id']: c['rect'] for c in m['cells']}
    rx = {r['id']: r['cellId'] for r in m['reactions']}
    links = [(seg[a], seg[b], w, rx[a], rx[b]) for a, b, w in pairs]
    total = sum(w for *_, w, _, _ in links)
    length = sum(math.hypot(p[0] - q[0], p[1] - q[1]) * w for p, q, w, _, _ in links) / total
    plain = sum(math.hypot(p[0] - q[0], p[1] - q[1]) for p, q, *_ in links) / len(links)
    crossings = wcross = 0
    for i in range(len(links)):
        for j in range(i + 1, len(links)):
            p1, p2, w1, *_ = links[i]
            p3, p4, w2, *_ = links[j]
            if seg_cross(p1, p2, p3, p4):
                crossings += 1
                wcross += math.sqrt(w1 * w2)
    through = sum(1 for p, q, w, a, b in links
                  if any(seg_hits_rect(p, q, r) for c, r in cells.items() if c not in (a, b)))
    own = sum(1 for p, q, w, a, b in links
              if seg_hits_rect(p, q, cells[a], -5) or seg_hits_rect(p, q, cells[b], -5))
    return {'links': len(links), 'mean length (flow-weighted)': round(length), 'mean length': round(plain),
            'crossings': crossings, 'crossings (flow-weighted)': round(wcross * 1000, 1),
            'through a third cell': through, 'through its own cells': own}


if __name__ == '__main__':
    pairs = json.load(open(sys.argv[2]))['pairs']
    for p in [sys.argv[1]] + sys.argv[3:]:
        print(p)
        for k, v in metrics(p, pairs).items():
            print(f'   {k:28} {v}')
