/* Petopia World — files a published home.
   Run by .github/workflows/petopia-world.yml when an issue titled
   "[world] …" (publish) or "[world-remove] …" (remove) is opened.

   Reads:  ISSUE_TITLE, ISSUE_BODY, ISSUE_AUTHOR   (from the issue, via env)
   Writes: world/rooms/<CODE>.json, world/index.json
           RESULT_FILE   — the reply posted on the issue
           GITHUB_OUTPUT — changed=true|false, summary=<one line>

   Everything in the issue came from a stranger, so the share code is
   checked field by field (same rules as cleanHome() in the game) and
   only the account that first published a home may update or remove it.
   No dependencies: plain Node 20.                                        */
import fs from "node:fs";
import path from "node:path";
import zlib from "node:zlib";
import crypto from "node:crypto";

const WORLD = path.resolve(process.env.WORLD_DIR || path.join(path.dirname(new URL(import.meta.url).pathname), ".."));
const INDEX = path.join(WORLD, "index.json");
const ROOMS = path.join(WORLD, "rooms");
const ALPHABET = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789";   // no 0/O or 1/I to mix up
const MAX_HOMES_PER_ACCOUNT = 3;
const NAME_BLOCK = ["fuck","shit","cunt","bitch","nigg","fag","slut","whore","rape","nazi","hitler","penis","vagina","dick","cock","pussy","porn","sex","kill","admin","moderator","petopia"];

const title = String(process.env.ISSUE_TITLE || "");
const body = String(process.env.ISSUE_BODY || "");
const login = String(process.env.ISSUE_AUTHOR || "").toLowerCase();

function finish(changed, summary, reply) {
  fs.writeFileSync(process.env.RESULT_FILE || path.join(WORLD, "..", "world-result.md"), reply);
  if (process.env.GITHUB_OUTPUT) fs.appendFileSync(process.env.GITHUB_OUTPUT, `changed=${changed}\nsummary=${summary.replace(/[\r\n]/g, " ")}\n`);
  console.log(summary);
  process.exit(0);
}
const sorry = (why) => finish(false, `rejected: ${why}`,
  `Sorry — Petopia couldn't add this home. ${why}\n\nIn the game, open 🌏 → Share my room → **Publish on GitHub** and press Submit without changing the text.`);

