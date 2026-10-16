"""Phase 3b of the gut9 map: Bt's pyruvate outlets and acetate branch.

Adds nine reactions of model-Bt.json, one at a time, each checked against the
GEM before the next (rules R01, R51):

  L_LACt2r, PYRt2r, D_LACt2, ACtr   proton symporters on the cytoplasmic membrane
  LDH_D                             pyruvate <=> D-lactate
  PFL, POR4i                        pyruvate -> acetyl-CoA (POR4i drawn as a bypass)
  PTAr, ACKr                        acetyl-CoA -> acetyl phosphate -> acetate

Bt's glycolysis sits in the cell's top-left corner, while phase 2b turned Bt's
exchanges towards its partners (Ef below, St to the right). Joined as they were,
the transporters would have run 10 000-13 000 long. So the glycolysis module is
moved as one block, its shape unchanged, to the bottom-right of the cytosol,
with pyruvate 2 L_ref above the bottom membrane, and the four exchanges it feeds
are moved onto those edges, next to their transporters. Pectin and starch stay
on the top edge.

Usage: python build_phase3b.py MAP MODEL_JSON FLUXES_CSV OUT_MAP LOG_MD
"""
import csv
import json
import math
import statistics
import sys
from collections import defaultdict

CELL = 'Bt'
AXIS_HALF = 17
EX_REACH = 130
LANE_OFFSET = 6
SAT_ALONG = (45, 60)
SAT_ACROSS = (60, 80, 100, 125)
TRANSPORT_GAP = 1.35       # between neighbouring transporters, in L_ref (0.75 before review)


def r2(v):
    return round(v, 2)


def pt(x, y):
    return {'x': r2(x), 'y': r2(y)}


def mid(cid):
    return f'{CELL}@{cid}'


def base_of(local):
    return local.split('[')[0]


def dist_point_seg(p, a, b):
    ax, ay, bx, by = a['x'], a['y'], b['x'], b['y']
    dx, dy = bx - ax, by - ay
    L2 = dx * dx + dy * dy
    t = 0 if L2 == 0 else max(0, min(1, ((p[0] - ax) * dx + (p[1] - ay) * dy) / L2))
    return math.hypot(p[0] - ax - t * dx, p[1] - ay - t * dy)


