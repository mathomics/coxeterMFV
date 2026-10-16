"""Phase 2b: place the species and their exchanges to shorten and uncross the
cross-feeding links, without changing any reaction, node id or connection.

1. Cross-feeding weights. In the scenario's flux file each species' fluxes are
   per gram of that species (its biomass flux equals mu), so the flow through the
   shared medium is X_k * v_k, with X_k = EX_biomassK(u) / mu. For each medium
   metabolite the transferred amount min(secreted, taken up) is shared out among
   donor/receiver pairs in proportion to their flows (a display allocation, not
   an identified pairing; rule R41). H+ and H2O are left out.
2. Species layout. Every assignment of the 9 species to the 9 slots of the 3x3
   grid is scored (9! = 362 880): sum of weight x distance between cell centres,
   plus penalties for weighted link crossings and for links that pass through a
   third cell. The best one is used; each cell is moved with everything in it.
3. Exchanges. An exchange with partners (species that move the same metabolite
   in the opposite direction) goes where the ray from its cell centre towards
   the flow-weighted centre of its partners leaves the cell. Exchanges on one
   edge keep that order and are spread to a minimum gap. Exchanges without
   partners fill the largest free gaps, preferring edges that face away from
   the other cells. Bt's hand-drawn exchanges (pectin, starch) stay where they
   are, and the part of Bt's top edge under its hand-drawn extracellular
   reactions is kept free.

Usage: python build_phase2b.py IN_MAP FLUXES_CSV OUT_MAP
"""
import csv
import itertools
import json
import math
import sys
from collections import defaultdict

import numpy as np

EX_REACH = 130
AXIS_HALF = 17
MIN_GAP = 300          # between exchanges on one edge
CORNER = 500           # keep exchanges this far from a cell corner
SLOT_W, SLOT_H = 15000, 14000
COL_X = [1200, 19700, 38200]
ROW_Y = [1200, 18200, 35200]
CROSS_PENALTY = 0.5    # x geometric mean of the two link weights x mean slot pitch
THROUGH_PENALTY = 1.0  # x link weight x mean slot pitch
SKIP = {'M@h_s', 'M@h2o_s'}
FIXED = {'Bt@EX_pect[u]tr_Bt', 'Bt@EX_starch1200[u]tr_Bt'}
BRANCH = (0.34, 0.72)


def r2(v):
    return round(v, 2)


def pt(x, y):
    return {'x': r2(x), 'y': r2(y)}


def centre(r):
    return (r['x'] + r['w'] / 2, r['y'] + r['h'] / 2)


