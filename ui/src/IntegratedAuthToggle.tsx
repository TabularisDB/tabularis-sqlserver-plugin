import { useEffect } from "react";
import type { SlotComponentProps } from "@tabularis/plugin-api";

// Optional: falls back gracefully on hosts predating TabularisDB/tabularis#780.
interface ExtraFieldsContext {
  extra?: Record<string, string>;
  setExtraField?: (key: string, value: string) => void;
  credentialFieldsHidden?: boolean;
  setCredentialFieldsHidden?: (hidden: boolean) => void;
}

export default function IntegratedAuthToggle({ context }: SlotComponentProps) {
  const c = context as ExtraFieldsContext;
  const checked = c.extra?.integrated_auth === "true";
  const setCredentialFieldsHidden = c.setCredentialFieldsHidden;

  // Also runs on mount: a saved connection arrives with extra.integrated_auth
  // already set and never fires onChange.
  useEffect(() => {
    setCredentialFieldsHidden?.(checked);
  }, [checked, setCredentialFieldsHidden]);

  return (
    <label style={{ display: "flex", alignItems: "center", gap: 8, cursor: "pointer" }}>
      <input
        type="checkbox"
        checked={checked}
        onChange={(e) => {
          c.setExtraField?.("integrated_auth", e.target.checked ? "true" : "");
        }}
      />
      Use Windows Authentication
    </label>
  );
}
