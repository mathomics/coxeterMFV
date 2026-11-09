"""Grow one species in batches, committing each batch that passes every check.

Each batch adds up to BATCH internal reactions with build_phase6_grow.Grower
(each one already checked against the GEM, crossings and clearance), then the
whole map is validated (validate_map.py) and audited (audit.py). Only a batch
with no error, crossing or crowded node becomes the community's map and a
commit; otherwise the driver stops and leaves the batch out.

Usage: python phase6_driver.py SPECIES TARGET [BATCH]
"""
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(HERE))
import audit  # noqa: E402
import validate_map  # noqa: E402
from build_phase5_transport import species  # noqa: E402
from build_phase6_grow import Grower  # noqa: E402
from coxmap import CoxMap, load, save, write_log  # noqa: E402

MAP = ROOT / 'models/published/steadycom-gut-nine/map.json'
LOG = ROOT / 'docs/gut9-map/FASE6_METABOLISMO.md'
SPECIES_NAME = {'St': 'S. thermophilus', 'Ef': 'E. faecalis', 'Bt': 'B. thetaiotaomicron', 'Fp': 'F. prausnitzii',
                'Ec': 'E. coli', 'Kp': 'K. pneumoniae', 'Er': 'E. rectale', 'Lc': 'L. casei', 'Ba': 'B. adolescentis'}


def git(*args):
    return subprocess.run(['git', '-C', str(ROOT), *args], capture_output=True, text=True)


def batch(sp, n):
    cm = CoxMap(load(MAP))
    log = []
    g = Grower(cm, species(sp), log)
    added = g.grow(n)
    return cm, log, added


def main(sp, target, size=10):
    target, size = int(target), int(size)
    total, k = 0, 0
    while total < target:
        cm, log, added = batch(sp, min(size, target - total))
        if not added:
            print(f'{sp}: no more reactions can be added cleanly')
            break
        tmp = Path(tempfile.gettempdir()) / f'gut9_{sp}_batch.json'
        save(cm.doc, tmp)
        errors = validate_map.main(str(tmp))
        bad = audit.main(str(tmp))
        if errors or bad:
            print(f'{sp}: batch rejected ({errors} validation errors, {bad} crossings/crowded nodes); stopping')
            break
        save(cm.doc, MAP)
        write_log(LOG, 'Fase 6 — metabolismo interno',
                  'Registro de `tools/gut9-map/phase6_driver.py`: cada tanda pasó la verificación R51 por reacción, '
                  'el validador y la auditoría de cruces del mapa completo antes de su commit.',
                  log, append=LOG.exists())
        k += 1
        total += added
        lines = [f'Grow {sp}\'s internal network in the gut9 map ({added} reactions)', '',
                 f'{SPECIES_NAME.get(sp, sp)}: active reactions of the scenario, grown from metabolites already drawn, '
                 'each checked against the GEM and against crossings before the next:', '']
        lines += [f'- {e["reaction"]} ({e["name"].strip()[:60]}), flux {e["flux"]:.3g}' for e in log]
        lines += ['', 'Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>']
        msg = Path(tempfile.gettempdir()) / 'gut9_commit.txt'
        msg.write_text('\n'.join(lines) + '\n', encoding='utf-8')
        git('add', str(MAP), str(LOG), str(HERE))
        r = git('-c', 'core.safecrlf=false', 'commit', '-q', '-F', str(msg))
        print(f'{sp} batch {k}: +{added} (total {total}) {"committed" if r.returncode == 0 else "COMMIT FAILED " + r.stderr}')
    return total


if __name__ == '__main__':
    main(*sys.argv[1:4])
