import type { ReactNode } from "react";
import type { DeviceOption, MediaAccess, MediaKind } from "../call/media";
import { useT } from "../ui/i18n";

/** Device picker, or why the device can't be used. */
export function DeviceRow(props: {
  name: string;
  icon: ReactNode;
  kind: MediaKind;
  access: MediaAccess;
  devices: DeviceOption[];
  selected: string | null;
  onSelect: (kind: MediaKind, deviceId: string) => void;
}) {
  const t = useT();
  const granted = props.access === "granted";
  return (
    <div className={`device-row ${granted ? "" : props.access === "pending" ? "pending" : "problem"}`}>
      <span className="row-icon" aria-hidden>
        {props.icon}
      </span>
      {granted ? (
        <select
          value={props.selected ?? ""}
          disabled={props.devices.length < 2}
          onChange={(e) => props.onSelect(props.kind, e.target.value)}
          aria-label={props.name}
        >
          {props.devices.length === 0 && <option value="">{t.defaultDevice}</option>}
          {props.devices.map((d) => (
            <option key={d.id} value={d.id}>
              {d.label}
            </option>
          ))}
        </select>
      ) : (
        <span className="row-status">{t.deviceStatus(props.name, t.access[props.access])}</span>
      )}
    </div>
  );
}