class Builder:
    def __init__(self, doc, model, flux):
        self.doc = doc
        self.M = doc['map']
        self.N = {n['id']: n for n in self.M['nodes']}
        self.R = {r['id']: r for r in self.M['reactions']}
        self.gem = {r['id']: r for r in model['reactions']}
        self.mets = {m['id']: m for m in model['metabolites']}
        self.flux = flux
        self.cell = next(c for c in self.M['cells'] if c['id'] == CELL)
        self.comp = {k['id']: k['rect'] for k in self.cell['compartments']}
        self.log = []
        # cofactor sides fixed by design, used whenever a reaction is redrawn
        self.hand_pref = {'LDH_L_Bt': {'nad[c_Bt]': -1, 'nadh[c_Bt]': -1, 'h[c_Bt]': -1}}

    # ------------------------------------------------------------ geometry
    def segs(self, rid):
        return [s for s in self.M['segments'] if s['reactionId'] == rid]

    def shape(self, seg, rxn):
        p0 = seg['from'] if seg['side'] == 'substrate' else seg['to']
        p3 = seg['to'] if seg['side'] == 'substrate' else seg['from']
        d = rxn['axis']['dir']
        outward = (p3['x'] - rxn['x']) * d['x'] + (p3['y'] - rxn['y']) * d['y']
        if abs(outward) < 1e-6:
            outward = (p0['x'] - rxn['x']) * d['x'] + (p0['y'] - rxn['y']) * d['y']
        if abs(outward) < 1e-6:
            outward = -1 if seg['side'] == 'substrate' else 1
        u = {'x': -d['x'], 'y': -d['y']} if outward < 0 else d
        vx, vy = p0['x'] - p3['x'], p0['y'] - p3['y']
        along = vx * u['x'] + vy * u['y']
        side = -vx * u['y'] + vy * u['x']
        across = abs(side)
        if across < 4 and along > 0 or seg.get('transportLane'):
            seg['b1'] = pt(p0['x'] + (p3['x'] - p0['x']) * 0.34, p0['y'] + (p3['y'] - p0['y']) * 0.34)
            seg['b2'] = pt(p0['x'] + (p3['x'] - p0['x']) * 0.72, p0['y'] + (p3['y'] - p0['y']) * 0.72)
        else:
            hand = -1 if side < 0 else 1
            n = {'x': -u['y'] * hand, 'y': u['x'] * hand}
            reach = max(along, 0.5 * across, 14)
            seg['b1'] = pt(p0['x'] - n['x'] * across * 0.55, p0['y'] - n['y'] * across * 0.55)
            seg['b2'] = pt(p3['x'] + u['x'] * reach * 0.8, p3['y'] + u['y'] * reach * 0.8)

    @staticmethod
    def curve(seg, n=8):
        """The branch as Coxeter draws it: a cubic from `from` to `to`, as a polyline."""
        p0, p3 = seg['from'], seg['to']
        p1, p2 = seg.get('b1', p0), seg.get('b2', p3)
        if seg.get('side') == 'product':
            p1, p2 = seg.get('b2', p0), seg.get('b1', p3)
        pts = []
        for i in range(n + 1):
            t = i / n
            a, b, c, d = (1 - t) ** 3, 3 * (1 - t) ** 2 * t, 3 * (1 - t) * t ** 2, t ** 3
            pts.append({'x': a * p0['x'] + b * p1['x'] + c * p2['x'] + d * p3['x'],
                        'y': a * p0['y'] + b * p1['y'] + c * p2['y'] + d * p3['y']})
        return list(zip(pts, pts[1:]))

    def obstacles(self, skip_rid=None):
        lines = []
        for s in self.M['segments']:
            if s['reactionId'] == skip_rid:
                continue
            lines.extend(self.curve(s) if s.get('nodeId') else [(s['from'], s['to'])])
        pts = [(n['x'], n['y']) for n in self.M['nodes']]
        centres = [(r['x'], r['y']) for r in self.M['reactions'] if r['id'] != skip_rid]
        return lines, pts, centres

    def clearance(self, p, lines, pts, centres):
        c = min((math.hypot(p[0] - q[0], p[1] - q[1]) for q in pts), default=1e9)
        c = min(c, min((math.hypot(p[0] - q[0], p[1] - q[1]) for q in centres), default=1e9))
        c = min(c, min((dist_point_seg(p, a, b) for a, b in lines), default=1e9))
        return c

    def inside(self, comp, p, pad=20):
        r = self.comp[comp]
        if not (r['x'] + pad <= p[0] <= r['x'] + r['w'] - pad and r['y'] + pad <= p[1] <= r['y'] + r['h'] - pad):
            return False
        # a node of the extracellular band must not fall into the cytosol
        if comp == 'e':
            c = self.comp['c']
            if c['x'] - pad <= p[0] <= c['x'] + c['w'] + pad and c['y'] - pad <= p[1] <= c['y'] + c['h'] + pad:
                return False
        return True

    def best_spot(self, rxn, side_sign, comp, taken, prefer_hand=None, redraw=False):
        """Free place for a cofactor at one end of a reaction's axis. When the
        reaction is being redrawn, its own cofactor branches are not obstacles."""
        d = rxn['axis']['dir']
        perp = (-d['y'], d['x'])
        if redraw:
            lines, pts, centres = self.obstacles(skip_rid=rxn['id'])
            for s in self.segs(rxn['id']):
                if s.get('nodeId') and self.N[s['nodeId']].get('visualRole') != 'secondary':
                    lines.extend(self.curve(s))
            centres.append((rxn['x'], rxn['y']))
        else:
            lines, pts, centres = self.obstacles()
        pts = pts + taken
        best = None
        for along in SAT_ALONG:
            for across in SAT_ACROSS:
                for hand in ((prefer_hand,) if prefer_hand else (1, -1)):
                    p = (rxn['x'] + d['x'] * along * side_sign + perp[0] * across * hand,
                         rxn['y'] + d['y'] * along * side_sign + perp[1] * across * hand)
                    if not self.inside(comp, p):
                        continue
                    c = self.clearance(p, lines, pts, centres)
                    score = min(c, 70) - 0.15 * across   # clear first, then close
                    if best is None or score > best[0]:
                        best = (score, p, c)
        return best[1], best[2]

    # ----------------------------------------------------------- creation
    def node(self, nid, local, kind, x, y, rid=None):
        m = self.mets[local]
        n = {'id': nid, 'kind': kind, 'mergedId': mid(local), 'localId': local, 'base': local,
             'name': m['name'], 'formula': m.get('formula', ''), 'compartment': m['compartment'], 'cellId': CELL,
             'x': r2(x), 'y': r2(y)}
        if kind == 'currency':
            n.update({'r': 7, 'primary': False, 'visualRole': 'secondary', 'reactionId': rid})
        else:
            n.update({'reactionLocalFor': rid, 'reactionLocal': True, 'r': 13, 'primary': True, 'visualRole': 'primary'})
        self.M['nodes'].append(n)
        self.N[nid] = n
        return n

    def equation(self, gem):
        left = [(m, -c) for m, c in gem['metabolites'].items() if c < 0]
        right = [(m, c) for m, c in gem['metabolites'].items() if c > 0]
        fmt = lambda side: ' + '.join((f'{c:g} ' if c != 1 else '') + mid(m) for m, c in side)
        arrow = '<=>' if gem['lower_bound'] < 0 else '-->'
        return f'{fmt(left)} {arrow} {fmt(right)}'

    def reaction(self, local_rid, x, y, dvec, membrane=None, normal=None):
        gem = self.gem[local_rid]
        rid = mid(local_rid)
        L = math.hypot(*dvec)
        d = {'x': round(dvec[0] / L, 4), 'y': round(dvec[1] / L, 4)}
        comps = sorted({self.mets[m]['compartment'] for m in gem['metabolites']})
        r = {'id': rid, 'localId': local_rid, 'name': gem['name'], 'subsystem': gem.get('subsystem', ''),
             'cellId': CELL, 'kind': 'transport' if len(comps) > 1 else 'metabolic', 'compartments': comps,
             'membrane': membrane, 'boundaryId': f'{CELL}/{membrane}' if membrane else None,
             'onMembrane': bool(membrane), 'normal': normal, 'placementDirection': dict(d),
             'axisDirectionHint': dict(d), 'x': r2(x), 'y': r2(y), 'equation': self.equation(gem),
             'terminalMetaboliteId': None, 'bounds': {'lower': gem['lower_bound'], 'upper': gem['upper_bound']},
             'axis': {'a1': pt(x - d['x'] * AXIS_HALF, y - d['y'] * AXIS_HALF),
                      'a2': pt(x + d['x'] * AXIS_HALF, y + d['y'] * AXIS_HALF), 'dir': d},
             'transportMode': None, 'transportLanes': None}
        self.M['reactions'].append(r)
        self.R[rid] = r
        self.M['segments'].append({'reactionId': rid, 'kind': 'axis', 'from': dict(r['axis']['a1']),
                                   'to': dict(r['axis']['a2']), 'arrow': False})
        return r

    def branch(self, r, node, coef, kind, start=None, end=None, extra=None):
        side = 'substrate' if coef < 0 else 'product'
        if side == 'substrate':
            seg = {'reactionId': r['id'], 'kind': kind, 'from': pt(node['x'], node['y']),
                   'to': end or dict(r['axis']['a1']), 'nodeId': node['id'], 'coefficient': coef,
                   'arrow': False, 'side': side, 'backboneAnchor': True}
        else:
            seg = {'reactionId': r['id'], 'kind': kind, 'from': start or dict(r['axis']['a2']),
                   'to': pt(node['x'], node['y']), 'nodeId': node['id'], 'coefficient': coef,
                   'arrow': True, 'side': side, 'backboneAnchor': True}
        if extra:
            seg.update(extra)
        self.shape(seg, r)
        self.M['segments'].append(seg)
        return seg

    def add_metabolic(self, local_rid, main_sub, main_prod, primaries=None, centre=None, hands=None, side_primaries=()):
        """A cytosolic reaction between two primary nodes; every other metabolite
        is a cofactor placed where there is room, or a given primary node. A
        metabolite in `side_primaries` is placed like a cofactor but drawn as a
        primary node: a pathway metabolite whose other reactions are not drawn yet."""
        a, b = self.N[main_sub], self.N[main_prod]
        cx, cy = centre or ((a['x'] + b['x']) / 2, (a['y'] + b['y']) / 2)
        r = self.reaction(local_rid, cx, cy, (b['x'] - a['x'], b['y'] - a['y']))
        gem = self.gem[local_rid]
        primaries = dict(primaries or {})
        primaries[a['mergedId']] = a['id']
        primaries[b['mergedId']] = b['id']
        taken = []
        for local, coef in gem['metabolites'].items():
            m = mid(local)
            if m in primaries:
                self.branch(r, self.N[primaries[m]], coef, 'primary')
                continue
            sign = -1 if coef < 0 else 1
            hand = (hands or {}).get(local)
            p, _ = self.best_spot(r, sign, self.mets[local]['compartment'], taken, prefer_hand=hand)
            taken.append(p)
            if local in side_primaries:
                n = self.node(f'met:{r["id"]}:{m}', local, 'metabolite', p[0], p[1], r['id'])
                self.branch(r, n, coef, 'primary')
                continue
            n = self.node(f'cur:{r["id"]}:{m}', local, 'currency', p[0], p[1], r['id'])
            self.branch(r, n, coef, 'currency')
        return r

    def add_symport(self, local_rid, cyt_node, ext_node, normal, along=None):
        """Proton symporter on the cytoplasmic membrane, drawn as Coxeter draws a
        two-lane transporter: one lane for the metabolite, one for H+. It sits
        on the membrane in line with the cytosolic node, or at `along`."""
        c, e = self.N[cyt_node], self.N[ext_node]
        nx, ny = normal
        rect = self.comp['c']
        if nx:
            x, y = (rect['x'] + rect['w'] if nx > 0 else rect['x']), (c['y'] if along is None else along)
        else:
            x, y = (c['x'] if along is None else along), (rect['y'] + rect['h'] if ny > 0 else rect['y'])
        # substrates are outside: the backbone runs from outside to inside
        r = self.reaction(local_rid, x, y, (-nx, -ny), membrane='c', normal={'x': nx, 'y': ny})
        perp = (-ny, nx)
        gem = self.gem[local_rid]
        main = base_of(c['localId'])
        lanes = []
        for i, b in enumerate([main, 'h']):
            off = (i - 0.5) * 2 * LANE_OFFSET
            lanes.append({'base': b, 'offset': off, 'direction': -1,
                          'insidePoint': pt(x - nx * AXIS_HALF + perp[0] * off, y - ny * AXIS_HALF + perp[1] * off),
                          'outsidePoint': pt(x + nx * AXIS_HALF + perp[0] * off, y + ny * AXIS_HALF + perp[1] * off)})
        r['transportMode'] = 'symport'
        r['transportLanes'] = lanes
        self.M['segments'] = [s for s in self.M['segments'] if not (s['reactionId'] == r['id'] and s['kind'] == 'axis')]
        for lane in lanes:
            self.M['segments'].append({'id': f'{local_rid}:lane:{lane["base"]}', 'reactionId': r['id'],
                                       'kind': 'transport-lane', 'from': lane['insidePoint'],
                                       'to': lane['outsidePoint'], 'arrow': False,
                                       'transportBase': lane['base'], 'transportDirection': -1})
        lane_of = {l['base']: l for l in lanes}
        # H+ on each side, beside the lane, where there is room
        lines, pts, centres = self.obstacles()
        best = None
        for hand in (1, -1):
            pin = (x - nx * 75 + perp[0] * 60 * hand, y - ny * 75 + perp[1] * 60 * hand)
            pout = (x + nx * 75 + perp[0] * 60 * hand, y + ny * 75 + perp[1] * 60 * hand)
            score = min(self.clearance(pin, lines, pts, centres), self.clearance(pout, lines, pts, centres))
            if best is None or score > best[0]:
                best = (score, pin, pout)
        hin = self.node(f'cur:{r["id"]}:{mid("h[c_Bt]")}', 'h[c_Bt]', 'currency', *best[1], rid=r['id'])
        hout = self.node(f'cur:{r["id"]}:{mid("h[e_Bt]")}', 'h[e_Bt]', 'currency', *best[2], rid=r['id'])
        for local, coef in gem['metabolites'].items():
            node = {c['mergedId']: c, e['mergedId']: e, hin['mergedId']: hin, hout['mergedId']: hout}[mid(local)]
            lane = lane_of[base_of(local)]
            outside = self.mets[local]['compartment'] == 'e'
            end = lane['outsidePoint'] if outside else lane['insidePoint']
            kind = 'currency' if node['kind'] == 'currency' else 'primary'
            extra = {'transportBase': lane['base'], 'transportLane': True}
            if coef < 0:
                self.branch(r, node, coef, kind, end=dict(end), extra=extra)
            else:
                self.branch(r, node, coef, kind, start=dict(end), extra=extra)
        return r

    # --------------------------------------------------------- relocation
    def translate_module(self, rids, dx, dy):
        ids = set(rids)
        moved = set()
        for s in self.M['segments']:
            if s['reactionId'] in ids:
                for k in ('from', 'to', 'b1', 'b2'):
                    if k in s:
                        s[k] = pt(s[k]['x'] + dx, s[k]['y'] + dy)
        for rid in ids:
            r = self.R[rid]
            r['x'], r['y'] = r2(r['x'] + dx), r2(r['y'] + dy)
            for k in ('a1', 'a2'):
                r['axis'][k] = pt(r['axis'][k]['x'] + dx, r['axis'][k]['y'] + dy)
            for s in self.segs(rid):
                nid = s.get('nodeId')
                if nid and nid not in moved:
                    n = self.N[nid]
                    n['x'], n['y'] = r2(n['x'] + dx), r2(n['y'] + dy)
                    moved.add(nid)

    def move_exchange(self, local_rid, x, y, normal):
        r = self.R[mid(local_rid)]
        nx, ny = normal
        r['x'], r['y'] = r2(x), r2(y)
        r['normal'] = {'x': nx, 'y': ny}
        r['placementDirection'] = {'x': nx, 'y': ny}
        r['axisDirectionHint'] = {'x': nx, 'y': ny}
        r['axis'] = {'a1': pt(x - nx * AXIS_HALF, y - ny * AXIS_HALF), 'a2': pt(x + nx * AXIS_HALF, y + ny * AXIS_HALF),
                     'dir': {'x': nx, 'y': ny}}
        for s in self.segs(r['id']):
            if s['kind'] == 'axis':
                s['from'], s['to'] = dict(r['axis']['a1']), dict(r['axis']['a2'])
                continue
            n = self.N[s['nodeId']]
            reach = -EX_REACH if s['side'] == 'substrate' else EX_REACH
            n['x'], n['y'] = r2(x + nx * reach), r2(y + ny * reach)
            if s['side'] == 'substrate':
                s['from'], s['to'] = pt(n['x'], n['y']), dict(r['axis']['a1'])
            else:
                s['from'], s['to'] = dict(r['axis']['a2']), pt(n['x'], n['y'])
            self.shape(s, r)

    def relayout_cofactors(self, local_rid, hands=None):
        """Put a reaction's cofactors back where there is room around it now."""
        r = self.R[mid(local_rid)]
        hands = hands or self.hand_pref.get(local_rid)
        segs = [s for s in self.segs(r['id']) if s.get('nodeId') and self.N[s['nodeId']].get('visualRole') == 'secondary']
        # take them out of the way first, so they do not block their own new places
        for s in segs:
            n = self.N[s['nodeId']]
            n['x'], n['y'] = -1e6, -1e6
        taken = []
        for s in segs:
            n = self.N[s['nodeId']]
            sign = -1 if s['side'] == 'substrate' else 1
            p, _ = self.best_spot(r, sign, n['compartment'], taken, prefer_hand=(hands or {}).get(n['localId']), redraw=True)
            taken.append(p)
            n['x'], n['y'] = r2(p[0]), r2(p[1])
        for s in self.segs(r['id']):
            if s['kind'] == 'axis':
                s['from'], s['to'] = dict(r['axis']['a1']), dict(r['axis']['a2'])
            elif s.get('nodeId'):
                n = self.N[s['nodeId']]
                if s['side'] == 'substrate':
                    s['from'], s['to'] = pt(n['x'], n['y']), dict(r['axis']['a1'])
                else:
                    s['from'], s['to'] = dict(r['axis']['a2']), pt(n['x'], n['y'])
                self.shape(s, r)

    def tidy(self, local_rids, passes=2):
        """Place the cofactors of these reactions again, now that everything
        around them is drawn, then check every logged reaction once more."""
        for _ in range(passes):
            for rid in local_rids:
                self.relayout_cofactors(rid)
        for e in self.log:
            problems = self.check(e['reaction'])
            if problems:
                raise SystemExit(f'{e["reaction"]} after tidying: ' + '; '.join(problems))

    def relayout_axis(self, local_rid, sub_node, prod_node):
        r = self.R[mid(local_rid)]
        a, b = self.N[sub_node], self.N[prod_node]
        cx, cy = (a['x'] + b['x']) / 2, (a['y'] + b['y']) / 2
        L = math.hypot(b['x'] - a['x'], b['y'] - a['y'])
        d = {'x': round((b['x'] - a['x']) / L, 4), 'y': round((b['y'] - a['y']) / L, 4)}
        r['x'], r['y'] = r2(cx), r2(cy)
        r['axis'] = {'a1': pt(cx - d['x'] * AXIS_HALF, cy - d['y'] * AXIS_HALF),
                     'a2': pt(cx + d['x'] * AXIS_HALF, cy + d['y'] * AXIS_HALF), 'dir': d}
        r['placementDirection'] = dict(d)
        r['axisDirectionHint'] = dict(d)
        if 'linear' in r:
            r['linear']['t'] = 0.5

    # ------------------------------------------------------------- checks
    def check(self, local_rid):
        """R51: the drawn reaction is the GEM's, and its drawing holds together."""
        problems = []
        gem = self.gem[local_rid]
        r = self.R[mid(local_rid)]
        drawn = defaultdict(float)
        for s in self.segs(r['id']):
            if s.get('nodeId'):
                n = self.N.get(s['nodeId'])
                if n is None:
                    problems.append(f'segment to missing node {s["nodeId"]}')
                    continue
                drawn[n['mergedId']] += s['coefficient']
                end = s['from'] if s['side'] == 'substrate' else s['to']
                if math.hypot(end['x'] - n['x'], end['y'] - n['y']) > 0.5:
                    problems.append(f'segment does not reach {n["id"]}')
                if n['compartment'] != self.mets[n['localId']]['compartment']:
                    problems.append(f'{n["id"]} has the wrong compartment')
                comp = self.comp[n['compartment']]
                if not (comp['x'] <= n['x'] <= comp['x'] + comp['w'] and comp['y'] <= n['y'] <= comp['y'] + comp['h']):
                    problems.append(f'{n["id"]} outside its compartment')
        want = {mid(m): c for m, c in gem['metabolites'].items()}
        if dict(drawn) != want:
            problems.append(f'stoichiometry {dict(drawn)} != GEM {want}')
        if r['bounds'] != {'lower': gem['lower_bound'], 'upper': gem['upper_bound']}:
            problems.append('bounds differ from the GEM')
        if r['onMembrane']:
            rect = self.comp['c']
            on = (abs(r['x'] - rect['x']) < 0.5 or abs(r['x'] - rect['x'] - rect['w']) < 0.5
                  or abs(r['y'] - rect['y']) < 0.5 or abs(r['y'] - rect['y'] - rect['h']) < 0.5)
            if not on:
                problems.append('transporter is not on the cytoplasmic membrane')
        else:
            if not self.inside('c', (r['x'], r['y']), 0):
                problems.append('reaction centre outside the cytosol')
        # new nodes must not crowd anything that is not theirs
        mine = {s['nodeId'] for s in self.segs(r['id']) if s.get('nodeId')}
        others = [n for n in self.M['nodes'] if n['id'] not in mine]
        for nid in mine:
            n = self.N[nid]
            if n['kind'] == 'currency' or n.get('reactionLocalFor') == r['id']:
                for o in others:
                    dd = math.hypot(n['x'] - o['x'], n['y'] - o['y'])
                    if dd < n['r'] + o['r'] + 10:
                        problems.append(f'{nid} overlaps {o["id"]} ({dd:.0f})')
        return problems

    def record(self, local_rid, note):
        problems = self.check(local_rid)
        r = self.R[mid(local_rid)]
        gem = self.gem[local_rid]
        segs = [s for s in self.segs(r['id']) if s.get('nodeId')]
        main = [s for s in segs if s['kind'] == 'primary']
        lengths = [math.hypot(self.N[s['nodeId']]['x'] - r['x'], self.N[s['nodeId']]['y'] - r['y']) for s in main]
        self.log.append({'reaction': local_rid, 'name': gem['name'], 'equation': r['equation'],
                         'bounds': f'[{gem["lower_bound"]}, {gem["upper_bound"]}]',
                         'flux': self.flux.get(local_rid, 0.0), 'kind': r['kind'], 'note': note,
                         'nodes': ', '.join(sorted({self.N[s['nodeId']]['localId'] for s in main})),
                         'max_main': max(lengths) if lengths else 0, 'problems': problems})
        if problems:
            raise SystemExit(f'{local_rid}: ' + '; '.join(problems))


