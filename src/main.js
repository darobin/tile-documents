import { LitElement, html, css } from 'lit';
import { listen } from '@tauri-apps/api/event';
import { addTab } from './state.js';
import './components/tab-bar.js';
import './components/tile-tab.js';

// ── Root app shell ────────────────────────────────────────────────────────────

class TileApp extends LitElement {
  static styles = css`
    :host {
      display: flex;
      flex-direction: column;
      height: 100vh;
      overflow: hidden;
    }
  `;

  connectedCallback() {
    super.connectedCallback();
    // Listen for tiles opened by the backend (OS file-open, CLI arg, etc.)
    listen('tile:opened', (event) => {
      const { authority, masl } = event.payload;
      addTab(authority, masl);
    });
  }

  render() {
    return html`
      <tile-tab-bar></tile-tab-bar>
      <tile-content></tile-content>
    `;
  }
}

customElements.define('tile-app', TileApp);
