export type HistoryItemType = "text" | "image";

export interface HistoryItem {
  id: number;
  type: HistoryItemType;
  content: string;
  thumbnail: string | null;
  created_at: string;
}
