"""Phase 7, automatic: a species' internal network redrawn substrate first.

What build_phase7_substrate_first.py does from a hand-made plan for St, done
from the fluxes for the other species:

* the phase 6 reactions of the species are taken out (hand-drawn ones from
  earlier phases are kept);
* every substrate the species takes up, by carbon flux, starts a spine at its
  transporter: the reaction with the most flux that consumes the metabolite,
  then the one that consumes its product, and so on, one step (S, about
  1.45 L_ref) at a time, straight inward from the membrane; a step turns 45 or
  90 degrees only when the straight place is taken, and towards the middle of
  the cell;
* a product is drawn where it is made. When the species exports it and its
  exit is within reach, the spine ends on the exit's node (decision (b));
  otherwise, if it is drawn elsewhere already, it is a duplicate there and the
  spine stops (decision (a)), so no pathway is drawn twice;
* then short branches (up to BRANCH steps) grow from the spines, by flux,
  until the species has its share of reactions;
* each spine of two or more reactions gets a text label, "Substrate -> End".

Each insertion goes through coxmap's checks (GEM, crossings, crowding) and is
rolled back when it fails; the next place is tried.

Usage: python build_phase7_spines.py IN OUT LOG SPECIES:TARGET [SPECIES:TARGET ...]
"""
import json
import math
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from build_phase5_transport import is_exchange, species  # noqa: E402
from build_phase6_grow import Grower, MIN_FLUX  # noqa: E402
from build_phase7_substrate_first import LABEL_SIZE, strip_internal  # noqa: E402
from coxmap import L_REF, Builder, CoxMap, dist, load, save, write_log  # noqa: E402

S = 280                    # one step along a spine
JOIN = 2.2 * S             # a product drawn this close is joined, not duplicated
SPINE_MAX = 12             # reactions in one spine
BRANCH = 3                 # reactions in one branch
SEED_MIN = 0.02            # carbon flux of the smallest substrate that starts a spine
ROOT = HERE.parents[1]
KEEP_FROM = ROOT / 'docs/gut9-map/maps/07-phase5-transporters.json'   # internal reactions drawn before phase 6


def unit(v):
    L = math.hypot(*v) or 1
    return (v[0] / L, v[1] / L)


def snap(v):
    """The axis direction nearest to v."""
    return (1 if v[0] > 0 else -1, 0) if abs(v[0]) >= abs(v[1]) else (0, 1 if v[1] > 0 else -1)


def rot(v, deg):
    a = math.radians(deg)
    return (v[0] * math.cos(a) - v[1] * math.sin(a), v[0] * math.sin(a) + v[1] * math.cos(a))


