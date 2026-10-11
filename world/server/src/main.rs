//! Petopia World server: published homes, their 6-letter codes and the leaderboard.
//!
//! The game reads    GET  index.json          the leaderboard (every published home's headline numbers)
//!                   GET  rooms/<CODE>.json   one home
//! and writes        POST publish  {"code":"PET1…","key":"…"}  → {"ok":true,"code":"KTZ4QA","added":true}
//!                   POST remove   {"key":"…"}                 → {"ok":true,"removed":["KTZ4QA"]}
//! (Caddy serves this at https://br8t.com/petopia/world/.)
//!
//! A home's owner is the game that first published it: each game keeps a random secret key in its
//! save, and only that key can update or remove its homes. The server keeps a hash of the key
//! (owners.json, never served), not the key. Everything in a share code came from a stranger, so it
//! is checked field by field with the same rules as cleanHome() in the game (and the old GitHub robot,
//! world/tools/publish.mjs, which this replaces).
//!
//! Files in DATA_DIR: index.json, rooms/<CODE>.json (both exactly what is served), owners.json.
//! Env: PORT (8015), BIND (127.0.0.1), DATA_DIR (./data).

use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, Path, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use base64::Engine;
use serde_json::{json, Map, Value};
use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789"; // no 0/O or 1/I to mix up
const MAX_HOMES_PER_PLAYER: usize = 3;
const MAX_HOMES: usize = 2000;
const NEW_HOMES_PER_ADDRESS_PER_DAY: usize = 6;
const POSTS_PER_ADDRESS_PER_10_MIN: usize = 20;
const NAME_BLOCK: [&str; 4] = ["nigger", "rape", "nazi", "hitler"]; // same list as NAME_BLOCK in Petopia.html

struct World {
    dir: PathBuf,
    homes: Vec<Value>,               // the leaderboard entries, newest first
    rooms: HashMap<String, String>,  // code → rooms/<CODE>.json as served
    owners: HashMap<String, String>, // code → hash of its owner's key
    index_text: String,
    posts: HashMap<String, Vec<Instant>>,     // address → recent publish/remove requests
    new_homes: HashMap<String, Vec<Instant>>, // address → recent new homes
}
type Shared = Arc<Mutex<World>>;

fn now_ms() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0) }

/* ---------------- reading a share code (same rules as cleanHome() in the game) ---------------- */

