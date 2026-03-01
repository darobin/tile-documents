import { store } from 'refrakt';

// ── Actions ──────────────────────────────────────────────────────────────────

export const ADD_TAB = 'ADD_TAB';
export const CLOSE_TAB = 'CLOSE_TAB';
export const ACTIVATE_TAB = 'ACTIVATE_TAB';
export const SET_FULLSCREEN = 'SET_FULLSCREEN';

// ── Reducer ───────────────────────────────────────────────────────────────────

function reducer(state, action) {
  switch (action.type) {
    case ADD_TAB: {
      const tabs = [...state.tabs, action.tab];
      return { tabs, activeIndex: tabs.length - 1 };
    }
    case CLOSE_TAB: {
      const tabs = state.tabs.filter((_, i) => i !== action.index);
      const activeIndex = Math.min(
        state.activeIndex >= action.index ? state.activeIndex - 1 : state.activeIndex,
        tabs.length - 1,
      );
      return { tabs, activeIndex };
    }
    case ACTIVATE_TAB: {
      return { ...state, activeIndex: action.index };
    }
    case SET_FULLSCREEN: {
      return { ...state, fullscreen: action.fullscreen };
    }
    default:
      return state;
  }
}

// ── Store ─────────────────────────────────────────────────────────────────────

export const appStore = store(reducer, { tabs: [], activeIndex: -1, fullscreen: false });

// ── Helpers ───────────────────────────────────────────────────────────────────

export function addTab(authority, masl) {
  appStore.send({ type: ADD_TAB, tab: { authority, masl } });
}

export function closeTab(index) {
  appStore.send({ type: CLOSE_TAB, index });
}

export function activateTab(index) {
  appStore.send({ type: ACTIVATE_TAB, index });
}

export function setFullscreen(fullscreen) {
  appStore.send({ type: SET_FULLSCREEN, fullscreen });
}
