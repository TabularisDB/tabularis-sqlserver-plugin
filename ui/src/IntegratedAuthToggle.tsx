import { useEffect } from "react";
import type { SlotComponentProps } from "@tabularis/plugin-api";

// Optional: falls back gracefully on hosts predating TabularisDB/tabularis#780.
interface ExtraFieldsContext {
  extra?: Record<string, string>;
  setExtraField?: (key: string, value: string) => void;
  credentialFieldsHidden?: boolean;
  setCredentialFieldsHidden?: (hidden: boolean) => void;
}

type AuthMode = "integrated_auth" | "ntlm";

interface ModeCheckboxProps {
  label: string;
  checked: boolean;
  onToggle: (on: boolean) => void;
}

function ModeCheckbox({ label, checked, onToggle }: ModeCheckboxProps) {
  return (
    <label style={{ display: "flex", alignItems: "center", gap: 8, cursor: "pointer" }}>
      <input type="checkbox" checked={checked} onChange={(e) => onToggle(e.target.checked)} />
      {label}
    </label>
  );
}

export default function IntegratedAuthToggle({ context }: SlotComponentProps) {
  const c = context as ExtraFieldsContext;
  const integratedAuth = c.extra?.integrated_auth === "true";
  const ntlm = c.extra?.ntlm === "true";
  const setCredentialFieldsHidden = c.setCredentialFieldsHidden;

  // Also runs on mount: a saved connection arrives with extra.integrated_auth
  // already set and never fires onChange.
  useEffect(() => {
    setCredentialFieldsHidden?.(integratedAuth);
  }, [integratedAuth, setCredentialFieldsHidden]);

  // The two modes exclude each other; the backend rejects both at once.
  const selectMode = (mode: AuthMode, other: AuthMode, on: boolean) => {
    c.setExtraField?.(mode, on ? "true" : "");
    if (on) c.setExtraField?.(other, "");
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
      <ModeCheckbox
        label="Use Windows Authentication"
        checked={integratedAuth}
        onToggle={(on) => selectMode("integrated_auth", "ntlm", on)}
      />
      <ModeCheckbox
        label="Use NTLM (username as DOMAIN\user)"
        checked={ntlm}
        onToggle={(on) => selectMode("ntlm", "integrated_auth", on)}
      />
    </div>
  );
}
