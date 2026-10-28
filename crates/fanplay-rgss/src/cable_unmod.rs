// SPDX-License-Identifier: GPL-2.0-or-later
//! Unmodified Cable Club's line-based rendezvous and peer relay.
//!
//! This preserves upstream behavior for clients that search by trainer public
//! ID: each client enters the other trainer's ID, and the server pairs them
//! only when both searches are reciprocal. EricLostie's shared-room-code
//! variant is implemented separately in [`crate::cable_club`].
//!
//! ## Protocol
//! Records are one line of comma-separated fields, with `\,` and `\\` escaping
//! a literal comma or backslash. A client opens with
//! `find,<version>,<peer-id>,<name>,<trainer-id>,<trainer-type>,<win-msg>,<losemsg>,<party…>`,
//! where `<peer-id>` is the trainer ID it is looking for and `<trainer-id>` is
//! its own (masked to 16 bits). Two `searching` clients match only when each
//! names the other's ID, after which the server sends each side
//! `found,<0|1>,<peer name>,<trainer type>,<win>,<lose>,<party…>,0` and
//! forwards every later line verbatim to the peer.
//!
//! ## API
//! [`start`] / [`stop`] manage a single process-wide server and [`status`]
//! returns a UI snapshot. [`save_client_endpoint`] writes the
//! `Data/serverinfo.ini` endpoint the client reads. Records are capped at
//! 256 KiB and the server at 64 simultaneous clients; each client has its own
//! thread.

use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Write},
    net::{Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, OnceLock,
    },
    thread,
    time::Duration,
};

const MAX_RECORD_BYTES: usize = 256 * 1024;
const MAX_PARTY_FIELDS: usize = 8192;
const MAX_CLIENTS: usize = 64;
const GAME_VERSION: (u32, u32, u32) = (1, 0, 0);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientStatus {
    pub name: String,
    pub address: String,
    pub peer_id: Option<u32>,
    pub state: &'static str,
}

#[derive(Clone, Debug, Default)]
pub struct ServerStatus {
    pub address: Option<String>,
    pub clients: Vec<ClientStatus>,
    pub error: Option<String>,
}

struct ServerHandle {
    stop: Arc<AtomicBool>,
    thread: thread::JoinHandle<()>,
    status: Arc<Mutex<ServerStatus>>,
    wake_address: SocketAddr,
}

static SERVER: OnceLock<Mutex<Option<ServerHandle>>> = OnceLock::new();
static NEXT_CLIENT: AtomicU64 = AtomicU64::new(1);

/// Starts a Cable Club server, binding the provided address (use `0.0.0.0`
/// when other computers must connect).
pub fn start(bind: &str, port: u16) -> Result<(), String> {
    if port == 0 {
        return Err("Cable Club port must be between 1 and 65535".into());
    }
    let server = SERVER.get_or_init(|| Mutex::new(None));
    let mut active = server
        .lock()
        .map_err(|_| "Cable Club state is unavailable")?;
    if active.is_some() {
        return Err("Cable Club server is already running".into());
    }
    let listener = TcpListener::bind((bind, port))
        .map_err(|error| format!("cannot listen on {bind}:{port}: {error}"))?;
    let bound_address = listener
        .local_addr()
        .map_err(|error| format!("cannot inspect Cable Club listener: {error}"))?;
    let wake_address = match bound_address {
        SocketAddr::V4(mut address) if address.ip().is_unspecified() => {
            address.set_ip(Ipv4Addr::LOCALHOST);
            SocketAddr::V4(address)
        }
        SocketAddr::V6(mut address) if address.ip().is_unspecified() => {
            address.set_ip(Ipv6Addr::LOCALHOST);
            SocketAddr::V6(address)
        }
        address => address,
    };
    let address = bound_address.to_string();
    let stop = Arc::new(AtomicBool::new(false));
    let status = Arc::new(Mutex::new(ServerStatus {
        address: Some(address.clone()),
        ..ServerStatus::default()
    }));
    let stop_thread = Arc::clone(&stop);
    let status_thread = Arc::clone(&status);
    let thread = thread::Builder::new()
        .name("fanplay-cable-club".into())
        .spawn(move || accept_loop(listener, stop_thread, status_thread))
        .map_err(|error| format!("cannot start Cable Club server: {error}"))?;
    eprintln!("[CableClub] listening on {address}");
    *active = Some(ServerHandle {
        stop,
        thread,
        status,
        wake_address,
    });
    Ok(())
}

