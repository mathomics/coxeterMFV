"""Phase 5: the transporter layer of one species.

Every exchange of the species gets the GEM reaction that carries its
metabolite between the extracellular band and the cytosol, so each
cross-feeding is drawn as an unbroken chain: medium -> e -> (p ->) c.

* Gram-positive cells: one transporter on the cytoplasmic membrane.
* Ec and Kp: the outer-membrane step (e <-> p, usually a porin) and the
  inner-membrane step (p <-> c), with a periplasmic node between them.

Among the GEM reactions that do a step, the one with the largest flux in the
scenario is drawn (R04: one reaction per step). Each transporter is placed on
the membrane in line with its exchange; the cytosolic node of the transported
metabolite sits 1 L_ref inside. Metabolites that cross with the same base get
a lane each (symport/antiport, as Coxeter draws them); everything else is a
cofactor on its own side, PTS phosphate donors included. A transporter whose
drawing would cross or crowd anything is rolled back and listed as pending.

Usage: python build_phase5_transport.py MAP OUT SPECIES LOG_MD [MAX_PER_BATCH]
"""
import csv
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from coxmap import BAND_P, L_REF, Builder, CoxMap, Species, load, save, write_log  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]


def species(sp):
    model = json.load(open(ROOT / f'models/published/steadycom-gut-nine/model-{sp}.json', encoding='utf-8'))
    flux = {r['local_id']: float(r['flux']) for r in csv.DictReader(open(ROOT / 'docs/gut9-map/inputs/fluxes.csv', encoding='utf-8'))
            if r['species'] == sp}
    return Species(sp, model, flux)


def is_exchange(rid):
    return rid.startswith(('EX_', 'IEX_', 'R_Ex_', 'R_EX_')) or 'biomass' in rid.lower() or rid.startswith(('DM_', 'SK_', 'sink_'))


def steps(s, local_e, comp_from, comp_to):
    """GEM reactions moving metabolite `local_e` (in comp_from) into comp_to,
    best first: same base on the other side first, then by |flux|."""
    base = s.base(local_e)
    currency = s.is_currency(local_e)
    out = []
    for rid in s.by_met[local_e]:
        if is_exchange(rid):
            continue
        r = s.rxns[rid]
        others = [k for k in r['metabolites'] if s.comp(k) == comp_to]
        if not others:
            continue
        same = any(s.base(k) == base for k in others)
        crossing = {s.base(k) for k in r['metabolites'] if s.comp(k) == comp_from} & {s.base(k) for k in others}
        # a cofactor's own exchange needs a transporter of its own, not a symporter
        # that happens to carry it beside another metabolite
        if currency and crossing - {base}:
            continue
        out.append((not same, -abs(s.flux.get(rid, 0.0)), rid))
    out.sort()
    active = [rid for _, f, rid in out if f < -1e-9]
    return active[:1] or ([out[0][2]] if out else [])


def main_inside(s, rid, local_out, comp_in):
    """The metabolite a transporter delivers on the inner side: the same base if
    it crosses, else the non-cofactor with the closest number of carbons."""
    r = s.rxns[rid]
    inner = [k for k in r['metabolites'] if s.comp(k) == comp_in]
    same = [k for k in inner if s.base(k) == s.base(local_out)]
    if same:
        return same[0]
    cand = [k for k in inner if not s.is_currency(k)] or inner
    c0 = s.carbons(local_out)
    return min(cand, key=lambda k: (abs(s.carbons(k) - c0), k))


def edge_of(r):
    n = r['normal']
    return 'top' if n['y'] < 0 else 'bottom' if n['y'] > 0 else 'left' if n['x'] < 0 else 'right'


def inward(edge, d):
    return {'top': (0, d), 'bottom': (0, -d), 'left': (d, 0), 'right': (-d, 0)}[edge]


