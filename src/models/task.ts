import type { TaskState } from "./cmp";

export type ActiveTask = {
  task_id: string;
  state: TaskState;
  updated_at: string;
  recoverable: boolean;
};
