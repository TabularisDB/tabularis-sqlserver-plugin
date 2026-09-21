import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, test, vi } from "vitest";
import type { SlotComponentProps } from "@tabularis/plugin-api";
import IntegratedAuthToggle from "./IntegratedAuthToggle";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

function mount(extra: Record<string, string>) {
  const setCredentialFieldsHidden = vi.fn();
  const container = document.createElement("div");
  document.body.appendChild(container);
  const context = { extra, setExtraField: vi.fn(), setCredentialFieldsHidden };
  act(() => {
    createRoot(container).render(
      <IntegratedAuthToggle
        context={context as unknown as SlotComponentProps["context"]}
        pluginId="sqlserver"
      />,
    );
  });
  const checkbox = container.querySelector("input")!;
  return { checkbox, setCredentialFieldsHidden };
}

test("hides credential fields for a saved connection without a click", () => {
  const { checkbox, setCredentialFieldsHidden } = mount({ integrated_auth: "true" });
  expect(checkbox.checked).toBe(true);
  expect(setCredentialFieldsHidden).toHaveBeenCalledWith(true);
});

test("leaves credential fields visible when the flag is unset", () => {
  const { checkbox, setCredentialFieldsHidden } = mount({});
  expect(checkbox.checked).toBe(false);
  expect(setCredentialFieldsHidden).toHaveBeenCalledWith(false);
});
