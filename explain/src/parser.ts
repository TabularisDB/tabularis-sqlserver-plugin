import type { RegisteredExplainParser } from "@tabularis/explain";

import { parseShowplanXml } from "./showplan";

/** SQL Server parser descriptor consumed by both package and plugin loaders. */
export const sqlServerExplainParser: RegisteredExplainParser = {
  engine: "sqlserver",
  format: "sqlserver-showplan-xml",
  label: "SQL Server SHOWPLAN XML",
  parse: parseShowplanXml,
  sniff: (payload) =>
    /<(?:\w+:)?ShowPlanXML(?:\s|>)/.test(payload.slice(0, 4096)),
};
