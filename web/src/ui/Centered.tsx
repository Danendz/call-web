import type { ReactNode } from "react";

export function Centered({ children }: { children: ReactNode }) {
  return <div className="centered">{children}</div>;
}
