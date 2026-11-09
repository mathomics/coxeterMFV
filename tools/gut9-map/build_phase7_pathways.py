"""Phase 7, second pass: complete the conceptual pathways of each species.

The spines of build_phase7_spines.py follow the flux, so a pathway whose steps
carry little or no flux in this scenario was left half drawn. Here every
pathway of pathways.PATHWAYS that a species uses at all (one active step is
enough) is completed with the steps its GEM really has (R18, R24), active or
not, before any bypass (R19):

* a step whose two metabolites are drawn is joined when they are close and the
  line is clear; when they are far apart, the product is drawn again next to
  the substrate as a traced copy (duplicateOf), and this is logged;
* a step with one metabolite drawn grows the other one step away from it,
  along the axes;
* a step with neither drawn waits for a later pass.

Then the bypasses and alternative routes that carry flux are added (R20):
a second reaction between two metabolites already joined is drawn as a
parallel arc beside the first, so it reads as a derivation of the same step;
other alternatives connect their own intermediates as above.

Each pathway with three or more steps drawn in a cell gets its name as a label.
Every insertion goes through coxmap's checks and is rolled back when it fails.

Usage: python build_phase7_pathways.py IN OUT LOG SPECIES [SPECIES ...]
"""
import math
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from build_phase5_transport import species  # noqa: E402
from build_phase7_spines import JOIN, LABEL_SIZE, S, SpineGrower, snap, unit  # noqa: E402
from coxmap import Builder, CoxMap, dist, load, pt, save, shape_branch, write_log  # noqa: E402
from pathways import PATHWAYS, canonical, step_reactions  # noqa: E402

ACTIVE = 1e-4
OFFSET = 130            # how far a parallel bypass sits beside the main reaction


