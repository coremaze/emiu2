//! Who is connected, what their codes are, and who is paired with whom.

use crate::ClientSender;
use emiu2_netplay::{FriendCode, Message};
use std::collections::HashMap;
use std::io::Read;

pub struct Registry {
    clients: HashMap<FriendCode, Client>,
}

struct Client {
    sender: ClientSender,
    paired_with: Option<FriendCode>,
}

pub enum JoinOutcome {
    Paired,
    UnknownCode,
    PeerBusy,
    SelfJoin,
    AlreadyPaired,
}

impl Registry {
    pub fn new() -> Self {
        Self {
            clients: HashMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.clients.len()
    }

    /// Assigns a fresh code to a new client.
    pub fn register(&mut self, sender: ClientSender) -> FriendCode {
        loop {
            let code = FriendCode::from_entropy(random_entropy());
            if self.clients.contains_key(&code) {
                continue;
            }
            self.clients.insert(
                code,
                Client {
                    sender,
                    paired_with: None,
                },
            );
            return code;
        }
    }

    /// Removes a client, notifying its peer if it was paired.
    pub fn unregister(&mut self, code: FriendCode) {
        self.unpair(code);
        self.clients.remove(&code);
    }

    /// Pairs `joiner` with the owner of `target`.
    pub fn join(&mut self, joiner: FriendCode, target: FriendCode) -> JoinOutcome {
        if joiner == target {
            return JoinOutcome::SelfJoin;
        }
        match self.clients.get(&joiner) {
            Some(client) if client.paired_with.is_some() => return JoinOutcome::AlreadyPaired,
            Some(_) => {}
            None => return JoinOutcome::UnknownCode,
        }
        match self.clients.get(&target) {
            Some(client) if client.paired_with.is_some() => return JoinOutcome::PeerBusy,
            Some(_) => {}
            None => return JoinOutcome::UnknownCode,
        }

        self.clients.get_mut(&joiner).unwrap().paired_with = Some(target);
        self.clients.get_mut(&target).unwrap().paired_with = Some(joiner);
        self.send(joiner, Message::Paired);
        self.send(target, Message::Paired);
        JoinOutcome::Paired
    }

    /// Dissolves `code`'s pairing, if any, notifying the peer.
    pub fn unpair(&mut self, code: FriendCode) {
        let Some(peer) = self
            .clients
            .get_mut(&code)
            .and_then(|client| client.paired_with.take())
        else {
            return;
        };
        if let Some(peer_client) = self.clients.get_mut(&peer) {
            peer_client.paired_with = None;
        }
        self.send(peer, Message::PeerLeft);
        self.send(code, Message::PeerLeft);
    }

    /// Forwards a message to `from`'s peer. Unpaired traffic is dropped:
    /// data racing a PeerLeft is normal, not an error.
    pub fn relay(&mut self, from: FriendCode, message: Message) {
        let Some(peer) = self
            .clients
            .get(&from)
            .and_then(|client| client.paired_with)
        else {
            return;
        };
        self.send(peer, message);
    }

    fn send(&self, to: FriendCode, message: Message) {
        if let Some(client) = self.clients.get(&to) {
            // A dead writer is cleaned up by its own connection teardown.
            let _ = client.sender.send(message.encode());
        }
    }
}

/// Entropy for friend codes: the codes gate who can talk to whom, so
/// they must not be guessable. /dev/urandom where available, with a
/// hasher-seed fallback elsewhere.
fn random_entropy() -> [u8; 6] {
    let mut bytes = [0u8; 6];
    if let Ok(mut urandom) = std::fs::File::open("/dev/urandom") {
        if urandom.read_exact(&mut bytes).is_ok() {
            return bytes;
        }
    }

    // Fallback: std's per-process random hasher seed, stretched over a
    // counter and the clock.
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let state = RandomState::new();
    for (i, byte) in bytes.iter_mut().enumerate() {
        let mut hasher = state.build_hasher();
        hasher.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
        hasher.write_u64(i as u64);
        hasher.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        );
        *byte = hasher.finish() as u8;
    }
    bytes
}
