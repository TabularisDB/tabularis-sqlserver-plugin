import type { ExplainNode, ExplainPlan } from "@tabularis/explain";

interface XmlElement {
  readonly qualifiedName: string;
  readonly name: string;
  readonly attributes: ReadonlyMap<string, string>;
  readonly children: XmlElement[];
}

class XmlParseError extends Error {}

function localName(qualifiedName: string): string {
  const separator = qualifiedName.lastIndexOf(":");
  return separator === -1 ? qualifiedName : qualifiedName.slice(separator + 1);
}

function decodeXmlEntities(value: string): string {
  let decoded = "";
  let position = 0;
  while (position < value.length) {
    const ampersand = value.indexOf("&", position);
    if (ampersand === -1) return decoded + value.slice(position);
    decoded += value.slice(position, ampersand);

    const semicolon = value.indexOf(";", ampersand + 1);
    if (semicolon === -1) throw new XmlParseError("unterminated XML entity");
    const entity = value.slice(ampersand + 1, semicolon);
    switch (entity) {
      case "amp":
        decoded += "&";
        break;
      case "lt":
        decoded += "<";
        break;
      case "gt":
        decoded += ">";
        break;
      case "quot":
        decoded += '"';
        break;
      case "apos":
        decoded += "'";
        break;
      default: {
        const numeric = /^#x[0-9a-f]+$/i.test(entity)
          ? Number.parseInt(entity.slice(2), 16)
          : /^#[0-9]+$/.test(entity)
            ? Number.parseInt(entity.slice(1), 10)
            : Number.NaN;
        if (
          !Number.isInteger(numeric) ||
          numeric <= 0 ||
          numeric > 0x10ffff ||
          (numeric >= 0xd800 && numeric <= 0xdfff)
        ) {
          throw new XmlParseError(`invalid XML entity &${entity};`);
        }
        decoded += String.fromCodePoint(numeric);
      }
    }
    position = semicolon + 1;
  }
  return decoded;
}

function findTagEnd(xml: string, start: number): number {
  let quote: string | null = null;
  for (let index = start; index < xml.length; index += 1) {
    const character = xml[index];
    if (quote !== null) {
      if (character === quote) quote = null;
    } else if (character === '"' || character === "'") {
      quote = character;
    } else if (character === ">") {
      return index;
    }
  }
  throw new XmlParseError("unterminated XML tag");
}

function parseStartTag(source: string): {
  qualifiedName: string;
  attributes: Map<string, string>;
  selfClosing: boolean;
} {
  let end = source.length;
  while (end > 0 && /\s/.test(source[end - 1] ?? "")) end -= 1;
  const selfClosing = source[end - 1] === "/";
  if (selfClosing) {
    end -= 1;
    while (end > 0 && /\s/.test(source[end - 1] ?? "")) end -= 1;
  }

  let index = 0;
  while (index < end && /\s/.test(source[index] ?? "")) index += 1;
  const nameStart = index;
  while (index < end && !/[\s/=]/.test(source[index] ?? "")) index += 1;
  const qualifiedName = source.slice(nameStart, index);
  if (!/^[A-Za-z_][\w.:-]*$/.test(qualifiedName)) {
    throw new XmlParseError(`invalid element name '${qualifiedName}'`);
  }

  const attributes = new Map<string, string>();
  while (index < end) {
    while (index < end && /\s/.test(source[index] ?? "")) index += 1;
    if (index >= end) break;

    const attributeStart = index;
    while (index < end && !/[\s=]/.test(source[index] ?? "")) index += 1;
    const attributeName = source.slice(attributeStart, index);
    if (!/^[A-Za-z_][\w.:-]*$/.test(attributeName)) {
      throw new XmlParseError(`invalid attribute name '${attributeName}'`);
    }
    while (index < end && /\s/.test(source[index] ?? "")) index += 1;
    if (source[index] !== "=") {
      throw new XmlParseError(`attribute '${attributeName}' has no value`);
    }
    index += 1;
    while (index < end && /\s/.test(source[index] ?? "")) index += 1;

    const quote = source[index];
    if (quote !== '"' && quote !== "'") {
      throw new XmlParseError(`attribute '${attributeName}' is not quoted`);
    }
    index += 1;
    const valueStart = index;
    while (index < end && source[index] !== quote) index += 1;
    if (index >= end) {
      throw new XmlParseError(`unterminated attribute '${attributeName}'`);
    }
    if (attributes.has(attributeName)) {
      throw new XmlParseError(`duplicate attribute '${attributeName}'`);
    }
    const rawValue = source.slice(valueStart, index);
    if (rawValue.includes("<")) {
      throw new XmlParseError(`attribute '${attributeName}' contains '<'`);
    }
    attributes.set(localName(attributeName), decodeXmlEntities(rawValue));
    index += 1;
  }

  return { qualifiedName, attributes, selfClosing };
}