class SpineGrower(Grower):
    def __init__(self, cm, s, log):
        super().__init__(cm, s, log)
        self.used = set()
        self.chains = []
        self.made = []          # nodes this phase created, in order (branch roots)

    # ---- the network
    def oriented(self, rid):
        """Primary substrates and products of an active reaction, in the direction it runs."""
        s = self.s
        r = s.rxns[rid]
        sign = 1 if s.flux.get(rid, 0.0) >= 0 else -1
        subs = [k for k, c in r['metabolites'].items() if c * sign < 0 and not s.is_currency(k)]
        prods = [k for k, c in r['metabolites'].items() if c * sign > 0 and not s.is_currency(k)]
        return subs, prods

    def usable(self, rid):
        s = self.s
        if rid in self.used or is_exchange(rid) or abs(s.flux.get(rid, 0.0)) < MIN_FLUX:
            return False
        if s.compartments_of(rid) != ['c'] or len(s.rxns[rid]['metabolites']) > 12:
            return False
        subs, prods = self.oriented(rid)
        return bool(subs and prods)

    def weight(self, rid):
        """Biomass-precursor work comes last: its subsystem says so, or it handles a
        large molecule (an acyl carrier, a lipid, a cell-wall unit: more than 30 carbons)."""
        s = self.s
        if any(not s.is_currency(k) and s.carbons(k) > 30 for k in s.rxns[rid]['metabolites']):
            return 0.12
        # ModelSEED keeps the carrier apart as a cofactor: an acyl-ACP has few carbons of its own
        if any('acyl-carrier' in (s.mets[k].get('name') or '').lower() or 'ACP' in (s.mets[k].get('name') or '')
               for k in s.rxns[rid]['metabolites']):
            return 0.12
        return super().weight(rid)

    def central(self, rid):
        return self.weight(rid) >= 1

    def cflux(self, rid, local):
        # carbon flux, with polymers and lipids (hundreds of carbons) counted like a hexose pair
        return abs(self.s.flux.get(rid, 0.0)) * min(12, max(1, self.s.carbons(local))) * self.weight(rid)

    def consumers(self, local):
        """Active reactions that consume `local`, most carbon flux first."""
        out = []
        for rid in self.s.by_met.get(local, ()):
            if not self.usable(rid):
                continue
            subs, prods = self.oriented(rid)
            if local in subs:
                out.append((self.cflux(rid, local), rid, prods))
        return sorted(out, key=lambda t: -t[0])

    def producers(self, local):
        out = []
        for rid in self.s.by_met.get(local, ()):
            if not self.usable(rid):
                continue
            subs, prods = self.oriented(rid)
            if local in prods:
                out.append((self.cflux(rid, local), rid, subs))
        return sorted(out, key=lambda t: -t[0])

    # ---- the drawing
    def cytosol(self):
        return self.cm.comp[self.cell]['c']

    def inward(self, n):
        c = self.cytosol()
        x, y = n['x'], n['y']
        gaps = [(x - c['x'], (1, 0)), (c['x'] + c['w'] - x, (-1, 0)), (y - c['y'], (0, 1)), (c['y'] + c['h'] - y, (0, -1))]
        return min(gaps)[1]

    def toward_centre(self, p):
        c = self.cytosol()
        return unit((c['x'] + c['w'] / 2 - p[0], c['y'] + c['h'] / 2 - p[1]))

    def transport_nodes(self, out):
        """Cytosolic nodes of the cell's transporters: of what it takes up (out=False) or exports (out=True)."""
        cm, s = self.cm, self.s
        res = []
        for r in cm.M['reactions']:
            if r['cellId'] != self.cell or r['kind'] != 'transport' or r['localId'] not in s.rxns:
                continue
            v = s.flux.get(r['localId'], 0.0)
            if abs(v) < MIN_FLUX:
                continue
            gem = s.rxns[r['localId']]
            for seg in cm.segs[r['id']]:
                n = cm.N.get(seg.get('nodeId'))
                if not n or n['kind'] != 'metabolite' or n['compartment'] != 'c' or s.is_currency(n['localId']):
                    continue
                coef = gem['metabolites'].get(n['localId'], 0)
                if (coef * v < 0) == out:
                    res.append((abs(v) * max(1, s.carbons(n['localId'])), n))
        res.sort(key=lambda t: -t[0])
        return res

    def canonical(self, local):
        nodes = self.nodes_of(local)
        return nodes[0] if nodes else None

    def add(self, rid, a, b, note):
        """Draw reaction `rid` between nodes a and b, in the GEM's orientation."""
        s, cm = self.s, self.cm
        builder = Builder(cm, s, self.log)
        coef = s.rxns[rid]['metabolites'].get(a['localId'], -1)
        sub, prod = (a, b) if coef < 0 else (b, a)
        centre = ((a['x'] + b['x']) / 2, (a['y'] + b['y']) / 2)
        problems = builder.add_metabolic(rid, sub['id'], prod['id'],
                                         self.side_nodes(rid, centre, {a['localId'], b['localId']}), note=note)
        return problems

    def try_join(self, rid, a, b, note):
        if dist((a['x'], a['y']), (b['x'], b['y'])) > JOIN:
            return False
        if not self.cm.path_clear((a['x'], a['y']), (b['x'], b['y']), ends={a['id'], b['id']}):
            return False
        mark, lm = self.cm.mark(), len(self.log)
        if not self.add(rid, a, b, note):
            return True
        self.cm.rollback(mark)
        del self.log[lm:]
        return False

    def try_place(self, rid, a, local, direction, note, dup_of=None):
        """A new node for `local` one step from `a`, straight on if it can be."""
        cm = self.cm
        direction = snap(direction)
        centre = self.toward_centre((a['x'], a['y']))
        # horizontal and vertical only, as in the hand-drawn St and in Escher's maps
        turns = [0]
        for t in (90,):
            pair = [t, -t]
            # turn towards the middle of the cell first
            pair.sort(key=lambda d: -(rot(direction, d)[0] * centre[0] + rot(direction, d)[1] * centre[1]))
            turns += pair
        for t in turns:
            d = rot(direction, t)
            for step in (S, 1.25 * S):
                p = (a['x'] + d[0] * step, a['y'] + d[1] * step)
                if not self.in_ring(p) or cm.clearance(p, radius=170) < 70:
                    continue
                if not cm.path_clear((a['x'], a['y']), p, ends={a['id']}):
                    continue
                mark, lm = cm.mark(), len(self.log)
                b = Builder(cm, self.s, self.log).node(local, 'metabolite', p[0], p[1], self.s.mid(rid), duplicate_of=dup_of)
                problems = self.add(rid, a, b, note)
                if not problems:
                    problems = Builder(cm, self.s, self.log).check(rid, [cm.N[b['id']]])
                if not problems:
                    self.made.append(b['id'])
                    return cm.N[b['id']], d
                cm.rollback(mark)
                del self.log[lm:]
        return None, None

    def step(self, rid, a, local, direction, exits, note):
        """One reaction from node `a` to metabolite `local`. Returns (node, direction, go_on)."""
        # (b) the product leaves the cell here: end on its exit
        for ex in exits.get(local, []):
            if self.try_join(rid, a, ex, note + ' (ends on its exit)'):
                return ex, direction, False
        canon = self.canonical(local)
        if canon is not None:
            # drawn already: join it when close, else a duplicate here, and stop
            if self.try_join(rid, a, canon, note + ' (joins it)'):
                return canon, direction, False
            node, d = self.try_place(rid, a, local, direction, note + ' (duplicate)', dup_of=canon['id'])
            return node, d, False
        node, d = self.try_place(rid, a, local, direction, note)
        return node, d, node is not None

    def chain(self, start, direction, limit, exits, label_from=None):
        """Follow the most flux from `start` through central metabolism; returns the reactions drawn."""
        drawn = []
        cur, d = start, direction
        while len(drawn) < limit:
            options = [o for o in self.consumers(cur['localId']) if self.central(o[1])]
            moved = False
            for _, rid, prods in options[:3]:
                local = self.partner(cur['localId'], prods)
                self.used.add(rid)
                node, nd, go_on = self.step(rid, cur, local, d, exits, f'phase 7: from {self.s.base(cur["localId"])}')
                if node is None:
                    self.used.discard(rid)
                    continue
                drawn.append((rid, cur, node))
                moved = True
                if not go_on:
                    return drawn
                cur, d = node, nd
                break
            if not moved:
                break
        return drawn

    # ---- the whole species
    def grow_species(self, target):
        exits = {}
        for _, n in self.transport_nodes(out=True):
            exits.setdefault(n['localId'], []).append(n)
        # 1. spines from every substrate taken up
        for cf, seed in self.transport_nodes(out=False):
            if cf < SEED_MIN or self.count() >= target:
                break
            drawn = self.chain(seed, self.inward(seed), SPINE_MAX, exits)
            if drawn:
                self.chains.append(drawn)
        # 2. branches from what is drawn, by flux, until the species has its share
        progressed = True
        while progressed and self.count() < target:
            progressed = False
            roots = [self.cm.N[i] for i in self.made if i in self.cm.N]
            roots += [n for n in self.cm.M['nodes'] if n.get('cellId') == self.cell and n['kind'] == 'metabolite'
                      and n['compartment'] == 'c' and n['id'] not in self.made and not n.get('duplicateOf')]
            best = []
            for n in roots:
                for cf, rid, prods in self.consumers(n['localId']):
                    best.append((cf, 'out', rid, n, prods))
                for cf, rid, subs in self.producers(n['localId']):
                    best.append((cf, 'in', rid, n, subs))
            best.sort(key=lambda t: -t[0])
            for cf, way, rid, n, others in best[:30]:
                if rid in self.used:
                    continue
                d = self.away(n)
                if way == 'out':
                    drawn = self.chain_from(rid, n, others, d, exits)
                else:
                    drawn = self.branch_in(rid, n, others, d)
                if drawn:
                    progressed = True
                    if len(drawn) >= 2:
                        self.chains.append(drawn)
                    break
                self.used.add(rid)        # no room for it: leave it
        return self.count()

    def chain_from(self, rid, n, prods, d, exits):
        local = self.partner(n['localId'], prods)
        self.used.add(rid)
        node, nd, go_on = self.step(rid, n, local, d, exits, f'phase 7: branch from {self.s.base(n["localId"])}')
        if node is None:
            self.used.discard(rid)
            return []
        drawn = [(rid, n, node)]
        if go_on:
            drawn += self.chain(node, nd, BRANCH - 1, exits)
        return drawn

    def branch_in(self, rid, n, subs, d):
        """A reaction that makes a drawn metabolite: its substrate goes on the free side."""
        local = self.partner(n['localId'], subs)
        self.used.add(rid)
        canon = self.canonical(local)
        note = f'phase 7: feeds {self.s.base(n["localId"])}'
        if canon is not None:
            if self.try_join(rid, canon, n, note + ' (joins it)'):
                return [(rid, canon, n)]
            node, _ = self.try_place(rid, n, local, d, note + ' (duplicate)', dup_of=canon['id'])
        else:
            node, _ = self.try_place(rid, n, local, d, note)
        if node is None:
            self.used.discard(rid)
            return []
        return [(rid, node, n)]

    def away(self, n):
        """Where a branch from node n should head: away from the reactions already at it."""
        cm = self.cm
        taken = []
        for seg in cm.M['segments']:
            if seg.get('nodeId') == n['id']:
                r = cm.R[seg['reactionId']]
                taken.append(unit((r['x'] - n['x'], r['y'] - n['y'])))
        if not taken:
            return self.inward(n) if self.is_transport_node(n) else self.toward_centre((n['x'], n['y']))
        best = None
        for k in range(4):
            d = (round(math.cos(k * math.pi / 2), 6), round(math.sin(k * math.pi / 2), 6))
            score = -max(t[0] * d[0] + t[1] * d[1] for t in taken)
            c = self.toward_centre((n['x'], n['y']))
            score += 0.3 * (c[0] * d[0] + c[1] * d[1])
            if best is None or score > best[0]:
                best = (score, d)
        return best[1]

    def is_transport_node(self, n):
        return any(self.cm.R[seg['reactionId']]['kind'] == 'transport'
                   for seg in self.cm.M['segments'] if seg.get('nodeId') == n['id'])

    def count(self):
        return sum(1 for r in self.cm.M['reactions'] if r['cellId'] == self.cell and r['kind'] == 'metabolic')

    # ---- labels
    def labels(self, names, most=8):
        out = []
        taken = []
        # the pathways that carry the most carbon first; one reaction is enough for a main one
        weight = lambda drawn: max(self.cflux(rid, a['localId']) for rid, a, b in drawn)
        ranked = sorted(self.chains, key=weight, reverse=True)
        top = weight(ranked[0]) if ranked else 0
        for drawn in ranked:
            if len(out) >= most or (len(drawn) < 2 and weight(drawn) < 0.2 * top):
                continue
            first, last = drawn[0][1], drawn[-1][2]
            # a last step that only gives off a small side product does not name the pathway
            if len(drawn) >= 2 and self.s.carbons(drawn[-1][1]['localId']) > self.s.carbons(last['localId']):
                last = drawn[-1][1]
            if self.s.base(first['localId']) == self.s.base(last['localId']):
                continue
            text = f'{display_name(names, self.s.base(first["localId"]))} → ' \
                   f'{display_name(names, self.s.base(last["localId"]))}'
            spot = self.label_spot(drawn, text, taken)
            if spot:
                out.append((text, spot[0], spot[1]))
                taken.append(spot[2])
        return out

    def label_spot(self, drawn, text, taken):
        """Beside the spine, nearest where it leaves its substrate; a tighter fit if nothing else."""
        return (self._label_spot(drawn, text, taken, 35, (140, 220, 320))
                or self._label_spot(drawn, text, taken, 12, (100, 180, 260, 360)))

    def _label_spot(self, drawn, text, taken, pad, offsets):
        cm = self.cm
        w = 0.58 * LABEL_SIZE * len(text)
        h = LABEL_SIZE
        c = self.cytosol()
        best = None
        for i, (rid, a, b) in enumerate(drawn):
            r = cm.R.get(self.s.mid(rid))
            if not r:
                continue
            px, py = r['x'], r['y']
            for dx, dy in ((0, -1), (0, 1), (1, 0), (-1, 0)):
                for off in offsets:
                    x0 = px + dx * off - (w if dx < 0 else (0 if dx > 0 else w / 2))
                    y0 = py + dy * off - (h if dy < 0 else (0 if dy > 0 else h / 2))
                    box = (x0, y0, x0 + w, y0 + h)
                    if box[0] < c['x'] + 60 or box[2] > c['x'] + c['w'] - 60 or box[1] < c['y'] + 60 or box[3] > c['y'] + c['h'] - 60:
                        continue
                    if any(not (box[2] < t[0] or t[2] < box[0] or box[3] < t[1] or t[3] < box[1]) for t in taken):
                        continue
                    if not self.box_clear(box, pad):
                        continue
                    clear = min(cm.clearance((x, y), radius=120)
                                for x in (box[0], (box[0] + box[2]) / 2, box[2])
                                for y in (box[1], (box[1] + box[3]) / 2, box[3]))
                    score = min(clear, 120) - 0.25 * off - 15 * i
                    if best is None or score > best[0]:
                        best = (score, (x0, y0 + h * 0.8, box))
        return best[1] if best else None

    def box_clear(self, box, pad):
        """No branch runs through the box, and no node sits in it (with a margin)."""
        from coxmap import crosses
        x0, y0, x1, y1 = box[0] - pad, box[1] - pad, box[2] + pad, box[3] + pad
        inside = lambda p: x0 <= p[0] <= x1 and y0 <= p[1] <= y1
        edges = [((x0, y0), (x1, y0)), ((x1, y0), (x1, y1)), ((x1, y1), (x0, y1)), ((x0, y1), (x0, y0))]
        for a, b, rid, nid in self.cm.grid.near_pieces(x0, y0, x1, y1, 0):
            if inside(a) or inside(b) or any(crosses(a, b, p, q) for p, q in edges):
                return False
        cx, cy = (x0 + x1) / 2, (y0 + y1) / 2
        for n in self.cm.grid.near_nodes(cx, cy, max(x1 - x0, y1 - y0) / 2 + 20):
            if inside((n['x'], n['y'])):
                return False
        return True


