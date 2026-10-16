#!/usr/bin/env python3
"""What a real browser makes of the map: reference for the parts of
coxeter-render that have no JavaScript module to compare with.

    python3 rust/tools/fixtures-render-browser.py          (after fixtures-render.mjs)

Writes rust/fixtures/render/browser.json, from headless Firefox:

  styles   for a few render cases, the computed style of every element of the
           SVG under assets/map.css (getComputedStyle), at several zooms. The
           Rust stylesheet resolver (css.rs) has to compute the same.
  xml      what XMLSerializer writes once the renderer's markup has been parsed
           as HTML, which is how the app's exportableSVG() makes its file.
  cssText  the rules of map.css as the browser's CSSOM writes them, and
           exportCss, the text exportableSVG() puts in the file's <style>: the
           rules of the page's two stylesheets that mention `.emc-`.
  exportable  what exportableSVG() returns for cases drawn with forExport.

Needs Firefox and the Python `websockets` package; drives the browser over
WebDriver BiDi. Starts its own headless instance on its own profile and stops
only that one.
"""
import asyncio
import json
import os
import shutil
import subprocess
import sys
import tempfile

import websockets

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), '..', '..'))
FIXTURES = os.path.join(ROOT, 'rust', 'fixtures', 'render')
PORT = int(os.environ.get('COXETER_BIDI_PORT', '9417'))
FIREFOX = os.environ.get('FIREFOX', '/usr/lib/firefox/firefox')

# The properties css.rs computes, in its order.
PROPS = [
    'color', 'font-size', 'fill', 'fill-opacity', 'stroke', 'stroke-width', 'stroke-opacity',
    'stroke-dasharray', 'stroke-linecap', 'stroke-linejoin', 'opacity', 'font-weight', 'font-family',
    'font-variant-numeric', 'letter-spacing', 'display', 'visibility', 'text-transform', 'text-anchor',
    'dominant-baseline', 'paint-order', 'filter', 'transform', 'vector-effect', 'pointer-events',
    'text-decoration',
]

# (fixture case, what the app's applyCamera() would set on the map)
STYLE_CASES = [
    ('gapd-default', {}),
    ('gapd-default', {'labelScale': 2.6, 'frameLabelScale': 5.25, 'isFar': True}),
    ('gapd-multi', {'labelScale': 1.357, 'frameLabelScale': 1.5}),
    ('gapd-edit', {}),
    ('gapd-fva', {'labelScale': 1.2}),
    ('edited-dark', {'frameLabelScale': 16}),
    ('alga-far', {'frameLabelScale': 16, 'labelScale': 2.6, 'isFar': True}),
    ('alga-geometry', {'frameLabelScale': 2}),
    ('zuniga-multi-texts', {}),
    ('zuniga-ko', {'labelScale': 1.8}),
    ('yeast-transport-ko', {}),
    ('pair-crossfeed-at', {'frameLabelScale': 3}),
    ('ribbons-subsystem', {}),
]
XML_CASES = ['gapd-export', 'audit-exported', 'pair-crossfeed-exported', 'gapd-edit', 'zuniga-multi-texts', 'edited-default']

COLLECT = r'''
(svgText, opts, props) => {
  const stage = document.getElementById('stage');
  stage.innerHTML = svgText;
  const svg = stage.querySelector('svg');
  if (opts.labelScale != null) svg.style.setProperty('--label-scale', String(opts.labelScale));
  if (opts.frameLabelScale != null) svg.style.setProperty('--frame-label-scale', String(opts.frameLabelScale));
  svg.classList.toggle('is-far', Boolean(opts.isFar));
  const els = [svg, ...svg.querySelectorAll('*')];
  const unique = new Map();
  const elements = [];
  for (const el of els) {
    const cs = getComputedStyle(el);
    const key = JSON.stringify(props.map((p) => cs.getPropertyValue(p)));
    if (!unique.has(key)) unique.set(key, unique.size);
    elements.push(unique.get(key));
  }
  stage.innerHTML = '';
  return JSON.stringify({ count: els.length, styles: [...unique.keys()].map((k) => JSON.parse(k)), elements });
}
'''

EXPORT_CASES = ['gapd-export', 'audit-exported', 'pair-crossfeed-exported']

# The body of exportableSVG() in src/ui/app.js, given what renderMap returned.
EXPORTABLE = r'''
(svgText) => {
  const template = document.createElement('template');
  template.innerHTML = svgText;
  const clone = template.content.querySelector('svg');
  const css = [...document.styleSheets]
    .flatMap((sheet) => { try { return [...sheet.cssRules].map((r) => r.cssText); } catch { return []; } })
    .filter((rule) => rule.includes('.emc-'))
    .join('\n');
  const style = document.createElementNS('http://www.w3.org/2000/svg', 'style');
  style.textContent = css;
  clone.insertBefore(style, clone.firstChild);
  return new XMLSerializer().serializeToString(clone);
}
'''

SERIALIZE = r'''
(svgText) => {
  const template = document.createElement('template');
  template.innerHTML = svgText;
  return new XMLSerializer().serializeToString(template.content.querySelector('svg'));
}
'''