class PathwayFiller(SpineGrower):
    def __init__(self, cm, s, log, forced=()):
        super().__init__(cm, s, log)
        self.canon = canonical(s, species)
        self.notes = []          # what could not be drawn, and why
        # pathways drawn whatever the flux, first and in this order: a species'
        # characteristic route when it carries no flux in the scenario (R18)
        self.forced = list(forced)

    def pathways(self):
        first = [p for name in self.forced for p in PATHWAYS if p[0] == name]
        return first + [p for p in PATHWAYS if p[0] not in self.forced]

    # ---- what is drawn
    def drawn_ids(self):
        return {r['localId'] for r in self.cm.M['reactions'] if r['cellId'] == self.cell}

    def nodes_for(self, base, comp):
        """Drawn primary nodes (not traced copies) of a BiGG base in a compartment of this cell."""
        return [n for n in self.cm.M['nodes']
                if n.get('cellId') == self.cell and n['kind'] == 'metabolite' and n['compartment'] == comp
                and not n.get('duplicateOf') and self.canon.get(n['localId']) == base]

    def local_for(self, rid, base, comp):
        for k in self.s.rxns[rid]['metabolites']:
            if self.canon.get(k) == base and self.s.comp(k) == comp:
                return k
        return None

    def joined(self, a_nodes, b_nodes):
        """A drawn reaction already links one of a_nodes to one of b_nodes."""
        ids_a = {n['id'] for n in a_nodes}
        ids_b = {n['id'] for n in b_nodes}
        by_rxn = {}
        for seg in self.cm.M['segments']:
            if seg.get('nodeId') in ids_a or seg.get('nodeId') in ids_b:
                by_rxn.setdefault(seg['reactionId'], set()).add(seg['nodeId'])
        for rid, ids in by_rxn.items():
            if ids & ids_a and ids & ids_b:
                return self.cm.R[rid]
        return None

    # ---- placing one step
    def try_place(self, rid, a, local, direction, note, dup_of=None):
        """As for the spines, but a completed pathway has to fit where the spines left
        room: the four axis directions first (straight on, the sides, back), then the
        diagonals, at 0.85 to 1.5 steps."""
        from build_phase7_spines import rot
        cm = self.cm
        direction = snap(direction)
        centre = self.toward_centre((a['x'], a['y']))
        def lean(t):
            d = rot(direction, t)
            return -(d[0] * centre[0] + d[1] * centre[1])
        turns = [0] + sorted([90, -90], key=lean) + [180] + sorted([45, -45, 135, -135], key=lean)
        for t in turns:
            d = rot(direction, t)
            for step in (S, 1.25 * S, 0.85 * S, 1.5 * S):
                p = (a['x'] + d[0] * step, a['y'] + d[1] * step)
                if not self.in_ring(p) or cm.clearance(p, radius=170) < 60:
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

    def place_step(self, rid, step, note):
        a_base, b_base, comp, _ = step
        a_loc, b_loc = self.local_for(rid, a_base, comp), self.local_for(rid, b_base, comp)
        if not a_loc or not b_loc:
            return False
        A, B = self.nodes_for(a_base, comp), self.nodes_for(b_base, comp)
        if A and B:
            pairs = sorted(((dist((x['x'], x['y']), (y['x'], y['y'])), x, y) for x in A for y in B), key=lambda t: t[0])
            for d, x, y in pairs[:3]:
                if self.try_join(rid, x, y, note):
                    return True
            # far apart: the product again beside the substrate, traced to its drawn node
            x, y = pairs[0][1], pairs[0][2]
            node, _ = self.try_place(rid, x, b_loc, self.away(x), note + ' (product drawn again beside it)',
                                     dup_of=y['id'])
            if node is not None:
                self.notes.append(f'{rid}: {b_base} drawn again next to {a_base} (the drawn one is {d:.0f} away)')
                return True
            return False
        if A:
            node, _ = self.try_place(rid, A[0], b_loc, self.away(A[0]), note)
            return node is not None or self.from_copy(rid, A[0], b_loc, note)
        if B:
            # a branch that feeds a drawn metabolite comes in from the side, leaving
            # the way ahead to the steps that go on from it
            from build_phase7_spines import rot
            node, _ = self.try_place(rid, B[0], a_loc, rot(self.away(B[0]), 90), note)
            return node is not None or self.from_copy(rid, B[0], a_loc, note)
        return False

    def from_copy(self, rid, anchor, local, note):
        """No room round the drawn intermediate: the pathway goes on from a traced copy
        of it in the nearest free space (a documented exception to R11)."""
        cm = self.cm
        for ring in (2, 3, 4, 5, 6):
            for k in range(8):
                ang = k * math.pi / 4
                p = (anchor['x'] + math.cos(ang) * ring * S, anchor['y'] + math.sin(ang) * ring * S)
                if not self.in_ring(p) or cm.clearance(p, radius=220) < 150:
                    continue
                mark, lm = cm.mark(), len(self.log)
                copy = Builder(cm, self.s, self.log).node(anchor['localId'], 'metabolite', p[0], p[1], self.s.mid(rid),
                                                          duplicate_of=anchor['id'])
                node, _ = self.try_place(rid, cm.N[copy['id']], local, self.toward_centre(p),
                                         note + ' (from a traced copy of its intermediate)')
                if node is not None:
                    self.notes.append(f'{rid}: no room beside {self.s.base(anchor["localId"])}; '
                                      f'continued from a traced copy {ring * S:.0f} away')
                    return True
                cm.rollback(mark)
                del self.log[lm:]
        return False

    def place_parallel(self, rid, main, step, note):
        """A second reaction for a step already drawn: an arc beside the main one."""
        cm, s = self.cm, self.s
        a_base, b_base, comp, _ = step
        a_loc, b_loc = self.local_for(rid, a_base, comp), self.local_for(rid, b_base, comp)
        ids = {seg['nodeId'] for seg in cm.segs[main['id']] if seg.get('nodeId')}
        A = [n for n in self.nodes_for(a_base, comp) if n['id'] in ids]
        B = [n for n in self.nodes_for(b_base, comp) if n['id'] in ids]
        if not A or not B or not a_loc or not b_loc:
            return False
        a, b = A[0], B[0]
        d = unit((b['x'] - a['x'], b['y'] - a['y']))
        perp = (-d[1], d[0])
        for side in (1, -1):
            for off in (OFFSET, 1.5 * OFFSET):
                centre = ((a['x'] + b['x']) / 2 + perp[0] * off * side, (a['y'] + b['y']) / 2 + perp[1] * off * side)
                mark, lm = cm.mark(), len(self.log)
                problems = add_metabolic_at(Builder(cm, s, self.log), rid, a, b, centre,
                                            self.side_nodes(rid, centre, {a_loc, b_loc}), note)
                if not problems:
                    return True
                cm.rollback(mark)
                del self.log[lm:]
        return False

    # ---- the species
    def relevant(self, name, steps):
        return name in self.forced or any(step_reactions(self.s, self.canon, st, ACTIVE) for st in steps)

    def choose(self, rids):
        """The reaction that stands for a step: the most active, then the plainest id."""
        return sorted(rids, key=lambda r: (-abs(self.s.flux.get(r, 0.0)), len(r), r))[0]

    def fill(self):
        added = 0
        # 1. the main route of every pathway the species uses, until nothing more fits:
        # first the steps that go on from what is drawn, then also the ones that feed it,
        # so a feeding branch never takes the place where a pathway goes on
        for feeding in (False, True):
            progress = True
            while progress:
                progress = False
                for name, steps in self.pathways():
                    if not self.relevant(name, steps):
                        continue
                    for step in steps:
                        if step[3]:
                            continue
                        rids = step_reactions(self.s, self.canon, step)
                        if not rids or set(rids) & self.drawn_ids():
                            continue
                        if not feeding and not self.nodes_for(step[0], step[2]):
                            continue
                        if self.place_step(self.choose(rids), step, f'phase 7 pathways: {name}'):
                            added += 1
                            progress = True
        # 2. bypasses and alternative routes that carry flux
        for name, steps in self.pathways():
            if not self.relevant(name, steps):
                continue
            for step in steps:
                for rid in step_reactions(self.s, self.canon, step, ACTIVE):
                    if rid in self.drawn_ids():
                        continue
                    a_base, b_base, comp, _ = step
                    main = self.joined(self.nodes_for(a_base, comp), self.nodes_for(b_base, comp))
                    note = f'phase 7 pathways: {name}, bypass'
                    ok = self.place_parallel(rid, main, step, note) if main else self.place_step(rid, step, note)
                    if ok:
                        added += 1
                    else:
                        self.notes.append(f'{rid} ({a_base} -> {b_base}): no room for the bypass')
        # what the GEM has but could not be drawn
        for name, steps in self.pathways():
            if not self.relevant(name, steps):
                continue
            for step in steps:
                rids = step_reactions(self.s, self.canon, step)
                if rids and not set(rids) & self.drawn_ids() and not step[3]:
                    self.notes.append(f'{name}: {step[0]} -> {step[1]} ({", ".join(r.replace("_" + self.cell, "") for r in rids[:3])}) not drawn')
                elif not rids and name in self.forced and not step[3]:
                    # R14, R24: a step the GEM does not have is a real gap, shown as one
                    self.notes.append(f'{name}: {step[0]} -> {step[1]} is not in the GEM; the route is incomplete there')
        return added

    def pathway_labels(self, taken):
        """The name of each pathway with three or more steps drawn, beside them."""
        out = []
        drawn = self.drawn_ids()
        for name, steps in PATHWAYS:
            rids = [r for st in steps for r in step_reactions(self.s, self.canon, st) if r in drawn]
            rx = [self.cm.R[self.s.mid(r)] for r in dict.fromkeys(rids) if self.s.mid(r) in self.cm.R]
            if len(rx) < 3:
                continue
            fake = [(r['localId'], None, None) for r in sorted(rx, key=lambda r: (r['y'], r['x']))]
            spot = self._label_spot(fake, name, taken, 30, (140, 220, 320))
            if spot:
                out.append((name, spot[0], spot[1]))
                taken.append(spot[2])
        return out