// JavaScript's +v, as the game's checks use it
fn to_num(v: &Value) -> f64 {
    match v {
        Value::Number(n) => n.as_f64().unwrap_or(f64::NAN),
        Value::String(s) => { let t = s.trim(); if t.is_empty() { 0.0 } else { t.parse::<f64>().unwrap_or(f64::NAN) } }
        Value::Bool(b) => if *b { 1.0 } else { 0.0 },
        Value::Null => 0.0,
        Value::Array(a) => match a.len() { 0 => 0.0, 1 => to_num(&a[0]), _ => f64::NAN },
        Value::Object(_) => f64::NAN,
    }
}
// Math.floor(+v || 0), kept to 0..max
fn num(v: &Value, max: f64) -> i64 { let x = to_num(v); let x = if x.is_nan() { 0.0 } else { x }; x.floor().clamp(0.0, max) as i64 }
// a fraction 0..1 to 3 decimals (d when it isn't a number)
fn frac(v: &Value, d: f64) -> f64 { let x = to_num(v); let x = if x.is_finite() { x } else { d }; (x.clamp(0.0, 1.0) * 1000.0).round() / 1000.0 }
// String(v ?? ""), without control characters or < >, at most n characters, trimmed
fn text(v: &Value, n: usize) -> String {
    let s = match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Number(x) => x.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Array(_) => String::new(),
        Value::Object(_) => "[object Object]".into(),
    };
    s.chars().filter(|&c| c as u32 >= 0x20 && c != '<' && c != '>').take(n).collect::<String>().trim().to_string()
}
// a string of `ok` characters, min..=max long, starting with `prefix`
fn word<'a>(v: &'a Value, prefix: &str, min: usize, max: usize, ok: fn(char) -> bool) -> Option<&'a str> {
    let s = v.as_str()?;
    let rest = s.strip_prefix(prefix)?;
    let n = rest.chars().count();
    (n >= min && n <= max && rest.chars().all(ok)).then_some(s)
}
fn lower(c: char) -> bool { c.is_ascii_lowercase() }
fn lower_digit(c: char) -> bool { c.is_ascii_lowercase() || c.is_ascii_digit() }
fn lower_digit_us(c: char) -> bool { c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' }
fn alnum_us(c: char) -> bool { c.is_ascii_alphanumeric() || c == '_' }
fn neg(v: &Value) -> i64 { if to_num(v) < 0.0 { -1 } else { 1 } }

fn name_problem(n: &str) -> Option<&'static str> {
    let n = n.trim();
    let len = n.chars().count();
    if !(2..=20).contains(&len) { return Some("The name needs 2 to 20 letters."); }
    let mut cs = n.chars();
    let first_ok = cs.next().is_some_and(|c| c.is_ascii_alphanumeric());
    if !first_ok || !cs.all(|c| c.is_ascii_alphanumeric() || " _.'-".contains(c)) {
        return Some("Names can use letters, numbers, spaces and - _ . ' only.");
    }
    let flat: String = n.to_lowercase().chars()
        .map(|c| match c { '0' => 'o', '1' => 'i', '3' => 'e', '4' => 'a', '5' => 's', c => c })
        .filter(|c| c.is_ascii_lowercase()).collect();
    if NAME_BLOCK.iter().any(|w| flat.contains(w)) { return Some("Statement/s in selected name was blocked."); }
    None
}

// the first PET0./PET1. code in the text: (compressed?, its letters)
fn find_code(t: &str) -> Option<(bool, &str)> {
    let mut from = 0;
    while let Some(i) = t[from..].find("PET") {
        let at = from + i;
        let b = t.as_bytes();
        if at + 4 < b.len() && (b[at + 3] == b'0' || b[at + 3] == b'1') && b[at + 4] == b'.' {
            let body = &t[at + 5..];
            let n = body.bytes().take(20000).take_while(|c| c.is_ascii_alphanumeric() || *c == b'_' || *c == b'-').count();
            if n >= 8 { return Some((b[at + 3] == b'1', &body[..n])); }
        }
        from = at + 3;
    }
    None
}

const DAMAGED: &str = "The code is damaged. Copy it again from the game.";

