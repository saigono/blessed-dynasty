// Stage 11b: worker.js without dependencies, Node 16 or later: `node deploy/stats-worker/test.mjs`.
// `cargo test -p cli` runs it too (crates/cli/tests/stats.rs).
import assert from "node:assert/strict";
import worker from "./worker.js";

// Node before 18 has no Response; the worker needs only its status, body and headers.
globalThis.Response ??= class {
  constructor(body, init) {
    Object.assign(this, { body, status: init.status, headers: init.headers });
  }
};

const ORIGIN = "https://user.github.io";
const GAME = { version: "abc1234", link: "https://user.github.io/bd/#p=AQdkZWZhdWx0", fall: "NoHeir", years: 214, score: 5123 };

/// A request as the worker reads it, and a D1 that keeps what is inserted.
async function send({ body = JSON.stringify(GAME), method = "POST", path = "/game", origin = ORIGIN, length } = {}) {
  const head = { Origin: origin, "Content-Length": String(length ?? Buffer.byteLength(body)) };
  const request = { url: `https://bd-stats.example.workers.dev${path}`, method, headers: new Map(Object.entries(head)), text: async () => body };
  const rows = [];
  const DB = { prepare: (sql) => ({ bind: (...v) => ({ run: async () => rows.push([sql, v]) }) }) };
  const res = await worker.fetch(request, { GAME_ORIGIN: ORIGIN, DB });
  return { res, rows };
}

// A good game is kept, all five fields in order.
let { res, rows } = await send();
assert.equal(res.status, 204);
assert.equal(res.headers["Access-Control-Allow-Origin"], ORIGIN);
assert.deepEqual(rows[0][1], ["abc1234", GAME.link, "NoHeir", 214, 5123]);

// Too large, by the header or by the text when the header lies; broken bodies.
const big = JSON.stringify({ ...GAME, link: "x".repeat(8 * 1024) });
const bad = [
  { body: big },
  { body: big, length: 10 },
  { body: "{not json" },
  { body: "null" },
  { body: "[]" },
  { body: JSON.stringify({ ...GAME, years: "214" }) },
  { body: JSON.stringify({ ...GAME, score: 1.5 }) },
  { body: JSON.stringify({ ...GAME, link: undefined }) },
];
for (const req of bad) {
  ({ res, rows } = await send(req));
  assert.equal(res.status, 400, req.body.slice(0, 40));
  assert.equal(rows.length, 0);
}

// CORS: the game's origin alone; its preflight; nothing but POST /game.
for (const [req, status] of [
  [{ origin: "https://evil.example" }, 403],
  [{ origin: null }, 403],
  [{ method: "OPTIONS" }, 204],
  [{ method: "GET" }, 405],
  [{ path: "/" }, 404],
]) {
  ({ res, rows } = await send(req));
  assert.equal(res.status, status, JSON.stringify(req));
  assert.equal(rows.length, 0);
}
console.log("worker.js: ok");
