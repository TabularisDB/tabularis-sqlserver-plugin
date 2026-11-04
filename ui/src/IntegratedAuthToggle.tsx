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
  const ntlm = c.extra?.ntlm === "true";
  const setCredentialFieldsHidden = c.setCredentialFieldsHidden;

  // Also runs on mount: a saved connection arrives with extra.integrated_auth
  // already set and never fires onChange.
  useEffect(() => {
    setCredentialFieldsHidden?.(checked);
  }, [checked, setCredentialFieldsHidden]);

  // The two modes exclude each other; the backend rejects both at once.
  const select = (key: "integrated_auth" | "ntlm", other: string, on: boolean) => {
    c.setExtraField?.(key, on ? "true" : "");
    if (on) c.setExtraField?.(other, "");
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
      <label style={labelStyle}>
        <input
          type="checkbox"
          checked={checked}
          onChange={(e) => select("integrated_auth", "ntlm", e.target.checked)}
        />
        Use Windows Authentication
      </label>
      <label style={labelStyle}>
        <input
          type="checkbox"
          checked={ntlm}
          onChange={(e) => select("ntlm", "integrated_auth", e.target.checked)}
        />
        Use NTLM (username as DOMAIN\user)
      </label>
    </div>
  );
}

const labelStyle = { display: "flex", alignItems: "center", gap: 8, cursor: "pointer" };