/// The share code → the home as stored and served (checked field by field).
fn read_code(code: &str) -> Result<Value, &'static str> {
    let (zipped, letters) = find_code(code).ok_or("There's no Petopia code here.")?;
    let mut bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(letters).map_err(|_| DAMAGED)?;
    if zipped { bytes = miniz_oxide::inflate::decompress_to_vec_with_limit(&bytes, 200_000).map_err(|_| DAMAGED)?; }
    let o: Value = serde_json::from_slice(&bytes).map_err(|_| DAMAGED)?;
    if o.get("v").and_then(Value::as_f64) != Some(1.0) { return Err("This code is from a different version of Petopia."); }
    let null = Value::Null;
    let g = |v: &Value, k: &str| -> Value { v.get(k).cloned().unwrap_or(Value::Null) };
    let id = word(o.get("id").unwrap_or(&null), "", 6, 24, lower_digit).ok_or(DAMAGED)?.to_string();
    let name = text(&g(&o, "name"), 20);
    if let Some(bad) = name_problem(&name) { return Err(bad); }
    let r = g(&o, "room");
    let placed: Vec<Value> = r.get("p").and_then(Value::as_array).map(|a| a.as_slice()).unwrap_or(&[]).iter().take(80)
        .filter_map(|a| {
            let a = a.as_array()?;
            let item = word(a.first()?, "i_", 1, 40, alnum_us)?;
            Some(json!([item, frac(a.get(1).unwrap_or(&null), 0.5), frac(a.get(2).unwrap_or(&null), 0.8), neg(a.get(3).unwrap_or(&null))]))
        }).collect();
    let room = json!({
        "k": word(&g(&r, "k"), "", 1, 12, lower).unwrap_or("starter"),
        "n": text(&g(&r, "n"), 24),
        "t": word(&g(&r, "t"), "", 1, 16, lower).unwrap_or("cottage"),
        "w": word(&g(&r, "w"), "", 1, 40, lower_digit_us).unwrap_or("w_cream"),
        "f": word(&g(&r, "f"), "", 1, 40, lower_digit_us).unwrap_or("f_oak"),
        "p": placed,
    });
    let cats: Vec<Value> = o.get("cats").and_then(Value::as_array).map(|a| a.as_slice()).unwrap_or(&[]).iter().take(12)
        .filter_map(|a| {
            let a = a.as_array()?;
            let at = |i: usize| a.get(i).unwrap_or(&null);
            let breed = word(at(1), "", 1, 16, lower)?;
            let mut name = text(at(0), 14); if name.is_empty() { name = "Kitty".into(); }
            let wear: Vec<&str> = at(5).as_array().map(|w| w.as_slice()).unwrap_or(&[]).iter()
                .filter_map(|w| word(w, "c_", 1, 24, lower_digit_us)).take(5).collect();
            Some(json!([name, breed, if at(2).as_str() == Some("m") { "m" } else { "f" }, num(at(3), 99999.0),
                word(at(4), "", 1, 16, lower).unwrap_or("none"), wear, neg(at(6))]))
        }).collect();
    let s = g(&o, "s");
    let stats = json!({ "c": num(&g(&s, "c"), 999.0), "r": num(&g(&s, "r"), 50.0), "cc": num(&g(&s, "cc"), 1e7),
        "st": num(&g(&s, "st"), 99999.0), "a": num(&g(&s, "a"), 999.0) });
    Ok(json!({ "v": 1, "id": id, "name": name, "at": now_ms(), "room": room, "cats": cats, "s": stats }))
}

/* ---------------- owners: a SHA-256 of the key ---------------- */

fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
        0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
        0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
        0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2];
    let mut h: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let mut m = data.to_vec();
    let bits = (data.len() as u64) * 8;
    m.push(0x80);
    while m.len() % 64 != 56 { m.push(0); }
    m.extend_from_slice(&bits.to_be_bytes());
    for chunk in m.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 { w[i] = u32::from_be_bytes([chunk[4 * i], chunk[4 * i + 1], chunk[4 * i + 2], chunk[4 * i + 3]]); }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let t1 = hh.wrapping_add(e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25)).wrapping_add((e & f) ^ (!e & g))
                .wrapping_add(K[i]).wrapping_add(w[i]);
            let t2 = (a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22)).wrapping_add((a & b) ^ (a & c) ^ (b & c));
            hh = g; g = f; f = e; e = d.wrapping_add(t1); d = c; c = b; b = a; a = t1.wrapping_add(t2);
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) { *x = x.wrapping_add(y); }
    }
    h.iter().map(|x| format!("{x:08x}")).collect()
}
fn owner_of(key: &str) -> String { sha256_hex(format!("petopia-world:{key}").as_bytes()) }
fn key_ok(k: &str) -> bool { (20..=64).contains(&k.len()) && k.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-') }

/* ---------------- storage ---------------- */

fn write_atomic(path: &PathBuf, text: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, text)?;
    fs::rename(&tmp, path)
}
impl World {
    fn load(dir: PathBuf) -> World {
        fs::create_dir_all(dir.join("rooms")).expect("can't make DATA_DIR/rooms");
        let read = |p: PathBuf| fs::read_to_string(p).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok());
        let homes: Vec<Value> = read(dir.join("index.json")).and_then(|i| i.get("homes").and_then(Value::as_array).cloned()).unwrap_or_default();
        let owners: HashMap<String, String> = read(dir.join("owners.json")).and_then(|o| o.as_object().cloned())
            .map(|m| m.into_iter().filter_map(|(k, v)| Some((k, v.as_str()?.to_string()))).collect()).unwrap_or_default();
        let mut rooms = HashMap::new();
        for h in &homes {
            if let Some(c) = h.get("code").and_then(Value::as_str) {
                if let Ok(t) = fs::read_to_string(dir.join("rooms").join(format!("{c}.json"))) { rooms.insert(c.to_string(), t); }
            }
        }
        let mut w = World { dir, homes, rooms, owners, index_text: String::new(), posts: HashMap::new(), new_homes: HashMap::new() };
        w.index_text = w.render_index(0);
        w
    }
    fn render_index(&self, updated: u64) -> String { json!({ "v": 1, "updated": updated, "homes": self.homes }).to_string() + "\n" }
    fn save(&mut self) -> std::io::Result<()> {
        self.index_text = self.render_index(now_ms());
        let owners: Map<String, Value> = self.owners.iter().map(|(k, v)| (k.clone(), Value::String(v.clone()))).collect();
        write_atomic(&self.dir.join("owners.json"), &Value::Object(owners).to_string())?;
        write_atomic(&self.dir.join("index.json"), &self.index_text)
    }
    // counts a request from `addr` against a limit over a time window (true: allowed)
    fn allow(map: &mut HashMap<String, Vec<Instant>>, addr: &str, limit: usize, window: Duration) -> bool {
        let now = Instant::now();
        let v = map.entry(addr.to_string()).or_default();
        v.retain(|t| now.duration_since(*t) < window);
        if v.len() >= limit { return false; }
        v.push(now);
        true
    }
    fn new_code(&self) -> String {
        loop {
            let mut b = [0u8; 6];
            getrandom::fill(&mut b).expect("no randomness");
            let c: String = b.iter().map(|x| ALPHABET[(*x as usize) % ALPHABET.len()] as char).collect();
            if !self.rooms.contains_key(&c) { return c; }
        }
    }
}

/* ---------------- HTTP ---------------- */

// every answer can be read by the game from any copy (GitHub Pages, a downloaded file)
fn reply(status: StatusCode, body: String, cache: &'static str) -> Response {
    let mut r = (status, body).into_response();
    let h = r.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json; charset=utf-8"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    h.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    h.insert(header::ACCESS_CONTROL_ALLOW_METHODS, HeaderValue::from_static("GET, POST, OPTIONS"));
    h.insert(header::ACCESS_CONTROL_ALLOW_HEADERS, HeaderValue::from_static("content-type"));
    r
}
fn ok(v: Value) -> Response { reply(StatusCode::OK, v.to_string(), "no-store") }
fn no(status: StatusCode, why: &str) -> Response { reply(status, json!({ "ok": false, "error": why }).to_string(), "no-store") }
// the player's address, as Caddy passes it on (this server only listens on 127.0.0.1)
fn address(h: &HeaderMap) -> String {
    h.get("x-forwarded-for").and_then(|v| v.to_str().ok()).and_then(|v| v.split(',').next()).unwrap_or("local").trim().to_string()
}
fn body_json(b: &Bytes) -> Option<Map<String, Value>> { serde_json::from_slice::<Value>(b).ok()?.as_object().cloned() }

async fn index(State(w): State<Shared>) -> Response { reply(StatusCode::OK, w.lock().unwrap().index_text.clone(), "no-cache") }
async fn room(State(w): State<Shared>, Path(file): Path<String>) -> Response {
    let code = file.strip_suffix(".json").unwrap_or(&file).to_ascii_uppercase();
    match w.lock().unwrap().rooms.get(&code) {
        Some(t) => reply(StatusCode::OK, t.clone(), "no-cache"),
        None => no(StatusCode::NOT_FOUND, "No home with that code."),
    }
}
async fn preflight() -> Response { reply(StatusCode::NO_CONTENT, String::new(), "max-age=86400") }
async fn status(State(w): State<Shared>) -> Response {
    let w = w.lock().unwrap();
    ok(json!({ "homes": w.homes.len(), "version": env!("CARGO_PKG_VERSION") }))
}

