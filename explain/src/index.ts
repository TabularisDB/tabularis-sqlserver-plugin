import { registerExplainParser } from "@tabularis/explain";

import { sqlServerExplainParser } from "./parser";

registerExplainParser(sqlServerExplainParser);

export { sqlServerExplainParser } from "./parser";
export { parseShowplanXml } from "./showplan";