/**
 * Parse the XML subset used by SQL Server SHOWPLAN without Node built-ins.
 * The parser validates tag nesting and attributes and is shared by browser and
 * server-side consumers, avoiding a runtime XML dependency in either bundle.
 */
function parseXml(xml: string): XmlElement {
  let root: XmlElement | null = null;
  let position = 0;
  const stack: XmlElement[] = [];

  while (position < xml.length) {
    const tagStart = xml.indexOf("<", position);
    const textEnd = tagStart === -1 ? xml.length : tagStart;
    const text = xml.slice(position, textEnd);
    if (stack.length === 0 && text.trim() !== "") {
      throw new XmlParseError("text outside the document element");
    }
    decodeXmlEntities(text);
    if (tagStart === -1) break;

    if (xml.startsWith("<!--", tagStart)) {
      const end = xml.indexOf("-->", tagStart + 4);
      if (end === -1) throw new XmlParseError("unterminated XML comment");
      if (xml.slice(tagStart + 4, end).includes("--")) {
        throw new XmlParseError("invalid '--' inside XML comment");
      }
      position = end + 3;
      continue;
    }
    if (xml.startsWith("<![CDATA[", tagStart)) {
      if (stack.length === 0) throw new XmlParseError("CDATA outside the document element");
      const end = xml.indexOf("]]>", tagStart + 9);
      if (end === -1) throw new XmlParseError("unterminated CDATA section");
      position = end + 3;
      continue;
    }
    if (xml.startsWith("<?", tagStart)) {
      const end = xml.indexOf("?>", tagStart + 2);
      if (end === -1) throw new XmlParseError("unterminated processing instruction");
      position = end + 2;
      continue;
    }
    if (xml.startsWith("<!", tagStart)) {
      throw new XmlParseError("unsupported XML declaration");
    }

    const tagEnd = findTagEnd(xml, tagStart + 1);
    const tag = xml.slice(tagStart + 1, tagEnd);
    if (tag.startsWith("/")) {
      const qualifiedName = tag.slice(1).trim();
      if (!/^[A-Za-z_][\w.:-]*$/.test(qualifiedName)) {
        throw new XmlParseError(`invalid closing tag '${qualifiedName}'`);
      }
      const open = stack.pop();
      if (open === undefined || open.qualifiedName !== qualifiedName) {
        throw new XmlParseError(`unexpected closing tag '${qualifiedName}'`);
      }
    } else {
      const parsed = parseStartTag(tag);
      const element: XmlElement = {
        qualifiedName: parsed.qualifiedName,
        name: localName(parsed.qualifiedName),
        attributes: parsed.attributes,
        children: [],
      };
      const parent = stack[stack.length - 1];
      if (parent === undefined) {
        if (root !== null) throw new XmlParseError("multiple document elements");
        root = element;
      } else {
        parent.children.push(element);
      }
      if (!parsed.selfClosing) stack.push(element);
    }
    position = tagEnd + 1;
  }

  if (stack.length !== 0) {
    throw new XmlParseError(`unclosed element '${stack[stack.length - 1]?.qualifiedName ?? ""}'`);
  }
  if (root === null) throw new XmlParseError("document has no root element");
  return root;
}

function firstDescendant(element: XmlElement, name: string): XmlElement | null {
  if (element.name === name) return element;
  for (const child of element.children) {
    const found = firstDescendant(child, name);
    if (found !== null) return found;
  }
  return null;
}

