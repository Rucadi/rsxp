// SPDX-License-Identifier: GPL-2.0-or-later
//! MCP stdio server for the live rsxp game API.
use std::{
    io::{self, BufRead, BufReader, Read, Write},
    net::TcpStream,
    time::Duration,
};
#[cfg(feature = "api-docs")]
#[path = "openapi.rs"]
mod openapi;

#[derive(Clone, Debug)]
enum Json {
    Null,
    Bool(bool),
    Number(String),
    String(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Json {
    fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Self::Object(fields) => fields.iter().find(|(name, _)| name == key).map(|(_, v)| v),
            _ => None,
        }
    }
    fn string(&self) -> Option<&str> { if let Self::String(value) = self { Some(value) } else { None } }
    fn integer(&self) -> Option<i64> {
        match self { Self::Number(v) => v.parse().ok(), _ => None }
    }
    fn encode(&self, out: &mut String) {
        match self {
            Self::Null => out.push_str("null"),
            Self::Bool(v) => out.push_str(if *v { "true" } else { "false" }),
            Self::Number(v) => out.push_str(v),
            Self::String(v) => quote(v, out),
            Self::Array(values) => {
                out.push('[');
                for (i, value) in values.iter().enumerate() { if i > 0 { out.push(','); } value.encode(out); }
                out.push(']');
            }
            Self::Object(fields) => {
                out.push('{');
                for (i, (key, value)) in fields.iter().enumerate() {
                    if i > 0 { out.push(','); }
                    quote(key, out); out.push(':'); value.encode(out);
                }
                out.push('}');
            }
        }
    }
}

fn quote(value: &str, out: &mut String) {
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""), '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"), '\r' => out.push_str("\\r"), '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)), c => out.push(c),
        }
    }
    out.push('"');
}

