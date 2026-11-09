"""Crossings and tight spots among the drawn branches of one region of a map.

Every branch is sampled along the cubic curve Coxeter draws (from, b1, b2, to),
so cofactor arcs count as they look. Two branches of the same reaction, or two
that share a node (a legitimate meeting point), are not counted as crossing.

Usage: python crossings.py MAP X0 Y0 X1 Y1 [MIN_CLEARANCE]
"""
import json
import math
import sys


def bezier(seg, n=16):
    p0, p3 = seg['from'], seg['to']
    p1, p2 = seg.get('b1', p0), seg.get('b2', p3)
    if seg.get('side') == 'product':
        p1, p2 = seg.get('b2', p0), seg.get('b1', p3)
    out = []
    for i in range(n + 1):
        t = i / n
        a, b, c, d = (1 - t) ** 3, 3 * (1 - t) ** 2 * t, 3 * (1 - t) * t ** 2, t ** 3
        out.append((a * p0['x'] + b * p1['x'] + c * p2['x'] + d * p3['x'],
                    a * p0['y'] + b * p1['y'] + c * p2['y'] + d * p3['y']))
    return out


def cross(p1, p2, p3, p4):
    def o(a, b, c):
        return (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
    return (o(p1, p2, p3) * o(p1, p2, p4) < 0) and (o(p3, p4, p1) * o(p3, p4, p2) < 0)


def seg_dist(p, a, b):
    dx, dy = b[0] - a[0], b[1] - a[1]
    L2 = dx * dx + dy * dy
    t = 0 if L2 == 0 else max(0, min(1, ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / L2))
    return math.hypot(p[0] - a[0] - t * dx, p[1] - a[1] - t * dy)


def main(path, x0, y0, x1, y1, min_clear=22):
    x0, y0, x1, y1, min_clear = map(float, (x0, y0, x1, y1, min_clear))
    m = json.load(open(path, encoding='utf-8'))['map']
    N = {n['id']: n for n in m['nodes']}
    inside = lambda p: x0 <= p[0] <= x1 and y0 <= p[1] <= y1
    curves = []
    for s in m['segments']:
        if s['kind'] in ('axis', 'transport-lane'):
            continue
        pts = bezier(s)
        if any(inside(p) for p in pts):
            curves.append((s, pts))
    found = []
    for i in range(len(curves)):
        si, pi = curves[i]
        for j in range(i + 1, len(curves)):
            sj, pj = curves[j]
            if si['reactionId'] == sj['reactionId']:
                continue
            if si.get('nodeId') and si.get('nodeId') == sj.get('nodeId'):
                continue
            hit = any(cross(pi[a], pi[a + 1], pj[b], pj[b + 1])
                      for a in range(len(pi) - 1) for b in range(len(pj) - 1))
            if hit:
                found.append((si['reactionId'], si.get('nodeId', '').split(':')[-1],
                              sj['reactionId'], sj.get('nodeId', '').split(':')[-1]))
    print(f'{len(curves)} branches in the region; {len(found)} crossings')
    for f in found:
        print('   cross:', f)
    # nodes too close to a branch they are not on
    tight = []
    for n in m['nodes']:
        p = (n['x'], n['y'])
        if not inside(p):
            continue
        for s, pts in curves:
            if s.get('nodeId') == n['id']:
                continue
            if any(s2.get('nodeId') == n['id'] for s2 in m['segments'] if s2['reactionId'] == s['reactionId']):
                continue
            d = min(seg_dist(p, pts[k], pts[k + 1]) for k in range(len(pts) - 1))
            if d < min_clear + n['r']:
                tight.append((n['id'].split(':')[-1], n['id'].split(':')[1] if n['id'].count(':') > 1 else '', s['reactionId'], round(d)))
    print(f'{len(tight)} nodes closer than {min_clear:.0f} + radius to a branch that is not theirs')
    for t in tight:
        print('   tight:', t)
    return len(found), len(tight)


if __name__ == '__main__':
    main(*sys.argv[1:7])
