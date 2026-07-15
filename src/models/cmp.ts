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

export type CmpValidationReport = {
  belongs_to_current_task_book: boolean;
  source_fingerprint_matches: boolean;
  /** 可安全应用的条目数，包含有意保持英文的条目。 */
  applicable_entries: number;
  format_guard_failures: number;
  unchanged_entries: number;
  files_to_modify: string[];
  blocking: boolean;
  blocking_issues: string[];
};
