"""Phase 7: a species' internal network redrawn substrate first.

The phase 6 network grew from every transporter alike and kept to the
membranes; most of what it drew hung from the products' exits. Here a species'
internal reactions are taken out and its central metabolism is drawn again
from a plan, the way the yeast central-carbon map in Escher reads:

* each substrate's pathway starts at its transporter, on the inside of the
  membrane, and runs inward as a straight spine;
* a product is drawn where it is made. Its exit transporter keeps its own
  node; the product is a duplicate of that node (decision (a)), unless the
  pathway ends next to the exit, where it joins it (decision (b));
* each pathway gets a text label.

Each reaction is added with coxmap's Builder: compared with the GEM (R51),
checked against crossings and crowding, and rolled back if it fails.

Usage: python build_phase7_substrate_first.py SPECIES IN OUT LOG
"""
import sys
from collections import defaultdict
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from build_phase5_transport import species  # noqa: E402
from coxmap import Builder, CoxMap, load, save, write_log  # noqa: E402

S = 280          # one step along a spine, about 1.45 L_ref
LABEL_SIZE = 96  # pathway labels, in map units

# Node keys name a metabolite (its base) and where it is drawn. 'T:' reuses the
# node of the transporter that brings the metabolite in or takes it out.
PLANS = {
    'St': {
        'nodes': {
            # glycolysis, left to right from the glucose PTS
            'g6p': 'T:g6p', 'f6p': ('f6p', 11513, 12800), 'fdp': ('fdp', 11793, 12800),
            'g3p': ('g3p', 12073, 12800), 'dhap': ('dhap', 12073, 12520), '13dpg': ('13dpg', 12353, 12800),
            '3pg': ('3pg', 12633, 12800), '2pg': ('2pg', 12913, 12800), 'pep': ('pep', 13193, 12800),
            'pyr': ('pyr', 13473, 12800),
            # mannose PTS
            'man6p': 'T:man6p', 'f6p_m': ('f6p', 11513, 12266),
            # Leloir, from the galactose permease
            'gal': 'T:gal', 'agal': ('a_gal_D', 11513, 13346), 'gal1p': ('gal1p', 11793, 13346),
            'udpgal': ('udpgal', 12073, 13346), 'udpg': ('udpg', 12353, 13346), 'g1p': ('g1p', 12633, 13346),
            'g6p_l': ('g6p', 12913, 13346),
            # fermentation products round pyruvate
            'lac': ('lac-L', 13473, 12520), 'ala': ('ala-L', 13473, 13080),
            'accoa': ('accoa', 13753, 12800), 'actp': ('actp', 14033, 12800), 'ac': ('ac', 14313, 12800),
            # dihydroxyacetone PTS
            'dha': 'T:dha', 'dhap_d': ('dhap', 11120, 10863),
            # citrate to glutamate, down from the citrate carrier
            'cit': 'T:cit', 'icit': ('icit', 14230, 10863), 'akg': ('akg', 14230, 11143),
            'glu': ('glu-L', 14230, 11423),
            # threonine to ethanol, down from the threonine carrier
            'thr': 'T:thr-L', 'acald': ('acald', 13435, 10863), 'etoh': ('etoh', 13435, 11143),
        },
        # (reaction, from node, to node, other metabolites drawn on given nodes)
        'reactions': [
            ('PGI', 'g6p', 'f6p'), ('PFK', 'f6p', 'fdp'), ('FBA', 'fdp', 'g3p', {'dhap': 'dhap'}),
            ('TPI', 'dhap', 'g3p'), ('GAPD', 'g3p', '13dpg'), ('PGK', '13dpg', '3pg'), ('PGM', '3pg', '2pg'),
            ('ENO', '2pg', 'pep'), ('PYK', 'pep', 'pyr'),
            ('MAN6PI', 'man6p', 'f6p_m'),
            ('GALM', 'gal', 'agal'), ('GALK2', 'agal', 'gal1p'), ('GALT', 'gal1p', 'udpgal'),
            ('UDPG4E', 'udpgal', 'udpg'), ('GALU', 'udpg', 'g1p'), ('PGMT', 'g1p', 'g6p_l'),
            ('LDH_L', 'pyr', 'lac'), ('ALAD_R', 'pyr', 'ala'), ('PFL', 'pyr', 'accoa'), ('PTAr', 'accoa', 'actp'),
            ('ACKr', 'actp', 'ac'),
            ('DHAPT', 'dha', 'dhap_d'),
            ('ACONT', 'cit', 'icit'), ('ICDHx', 'icit', 'akg'), ('ALATA_L', 'akg', 'glu'),
            ('THRAr', 'thr', 'acald'), ('ALCD2x', 'acald', 'etoh'),
        ],
        'labels': [
            ('Glycolysis', 12300, 12420),
            ('Leloir pathway', 11500, 13700),
            ('Mixed-acid fermentation', 13700, 12450),
            ('Citrate to glutamate', 14400, 11000),
            ('Threonine to ethanol', 12150, 11060),
        ],
    },
}


