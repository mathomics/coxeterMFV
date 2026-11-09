import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { saveFile, extensionOf, downloadSettingHelp, NOTICE_OFF_KEY } from '../src/ui/save-file.js';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

/* A browser with a Save As window, in which the user does what `choose` says. */
function pickerEnv(choose) {
  const log = [];
  const env = {
    log,
    async showSaveFilePicker(options) {
      log.push(['picker', options]);
      const name = choose(options);
      if (name instanceof Error) throw name;
      return {
        name,
        async createWritable() {
          log.push(['open', name]);
          return {
            async write(blob) { log.push(['write', await blob.text(), blob.type]); },
            async close() { log.push(['close']); },
            async abort() { log.push(['abort']); },
          };
        },
      };
    },
  };
  return env;
}

const abort = () => Object.assign(new Error('The user aborted a request.'), { name: 'AbortError' });

test('the Save As window offers the name and type, and saves under the name the user gave', async () => {
  const env = pickerEnv(() => 'my-map.json');
  const saved = await saveFile('yeast_map.json', 'application/json', '{"a":1}', env);
  assert.equal(saved, 'my-map.json');
  const [[, options]] = env.log;
  assert.equal(options.suggestedName, 'yeast_map.json');
  assert.deepEqual(options.types, [{ description: 'JSON file', accept: { 'application/json': ['.json'] } }]);
  assert.ok(options.id, 'the window remembers the folder last chosen');
  assert.deepEqual(env.log.slice(1), [['open', 'my-map.json'], ['write', '{"a":1}', 'application/json'], ['close']]);
});

test('the file is made only once the user has chosen where it goes', async () => {
  const env = pickerEnv(() => 'map.svg');
  const saved = await saveFile('map.svg', 'image/svg+xml', () => {
    env.log.push(['make']);
    return '<svg/>';
  }, env);
  assert.equal(saved, 'map.svg');
  assert.deepEqual(env.log.map(([step]) => step), ['picker', 'make', 'open', 'write', 'close']);
});

test('cancelling the Save As window saves nothing and makes nothing', async () => {
  const env = pickerEnv(abort);
  let made = false;
  const saved = await saveFile('map.png', 'image/png', () => { made = true; return 'x'; }, env);
  assert.equal(saved, null);
  assert.equal(made, false);
  assert.deepEqual(env.log.map(([step]) => step), ['picker']);
});

test('a file that cannot be made or written is reported, not called saved', async () => {
  const env = pickerEnv(() => 'map.png');
  await assert.rejects(
    saveFile('map.png', 'image/png', async () => { throw new Error('too large'); }, env),
    /too large/,
  );
  assert.deepEqual(env.log.map(([step]) => step), ['picker'], 'nothing was opened for writing');

  const failing = pickerEnv(() => 'map.json');
  const open = failing.showSaveFilePicker;
  failing.showSaveFilePicker = async (o) => {
    const handle = await open(o);
    return { ...handle, async createWritable() {
      return { async write() { throw new Error('disk full'); }, async close() {}, async abort() { failing.log.push(['abort']); } };
    } };
  };
  await assert.rejects(saveFile('map.json', 'application/json', '{}', failing), /disk full/);
  assert.deepEqual(failing.log.map(([step]) => step), ['picker', 'abort']);
});

test('the extension is read from the end of the name', () => {
  assert.equal(extensionOf('a.b.SBML.xml'), 'xml');
  assert.equal(extensionOf('noext'), '');
});

/*
 * A browser without a Save As window for pages, such as Firefox. The notice
 * is answered with `answer`: 'save' or 'cancel', and `tick` for "It is set:
 * don't show this again". `noticeOff` is that answer given on an earlier save.
 */
