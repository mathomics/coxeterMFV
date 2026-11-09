"""Generic building blocks for growing the gut9 Coxeter map.

What phases 3a-3c did by hand for Bt, for any species of the community:

* names: the four namespaces of the nine models (BiGG `x[c_Sp]`, Ef's
  `M_x_c[c_Ef]`, ModelSEED `cpd00002[c_Lc]`) reduced to one base, so a
  cofactor is recognised and `x[e]`/`x[c]` are known to be one metabolite;
* drawing: nodes, reactions and branches in the format Coxeter writes, with
  transporters as Coxeter draws them (lanes for every metabolite that crosses);
* checks: every reaction is compared with its GEM and its branches with the
  curves already drawn, and an insertion that fails is rolled back (R51, R52).

The scale is the hand-drawn map's: L_ref = 193.8, reaction axis 17.
"""
import copy
import json
import math
import re
from collections import defaultdict

L_REF = 193.8
AXIS_HALF = 17
EX_REACH = 130
LANE_OFFSET = 6
BAND_E = 340
BAND_P = 250

CURRENCY = {
    'h', 'h2o', 'atp', 'adp', 'amp', 'gtp', 'gdp', 'gmp', 'utp', 'udp', 'ump', 'ctp', 'cdp', 'cmp', 'itp', 'idp',
    'nad', 'nadh', 'nadp', 'nadph', 'fad', 'fadh2', 'fmn', 'fmnh2', 'pi', 'ppi', 'pppi', 'coa', 'co2', 'hco3', 'o2',
    'h2o2', 'nh4', 'nh3', 'q8', 'q8h2', 'mqn8', 'mql8', 'mqn7', 'mql7', 'mqn9', 'mql9', 'mqn10', 'mql10', 'mqn11',
    'mql11', '2dmmq8', '2dmmql8', 'fdxox', 'fdxrd', 'fdox', 'fdred', 'flxso', 'flxr', 'trdox', 'trdrd', 'grxox',
    'grxrd', 'gthrd', 'gthox', 'na1', 'k', 'so4', 'so3', 'h2s', 'h2', 'ACP', 'apoACP', 'fe2', 'fe3', 'mg2', 'mn2',
    'zn2', 'ca2', 'cl', 'cobalt2', 'cu2', 'cu', 'mobd', 'ni2', 'thf', 'mlthf', '5mthf', '10fthf', 'methf', 'amet',
    'ahcys', 'dhap_dummy',
}
CURRENCY_SEED = {
    'cpd00001', 'cpd00002', 'cpd00003', 'cpd00004', 'cpd00005', 'cpd00006', 'cpd00007', 'cpd00008', 'cpd00009',
    'cpd00010', 'cpd00011', 'cpd00012', 'cpd00013', 'cpd00018', 'cpd00067', 'cpd00038', 'cpd00031', 'cpd00062',
    'cpd00014', 'cpd00052', 'cpd00096', 'cpd00015', 'cpd00982', 'cpd00050', 'cpd00242', 'cpd11620', 'cpd11621',
    'cpd15560', 'cpd15561', 'cpd15499', 'cpd15500', 'cpd00025', 'cpd00971', 'cpd00205', 'cpd00048', 'cpd00239',
    'cpd11493', 'cpd12370', 'cpd00126', 'cpd00091', 'cpd00046', 'cpd10515', 'cpd10516', 'cpd00254', 'cpd00030',
    'cpd00034', 'cpd00063', 'cpd00099', 'cpd00149', 'cpd00058', 'cpd00087', 'cpd00125', 'cpd00017', 'cpd00019',
}
# subsystems that are mostly biomass precursors: kept for later, after the central network
LOW_PRIORITY = ('envelope', 'lipid', 'fatty', 'glycerophospho', 'peptidoglycan', 'lipopolysacch', 'murein',
                'cofactor', 'vitamin', 'trna', 'biomass', 'teichoic', 'membrane lipid', 'cell wall')


