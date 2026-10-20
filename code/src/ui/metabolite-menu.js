/**
 * Add Reaction panel anchored to one drawn metabolite.
 *
 * The selected metabolite is the sole reuse exception: the new reaction starts
 * from that existing node. Every other metabolite is created as a fresh visual
 * instance by the placement engine.
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

export class MetaboliteMenuController {
  constructor({ stage, store, onChange }) {
    this.stage = stage;
    this.store = store;
    this.onChange = onChange;
    this.state = null;
    this.stage.addEventListener('click', (event) => this.handleClick(event));
    this.stage.addEventListener('input', (event) => this.handleFilterInput(event));
    this.stage.addEventListener('pointerdown', (event) => beginPanelDrag(this, event));
  }

  isMenuTarget(target) {
    return Boolean(target?.closest?.('.add-reaction-menu'));
  }

  open(nodeId, event) {
    const node = this.store.map?.nodes.get(nodeId);
    if (!node) return;

    const rect = this.stage.getBoundingClientRect();
    const reactions = this.store.reactionsAvailableFromMetabolite(nodeId);
    this.state = {
      nodeId,
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
    if (!this.state || !this.store.map || !this.store.community) return '';
    const node = this.store.map.nodes.get(this.state.nodeId);
    if (!node) return '';

    const reactions = this.store.reactionsAvailableFromMetabolite(node.id);
    if (this.state.selectedReactionId && !reactions.includes(this.state.selectedReactionId)) {
      this.state.selectedReactionId = reactions.length === 1 ? reactions[0] : null;
    }

    const { left, top, width, maxHeight } = panelPlacement(this.stage, this.state);
    const label = this.store.mapLabelText?.('metabolite', node.id)
      || node.base || node.localId || node.mergedId || 'Selected metabolite';

    return renderAddReactionPanel({
      store: this.store,
      reactionIds: reactions,
      selectedReactionId: this.state.selectedReactionId,
      title: 'Add reaction',
      subtitle: `From ${label} · selected metabolite will be reused`,
      left,
      top,
      width,
      maxHeight,
      terminalMetaboliteId: this.state.terminalMetaboliteId,
      // The list is ordered by these fluxes (see Store#orderByFlux).
      fluxes: this.store.solution?.status === 'optimal' ? this.store.solution.fluxes : null,
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
    const node = this.store.map?.nodes.get(this.state.nodeId);
    if (!direction || !node) return;

    event.preventDefault();
    event.stopPropagation();

    const reactionId = this.state.selectedReactionId;
    const terminalMetaboliteId = this.state.terminalMetaboliteId;
    const anchor = {
      x: node.x + direction.dx * REACTION_PLACEMENT_DISTANCE,
      y: node.y + direction.dy * REACTION_PLACEMENT_DISTANCE,
    };
    this.state = null;

    const result = this.store.addMapReaction(reactionId, {
      anchor,
      anchorNodeId: node.id,
      placementDirection: { x: direction.dx, y: direction.dy },
      terminalMetaboliteId,
    });
    if (!result) this.onChange();
  }
}
