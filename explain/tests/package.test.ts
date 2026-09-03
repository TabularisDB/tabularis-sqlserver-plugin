import { readFile } from "node:fs/promises";

import {
  registrations,
  type RegisteredExplainParser,
} from "@tabularis/explain";
import { describe, expect, it } from "vitest";

import {
  parseShowplanXml,
  sqlServerExplainParser,
} from "../src/index";

const fixtureUrl = new URL("./fixtures/trivial-scan.xml", import.meta.url);

describe("package entry points", () => {
  it("registers on ESM import and exports the direct parser API", async () => {
    expect(registrations).toEqual([sqlServerExplainParser]);
    expect(sqlServerExplainParser).toMatchObject({
      engine: "sqlserver",
      format: "sqlserver-showplan-xml",
      label: "SQL Server SHOWPLAN XML",
      parse: parseShowplanXml,
    });
    expect(sqlServerExplainParser.sniff?.(await readFile(fixtureUrl, "utf8"))).toBe(true);
    expect(sqlServerExplainParser.sniff?.("<not-a-showplan/>")).toBe(false);
  });

  it("builds an isolated IIFE descriptor without self-registration", async () => {
    const source = await readFile(
      new URL("../dist/index.iife.js", import.meta.url),
      "utf8",
    );
    const registrationsBeforeEvaluation = registrations.length;
    const evaluate = new Function(
      "__TABULARIS_EXPLAIN__",
      `${source}\nreturn typeof __tabularis_explain_parser__ !== "undefined" ? __tabularis_explain_parser__ : null;`,
    );
    const raw = evaluate({}) as Record<string, unknown>;
    const descriptor = (raw.default ?? raw) as RegisteredExplainParser;

    expect(descriptor).toMatchObject({
      engine: "sqlserver",
      format: "sqlserver-showplan-xml",
      label: "SQL Server SHOWPLAN XML",
    });
    expect(descriptor.parse(await readFile(fixtureUrl, "utf8")).driver).toBe("sqlserver");
    expect(registrations).toHaveLength(registrationsBeforeEvaluation);
  });
});