function downloadEnv({ answer = 'save', tick = false, noticeOff = false } = {}) {
  const log = [];
  const stored = new Map(noticeOff ? [[NOTICE_OFF_KEY, '1']] : []);
  const dialog = () => {
    const listeners = {};
    const parts = {
      '[data-save-cancel]': { addEventListener() {} },
      '[data-save-notice-off]': { checked: tick },
      'button[value="save"]': { focus() {} },
    };
    const el = {
      returnValue: '',
      setAttribute() {},
      querySelector: (selector) => parts[selector],
      addEventListener(type, fn) { listeners[type] = fn; },
      showModal() {
        log.push(['notice', el.innerHTML]);
        // The user answers once the notice is up.
        queueMicrotask(() => { el.returnValue = answer === 'save' ? 'save' : ''; listeners.close(); });
      },
      remove() {},
    };
    return el;
  };
  return {
    log,
    stored,
    navigator: { userAgent: 'Mozilla/5.0 (X11; Linux x86_64; rv:157.0) Gecko/20100101 Firefox/157.0' },
    localStorage: { getItem: (k) => stored.get(k) ?? null, setItem: (k, v) => stored.set(k, String(v)) },
    URL: {
      createObjectURL(blob) { log.push(['url', blob]); return 'blob:x'; },
      revokeObjectURL() {},
    },
    document: {
      createElement(tag) {
        if (tag === 'dialog') return dialog();
        const el = { tag, click() { log.push(['click', el.download, el.href]); } };
        return el;
      },
      body: { append() {} },
    },
  };
}

test('without a Save As window for pages, the notice says where the browser setting is, then the browser saves', async () => {
  const env = downloadEnv();
  const saved = await saveFile('yeast_map.json', 'application/json', () => '{"a":1}', env);
  assert.equal(saved, 'yeast_map.json');
  assert.deepEqual(env.log.map(([step]) => step), ['notice', 'url', 'click']);
  const [[, notice], [, blob], [, name, href]] = env.log;
  assert.match(notice, /Firefox saves this file straight to your Downloads folder/);
  assert.match(notice, /Settings → General → Files and Applications → Downloads → <strong>“Always ask you where to save files”<\/strong>/);
  assert.equal(await blob.text(), '{"a":1}');
  assert.equal(blob.type, 'application/json');
  assert.equal(name, 'yeast_map.json');
  assert.equal(href, 'blob:x');
  assert.equal(env.stored.has(NOTICE_OFF_KEY), false, 'shown again next time unless the user says it is set');
});

test('cancelling the notice saves nothing and makes nothing', async () => {
  const env = downloadEnv({ answer: 'cancel' });
  let made = false;
  const saved = await saveFile('map.png', 'image/png', () => { made = true; return 'x'; }, env);
  assert.equal(saved, null);
  assert.equal(made, false);
  assert.deepEqual(env.log.map(([step]) => step), ['notice']);
});

test('once the user says the setting is on, the notice is not shown again', async () => {
  const first = downloadEnv({ tick: true });
  await saveFile('map.svg', 'image/svg+xml', '<svg/>', first);
  assert.equal(first.stored.get(NOTICE_OFF_KEY), '1');

  const later = downloadEnv({ noticeOff: true });
  assert.equal(await saveFile('map.svg', 'image/svg+xml', '<svg/>', later), 'map.svg');
  assert.deepEqual(later.log.map(([step]) => step), ['url', 'click']);
});

test('a refused Save As window falls back to the browser download', async () => {
  const env = downloadEnv({ noticeOff: true });
  env.showSaveFilePicker = async () => { throw Object.assign(new Error('Not allowed'), { name: 'SecurityError' }); };
  const saved = await saveFile('map.svg', 'image/svg+xml', '<svg/>', env);
  assert.equal(saved, 'map.svg');
  assert.deepEqual(env.log.map(([step]) => step), ['url', 'click']);
});

test('the notice names the setting as each browser calls it', () => {
  const firefox = downloadSettingHelp('Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:157.0) Gecko/20100101 Firefox/157.0');
  assert.equal(firefox.name, 'Firefox');
  assert.equal(firefox.steps.at(-1), 'Always ask you where to save files');
  const safari = downloadSettingHelp('Mozilla/5.0 (Macintosh; Intel Mac OS X 14_5) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Safari/605.1.15');
  assert.equal(safari.name, 'Safari');
  assert.equal(safari.steps.at(-1), 'Ask for each download');
  const chromium = downloadSettingHelp('Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36');
  assert.equal(chromium.steps.at(-1), 'Ask where to save each file before downloading');
  assert.equal(downloadSettingHelp('').name, 'This browser');
});

test('every export in the app goes through the Save As window', async () => {
  const app = await readFile(join(root, 'src/ui/app.js'), 'utf8');
  assert.doesNotMatch(app, /\.download\s*=/, 'no export bypasses the window with a bare download link');
  assert.match(app, /import \{ saveFile \} from '\.\/save-file\.js';/);
});
