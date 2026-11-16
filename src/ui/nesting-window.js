/**
 * The Edit compartments side window.
 *
 * It opens with the Edit Compartments tool, over the map's left edge right
 * beside the tool column, and shows one drawn cell's compartments as a tree: every
 * compartment of the species' model, each under the one it is drawn inside,
 * with a small drawing of the nested boxes underneath. The nesting is changed
 * by dragging a row onto another, by Tab and Shift+Tab, or by starting over
 * from a preset. Every change goes through the store's MAP operations, so it
 * is one undo step and is saved with the map; the model is never touched.
 *
 * The window lives in its own layer over the map, outside the map stage, so
 * repainting the map neither rebuilds it nor takes the keyboard focus away
 * from the tree.
 */

import { NESTING_PRESETS, compartmentColors } from '../core/compartments.js';
import { beginPanelDrag, panelPlacement } from './add-reaction-panel.js';

/**
 * Width of the window in screen pixels: room for a compartment name, its id
 * and a badge with four levels of indent in front.
 */
const NESTING_WINDOW_WIDTH = 320;

/** The tallest the window grows before its contents scroll. */
const NESTING_WINDOW_HEIGHT = 640;

/** How far each nesting level is indented in the tree, in pixels. */
const NESTING_INDENT = 16;

/**
 * The share of a row's height, at its top and at its bottom edge, where a drop
 * puts the dragged compartment beside that row instead of inside it.
 */
const DROP_EDGE = 0.25;

/** How the presets are named in the Start from list. */
const PRESET_LABELS = { auto: 'Automatic', yeast9: 'Yeast9', bigg: 'BiGG', empty: 'Empty' };