struct Parser<'a> { input: &'a [u8], offset: usize }
impl<'a> Parser<'a> {
    fn parse(input: &'a str) -> Result<Json, String> {
        let mut parser = Self { input: input.as_bytes(), offset: 0 };
        let value = parser.value()?;
        parser.space();
        if parser.offset != parser.input.len() { return Err("trailing JSON data".into()); }
        Ok(value)
    }
    fn space(&mut self) { while self.input.get(self.offset).is_some_and(u8::is_ascii_whitespace) { self.offset += 1; } }
    fn value(&mut self) -> Result<Json, String> {
        self.space();
        match self.input.get(self.offset).copied() {
            Some(b'n') => { self.word(b"null")?; Ok(Json::Null) }
            Some(b't') => { self.word(b"true")?; Ok(Json::Bool(true)) }
            Some(b'f') => { self.word(b"false")?; Ok(Json::Bool(false)) }
            Some(b'"') => self.string().map(Json::String),
            Some(b'[') => self.array(), Some(b'{') => self.object(),
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => Err("invalid JSON value".into()),
        }
    }
    fn word(&mut self, word: &[u8]) -> Result<(), String> {
        if self.input.get(self.offset..self.offset + word.len()) == Some(word) { self.offset += word.len(); Ok(()) }
        else { Err("invalid JSON token".into()) }
    }
    fn string(&mut self) -> Result<String, String> {
        self.offset += 1;
        let mut output = String::new();
        while let Some(byte) = self.input.get(self.offset).copied() {
            self.offset += 1;
            match byte {
                b'"' => return Ok(output), b'\\' => {
                    let escaped = *self.input.get(self.offset).ok_or("incomplete JSON escape")?; self.offset += 1;
                    match escaped {
                        b'"' => output.push('"'), b'\\' => output.push('\\'), b'/' => output.push('/'),
                        b'b' => output.push('\u{8}'), b'f' => output.push('\u{c}'), b'n' => output.push('\n'),
                        b'r' => output.push('\r'), b't' => output.push('\t'),
                        b'u' => {
                            let end = self.offset + 4;
                            let digits = std::str::from_utf8(self.input.get(self.offset..end).ok_or("incomplete unicode escape")?).map_err(|_| "invalid unicode escape")?;
                            let code = u16::from_str_radix(digits, 16).map_err(|_| "invalid unicode escape")?;
                            self.offset = end;
                            let scalar = if (0xd800..=0xdbff).contains(&code) {
                                if self.input.get(self.offset..self.offset + 2) != Some(b"\\u") {
                                    return Err("incomplete unicode surrogate pair".into());
                                }
                                self.offset += 2;
                                let end = self.offset + 4;
                                let digits = std::str::from_utf8(self.input.get(self.offset..end).ok_or("incomplete unicode escape")?).map_err(|_| "invalid unicode escape")?;
                                let low = u16::from_str_radix(digits, 16).map_err(|_| "invalid unicode escape")?;
                                if !(0xdc00..=0xdfff).contains(&low) { return Err("invalid unicode surrogate pair".into()); }
                                self.offset = end;
                                let value = 0x10000 + (((code as u32 - 0xd800) << 10) | (low as u32 - 0xdc00));
                                char::from_u32(value).ok_or("invalid unicode scalar")?
                            } else {
                                char::from_u32(code as u32).ok_or("invalid unicode scalar")?
                            };
                            output.push(scalar);
                        }
                        _ => return Err("invalid JSON escape".into()),
                    }
                }
                b if b < 0x20 => return Err("control character in JSON string".into()),
                b if b < 0x80 => output.push(b as char),
                _ => {
                    self.offset -= 1;
                    let remainder = std::str::from_utf8(&self.input[self.offset..]).map_err(|_| "invalid UTF-8")?;
                    let c = remainder.chars().next().ok_or("incomplete UTF-8")?;
                    self.offset += c.len_utf8(); output.push(c);
                }
            }
        }
        Err("unterminated JSON string".into())
    }
    fn number(&mut self) -> Result<Json, String> {
        let start = self.offset;
        if self.input.get(self.offset) == Some(&b'-') { self.offset += 1; }
        match self.input.get(self.offset) {
            Some(b'0') => {
                self.offset += 1;
                if self.input.get(self.offset).is_some_and(u8::is_ascii_digit) {
                    return Err("leading zero in JSON number".into());
                }
            }
            Some(b'1'..=b'9') => {
                self.offset += 1;
                while self.input.get(self.offset).is_some_and(u8::is_ascii_digit) { self.offset += 1; }
            }
            _ => return Err("invalid JSON number".into()),
        }
        if self.input.get(self.offset) == Some(&b'.') {
            self.offset += 1;
            let fraction = self.offset;
            while self.input.get(self.offset).is_some_and(u8::is_ascii_digit) { self.offset += 1; }
            if self.offset == fraction { return Err("missing JSON fraction digits".into()); }
        }
        if self.input.get(self.offset).is_some_and(|b| matches!(b, b'e' | b'E')) {
            self.offset += 1;
            if self.input.get(self.offset).is_some_and(|b| matches!(b, b'+' | b'-')) { self.offset += 1; }
            let exponent = self.offset;
            while self.input.get(self.offset).is_some_and(u8::is_ascii_digit) { self.offset += 1; }
            if self.offset == exponent { return Err("missing JSON exponent digits".into()); }
        }
        let number = std::str::from_utf8(&self.input[start..self.offset]).map_err(|_| "invalid number")?;
        Ok(Json::Number(number.to_string()))
    }
    fn array(&mut self) -> Result<Json, String> {
        self.offset += 1; self.space(); let mut values = Vec::new();
        if self.input.get(self.offset) == Some(&b']') { self.offset += 1; return Ok(Json::Array(values)); }
        loop { values.push(self.value()?); self.space(); match self.input.get(self.offset) {
            Some(b',') => self.offset += 1, Some(b']') => { self.offset += 1; break; }, _ => return Err("expected , or ]".into()),
        }}
        Ok(Json::Array(values))
    }
    fn object(&mut self) -> Result<Json, String> {
        self.offset += 1; self.space(); let mut fields = Vec::new();
        if self.input.get(self.offset) == Some(&b'}') { self.offset += 1; return Ok(Json::Object(fields)); }
        loop {
            self.space(); if self.input.get(self.offset) != Some(&b'"') { return Err("expected JSON object key".into()); }
            let key = self.string()?; self.space(); if self.input.get(self.offset) != Some(&b':') { return Err("expected :".into()); }
            self.offset += 1; let value = self.value()?; fields.push((key, value)); self.space();
            match self.input.get(self.offset) { Some(b',') => self.offset += 1, Some(b'}') => { self.offset += 1; break; }, _ => return Err("expected , or }".into()) }
        }
        Ok(Json::Object(fields))
    }
}

