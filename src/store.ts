import { invoke } from "@tauri-apps/api/core";
import { create } from "zustand";

import type { HistoryItem } from "./types";

interface HistoryStore {
  items: HistoryItem[];
  isLoading: boolean;
  error: string | null;
  fetchHistory: () => Promise<void>;
  captureScreenshot: () => Promise<void>;
  deleteItem: (id: number) => Promise<void>;
  clearHistory: () => Promise<void>;
}

export const useHistoryStore = create<HistoryStore>((set) => ({
  items: [],
  isLoading: false,
  error: null,

  fetchHistory: async () => {
    set({ isLoading: true, error: null });
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