async fn publish(State(w): State<Shared>, headers: HeaderMap, body: Bytes) -> Response {
    let Some(req) = body_json(&body) else { return no(StatusCode::BAD_REQUEST, "That wasn't a Petopia request.") };
    let key = req.get("key").and_then(Value::as_str).unwrap_or("");
    if !key_ok(key) { return no(StatusCode::BAD_REQUEST, "Your game is missing its World key. Reload the game and try again."); }
    let addr = address(&headers);
    let mut w = w.lock().unwrap();
    if !World::allow(&mut w.posts, &addr, POSTS_PER_ADDRESS_PER_10_MIN, Duration::from_secs(600)) {
        return no(StatusCode::TOO_MANY_REQUESTS, "Too many tries from here. Wait ten minutes and publish again.");
    }
    let home = match read_code(req.get("code").and_then(Value::as_str).unwrap_or("")) { Ok(h) => h, Err(e) => return no(StatusCode::BAD_REQUEST, e) };
    let id = home["id"].as_str().unwrap_or("").to_string();
    let owner = owner_of(key);
    let existing = w.homes.iter().find(|h| h["id"] == id.as_str()).and_then(|h| h["code"].as_str()).map(str::to_string);
    if let Some(c) = &existing {
        if w.owners.get(c) != Some(&owner) { return no(StatusCode::FORBIDDEN, "This home was published from a different game, so only that one can update it."); }
    } else {
        if w.owners.values().filter(|o| **o == owner).count() >= MAX_HOMES_PER_PLAYER {
            return no(StatusCode::FORBIDDEN, "You've published 3 homes already. Take them down first (🌏 → Share → Take my homes down).");
        }
        if w.homes.len() >= MAX_HOMES { return no(StatusCode::SERVICE_UNAVAILABLE, "Petopia World is full right now. Try again another day."); }
        if !World::allow(&mut w.new_homes, &addr, NEW_HOMES_PER_ADDRESS_PER_DAY, Duration::from_secs(86400)) {
            return no(StatusCode::TOO_MANY_REQUESTS, "Lots of new homes have come from here today. Try again tomorrow.");
        }
    }
    let code = existing.clone().unwrap_or_else(|| w.new_code());
    let now = now_ms();
    let s = &home["s"];
    let entry = json!({ "code": code, "id": id, "name": home["name"], "at": home["at"], "room": home["room"]["n"],
        "cats": s["c"], "catCoins": s["cc"], "rooms": s["r"], "streak": s["st"], "ach": s["a"] });
    let file = json!({ "v": 1, "code": code, "updated": now, "home": home }).to_string() + "\n";
    if write_atomic(&w.dir.join("rooms").join(format!("{code}.json")), &file).is_err() {
        return no(StatusCode::INTERNAL_SERVER_ERROR, "Petopia World couldn't save that. Try again soon.");
    }
    w.rooms.insert(code.clone(), file);
    w.owners.insert(code.clone(), owner);
    w.homes.retain(|h| h["id"] != id.as_str());
    w.homes.insert(0, entry);
    if w.save().is_err() { return no(StatusCode::INTERNAL_SERVER_ERROR, "Petopia World couldn't save that. Try again soon."); }
    println!("{} {code} ({})", if existing.is_some() { "updated" } else { "added" }, home["name"].as_str().unwrap_or(""));
    ok(json!({ "ok": true, "code": code, "added": existing.is_none() }))
}

