"""Phase 7: room in Bt's envelope for its extracellular fructan degradation.

Bt hydrolyses kestose, kestotetraose, kestopentaose and inulin outside the
cell (KESTOASEe, KESTOTTRASEe, KESTOPTASEe, INULINASEe) and has no transporter
for them. Their exchanges sit on the left side of the cell, where the envelope
band (between the outer membrane and the cytoplasmic one) was 340 wide: no
room for reactions. This is a local change (R34):

1. the left side of Bt's outer membrane moves WIDEN further out; the exchanges
   on that side, their extracellular nodes and their medium nodes move with
   it, and the transporters' branches to those nodes are reshaped;
2. the five exchanges with no transporter on that side are put next to each
   other in pathway order (kestose, kestotetraose, kestopentaose, inulin, kdo);
3. in the room this leaves, the degradation is drawn: the three kestose
   hydrolases converge on one fructose node (R15), each with its glucose
   beside it, and inulinase turns inulin into kestopentaose beside it. The
   fructose and glucose drawn here are traced copies of the nodes their
   transporters use on the other side of the cell.

Every insertion goes through coxmap's checks.

Usage: python build_phase7_bt_band.py IN OUT LOG
"""
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from build_phase5_transport import species  # noqa: E402
from coxmap import Builder, CoxMap, load, pt, save, shape_branch, write_log  # noqa: E402

CELL = 'Bt'
WIDEN = 400
ORDER = ['kesto', 'kestottr', 'kestopt', 'inulin', 'kdo']   # top to bottom, in these five slots


def shift_point(p, dx, dy):
    return pt(p['x'] + dx, p['y'] + dy)


def move_exchange_group(cm, r, e_node, dx, dy):
    """An exchange reaction, its extracellular node and its medium node, moved together."""
    r['x'] += dx
    r['y'] += dy
    for k in ('a1', 'a2'):
        r['axis'][k] = shift_point(r['axis'][k], dx, dy)
    for seg in cm.segs[r['id']]:
        for k in ('from', 'to', 'b1', 'b2'):
            if k in seg:
                seg[k] = shift_point(seg[k], dx, dy)
        node = cm.N.get(seg.get('nodeId'))
        if node and node['kind'] == 'medium' and not node.get('_moved'):
            node['x'] += dx
            node['y'] += dy
            node['_moved'] = True
    if not e_node.get('_moved'):
        e_node['x'] += dx
        e_node['y'] += dy
        e_node['_moved'] = True


def follow_node(cm, node):
    """Re-anchor every other reaction's branch on a node that moved, and reshape it."""
    for seg in cm.M['segments']:
        if seg.get('nodeId') != node['id']:
            continue
        end = 'from' if seg['side'] == 'substrate' else 'to'
        seg[end] = pt(node['x'], node['y'])
        shape_branch(seg, cm.R[seg['reactionId']])


def widen_left(cm, s):
    cell = cm.cells[CELL]
    rect = cell['rect']
    old_x = rect['x']
    rect['x'] -= WIDEN
    rect['w'] += WIDEN
    for comp in cell['compartments']:
        if comp['id'] == 'e':
            comp['rect']['x'] -= WIDEN
            comp['rect']['w'] += WIDEN
    moved = []
    for r in cm.M['reactions']:
        if r['cellId'] == CELL and r['kind'] == 'transfer' and abs(r['x'] - old_x) < 1:
            e_node = next(cm.N[g['nodeId']] for g in cm.segs[r['id']]
                          if g.get('nodeId') and cm.N[g['nodeId']]['kind'] == 'metabolite')
            move_exchange_group(cm, r, e_node, -WIDEN, 0)
            moved.append((r, e_node))
    for r, e_node in moved:
        follow_node(cm, e_node)
    return moved


