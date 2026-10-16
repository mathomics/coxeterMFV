"""Rebuild phase 3a of the gut9 Coxeter map at the scale of the hand-drawn map.

Phase 3a (gut-nine-coxeter-map-phase3a-Bt-glycolysis.json) stretched the manual
Bt glycolysis about 4x, used reaction axes of 60-68 instead of 17, put cofactors
230-280 away from their reaction and drew every exchange about 1100-2150 long.

This script keeps the 3x3 cell grid and every reaction, node and segment id of
phase 3a, and only redraws the geometry:

* the 10 hand-drawn Bt reactions are copied from the manual map (published map v3)
  exactly, translated as one block into the new Bt cell (R02);
* the 6 reactions added in phase 3a are laid out at L_ref, the median distance
  between consecutive glycolysis intermediates in the manual map (R25);
* the extracellular (and periplasmic) bands go back to the manual width, and each
  exchange to the manual length: e node and medium node about 130 from the
  membrane, axis half-length 17.

Usage:
  python build_phase3a_v2.py BASE_MAP PHASE3A_MAP OUT_MAP
"""
import json
import math
import statistics
import sys

AXIS_HALF = 17           # SIZES.reactionAxis in src/layout/layout.js
EX_REACH = 130           # membrane -> e node and membrane -> medium node
BAND_E = 340             # e -> next compartment (manual Bt: 250-337)
BAND_P = 250             # p -> c in Ec and Kp
SAT_ALONG = 50           # cofactor offset along the reaction axis
SAT_ACROSS = 75          # cofactor offset across the axis, first row
SAT_ROW = 70             # extra offset for each further row
BRANCH_TURN_ALONG = 0.8  # src/layout/reaction-edit.js
BRANCH_TURN_ACROSS = 0.55

BASE_RXNS_NEW_GEOMETRY = ['Bt@PGMT_Bt', 'Bt@TPI_Bt', 'Bt@PGM_Bt', 'Bt@ENO_Bt', 'Bt@PYK_Bt', 'Bt@LDH_L_Bt']


def r2(v):
    return round(v, 2)


def pt(x, y):
    return {'x': r2(x), 'y': r2(y)}


def lerp(a, b, t):
    return pt(a['x'] + (b['x'] - a['x']) * t, a['y'] + (b['y'] - a['y']) * t)


def shape_branch(seg, rxn):
    """Port of shapeBranch() in src/layout/reaction-edit.js."""
    p0 = seg['from'] if seg['side'] == 'substrate' else seg['to']   # metabolite end
    p3 = seg['to'] if seg['side'] == 'substrate' else seg['from']   # reaction end
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
    if across < 4 and along > 0:
        seg['b1'] = lerp(p0, p3, 0.34)
        seg['b2'] = lerp(p0, p3, 0.72)
    else:
        hand = -1 if side < 0 else 1
        n = {'x': -u['y'] * hand, 'y': u['x'] * hand}
        reach = max(along, 0.5 * across, 14)
        seg['b1'] = pt(p0['x'] - n['x'] * across * BRANCH_TURN_ACROSS, p0['y'] - n['y'] * across * BRANCH_TURN_ACROSS)
        seg['b2'] = pt(p3['x'] + u['x'] * reach * BRANCH_TURN_ALONG, p3['y'] + u['y'] * reach * BRANCH_TURN_ALONG)
    seg.pop('control', None)


def set_axis(rxn, cx, cy, dx, dy):
    L = math.hypot(dx, dy)
    dx, dy = dx / L, dy / L
    rxn['x'], rxn['y'] = r2(cx), r2(cy)
    d = {'x': round(dx, 4), 'y': round(dy, 4)}
    rxn['axis'] = {'a1': pt(cx - dx * AXIS_HALF, cy - dy * AXIS_HALF),
                   'a2': pt(cx + dx * AXIS_HALF, cy + dy * AXIS_HALF), 'dir': d}
    rxn['axisDirectionHint'] = dict(d)
    rxn['placementDirection'] = dict(d)


