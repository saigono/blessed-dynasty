// Stage 11b: the receiver of the game statistics, a Cloudflare Worker over a D1 database
// (docs/stats.md). POST /game takes {version, link, fall, years, score} from the game at the
// end of a game and keeps it in the table `games` (schema.sql) with the date.

const MAX_BODY = 8 * 1024;

export default {
  async fetch(request, env) {
    // The game's address alone, e.g. https://user.github.io; unset, every request is refused.
    const origin = env.GAME_ORIGIN;
    const headers = {
      "Access-Control-Allow-Origin": origin,
      "Access-Control-Allow-Methods": "POST",
      "Access-Control-Allow-Headers": "Content-Type",
      Vary: "Origin",
    };
    const reply = (status, text = null) => new Response(text, { status, headers });
    if (new URL(request.url).pathname !== "/game") return reply(404);
    if (!origin || request.headers.get("Origin") !== origin) return reply(403);
    if (request.method === "OPTIONS") return reply(204);
    if (request.method !== "POST") return reply(405);
    // The header may lie or be missing: the text read is checked too.
    if (Number(request.headers.get("Content-Length")) > MAX_BODY) return reply(400, "too large");
    const text = await request.text();
    if (new TextEncoder().encode(text).length > MAX_BODY) return reply(400, "too large");
    let g;
    try {
      g = JSON.parse(text);
    } catch {
      return reply(400, "not JSON");
    }
    const ok =
      g !== null &&
      typeof g === "object" &&
      typeof g.version === "string" &&
      typeof g.link === "string" &&
      typeof g.fall === "string" &&
      Number.isInteger(g.years) &&
      Number.isInteger(g.score);
    if (!ok) return reply(400, "bad fields");
    await env.DB.prepare(
      "INSERT INTO games (version, link, fall, years, score) VALUES (?, ?, ?, ?, ?)",
    )
      .bind(g.version, g.link, g.fall, g.years, g.score)
      .run();
    return reply(204);
  },
};