def seg_cross(p1, p2, p3, p4):
    def o(a, b, c):
        return (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
    if len({p1, p2, p3, p4}) < 4:
        return False      # links sharing an end do not cross
    return (o(p1, p2, p3) * o(p1, p2, p4) < 0) and (o(p3, p4, p1) * o(p3, p4, p2) < 0)


def seg_hits_rect(p, q, r, pad=0):
    """Does segment pq pass through rect r (Liang-Barsky)?"""
    x0, y0, x1, y1 = r['x'] - pad, r['y'] - pad, r['x'] + r['w'] + pad, r['y'] + r['h'] + pad
    dx, dy = q[0] - p[0], q[1] - p[1]
    t0, t1 = 0.0, 1.0
    for pk, qk in ((-dx, p[0] - x0), (dx, x1 - p[0]), (-dy, p[1] - y0), (dy, y1 - p[1])):
        if pk == 0:
            if qk < 0:
                return False
            continue
        t = qk / pk
        if pk < 0:
            t0 = max(t0, t)
        else:
            t1 = min(t1, t)
        if t0 > t1:
            return False
    return True


def ray_exit(rect, c, target):
    """Point where the ray from c towards target leaves rect, and its edge."""
    dx, dy = target[0] - c[0], target[1] - c[1]
    best = None
    for edge, (nx, ny) in (('left', (-1, 0)), ('right', (1, 0)), ('top', (0, -1)), ('bottom', (0, 1))):
        if nx and dx * nx > 0:
            t = ((rect['x'] if nx < 0 else rect['x'] + rect['w']) - c[0]) / dx
        elif ny and dy * ny > 0:
            t = ((rect['y'] if ny < 0 else rect['y'] + rect['h']) - c[1]) / dy
        else:
            continue
        if best is None or t < best[0]:
            best = (t, edge)
    t, edge = best
    return edge, (c[0] + dx * t, c[1] + dy * t)


NORMAL = {'left': (-1, 0), 'right': (1, 0), 'top': (0, -1), 'bottom': (0, 1)}


def edge_span(rect, edge):
    if edge in ('left', 'right'):
        return rect['y'] + CORNER, rect['y'] + rect['h'] - CORNER
    return rect['x'] + CORNER, rect['x'] + rect['w'] - CORNER


def edge_point(rect, edge, s):
    if edge == 'left':
        return rect['x'], s
    if edge == 'right':
        return rect['x'] + rect['w'], s
    if edge == 'top':
        return s, rect['y']
    return s, rect['y'] + rect['h']


def spread(wanted, lo, hi, gap):
    """Positions in the order of `wanted`, at least `gap` apart, inside [lo, hi],
    as close to `wanted` as that allows (forward/backward passes)."""
    n = len(wanted)
    if n == 0:
        return []
    if (n - 1) * gap > hi - lo:
        gap = (hi - lo) / max(n - 1, 1)
    p = [min(max(w, lo), hi) for w in wanted]
    for i in range(1, n):
        p[i] = max(p[i], p[i - 1] + gap)
    p[-1] = min(p[-1], hi)
    for i in range(n - 2, -1, -1):
        p[i] = min(p[i], p[i + 1] - gap)
    p[0] = max(p[0], lo)
    for i in range(1, n):
        p[i] = max(p[i], p[i - 1] + gap)
    return p


def main(in_path, flux_path, out_path):
    doc = json.load(open(in_path, encoding='utf-8'))
    M = doc['map']
    N = {n['id']: n for n in M['nodes']}
    R = {r['id']: r for r in M['reactions']}
    segs_of = defaultdict(list)
    for s in M['segments']:
        segs_of[s['reactionId']].append(s)
    cells = {c['id']: c for c in M['cells']}
    SP = [c['id'] for c in M['cells']]

    rows = list(csv.DictReader(open(flux_path, encoding='utf-8')))
    flux = {(r['species'], r['local_id']): float(r['flux']) for r in rows}
    mu = next(float(r['flux']) for r in rows if r['species'] == 'Bt' and r['local_id'].startswith('EX_biomassBt'))
    X = {r['reaction'][len('EX_biomass'):-3]: float(r['flux']) / mu
         for r in rows if r['reaction'].startswith('EX_biomass') and r['reaction'].endswith('(u)')}

    # ------------------------------------------------ exchanges and their flows
    ex = {}   # reaction id -> (medium node, medium mergedId, aggregate flow)
    for r in M['reactions']:
        if r['kind'] != 'transfer':
            continue
        med = [N[s['nodeId']] for s in segs_of[r['id']] if N.get(s.get('nodeId'), {}).get('kind') == 'medium']
        v = flux.get((r['cellId'], r['localId']), 0.0) * X.get(r['cellId'], 0.0)
        ex[r['id']] = (med[0], med[0]['mergedId'], v)
    by_met = defaultdict(list)
    for rid, (_, mid, v) in ex.items():
        by_met[mid].append((rid, v))

    pair = defaultdict(float)      # (exchange a, exchange b) -> allocated flow
    for mid, lst in by_met.items():
        if mid in SKIP:
            continue
        S = [(rid, v) for rid, v in lst if v > 0]
        U = [(rid, -v) for rid, v in lst if v < 0]
        s, u = sum(v for _, v in S), sum(v for _, v in U)
        if not S or not U:
            continue
        T = min(s, u)
        for a, va in S:
            for b, vb in U:
                if R[a]['cellId'] != R[b]['cellId']:
                    pair[(a, b)] += T * va / s * vb / u
    W = np.zeros((len(SP), len(SP)))
    for (a, b), w in pair.items():
        i, j = SP.index(R[a]['cellId']), SP.index(R[b]['cellId'])
        W[i, j] += w
        W[j, i] += w

    # ------------------------------------------------------- species layout
    slots = [(cx + SLOT_W / 2, ry + SLOT_H / 2) for ry in ROW_Y for cx in COL_X]
    slot_rect = [{'x': x - SLOT_W / 2, 'y': y - SLOT_H / 2, 'w': SLOT_W, 'h': SLOT_H} for x, y in slots]
    pitch = (COL_X[1] - COL_X[0] + ROW_Y[1] - ROW_Y[0]) / 2
    S9 = range(9)
    pidx = {}
    for k, (i, j) in enumerate(itertools.combinations(S9, 2)):
        pidx[(i, j)] = pidx[(j, i)] = k
    npair = len(pidx) // 2
    dist = np.zeros(npair)
    through = np.zeros(npair)
    cross = np.zeros((npair, npair))
    pairs = list(itertools.combinations(S9, 2))
    for (i, j) in pairs:
        k = pidx[(i, j)]
        dist[k] = math.dist(slots[i], slots[j]) if hasattr(math, 'dist') else math.hypot(slots[i][0] - slots[j][0], slots[i][1] - slots[j][1])
        through[k] = sum(seg_hits_rect(slots[i], slots[j], slot_rect[o], -2000) for o in S9 if o not in (i, j))
    for (i, j), (a, b) in itertools.combinations(pairs, 2):
        if seg_cross(slots[i], slots[j], slots[a], slots[b]):
            cross[pidx[(i, j)], pidx[(a, b)]] = cross[pidx[(a, b)], pidx[(i, j)]] = 1

    links = [(i, j, W[i, j]) for i, j in itertools.combinations(range(len(SP)), 2) if W[i, j] > 0]
    perms = np.array(list(itertools.permutations(S9)), dtype=np.int8)   # perms[p, species] = slot
    ptab = np.zeros((9, 9), dtype=np.int64)
    for (i, j), k in pidx.items():
        ptab[i, j] = k

    def score(perms):
        cost = np.zeros(len(perms))
        ks = []
        for i, j, w in links:
            k = ptab[perms[:, i], perms[:, j]]
            ks.append((k, w))
            cost += w * (dist[k] + THROUGH_PENALTY * pitch * through[k])
        for (k1, w1), (k2, w2) in itertools.combinations(ks, 2):
            cost += CROSS_PENALTY * pitch * math.sqrt(w1 * w2) * cross[k1, k2]
        return cost

    costs = score(perms)
    current = np.array([[min(S9, key=lambda s: math.hypot(*(np.subtract(centre(cells[sp]['rect']), slots[s]))))
                         for sp in SP]], dtype=np.int8)
    cur_cost = score(current)[0]
    best = perms[int(np.argmin(costs))]

    # ------------------------------------------------- move cells to slots
    moved_media = set()
    for k, sp in enumerate(SP):
        c = cells[sp]
        cx, cy = centre(c['rect'])
        tx, ty = slots[best[k]]
        dx, dy = round(tx - cx), round(ty - cy)
        for comp in c['compartments']:
            comp['rect']['x'] += dx
            comp['rect']['y'] += dy
        c['rect']['x'] += dx
        c['rect']['y'] += dy
        for n in M['nodes']:
            if n.get('cellId') == sp:
                n['x'], n['y'] = r2(n['x'] + dx), r2(n['y'] + dy)
        for r in M['reactions']:
            if r['cellId'] != sp:
                continue
            r['x'], r['y'] = r2(r['x'] + dx), r2(r['y'] + dy)
            for key in ('a1', 'a2'):
                r['axis'][key] = pt(r['axis'][key]['x'] + dx, r['axis'][key]['y'] + dy)
            for s in segs_of[r['id']]:
                for key in ('from', 'to', 'b1', 'b2', 'control'):
                    if key in s:
                        s[key] = pt(s[key]['x'] + dx, s[key]['y'] + dy)
                nid = s.get('nodeId')
                if nid and N[nid]['kind'] == 'medium' and nid not in moved_media:
                    N[nid]['x'], N[nid]['y'] = r2(N[nid]['x'] + dx), r2(N[nid]['y'] + dy)
                    moved_media.add(nid)
    centres = {sp: centre(cells[sp]['rect']) for sp in SP}

    # --------------------------------------------------------- exchanges
    reserved = defaultdict(list)    # (cell, edge) -> [(lo, hi)]
    bt = cells['Bt']['rect']
    hand = [n for n in M['nodes'] if n.get('cellId') == 'Bt' and n['compartment'] == 'e'
            and not any(n['id'].startswith(f'met:{rid}:') for rid in ex if rid not in FIXED)]
    if hand:
        reserved[('Bt', 'top')].append((min(n['x'] for n in hand) - 250, max(n['x'] for n in hand) + 250))

    wanted = defaultdict(list)      # (cell, edge) -> [(position, rid)]
    loose = defaultdict(list)       # cell -> [rid] without partners
    for rid, (_, mid, v) in ex.items():
        if rid in FIXED:
            continue
        sp = R[rid]['cellId']
        partners = [(b if a == rid else a, w) for (a, b), w in pair.items() if rid in (a, b)]
        if not partners:
            loose[sp].append(rid)
            continue
        tw = sum(w for _, w in partners)
        tx = sum(centres[R[p]['cellId']][0] * w for p, w in partners) / tw
        ty = sum(centres[R[p]['cellId']][1] * w for p, w in partners) / tw
        edge, (px, py) = ray_exit(cells[sp]['rect'], centres[sp], (tx, ty))
        wanted[(sp, edge)].append((py if edge in ('left', 'right') else px, rid))

    def blocked(sp, edge, s):
        return any(lo <= s <= hi for lo, hi in reserved[(sp, edge)])

    placed = {}
    for (sp, edge), lst in wanted.items():
        lst.sort()
        lo, hi = edge_span(cells[sp]['rect'], edge)
        pos = spread([s for s, _ in lst], lo, hi, MIN_GAP)
        for (s0, rid), s in zip(lst, pos):
            if blocked(sp, edge, s):    # step over the reserved stretch
                for a, b in reserved[(sp, edge)]:
                    if a <= s <= b:
                        s = a - 1 if abs(s - a) < abs(s - b) else b + 1
            placed[rid] = (edge, s)

    others = {sp: [centres[o] for o in SP if o != sp] for sp in SP}
    for sp in SP:
        rect = cells[sp]['rect']
        for rid in sorted(loose[sp]):
            best_gap = None
            for edge, (nx, ny) in NORMAL.items():
                lo, hi = edge_span(rect, edge)
                taken = sorted([s for e2, s in [placed[r] for r in placed if R[r]['cellId'] == sp] if e2 == edge]
                               + [s for a, b in reserved[(sp, edge)] for s in (a, b)]
                               + [R[f][('x' if edge in ('top', 'bottom') else 'y')] for f in FIXED
                                  if R[f]['cellId'] == sp and edge == 'top'])
                marks = [lo - MIN_GAP] + taken + [hi + MIN_GAP]
                c = centres[sp]
                # facing away from the community counts as more room
                away = -sum(((o[0] - c[0]) * nx + (o[1] - c[1]) * ny) / math.hypot(o[0] - c[0], o[1] - c[1])
                            for o in others[sp]) / len(others[sp])
                for a, b in zip(marks, marks[1:]):
                    mid = (a + b) / 2
                    if b - a < 2 * MIN_GAP or blocked(sp, edge, mid):
                        continue
                    room = (b - a) * (1 + 0.5 * away)
                    if best_gap is None or room > best_gap[0]:
                        best_gap = (room, edge, mid)
            placed[rid] = (best_gap[1], best_gap[2])

    for rid, (edge, s) in placed.items():
        r = R[rid]
        rect = cells[r['cellId']]['rect']
        x, y = edge_point(rect, edge, s)
        nx, ny = NORMAL[edge]
        r['x'], r['y'] = r2(x), r2(y)
        r['normal'] = {'x': nx, 'y': ny}
        r['placementDirection'] = {'x': nx, 'y': ny}
        r['axisDirectionHint'] = {'x': nx, 'y': ny}
        r['axis'] = {'a1': pt(x - nx * AXIS_HALF, y - ny * AXIS_HALF), 'a2': pt(x + nx * AXIS_HALF, y + ny * AXIS_HALF),
                     'dir': {'x': nx, 'y': ny}}
        for sg in segs_of[rid]:
            if sg['kind'] == 'axis':
                sg['from'], sg['to'] = dict(r['axis']['a1']), dict(r['axis']['a2'])
                continue
            n = N[sg['nodeId']]
            reach = -EX_REACH if sg['side'] == 'substrate' else EX_REACH
            n['x'], n['y'] = r2(x + nx * reach), r2(y + ny * reach)
            if sg['side'] == 'substrate':
                sg['from'], sg['to'] = pt(n['x'], n['y']), dict(r['axis']['a1'])
            else:
                sg['from'], sg['to'] = dict(r['axis']['a2']), pt(n['x'], n['y'])
            p0 = sg['from'] if sg['side'] == 'substrate' else sg['to']
            p3 = sg['to'] if sg['side'] == 'substrate' else sg['from']
            sg['b1'] = pt(p0['x'] + (p3['x'] - p0['x']) * BRANCH[0], p0['y'] + (p3['y'] - p0['y']) * BRANCH[0])
            sg['b2'] = pt(p0['x'] + (p3['x'] - p0['x']) * BRANCH[1], p0['y'] + (p3['y'] - p0['y']) * BRANCH[1])

    M['canvas'] = {'width': COL_X[-1] + SLOT_W + 1200, 'height': ROW_Y[-1] + SLOT_H + 1200}
    M['environment']['rect'] = {'x': 0, 'y': 0, 'w': M['canvas']['width'], 'h': M['canvas']['height']}
    with open(out_path, 'w', encoding='utf-8') as f:
        json.dump(doc, f, ensure_ascii=False, indent=1)

    grid = [['' for _ in range(3)] for _ in range(3)]
    for k, sp in enumerate(SP):
        grid[best[k] // 3][best[k] % 3] = sp
    print('layout score: current', round(cur_cost, 1), '-> best', round(float(costs.min()), 1))
    for row in grid:
        print('   ', ' | '.join(f'{s:^4}' for s in row))
    json.dump({'pairs': [[a, b, w] for (a, b), w in pair.items()], 'species': SP, 'W': W.tolist(), 'X': X},
              open(out_path.replace('.json', '.crossfeeding.json'), 'w'), indent=1)


if __name__ == '__main__':
    main(*sys.argv[1:4])