const esc = (value) => String(value ?? '')
  .replace(/&/g, '&amp;')
  .replace(/</g, '&lt;')
  .replace(/>/g, '&gt;')
  .replace(/"/g, '&quot;');

export class NestingWindowController {
  /**
   * @param {object} options
   * @param {HTMLElement} options.layer the element over the map the window is drawn in
   * @param {import('./store.js').Store} options.store
   */
  constructor({ layer, store }) {
    this.layer = layer;
    // panelPlacement and beginPanelDrag keep the window inside this element.
    this.stage = layer;
    this.store = store;
    this.state = null;
    this.panelDrag = null;
    // Closed by hand while the tool is still on: it stays closed until the
    // Nesting button opens it again or the tool is picked anew.
    this.dismissed = false;
    layer.addEventListener('click', (event) => this.handleClick(event));
    layer.addEventListener('change', (event) => this.handleChange(event));
    layer.addEventListener('keydown', (event) => this.handleKey(event));
    layer.addEventListener('pointerdown', (event) => beginPanelDrag(this, event));
    layer.addEventListener('dragstart', (event) => this.handleDragStart(event));
    layer.addEventListener('dragover', (event) => this.handleDragOver(event));
    layer.addEventListener('drop', (event) => this.handleDrop(event));
    layer.addEventListener('dragend', () => this.endRowDrag());
  }

  get isOpen() {
    return Boolean(this.state);
  }

  /**
   * Keep the window in step with the active tool: open while Edit
   * Compartments is the tool, unless closed by hand, and repainted with
   * whatever the store now holds.
   */
  follow(toolActive, speciesId = null) {
    if (!toolActive) {
      this.close();
      this.dismissed = false;
      return;
    }
    if (this.state) {
      this.paint();
    } else if (!this.dismissed) {
      this.open(speciesId);
    }
  }

  /** Open the window, on the given cell if it is drawn. */
  open(speciesId = null) {
    this.dismissed = false;
    const cells = this.store.nestingCells();
    if (!this.state) {
      // It starts at the top left of the map, next to the tools it belongs to.
      this.state = { left: 0, top: 0, moved: false, speciesId: null, selectedId: null, dragId: null };
    }
    const chosen = cells.find((cell) => cell.speciesId === speciesId) ?? null;
    if (chosen) {
      this.state.speciesId = chosen.speciesId;
      this.state.selectedId = null;
    }
    this.paint();
  }

  close() {
    if (!this.state) return;
    this.panelDrag?.end();
    this.state = null;
    this.layer.innerHTML = '';
    this.layer.hidden = true;
  }

  paint() {
    if (!this.state) return;
    // Rebuilding the tree in the middle of a row drag would cancel it.
    if (this.state.dragId) return;
    const active = document.activeElement;
    const treeHadFocus = this.layer.contains(active) && active?.getAttribute?.('role') === 'treeitem';
    // The window is repainted on every change in the store; where its list
    // was scrolled to is kept, or each change would jump it back to the top.
    const scrolled = this.layer.querySelector('.nesting-body')?.scrollTop ?? 0;
    this.panelDrag?.end();
    this.layer.hidden = false;
    this.layer.innerHTML = this.render();
    const body = this.layer.querySelector('.nesting-body');
    if (body) {
      body.scrollTop = scrolled;
    }
    if (treeHadFocus) {
      this.focusSelectedRow();
    }
  }

  focusSelectedRow() {
    const rows = this.layer.querySelectorAll('[role="treeitem"]');
    for (const row of rows) {
      if (row.dataset.id === this.state?.selectedId) {
        row.focus();
        return;
      }
    }
  }

  // Rendering

  render() {
    const store = this.store;
    const state = this.state;
    const cells = store.nestingCells();
    if (!cells.some((cell) => cell.speciesId === state.speciesId)) {
      state.speciesId = cells[0]?.speciesId ?? null;
      state.selectedId = null;
    }
    const view = state.speciesId ? store.compartmentNesting(state.speciesId) : null;
    if (view && !view.rows.some((row) => row.id === state.selectedId)) {
      state.selectedId = view.rows.find((row) => !row.outer)?.id ?? view.rows[0]?.id ?? null;
    }

    const place = panelPlacement(this.layer, state, { width: NESTING_WINDOW_WIDTH, maxHeight: NESTING_WINDOW_HEIGHT });
    const cellName = cells.find((cell) => cell.speciesId === state.speciesId)?.name ?? '';
    const head = `<div class="add-reaction-head">
        <div><strong>Edit compartments</strong>${cells.length === 1 ? `<span>${esc(cellName)}</span>` : ''}</div>
        <button class="add-reaction-close" type="button" data-nesting="close" aria-label="Close" title="Close">×</button>
      </div>`;

    let body;
    if (!view) {
      body = '<p class="empty">Draw a cell first.</p>';
    } else {
      body = this.renderCellPicker(cells) + this.renderPresets(view) + this.renderTree(view)
        + `<div class="nesting-actions">
            <button type="button" data-nesting="out" title="Move out (Shift+Tab)">⇤ Out</button>
            <button type="button" data-nesting="in" title="Nest under the row above (Tab)">In ⇥</button>
          </div>`
        + this.renderPreview(view);
    }

    return `<div class="add-reaction-menu nesting-window" role="dialog" aria-label="Edit compartments"
        style="left:${place.left}px;top:${place.top}px;width:${place.width}px;max-height:${place.maxHeight}px">
        ${head}<div class="nesting-body">${body}</div>
      </div>`;
  }

  renderCellPicker(cells) {
    if (cells.length < 2) return '';
    const options = cells.map((cell) =>
      `<option value="${esc(cell.speciesId)}"${cell.speciesId === this.state.speciesId ? ' selected' : ''}>${esc(cell.name)}</option>`,
    ).join('');
    return `<label class="nesting-field">Cell <select data-nesting="cell">${options}</select></label>`;
  }

  renderPresets(view) {
    // A nesting set by hand matches none of the presets, and picking one of
    // them replaces it, so it is listed as what it is.
    let options = view.custom ? '<option value="" selected disabled>Edited</option>' : '';
    for (const preset of NESTING_PRESETS) {
      const selected = !view.custom && preset === 'auto' ? ' selected' : '';
      options += `<option value="${preset}"${selected}>${PRESET_LABELS[preset]}</option>`;
    }
    return `<label class="nesting-field">Start from <select data-nesting="preset">${options}</select></label>`;
  }

  renderTree(view) {
    const dark = Boolean(this.store.view?.dark);
    let items = '';
    for (const row of view.rows) {
      const colors = compartmentColors(row.visualId, dark);
      const selected = row.id === this.state.selectedId;
      let badge = '<span class="nesting-badge">Not drawn</span>';
      if (row.outer) {
        badge = '<span class="nesting-badge">Outermost</span>';
      } else if (row.parentId === null) {
        badge = '<span class="nesting-badge is-warning">Outside cell</span>';
      } else if (row.drawn) {
        badge = '<span class="nesting-badge is-drawn">On map</span>';
      }
      items += `<li role="treeitem" class="nesting-row${selected ? ' is-selected' : ''}" data-id="${esc(row.id)}"
          aria-level="${row.depth + 1}" aria-selected="${selected}" tabindex="${selected ? 0 : -1}"
          ${row.outer ? '' : 'draggable="true"'} style="padding-left:${8 + row.depth * NESTING_INDENT}px">
          <span class="nesting-swatch" style="background:${colors.fill};border-color:${colors.stroke}"></span>
          <span class="nesting-name">${esc(row.name)}</span><code>${esc(row.id)}</code>${badge}
        </li>`;
    }
    return `<ul class="nesting-tree" role="tree" aria-label="Compartment nesting">${items}</ul>`;
  }

  /** The cell drawn small: each compartment a box inside the box of its parent. */
  renderPreview(view) {
    const dark = Boolean(this.store.view?.dark);
    const box = (row) => {
      const colors = compartmentColors(row.visualId, dark);
      let inner = '';
      for (const child of view.rows) {
        if (child.parentId === row.id) inner += box(child);
      }
      const stray = row.parentId === null && !row.outer ? ' is-stray' : '';
      return `<div class="nesting-box${stray}" style="background:${colors.fill};border-color:${colors.stroke}">`
        + `<span style="color:${colors.text}">${esc(row.id)}</span>${inner}</div>`;
    };
    let tops = '';
    for (const row of view.rows) {
      if (row.parentId === null) tops += box(row);
    }
    return `<div class="nesting-preview" aria-hidden="true">${tops}</div>`;
  }

  // Events

  handleClick(event) {
    if (!this.state) return;
    const button = event.target.closest?.('button[data-nesting]');
    if (button) {
      const action = button.dataset.nesting;
      if (action === 'close') {
        this.close();
        this.dismissed = true;
      } else if (action === 'in' || action === 'out') {
        this.nest(action);
      }
      return;
    }
    const row = event.target.closest?.('[role="treeitem"]');
    if (row) {
      this.state.selectedId = row.dataset.id;
      this.paint();
      this.focusSelectedRow();
    }
  }

  handleChange(event) {
    if (!this.state) return;
    const select = event.target.closest?.('select[data-nesting]');
    if (!select) return;
    if (select.dataset.nesting === 'cell') {
      this.state.speciesId = select.value;
      this.state.selectedId = null;
      this.paint();
    } else if (select.dataset.nesting === 'preset' && select.value) {
      this.store.applyNestingPreset(this.state.speciesId, select.value);
    }
  }

  /**
   * Keys pressed in the window stay in it: Delete would otherwise remove the
   * selected compartment from the map, and Escape would leave the tool.
   */
  handleKey(event) {
    if (!this.state) return;
    if (event.key === 'Escape') {
      event.stopPropagation();
      this.close();
      this.dismissed = true;
      return;
    }
    if (event.key === 'Delete' || event.key === 'Backspace') {
      event.stopPropagation();
      return;
    }
    const row = event.target.closest?.('[role="treeitem"]');
    if (!row) return;
    if (event.key === 'Tab') {
      event.preventDefault();
      event.stopPropagation();
      this.nest(event.shiftKey ? 'out' : 'in');
      return;
    }
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      event.stopPropagation();
      const view = this.store.compartmentNesting(this.state.speciesId);
      const ids = view?.rows.map((item) => item.id) ?? [];
      const next = ids.indexOf(this.state.selectedId) + (event.key === 'ArrowDown' ? 1 : -1);
      if (next >= 0 && next < ids.length) {
        this.state.selectedId = ids[next];
        this.paint();
        this.focusSelectedRow();
      }
    }
  }

  nest(direction) {
    const id = this.state?.selectedId;
    if (!id) return;
    this.store.nestCompartment(this.state.speciesId, id, direction);
  }

  // Dragging rows

  handleDragStart(event) {
    const row = event.target.closest?.('[role="treeitem"][draggable="true"]');
    if (!row || !this.state) return;
    this.state.dragId = row.dataset.id;
    this.state.selectedId = row.dataset.id;
    event.dataTransfer.effectAllowed = 'move';
    event.dataTransfer.setData('text/plain', row.dataset.id);
  }

  /**
   * Where a drop on this row would put the dragged compartment, or null when
   * it cannot go there (on itself, or inside something nested in it).
   * @returns {{row: HTMLElement, parentId: string, zone: 'inside'|'before'|'after'}|null}
   */
  dropTarget(event) {
    const row = event.target.closest?.('[role="treeitem"]');
    const dragId = this.state?.dragId;
    if (!row || !dragId) return null;
    const view = this.store.compartmentNesting(this.state.speciesId);
    const parentOf = new Map(view?.rows.map((item) => [item.id, item.parentId]) ?? []);
    const target = row.dataset.id;
    for (let at = target; at; at = parentOf.get(at)) {
      if (at === dragId) return null;
    }
    const box = row.getBoundingClientRect();
    const share = box.height > 0 ? (event.clientY - box.top) / box.height : 0.5;
    let zone = 'inside';
    if (target !== view.outerId && share < DROP_EDGE) {
      zone = 'before';
    } else if (target !== view.outerId && share > 1 - DROP_EDGE) {
      zone = 'after';
    }
    const parentId = zone === 'inside' ? target : (parentOf.get(target) ?? view.outerId);
    return { row, parentId, zone };
  }

  handleDragOver(event) {
    const drop = this.dropTarget(event);
    for (const row of this.layer.querySelectorAll('.nesting-row')) {
      row.classList.remove('drop-inside', 'drop-before', 'drop-after');
    }
    if (!drop) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = 'move';
    drop.row.classList.add(`drop-${drop.zone}`);
  }

  handleDrop(event) {
    const drop = this.dropTarget(event);
    if (!drop) return;
    event.preventDefault();
    const dragId = this.state.dragId;
    this.state.dragId = null;
    if (!this.store.setCompartmentParent(this.state.speciesId, dragId, drop.parentId)) {
      this.paint();
    }
  }

  endRowDrag() {
    if (!this.state?.dragId) return;
    this.state.dragId = null;
    this.paint();
  }
}
