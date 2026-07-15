export function QuestMark({ compact = false }: { compact?: boolean }) {
  return (
    <div className={`brand-mark ${compact ? "compact" : ""}`} aria-hidden="true">
      <span />
      <span />
      <span />
    </div>
  );
}