ID_LIKE = __import__('re').compile(r'^(M_)?[\w\-]+(\[[\w]+\])?$')


def readable_names():
    """One readable name per metabolite base, from whichever of the nine GEMs has one."""
    from collections import Counter, defaultdict
    votes = defaultdict(Counter)
    for sp in ('Bt', 'St', 'Fp', 'Ec', 'Kp', 'Ef', 'Er', 'Lc', 'Ba'):
        try:
            s = species(sp)
        except Exception:
            continue
        for k, m in s.mets.items():
            name = (m.get('name') or '').strip()
            base = s.base(k)
            # an id written as a name (acald_c[c], M_pyr_c, cpd00020) is not one
            if not name or '[' in name or ID_LIKE.match(name) and not any(ch.isupper() for ch in name):
                continue
            if name.lower().replace('-', '_') == base.lower() or name.startswith(('M_', 'cpd')):
                continue
            votes[base][name] += 1
    out = {}
    for base, names in votes.items():
        name = min(names.items(), key=lambda kv: (-kv[1], len(kv[0])))[0]
        out[base] = name if len(name) <= 30 else name[:28].rstrip(' ,-(') + '…'
    out.update(NAMES)
    return out


def display_name(names, base):
    """The readable name of a base; charged and free tRNAs spelled out."""
    import re
    if base in names:
        return names[base]
    m = re.match(r'^trna(\w+)$', base)
    if m:
        return f'tRNA({m.group(1).capitalize()})'
    m = re.match(r'^(\w+?)trna$', base)
    if m:
        return f'{m.group(1).capitalize()}-tRNA'
    return base