/// Stops the listener and closes active peer sockets.
pub fn stop() {
    let server = SERVER.get_or_init(|| Mutex::new(None));
    let handle = server.lock().ok().and_then(|mut active| active.take());
    if let Some(handle) = handle {
        handle.stop.store(true, Ordering::Release);
        // Wake the blocking accept so shutdown does not depend on nonblocking
        // Winsock ioctls, which are unreliable in some Wine environments.
        let _ = TcpStream::connect_timeout(&handle.wake_address, Duration::from_millis(100));
        let _ = handle.thread.join();
    }
}

pub fn status() -> ServerStatus {
    SERVER
        .get()
        .and_then(|server| server.lock().ok())
        .and_then(|active| active.as_ref().map(|handle| Arc::clone(&handle.status)))
        .and_then(|status| status.lock().ok().map(|status| status.clone()))
        .unwrap_or_default()
}

#[derive(Clone)]
struct Client {
    name: String,
    address: String,
    peer_id: u32,
    trainer_id: u32,
    // `found` payload fields after the command: name, trainer type, win/lose
    // message IDs, and the game's serialized party.
    found_fields: Vec<String>,
    peer: Option<u64>,
    state: &'static str,
    writer: Arc<Mutex<TcpStream>>,
}

fn accept_loop(listener: TcpListener, stop: Arc<AtomicBool>, status: Arc<Mutex<ServerStatus>>) {
    let clients = Arc::new(Mutex::new(HashMap::<u64, Client>::new()));
    while !stop.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, address)) => {
                if stop.load(Ordering::Acquire) {
                    break;
                }
                let id = NEXT_CLIENT.fetch_add(1, Ordering::Relaxed);
                let writer = match stream.try_clone() {
                    Ok(writer) => Arc::new(Mutex::new(writer)),
                    Err(error) => {
                        eprintln!("[CableClub] cannot clone client stream: {error}");
                        continue;
                    }
                };
                if let Ok(writer) = writer.lock() {
                    let _ = writer.set_write_timeout(Some(Duration::from_secs(2)));
                }
                let client = Client {
                    name: "Connecting".into(),
                    address: address.to_string(),
                    peer_id: 0,
                    trainer_id: 0,
                    found_fields: Vec::new(),
                    peer: None,
                    state: "connecting",
                    writer: Arc::clone(&writer),
                };
                if let Ok(mut peers) = clients.lock() {
                    if peers.len() >= MAX_CLIENTS {
                        drop(peers);
                        let _ =
                            write_record(&writer, &["disconnect".into(), "server is full".into()]);
                        let _ = writer
                            .lock()
                            .map(|stream| stream.shutdown(std::net::Shutdown::Both));
                        continue;
                    }
                    peers.insert(id, client);
                }
                update_status(&clients, &status);
                let clients_thread = Arc::clone(&clients);
                let status_thread = Arc::clone(&status);
                let _ = thread::Builder::new()
                    .name(format!("fanplay-cable-client-{id}"))
                    .spawn(move || client_loop(id, stream, clients_thread, status_thread));
            }
            Err(error) => {
                if stop.load(Ordering::Acquire) {
                    break;
                }
                if let Ok(mut current) = status.lock() {
                    current.error = Some(format!("listener error: {error}"));
                }
                thread::sleep(Duration::from_millis(100));
            }
        }
    }
    let peers: Vec<Arc<Mutex<TcpStream>>> = clients
        .lock()
        .map(|peers| {
            peers
                .values()
                .map(|client| Arc::clone(&client.writer))
                .collect()
        })
        .unwrap_or_default();
    for writer in peers {
        if let Ok(stream) = writer.lock() {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
    }
    if let Ok(mut current) = status.lock() {
        current.address = None;
        current.clients.clear();
    }
}