def strip_internal(cm, cell, keep=()):
    """Remove a cell's metabolic reactions (but those in `keep`), their segments,
    and the nodes only they used."""
    M = cm.M
    gone = {r['id'] for r in M['reactions'] if r['cellId'] == cell and r['kind'] == 'metabolic' and r['id'] not in keep}
    M['reactions'] = [r for r in M['reactions'] if r['id'] not in gone]
    M['segments'] = [s for s in M['segments'] if s['reactionId'] not in gone]
    used = {s['nodeId'] for s in M['segments'] if s.get('nodeId')}
    dropped = {n['id'] for n in M['nodes'] if n.get('cellId') == cell and n['id'] not in used}
    M['nodes'] = [n for n in M['nodes'] if n['id'] not in dropped]
    # a duplicate whose canonical node went points at the first twin left, or stands alone
    first = {}
    for n in M['nodes']:
        if n['kind'] == 'metabolite' and not n.get('duplicateOf'):
            first.setdefault(n['mergedId'], n['id'])
    for n in M['nodes']:
        if n.get('duplicateOf') in dropped:
            canon = first.get(n['mergedId'])
            if canon and canon != n['id']:
                n['duplicateOf'] = canon
            else:
                del n['duplicateOf']
                first.setdefault(n['mergedId'], n['id'])
    cm.reindex()
    return len(gone), len(dropped)


def local_of(sp, base):
    """The cytosolic metabolite of the species' GEM with this base."""
    for k in sp.mets:
        if sp.comp(k) == 'c' and sp.base(k) == base.replace('-', '_'):
            return k
    raise KeyError(base)


def transporter_node(cm, sp, local):
    """The node a transporter of the cell draws for this cytosolic metabolite."""
    mid = sp.mid(local)
    for s in cm.M['segments']:
        r = cm.R.get(s['reactionId'])
        if not r or r['cellId'] != sp.sp or r['kind'] != 'transport' or not s.get('nodeId'):
            continue
        n = cm.N[s['nodeId']]
        if n['mergedId'] == mid and n['kind'] == 'metabolite':
            return n['id']
    raise KeyError(f'no transporter draws {local}')


def build(cm, sp, plan, log):
    b = Builder(cm, sp, log)
    nodes = {}          # plan key -> node id
    results = []
    for entry in plan['reactions']:
        rid_base, ka, kb = entry[:3]
        sides = entry[3] if len(entry) > 3 else {}
        rid = f'{rid_base}_{sp.sp}'
        if rid not in sp.rxns:
            results.append((rid_base, ['not in the GEM']))
            continue
        mark = cm.mark()
        created = []

        def node_for(key):
            if key in nodes:
                return nodes[key]
            spec = plan['nodes'][key]
            if isinstance(spec, str):
                nid = transporter_node(cm, sp, local_of(sp, spec[2:]))
            else:
                base, x, y = spec
                local = local_of(sp, base)
                twins = [n for n in cm.primary_nodes(sp.sp, sp.mid(local)) if not n.get('duplicateOf')]
                n = b.node(local, 'metabolite', x, y, sp.mid(rid), duplicate_of=twins[0]['id'] if twins else None)
                created.append(n)
                nid = n['id']
            nodes[key] = nid
            return nid

        a, z = node_for(ka), node_for(kb)
        side_nodes = {cm.N[node_for(k)]['localId']: node_for(k) for k in sides.values()}
        # draw the reaction the way its GEM writes it: substrate side first
        coef = sp.rxns[rid]['metabolites'].get(cm.N[a]['localId'], -1)
        if coef > 0:
            a, z = z, a
        problems = b.add_metabolic(rid, a, z, side_nodes, note='phase 7: ' + rid_base)
        if not problems and created:
            problems = b.check(rid, created)
        if problems:
            cm.rollback(mark)
            for k in [k for k, v in nodes.items() if v not in cm.N]:
                del nodes[k]
            log[:] = [e for e in log if e['reaction'] != rid]
        results.append((rid_base, problems))
    return results


def add_labels(cm, cell, labels):
    texts = cm.M.setdefault('texts', [])
    texts[:] = [t for t in texts if not t['id'].startswith(f'pathway-{cell}-')]
    for text, x, y in labels:
        slug = ''.join(ch if ch.isalnum() else '-' for ch in text.lower()).strip('-')
        texts.append({'id': f'pathway-{cell}-{slug}', 'text': text, 'x': x, 'y': y,
                      'fontSize': LABEL_SIZE, 'fontFamily': 'sans'})


def main(cell, src, out, log_path):
    cm = CoxMap(load(src))
    sp = species(cell)
    removed, dropped = strip_internal(cm, cell)
    log = []
    results = build(cm, sp, PLANS[cell], log)
    add_labels(cm, cell, PLANS[cell]['labels'])
    save(cm.doc, out)
    ok = [r for r, p in results if not p]
    bad = [(r, p) for r, p in results if p]
    print(f'{cell}: removed {removed} internal reactions and {dropped} nodes; drew {len(ok)}, failed {len(bad)}')
    for r, p in bad:
        print('  ', r, '; '.join(p[:3]))
    write_log(log_path, f'Fase 7: {cell} redibujado desde los sustratos',
              f'Se quitaron las {removed} reacciones internas de {cell} y se dibujó su metabolismo central '
              'desde los transportadores de sus sustratos. Cada reacción se comparó con el GEM y con lo ya dibujado.',
              log, closing='\n'.join(f'- No se dibujó {r}: {"; ".join(p)}' for r, p in bad))
    return bad


if __name__ == '__main__':
    main(*sys.argv[1:5])
