// Node loader hook: resolves the bare `sdk` specifier (used by plugin
// modules) to the in-memory mock, and forces ESM semantics for the plugin's
// `.js` sources (they use `import/export` but have a `.js` extension).
//
// Usage:
//   node --import ./test/loader.mjs --test test/*.test.mjs
// (Node ≥ 20.6 for `--import`; hooks run before application modules load.)

import { fileURLToPath } from "node:url";
import path from "node:path";
import { register } from "node:module";

const mockUrl = new URL("./sdk-mock.mjs", import.meta.url).href;
const root = path.resolve(fileURLToPath(new URL("..", import.meta.url)));

// Register this file's resolve/load hooks (Node ≥ 20.6).
register(new URL(import.meta.url), { parentURL: import.meta.url });

export async function resolve(specifier, context, next) {
  if (specifier === "sdk") {
    return { url: mockUrl, shortCircuit: true };
  }
  return next(specifier, context);
}

export async function load(url, context, next) {
  // The plugin's own `.js` files are ESM-in-practice (import/export) but the
  // loader only knows `.mjs` as ESM by default. Mark everything under the
  // plugin root as ESM so `import/export` parses.
  if (url.startsWith("file:") && !url.endsWith(".mjs")) {
    const p = fileURLToPath(url);
    if (p.startsWith(root)) {
      return { ...(await next(url, { ...context, format: "module" })), format: "module" };
    }
  }
  return next(url, context);
}