def main(map_path, out_path, sp, log_path, max_batch=40):
    max_batch = int(max_batch)
    cm = CoxMap(load(map_path))
    s = species(sp)
    gram_neg = 'p' in cm.comp[sp]
    log, pending = [], []
    used = {seg['nodeId'] for seg in cm.M['segments'] if seg.get('nodeId') and cm.R[seg['reactionId']]['kind'] != 'transfer'}
    exchanges = [r for r in cm.M['reactions'] if r['cellId'] == sp and r['kind'] == 'transfer']
    # perimeter order, so a batch is a stretch of membrane
    exchanges.sort(key=lambda r: (edge_of(r), r['x'] if edge_of(r) in ('top', 'bottom') else r['y']))
    done = 0
    for ex in exchanges:
        e_node = next((cm.N[g['nodeId']] for g in cm.segs[ex['id']] if g.get('nodeId') and cm.N[g['nodeId']]['kind'] != 'medium'), None)
        if e_node is None or e_node['id'] in used:
            continue
        if done >= max_batch:
            break
        local_e = e_node['localId']
        edge = edge_of(ex)
        along = ex['x'] if edge in ('top', 'bottom') else ex['y']
        # keep clear of the cytosol's corners, so the inner node is not on the side membrane
        c = cm.comp[sp]['c']
        lo, hi = (c['x'], c['x'] + c['w']) if edge in ('top', 'bottom') else (c['y'], c['y'] + c['h'])
        along = min(max(along, lo + 80), hi - 80)
        b = Builder(cm, s, log)
        mark, log_mark = cm.mark(), len(log)
        chain = []
        if gram_neg:
            st1 = steps(s, local_e, 'e', 'p')
            direct = steps(s, local_e, 'e', 'c')
            if st1:
                local_p = main_inside(s, st1[0], local_e, 'p')
                st2 = steps(s, local_p, 'p', 'c')
                if st2:
                    chain = [(st1[0], 'p', 'e', local_e, local_p), (st2[0], 'c', 'p', local_p, None)]
            if not chain and direct:
                chain = [(direct[0], 'c', 'e', local_e, None)]
        else:
            st = steps(s, local_e, 'e', 'c')
            if st:
                chain = [(st[0], 'c', 'e', local_e, None)]
        if not chain:
            pending.append((ex['localId'], 'the GEM has no reaction carrying it into the cell'))
            continue
        if any(s.mid(rid) in cm.R for rid, *_ in chain):
            pending.append((ex['localId'], f'{chain[0][0]} is already drawn elsewhere'))
            continue
        # if it does not fit, slide it along the membrane, first away from the nearer corner
        toward_centre = 1 if along < (lo + hi) / 2 else -1
        for shift in (0, 90 * toward_centre, 180 * toward_centre, -90 * toward_centre):
            problems = place(cm, s, b, chain, e_node, edge, min(max(along + shift, lo + 80), hi - 80), ex, gram_neg)
            if not problems:
                break
            cm.rollback(mark)
            del log[log_mark:]
        if problems:
            pending.append((ex['localId'], '; '.join(problems[:2])))
            continue
        done += len(chain)
    finish(cm, out_path, log_path, log, pending, sp)
    return len(log)


def place(cm, s, b, chain, e_node, edge, along, ex, gram_neg):
    prev = e_node
    problems = []
    for rid, comp_in, comp_out, local_out, local_in in chain:
        local_in = local_in or main_inside(s, rid, local_out, comp_in)
        # the inner node: in the middle of the periplasm, or 1 L_ref into the cytosol
        mx, my = b.membrane_point(comp_in, edge, along)
        depth = BAND_P / 2 if comp_in == 'p' else L_REF
        dx, dy = inward(edge, depth)
        twins = [n for n in cm.primary_nodes(s.sp, s.mid(local_in)) if n['compartment'] == comp_in]
        near = [n for n in twins if abs(n['x'] - (mx + dx)) + abs(n['y'] - (my + dy)) < 2.5 * L_REF]
        if near:
            inner = near[0]
        elif twins:
            problems = [f'{local_in} is already drawn away from this membrane']
            break
        else:
            inner = b.node(local_in, 'metabolite', mx + dx, my + dy, s.mid(rid))
        res = b.add_transport(rid, comp_in, comp_out, edge, along, {local_out: prev['id']}, {local_in: inner['id']},
                              note=f'{ex["localId"]}: {"outer" if comp_in == "p" else "inner" if gram_neg else "cytoplasmic"} membrane')
        if res is None:
            problems = ['no room for its cofactors']
            break
        if res:
            problems = res
            break
        prev = inner
    return problems


def finish(cm, out_path, log_path, log, pending, sp):
    save(cm.doc, out_path)
    write_log(log_path, f'Fase 5 — capa de transportadores', '', log,
              closing=('Pendientes: ' + '; '.join(f'{a} ({b})' for a, b in pending)) if pending else 'Sin pendientes.',
              append=Path(log_path).exists())
    print(f'{sp}: {len(log)} transport reactions added; {len(pending)} pending')
    for a, b_ in pending:
        print('   pending', a, '-', b_)


if __name__ == '__main__':
    main(*sys.argv[1:6])
