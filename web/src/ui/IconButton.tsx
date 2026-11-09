import type { ReactNode } from "react";

export type IconButtonVariant = "active" | "off" | "danger" | "small";

/** Round icon-only button. The label is both the screen-reader name and the hover tooltip. */
export function IconButton({
  label,
  variant,
  pressed,
  expanded,
  badge,
  mark,
  disabled,
  className = "",
  onClick,
  children,
}: {
  label: string;
  variant?: IconButtonVariant | null;
  pressed?: boolean;
  expanded?: boolean;
  badge?: number;
  /** A small icon on the corner: a state the button's own icon doesn't show. */
  mark?: ReactNode;
  disabled?: boolean;
  className?: string;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      className={`icon-btn ${variant ?? ""} ${className}`}
      aria-label={label}
      aria-pressed={pressed}
      aria-expanded={expanded}
      title={label}
      disabled={disabled}
      onClick={onClick}
    >
      {children}
      {badge ? (
        <span className="badge" key={badge} aria-hidden>
          {badge > 9 ? "9+" : badge}
        </span>
      ) : null}
      {mark && !badge ? (
        <span className="mark" aria-hidden>
          {mark}
        </span>
      ) : null}
    </button>
  );
}
