"""Phase 4: shrink the cells to fit a network at the hand-drawn scale.

At L_ref = 194 a species with ~100 reactions fits in about 4000 x 4000, while
the cells were 14 000 x 12 000: every transporter far from the content made
a long line. The 3x3 grid of phase 2b is kept, with smaller cells and slots:

  Bt 6600 x 6000 (the most exchanges and content), Ec and Kp 6400 x 5900
  (periplasm), Ba 5000 x 4500, the rest 6000 x 5500.

Content moves as blocks, keeping its distance to the corner it belongs to: a
block with only extracellular reactions (Bt's pectin and starch) stays at the
top-left, any other block (Bt's fermentation module) at the bottom-right. An
exchange whose extracellular node is used by a reaction of a block moves with
that block. Every other exchange is assigned again around the perimeter, at
TRANSPORT_GAP spacing, facing its partners as in phase 2b and keeping the
order around the cell, so that the transporters the next phase aligns with
them do not cross.

Usage: python build_phase4_resize.py MAP FLUXES_CSV OUT_MAP
"""
import csv
import math
import sys
from collections import defaultdict
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from coxmap import BAND_E, BAND_P, EX_REACH, AXIS_HALF, L_REF, CoxMap, load, pt, r2, save  # noqa: E402

GAP = 1.35 * L_REF        # TRANSPORT_GAP, as asked after reviewing phase 3c
CORNER = 350
SIZES = {'Bt': (6600, 6000), 'Ec': (6400, 5900), 'Kp': (6400, 5900), 'Ba': (5000, 4500)}
DEFAULT = (6000, 5500)
MARGIN, COL_GAP, ROW_GAP = 1200, 2600, 2600
SKIP = {'M@h_s', 'M@h2o_s'}


def inset(r, d):
    return {'x': r['x'] + d, 'y': r['y'] + d, 'w': r['w'] - 2 * d, 'h': r['h'] - 2 * d}


def centre(r):
    return (r['x'] + r['w'] / 2, r['y'] + r['h'] / 2)


# ---- the perimeter as one loop: t runs clockwise from the top-left corner
def edges_of(r):
    L, T, R, B = r['x'], r['y'], r['x'] + r['w'], r['y'] + r['h']
    return [('top', L + CORNER, R - CORNER), ('right', T + CORNER, B - CORNER),
            ('bottom', R - CORNER, L + CORNER), ('left', B - CORNER, T + CORNER)]


CORNER_SPAN = CORNER * math.sqrt(2)     # the loop goes round each corner, where nothing is placed


def to_t(r, edge, s):
    t = 0
    for e, a, b in edges_of(r):
        if e == edge:
            return t + abs(s - a)
        t += abs(b - a) + CORNER_SPAN
    raise ValueError(edge)


def from_t(r, t):
    t %= loop_len(r)
    for e, a, b in edges_of(r):
        L = abs(b - a)
        if t <= L + 1e-6:
            return e, a + (t if b > a else -t)
        t -= L
        if t < CORNER_SPAN - 1e-6:
            return e, b            # inside a corner: the end of this edge
        t -= CORNER_SPAN
    e, a, b = edges_of(r)[-1]
    return e, b


def loop_len(r):
    return sum(abs(b - a) + CORNER_SPAN for _, a, b in edges_of(r))