def main(map_path, model_path, flux_path, out_path, log_path):
    doc = json.load(open(map_path, encoding='utf-8'))
    model = json.load(open(model_path, encoding='utf-8'))
    flux = {r['local_id']: float(r['flux']) for r in csv.DictReader(open(flux_path, encoding='utf-8')) if r['species'] == CELL}
    b = Builder(doc, model, flux)
    N = b.N

    # L_ref from the hand-drawn chain
    chain = ['met:Bt@PGI_Bt:Bt@g6p[c_Bt]', 'met:Bt@PGI_Bt:Bt@f6p[c_Bt]', 'met:Bt@PFK_Bt:Bt@fdp[c_Bt]',
             'met:Bt@FBA_Bt:Bt@g3p[c_Bt]', 'met:Bt@GAPD_Bt:Bt@13dpg[c_Bt]', 'met:Bt@PGK_Bt:Bt@3pg[c_Bt]']
    S = statistics.median(math.hypot(N[p]['x'] - N[q]['x'], N[p]['y'] - N[q]['y']) for p, q in zip(chain, chain[1:]))

    PYR = 'met:Bt@PYK_Bt:Bt@pyr[c_Bt]'
    LACL = 'met:Bt@LDH_L_Bt:Bt@lac-L[c_Bt]'
    c = b.comp['c']
    MR, MB = c['x'] + c['w'], c['y'] + c['h']

    # 1. the glycolysis module, as one block, to the bottom-right of the cytosol
    glyco = [r['id'] for r in b.M['reactions'] if r['cellId'] == CELL and r['kind'] != 'transfer'
             and r['compartments'] == ['c']]
    P = (MR - 4.5 * S, MB - 2 * S)
    dx, dy = P[0] - N[PYR]['x'], P[1] - N[PYR]['y']
    b.translate_module(glyco, dx, dy)
    moved = f'translated by ({dx:.1f}, {dy:.1f}); shape unchanged'

    # L-lactate now comes up into pyruvate from below, its transporter under it
    N[LACL]['x'], N[LACL]['y'] = r2(P[0]), r2(P[1] + 1.3 * S)
    b.relayout_axis('LDH_L_Bt', LACL, PYR)

    # 2. the exchanges this batch feeds, next to where their transporters go
    # L-lactate straight under pyruvate, D-lactate to its left, pyruvate out to its right
    gap = TRANSPORT_GAP * S
    col = {'lac-D': P[0] - gap, 'lac-L': P[0], 'pyr': P[0] + gap}
    e_rect = b.cell['rect']
    EB, ER = e_rect['y'] + e_rect['h'], e_rect['x'] + e_rect['w']
    b.move_exchange('EX_lac-L[u]tr_Bt', col['lac-L'], EB, (0, 1))
    b.move_exchange('EX_pyr[u]tr_Bt', col['pyr'], EB, (0, 1))
    b.move_exchange('EX_lac-D[u]tr_Bt', col['lac-D'], EB, (0, 1))
    b.move_exchange('EX_ac[u]tr_Bt', ER, P[1], (1, 0))

    ext = lambda m: f'met:Bt@EX_{m}[u]tr_Bt:Bt@{m}[e_Bt]'

    # 3. reactions, one at a time
    # 3.1 L-lactate in, under lactate
    b.relayout_cofactors('PYK_Bt')
    b.relayout_cofactors('LDH_L_Bt')
    b.add_symport('L_LACt2r_Bt', LACL, ext('lac-L'), (0, 1))
    b.record('L_LACt2r_Bt', 'reuses lac-L[c] (LDH_L) and lac-L[e] (EX_lac-L); LDH_L redrawn vertical; ' + moved)

    # 3.2 D-lactate: LDH_D to the lower left, D_LACt2 under it
    lacd = b.node('met:Bt@LDH_D_Bt:Bt@lac-D[c_Bt]', 'lac-D[c_Bt]', 'metabolite', col['lac-D'], P[1] + 0.5 * S, rid='Bt@LDH_D_Bt')
    b.add_metabolic('LDH_D_Bt', lacd['id'], PYR, hands={'nad[c_Bt]': -1, 'nadh[c_Bt]': -1, 'h[c_Bt]': -1})
    b.record('LDH_D_Bt', 'reuses pyr[c]; creates lac-D[c]')
    b.add_symport('D_LACt2_Bt', lacd['id'], ext('lac-D'), (0, 1))
    b.record('D_LACt2_Bt', 'reuses lac-D[c] and lac-D[e] (EX_lac-D)')

    # 3.3 pyruvate out, from the hub down to its own column, between the lactates
    b.add_symport('PYRt2r_Bt', PYR, ext('pyr'), (0, 1), along=col['pyr'])
    b.record('PYRt2r_Bt', 'reuses pyr[c] (glycolysis) and pyr[e] (EX_pyr)')

    # 3.4 acetyl-CoA to the right, PFL straight and POR4i as a bypass above it
    accoa = b.node('met:Bt@PFL_Bt:Bt@accoa[c_Bt]', 'accoa[c_Bt]', 'metabolite', P[0] + 1.5 * S, P[1], rid='Bt@PFL_Bt')
    form = b.node('met:Bt@PFL_Bt:Bt@for[c_Bt]', 'for[c_Bt]', 'metabolite', P[0] + 0.95 * S, P[1] + 0.75 * S, rid='Bt@PFL_Bt')
    b.add_metabolic('PFL_Bt', PYR, accoa['id'], primaries={form['mergedId']: form['id']}, hands={'coa[c_Bt]': -1})
    b.record('PFL_Bt', 'reuses pyr[c]; creates accoa[c] and for[c] (formate is not exported in this scenario)')
    b.add_metabolic('POR4i_Bt', PYR, accoa['id'], centre=(P[0] + 0.75 * S, P[1] - 0.85 * S),
                    hands={m: -1 for m in ['coa[c_Bt]', 'fdxox[c_Bt]', 'h[c_Bt]', 'co2[c_Bt]', 'fdxrd[c_Bt]']})
    b.record('POR4i_Bt', 'reuses pyr[c] and accoa[c]; drawn as a bypass above PFL (R20)')

    # 3.5 acetyl phosphate and acetate, on to the right membrane
    actp = b.node('met:Bt@PTAr_Bt:Bt@actp[c_Bt]', 'actp[c_Bt]', 'metabolite', P[0] + 2.5 * S, P[1], rid='Bt@PTAr_Bt')
    b.add_metabolic('PTAr_Bt', accoa['id'], actp['id'], hands={'pi[c_Bt]': 1, 'coa[c_Bt]': 1})
    b.record('PTAr_Bt', 'reuses accoa[c]; creates actp[c]')
    ac = b.node('met:Bt@ACKr_Bt:Bt@ac[c_Bt]', 'ac[c_Bt]', 'metabolite', P[0] + 3.5 * S, P[1], rid='Bt@ACKr_Bt')
    b.add_metabolic('ACKr_Bt', ac['id'], actp['id'], hands={'atp[c_Bt]': 1, 'adp[c_Bt]': 1})
    b.record('ACKr_Bt', 'reuses actp[c]; creates ac[c]')
    b.add_symport('ACtr_Bt', ac['id'], ext('ac'), (1, 0))
    b.record('ACtr_Bt', 'reuses ac[c] and ac[e] (EX_ac)')
    b.tidy(['PYK_Bt', 'LDH_L_Bt', 'LDH_D_Bt', 'PFL_Bt', 'POR4i_Bt', 'PTAr_Bt', 'ACKr_Bt'])

    md = b.doc['metadata']
    md['reactionCount'], md['nodeCount'], md['segmentCount'] = len(b.M['reactions']), len(b.M['nodes']), len(b.M['segments'])
    json.dump(b.doc, open(out_path, 'w', encoding='utf-8'), ensure_ascii=False, indent=1)

    with open(log_path, 'w', encoding='utf-8') as f:
        f.write('# Fase 3b de Bt — registro de inserciones\n\n')
        f.write(f'Generado por `tools/gut9-map/build_phase3b.py`. L_ref = {S:.1f}. Flujos del escenario a μ = 0,078998.\n\n')
        f.write('| # | Reacción | Ecuación del GEM | Bounds | Flujo | Nodos principales | Tramo máx. | Nota |\n|---|---|---|---|---|---|---|---|\n')
        for i, e in enumerate(b.log, 1):
            f.write(f'| {i} | {e["reaction"]} | `{e["equation"]}` | {e["bounds"]} | {e["flux"]:.4g} | {e["nodes"]} | '
                    f'{e["max_main"]:.0f} | {e["note"]} |\n')
        f.write('\nCada inserción pasó la verificación de R51 antes de la siguiente: estequiometría y bounds iguales al GEM, '
                'cada segmento llega a su nodo, nodos en su compartimento, transportadores sobre la membrana citoplasmática '
                'y sin superposición de nodos.\n')
    print(f'L_ref {S:.1f}; pyruvate at ({P[0]:.0f}, {P[1]:.0f}); {len(b.log)} reactions added')
    for e in b.log:
        print(f'  {e["reaction"]:12} max main branch {e["max_main"]:6.0f}  ok')


if __name__ == '__main__':
    main(*sys.argv[1:6])
