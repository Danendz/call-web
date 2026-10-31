import type { DeviceOption, MediaAccess, MediaKind } from "../call/media";

export const ACCESS_TEXT: Record<MediaAccess, string> = {
  granted: "",
  pending: "Asking for permission…",
  denied: "Blocked by the browser",
  missing: "Not found",
  busy: "In use by another app",
};

export function DeviceRow(props: {
  name: string;
  kind: MediaKind;
  access: MediaAccess;
  on: boolean;
  devices: DeviceOption[];
  selected: string | null;
  onToggle: () => void;
  onSelect: (kind: MediaKind, deviceId: string) => void;
}) {
  const granted = props.access === "granted";
  return (
    <div className="device-row">
      <button className={granted && !props.on ? "off" : ""} disabled={!granted} onClick={props.onToggle}>
        {props.name} {granted ? (props.on ? "on" : "off") : ""}
      </button>
      {granted ? (
        <select
          value={props.selected ?? ""}
          disabled={props.devices.length < 2}
          onChange={(e) => props.onSelect(props.kind, e.target.value)}
          aria-label={`${props.name} device`}
        >
          {props.devices.length === 0 && <option value="">Default</option>}
          {props.devices.map((d) => (
            <option key={d.id} value={d.id}>
              {d.label}
            </option>
          ))}
        </select>
      ) : (
        <span className={props.access === "pending" ? "muted" : "error"}>{ACCESS_TEXT[props.access]}</span>
      )}
    </div>
  );
}
