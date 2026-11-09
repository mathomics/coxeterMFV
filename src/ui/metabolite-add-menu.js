/**
 * Add Metabolite panel anchored to free space in one compartment. The chosen
 * metabolite is placed at the click point as a new visual instance.
 */
import { beginPanelDrag, panelPlacement } from './add-reaction-panel.js';

const esc = value => String(value ?? '').replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/"/g, '&quot;');

export class MetaboliteAddMenuController {
  constructor({ stage, store, screenToMap, onChange }) {
    this.stage = stage;
    this.store = store;
    this.screenToMap = screenToMap;
    this.onChange = onChange;
    this.state = null;
    stage.addEventListener('click', event => this.click(event));
    stage.addEventListener('input', event => this.input(event));
    stage.addEventListener('pointerdown', event => beginPanelDrag(this, event));
  }

  isMenuTarget(target) {
    return Boolean(target?.closest?.('.add-metabolite-menu'));
  }

  close() {
    if (this.state) {
      this.panelDrag?.end();
      this.state = null;
      this.onChange();
    }
  }

  open(cellId, compartmentId, event) {
    const cell = this.store.map?.cells.find(item => item.id === cellId);
    const compartment = cell?.compartments.find(item => item.id === compartmentId);
    if (!compartment) {
      return;
    }
    const rect = this.stage.getBoundingClientRect();
    this.panelDrag?.end();
    this.state = {
      cellId, compartmentId, label: compartment.label || compartmentId,
      origin: this.screenToMap(event.clientX, event.clientY),
      left: event.clientX - rect.left + 12, top: event.clientY - rect.top + 12,
      filter: '',
    };
    this.onChange();
  }

  render() {
    this.panelDrag?.end();
    if (!this.state) {
      return '';
    }
    const metabolites = this.store.metabolitesAvailableInCompartment(this.state.cellId, this.state.compartmentId);
    const query = this.state.filter.trim().toLowerCase();
    const rows = metabolites.map(metabolite => {
      const search = `${metabolite.label} ${metabolite.name} ${metabolite.modelId ?? ''}`.toLowerCase();
      const matches = !query || search.includes(query);
      return `<button class="add-metabolite-choice" type="button" data-add-metabolite="${esc(metabolite.id)}"
        data-search="${esc(search)}"${matches ? '' : ' hidden'}>
        <span><strong>${esc(metabolite.label)}</strong><small>${esc(metabolite.name)}</small></span>
        <em>${metabolite.count ? `on map (${metabolite.count})` : ''}</em></button>`;
    }).join('');
    const { left, top, width, maxHeight } = panelPlacement(this.stage, this.state);
    const scrollable = maxHeight < 320 || width < 240 ? ' is-scrollable' : '';
    return `<div class="add-reaction-menu add-metabolite-menu${scrollable}"
      style="left:${left}px;top:${top}px;width:${width}px;max-height:${maxHeight}px">
      <div class="add-reaction-head" title="Drag to move this panel">
        <div><strong>Add metabolite</strong><span>${esc(this.state.label)}</span></div>
        <button class="add-reaction-close" data-close-add-metabolite type="button" title="Close" aria-label="Close">×</button>
      </div>
      <div class="add-reaction-search"><input data-filter-metabolites value="${esc(this.state.filter)}"
        placeholder="Search metabolites…" autocomplete="off"></div>
      <div class="add-reaction-list">${rows || '<p class="add-reaction-empty">No metabolites available here.</p>'}</div>
    </div>`;
  }

  input(event) {
    const input = event.target.closest?.('[data-filter-metabolites]');
    if (!input || !this.state) {
      return;
    }
    event.stopPropagation();
    this.state.filter = input.value;
    const query = input.value.trim().toLowerCase();
    const choices = input.closest('.add-metabolite-menu')?.querySelectorAll('.add-metabolite-choice') ?? [];
    for (const button of choices) {
      button.hidden = Boolean(query) && !button.dataset.search.includes(query);
    }
  }

  click(event) {
    if (!this.state) {
      return;
    }
    if (event.target.closest?.('[data-close-add-metabolite]')) {
      event.preventDefault();
      event.stopPropagation();
      this.close();
      return;
    }
    const button = event.target.closest?.('[data-add-metabolite]');
    if (!button) {
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    const point = this.state.origin;
    const id = button.dataset.addMetabolite;
    this.state = null;
    const result = this.store.addMapMetabolite(id, { x: point.x, y: point.y });
    if (!result) {
      this.onChange();
    }
  }
}