async def run():
    css = open(os.path.join(ROOT, 'assets', 'map.css'), encoding='utf8').read()
    cases = {c['name']: c for c in json.load(open(os.path.join(FIXTURES, 'cases.json'), encoding='utf8'))}
    svg_of = lambda name: open(os.path.join(FIXTURES, cases[name]['svg']), encoding='utf8').read()

    async with websockets.connect(f'ws://127.0.0.1:{PORT}/session', max_size=2 ** 28) as ws:
        counter = 0

        async def call(method, params):
            nonlocal counter
            counter += 1
            mid = counter
            await ws.send(json.dumps({'id': mid, 'method': method, 'params': params}))
            while True:
                msg = json.loads(await ws.recv())
                if msg.get('id') == mid:
                    if 'error' in msg:
                        raise RuntimeError(json.dumps(msg)[:2000])
                    return msg['result']

        session = await call('session.new', {'capabilities': {}})
        try:
            ctx = (await call('browsingContext.getTree', {}))['contexts'][0]['context']
            await call('browsingContext.navigate', {'context': ctx, 'url': 'about:blank', 'wait': 'complete'})

            async def fn(declaration, *args):
                result = await call('script.callFunction', {
                    'functionDeclaration': declaration,
                    'arguments': [{'type': 'string', 'value': json.dumps(a)} for a in args],
                    'target': {'context': ctx}, 'awaitPromise': True,
                })
                if result.get('type') == 'exception':
                    raise RuntimeError(json.dumps(result)[:2000])
                return result['result']['value']

            # A page with nothing but the map's stylesheet: what an exported SVG
            # opened on its own is styled by, and what css.rs resolves against.
            await fn('''(css) => {
                const style = document.createElement('style');
                style.textContent = JSON.parse(css);
                document.head.append(style);
                const stage = document.createElement('div');
                stage.id = 'stage';
                document.body.append(stage);
                return 'ok';
            }''', css)
            wrap = lambda body, n: f'(...a) => ({body})(...a.map((x) => JSON.parse(x)))'
            agent = await fn('() => navigator.userAgent')
            css_text = json.loads(await fn('() => JSON.stringify([...document.styleSheets[0].cssRules].map((r) => r.cssText))'))
            root = json.loads(await fn('''() => { const cs = getComputedStyle(document.getElementById('stage'));
                return JSON.stringify({ color: cs.color, fontSize: cs.fontSize }); }'''))

            styles = []
            for name, opts in STYLE_CASES:
                data = json.loads(await fn(wrap(COLLECT, 3), svg_of(name), opts, PROPS))
                styles.append({'case': name, 'options': opts, **data})
                print(f'  {name} {opts}: {data["count"]} elements, {len(data["styles"])} distinct styles')
            xml = []
            for name in XML_CASES:
                xml.append({'case': name, 'xml': await fn(wrap(SERIALIZE, 1), svg_of(name))})

            # The page as the app has it — app.css, then map.css — and the rules
            # exportableSVG() takes from it for the exported file's <style>.
            await call('browsingContext.navigate', {'context': ctx, 'url': 'about:blank', 'wait': 'complete'})
            app_css = open(os.path.join(ROOT, 'assets', 'app.css'), encoding='utf8').read()
            export_css = await fn('''(app, map) => {
                for (const css of [app, map]) {
                    const style = document.createElement('style');
                    style.textContent = JSON.parse(css);
                    document.head.append(style);
                }
                return [...document.styleSheets]
                    .flatMap((sheet) => { try { return [...sheet.cssRules].map((r) => r.cssText); } catch { return []; } })
                    .filter((rule) => rule.includes('.emc-'))
                    .join('\\n');
            }''', app_css, css)
            # exportableSVG() itself, for cases drawn with forExport.
            exported = []
            for name in EXPORT_CASES:
                exported.append({'case': name, 'svg': await fn(wrap(EXPORTABLE, 1), svg_of(name))})
        finally:
            await call('session.end', {})

    out = {'browser': agent, 'properties': PROPS, 'root': root, 'styles': styles, 'xml': xml, 'cssText': css_text,
           'exportCss': export_css, 'exportable': exported}
    path = os.path.join(FIXTURES, 'browser.json')
    json.dump(out, open(path, 'w', encoding='utf8'), ensure_ascii=False, separators=(',', ':'))
    print(f'wrote {path} ({os.path.getsize(path) // 1024} KB) from {agent}')


def main():
    profile = tempfile.mkdtemp(prefix='coxeter-render-ff-')
    browser = subprocess.Popen(
        [FIREFOX, '--headless', '--no-remote', f'--remote-debugging-port={PORT}', '--profile', profile, 'about:blank'],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        last = None
        for _ in range(60):
            try:
                asyncio.run(run())
                return 0
            except (OSError, websockets.exceptions.InvalidHandshake, websockets.exceptions.InvalidStatusCode) as error:
                last = error  # the browser is still starting
                import time
                time.sleep(1)
        print(f'could not reach Firefox on port {PORT}: {last}', file=sys.stderr)
        return 1
    finally:
        browser.terminate()
        try:
            browser.wait(timeout=20)
        except subprocess.TimeoutExpired:
            browser.kill()
        shutil.rmtree(profile, ignore_errors=True)


if __name__ == '__main__':
    sys.exit(main())