function nameProblem(n) {
  n = String(n || "").trim();
  if (n.length < 2 || n.length > 20) return "The name needs 2 to 20 letters.";
  if (!/^[A-Za-z0-9][A-Za-z0-9 _.'-]*$/.test(n)) return "Names can use letters, numbers, spaces and - _ . ' only.";
  const flat = n.toLowerCase().replace(/0/g, "o").replace(/1/g, "i").replace(/3/g, "e").replace(/4/g, "a").replace(/5/g, "s").replace(/[^a-z]/g, "");
  if (NAME_BLOCK.some((w) => flat.includes(w))) return "That name isn't allowed — please pick a friendlier one in the game.";
  return null;
}

/* the share code → the compact home object, checked field by field */
function readCode(text) {
  const m = /PET([01])\.([A-Za-z0-9_-]{8,20000})/.exec(text);
  if (!m) return { error: "There's no Petopia code in this issue." };
  let bytes = Buffer.from(m[2].replace(/-/g, "+").replace(/_/g, "/"), "base64");
  try { if (m[1] === "1") bytes = zlib.inflateRawSync(bytes, { maxOutputLength: 200000 }); }
  catch { return { error: "The code is damaged — copy it again from the game." }; }
  let o;
  try { o = JSON.parse(bytes.toString("utf8")); } catch { return { error: "The code is damaged — copy it again from the game." }; }
  if (!o || o.v !== 1) return { error: "This code is from a different version of Petopia." };
  const str = (v, n) => String(v ?? "").replace(/[\u0000-\u001f<>]/g, "").slice(0, n).trim();
  const num = (v, max) => Math.max(0, Math.min(max, Math.floor(+v || 0)));
  const frac = (v, d) => Math.max(0, Math.min(1, Number.isFinite(+v) ? +v : d));
  const word = (v, re) => (typeof v === "string" && re.test(v) ? v : null);
  const id = word(o.id, /^[a-z0-9]{6,24}$/);
  if (!id) return { error: "The code is damaged — copy it again from the game." };
  const name = str(o.name, 20), bad = nameProblem(name);
  if (bad) return { error: bad };
  const R = o.room || {};
  const room = {
    k: word(R.k, /^[a-z]{1,12}$/) || "starter", n: str(R.n, 24), t: word(R.t, /^[a-z]{1,16}$/) || "cottage",
    w: word(R.w, /^[a-z_0-9]{1,40}$/) || "w_cream", f: word(R.f, /^[a-z_0-9]{1,40}$/) || "f_oak",
    p: (Array.isArray(R.p) ? R.p : []).slice(0, 80).filter((a) => Array.isArray(a) && word(a[0], /^i_[A-Za-z0-9_]{1,40}$/))
      .map((a) => [a[0], +frac(a[1], 0.5).toFixed(3), +frac(a[2], 0.8).toFixed(3), a[3] < 0 ? -1 : 1]),
  };
  const cats = (Array.isArray(o.cats) ? o.cats : []).slice(0, 12).filter((a) => Array.isArray(a) && word(a[1], /^[a-z]{1,16}$/))
    .map((a) => [str(a[0], 14) || "Kitty", a[1], a[2] === "m" ? "m" : "f", num(a[3], 99999), word(a[4], /^[a-z]{1,16}$/) || "none",
      (Array.isArray(a[5]) ? a[5] : []).filter((w) => word(w, /^c_[a-z0-9_]{1,24}$/)).slice(0, 5), a[6] < 0 ? -1 : 1]);
  const S = o.s || {};
  const s = { c: num(S.c, 999), r: num(S.r, 50), cc: num(S.cc, 1e7), st: num(S.st, 99999), a: num(S.a, 999) };
  return { home: { v: 1, id, name, at: Date.now(), room, cats, s } };
}

function shortCode(seed, taken) {
  for (let i = 0; ; i++) {
    const h = crypto.createHash("sha256").update(`${seed}:${i}`).digest();
    let c = ""; for (let k = 0; k < 6; k++) c += ALPHABET[h[k] % ALPHABET.length];
    if (!taken(c)) return c;
  }
}

if (!login) sorry("GitHub didn't say who opened this issue.");
const index = fs.existsSync(INDEX) ? JSON.parse(fs.readFileSync(INDEX, "utf8")) : { v: 1, updated: 0, homes: [] };
index.homes = Array.isArray(index.homes) ? index.homes : [];
fs.mkdirSync(ROOMS, { recursive: true });
const save = () => { index.updated = Date.now(); fs.writeFileSync(INDEX, JSON.stringify(index, null, 1) + "\n"); };

/* ── remove: everything this account has published ── */
if (/^\s*\[world-remove\]/i.test(title)) {
  const mine = index.homes.filter((h) => h.owner === login);
  if (!mine.length) finish(false, "nothing to remove", "There's no Petopia home published from this GitHub account.");
  for (const h of mine) fs.rmSync(path.join(ROOMS, `${h.code}.json`), { force: true });
  index.homes = index.homes.filter((h) => h.owner !== login);
  save();
  finish(true, `removed ${mine.map((h) => h.code).join(", ")}`,
    `Done — removed ${mine.length === 1 ? "your home" : `${mine.length} homes`} (${mine.map((h) => h.code).join(", ")}) from Petopia World.`);
}

/* ── publish ── */
if (!/^\s*\[world\]/i.test(title)) finish(false, "not a Petopia World issue", "This issue isn't for Petopia World.");
const { home, error } = readCode(body);
if (error) sorry(error);
const existing = index.homes.find((h) => h.id === home.id);
if (existing && existing.owner !== login) sorry("This home was published from a different GitHub account.");
if (!existing && index.homes.filter((h) => h.owner === login).length >= MAX_HOMES_PER_ACCOUNT)
  sorry(`Each GitHub account can publish up to ${MAX_HOMES_PER_ACCOUNT} homes. Open an issue titled [world-remove] to clear yours first.`);
const code = existing ? existing.code : shortCode(`${login}:${home.id}`, (c) => index.homes.some((h) => h.code === c));
fs.writeFileSync(path.join(ROOMS, `${code}.json`), JSON.stringify({ v: 1, code, updated: Date.now(), home }) + "\n");
const entry = { code, id: home.id, name: home.name, owner: login, at: home.at, room: home.room.n,
  cats: home.s.c, catCoins: home.s.cc, rooms: home.s.r, streak: home.s.st, ach: home.s.a };
index.homes = [entry, ...index.homes.filter((h) => h.id !== home.id)].slice(0, 1000);
save();
finish(true, `${existing ? "updated" : "added"} ${code} (${home.name})`,
  `🎉 **${home.name}**'s home is in Petopia World!\n\nYour code is **${code}** — friends type it in 🌏 → *Visit a home*. ` +
  `(New homes can take about 5 minutes to appear for everyone.)\n\n` +
  `Publish again any time to update it — you'll keep the same code. To take it down, open an issue titled \`[world-remove]\`.\n\n` +
  `Published homes are public: anyone can visit this one, and it's on the leaderboards.`);
