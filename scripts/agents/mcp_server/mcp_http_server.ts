/**
 * Local MCP streamable-HTTP test server (TypeScript + hono, run with bun).
 * Serves the minimal tools subset our raisfast HTTP MCP client uses and
 * always replies with JSON (no SSE), so the client's JSON mode can be
 * exercised end-to-end.
 *
 * Run:  bun run scripts/agents/mcp_http_server.ts   (default :9899, PORT to override)
 * Deps: bun add hono   (run in repo root)
 */
import { Hono } from "hono";

const app = new Hono();
const port = Number(process.env.PORT ?? 9899);

function json(id: number | null, result?: unknown, error?: unknown) {
  return {
    jsonrpc: "2.0",
    id,
    ...(error !== undefined ? { error } : { result }),
  };
}

app.post("/", async (c) => {
  let req: any;
  try {
    req = await c.req.json();
  } catch {
    return c.json(json(null, undefined, { code: -32700, message: "parse error" }), 400);
  }
  const id = req?.id ?? null;
  const method: string = req?.method ?? "";
  const params: any = req?.params ?? {};

  if (method === "initialize") {
    // Echo a session id so the client's session header handling is exercised.
    return c.json(
      json(id, {
        protocolVersion: params.protocolVersion ?? "2026-07-28",
        capabilities: { tools: {} },
        serverInfo: { name: "hono-echo", version: "1.0" },
      }),
      200,
      { "mcp-session-id": "sess-123" },
    );
  }
  if (method === "tools/list") {
    return c.json(
      json(id, {
        tools: [
          {
            name: "echo",
            description: "Echoes back the msg argument",
            inputSchema: {
              type: "object",
              properties: { msg: { type: "string" } },
              required: ["msg"],
            },
          },
        ],
      }),
    );
  }
  if (method === "tools/call") {
    const args = params.arguments ?? {};
    const text =
      params.name === "echo"
        ? `echo:${String(args.msg ?? "")}`
        : `unknown tool ${params.name}`;
    return c.json(
      json(id, { content: [{ type: "text", text }], isError: false }),
    );
  }
  return c.json(json(id, undefined, { code: -32601, message: "not implemented" }), 404);
});

export default {
  port,
  fetch: app.fetch,
};
