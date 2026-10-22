// Automatic toolbar panels are scoped to one task; manual clicks show all tasks.
export function progressPopupPath(taskId: string): string {
  return `popup.html?view=progress&task=${encodeURIComponent(taskId)}`;
}

export function progressTaskId(search: string): string | null {
  const params = new URLSearchParams(search);
  const id = params.get("task");
  return params.get("view") === "progress" && id?.trim() ? id : null;
}
