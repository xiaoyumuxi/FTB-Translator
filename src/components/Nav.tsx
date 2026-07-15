import type { ReactNode } from "react";

export function Nav({
  active,
  icon,
  label,
  onClick,
  badge,
}: {
  active: boolean;
  icon: ReactNode;
  label: string;
  onClick: () => void;
  badge?: number;
}) {
  return (
    <button className={`nav-item ${active ? "active" : ""}`} onClick={onClick}>
      {icon}
      <span>{label}</span>
      {badge !== undefined && <em>{badge}</em>}
    </button>
  );
}
