use malt_protocol::common::InputAuthority;
use std::collections::VecDeque;

#[derive(Debug)]
struct AttachedClient {
    client_id: u64,
    authority: InputAuthority,
}

/// Tracks input authority for a session's attached clients.
///
/// Rules:
/// - Most recent Exclusive/Shared attach gets authority
/// - Observe attach never claims authority
/// - On holder detach, authority falls to next eligible client (FIFO)
/// - `claim()` transfers authority explicitly
#[derive(Debug)]
pub struct AuthorityTracker {
    clients: VecDeque<AttachedClient>,
    holder: Option<u64>,
}

impl AuthorityTracker {
    pub fn new() -> Self {
        Self {
            clients: VecDeque::new(),
            holder: None,
        }
    }

    /// Attach a client. If authority is Exclusive or Shared, claim input.
    pub fn attach(&mut self, client_id: u64, authority: InputAuthority) {
        self.clients.push_back(AttachedClient {
            client_id,
            authority,
        });
        match authority {
            InputAuthority::Exclusive | InputAuthority::Shared => {
                self.holder = Some(client_id);
            }
            InputAuthority::Observe => {
                // Observe does not claim
            }
            _ => {}
        }
    }

    /// Detach a client. If they held authority, fall back to FIFO.
    pub fn detach(&mut self, client_id: u64) {
        self.clients.retain(|c| c.client_id != client_id);
        if self.holder == Some(client_id) {
            self.holder = self.find_next_eligible();
        }
    }

    /// Explicitly claim authority for a client.
    pub fn claim(&mut self, client_id: u64, authority: InputAuthority) {
        if let Some(c) = self.clients.iter_mut().find(|c| c.client_id == client_id) {
            c.authority = authority;
        }
        match authority {
            InputAuthority::Exclusive | InputAuthority::Shared => {
                self.holder = Some(client_id);
            }
            _ => {}
        }
    }

    /// Returns the client that currently holds input authority.
    pub fn holder(&self) -> Option<u64> {
        self.holder
    }

    /// Returns all attached client IDs.
    pub fn attached_clients(&self) -> Vec<u64> {
        self.clients.iter().map(|c| c.client_id).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.clients.is_empty()
    }

    /// Find the next eligible client for authority (FIFO order).
    /// Prefers Exclusive/Shared over Observe.
    fn find_next_eligible(&self) -> Option<u64> {
        for c in &self.clients {
            match c.authority {
                InputAuthority::Exclusive | InputAuthority::Shared => {
                    return Some(c.client_id);
                }
                _ => {}
            }
        }
        None
    }
}

impl Default for AuthorityTracker {
    fn default() -> Self {
        Self::new()
    }
}