fn json(value: &Json) -> String { let mut out = String::new(); value.encode(&mut out); out }
fn response(id: &Json, result: &str) -> String {
    let mut out = format!("{{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":", json(id));
    out.push_str(result); out.push('}'); out
}
fn error(id: &Json, code: i64, message: &str) -> String {
    let mut out = format!("{{\"jsonrpc\":\"2.0\",\"id\":{},\"error\":{{\"code\":{code},\"message\":", json(id));
    quote(message, &mut out); out.push_str("}}"); out
}
fn text_result(text: &str, is_error: bool) -> String {
    let mut out = String::from("{\"content\":[{\"type\":\"text\",\"text\":");
    quote(text, &mut out); out.push_str("}]");
    if is_error { out.push_str(",\"isError\":true"); }
    out.push('}'); out
}

fn tools() -> &'static str {
    r#"{"tools":[
      {"name":"game_status","description":"Get live scene, frame, screen size, map dimensions, camera position and player state.","inputSchema":{"type":"object","properties":{},"additionalProperties":false}},
      {"name":"get_surroundings","description":"Inspect nearby tile IDs and events on the current map.","inputSchema":{"type":"object","properties":{"radius":{"type":"integer","minimum":1,"maximum":15,"default":5}},"additionalProperties":false}},
      {"name":"world_map","description":"List all game maps, dimensions, metadata positions, and explicit map-connection offsets.","inputSchema":{"type":"object","properties":{},"additionalProperties":false}},
      {"name":"map_preview","description":"Get actual RGSS tile layers and tileset filenames for one map.","inputSchema":{"type":"object","properties":{"map_id":{"type":"integer","minimum":1}},"required":["map_id"],"additionalProperties":false}},
      {"name":"teleport","description":"Transfer the player to any in-bounds tile on a game map.","inputSchema":{"type":"object","properties":{"map_id":{"type":"integer","minimum":1},"x":{"type":"integer","minimum":0},"y":{"type":"integer","minimum":0}},"required":["map_id","x","y"],"additionalProperties":false}},
      {"name":"pokemon_essentials_movedex","description":"List all loaded game moves with power, type, category, accuracy, PP and descriptions.","inputSchema":{"type":"object","properties":{},"additionalProperties":false}},
      {"name":"pokemon_essentials_add_item","description":"Add a quantity of a game item to the current player bag, respecting capacity.","inputSchema":{"type":"object","properties":{"item":{"type":"string"},"quantity":{"type":"integer","minimum":1,"maximum":9999,"default":1}},"required":["item"],"additionalProperties":false}},
      {"name":"pokemon_essentials_itemdex","description":"List all loaded game items with pockets, prices, use modes, flags and taught moves.","inputSchema":{"type":"object","properties":{},"additionalProperties":false}},
      {"name":"pokemon_essentials_pokedex","description":"List all loaded game species and forms with known acquisition evidence.","inputSchema":{"type":"object","properties":{},"additionalProperties":false}},
      {"name":"pokemon_essentials_species_entry","description":"Inspect game-specific species stats, full learnsets, evolution rules, abilities and encounter locations.","inputSchema":{"type":"object","properties":{"species":{"type":"string"}},"required":["species"],"additionalProperties":false}},
      {"name":"pokemon_essentials_status","description":"Check Pokémon Essentials availability, trainer/map state and battle readiness.","inputSchema":{"type":"object","properties":{},"additionalProperties":false}},
      {"name":"pokemon_essentials_party","description":"List the Pokémon Essentials player's party.","inputSchema":{"type":"object","properties":{},"additionalProperties":false}},
      {"name":"pokemon_essentials_info","description":"Get a party Pokémon's stats, IVs, EVs, moves/PP and Essentials extension data.","inputSchema":{"type":"object","properties":{"slot":{"type":"integer","minimum":0,"maximum":5}},"required":["slot"],"additionalProperties":false}},
      {"name":"pokemon_essentials_encounters","description":"Inspect current-map Essentials encounter tables, levels and per-slot selection percentages.","inputSchema":{"type":"object","properties":{},"additionalProperties":false}},
      {"name":"pokemon_essentials_npcs","description":"Inspect current-map events/NPCs, positions, page triggers, dialogue and likely trainer scripts.","inputSchema":{"type":"object","properties":{},"additionalProperties":false}},
      {"name":"pokemon_essentials_battle","description":"Inspect battle battlers, HP/status/stages/moves/PP and available fight/switch/bag/run options.","inputSchema":{"type":"object","properties":{},"additionalProperties":false}},
      {"name":"pokemon_essentials_wild_encounter","description":"Start a wild battle with the given species and optional level, shiny, form, gender, nature, ability, held item, max IVs and friendship. Only runs on the overworld with no battle, menu or dialogue open; the battle starts on the next game frame.","inputSchema":{"type":"object","properties":{"species":{"type":"string"},"level":{"type":"integer","minimum":1,"maximum":100,"default":5},"shiny":{"type":"boolean"},"form":{"type":"integer","minimum":0},"gender":{"type":"string","enum":["male","female"]},"nature":{"type":"string"},"ability":{"type":"string"},"item":{"type":"string"},"max_ivs":{"type":"boolean"},"friendship":{"type":"integer","minimum":0,"maximum":255}},"required":["species"],"additionalProperties":false}},
      {"name":"pokemon_essentials_debug","description":"Read or set Pokémon Essentials debug mode ($DEBUG), which enables the game's debug menus and shortcuts. Omit enabled to read the current value; set it to true or false to change it.","inputSchema":{"type":"object","properties":{"enabled":{"type":"boolean"}},"additionalProperties":false}},
      {"name":"press_key","description":"Press a control through RGSS input for 1–60 game updates.","inputSchema":{"type":"object","properties":{"key":{"type":"string","enum":["up","down","left","right","action","cancel"]},"frames":{"type":"integer","minimum":1,"maximum":60,"default":1}},"required":["key"],"additionalProperties":false}},
      {"name":"release_all_keys","description":"Release controls held through this API.","inputSchema":{"type":"object","properties":{},"additionalProperties":false}}
    ]}"#
}

