import type { SlotComponentProps } from "@tabularis/plugin-api";

// `credentialFieldsHidden` / `setCredentialFieldsHidden` are optional here so
// this keeps working (checkbox visible, login inputs untouched) on a host
// that predates that hook, and typechecks even before a plugin-api release
// that types them lands. See TabularisDB/tabularis#780.
interface ExtraFieldsContext {
  extra?: Record<string, string>;
  setExtraField?: (key: string, value: string) => void;
  credentialFieldsHidden?: boolean;
  setCredentialFieldsHidden?: (hidden: boolean) => void;
}

export default function IntegratedAuthToggle({ context }: SlotComponentProps) {
  const c = context as ExtraFieldsContext;
  const checked = c.extra?.integrated_auth === "true";

  return (
    <label style={{ display: "flex", alignItems: "center", gap: 8, cursor: "pointer" }}>
      <input
        type="checkbox"
        checked={checked}
        onChange={(e) => {
          const next = e.target.checked;
          c.setExtraField?.("integrated_auth", next ? "true" : "");
          c.setCredentialFieldsHidden?.(next);
        }}
      />
      Use Windows Authentication
    </label>
  );
}
