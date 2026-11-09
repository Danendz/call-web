import { LoaderCircle } from "lucide-react";

/** Something is on its way: a device opening. */
export function Spinner({ size }: { size: number }) {
  return <LoaderCircle size={size} className="spinner" aria-hidden />;
}
