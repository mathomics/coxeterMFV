"""Make a built map the nine-species community's default map.

Copies MAP to models/published/steadycom-gut-nine/map.json and replaces the
copy embedded in coxeter_mfv.html, so the single-file build shows the same
map (tools/build-standalone.mjs would do the same where Node is available).
With --section FILE, also inserts that Markdown section into the log, before
its "## Aprendizajes".

Usage: python publish.py MAP [--section FILE] [--snapshot NAME]
"""
import json
import shutil
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MAP = ROOT / 'models/published/steadycom-gut-nine/map.json'
STANDALONE = ROOT / 'coxeter_mfv.html'
LOG = ROOT / 'docs/gut9-map/GUT9_MAPA_BITACORA.md'
KEY = '"models/published/steadycom-gut-nine/map.json":'


def embed():
    s = open(STANDALONE, encoding='utf-8', newline='').read()   # it mixes line endings: keep them
    assert s.count(KEY) == 1, 'the standalone does not embed the gut9 map'
    start = s.index(KEY) + len(KEY)
    _, end = json.JSONDecoder().raw_decode(s, start)
    doc = json.loads(MAP.read_text(encoding='utf-8'))
    new = json.dumps(doc, ensure_ascii=False, separators=(',', ':')).replace('<', '\\u003c')
    with open(STANDALONE, 'w', encoding='utf-8', newline='') as f:
        f.write(s[:start] + new + s[end:])
    return len(doc['map']['reactions'])


def section(path):
    text = Path(path).read_text(encoding='utf-8').rstrip() + '\n\n'
    log = LOG.read_text(encoding='utf-8')
    at = log.index('## Aprendizajes')
    LOG.write_text(log[:at] + text + log[at:], encoding='utf-8')


def main(args):
    src = args[0]
    shutil.copyfile(src, MAP)
    n = embed()
    if '--section' in args:
        section(args[args.index('--section') + 1])
    if '--snapshot' in args:
        name = args[args.index('--snapshot') + 1]
        shutil.copyfile(src, ROOT / 'docs/gut9-map/maps' / name)
    print(f'published: {n} reactions')


if __name__ == '__main__':
    main(sys.argv[1:])
