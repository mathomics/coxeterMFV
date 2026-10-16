/**
 * Boundary reaction menu.
 *
 * Lists model reactions crossing a membrane that are not already drawn.
 */

import { reactionsCrossing } from '../map/boundaries.js';
import { reactionListing } from './add-reaction-panel.js';

const esc = (value) => String(value ?? '')
  .replace(/&/g, '&amp;')
  .replace(/</g, '&lt;')
  .replace(/>/g, '&gt;')
  .replace(/"/g, '&quot;');

export class BoundaryMenuController {
  constructor({ stage, store, screenToMap, onChange }) {
    this.stage = stage;
    this.store = store;
    this.screenToMap = screenToMap;
    this.onChange = onChange;
    this.state = null;
    this.stage.addEventListener('click', (event) => this.handleClick(event));
  }

  isOpen() { return Boolean(this.state); }

  isMenuTarget(target) {
    return Boolean(target?.closest?.('.boundary-menu'));
  }

  open(boundaryId, event) {
    if (!boundaryId || !this.store.map || !this.store.community) return;
    const boundary = this.store.map.boundary(boundaryId);
    if (!boundary) return;
    if (this.state?.boundaryId === boundaryId) return;

    // A full map often has every transporter already drawn. In that case the
    // membrane still highlights on hover, but no empty menu obscures the map.
    if (this.availableFor(boundaryId).length === 0) return;

    const rect = this.stage.getBoundingClientRect();
    this.state = {
      boundaryId,
      anchor: this.screenToMap(event.clientX, event.clientY),
      left: event.clientX - rect.left + 12,
      top: event.clientY - rect.top + 12,
    };
    this.onChange();
  }

  close() {
    if (!this.state) return;
    this.state = null;
    this.onChange();
  }

  availableFor(boundaryId) {
    if (!boundaryId || !this.store.community || !this.store.map) return [];
    return reactionsCrossing(this.store.community, boundaryId)
      .filter((reactionId) => !this.store.map.hasReaction(reactionId));
  }

  available() {
    return this.state ? this.availableFor(this.state.boundaryId) : [];
  }

  render() {
    if (!this.state || !this.store.map) return '';
    const boundary = this.store.map.boundary(this.state.boundaryId);
    if (!boundary) return '';

    const reactions = this.available();
    const width = 330;
    const left = Math.max(8, Math.min(this.state.left, this.stage.clientWidth - width - 8));
    const top = Math.max(8, Math.min(this.state.top, this.stage.clientHeight - 300));
    const title = boundary.label || boundary.inside || 'Membrana';
    const subtitle = `${boundary.outside ?? 'exterior'} ↔ ${boundary.inside}`;

    const rows = reactions.length
      ? reactions.map((reactionId) => {
          const { label, detail } = reactionListing(this.store, reactionId);
          return `
            <button class="boundary-reaction" data-add-boundary-rxn="${esc(reactionId)}" type="button">
              <strong>${esc(label)}</strong>
              <span>${esc(detail)}</span>
            </button>`;
        }).join('')
      : '<p class="boundary-menu-empty">No undrawn transport reactions are left on this membrane.</p>';

    return `
      <div class="boundary-menu" style="left:${left}px;top:${top}px" data-boundary-menu="${esc(this.state.boundaryId)}">
        <div class="boundary-menu-head">
          <div><strong>${esc(title)}</strong><span>${esc(subtitle)}</span></div>
          <button class="boundary-menu-close" data-action="close-boundary-menu" type="button" title="Close">×</button>
        </div>
        <div class="boundary-menu-summary">
          ${reactions.length} ${reactions.length === 1 ? 'reaction available' : 'reactions available'}
        </div>
        <div class="boundary-menu-list">${rows}</div>
      </div>`;
  }

  handleClick(event) {
    const close = event.target.closest('[data-action="close-boundary-menu"]');
    if (close) {
      event.preventDefault();
      event.stopPropagation();
      this.close();
      return;
    }

    const button = event.target.closest('[data-add-boundary-rxn]');
    if (!button || !this.state) return;

    event.preventDefault();
    event.stopPropagation();

    const reactionId = button.dataset.addBoundaryRxn;
    const { boundaryId, anchor } = this.state;
    this.state = null;
    const result = this.store.addMapReaction(reactionId, { boundaryId, anchor });
    if (!result) this.onChange();
  }
}
