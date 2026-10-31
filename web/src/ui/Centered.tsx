import type { ReactNode } from "react";

export function Centered({ icon, children }: { icon?: ReactNode; children: ReactNode }) {
  return (
    <div className="centered">
      {icon && <div className="centered-icon">{icon}</div>}
      {children}
    </div>
  );
}