def dist(a, b):
    return math.hypot(a[0] - b[0], a[1] - b[1])


def r2(v):
    return round(v, 2)


def pt(x, y):
    return {'x': r2(x), 'y': r2(y)}


# --------------------------------------------------------------------- species


class Species:
    """One member's GEM and fluxes, with its names reduced to bases."""

    def __init__(self, sp, model, flux):
        self.sp = sp
        self.mets = {m['id']: m for m in model['metabolites']}
        self.rxns = {r['id']: r for r in model['reactions']}
        self.flux = flux
        self.by_met = defaultdict(list)
        for r in model['reactions']:
            for m in r['metabolites']:
                self.by_met[m].append(r['id'])

    def mid(self, local):
        return f'{self.sp}@{local}'

    def comp(self, local):
        return self.mets[local]['compartment']

    def base(self, local):
        raw = local.split('[')[0]
        if raw.startswith('M_'):
            raw = re.sub(r'_(c|e|p)$', '', raw[2:])
        # Ef mixes BiGG (lac-L) and SBML-escaped (lac_L, glc__D) names: one form
        return raw.replace('__', '_').replace('-', '_')

    def is_currency(self, local):
        b = self.base(local)
        if b in CURRENCY or b in CURRENCY_SEED:
            return True
        name = (self.mets[local].get('name') or '').lower()
        return name in ('h+', 'h2o', 'water', 'atp', 'adp', 'amp', 'nad', 'nadh', 'nadp', 'nadph', 'phosphate',
                        'ppi', 'diphosphate', 'coa', 'co2', 'o2', 'nh3', 'ammonium', 'h2')

    def carbons(self, local):
        f = self.mets[local].get('formula') or ''
        m = re.match(r'^C(\d*)(?![a-z])', f)
        if not m:
            return 0
        return int(m.group(1) or 1)

    def name(self, local):
        return self.mets[local].get('name') or local

    def equation(self, rid):
        r = self.rxns[rid]
        left = [(m, -c) for m, c in r['metabolites'].items() if c < 0]
        right = [(m, c) for m, c in r['metabolites'].items() if c > 0]
        fmt = lambda side: ' + '.join((f'{c:g} ' if c != 1 else '') + self.mid(m) for m, c in side)
        arrow = '<=>' if r['lower_bound'] < 0 else '-->'
        return f'{fmt(left)} {arrow} {fmt(right)}'

    def compartments_of(self, rid):
        return sorted({self.comp(m) for m in self.rxns[rid]['metabolites'] if m in self.mets})


# -------------------------------------------------------------------- geometry


def curve(seg, n=8):
    p0, p3 = seg['from'], seg['to']
    p1, p2 = seg.get('b1', p0), seg.get('b2', p3)
    if seg.get('side') == 'product':
        p1, p2 = seg.get('b2', p0), seg.get('b1', p3)
    pts = []
    for i in range(n + 1):
        t = i / n
        a, b, c, d = (1 - t) ** 3, 3 * (1 - t) ** 2 * t, 3 * (1 - t) * t ** 2, t ** 3
        pts.append((a * p0['x'] + b * p1['x'] + c * p2['x'] + d * p3['x'],
                    a * p0['y'] + b * p1['y'] + c * p2['y'] + d * p3['y']))
    return list(zip(pts, pts[1:]))


def seg_pieces(seg):
    if seg.get('nodeId'):
        return curve(seg)
    return [((seg['from']['x'], seg['from']['y']), (seg['to']['x'], seg['to']['y']))]


def pdist(p, a, b):
    dx, dy = b[0] - a[0], b[1] - a[1]
    L2 = dx * dx + dy * dy
    t = 0 if L2 == 0 else max(0, min(1, ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / L2))
    return math.hypot(p[0] - a[0] - t * dx, p[1] - a[1] - t * dy)


