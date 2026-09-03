import { readFile } from "node:fs/promises";

import type { ExplainNode, ExplainPlan } from "@tabularis/explain";
import { describe, expect, it } from "vitest";

import { parseShowplanXml } from "../src/showplan";

const fixtureNames = [
  "trivial-scan",
  "index-seek-key-lookup",
  "parallel-hash-join",
  "statistics-xml",
  "missing-index",
  "multi-statement",
] as const;
const fixtureDirectory = new URL("./fixtures/", import.meta.url);

async function readFixture(name: string): Promise<string> {
  return readFile(new URL(`${name}.xml`, fixtureDirectory), "utf8");
}

function flatten(node: ExplainNode): ExplainNode[] {
  return [node, ...node.children.flatMap(flatten)];
}

describe("parseShowplanXml", () => {
  it.each(fixtureNames)("matches the committed SHOWPLAN golden for %s", async (name) => {
    const xml = await readFixture(name);
    const expected = JSON.parse(
      await readFile(new URL(`expected/${name}.json`, fixtureDirectory), "utf8"),
    ) as ExplainPlan;

    expect(parseShowplanXml(xml)).toEqual(expected);
  });

  it("sums rows and executions and takes maximum elapsed time across threads", async () => {
    const plan = parseShowplanXml(await readFixture("parallel-hash-join"));
    const hashJoin = flatten(plan.root).find((node) => node.id === "sqlserver-4");

    expect(hashJoin).toMatchObject({
      node_type: "Hash Match",
      join_type: "Inner Join",
      actual_rows: 900_000,
      actual_loops: 4,
      actual_time_ms: 16,
    });
    expect(plan.execution_time_ms).toBe(36);
    expect(plan.has_analyze_data).toBe(true);
  });

  it("uses only the first statement's first operator", async () => {
    const plan = parseShowplanXml(await readFixture("multi-statement"));

    expect(plan.root).toMatchObject({
      id: "sqlserver-0",
      node_type: "Table Scan",
      relation: "ss034_small",
    });
    expect(plan.root.children).toEqual([]);
  });

  it("keeps missing-index documents parseable without synthesizing model fields", async () => {
    const xml = await readFixture("missing-index");
    const plan = parseShowplanXml(xml);

    expect(xml).toContain("<MissingIndexes>");
    expect(plan.raw_output).toBe(xml);
    expect(plan.root.extra).toEqual({ logical_operation: "Gather Streams" });
  });

  it("is namespace-insensitive and assigns deterministic fallback ids", () => {
    const xml =
      '<sp:ShowPlanXML xmlns:sp="urn:showplan"><sp:RelOp PhysicalOp="Select"><sp:RelOp LogicalOp="Table Scan"/></sp:RelOp></sp:ShowPlanXML>';
    const plan = parseShowplanXml(xml);

    expect(plan.root.id).toBe("sqlserver-0");
    expect(plan.root.children[0]?.id).toBe("sqlserver-1");
    expect(plan.root.children[0]?.node_type).toBe("Table Scan");
  });

  it("retains the established SHOWPLAN error prefixes", () => {
    expect(() => parseShowplanXml("<ShowPlanXML><RelOp></ShowPlanXML>")).toThrowError(
      /^Failed to parse SQL Server SHOWPLAN_XML:/,
    );
    expect(() => parseShowplanXml("<ShowPlanXML/>")).toThrowError(
      "SQL Server SHOWPLAN_XML does not contain a RelOp",
    );
  });
});