def slots(r, gap):
    out = []
    for e, a, b in edges_of(r):
        L = abs(b - a)
        n = int(L // gap) + 1
        step = L / (n - 1) if n > 1 else 0
        for i in range(n):
            out.append((e, a + (i * step if b > a else -i * step)))
    return out


def ray_exit(r, c, target):
    dx, dy = target[0] - c[0], target[1] - c[1]
    best = None
    for edge, nx, ny in (('left', -1, 0), ('right', 1, 0), ('top', 0, -1), ('bottom', 0, 1)):
        if nx and dx * nx > 0:
            t = ((r['x'] if nx < 0 else r['x'] + r['w']) - c[0]) / dx
        elif ny and dy * ny > 0:
            t = ((r['y'] if ny < 0 else r['y'] + r['h']) - c[1]) / dy
        else:
            continue
        if best is None or t < best[0]:
            best = (t, edge)
    t, edge = best
    x, y = c[0] + dx * t, c[1] + dy * t
    lo = {'top': r['x'] + CORNER, 'bottom': r['x'] + CORNER, 'left': r['y'] + CORNER, 'right': r['y'] + CORNER}[edge]
    hi = {'top': r['x'] + r['w'] - CORNER, 'bottom': r['x'] + r['w'] - CORNER,
          'left': r['y'] + r['h'] - CORNER, 'right': r['y'] + r['h'] - CORNER}[edge]
    s = min(max(x if edge in ('top', 'bottom') else y, lo), hi)
    return edge, s


def assign(items, slot_ts, total):
    """Order-preserving assignment of items (desired t) to slots around a loop,
    least total displacement; the loop is cut where the items are farthest apart."""
    if not items:
        return {}
    items = sorted(items, key=lambda it: it[1])
    ts = [t for _, t in items]
    gaps = [((ts[(i + 1) % len(ts)] - ts[i]) % total, i) for i in range(len(ts))]
    _, cut = max(gaps)
    start = (ts[cut] + ((ts[(cut + 1) % len(ts)] - ts[cut]) % total) / 2) % total
    order = items[cut + 1:] + items[:cut + 1]
    rel = lambda t: (t - start) % total
    sl = sorted(slot_ts, key=rel)
    n, m = len(order), len(sl)
    if n > m:
        raise ValueError(f'{n} exchanges for {m} places')
    INF = float('inf')
    D = [[INF] * (m + 1) for _ in range(n + 1)]
    W = [[False] * (m + 1) for _ in range(n + 1)]
    for j in range(m + 1):
        D[0][j] = 0
    for i in range(1, n + 1):
        for j in range(i, m + 1):
            skip = D[i][j - 1]
            d = abs(rel(order[i - 1][1]) - rel(sl[j - 1]))
            take = D[i - 1][j - 1] + min(d, total - d)
            if take <= skip:
                D[i][j], W[i][j] = take, True
            else:
                D[i][j] = skip
    out, i, j = {}, n, m
    while i > 0:
        if W[i][j]:
            out[order[i - 1][0]] = sl[j - 1]
            i -= 1
        j -= 1
    return out


def main(map_path, flux_path, out_path):
    doc = load(map_path)
    cm = CoxMap(doc)
    M = cm.M
    rows = list(csv.DictReader(open(flux_path, encoding='utf-8')))
    flux = {(r['species'], r['local_id']): float(r['flux']) for r in rows}
    mu = next(float(r['flux']) for r in rows if r['species'] == 'Bt' and r['local_id'].startswith('EX_biomassBt'))
    X = {r['reaction'][len('EX_biomass'):-3]: float(r['flux']) / mu
         for r in rows if r['reaction'].startswith('EX_biomass') and r['reaction'].endswith('(u)')}

    # ---------------------------------------------------------------- grid
    cells = M['cells']
    xs = sorted({round(centre(c['rect'])[0]) for c in cells})
    ys = sorted({round(centre(c['rect'])[1]) for c in cells})
    colw = max(SIZES.get(c['id'], DEFAULT)[0] for c in cells)
    rowh = max(SIZES.get(c['id'], DEFAULT)[1] for c in cells)
    old = {c['id']: dict(c['rect']) for c in cells}
    new = {}
    for c in cells:
        cx, cy = centre(c['rect'])
        col = min(range(3), key=lambda i: abs(xs[i] - cx))
        row = min(range(3), key=lambda i: abs(ys[i] - cy))
        w, h = SIZES.get(c['id'], DEFAULT)
        sx = MARGIN + col * (colw + COL_GAP) + colw / 2
        sy = MARGIN + row * (rowh + ROW_GAP) + rowh / 2
        new[c['id']] = {'x': r2(sx - w / 2), 'y': r2(sy - h / 2), 'w': w, 'h': h}

    # ------------------------------------------------- blocks of content
    exch = {r['id']: r for r in M['reactions'] if r['kind'] == 'transfer'}
    ex_e_node = {}
    for rid in exch:
        for s in cm.segs[rid]:
            n = cm.N.get(s.get('nodeId'))
            if n and n['kind'] != 'medium':
                ex_e_node[rid] = n['id']
    internal = [r for r in M['reactions'] if r['kind'] != 'transfer']
    used_by = defaultdict(set)
    for r in internal:
        for s in cm.segs[r['id']]:
            if s.get('nodeId'):
                used_by[s['nodeId']].add(r['id'])
    blocks = defaultdict(set)   # (cell, anchor) -> reaction ids
    for r in internal:
        comps = {cm.N[s['nodeId']]['compartment'] for s in cm.segs[r['id']] if s.get('nodeId')}
        blocks[(r['cellId'], 'tl' if comps == {'e'} else 'br')].add(r['id'])
    pinned = {}
    for rid, nid in ex_e_node.items():
        for key, rset in blocks.items():
            if used_by[nid] & rset:
                rset.add(rid)
                pinned[rid] = key
    moved = set()

    def shift(p, dx, dy):
        return pt(p['x'] + dx, p['y'] + dy)

    for (cell, anchor), rset in blocks.items():
        o, n = old[cell], new[cell]
        if anchor == 'tl':
            dx, dy = n['x'] - o['x'], n['y'] - o['y']
        else:
            dx, dy = (n['x'] + n['w']) - (o['x'] + o['w']), (n['y'] + n['h']) - (o['y'] + o['h'])
        for rid in rset:
            r = cm.R[rid]
            r['x'], r['y'] = r2(r['x'] + dx), r2(r['y'] + dy)
            r['axis']['a1'], r['axis']['a2'] = shift(r['axis']['a1'], dx, dy), shift(r['axis']['a2'], dx, dy)
            for lane in r.get('transportLanes') or []:
                lane['insidePoint'], lane['outsidePoint'] = shift(lane['insidePoint'], dx, dy), shift(lane['outsidePoint'], dx, dy)
            for s in cm.segs[rid]:
                for k in ('from', 'to', 'b1', 'b2'):
                    if k in s:
                        s[k] = shift(s[k], dx, dy)
                nid = s.get('nodeId')
                if nid and nid not in moved:
                    cm.N[nid]['x'], cm.N[nid]['y'] = r2(cm.N[nid]['x'] + dx), r2(cm.N[nid]['y'] + dy)
                    moved.add(nid)

    # ------------------------------------------------------ new compartments
    for c in cells:
        c['rect'] = dict(new[c['id']])
        comps = {k['id']: k for k in c['compartments']}
        comps['e']['rect'] = dict(new[c['id']])
        if 'p' in comps:
            comps['p']['rect'] = inset(new[c['id']], BAND_E)
            comps['c']['rect'] = inset(comps['p']['rect'], BAND_P)
        else:
            comps['c']['rect'] = inset(new[c['id']], BAND_E)
    W = MARGIN * 2 + 3 * colw + 2 * COL_GAP
    H = MARGIN * 2 + 3 * rowh + 2 * ROW_GAP
    M['canvas'] = {'width': W, 'height': H}
    M['environment']['rect'] = {'x': 0, 'y': 0, 'w': W, 'h': H}
    cm.reindex()

    # ----------------------------------------- cross-feeding pairs (X * v)
    med = {}
    for rid in exch:
        for s in cm.segs[rid]:
            n = cm.N.get(s.get('nodeId'))
            if n and n['kind'] == 'medium':
                med[rid] = n['mergedId']
    by_met = defaultdict(list)
    for rid, r in exch.items():
        v = flux.get((r['cellId'], r['localId']), 0.0) * X.get(r['cellId'], 0.0)
        by_met[med[rid]].append((rid, v))
    partner_w = defaultdict(list)    # rid -> [(partner cell, weight)]
    for mid_, lst in by_met.items():
        if mid_ in SKIP:
            continue
        S = [(rid, v) for rid, v in lst if v > 0]
        U = [(rid, -v) for rid, v in lst if v < 0]
        s_, u_ = sum(v for _, v in S), sum(v for _, v in U)
        if not S or not U:
            continue
        T = min(s_, u_)
        for a, va in S:
            for b, vb in U:
                if exch[a]['cellId'] != exch[b]['cellId']:
                    w = T * va / s_ * vb / u_
                    partner_w[a].append((exch[b]['cellId'], w))
                    partner_w[b].append((exch[a]['cellId'], w))

    # ------------------------------------------------- exchanges around cells
    canvas_c = (W / 2, H / 2)
    report = {}
    for c in cells:
        cid, rect = c['id'], c['rect']
        total = loop_len(rect)
        mine = [rid for rid, r in exch.items() if r['cellId'] == cid]
        free = [rid for rid in mine if rid not in pinned]
        reserved = []
        for rid in mine:
            if rid in pinned:
                r = cm.R[rid]
                edge = 'top' if r['normal']['y'] < 0 else 'bottom' if r['normal']['y'] > 0 else 'left' if r['normal']['x'] < 0 else 'right'
                s = r['x'] if edge in ('top', 'bottom') else r['y']
                reserved.append((to_t(rect, edge, min(max(s, rect['x'] if edge in ('top', 'bottom') else rect['y']), 1e9)), GAP * 0.99))
        top_e = [n for n in M['nodes'] if n.get('cellId') == cid and n['compartment'] == 'e'
                 and any(rid in blocks.get((cid, 'tl'), ()) for rid in used_by.get(n['id'], ()))]
        if top_e:
            a, b = min(n['x'] for n in top_e) - 250, max(n['x'] for n in top_e) + 250
            ta, tb = to_t(rect, 'top', max(a, rect['x'] + CORNER)), to_t(rect, 'top', min(b, rect['x'] + rect['w'] - CORNER))
            reserved.append(((ta + tb) / 2, (tb - ta) / 2))
        gap = GAP
        while True:
            sl = [s for s in slots(rect, gap)
                  if all(min(abs(to_t(rect, *s) - t0), total - abs(to_t(rect, *s) - t0)) > hw for t0, hw in reserved)]
            if len(sl) >= len(free):
                break
            gap *= 0.95
        cc = centre(rect)
        items = []
        out_dir = (cc[0] - canvas_c[0], cc[1] - canvas_c[1])
        lonely = []
        for rid in free:
            ps = partner_w.get(rid)
            if ps:
                tw = sum(w for _, w in ps)
                tx = sum(centre(new[p])[0] * w for p, w in ps) / tw
                ty = sum(centre(new[p])[1] * w for p, w in ps) / tw
                items.append((rid, to_t(rect, *ray_exit(rect, cc, (tx, ty)))))
            else:
                lonely.append(rid)
        # exchanges without partners face away from the community
        far = (cc[0] + out_dir[0] * 10 + 1, cc[1] + out_dir[1] * 10 + 1)
        t_far = to_t(rect, *ray_exit(rect, cc, far))
        for k, rid in enumerate(sorted(lonely, key=lambda x: exch[x]['localId'])):
            items.append((rid, (t_far + (k - len(lonely) / 2) * gap) % total))
        place = assign(items, [to_t(rect, *s) for s in sl], total)
        for rid, t in place.items():
            edge, s = from_t(rect, t)
            r = cm.R[rid]
            nx, ny = {'top': (0, -1), 'bottom': (0, 1), 'left': (-1, 0), 'right': (1, 0)}[edge]
            x = s if edge in ('top', 'bottom') else (rect['x'] if edge == 'left' else rect['x'] + rect['w'])
            y = s if edge in ('left', 'right') else (rect['y'] if edge == 'top' else rect['y'] + rect['h'])
            r['x'], r['y'] = r2(x), r2(y)
            r['normal'] = {'x': nx, 'y': ny}
            r['placementDirection'] = {'x': nx, 'y': ny}
            r['axisDirectionHint'] = {'x': nx, 'y': ny}
            r['axis'] = {'a1': pt(x - nx * AXIS_HALF, y - ny * AXIS_HALF), 'a2': pt(x + nx * AXIS_HALF, y + ny * AXIS_HALF),
                         'dir': {'x': nx, 'y': ny}}
            for s in cm.segs[rid]:
                if s['kind'] == 'axis':
                    s['from'], s['to'] = dict(r['axis']['a1']), dict(r['axis']['a2'])
                    continue
                n = cm.N[s['nodeId']]
                reach = -EX_REACH if s['side'] == 'substrate' else EX_REACH
                n['x'], n['y'] = r2(x + nx * reach), r2(y + ny * reach)
                if s['side'] == 'substrate':
                    s['from'], s['to'] = pt(n['x'], n['y']), dict(r['axis']['a1'])
                else:
                    s['from'], s['to'] = dict(r['axis']['a2']), pt(n['x'], n['y'])
                s['b1'] = pt(s['from']['x'] + (s['to']['x'] - s['from']['x']) * 0.34, s['from']['y'] + (s['to']['y'] - s['from']['y']) * 0.34)
                s['b2'] = pt(s['from']['x'] + (s['to']['x'] - s['from']['x']) * 0.72, s['from']['y'] + (s['to']['y'] - s['from']['y']) * 0.72)
        report[cid] = (len(mine), len(free), round(gap))
    save(doc, out_path)
    print(f'canvas {W} x {H}')
    for cid, (n, f, g) in report.items():
        print(f'  {cid}: {new[cid]["w"]} x {new[cid]["h"]}, {n} exchanges ({n - f} move with content), spacing {g}')


if __name__ == '__main__':
    main(*sys.argv[1:4])