fn client_loop(
    id: u64,
    stream: TcpStream,
    clients: Arc<Mutex<HashMap<u64, Client>>>,
    status: Arc<Mutex<ServerStatus>>,
) {
    let mut reader = BufReader::new(stream);
    let mut line = Vec::new();
    loop {
        line.clear();
        match reader
            .by_ref()
            .take((MAX_RECORD_BYTES + 1) as u64)
            .read_until(b'\n', &mut line)
        {
            Ok(0) => break,
            Ok(size) if size > MAX_RECORD_BYTES => {
                send_disconnect(&clients, id, "record too large");
                break;
            }
            Ok(_) => {
                while matches!(line.last(), Some(b'\n' | b'\r')) {
                    line.pop();
                }
                let Ok(record) = std::str::from_utf8(&line) else {
                    send_disconnect(&clients, id, "invalid UTF-8");
                    break;
                };
                if !handle_record(&clients, id, record) {
                    break;
                }
                update_status(&clients, &status);
            }
            Err(error) => {
                if !matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) {
                    break;
                }
            }
        }
    }
    disconnect(&clients, id, "client disconnected");
    update_status(&clients, &status);
}

fn handle_record(clients: &Arc<Mutex<HashMap<u64, Client>>>, id: u64, line: &str) -> bool {
    let Some(fields) = parse_record(line) else {
        send_disconnect(clients, id, "invalid record");
        return false;
    };
    let command = fields.first().map(String::as_str).unwrap_or("");
    let state = clients
        .lock()
        .ok()
        .and_then(|peers| peers.get(&id).map(|client| client.state));
    match (state, command) {
        (Some("connecting"), "find") => {
            if fields.len() < 9 || fields.len() > MAX_PARTY_FIELDS + 9 {
                send_disconnect(clients, id, "invalid party");
                return false;
            }
            let Some(version) = fields.get(1).and_then(|value| parse_version(value)) else {
                send_disconnect(clients, id, "invalid version");
                return false;
            };
            if version < GAME_VERSION {
                send_disconnect(clients, id, "invalid version");
                return false;
            }
            let (Some(peer_id), Some(trainer_id), Some(_win_text), Some(_lose_text)) = (
                fields.get(2).and_then(|value| value.parse::<u32>().ok()),
                fields.get(4).and_then(|value| value.parse::<u32>().ok()),
                fields.get(6).and_then(|value| value.parse::<i32>().ok()),
                fields.get(7).and_then(|value| value.parse::<i32>().ok()),
            ) else {
                send_disconnect(clients, id, "invalid trainer data");
                return false;
            };
            let name = fields.get(3).cloned().unwrap_or_default();
            if name.chars().count() > 10 || name.is_empty() {
                send_disconnect(clients, id, "invalid trainer name");
                return false;
            }
            let mut found_fields = Vec::with_capacity(fields.len().saturating_sub(4));
            found_fields.push(fields[3].clone());
            found_fields.extend(fields[5..].iter().cloned());
            let match_id = if let Ok(mut peers) = clients.lock() {
                if let Some(client) = peers.get_mut(&id) {
                    client.name = name;
                    client.peer_id = peer_id;
                    client.trainer_id = trainer_id & 0xFFFF;
                    client.found_fields = found_fields;
                    client.state = "searching";
                }
                let match_id = peers.iter().find_map(|(other_id, other)| {
                    (*other_id != id
                        && other.state == "searching"
                        && other.trainer_id == peer_id
                        && other.peer_id == (trainer_id & 0xFFFF))
                        .then_some(*other_id)
                });
                match_id
            } else {
                None
            };
            if let Some(other) = match_id {
                connect_peers(clients, id, other);
            }
            true
        }
        (Some("paired"), _) => {
            let peer_writer = clients.lock().ok().and_then(|peers| {
                let peer_id = peers.get(&id)?.peer?;
                peers.get(&peer_id).map(|peer| Arc::clone(&peer.writer))
            });
            if let Some(writer) = peer_writer {
                if let Ok(mut writer) = writer.lock() {
                    let _ = writer.write_all(line.as_bytes());
                    let _ = writer.write_all(b"\n");
                    let _ = writer.flush();
                }
            }
            true
        }
        (Some("searching"), _) => true, // Official server drops early messages.
        _ => {
            send_disconnect(clients, id, "bad assert");
            false
        }
    }
}