def reorder(cm, moved):
    """Put the exchanges of ORDER in the slots they already hold, in that order."""
    by_base = {}
    for r, e_node in moved:
        base = e_node['localId'].split('[')[0]
        if base in ORDER:
            by_base[base] = (r, e_node)
    slots = sorted(r['y'] for r, _ in by_base.values())
    for base, y in zip(ORDER, slots):
        r, e_node = by_base[base]
        dy = y - r['y']
        if abs(dy) < 1:
            continue
        for node in (e_node,):
            node.pop('_moved', None)
        for seg in cm.segs[r['id']]:
            node = cm.N.get(seg.get('nodeId'))
            if node:
                node.pop('_moved', None)
        move_exchange_group(cm, r, e_node, 0, dy)
        follow_node(cm, e_node)
    return {base: by_base[base][1] for base in by_base}


def main(src, out, log_path):
    cm = CoxMap(load(src))
    s = species(CELL)
    moved = widen_left(cm, s)
    nodes = reorder(cm, moved)
    for n in cm.M['nodes']:
        n.pop('_moved', None)
    cm.reindex()
    log, notes = [], []
    b = Builder(cm, s, log)
    x0 = nodes['kesto']['x']
    # the fructose the hydrolases converge on, beside the kestotetraose row
    fru_twin = next(n for n in cm.primary_nodes(CELL, s.mid('fru[e_Bt]')) if not n.get('duplicateOf'))
    glc_twin = next(n for n in cm.primary_nodes(CELL, s.mid('glc-D[e_Bt]')) if not n.get('duplicateOf'))
    # the two diagonals first, the straight one last: its glucose finds room between them
    plan = [
        ('KESTOASEe_Bt', 'kesto'),
        ('KESTOPTASEe_Bt', 'kestopt'),
        ('KESTOTTRASEe_Bt', 'kestottr'),
    ]
    # one row per kestose: kestose -> fructose (+ glucose beside), across the band
    for rid, base in plan:
        mark = cm.mark()
        fru = b.node('fru[e_Bt]', 'metabolite', x0 + 400, nodes[base]['y'], s.mid(rid), duplicate_of=fru_twin['id'])
        problems = b.add_metabolic(rid, nodes[base]['id'], fru['id'], {}, note='phase 7: Bt envelope, fructans')
        if not problems:
            problems = b.check(rid, [cm.N[fru['id']]])
        if problems:
            cm.rollback(mark)
            notes.append(f'{rid}: not drawn ({"; ".join(problems[:2])})')
    # inulin -> kestopentaose, down the column of exchanges
    mark = cm.mark()
    problems = b.add_metabolic('INULINASEe_Bt', nodes['inulin']['id'], nodes['kestopt']['id'], {},
                               note='phase 7: Bt envelope, inulin')
    if problems:
        cm.rollback(mark)
        notes.append(f'INULINASEe_Bt: not drawn ({"; ".join(problems[:2])})')
    # a smaller label: the band is narrower than the cell's labels are long
    texts = cm.M.setdefault('texts', [])
    texts[:] = [t for t in texts if t['id'] != f'pathway-{CELL}-fructans']
    if log:
        texts.append({'id': f'pathway-{CELL}-fructans', 'text': 'Fructan hydrolysis', 'x': x0 - 80,
                      'y': nodes['kesto']['y'] - 95, 'fontSize': 64, 'fontFamily': 'sans'})
    save(cm.doc, out)
    drawn = [e['reaction'] for e in log]
    report = f'{CELL}: left envelope band widened by {WIDEN}; {len(moved)} exchanges moved; drew {", ".join(drawn) or "nothing"}'
    print(report)
    for x in notes:
        print('  ', x)
    write_log(log_path, 'Fase 7: espacio en la envoltura de Bt',
              'Se ensanchó hacia afuera la banda extracelular izquierda de Bt y se dibujó la degradación de '
              'fructanos que ocurre fuera de la célula.', log,
              closing='\n'.join('- ' + x for x in [report] + notes), append=Path(log_path).exists())


if __name__ == '__main__':
    main(*sys.argv[1:4])