def crosses(p1, p2, p3, p4):
    def o(a, b, c):
        return (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
    return (o(p1, p2, p3) * o(p1, p2, p4) < 0) and (o(p3, p4, p1) * o(p3, p4, p2) < 0)


def shape_branch(seg, rxn):
    """Port of shapeBranch() in src/layout/reaction-edit.js; lanes stay straight."""
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
    if (across < 4 and along > 0) or seg.get('transportLane') or rxn.get('kind') == 'transfer':
        seg['b1'] = pt(p0['x'] + (p3['x'] - p0['x']) * 0.34, p0['y'] + (p3['y'] - p0['y']) * 0.34)
        seg['b2'] = pt(p0['x'] + (p3['x'] - p0['x']) * 0.72, p0['y'] + (p3['y'] - p0['y']) * 0.72)
    else:
        hand = -1 if side < 0 else 1
        n = {'x': -u['y'] * hand, 'y': u['x'] * hand}
        reach = max(along, 0.5 * across, 14)
        seg['b1'] = pt(p0['x'] - n['x'] * across * 0.55, p0['y'] - n['y'] * across * 0.55)
        seg['b2'] = pt(p3['x'] + u['x'] * reach * 0.8, p3['y'] + u['y'] * reach * 0.8)


class Grid:
    """Uniform spatial hash of branch pieces and nodes, for clearance queries."""

    def __init__(self, size=250):
        self.size = size
        self.pieces = defaultdict(list)   # cell -> [(a, b, reactionId, nodeId)]
        self.nodes = defaultdict(list)    # cell -> [node]

    def _cells(self, x0, y0, x1, y1):
        s = self.size
        for i in range(int(math.floor(min(x0, x1) / s)), int(math.floor(max(x0, x1) / s)) + 1):
            for j in range(int(math.floor(min(y0, y1) / s)), int(math.floor(max(y0, y1) / s)) + 1):
                yield (i, j)

    def add_piece(self, a, b, rid, nid):
        for c in self._cells(a[0], a[1], b[0], b[1]):
            self.pieces[c].append((a, b, rid, nid))

    def add_node(self, n):
        for c in self._cells(n['x'], n['y'], n['x'], n['y']):
            self.nodes[c].append(n)

    def near_pieces(self, x0, y0, x1, y1, pad):
        seen = set()
        for c in self._cells(x0 - pad, y0 - pad, x1 + pad, y1 + pad):
            for item in self.pieces.get(c, ()):
                k = id(item)
                if k not in seen:
                    seen.add(k)
                    yield item

    def near_nodes(self, x, y, pad):
        seen = set()
        for c in self._cells(x - pad, y - pad, x + pad, y + pad):
            for n in self.nodes.get(c, ()):
                if id(n) not in seen:
                    seen.add(id(n))
                    yield n


# ------------------------------------------------------------------------- map


class CoxMap:
    def __init__(self, doc):
        self.doc = doc
        self.M = doc['map']
        self.reindex()

    def reindex(self):
        M = self.M
        self.N = {n['id']: n for n in M['nodes']}
        self.R = {r['id']: r for r in M['reactions']}
        self.segs = defaultdict(list)
        for s in M['segments']:
            self.segs[s['reactionId']].append(s)
        self.cells = {c['id']: c for c in M['cells']}
        self.comp = {c['id']: {k['id']: k['rect'] for k in c['compartments']} for c in M['cells']}
        self.grid = Grid()
        for s in M['segments']:
            if s['kind'] in ('axis', 'transport-lane'):
                continue
            for a, b in seg_pieces(s):
                self.grid.add_piece(a, b, s['reactionId'], s.get('nodeId'))
        for n in M['nodes']:
            self.grid.add_node(n)

    # ---- transactions: insertions only append, so a rollback truncates
    def mark(self):
        return (len(self.M['nodes']), len(self.M['reactions']), len(self.M['segments']))

    def rollback(self, mark):
        n, r, s = mark
        del self.M['nodes'][n:]
        del self.M['reactions'][r:]
        del self.M['segments'][s:]
        self.reindex()

    # ---- creation
    def add_node(self, node):
        self.M['nodes'].append(node)
        self.N[node['id']] = node
        self.grid.add_node(node)
        return node

    def add_segment(self, seg):
        self.M['segments'].append(seg)
        self.segs[seg['reactionId']].append(seg)
        if seg['kind'] not in ('axis', 'transport-lane'):
            for a, b in seg_pieces(seg):
                self.grid.add_piece(a, b, seg['reactionId'], seg.get('nodeId'))
        return seg

    def free_id(self, root):
        if root not in self.N:
            return root
        k = 2
        while f'{root}#{k}' in self.N:
            k += 1
        return f'{root}#{k}'

    def primary_nodes(self, cell, merged):
        return [n for n in self.M['nodes'] if n.get('cellId') == cell and n.get('mergedId') == merged
                and n['kind'] == 'metabolite']

    def inside(self, cell, comp, p, pad=20):
        r = self.comp[cell][comp]
        if not (r['x'] + pad <= p[0] <= r['x'] + r['w'] - pad and r['y'] + pad <= p[1] <= r['y'] + r['h'] - pad):
            return False
        inner = {'e': 'p' if 'p' in self.comp[cell] else 'c', 'p': 'c'}.get(comp)
        if inner:
            c = self.comp[cell][inner]
            if c['x'] - pad <= p[0] <= c['x'] + c['w'] + pad and c['y'] - pad <= p[1] <= c['y'] + c['h'] + pad:
                return False
        return True

    # ---- clearance
    def clearance(self, p, skip_rid=None, own_nodes=(), radius=160):
        best = radius
        for a, b, rid, nid in self.grid.near_pieces(p[0], p[1], p[0], p[1], radius):
            if rid == skip_rid or (nid and nid in own_nodes):
                continue
            best = min(best, pdist(p, a, b))
        for n in self.grid.near_nodes(p[0], p[1], radius):
            if n['id'] in own_nodes or n['x'] < -1e5:
                continue
            best = min(best, math.hypot(p[0] - n['x'], p[1] - n['y']) - n.get('r', 13))
        return best

    def path_clear(self, a, b, skip_rid=None, ends=()):
        """Does the straight path a-b avoid every drawn branch and node, apart from
        the nodes it joins?"""
        for p, q, rid, nid in self.grid.near_pieces(a[0], a[1], b[0], b[1], 5):
            if rid == skip_rid or (nid and nid in ends):
                continue
            if crosses(a, b, p, q):
                return False
        for n in self.grid.near_nodes((a[0] + b[0]) / 2, (a[1] + b[1]) / 2, dist(a, b) / 2 + 40):
            if n['id'] in ends:
                continue
            if pdist((n['x'], n['y']), a, b) < n.get('r', 13) + 18:
                return False
        return True


# ------------------------------------------------------------------- builder


class Builder:
    """Adds reactions of one species to a CoxMap, each checked before the next."""

    SAT_ALONG = (45, 60)
    SAT_ACROSS = (60, 80, 100, 125)

    def __init__(self, cmap, species, log):
        self.m = cmap
        self.s = species
        self.cell = species.sp
        self.log = log

    # ---- nodes and reactions
    def node(self, local, kind, x, y, rid, root=None, duplicate_of=None):
        s = self.s
        merged = s.mid(local)
        nid = self.m.free_id(root or (f'cur:{rid}:{merged}' if kind == 'currency' else f'met:{rid}:{merged}'))
        n = {'id': nid, 'kind': kind, 'mergedId': merged, 'localId': local, 'base': local, 'name': s.name(local),
             'formula': s.mets[local].get('formula', ''), 'compartment': s.comp(local), 'cellId': self.cell,
             'x': r2(x), 'y': r2(y)}
        if kind == 'currency':
            n.update({'r': 7, 'primary': False, 'visualRole': 'secondary', 'reactionId': rid})
        else:
            n.update({'reactionLocalFor': rid, 'reactionLocal': True, 'r': 13, 'primary': True, 'visualRole': 'primary'})
        if duplicate_of:
            n['duplicateOf'] = duplicate_of
        return self.m.add_node(n)

    def reaction(self, local_rid, x, y, dvec, membrane=None, normal=None):
        s = self.s
        gem = s.rxns[local_rid]
        rid = s.mid(local_rid)
        L = math.hypot(*dvec) or 1
        d = {'x': round(dvec[0] / L, 4), 'y': round(dvec[1] / L, 4)}
        comps = s.compartments_of(local_rid)
        r = {'id': rid, 'localId': local_rid, 'name': gem.get('name') or local_rid, 'subsystem': gem.get('subsystem', ''),
             'cellId': self.cell, 'kind': 'transport' if len(comps) > 1 else 'metabolic', 'compartments': comps,
             'membrane': membrane, 'boundaryId': f'{self.cell}/{membrane}' if membrane else None,
             'onMembrane': bool(membrane), 'normal': normal, 'placementDirection': dict(d),
             'axisDirectionHint': dict(d), 'x': r2(x), 'y': r2(y), 'equation': s.equation(local_rid),
             'terminalMetaboliteId': None, 'bounds': {'lower': gem['lower_bound'], 'upper': gem['upper_bound']},
             'axis': {'a1': pt(x - d['x'] * AXIS_HALF, y - d['y'] * AXIS_HALF),
                      'a2': pt(x + d['x'] * AXIS_HALF, y + d['y'] * AXIS_HALF), 'dir': d},
             'transportMode': None, 'transportLanes': None}
        self.m.M['reactions'].append(r)
        self.m.R[rid] = r
        return r

    def axis_segment(self, r):
        self.m.add_segment({'reactionId': r['id'], 'kind': 'axis', 'from': dict(r['axis']['a1']),
                            'to': dict(r['axis']['a2']), 'arrow': False})

    def branch(self, r, node, coef, kind, end=None, extra=None):
        side = 'substrate' if coef < 0 else 'product'
        p = pt(node['x'], node['y'])
        if side == 'substrate':
            seg = {'reactionId': r['id'], 'kind': kind, 'from': p, 'to': end or dict(r['axis']['a1']),
                   'nodeId': node['id'], 'coefficient': coef, 'arrow': False, 'side': side, 'backboneAnchor': True}
        else:
            seg = {'reactionId': r['id'], 'kind': kind, 'from': end or dict(r['axis']['a2']), 'to': p,
                   'nodeId': node['id'], 'coefficient': coef, 'arrow': True, 'side': side, 'backboneAnchor': True}
        if extra:
            seg.update(extra)
        shape_branch(seg, r)
        return self.m.add_segment(seg)

    def spot(self, r, sign, comp, taken, along=None, across=None):
        """Free place for a side metabolite at one end of a reaction's axis."""
        d = r['axis']['dir']
        perp = (-d['y'], d['x'])
        best = None
        for a in (along or self.SAT_ALONG):
            for c in (across or self.SAT_ACROSS):
                for hand in (1, -1):
                    p = (r['x'] + d['x'] * a * sign + perp[0] * c * hand, r['y'] + d['y'] * a * sign + perp[1] * c * hand)
                    if not self.m.inside(self.cell, comp, p):
                        continue
                    cl = min([self.m.clearance(p, skip_rid=r['id'])] + [dist(p, q) - 14 for q in taken])
                    score = min(cl, 70) - 0.15 * c
                    if best is None or score > best[0]:
                        best = (score, p, cl)
        return best

    # ---- checks
    def check(self, rid_local, new_nodes):
        """R51 for one reaction, and its drawing against everything else."""
        s, m = self.s, self.m
        r = m.R[s.mid(rid_local)]
        gem = s.rxns[rid_local]
        problems = []
        drawn = defaultdict(float)
        for seg in m.segs[r['id']]:
            if not seg.get('nodeId'):
                continue
            n = m.N.get(seg['nodeId'])
            if n is None:
                problems.append(f'segment to missing node {seg["nodeId"]}')
                continue
            drawn[n['mergedId']] += seg['coefficient']
            end = seg['from'] if seg['side'] == 'substrate' else seg['to']
            if math.hypot(end['x'] - n['x'], end['y'] - n['y']) > 0.5:
                problems.append(f'segment does not reach {n["id"]}')
            if n['compartment'] != s.comp(n['localId']):
                problems.append(f'{n["id"]} has the wrong compartment')
            if not m.inside(self.cell, n['compartment'], (n['x'], n['y']), 0):
                problems.append(f'{n["id"]} outside its compartment')
        want = {s.mid(k): v for k, v in gem['metabolites'].items()}
        if {k: round(v, 9) for k, v in drawn.items()} != {k: round(v, 9) for k, v in want.items()}:
            problems.append(f'stoichiometry differs from the GEM')
        if r['bounds'] != {'lower': gem['lower_bound'], 'upper': gem['upper_bound']}:
            problems.append('bounds differ from the GEM')
        # crossings of the new branches with anything drawn before
        mine = [seg for seg in m.segs[r['id']] if seg.get('nodeId') and seg['kind'] != 'transport-lane']
        my_nodes = {seg['nodeId'] for seg in mine}
        for seg in mine:
            for a, b in seg_pieces(seg):
                for p, q, rid2, nid2 in m.grid.near_pieces(a[0], a[1], b[0], b[1], 2):
                    if rid2 == r['id'] or (nid2 and nid2 == seg['nodeId']):
                        continue
                    if crosses(a, b, p, q):
                        problems.append(f'branch to {seg["nodeId"]} crosses {rid2}')
                        break
        # new nodes must be clear of each other and of what is not theirs
        for i, n in enumerate(new_nodes):
            for o in new_nodes[i + 1:]:
                if dist((n['x'], n['y']), (o['x'], o['y'])) < n['r'] + o['r'] + 8:
                    problems.append(f'{n["id"]} overlaps {o["id"]}')
        # same criterion as audit.py: no node within CLEAR + its radius of a branch not its own
        CLEAR = 22
        for n in new_nodes:
            cl = m.clearance((n['x'], n['y']), skip_rid=r['id'], own_nodes={n['id']} | my_nodes, radius=60)
            if cl < 20:
                problems.append(f'{n["id"]} is {cl:.0f} from another drawing')
            for a, b, rid2, nid2 in m.grid.near_pieces(n['x'], n['y'], n['x'], n['y'], CLEAR + 20):
                if rid2 == r['id'] or nid2 == n['id']:
                    continue
                if pdist((n['x'], n['y']), a, b) < CLEAR + n.get('r', 13):
                    problems.append(f'{n["id"]} crowds a branch of {rid2}')
                    break
        # and the new branches must not run past a node that is not theirs
        for seg in mine:
            for a, b in seg_pieces(seg):
                for n in m.grid.near_nodes((a[0] + b[0]) / 2, (a[1] + b[1]) / 2, dist(a, b) / 2 + CLEAR + 20):
                    if n['id'] in my_nodes or n['x'] < -1e5:
                        continue
                    if pdist((n['x'], n['y']), a, b) < CLEAR + n.get('r', 13):
                        problems.append(f'branch to {seg["nodeId"]} runs past {n["id"]}')
                        break
        return sorted(set(problems))

    def record(self, rid_local, note, kind):
        s = self.s
        r = self.m.R[s.mid(rid_local)]
        main = [seg for seg in self.m.segs[r['id']] if seg.get('kind') == 'primary']
        longest = max((math.hypot(self.m.N[g['nodeId']]['x'] - r['x'], self.m.N[g['nodeId']]['y'] - r['y']) for g in main),
                      default=0)
        self.log.append({'species': s.sp, 'reaction': rid_local, 'name': s.rxns[rid_local].get('name') or '',
                         'equation': r['equation'], 'flux': s.flux.get(rid_local, 0.0), 'kind': kind, 'note': note,
                         'longest': longest})

    # ---- membranes
    def membrane_point(self, comp, edge, along):
        """A point on the boundary of compartment `comp`, on `edge`, at `along`."""
        r = self.m.comp[self.cell][comp]
        if edge == 'top':
            return (along, r['y'])
        if edge == 'bottom':
            return (along, r['y'] + r['h'])
        if edge == 'left':
            return (r['x'], along)
        return (r['x'] + r['w'], along)

    def add_transport(self, rid_local, comp_in, comp_out, edge, along, out_nodes, in_nodes, note):
        """A transporter on the boundary of `comp_in` (the compartment it opens
        into). `out_nodes`/`in_nodes` map metabolite -> node id for the primary
        metabolites already placed; every other metabolite becomes a cofactor on
        its own side. Metabolites crossing with the same base get one lane each."""
        s, m = self.s, self.m
        gem = s.rxns[rid_local]
        normal = {'top': (0, -1), 'bottom': (0, 1), 'left': (-1, 0), 'right': (1, 0)}[edge]
        nx, ny = normal
        x, y = self.membrane_point(comp_in, edge, along)
        subs_out = any(c < 0 and s.comp(k) == comp_out for k, c in gem['metabolites'].items())
        d = (-nx, -ny) if subs_out else (nx, ny)
        r = self.reaction(rid_local, x, y, d, membrane=comp_in, normal={'x': nx, 'y': ny})
        by_base = defaultdict(set)
        for k in gem['metabolites']:
            by_base[s.base(k)].add(s.comp(k))
        crossing = [b for b, cs in by_base.items() if comp_in in cs and comp_out in cs]
        perp = (-ny, nx)
        lanes = {}
        if len(crossing) >= 2:
            order = sorted(crossing, key=lambda b: (s.is_currency(next(k for k in gem['metabolites'] if s.base(k) == b)), b))
            dirs = set()
            for i, b in enumerate(order):
                off = (i - (len(order) - 1) / 2) * 2 * LANE_OFFSET
                k_out = next(k for k in gem['metabolites'] if s.base(k) == b and s.comp(k) == comp_out)
                direction = 1 if gem['metabolites'][k_out] > 0 else -1     # +: outward at positive flux
                dirs.add(direction)
                lanes[b] = {'base': b, 'offset': off, 'direction': direction,
                            'insidePoint': pt(x - nx * AXIS_HALF + perp[0] * off, y - ny * AXIS_HALF + perp[1] * off),
                            'outsidePoint': pt(x + nx * AXIS_HALF + perp[0] * off, y + ny * AXIS_HALF + perp[1] * off)}
            r['transportMode'] = 'antiport' if len(dirs) > 1 else 'symport'
            r['transportLanes'] = list(lanes.values())
            for lane in lanes.values():
                m.add_segment({'id': f'{rid_local}:lane:{lane["base"]}', 'reactionId': r['id'], 'kind': 'transport-lane',
                               'from': lane['insidePoint'], 'to': lane['outsidePoint'], 'arrow': False,
                               'transportBase': lane['base'], 'transportDirection': lane['direction']})
        else:
            r['transportMode'] = 'uniport'
            self.axis_segment(r)
        new_nodes = []
        taken = []
        for k, coef in gem['metabolites'].items():
            side_out = s.comp(k) == comp_out
            given = (out_nodes if side_out else in_nodes).get(k)
            if given:
                node = m.N[given]
            else:
                # a cofactor beside the transporter, on its own side
                sign = 1 if side_out else -1
                best = None
                for depth in (75, 125, 175):
                    for off in (50, 80, 110):
                        for hand in (1, -1):
                            p = (x + nx * depth * sign + perp[0] * off * hand, y + ny * depth * sign + perp[1] * off * hand)
                            if not m.inside(self.cell, s.comp(k), p, 5) or any(dist(p, q) < 30 for q in taken):
                                continue
                            cl = min([m.clearance(p, skip_rid=r['id'])] + [dist(p, q) - 14 for q in taken])
                            score = min(cl, 60) - 0.05 * depth - 0.2 * off   # deeper rather than wider: neighbours are 262 away
                            if best is None or score > best[0]:
                                best = (score, p)
                if best is None:
                    return None
                taken.append(best[1])
                node = self.node(k, 'currency', best[1][0], best[1][1], r['id'])
                new_nodes.append(node)
            kind = 'currency' if node['kind'] == 'currency' else 'primary'
            b = s.base(k)
            if b in lanes:
                lane = lanes[b]
                end = dict(lane['outsidePoint'] if side_out else lane['insidePoint'])
                self.branch(r, node, coef, kind, end=end, extra={'transportBase': b, 'transportLane': True})
            else:
                self.branch(r, node, coef, kind)
        problems = self.check(rid_local, new_nodes)
        if problems:
            return problems
        self.record(rid_local, note, 'transport')
        return []

    # ---- metabolic reactions
    def add_metabolic(self, rid_local, sub_node, prod_node, side_nodes=None, note=''):
        """A reaction drawn between two primary nodes. `side_nodes` maps other
        metabolites to existing nodes; the rest are placed beside it: cofactors as
        secondary nodes, other metabolites as primary ones."""
        s, m = self.s, self.m
        gem = s.rxns[rid_local]
        a, b = m.N[sub_node], m.N[prod_node]
        r = self.reaction(rid_local, (a['x'] + b['x']) / 2, (a['y'] + b['y']) / 2, (b['x'] - a['x'], b['y'] - a['y']))
        self.axis_segment(r)
        given = {a['localId']: a['id'], b['localId']: b['id']}
        given.update(side_nodes or {})
        new_nodes, taken = [], []
        for k, coef in gem['metabolites'].items():
            if k in given:
                self.branch(r, m.N[given[k]], coef, 'primary' if m.N[given[k]]['kind'] == 'metabolite' else 'currency')
                continue
            sign = -1 if coef < 0 else 1
            cur = s.is_currency(k)
            best = self.spot(r, sign, s.comp(k), taken,
                             across=None if cur else (90, 120, 150), along=None if cur else (60, 90))
            if best is None:
                return ['no room for ' + k]
            taken.append(best[1])
            if cur:
                n = self.node(k, 'currency', best[1][0], best[1][1], r['id'])
            else:
                twins = m.primary_nodes(self.cell, s.mid(k))
                n = self.node(k, 'metabolite', best[1][0], best[1][1], r['id'],
                              duplicate_of=twins[0]['id'] if twins else None)
            new_nodes.append(n)
            self.branch(r, n, coef, 'currency' if cur else 'primary')
        problems = self.check(rid_local, new_nodes)
        if problems:
            return problems
        self.record(rid_local, note, 'metabolic')
        return []


# ------------------------------------------------------------------- reports


def write_log(path, title, intro, entries, closing='', append=False):
    with open(path, 'a' if append else 'w', encoding='utf-8') as f:
        if not append:
            f.write(f'# {title}\n\n{intro}\n\n')
        f.write('| # | Especie | Reacción | Ecuación del GEM | Flujo | Tipo | Nota |\n|---|---|---|---|---|---|---|\n')
        for i, e in enumerate(entries, 1):
            f.write(f'| {i} | {e["species"]} | {e["reaction"]} | `{e["equation"]}` | {e["flux"]:.4g} | {e["kind"]} | {e["note"]} |\n')
        if closing:
            f.write(f'\n{closing}\n')
        f.write('\n')


def load(path):
    return json.load(open(path, encoding='utf-8'))


def save(doc, path):
    md = doc['metadata']
    M = doc['map']
    md['reactionCount'], md['nodeCount'], md['segmentCount'] = len(M['reactions']), len(M['nodes']), len(M['segments'])
    json.dump(doc, open(path, 'w', encoding='utf-8'), ensure_ascii=False, indent=1)