def attach(seg, rxn, node):
    """Segment from a metabolite to the axis end of its side (as phase 3a and Coxeter do)."""
    if seg['side'] == 'substrate':
        seg['from'] = pt(node['x'], node['y'])
        seg['to'] = dict(rxn['axis']['a1'])
    else:
        seg['from'] = dict(rxn['axis']['a2'])
        seg['to'] = pt(node['x'], node['y'])
    shape_branch(seg, rxn)


def inset(rect, d):
    return {'x': rect['x'] + d, 'y': rect['y'] + d, 'w': rect['w'] - 2 * d, 'h': rect['h'] - 2 * d}


def main(base_path, p3a_path, out_path):
    base_doc = json.load(open(base_path, encoding='utf-8'))
    doc = json.load(open(p3a_path, encoding='utf-8'))
    B, M = base_doc['map'], doc['map']
    N = {n['id']: n for n in M['nodes']}
    R = {r['id']: r for r in M['reactions']}
    segs_of = {}
    for s in M['segments']:
        segs_of.setdefault(s['reactionId'], []).append(s)
    cells = {c['id']: c for c in M['cells']}

    # ---------------------------------------------------------------- bands
    for c in M['cells']:
        comps = {k['id']: k for k in c['compartments']}
        e = comps['e']['rect']
        if 'p' in comps:
            comps['p']['rect'] = inset(e, BAND_E)
            comps['c']['rect'] = inset(comps['p']['rect'], BAND_P)
        else:
            comps['c']['rect'] = inset(e, BAND_E)

    # ------------------------------------------ manual Bt block, translated
    bt_old = next(c for c in B['cells'] if c['id'] == 'Bt')['rect']
    bt_new = cells['Bt']['rect']
    T = (bt_new['x'] - bt_old['x'], bt_new['y'] - bt_old['y'])

    def tr(p):
        return pt(p['x'] + T[0], p['y'] + T[1])

    base_rxn_ids = [r['id'] for r in B['reactions']]
    for bn in B['nodes']:
        n = N[bn['id']]
        n['x'], n['y'] = r2(bn['x'] + T[0]), r2(bn['y'] + T[1])
    for br in B['reactions']:
        r = R[br['id']]
        r['x'], r['y'] = r2(br['x'] + T[0]), r2(br['y'] + T[1])
        r['axis'] = {'a1': tr(br['axis']['a1']), 'a2': tr(br['axis']['a2']), 'dir': dict(br['axis']['dir'])}
        if br.get('normal') is not None:
            r['normal'] = dict(br['normal'])
    # The base segments, translated, replace the phase 3a ones of these reactions.
    base_segs = []
    for bs in B['segments']:
        s = json.loads(json.dumps(bs))
        for k in ('from', 'to', 'b1', 'b2', 'control'):
            if k in s:
                s[k] = tr(s[k])
        base_segs.append(s)

    # ------------------------------------------- phase 3a reactions at L_ref
    def node_xy(nid):
        return N[nid]['x'], N[nid]['y']

    chain = ['met:Bt@PGI_Bt:Bt@g6p[c_Bt]', 'met:Bt@PGI_Bt:Bt@f6p[c_Bt]', 'met:Bt@PFK_Bt:Bt@fdp[c_Bt]',
             'met:Bt@FBA_Bt:Bt@g3p[c_Bt]', 'met:Bt@GAPD_Bt:Bt@13dpg[c_Bt]', 'met:Bt@PGK_Bt:Bt@3pg[c_Bt]']
    gaps = [math.hypot(node_xy(a)[0] - node_xy(b)[0], node_xy(a)[1] - node_xy(b)[1]) for a, b in zip(chain, chain[1:])]
    L = statistics.median(gaps)

    g6x, g6y = node_xy('met:Bt@PGI_Bt:Bt@g6p[c_Bt]')
    p3x, p3y = node_xy('met:Bt@PGK_Bt:Bt@3pg[c_Bt]')
    place = {
        'met:Bt@PGMT_Bt:Bt@g1p[c_Bt]': (g6x - L, g6y),
        'met:Bt@PGM_Bt:Bt@2pg[c_Bt]': (p3x, p3y + L),
        'met:Bt@ENO_Bt:Bt@pep[c_Bt]': (p3x, p3y + 2 * L),
        'met:Bt@PYK_Bt:Bt@pyr[c_Bt]': (p3x, p3y + 3 * L),
        'met:Bt@LDH_L_Bt:Bt@lac-L[c_Bt]': (p3x + L, p3y + 3 * L),
    }
    for nid, (x, y) in place.items():
        N[nid]['x'], N[nid]['y'] = r2(x), r2(y)

    for rid in BASE_RXNS_NEW_GEOMETRY:
        r = R[rid]
        segs = [s for s in segs_of[rid] if 'nodeId' in s]
        prim_s = [s for s in segs if s['side'] == 'substrate' and s['kind'] == 'primary']
        prim_p = [s for s in segs if s['side'] == 'product' and s['kind'] == 'primary']
        a, b = N[prim_s[0]['nodeId']], N[prim_p[0]['nodeId']]
        set_axis(r, (a['x'] + b['x']) / 2, (a['y'] + b['y']) / 2, b['x'] - a['x'], b['y'] - a['y'])
        d = r['axis']['dir']
        perp = (-d['y'], d['x'])
        for side, sign in (('substrate', -1), ('product', 1)):
            cur = [s for s in segs if s['side'] == side and s['kind'] == 'currency']
            for i, s in enumerate(cur):
                hand = 1 if i % 2 == 0 else -1
                across = (SAT_ACROSS + (i // 2) * SAT_ROW) * hand
                n = N[s['nodeId']]
                n['x'] = r2(r['x'] + d['x'] * SAT_ALONG * sign + perp[0] * across)
                n['y'] = r2(r['y'] + d['y'] * SAT_ALONG * sign + perp[1] * across)
        for s in segs:
            attach(s, r, N[s['nodeId']])
        for s in segs_of[rid]:
            if s['kind'] == 'axis':
                s['from'], s['to'] = dict(r['axis']['a1']), dict(r['axis']['a2'])
        if 'linear' in r:
            r['linear']['t'] = 0.5

    # --------------------------------------------------------------- exchanges
    for r in M['reactions']:
        if r['kind'] != 'transfer' or r['id'] in base_rxn_ids:
            continue
        nx, ny = r['normal']['x'], r['normal']['y']
        rect = cells[r['cellId']]['rect']
        # stay on the outer membrane, at the same place along it
        x = min(max(r['x'], rect['x']), rect['x'] + rect['w'])
        y = min(max(r['y'], rect['y']), rect['y'] + rect['h'])
        r['x'], r['y'] = r2(x), r2(y)
        r['axis'] = {'a1': pt(x - nx * AXIS_HALF, y - ny * AXIS_HALF),
                     'a2': pt(x + nx * AXIS_HALF, y + ny * AXIS_HALF), 'dir': {'x': nx, 'y': ny}}
        for s in segs_of[r['id']]:
            if s['kind'] == 'axis':
                s['from'], s['to'] = dict(r['axis']['a1']), dict(r['axis']['a2'])
                continue
            n = N[s['nodeId']]
            reach = -EX_REACH if s['side'] == 'substrate' else EX_REACH
            n['x'], n['y'] = r2(x + nx * reach), r2(y + ny * reach)
            attach(s, r, n)

    keep = [s for s in M['segments'] if s['reactionId'] not in base_rxn_ids]
    M['segments'] = base_segs + keep
    doc['metadata']['reactionCount'] = len(M['reactions'])
    doc['metadata']['nodeCount'] = len(M['nodes'])
    doc['metadata']['segmentCount'] = len(M['segments'])
    with open(out_path, 'w', encoding='utf-8') as f:
        json.dump(doc, f, ensure_ascii=False, indent=1)
    print(f'L_ref = {L:.1f} (gaps {[round(g, 1) for g in gaps]}); translation {T}')


if __name__ == '__main__':
    main(*sys.argv[1:4])
