//! Room envelopes shared by WebTransport, binary WebSocket and standard JSON.
use hyve_core::{HyveError, HyveResult, Timestamp, Ulid, Value};
use hyve_serde::{AlignedBytes, Binary, FORMAT_FINGERPRINT, from_json, to_json};
use std::collections::VecDeque;
use std::time::Duration;

/// WebSocket subprotocol for the archived envelope.
pub const BINARY_PROTOCOL: &str = "hyve.room.v1";
/// WebSocket subprotocol for the JSON envelope.
pub const JSON_PROTOCOL: &str = "hyve.room.json.v1";
/// Maximum encoded message size on every carrier.
pub const MAX_FRAME: usize = 64 * 1024;
/// Encoded size limit of ephemeral hints, including their envelope.
pub const MAX_HINT: usize = 1024;

/// The negotiated envelope encoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    /// A validated Hyve archive, including the codec fingerprint.
    Binary,
    /// A UTF-8 JSON object.
    Json,
}

impl Codec {
    /// Encodes an envelope for a frame boundary.
    pub fn encode(self, message: &Value) -> HyveResult<Vec<u8>> {
        let bytes = match self {
            Self::Binary => {
                let archive = message.to_binary()?;
                let mut bytes = Vec::with_capacity(8 + archive.len());
                bytes.extend(FORMAT_FINGERPRINT.to_le_bytes());
                bytes.extend_from_slice(&archive);
                bytes
            }
            Self::Json => to_json(message).into_bytes(),
        };
        if bytes.len() > MAX_FRAME {
            return Err(HyveError::invalid_request("room frame too large"));
        }
        Ok(bytes)
    }

    /// Checks the bounded input before reading the envelope.
    pub fn decode(self, bytes: &[u8]) -> HyveResult<Value> {
        if bytes.len() > MAX_FRAME {
            return Err(HyveError::invalid_request("room frame too large"));
        }
        match self {
            Self::Binary => {
                let archive = bytes
                    .strip_prefix(&FORMAT_FINGERPRINT.to_le_bytes()[..])
                    .ok_or_else(|| HyveError::invalid_request("unknown room archive format"))?;
                Value::from_binary(&AlignedBytes::copy_from(archive))
                    .map_err(|e| e.into_request_error())
            }
            Self::Json => from_json(
                std::str::from_utf8(bytes)
                    .map_err(|_| HyveError::invalid_request("invalid UTF-8"))?,
            )
            .map_err(|e| e.into_request_error()),
        }
    }
}

fn text<'a>(value: &'a Value, field: &str) -> HyveResult<&'a str> {
    match value.get(field) {
        Some(Value::String(s)) if !s.is_empty() && s.len() <= 128 => Ok(s),
        _ => Err(HyveError::invalid_request(format!(
            "missing or invalid room field {field}"
        ))),
    }
}

/// Reads a nonnegative sequence from a payload or envelope.
pub fn sequence(value: &Value, field: &str) -> HyveResult<i64> {
    match value.get(field) {
        Some(Value::Int(n)) if *n >= 0 => Ok(*n),
        _ => Err(HyveError::invalid_request(format!(
            "invalid sequence field {field}"
        ))),
    }
}

fn common(kind: &str, id: &str, payload: Value) -> Value {
    let mut v = Value::map();
    v.insert("type", kind);
    v.insert("id", id);
    v.insert("payload", payload);
    v
}

/// One client operation. Retrying preserves its ID, type and payload.
#[derive(Clone, Debug, PartialEq)]
pub struct Command {
    /// Application command or room control operation.
    pub kind: String,
    /// Stable client request ID.
    pub id: String,
    /// Application input.
    pub payload: Value,
}

impl Command {
    /// Allocates a fresh client ID.
    pub fn new(kind: &str, payload: Value) -> Self {
        Self {
            kind: kind.into(),
            id: Ulid::new().to_string(),
            payload,
        }
    }
    /// Wire representation.
    pub fn envelope(&self) -> Value {
        common(&self.kind, &self.id, self.payload.clone())
    }
    /// Parses a client envelope.
    pub fn parse(v: &Value) -> HyveResult<Self> {
        let kind = text(v, "type")?;
        if !kind.contains('.')
            || !kind.split('.').all(|s| {
                !s.is_empty()
                    && s.bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            })
        {
            return Err(HyveError::invalid_request("invalid room command type"));
        }
        Ok(Self {
            kind: kind.into(),
            id: text(v, "id")?.into(),
            payload: v
                .get("payload")
                .cloned()
                .ok_or_else(|| HyveError::invalid_request("missing room payload"))?,
        })
    }
}

