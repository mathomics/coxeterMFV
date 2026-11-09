"""Phase 6: grow a species' internal network from what is already drawn.

Candidates are the GEM reactions of the species that are active in the
scenario (|v| >= MIN_FLUX), lie in the cytosol, and touch a metabolite that
already has a primary node there (a transporter's cytosolic node, or an
earlier reaction's product). They are taken by flux, with biomass precursor
subsystems (cell envelope, lipids, cofactors...) far behind (R18, R19).

Placement, at the hand-drawn scale (L_ref = 193.8):

* the reaction's main pair is the drawn metabolite and the non-cofactor on
  the other side with the closest number of carbons (R22);
* a new main metabolite goes 1-1.5 L_ref from the drawn one, preferring to
  continue the direction its pathway arrives from, so chains run straight
  (R21), never closer than 1.4 L_ref to the membrane (the transporters' ring);
* when both main metabolites are drawn, they are joined only if they are at
  most 3 L_ref apart and the line crosses nothing (R08, R27); otherwise the
  reaction waits, rather than duplicating an intermediate (R11);
* other metabolites: an existing node within 2.5 L_ref is reused, cofactors
  are secondary nodes beside the reaction (R13), and a side metabolite drawn
  elsewhere gets a traced copy (`duplicateOf`).

Every insertion goes through the R51 check, crossings and clearance, and is
rolled back if it fails (R52); the next free place is then tried.

Usage: python build_phase6_grow.py MAP OUT SPECIES N LOG_MD
"""
import math
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from build_phase5_transport import is_exchange, species  # noqa: E402
from coxmap import LOW_PRIORITY, L_REF, Builder, CoxMap, dist, load, save, write_log  # noqa: E402

MIN_FLUX = 1e-4
RING = 1.4 * L_REF          # keep new metabolites this far from the cytoplasmic membrane
JOIN = 3.0 * L_REF          # longest link between two drawn metabolites
REUSE = 2.5 * L_REF         # a side metabolite drawn this close is reused
ANGLES = 16