# names no GEM of the community writes out
NAMES = {'asc6p': 'L-Ascorbate 6-phosphate', 'ibut': 'Isobutyrate', '2mbut': '2-Methylbutyrate', 'ival': 'Isovalerate',
         'WTAII': 'Wall teichoic acid', 'WTA_EFA': 'Wall teichoic acid', 'WTAI': 'Wall teichoic acid',
         'PG_EFA': 'Peptidoglycan', 'LTA_EFA': 'Lipoteichoic acid', 'CELB': 'Cellobiose', 'cellb': 'Cellobiose'}


def phase5_internal():
    M = json.load(open(KEEP_FROM, encoding='utf-8'))['map']
    return {r['id'] for r in M['reactions'] if r['kind'] == 'metabolic'}


def add_labels(cm, cell, labels):
    texts = cm.M.setdefault('texts', [])
    texts[:] = [t for t in texts if not t['id'].startswith(f'pathway-{cell}-')]
    for i, (text, x, y) in enumerate(labels, 1):
        texts.append({'id': f'pathway-{cell}-{i}', 'text': text, 'x': round(x, 1), 'y': round(y, 1),
                      'fontSize': LABEL_SIZE, 'fontFamily': 'sans'})


def main(src, out, log_path, *targets):
    cm = CoxMap(load(src))
    keep = phase5_internal()
    names = readable_names()
    entries = []
    report = []
    for spec in targets:
        cell, target = spec.split(':')
        s = species(cell)
        removed, dropped = strip_internal(cm, cell, keep=keep)
        log = []
        g = SpineGrower(cm, s, log)
        n = g.grow_species(int(target))
        labels = g.labels(names)
        add_labels(cm, cell, labels)
        entries += log
        report.append(f'{cell}: removed {removed} phase 6 reactions; now {n} internal ({len(log)} new), '
                      f'{len(g.chains)} spines/branches, {len(labels)} labels')
        print(report[-1], flush=True)
    save(cm.doc, out)
    write_log(log_path, 'Fase 7: espinas desde los sustratos (automático)',
              'Para cada especie se quitaron las reacciones de la fase 6 y se dibujó su metabolismo desde los '
              'transportadores de sus sustratos, siguiendo el flujo. Cada reacción se comparó con el GEM y con lo ya '
              'dibujado.', entries, closing='\n'.join('- ' + r for r in report), append=Path(log_path).exists())


if __name__ == '__main__':
    main(*sys.argv[1:])
