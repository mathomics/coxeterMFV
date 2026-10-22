/**
 * Add Reaction panel anchored to free space in one cellular compartment.
 *
 * Unlike the metabolite-origin workflow, this controller never passes an
 * anchorNodeId. Therefore every metabolite in the inserted reaction is created
 * as a fresh visual instance.
 */
import {
  REACTION_PLACEMENT_DISTANCE,
  directionById,
  filterReactionButtons,
  primaryMetabolitesForSide,
  renderAddReactionPanel,
  panelPlacement,
  beginPanelDrag,
} from './add-reaction-panel.js';

export class CompartmentMenuController {
  constructor({ stage, store, screenToMap, onChange }) {
    this.stage = stage;
    this.store = store;
    this.screenToMap = screenToMap;
    this.onChange = onChange;
    this.state = null;
    this.stage.addEventListener('click', (event) => this.handleClick(event));
    this.stage.addEventListener('input', (event) => this.handleFilterInput(event));
    this.stage.addEventListener('pointerdown', (event) => beginPanelDrag(this, event));
  }

  isMenuTarget(target) {
    return Boolean(target?.closest?.('.add-reaction-menu'));
  }

  availableFor(cellId, compartmentId) {
    return this.store.reactionsAvailableInCompartment(cellId, compartmentId);
  }

  open(cellId, compartmentId, event) {
    if (!this.store.map || !this.store.community) return;

    const cell = this.store.map.cells.find((item) => item.id === cellId);
    const compartment = cell?.compartments.find((item) => item.id === compartmentId);
    if (!cell || !compartment) return;

    const rect = this.stage.getBoundingClientRect();
    const reactions = this.availableFor(cellId, compartmentId);
    this.state = {
      key: `${cellId}/${compartmentId}`,
      cellId,
      compartmentId,
      label: compartment.label || compartmentId,
      origin: this.screenToMap(event.clientX, event.clientY),
      left: event.clientX - rect.left + 12,
      top: event.clientY - rect.top + 12,
      selectedReactionId: reactions.length === 1 ? reactions[0] : null,
      terminalMetaboliteId: null,
    };
    this.onChange();
  }

  close() {
    if (!this.state) return;
    this.state = null;
    this.onChange();
  }

  render() {
    if (!this.state) return '';

    const reactions = this.availableFor(this.state.cellId, this.state.compartmentId);
    if (this.state.selectedReactionId && !reactions.includes(this.state.selectedReactionId)) {
      this.state.selectedReactionId = reactions.length === 1 ? reactions[0] : null;
    }

    const { left, top, width, maxHeight } = panelPlacement(this.stage, this.state);

    return renderAddReactionPanel({
      store: this.store,
      reactionIds: reactions,
      selectedReactionId: this.state.selectedReactionId,
      title: 'Add reaction',
      subtitle: this.state.label,
      left,
      top,
      width,
      maxHeight,
      terminalMetaboliteId: this.state.terminalMetaboliteId,
    });
  }

  handleFilterInput(event) {
    const input = event.target.closest?.('[data-action="filter-add-reactions"]');
    if (!input || !this.state) return;
    event.stopPropagation();
    filterReactionButtons(input);
  }

  handleClick(event) {
    if (!this.state) return;

    const close = event.target.closest?.('[data-action="close-add-reaction-menu"]');
    if (close) {
      event.preventDefault();
      event.stopPropagation();
      this.close();
      return;
    }

    const choice = event.target.closest?.('[data-select-add-rxn]');
    if (choice) {
      event.preventDefault();
      event.stopPropagation();
      this.state.selectedReactionId = choice.dataset.selectAddRxn;
      this.state.terminalMetaboliteId = null;
      this.onChange();
      return;
    }

    const rotateTerminal = event.target.closest?.('[data-rotate-terminal]');
    if (rotateTerminal && this.state.selectedReactionId) {
      event.preventDefault();
      event.stopPropagation();
      const terminals = primaryMetabolitesForSide(this.store, this.state.selectedReactionId, 'product');
      if (terminals.length) {
        const current = terminals.findIndex((item) => item.id === this.state.terminalMetaboliteId);
        const start = current >= 0 ? current : 0;
        const step = Number(rotateTerminal.dataset.rotateTerminal) < 0 ? -1 : 1;
        this.state.terminalMetaboliteId = terminals[(start + step + terminals.length) % terminals.length].id;
        this.onChange();
      }
      return;
    }

    const directionButton = event.target.closest?.('[data-add-rxn-direction]');
    if (!directionButton || !this.state.selectedReactionId) return;

    const direction = directionById(directionButton.dataset.addRxnDirection);
    if (!direction) return;

    event.preventDefault();
    event.stopPropagation();

    const reactionId = this.state.selectedReactionId;
    const terminalMetaboliteId = this.state.terminalMetaboliteId;
    const origin = this.state.origin;
    const anchor = {
      x: origin.x + direction.dx * REACTION_PLACEMENT_DISTANCE,
      y: origin.y + direction.dy * REACTION_PLACEMENT_DISTANCE,
    };
    this.state = null;

    // Intentionally no anchorNodeId: compartment-origin insertion duplicates
    // every metabolite even if matching nodes already exist elsewhere.
    const result = this.store.addMapReaction(reactionId, {
      anchor,
      placementDirection: { x: direction.dx, y: direction.dy },
      terminalMetaboliteId,
    });
    if (!result) this.onChange();
  }
}
