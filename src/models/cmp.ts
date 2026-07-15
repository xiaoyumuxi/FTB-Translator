export type CmpDraft = {
  cmp_path: string;
  task_id?: string;
  total_entries: number;
  warning_count: number;
  failed_count: number;
};

export type CmpEntry = {
  index: number;
  entry_id: string;
  path: string;
  file: string;
  source: string;
  target: string;
  status: string;
};
