import { invoke } from "@tauri-apps/api/core";
import { create } from "zustand";

import type { HistoryItem } from "./types";

// ── Settings store ──────────────────────────────────────────────────────────

interface SettingsStore {
  captureShortcut: string;
  isSaving: boolean;
  error: string | null;
  fetchShortcut: () => Promise<void>;
  updateShortcut: (shortcut: string) => Promise<void>;
}

export const useSettingsStore = create<SettingsStore>((set) => ({
  captureShortcut: "Ctrl+Shift+S",
  isSaving: false,
  error: null,

  fetchShortcut: async () => {
    set({ error: null });
    try {
      const s = await invoke<string>("get_capture_shortcut");
      set({ captureShortcut: s });
    } catch (err) {
      set({ error: String(err) });
    }
  },

  updateShortcut: async (shortcut: string) => {
    set({ isSaving: true, error: null });
    try {
      await invoke("update_capture_shortcut", { shortcut });
      set({ captureShortcut: shortcut, isSaving: false });
    } catch (err) {
      set({ error: String(err), isSaving: false });
    }
  },
}));

interface HistoryStore {
  items: HistoryItem[];
  isLoading: boolean;
  error: string | null;
  fetchHistory: () => Promise<void>;
  captureScreenshot: () => Promise<void>;
  deleteItem: (id: number) => Promise<void>;
  clearHistory: () => Promise<void>;
}

export const useHistoryStore = create<HistoryStore>((set, get) => ({
  items: [],
  isLoading: true,
  error: null,

  fetchHistory: async () => {
    // Si ya tenemos items, refrescamos en background sin mostrar loading
    const hasItems = get().items.length > 0;
    if (!hasItems) set({ isLoading: true, error: null });
    try {
      const items = await invoke<HistoryItem[]>("get_history", { limit: 100 });
      set({ items, isLoading: false });
    } catch (err) {
      set({ error: String(err), isLoading: false });
    }
  },

  captureScreenshot: async () => {
    // Dispara el overlay de selección; el resultado llega via evento "history-updated"
    try {
      await invoke("start_area_capture");
    } catch (err) {
      set({ error: String(err) });
    }
  },

  deleteItem: async (id: number) => {
    try {
      await invoke("delete_history_item", { id });
      set((state) => ({ items: state.items.filter((i) => i.id !== id) }));
    } catch (err) {
      set({ error: String(err) });
    }
  },

  clearHistory: async () => {
    try {
      await invoke("clear_history");
      set({ items: [] });
    } catch (err) {
      set({ error: String(err) });
    }
  },
}));
