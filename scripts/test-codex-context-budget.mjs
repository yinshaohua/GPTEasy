// Offline acceptance: only a loopback fixture is contacted; CODEX_HOME is disposable.
import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import { createServer } from "node:http";
import { mkdir, mkdtemp, readFile, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { createInterface } from "node:readline";

const [codex, catalogPath] = process.argv.slice(2);
assert(codex && catalogPath, "Usage: node scripts/test-codex-context-budget.mjs CODEX CATALOG");
const versionOutput = execFileSync(resolve(codex), ["--version"], { encoding: "utf8", windowsHide: true });
const codexVersion = versionOutput.match(/^codex-cli (\d+\.\d+\.\d+)\s*$/)?.[1];
assert(codexVersion, "expected a versioned Codex executable");
const catalog = JSON.parse(await readFile(catalogPath, "utf8"));
assert(catalog.models.length >= 2, "use the full generated multi-model catalog");
for (const model of catalog.models) {
  assert.equal(model.context_window, 272000);
  assert.equal(model.max_context_window, 872000);
  assert.equal(model.effective_context_window_percent, 95);
  assert.equal(model.auto_compact_token_limit, null);
}
const workspace = await mkdtemp(join(tmpdir(), "gpteasy-context-"));
const children = new Set();
let requests = 0;
const server = createServer(async (request, response) => {
  for await (const chunk of request) { void chunk; }
  requests++;
  const id = "fixture-response-" + requests;
  const item = { id: "fixture-message-" + requests, type: "message", role: "assistant", status: "completed",
    content: [{ type: "output_text", text: "Fixture response.", annotations: [] }] };
  const usage = { input_tokens: requests === 1 ? 18500 : 100, output_tokens: 10,
    total_tokens: requests === 1 ? 18510 : 110, input_tokens_details: { cached_tokens: 0 } };
  response.writeHead(200, { "content-type": "text/event-stream" });
  for (const event of [
    { type: "response.created", response: { id, status: "in_progress", output: [] } },
    { type: "response.output_item.added", output_index: 0, item: { ...item, status: "in_progress", content: [] } },
    { type: "response.output_text.delta", item_id: item.id, output_index: 0, content_index: 0, delta: "Fixture response." },
    { type: "response.output_item.done", output_index: 0, item },
    { type: "response.completed", response: { id, status: "completed", output: [item], usage } },
  ]) response.write("event: " + event.type + "\ndata: " + JSON.stringify(event) + "\n\n");
  response.end();
});
await new Promise((done) => server.listen(0, "127.0.0.1", done));
const port = server.address().port;

async function check(smallWindow) {
  const home = join(workspace, smallWindow ? "small" : "default");
  await mkdir(home);
  const fixture = structuredClone(catalog);
  if (smallWindow) for (const entry of fixture.models) {
    entry.context_window = 20000;
    entry.max_context_window = 20000;
  }
  await writeFile(join(home, "catalog.json"), JSON.stringify(fixture));
  await writeFile(join(home, "config.toml"), [
    "model = " + JSON.stringify(fixture.models[0].slug),
    'model_catalog_json = "catalog.json"',
    'model_provider = "gpteasy_context_fixture"',
    'approval_policy = "never"',
    '[model_providers.gpteasy_context_fixture]',
    'name = "GPTEasy offline fixture"',
    'wire_api = "responses"',
    'base_url = "http://127.0.0.1:' + port + '/v1"',
    'requires_openai_auth = false',
  ].join("\n") + "\n");
  const env = { ...process.env, CODEX_HOME: home, NO_PROXY: "127.0.0.1,localhost", no_proxy: "127.0.0.1,localhost", RUST_LOG: "error" };
  for (const key of ["OPENAI_API_KEY", "CODEX_API_KEY", "CODEX_API_URL", "OPENAI_BASE_URL"]) delete env[key];
  const child = spawn(resolve(codex), ["app-server", "--listen", "stdio://"], {
    cwd: workspace, env, windowsHide: true, stdio: ["pipe", "pipe", "pipe"],
  });
  children.add(child);
  let sequence = 0;
  const pending = new Map();
  const notifications = [];
  // Keep raw process diagnostics private: do not print home paths or environment data.
  child.stderr.resume();
  const lines = createInterface({ input: child.stdout });
  lines.on("line", (line) => {
    let message;
    try { message = JSON.parse(line); } catch { return; }
    if (message.id != null && pending.has(message.id)) {
      const { done, fail, timer } = pending.get(message.id);
      pending.delete(message.id);
      clearTimeout(timer);
      if (message.error) fail(new Error("RPC " + message.error.code)); else done(message.result);
    } else notifications.push(message);
  });
  function rpc(method, params) {
    const id = ++sequence;
    return new Promise((done, fail) => {
      const timer = setTimeout(() => { pending.delete(id); fail(new Error("RPC timeout: " + method)); }, 20000);
      pending.set(id, { done, fail, timer });
      child.stdin.write(JSON.stringify({ id, method, params }) + "\n");
    });
  }
  async function until(predicate, label) {
    const deadline = Date.now() + 25000;
    while (Date.now() < deadline) {
      const message = notifications.find(predicate);
      if (message) return message;
      await new Promise((done) => setTimeout(done, 30));
    }
    throw new Error("notification timeout: " + label + "; methods=" + [...new Set(notifications.map((m) => m.method))].join(","));
  }
  try {
    await rpc("initialize", { clientInfo: { name: "gpteasy_context_acceptance", version: "1" }, capabilities: { experimentalApi: true } });
    child.stdin.write('{"method":"initialized"}\n');
    const listed = await rpc("model/list", { includeHidden: true });
    const ids = listed.data.map((entry) => entry.model || entry.id);
    for (const entry of fixture.models) assert(ids.includes(entry.slug), "catalog fallback: missing " + entry.slug);
    const thread = await rpc("thread/start", { model: fixture.models[0].slug, cwd: workspace, approvalPolicy: "never", sandbox: "read-only", experimentalRawEvents: true });
    const threadId = thread.thread.id;
    requests = 0;
    const turn = await rpc("turn/start", { threadId, input: [{ type: "text", text: "Reply with a brief fixture response." }] });
    await until((m) => m.method === "turn/completed" && m.params.turn.id === turn.turn.id, "first turn");
    const usage = await until((m) => m.method === "thread/tokenUsage/updated", "token count");
    const window = usage.params.tokenUsage.modelContextWindow;
    assert.equal(window, smallWindow ? 19000 : 258400, "effective context window");
    if (smallWindow) {
      const second = await rpc("turn/start", { threadId, input: [{ type: "text", text: "Reply again." }] });
      await until((m) => m.method === "turn/completed" && m.params.turn.id === second.turn.id, "second turn");
      assert(notifications.some((m) => m.method === "item/completed" && m.params.item.type === "contextCompaction"), "automatic compaction was not observed");
      assert(requests >= 3, "expected compact request plus two turns");
    }
    return { codexVersion, case: smallWindow ? "null-auto-compact-small-window" : "generated-default-catalog", modelCount: fixture.models.length, modelContextWindow: window, requests, status: "passed" };
  } finally {
    for (const { timer } of pending.values()) clearTimeout(timer);
    lines.close();
    child.kill();
    await new Promise((done) => { if (child.exitCode !== null || child.signalCode !== null) done(); else child.once("exit", done); });
    children.delete(child);
  }
}
try {
  const results = [];
  for (const small of [false, true]) {
    const result = await check(small);
    results.push(result);
    console.log(JSON.stringify(result));
  }
  if (process.env.GPTEASY_CONTEXT_ACCEPTANCE_REPORT) {
    const destination = resolve(process.env.GPTEASY_CONTEXT_ACCEPTANCE_REPORT);
    await mkdir(destination, { recursive: true });
    await writeFile(join(destination, "catalog.json"), JSON.stringify(catalog, null, 2) + "\n");
    await writeFile(join(destination, "results.json"), JSON.stringify(results, null, 2) + "\n");
  }
} finally {
  for (const child of children) child.kill();
  server.closeAllConnections();
  await new Promise((done) => server.close(done));
  await rm(workspace, { recursive: true, force: true });
}