fn connect_peers(clients: &Arc<Mutex<HashMap<u64, Client>>>, first: u64, second: u64) {
    let Ok(mut peers) = clients.lock() else {
        return;
    };
    let (Some(first_client), Some(second_client)) =
        (peers.get(&first).cloned(), peers.get(&second).cloned())
    else {
        return;
    };
    let first_record = found_record(1, &second_client.found_fields);
    let second_record = found_record(0, &first_client.found_fields);
    if write_record(&first_client.writer, &first_record).is_err()
        || write_record(&second_client.writer, &second_record).is_err()
    {
        return;
    }
    if let Some(client) = peers.get_mut(&first) {
        client.peer = Some(second);
        client.state = "paired";
    }
    if let Some(client) = peers.get_mut(&second) {
        client.peer = Some(first);
        client.state = "paired";
    }
}

fn found_record(number: u8, fields: &[String]) -> Vec<String> {
    let mut result = vec!["found".into(), number.to_string()];
    result.extend(fields.iter().cloned());
    result.push("0".into()); // Number of server-side battle presets.
    result
}

fn parse_record(line: &str) -> Option<Vec<String>> {
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut chars = line.chars();
    while let Some(character) = chars.next() {
        match character {
            ',' => {
                fields.push(std::mem::take(&mut field));
            }
            '\\' => match chars.next() {
                Some(next) => field.push(next),
                None => return None,
            },
            _ => field.push(character),
        }
        if fields.len() > MAX_PARTY_FIELDS + 9 || field.len() > MAX_RECORD_BYTES {
            return None;
        }
    }
    fields.push(field);
    Some(fields)
}

fn encode_record(fields: &[String]) -> String {
    fields
        .iter()
        .map(|field| field.replace('\\', "\\\\").replace(',', "\\,"))
        .collect::<Vec<_>>()
        .join(",")
}

