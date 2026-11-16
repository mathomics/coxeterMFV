/**
 * The welcome page that opens over the map when the app starts. It says what
 * Coxeter-MFV is, opens one of the paper's three case studies or the user's own
 * model, links the user guide, the source code and the citation, and states the
 * license. It opens at every start until the user turns that off, except when
 * the address already names a scenario, and the ? button in the header opens it
 * again. It changes neither the map nor the model: an example loads through the
 * same scenario list as the Scenario page.
 */

/** The release repository, which holds the source code, the licenses and the citation file. */
export const REPOSITORY_URL = 'https://github.com/mathomics/coxeterMFV';

/** The website, whose help page explains the analyses and how to read a solved map. */
export const SITE_URL = 'https://mathomics.github.io/coxeterMFV/';

/** The help links, in the order shown. */
export const WELCOME_LINKS = [
  { label: 'User guide', href: `${SITE_URL}help.html` },
  { label: 'Source code', href: REPOSITORY_URL },
  { label: 'How to cite', href: `${REPOSITORY_URL}/blob/main/CITATION.cff` },
];

/**
 * The paper's three case studies, as values of the scenario list: `sc:` names
 * a teaching scenario and `pub:` a published community.
 */
export const WELCOME_EXAMPLES = [
  { label: 'Yeast9 compartments', value: 'sc:yeast9-9-1-1', title: 'Yeast9 9.1.1 drawn across six nested compartments' },
  { label: 'Phenylalanine rescue', value: 'pub:zuniga-2020-phea-rescue', title: 'S. elongatus rescues an E. coli ΔpheA mutant (Zuñiga et al. 2020)' },
  { label: 'Auxotrophic community', value: 'pub:steadycom-ecoli-auxotrophs', title: 'Four auxotrophic E. coli strains solved with SteadyCom' },
];

/** The license of the source code. */
export const CODE_LICENSE_URL = `${REPOSITORY_URL}/blob/main/LICENSE`;

/** The license of the bundled glpk.js, which makes the built page GPL-3.0. */
export const BUILD_LICENSE_URL = `${REPOSITORY_URL}/blob/main/LICENSES/GPL-3.0.txt`;

/** The browser storage key that remembers "Show at startup" turned off. */
export const WELCOME_STORAGE_KEY = 'coxeter-welcome';

/**
 * Whether the welcome page opens at start: yes unless the user turned it off,
 * or the address names a scenario to open (a shared link or a test page).
 * Storage that cannot be read (private windows) counts as never turned off.
 *
 * @param {Storage | null | undefined} storage
 * @param {string} search The page address's query, `location.search`.
 * @returns {boolean}
 */
export function welcomeOpensAtStart(storage, search = '') {
  if (new URLSearchParams(search).has('scenario')) return false;
  try {
    return storage?.getItem(WELCOME_STORAGE_KEY) !== 'off';
  } catch (_) {
    return true;
  }
}

/**
 * The scenario a link opens with `?scenario=`: a teaching scenario, by its id
 * or as `sc:id`, or a published community as `pub:id`, the same values the
 * examples above use, so a web page can link to each case study already open.
 * Null when the address names none.
 *
 * @param {string} search The page address's query, `location.search`.
 * @returns {{kind: 'scenario'|'published', id: string} | null}
 */
export function scenarioFromAddress(search = '') {
  const value = new URLSearchParams(search).get('scenario');
  if (!value) return null;
  if (value.startsWith('pub:')) {
    return { kind: 'published', id: value.slice('pub:'.length) };
  }
  if (value.startsWith('sc:')) {
    return { kind: 'scenario', id: value.slice('sc:'.length) };
  }
  return { kind: 'scenario', id: value };
}

/**
 * Remember the "Show at startup" box.
 *
 * @param {Storage | null | undefined} storage
 * @param {boolean} shown
 */
export function rememberWelcomeAtStart(storage, shown) {
  try {
    if (shown) {
      storage?.removeItem(WELCOME_STORAGE_KEY);
    } else {
      storage?.setItem(WELCOME_STORAGE_KEY, 'off');
    }
  } catch (_) {
    // Storage refused (a private window): the box applies to this visit only.
  }
}

/**
 * The page's content, for `<dialog id="welcome">`.
 *
 * @param {boolean} showAtStart The state of the "Show at startup" box.
 * @returns {string}
 */
export function welcomeMarkup(showAtStart) {
  const link = (href, label) => `<a href="${href}" target="_blank" rel="noopener">${label}</a>`;
  const links = WELCOME_LINKS.map((item) => link(item.href, item.label)).join('');
  const examples = WELCOME_EXAMPLES.map((item) => (
    `<button type="button" data-action="welcome-example" data-value="${item.value}" title="${item.title}">${item.label}</button>`
  )).join('');
  return `<form method="dialog" class="welcome-body">
    <h2 id="welcome-title">Coxeter-MFV</h2>
    <p class="welcome-lead">Multicompartment metabolic maps for flux analysis of cells and microbial communities.</p>
    <nav class="welcome-links" aria-label="Help">${links}</nav>
    <h3>Try an example</h3>
    <div class="welcome-examples">${examples}</div>
    <h3>Your own model</h3>
    <button type="button" data-action="welcome-import" title="SBML, COBRA JSON, MAT or XLSX">Import a model</button>
    <p class="welcome-license">Free to use, no login. Code under the ${link(CODE_LICENSE_URL, 'MIT License')}.
      This build includes glpk.js and is distributed under ${link(BUILD_LICENSE_URL, 'GPL-3.0')}.</p>
    <div class="welcome-foot">
      <label><input type="checkbox" data-action="welcome-at-start"${showAtStart ? ' checked' : ''}> Show at startup</label>
      <button type="submit" class="primary">Open the editor</button>
    </div>
  </form>`;
}
