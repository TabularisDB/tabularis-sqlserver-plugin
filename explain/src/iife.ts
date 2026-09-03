import { sqlServerExplainParser } from "./parser";

// The desktop loader registers this descriptor after matching it to the
// plugin manifest. This entry intentionally has no registration side effect.
export default sqlServerExplainParser;
