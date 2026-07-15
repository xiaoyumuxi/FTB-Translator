export type View = "workbench" | "history" | "settings";

export type Stage = "idle" | "scanned" | "running" | "review" | "done" | "error";

export type ScanResult = {
  quests_dir: string;
  pack_name: string;
  mode: "lang" | "chapters";
  mode_label: string;
  source: string;
  entry_count: number;
  file_count: number;
  files: { path: string; entry_count: number }[];
  estimated_batches: number;
};

export type Report = {
  source_file: string;
  target_file: string;
  backup_dir: string;
  total_entries: number;
  translated_entries: number;
  cache_hits: number;
  failed_entries: string[];
  warnings: Record<string, string[]>;
  failed_translations: Record<string, { source: string; failed: string; error?: string }>;
};

export type Run = {
  id: number;
  pack_name: string;
  quests_dir: string;
  mode: string;
  model: string;
  style: string;
  total_entries: number;
  translated_entries: number;
  cache_hits: number;
  failed_count: number;
  warning_count: number;
  created_at: string;
};

export type Activity =
  | { type: "message"; message: string }
  | { type: "translation"; entry_id: string; source: string; target: string; status: string };

export type TranslationEvent = {
  type: "progress" | "log" | "translation_preview" | "done" | "review_ready" | "error";
  task_id?: string;
  stage?: string;
  done?: number;
  total?: number;
  message?: string;
  entry_id?: string;
  source?: string;
  target?: string;
  status?: string;
  report?: Report;
  run_id?: number;
  cmp_path?: string;
  total_entries?: number;
  warning_count?: number;
  failed_count?: number;
};

export const note = (message: string): Activity => ({ type: "message", message });