def add_metabolic_at(b, rid_local, a, z, centre, side_nodes, note):
    """Builder.add_metabolic with the reaction at `centre` instead of midway (a bypass beside the main reaction)."""
    s, m = b.s, b.m
    gem = s.rxns[rid_local]
    coef = gem['metabolites'].get(a['localId'], -1)
    sub, prod = (a, z) if coef < 0 else (z, a)
    r = b.reaction(rid_local, centre[0], centre[1], (prod['x'] - sub['x'], prod['y'] - sub['y']))
    b.axis_segment(r)
    given = {sub['localId']: sub['id'], prod['localId']: prod['id']}
    given.update(side_nodes or {})
    new_nodes, taken = [], []
    for k, c in gem['metabolites'].items():
        if k in given:
            b.branch(r, m.N[given[k]], c, 'primary' if m.N[given[k]]['kind'] == 'metabolite' else 'currency')
            continue
        sign = -1 if c < 0 else 1
        cur = s.is_currency(k)
        best = b.spot(r, sign, s.comp(k), taken, across=None if cur else (90, 120, 150), along=None if cur else (60, 90))
        if best is None:
            return ['no room for ' + k]
        taken.append(best[1])
        if cur:
            n = b.node(k, 'currency', best[1][0], best[1][1], r['id'])
        else:
            twins = m.primary_nodes(b.cell, s.mid(k))
            n = b.node(k, 'metabolite', best[1][0], best[1][1], r['id'], duplicate_of=twins[0]['id'] if twins else None)
        new_nodes.append(n)
        b.branch(r, n, c, 'currency' if cur else 'primary')
    problems = b.check(rid_local, new_nodes)
    if problems:
        return problems
    b.record(rid_local, note, 'metabolic')
    return []