fn write_record(writer: &Arc<Mutex<TcpStream>>, fields: &[String]) -> std::io::Result<()> {
    let mut stream = writer
        .lock()
        .map_err(|_| std::io::Error::other("client stream is unavailable"))?;
    stream.write_all(encode_record(fields).as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()
}

fn send_disconnect(clients: &Arc<Mutex<HashMap<u64, Client>>>, id: u64, reason: &str) {
    let writer = clients
        .lock()
        .ok()
        .and_then(|peers| peers.get(&id).map(|client| Arc::clone(&client.writer)));
    if let Some(writer) = writer {
        let _ = write_record(&writer, &["disconnect".into(), reason.into()]);
    }
}

fn disconnect(clients: &Arc<Mutex<HashMap<u64, Client>>>, id: u64, reason: &str) {
    let Some((removed, peer_id)) = clients.lock().ok().and_then(|mut peers| {
        let removed = peers.remove(&id)?;
        let peer_id = removed.peer;
        if let Some(peer_id) = peer_id {
            if let Some(peer) = peers.get_mut(&peer_id) {
                peer.peer = None;
                peer.state = "disconnected";
            }
        }
        Some((removed, peer_id))
    }) else {
        return;
    };
    let _ = write_record(&removed.writer, &["disconnect".into(), reason.into()]);
    let _ = removed
        .writer
        .lock()
        .map(|stream| stream.shutdown(std::net::Shutdown::Both));
    if let Some(peer_id) = peer_id {
        send_disconnect(clients, peer_id, "peer disconnected");
    }
}

fn update_status(clients: &Arc<Mutex<HashMap<u64, Client>>>, status: &Arc<Mutex<ServerStatus>>) {
    let snapshot = clients
        .lock()
        .map(|peers| {
            let mut clients: Vec<_> = peers
                .values()
                .map(|client| ClientStatus {
                    name: client.name.clone(),
                    address: client.address.clone(),
                    peer_id: (client.state == "searching" || client.state == "paired")
                        .then_some(client.peer_id),
                    state: client.state,
                })
                .collect();
            clients.sort_by(|a, b| a.address.cmp(&b.address));
            clients
        })
        .unwrap_or_default();
    if let Ok(mut current) = status.lock() {
        current.clients = snapshot;
    }
}

fn parse_version(value: &str) -> Option<(u32, u32, u32)> {
    let mut parts = value.split('.').map(str::parse::<u32>);
    let version = (
        parts.next()?.ok()?,
        parts.next()?.ok()?,
        parts.next()?.ok()?,
    );
    parts.next().is_none().then_some(version)
}

/// Configures the official EricLostie client, which reads this file whenever it
/// opens a Cable Club connection.
pub fn save_client_endpoint(host: &str, port: u16) -> Result<std::path::PathBuf, String> {
    if host.trim().is_empty() || host.contains(['\r', '\n', '=']) || port == 0 {
        return Err("Enter a host and a port between 1 and 65535".into());
    }
    let root = std::env::current_dir().map_err(|error| error.to_string())?;
    let data = root.join("Data");
    std::fs::create_dir_all(&data)
        .map_err(|error| format!("cannot create Data directory: {error}"))?;
    let path = data.join("serverinfo.ini");
    let contents = format!("Host={}\nPort={}\n", host.trim(), port);
    std::fs::write(&path, contents)
        .map_err(|error| format!("cannot save server endpoint: {error}"))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_codec_preserves_escaped_commas_and_backslashes() {
        let input = vec!["find".to_string(), "a,b".into(), r"c\d".into()];
        assert_eq!(parse_record(&encode_record(&input)), Some(input));
    }

    #[test]
    fn found_payload_preserves_party_and_appends_empty_rule_list() {
        assert_eq!(
            found_record(
                1,
                &[
                    "Trainer".into(),
                    "TYPE".into(),
                    "0".into(),
                    "0".into(),
                    "1".into(),
                    "Pkmn".into()
                ]
            ),
            ["found", "1", "Trainer", "TYPE", "0", "0", "1", "Pkmn", "0"].map(str::to_owned)
        );
    }

    #[test]
    fn protocol_version_requires_three_numeric_components() {
        assert_eq!(parse_version("1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("1.x.3"), None);
        assert_eq!(parse_version("1.2"), None);
    }

    #[test]
    fn two_clients_match_and_exchange_records_through_the_relay() {
        use std::io::{BufRead, BufReader};

        let probe = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        start("127.0.0.1", port).unwrap();

        let mut first = TcpStream::connect(("127.0.0.1", port)).unwrap();
        first
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut first_reader = BufReader::new(first.try_clone().unwrap());
        first
            .write_all(b"find,1.0.0,2,Alice,1,TRAINER,0,0,1,Pkmn\n")
            .unwrap();
        // Each client is served by its own thread, so wait for Alice to be
        // queued before Bob searches. Otherwise the two `find` records can be
        // processed out of order and swap the 0/1 slots in `found`.
        wait_until_searching("Alice");

        let mut second = TcpStream::connect(("127.0.0.1", port)).unwrap();
        second
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut second_reader = BufReader::new(second.try_clone().unwrap());
        second
            .write_all(b"find,1.0.0,1,Bob,2,TRAINER,0,0,1,Eevee\n")
            .unwrap();

        let mut received = String::new();
        first_reader.read_line(&mut received).unwrap();
        assert_eq!(received.trim_end(), "found,0,Bob,TRAINER,0,0,1,Eevee,0");
        received.clear();
        second_reader.read_line(&mut received).unwrap();
        assert_eq!(received.trim_end(), "found,1,Alice,TRAINER,0,0,1,Pkmn,0");

        first.write_all(b"battle,turn,1\n").unwrap();
        received.clear();
        second_reader.read_line(&mut received).unwrap();
        assert_eq!(received.trim_end(), "battle,turn,1");
        stop();
    }

    fn wait_until_searching(name: &str) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !status()
            .clients
            .iter()
            .any(|client| client.name == name && client.state == "searching")
        {
            assert!(
                std::time::Instant::now() < deadline,
                "{name} never reached the searching state"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
