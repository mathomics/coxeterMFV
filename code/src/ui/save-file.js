/*
 * Saving a file as a desktop program does: the system's Save As window, in
 * which the user picks the folder and may change the name.
 *
 * Chrome and Edge let a page open it (showSaveFilePicker). Firefox and Safari
 * do not, so there the file goes to the browser's download, and the browser
 * opens that same system window only when it is set to ask where to save
 * files; otherwise it saves to the downloads folder. A page can neither read
 * nor change that setting, so before such a download a short notice says
 * where the setting is in this browser, until the user says it is set.
 */

const DESCRIPTIONS = {
  json: 'JSON file',
  svg: 'SVG image',
  png: 'PNG image',
  csv: 'CSV table',
  xml: 'SBML model',
  md: 'Markdown document',
};

// The window remembers the folder it was last given under this id.
const PICKER_ID = 'coxeter-export';

// Set once the user says the browser already asks where to save.
export const NOTICE_OFF_KEY = 'coxeter-save-notice-off';

/** The lower-case extension of a file name, without the dot, or '' when it has none. */
export function extensionOf(name) {
  const match = /\.([A-Za-z0-9]{1,15})$/.exec(String(name ?? ''));
  return match ? match[1].toLowerCase() : '';
}

/**
 * Ask where to save `suggestedName`, then write there what `content` gives.
 *
 * `content` is the data (a string or a Blob), or a function that makes it.
 * A function runs only once the user has chosen, so the window opens while
 * the click that asked for it still counts (the browser refuses it otherwise)
 * and a slow export does not hold it up.
 *
 * Resolves to the name the file was saved under, or null when the user
 * cancelled. Where the browser's own window does the asking, the page never
 * learns the name chosen there, and this is the suggested one. A failure to
 * make or write the file rejects.
 */
export async function saveFile(suggestedName, type, content, env = globalThis) {
  if (typeof env.showSaveFilePicker === 'function') {
    let handle = null;
    try {
      handle = await env.showSaveFilePicker(pickerOptions(suggestedName, type));
    } catch (error) {
      if (error?.name === 'AbortError') return null;
      // Refused rather than cancelled, as inside a frame: hand the file to the
      // browser's download instead.
    }
    if (handle) {
      const blob = toBlob(await make(content), type);
      const writable = await handle.createWritable();
      try {
        await writable.write(blob);
        await writable.close();
      } catch (error) {
        await writable.abort?.().catch?.(() => {});
        throw error;
      }
      return handle.name ?? suggestedName;
    }
  }

  if (!(await confirmDownload(env))) return null;
  downloadBlob(suggestedName, toBlob(await make(content), type), env);
  return suggestedName;
}

/**
 * Where this browser keeps the setting that makes it ask where to save each
 * download: `{ name, steps }`, the browser's name for the notice's opening
 * sentence and the path to the option, ending with the option itself.
 */
export function downloadSettingHelp(userAgent = '') {
  const ua = String(userAgent);
  if (/Firefox\//.test(ua)) {
    return { name: 'Firefox', steps: ['Settings', 'General', 'Files and Applications', 'Downloads', 'Always ask you where to save files'] };
  }
  // Chromium browsers that hide the page's Save As window, such as Brave.
  if (/Chrome\/|Chromium\//.test(ua)) {
    return { name: 'This browser', steps: ['Settings', 'Downloads', 'Ask where to save each file before downloading'] };
  }
  if (/Safari\//.test(ua)) {
    return { name: 'Safari', steps: ['Safari', 'Settings', 'General', 'File download location', 'Ask for each download'] };
  }
  return { name: 'This browser', steps: ['Its download settings', 'Ask where to save each file'] };
}

function noticeOff(env) {
  try { return env.localStorage?.getItem(NOTICE_OFF_KEY) === '1'; } catch { return false; }
}

function setNoticeOff(env) {
  try { env.localStorage?.setItem(NOTICE_OFF_KEY, '1'); } catch { /* not remembered */ }
}

/**
 * Before a download this browser may save without asking, say how to make it
 * ask. Resolves true to go on with the download, false when cancelled.
 */
function confirmDownload(env) {
  if (noticeOff(env)) return Promise.resolve(true);
  const { name, steps } = downloadSettingHelp(env.navigator?.userAgent);
  const path = steps.slice(0, -1).join(' → ');
  const option = steps[steps.length - 1];
  const doc = env.document;
  return new Promise((resolve) => {
    const dialog = doc.createElement('dialog');
    dialog.className = 'save-dialog';
    dialog.setAttribute('aria-labelledby', 'save-dialog-title');
    dialog.innerHTML = `<form method="dialog">
      <h2 id="save-dialog-title">Choose where to save?</h2>
      <p>${name} saves this file straight to your Downloads folder unless it is set to ask. To pick the folder and the name each time, turn on:</p>
      <p class="save-dialog-setting">${path} → <strong>“${option}”</strong></p>
      <p class="hint">A web page cannot read or change your browser's settings, so this has to be done there. Change it, then press Save.</p>
      <label class="save-dialog-check"><input type="checkbox" data-save-notice-off> It is set: don't show this again</label>
      <div class="save-dialog-actions">
        <button type="button" data-save-cancel>Cancel</button>
        <button type="submit" class="primary" value="save">Save</button>
      </div>
    </form>`;

    dialog.querySelector('[data-save-cancel]').addEventListener('click', () => dialog.close());
    // The map's own shortcuts (Escape, Delete, Ctrl+Z) are not for this window.
    dialog.addEventListener('keydown', (event) => event.stopPropagation());
    dialog.addEventListener('close', () => {
      if (dialog.querySelector('[data-save-notice-off]').checked) setNoticeOff(env);
      dialog.remove();
      resolve(dialog.returnValue === 'save');
    }, { once: true });

    doc.body.append(dialog);
    dialog.showModal();
    dialog.querySelector('button[value="save"]').focus();
  });
}

function pickerOptions(suggestedName, type) {
  const ext = extensionOf(suggestedName);
  const options = { id: PICKER_ID, suggestedName };
  if (ext) {
    options.types = [{
      description: DESCRIPTIONS[ext] ?? `${ext.toUpperCase()} file`,
      accept: { [baseType(type)]: [`.${ext}`] },
    }];
  }
  return options;
}

function baseType(type) {
  return String(type || 'application/octet-stream').split(';')[0].trim();
}

async function make(content) {
  return typeof content === 'function' ? content() : content;
}

function toBlob(data, type) {
  if (data === null || data === undefined) throw new Error('there was nothing to save');
  return data instanceof Blob ? data : new Blob([data], { type });
}

function downloadBlob(name, blob, env) {
  const url = env.URL.createObjectURL(blob);
  const a = env.document.createElement('a');
  a.href = url;
  a.download = name;
  a.click();
  setTimeout(() => env.URL.revokeObjectURL(url), 2000);
}