function ownedDescendant(operator: XmlElement, name: string): XmlElement | null {
  for (const child of operator.children) {
    if (child.name === "RelOp") continue;
    if (child.name === name) return child;
    const found = ownedDescendant(child, name);
    if (found !== null) return found;
  }
  return null;
}

function childOperators(operator: XmlElement): XmlElement[] {
  const operators: XmlElement[] = [];
  const visit = (element: XmlElement): void => {
    for (const child of element.children) {
      if (child.name === "RelOp") operators.push(child);
      else visit(child);
    }
  };
  visit(operator);
  return operators;
}

function attributeNumber(element: XmlElement, name: string): number | null {
  const text = element.attributes.get(name);
  if (text === undefined || text.trim() === "") return null;
  const value = Number(text);
  return Number.isFinite(value) ? value : null;
}

function runtimeMetrics(operator: XmlElement): {
  actualRows: number | null;
  actualTimeMs: number | null;
  actualLoops: number | null;
} {
  const runtime = ownedDescendant(operator, "RunTimeInformation");
  if (runtime === null) {
    return { actualRows: null, actualTimeMs: null, actualLoops: null };
  }
  const counters = runtime.children.filter(
    (child) => child.name === "RunTimeCountersPerThread",
  );
  if (counters.length === 0) {
    return { actualRows: null, actualTimeMs: null, actualLoops: null };
  }
  const metric = (counter: XmlElement, name: string): number =>
    attributeNumber(counter, name) ?? 0;

  return {
    actualRows: counters.reduce((sum, counter) => sum + metric(counter, "ActualRows"), 0),
    actualTimeMs: Math.max(...counters.map((counter) => metric(counter, "ActualElapsedms"))),
    actualLoops: counters.reduce(
      (sum, counter) => sum + metric(counter, "ActualExecutions"),
      0,
    ),
  };
}

function parseOperator(operator: XmlElement, fallbackId: number): ExplainNode {
  const physical =
    operator.attributes.get("PhysicalOp") ??
    operator.attributes.get("LogicalOp") ??
    "Unknown";
  const logical = operator.attributes.get("LogicalOp") ?? physical;
  const runtime = runtimeMetrics(operator);
  const object = ownedDescendant(operator, "Object");
  const scalar = ownedDescendant(operator, "ScalarOperator");
  const children = childOperators(operator).map((child, index) =>
    parseOperator(child, fallbackId * 10 + index + 1),
  );

  return {
    id: `sqlserver-${operator.attributes.get("NodeId") ?? String(fallbackId)}`,
    node_type: physical,
    relation: object?.attributes.get("Table")?.replace(/[\[\]]/g, "") ?? null,
    startup_cost: null,
    total_cost: attributeNumber(operator, "EstimatedTotalSubtreeCost"),
    plan_rows: attributeNumber(operator, "EstimateRows"),
    actual_rows: runtime.actualRows,
    actual_time_ms: runtime.actualTimeMs,
    actual_loops: runtime.actualLoops,
    buffers_hit: null,
    buffers_read: null,
    filter: scalar?.attributes.get("ScalarString") ?? null,
    index_condition: null,
    join_type: logical.toLowerCase().includes("join") ? logical : null,
    hash_condition: null,
    extra: { logical_operation: logical },
    children,
  };
}

/** Parse SQL Server SHOWPLAN XML into the shared Tabularis visual-plan model. */
export function parseShowplanXml(xml: string): ExplainPlan {
  let document: XmlElement;
  try {
    document = parseXml(xml);
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    throw new Error(`Failed to parse SQL Server SHOWPLAN_XML: ${message}`);
  }

  const operator = firstDescendant(document, "RelOp");
  if (operator === null) {
    throw new Error("SQL Server SHOWPLAN_XML does not contain a RelOp");
  }
  const root = parseOperator(operator, 0);

  return {
    root,
    planning_time_ms: null,
    execution_time_ms: root.actual_time_ms,
    original_query: "",
    driver: "sqlserver",
    has_analyze_data: root.actual_rows !== null,
    raw_output: xml,
  };
}