fn call_game(path: &str) -> Result<String, String> {
    let host = std::env::var("RSXP_API_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let port = std::env::var("RSXP_API_PORT").unwrap_or_else(|_| "47821".into());
    let mut stream = TcpStream::connect(format!("{host}:{port}"))
        .map_err(|e| format!("cannot connect to game API at {host}:{port}: {e}"))?;
    let timeout = Some(Duration::from_secs(10));
    stream.set_read_timeout(timeout).map_err(|e| e.to_string())?;
    stream.set_write_timeout(timeout).map_err(|e| e.to_string())?;
    let method = if path == "/v1/input/held" { "DELETE" } else if path.starts_with("/v1/input/press") || path.starts_with("/v1/game/teleport") || path.starts_with("/v1/pokemon-essentials/inventory/add") || path.starts_with("/v1/pokemon-essentials/wild-encounter") || path.starts_with("/v1/pokemon-essentials/debug?") { "POST" } else { "GET" };
    write!(stream, "{method} {path} HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: close\r\n\r\n").map_err(|e| e.to_string())?;
    let mut response = String::new();
    BufReader::new(stream).read_to_string(&mut response).map_err(|e| format!("game API read failed: {e}"))?;
    let (headers, body) = response.split_once("\r\n\r\n").ok_or("invalid HTTP response from game API")?;
    let status = headers.lines().next().unwrap_or_default();
    if !status.contains(" 200 ") { 
        let value = Parser::parse(body)?;
        return Err(value.get("error").and_then(Json::string).unwrap_or(status).to_string());
    }
    let value = Parser::parse(body)?;
    if let Some(message) = value.get("error").and_then(Json::string) { return Err(message.to_string()); }
    Ok(body.to_string())
}

/// Builds the wild-encounter endpoint path from MCP tool arguments, validating
/// values before they reach the game bridge.
fn wild_encounter_path(args: &Json) -> Result<String, String> {
    let species = args.get("species").and_then(Json::string).unwrap_or("");
    if species.is_empty() || !species.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err("species must be a game species ID".into());
    }
    let mut path = format!("/v1/pokemon-essentials/wild-encounter?species={species}");
    if let Some(level) = args.get("level").and_then(Json::integer) {
        if !(1..=100).contains(&level) {
            return Err("level must be 1..100".into());
        }
        path.push_str(&format!("&level={level}"));
    }
    if let Some(form) = args.get("form").and_then(Json::integer) {
        if form < 0 {
            return Err("form must be zero or greater".into());
        }
        path.push_str(&format!("&form={form}"));
    }
    if let Some(friendship) = args.get("friendship").and_then(Json::integer) {
        if !(0..=255).contains(&friendship) {
            return Err("friendship must be 0..255".into());
        }
        path.push_str(&format!("&friendship={friendship}"));
    }
    for field in ["gender", "nature", "ability", "item"] {
        if let Some(value) = args.get(field).and_then(Json::string) {
            if value.is_empty() {
                continue;
            }
            if !value.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_') {
                return Err(format!("{field} contains invalid characters"));
            }
            path.push_str(&format!("&{field}={value}"));
        }
    }
    for field in ["shiny", "max_ivs"] {
        if let Some(value) = args.get(field) {
            let flag = match value {
                Json::Bool(flag) => *flag,
                _ => return Err(format!("{field} must be a boolean")),
            };
            path.push_str(&format!("&{field}={flag}"));
        }
    }
    Ok(path)
}