/// A server event, targeted acknowledgment, snapshot or control frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    /// Event type or protocol control name.
    pub kind: String,
    /// Unique server event identifier.
    pub id: String,
    /// Committed per-room sequence.
    pub seq: i64,
    /// Server epoch microseconds.
    pub ts: i64,
    /// Original client request ID of a directed reply.
    pub request_id: Option<String>,
    /// Event or control body.
    pub payload: Value,
}

impl Event {
    /// Stamps an event on the server.
    pub fn new(kind: &str, seq: i64, payload: Value, request_id: Option<String>) -> Self {
        Self {
            kind: kind.into(),
            id: Ulid::new().to_string(),
            seq,
            ts: Timestamp::now().as_unix_micros(),
            request_id,
            payload,
        }
    }
    /// Wire representation.
    pub fn envelope(&self) -> Value {
        let mut v = common(&self.kind, &self.id, self.payload.clone());
        v.insert("seq", self.seq);
        v.insert("ts", self.ts);
        if let Some(id) = &self.request_id {
            v.insert("requestId", id.as_str());
        }
        v
    }
    /// Parses a server envelope.
    pub fn parse(v: &Value) -> HyveResult<Self> {
        Ok(Self {
            kind: text(v, "type")?.into(),
            id: text(v, "id")?.into(),
            seq: sequence(v, "seq")?,
            ts: sequence(v, "ts")?,
            request_id: v
                .get("requestId")
                .map(|_| text(v, "requestId").map(str::to_owned))
                .transpose()?,
            payload: v
                .get("payload")
                .cloned()
                .ok_or_else(|| HyveError::invalid_request("missing room payload"))?,
        })
    }
}

/// Client-side replay state for a single authenticated principal and room.
#[derive(Default)]
pub struct RoomCursor {
    seq: i64,
    pending: VecDeque<Command>,
}

impl RoomCursor {
    /// Highest contiguous event (or snapshot) applied by the client.
    pub fn sequence(&self) -> i64 {
        self.seq
    }
    /// First command on each new carrier.
    pub fn resume(&self) -> Command {
        let mut value = Value::map();
        value.insert("lastSeq", self.seq);
        Command::new("room.resume", value)
    }
    /// Queues an operation before attempting delivery.
    pub fn enqueue(&mut self, kind: &str, payload: Value) -> HyveResult<&Command> {
        if self.pending.len() >= 256 {
            return Err(HyveError::domain("queue_full", "room outbox is full"));
        }
        let cmd = Command::new(kind, payload);
        let v = cmd.envelope();
        Command::parse(&v)?;
        Codec::Binary.encode(&v)?;
        self.pending.push_back(cmd);
        Ok(self.pending.back().expect("just added"))
    }
    /// Unacknowledged commands in submission order.
    pub fn pending(&self) -> impl Iterator<Item = &Command> {
        self.pending.iter()
    }
    /// Accepts a received event. Returns true for state to deliver to the UI.
    /// A sequence gap leaves the cursor in place so a new resume can repair it.
    pub fn accept(&mut self, event: &Event) -> HyveResult<bool> {
        match event.kind.as_str() {
            "room.ack" | "error" => {
                // Retryable errors leave the original command in the outbox.
                if event.payload.get("retryable") != Some(&Value::Bool(true)) {
                    if let Some(id) = &event.request_id {
                        self.pending.retain(|c| c.id != *id);
                    }
                }
                Ok(false)
            }
            "hello" | "room.ready" | "room.pong" | "room.hinted" => Ok(false),
            "room.snapshot" => {
                self.seq = event.seq;
                Ok(true)
            }
            _ if event.seq <= self.seq => Ok(false),
            _ if self.seq.checked_add(1) == Some(event.seq) => {
                self.seq = event.seq;
                Ok(true)
            }
            _ => Err(HyveError::domain(
                "sequence_gap",
                "resume the room from its last applied sequence",
            )
            .retryable(true)),
        }
    }
}

/// Jittered reconnect delay. Supply fresh random bits per attempt; the
/// delay grows from 100 milliseconds to at most 30 seconds.
pub fn reconnect_delay(attempt: u32, random: u32) -> Duration {
    let ceiling = (100u32 << attempt.min(9)).min(30_000);
    Duration::from_millis(u64::from(ceiling / 2 + random % (ceiling / 2 + 1)))
}