class Grower:
    def __init__(self, cm, s, log):
        self.cm, self.s, self.log = cm, s, log
        self.cell = s.sp
        self.skipped = {}

    def drawn_rxns(self):
        return {r['localId'] for r in self.cm.M['reactions'] if r['cellId'] == self.cell}

    def nodes_of(self, local):
        return [n for n in self.cm.primary_nodes(self.cell, self.s.mid(local))
                if n['compartment'] == 'c' and not n.get('duplicateOf')]

    def weight(self, rid):
        sub = (self.s.rxns[rid].get('subsystem') or '').lower()
        return 0.12 if any(k in sub for k in LOW_PRIORITY) else 1.0

    def candidates(self):
        s = self.s
        drawn = self.drawn_rxns()
        out = []
        for rid, r in s.rxns.items():
            if rid in drawn or rid in self.skipped or is_exchange(rid):
                continue
            v = abs(s.flux.get(rid, 0.0))
            if v < MIN_FLUX:
                continue
            if s.compartments_of(rid) != ['c']:
                continue
            subs = [k for k, c in r['metabolites'].items() if c < 0 and not s.is_currency(k)]
            prods = [k for k, c in r['metabolites'].items() if c > 0 and not s.is_currency(k)]
            if not subs or not prods or len(r['metabolites']) > 12:
                continue
            anchored = [k for k in subs + prods if self.nodes_of(k)]
            if not anchored:
                continue
            both = any(self.nodes_of(k) for k in subs) and any(self.nodes_of(k) for k in prods)
            out.append((v * self.weight(rid) * (1.5 if both else 1.0), rid, subs, prods))
        out.sort(key=lambda t: -t[0])
        return out

    def partner(self, anchor, others):
        c0 = self.s.carbons(anchor)
        return min(others, key=lambda k: (abs(self.s.carbons(k) - c0), -self.s.carbons(k), k))

    def in_ring(self, p):
        c = self.cm.comp[self.cell]['c']
        return (c['x'] + RING <= p[0] <= c['x'] + c['w'] - RING and c['y'] + RING <= p[1] <= c['y'] + c['h'] - RING)

    def preferred(self, node):
        """Direction a pathway arrives at `node` from, to continue it straight;
        for a transporter's node, away from its membrane."""
        cm = self.cm
        incoming = []
        for seg in cm.M['segments']:
            if seg.get('nodeId') != node['id'] or seg['kind'] != 'primary':
                continue
            r = cm.R[seg['reactionId']]
            incoming.append((node['x'] - r['x'], node['y'] - r['y']))
        if incoming:
            dx = sum(v[0] for v in incoming)
            dy = sum(v[1] for v in incoming)
            L = math.hypot(dx, dy)
            if L > 1e-6:
                # snapped to the nearest of eight directions, so chains stay on the axes
                ang = round(math.atan2(dy, dx) / (math.pi / 4)) * (math.pi / 4)
                return (math.cos(ang), math.sin(ang))
        c = cm.comp[self.cell]['c']
        cx, cy = c['x'] + c['w'] / 2, c['y'] + c['h'] / 2
        L = math.hypot(cx - node['x'], cy - node['y']) or 1
        return ((cx - node['x']) / L, (cy - node['y']) / L)

    def spots(self, a):
        """Free places for a new main metabolite next to node `a`, best first."""
        cm = self.cm
        pref = self.preferred(a)
        out = []
        for rad in (1.0, 1.25, 1.5):
            for k in range(ANGLES):
                ang = 2 * math.pi * k / ANGLES
                d = (math.cos(ang), math.sin(ang))
                p = (a['x'] + d[0] * rad * L_REF, a['y'] + d[1] * rad * L_REF)
                if not self.in_ring(p):
                    continue
                cl = cm.clearance(p, radius=170)
                if cl < 70:
                    continue
                if not cm.path_clear((a['x'], a['y']), p, ends={a['id']}):
                    continue
                # hand-drawn maps run pathways along the axes: prefer those directions
                axis = 25 if k % (ANGLES // 4) == 0 else 8 if k % (ANGLES // 8) == 0 else 0
                # and a little room from the membrane, so the middle of the cell is used too
                c = cm.comp[self.cell]['c']
                depth = min(p[0] - c['x'], c['x'] + c['w'] - p[0], p[1] - c['y'], c['y'] + c['h'] - p[1])
                inward = 12 * min(depth / L_REF, 4)
                score = min(cl, 160) + 60 * (d[0] * pref[0] + d[1] * pref[1]) - 30 * (rad - 1) + axis + inward
                out.append((score, p))
        out.sort(key=lambda t: -t[0])
        return [p for _, p in out[:4]]

    def side_nodes(self, rid, centre, mains):
        s = self.s
        out = {}
        for k in s.rxns[rid]['metabolites']:
            if k in mains or s.is_currency(k):
                continue
            near = [n for n in self.nodes_of(k) if dist((n['x'], n['y']), centre) <= REUSE]
            if near:
                out[k] = min(near, key=lambda n: dist((n['x'], n['y']), centre))['id']
        return out

    def try_add(self, rid, subs, prods):
        s, cm = self.s, self.cm
        b = Builder(cm, s, self.log)
        drawn_s = [k for k in subs if self.nodes_of(k)]
        drawn_p = [k for k in prods if self.nodes_of(k)]
        note = 'grown from ' + ', '.join(sorted(set(drawn_s + drawn_p)))
        # both sides drawn: join them if close enough
        if drawn_s and drawn_p:
            pairs = sorted(((dist((x['x'], x['y']), (y['x'], y['y'])), x, y)
                            for k1 in drawn_s for x in self.nodes_of(k1) for k2 in drawn_p for y in self.nodes_of(k2)),
                           key=lambda t: t[0])
            for d, x, y in pairs[:3]:
                if d > JOIN or d < 0.6 * L_REF:
                    continue
                if not cm.path_clear((x['x'], x['y']), (y['x'], y['y']), ends={x['id'], y['id']}):
                    continue
                mark, lm = cm.mark(), len(self.log)
                centre = ((x['x'] + y['x']) / 2, (x['y'] + y['y']) / 2)
                problems = b.add_metabolic(rid, x['id'], y['id'], self.side_nodes(rid, centre, {x['localId'], y['localId']}),
                                           note=note + ' (joins two drawn metabolites)')
                if not problems:
                    return True
                cm.rollback(mark)
                del self.log[lm:]
            return False
        anchor_side = 'sub' if drawn_s else 'prod'
        anchors = drawn_s or drawn_p
        anchor = max(anchors, key=lambda k: (s.carbons(k), k))
        partner = self.partner(anchor, prods if anchor_side == 'sub' else subs)
        for a in sorted(self.nodes_of(anchor), key=lambda n: n['id']):
            for p in self.spots(a):
                mark, lm = cm.mark(), len(self.log)
                rx_id = s.mid(rid)
                new = b.node(partner, 'metabolite', p[0], p[1], rx_id)
                sub_id, prod_id = (a['id'], new['id']) if anchor_side == 'sub' else (new['id'], a['id'])
                centre = ((a['x'] + p[0]) / 2, (a['y'] + p[1]) / 2)
                problems = b.add_metabolic(rid, sub_id, prod_id, self.side_nodes(rid, centre, {anchor, partner}), note=note)
                if not problems:
                    # the new node must also be clear of what the reaction itself drew
                    return True
                cm.rollback(mark)
                del self.log[lm:]
        return False

    def grow(self, n):
        added = 0
        while added < n:
            cands = self.candidates()
            if not cands:
                break
            progressed = False
            for _, rid, subs, prods in cands[:25]:
                if self.try_add(rid, subs, prods):
                    added += 1
                    progressed = True
                    break
                self.skipped[rid] = 'no free place that keeps the drawing clear'
            if not progressed and len(cands) <= 25:
                break
        return added


def main(map_path, out_path, sp, n, log_path):
    cm = CoxMap(load(map_path))
    s = species(sp)
    log = []
    g = Grower(cm, s, log)
    added = g.grow(int(n))
    save(cm.doc, out_path)
    write_log(log_path, 'Fase 6 — metabolismo interno', '', log, append=Path(log_path).exists())
    print(f'{sp}: {added} reactions added')
    for e in log:
        print(f'   {e["reaction"]:22} {e["flux"]:10.4g}  {e["name"][:50]}')
    return added, log


if __name__ == '__main__':
    main(*sys.argv[1:6])
