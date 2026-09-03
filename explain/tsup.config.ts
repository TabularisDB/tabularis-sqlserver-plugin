import { defineConfig } from "tsup";

const common = {
  platform: "browser" as const,
  target: "es2022" as const,
  sourcemap: false,
  minify: false,
  splitting: false,
  treeshake: true,
  external: ["@tabularis/explain"],
};

export default defineConfig([
  {
    ...common,
    entry: { index: "src/index.ts" },
    format: ["esm"],
    dts: true,
    clean: true,
  },
  {
    ...common,
    entry: { "index.iife": "src/iife.ts" },
    format: ["iife"],
    globalName: "__tabularis_explain_parser__",
    dts: false,
    clean: false,
    outExtension: () => ({ js: ".js" }),
  },
]);