async fn remove(State(w): State<Shared>, headers: HeaderMap, body: Bytes) -> Response {
    let Some(req) = body_json(&body) else { return no(StatusCode::BAD_REQUEST, "That wasn't a Petopia request.") };
    let key = req.get("key").and_then(Value::as_str).unwrap_or("");
    if !key_ok(key) { return no(StatusCode::BAD_REQUEST, "Your game is missing its World key."); }
    let addr = address(&headers);
    let mut w = w.lock().unwrap();
    if !World::allow(&mut w.posts, &addr, POSTS_PER_ADDRESS_PER_10_MIN, Duration::from_secs(600)) {
        return no(StatusCode::TOO_MANY_REQUESTS, "Too many tries from here. Wait ten minutes and try again.");
    }
    let owner = owner_of(key);
    let mine: Vec<String> = w.owners.iter().filter(|(_, o)| **o == owner).map(|(c, _)| c.clone()).collect();
    for c in &mine {
        let _ = fs::remove_file(w.dir.join("rooms").join(format!("{c}.json")));
        w.rooms.remove(c);
        w.owners.remove(c);
    }
    w.homes.retain(|h| !mine.iter().any(|c| h["code"] == c.as_str()));
    if !mine.is_empty() {
        if w.save().is_err() { return no(StatusCode::INTERNAL_SERVER_ERROR, "Petopia World couldn't save that. Try again soon."); }
        println!("removed {}", mine.join(", "));
    }
    ok(json!({ "ok": true, "removed": mine }))
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let port = std::env::var("PORT").ok().and_then(|p| p.parse::<u16>().ok()).unwrap_or(8015);
    let bind = std::env::var("BIND").unwrap_or_else(|_| "127.0.0.1".into());
    let dir = PathBuf::from(std::env::var("DATA_DIR").unwrap_or_else(|_| "./data".into()));
    let world: Shared = Arc::new(Mutex::new(World::load(dir.clone())));
    println!("Petopia World: {} homes in {}, on {bind}:{port}", world.lock().unwrap().homes.len(), dir.display());
    let app = Router::new()
        .route("/index.json", get(index))
        .route("/rooms/{file}", get(room))
        .route("/publish", axum::routing::post(publish).options(preflight))
        .route("/remove", axum::routing::post(remove).options(preflight))
        .route("/status", get(status))
        .layer(DefaultBodyLimit::max(64 * 1024))
        .with_state(world);
    let listener = tokio::net::TcpListener::bind((bind.as_str(), port)).await.expect("can't listen");
    axum::serve(listener, app).await.expect("server stopped");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sha256_known() {
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    }
    #[test]
    fn names() {
        assert!(name_problem("cat lover").is_none());
        assert!(name_problem("x").is_some());
        assert!(name_problem("h1tler fan").is_some());
        assert!(name_problem("-cat").is_some());
    }
    #[test]
    fn code_round_trip() {
        let home = json!({"v":1,"id":"abc123def456","name":"cat lover","at":1,
            "room":{"k":"starter","n":"Lounge <b>","t":"cottage","w":"w_cream","f":"f_oak","p":[["i_sofa",0.5,0.8,1],["bad",0,0,1],["i_lamp","0.25",2,-1]]},
            "cats":[["Mittens","tabby","m",120,"lazy",["c_hat","x_bad"],1],["Nope","Tabby","f",1,"x",[],1]],
            "s":{"c":2,"r":"3","cc":1e12,"st":-4,"a":null}});
        let code = format!("hi PET0.{} bye", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(home.to_string()));
        let h = read_code(&code).unwrap();
        assert_eq!(h["room"]["n"], "Lounge b");
        assert_eq!(h["room"]["p"], json!([["i_sofa", 0.5, 0.8, 1], ["i_lamp", 0.25, 1.0, -1]]));
        assert_eq!(h["cats"], json!([["Mittens", "tabby", "m", 120, "lazy", ["c_hat"], 1]]));
        assert_eq!(h["s"], json!({"c":2,"r":3,"cc":10000000,"st":0,"a":0}));
        assert!(read_code("PET0.short").is_err());
    }
}