def main(src, out, log_path, *cells):
    from build_phase7_spines import add_labels  # noqa: F401  (labels are appended, not replaced)
    cm = CoxMap(load(src))
    entries, report, notes = [], [], []
    for cell in cells:
        s = species(cell)
        log = []
        f = PathwayFiller(cm, s, log)
        n = f.fill()
        texts = cm.M.setdefault('texts', [])
        # a label of the spines that the new reactions now run through is dropped
        kept = []
        for t in texts:
            if t['id'].startswith(f'pathway-{cell}-') and not f.box_clear(label_box(t), 8):
                notes.append(f'{cell}: label "{t["text"]}" dropped, the new reactions run through it')
                continue
            kept.append(t)
        texts[:] = kept
        taken = [label_box(t) for t in texts if t['id'].startswith(f'pathway-{cell}-')]
        labels = f.pathway_labels(taken)
        texts[:] = [t for t in texts if not t['id'].startswith(f'pathname-{cell}-')]
        for i, (text, x, y) in enumerate(labels, 1):
            texts.append({'id': f'pathname-{cell}-{i}', 'text': text, 'x': round(x, 1), 'y': round(y, 1),
                          'fontSize': LABEL_SIZE, 'fontFamily': 'sans'})
        entries += log
        notes += [f'{cell}: {x}' for x in f.notes]
        report.append(f'{cell}: {n} reactions added, {len(labels)} pathway labels, {len(f.notes)} notes')
        print(report[-1], flush=True)
    save(cm.doc, out)
    write_log(log_path, 'Fase 7: vías conceptuales completadas',
              'Se completaron las vías de cada especie con los pasos que su GEM tiene, antes que los desvíos (R18, R19, R24); '
              'los desvíos y rutas alternativas activos se agregaron después, como derivaciones de sus pasos (R20).',
              entries, closing='\n'.join('- ' + r for r in report + notes), append=Path(log_path).exists())


def label_box(t):
    w = 0.58 * t.get('fontSize', LABEL_SIZE) * len(t['text'])
    h = t.get('fontSize', LABEL_SIZE)
    return (t['x'], t['y'] - 0.8 * h, t['x'] + w, t['y'] + 0.2 * h)


if __name__ == '__main__':
    main(*sys.argv[1:])
