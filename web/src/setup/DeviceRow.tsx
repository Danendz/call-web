import type { ReactNode } from "react";
import type { DeviceOption, MediaAccess } from "../call/media";
import { useT } from "../ui/i18n";
import { Spinner } from "../ui/Spinner";

/** Device picker, or why the device can't be used. */
export function DeviceRow(props: {
  name: string;
  icon: ReactNode;
  access: MediaAccess;
  devices: DeviceOption[];
  selected: string | null;
  /** The device being opened: the list already shows it, with a spinner for the icon. */
  opening?: string | null;
  onSelect: (deviceId: string) => void;
}) {
  const t = useT();
  const granted = props.access === "granted";
  return (
    <div className={`device-row ${granted ? "" : props.access === "pending" ? "pending" : "problem"}`}>
      <span className="row-icon" aria-hidden>
        {props.opening ? <Spinner size={18} /> : props.icon}
      </span>
      {granted ? (
        <select
          value={props.opening ?? props.selected ?? ""}
          aria-busy={!!props.opening}
          disabled={props.devices.length < 2}
          onChange={(e) => props.onSelect(e.target.value)}
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
