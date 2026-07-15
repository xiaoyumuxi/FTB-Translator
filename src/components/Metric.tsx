export function Metric({ label, value, warn = false }: { label: string; value: number; warn?: boolean }) {
  return (
    <div className="metric">
      <span>{label}</span>
      <strong className={warn ? "warn" : ""}>{value}</strong>
    </div>
  );
}