fn handle(message: Json) -> Option<String> {    let id = message.get("id")?;
    let method = message.get("method").and_then(Json::string).unwrap_or("");
    let params = message.get("params").unwrap_or(&Json::Null);
    match method {
        "initialize" => {
            let requested = params.get("protocolVersion").and_then(Json::string).unwrap_or("");
            let version = if ["2024-11-05", "2025-03-26", "2025-06-18"].contains(&requested) {
                requested
            } else {
                "2025-06-18"
            };
            let result = format!(
                "{{\"protocolVersion\":{},\"capabilities\":{{\"tools\":{{\"listChanged\":false}}}},\"serverInfo\":{{\"name\":\"rsxp-game-api\",\"version\":\"0.1.0\"}}}}",
                { let mut encoded = String::new(); quote(version, &mut encoded); encoded }
            );
            Some(response(id, &result))
        }
        "ping" => Some(response(id, "{}")),
        "tools/list" => Some(response(id, tools())),
        "tools/call" => {
            let name = params.get("name").and_then(Json::string).unwrap_or("");
            let args_obj = params.get("arguments").unwrap_or(&Json::Null);
            let result = match name {
                "game_status" => call_game("/v1/game/status"),
                "pokemon_essentials_movedex" => call_game("/v1/pokemon-essentials/movedex"),
                "pokemon_essentials_add_item" => {
                    let item = args_obj.get("item").and_then(Json::string).unwrap_or("");
                    let quantity = args_obj.get("quantity").map(Json::integer).unwrap_or(Some(1));
                    if item.is_empty() || !item.bytes().all(|byte|byte.is_ascii_alphanumeric() || byte==b'_') {
                        Err("item must be a game item ID".into())
                    } else if !quantity.is_some_and(|quantity|(1..=9999).contains(&quantity)) { Err("quantity must be an integer in 1..9999".into()) }
                    else { let quantity=quantity.unwrap(); call_game(&format!("/v1/pokemon-essentials/inventory/add?item={item}&quantity={quantity}")) }
                }
                "pokemon_essentials_itemdex" => call_game("/v1/pokemon-essentials/itemdex"),
                "pokemon_essentials_pokedex" => call_game("/v1/pokemon-essentials/pokedex"),
                "pokemon_essentials_species_entry" => {
                    let species = args_obj.get("species").and_then(Json::string).unwrap_or("");
                    if species.is_empty() || !species.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_') {
                        Err("species must be a game species/form ID".into())
                    } else { call_game(&format!("/v1/pokemon-essentials/species/{species}")) }
                }
                "pokemon_essentials_status" => call_game("/v1/pokemon-essentials/status"),
                "pokemon_essentials_party" => call_game("/v1/pokemon-essentials/party"),
                "pokemon_essentials_info" => {
                    let slot = args_obj.get("slot").and_then(Json::integer).unwrap_or(-1);
                    if !(0..=5).contains(&slot) { Err("slot must be 0..5".into()) }
                    else { call_game(&format!("/v1/pokemon-essentials/party/{slot}")) }
                }
                "pokemon_essentials_encounters" => call_game("/v1/pokemon-essentials/encounters/current-map"),
                "pokemon_essentials_npcs" => call_game("/v1/pokemon-essentials/npcs/current-map"),
                "pokemon_essentials_battle" => call_game("/v1/pokemon-essentials/battle"),
                "pokemon_essentials_wild_encounter" => {
                    wild_encounter_path(args_obj).and_then(|path| call_game(&path))
                }
                "pokemon_essentials_debug" => match args_obj.get("enabled") {
                    Some(Json::Bool(enabled)) => {
                        call_game(&format!("/v1/pokemon-essentials/debug?enabled={enabled}"))
                    }
                    Some(_) => Err("enabled must be a boolean".into()),
                    None => call_game("/v1/pokemon-essentials/debug"),
                },
                "release_all_keys" => call_game("/v1/input/held"),
                "get_surroundings" => {
                    let radius = args_obj.get("radius").and_then(Json::integer).unwrap_or(5);
                    if !(1..=15).contains(&radius) { Err("radius must be 1..15".into()) }
                    else { call_game(&format!("/v1/game/surroundings?radius={radius}")) }
                }
                "world_map" => call_game("/v1/game/world-map"),
                "map_preview" => {
                    let map_id = args_obj.get("map_id").and_then(Json::integer).unwrap_or(-1);
                    if map_id < 1 { Err("map_id must be positive".into()) }
                    else { call_game(&format!("/v1/game/maps/{map_id}/tiles")) }
                }
                "teleport" => {
                    let map_id = args_obj.get("map_id").and_then(Json::integer).unwrap_or(-1);
                    let x = args_obj.get("x").and_then(Json::integer).unwrap_or(-1);
                    let y = args_obj.get("y").and_then(Json::integer).unwrap_or(-1);
                    call_game(&format!("/v1/game/teleport?map_id={map_id}&x={x}&y={y}"))
                }
                "press_key" => {
                    let key = args_obj.get("key").and_then(Json::string).unwrap_or("");
                    let frames = args_obj.get("frames").and_then(Json::integer).unwrap_or(1);
                    if !["up", "down", "left", "right", "action", "cancel"].contains(&key) { Err("key must be up, down, left, right, action, or cancel".into()) }
                    else if !(1..=60).contains(&frames) { Err("frames must be 1..60".into()) }
                    else { call_game(&format!("/v1/input/press?key={key}&frames={frames}")) }
                }
                _ => Err(format!("unknown tool: {name}")),
            };
            let (content, failed) = match result {
                Ok(raw) => match Parser::parse(&raw) {
                    Ok(value) => (json(&value), false),
                    Err(_) => (raw, false),
                },
                Err(message) => (message, true),
            };
            Some(response(id, &text_result(&content, failed)))
        }
        _ => Some(error(id, -32601, &format!("Method not found: {method}"))),
    }
}

