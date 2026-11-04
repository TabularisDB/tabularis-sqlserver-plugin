import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, test, vi } from "vitest";
import type { SlotComponentProps } from "@tabularis/plugin-api";
import IntegratedAuthToggle from "./IntegratedAuthToggle";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

function mount(extra: Record<string, string>) {
  const setCredentialFieldsHidden = vi.fn();
  const setExtraField = vi.fn();
  const container = document.createElement("div");
  document.body.appendChild(container);
  const context = { extra, setExtraField, setCredentialFieldsHidden };
  act(() => {
    createRoot(container).render(
      <IntegratedAuthToggle
        context={context as unknown as SlotComponentProps["context"]}
        pluginId="sqlserver"
      />,
    );
  });
  const inputs = container.querySelectorAll("input");
  const checkbox = inputs[0];
  const ntlmCheckbox = inputs[1];
  return { checkbox, ntlmCheckbox, setCredentialFieldsHidden, setExtraField };
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

test("NTLM keeps credential fields visible for DOMAIN\\user and password", () => {
  const { checkbox, ntlmCheckbox, setCredentialFieldsHidden } = mount({ ntlm: "true" });
  expect(ntlmCheckbox.checked).toBe(true);
  expect(checkbox.checked).toBe(false);
  expect(setCredentialFieldsHidden).toHaveBeenCalledWith(false);
});

test("checking NTLM clears Windows Authentication", () => {
  const { ntlmCheckbox, setExtraField } = mount({ integrated_auth: "true" });
  act(() => ntlmCheckbox.click());
  expect(setExtraField).toHaveBeenCalledWith("ntlm", "true");
  expect(setExtraField).toHaveBeenCalledWith("integrated_auth", "");
});

test("checking Windows Authentication clears NTLM", () => {
  const { checkbox, setExtraField } = mount({ ntlm: "true" });
  act(() => checkbox.click());
  expect(setExtraField).toHaveBeenCalledWith("integrated_auth", "true");
  expect(setExtraField).toHaveBeenCalledWith("ntlm", "");
});