fn main() -> io::Result<()> {
    #[cfg(feature = "api-docs")]
    if std::env::args().any(|arg| arg == "--openapi") {
        use utoipa::OpenApi as _;
        println!("{}", openapi::ApiDoc::openapi().to_pretty_json().map_err(io::Error::other)?);
        return Ok(());
    }
    let stdin = io::stdin();
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() { continue; }
        let reply = match Parser::parse(&line) {
            Ok(message) => handle(message),
            Err(message) => Some(error(&Json::Null, -32700, &format!("Parse error: {message}"))),
        };
        if let Some(reply) = reply { writeln!(stdout, "{reply}")?; stdout.flush()?; }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_requests_and_supplementary_unicode() {
        let value = Parser::parse(
            r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"press_key","arguments":{"key":"up","label":"\ud83d\ude80"}}}"#,
        )
        .unwrap();
        assert_eq!(value.get("id").and_then(Json::integer), Some(7));
        assert_eq!(
            value.get("params").and_then(|v| v.get("arguments"))
                .and_then(|v| v.get("label")).and_then(Json::string),
            Some("🚀")
        );
    }

    #[test]
    fn rejects_malformed_numbers_and_unpaired_surrogates() {
        assert!(Parser::parse(r#"{"id":01}"#).is_err());
        assert!(Parser::parse(r#"{"label":"\ud83d"}"#).is_err());
    }

    #[test]
    fn invalid_control_arguments_return_mcp_tool_error() {
        let request = Parser::parse(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"press_key","arguments":{"key":"sideways"}}}"#,
        )
        .unwrap();
        let reply = handle(request).unwrap();
        assert!(reply.contains("\"isError\":true"));
        assert!(reply.contains("key must be up"));
    }

    #[test]
    fn essentials_tools_are_advertised_and_slot_is_bounded() {
        let listing = Parser::parse(tools()).unwrap();
        let names = listing.get("tools").and_then(|value| match value { Json::Array(values) => Some(values), _ => None }).unwrap();
        for name in ["pokemon_essentials_status", "pokemon_essentials_party", "pokemon_essentials_info", "pokemon_essentials_encounters", "pokemon_essentials_npcs", "pokemon_essentials_battle", "pokemon_essentials_wild_encounter", "pokemon_essentials_debug"] {
            assert!(names.iter().any(|tool| tool.get("name").and_then(Json::string) == Some(name)), "missing {name}");
        }
        assert!(names.iter().any(|tool| tool.get("name").and_then(Json::string) == Some("world_map")));
        assert!(names.iter().any(|tool| tool.get("name").and_then(Json::string) == Some("teleport")));
        let request = Parser::parse(
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"pokemon_essentials_info","arguments":{"slot":6}}}"#,
        ).unwrap();
        let reply = handle(request).unwrap();
        assert!(reply.contains("slot must be 0..5"));
        assert!(reply.contains("\"isError\":true"));
    }

    #[test]
    fn wild_encounter_tool_builds_and_validates_paths() {
        assert_eq!(
            wild_encounter_path(
                &Parser::parse(r#"{"species":"PIKACHU","level":30,"shiny":true,"nature":"JOLLY"}"#)
                    .unwrap()
            )
            .unwrap(),
            "/v1/pokemon-essentials/wild-encounter?species=PIKACHU&level=30&nature=JOLLY&shiny=true"
        );
        assert!(wild_encounter_path(&Parser::parse(r#"{"species":"PIKACHU","level":0}"#).unwrap()).is_err());
        assert!(wild_encounter_path(&Parser::parse(r#"{"species":"PI/KACHU"}"#).unwrap()).is_err());
        assert!(wild_encounter_path(&Parser::parse(r#"{"species":"PIKACHU","friendship":300}"#).unwrap()).is_err());
    }

    #[cfg(feature = "api-docs")]
    #[test]
    fn generated_openapi_lists_the_live_and_essentials_endpoints() {
        use utoipa::OpenApi as _;
        let spec = openapi::ApiDoc::openapi().to_pretty_json().unwrap();
        Parser::parse(&spec).expect("generated OpenAPI should be valid JSON");
        for endpoint in [
            "/v1/game/status",
            "/v1/game/surroundings",
            "/v1/game/world-map",
            "/v1/game/maps/{map_id}/tiles",
            "/v1/game/teleport",
            "/v1/pokemon-essentials/status",
            "/v1/pokemon-essentials/party",
            "/v1/pokemon-essentials/party/{slot}",
            "/v1/pokemon-essentials/encounters/current-map",
            "/v1/pokemon-essentials/npcs/current-map",
            "/v1/pokemon-essentials/battle",
            "/v1/pokemon-essentials/wild-encounter",
            "/v1/pokemon-essentials/debug",
            "/v1/input/press",
            "/v1/input/held",
        ] {
            assert!(spec.contains(endpoint), "missing OpenAPI path {endpoint}");
        }
    }
}
