//! Group management and policy enforcement.
//!
//! Stub module — full implementation in a follow-up task.

use std::collections::HashMap;

use malt_protocol::common::{GroupId, OnEmpty, OnOom, SessionId};
use malt_protocol::persist::daemon::GroupPolicy;

use crate::session::SessionError;

/// Runtime state for a single session group.
#[derive(Debug, Clone)]
pub struct GroupRuntime {
    pub id: GroupId,
    pub name: String,
    pub policy: GroupPolicy,
    pub sessions: Vec<SessionId>,
}

/// Manages session groups and enforces group policies.
#[derive(Debug)]
pub struct GroupManager {
    groups: HashMap<u32, GroupRuntime>,
}

impl GroupManager {
    /// Create an empty group manager.
    pub fn new() -> Self {
        Self {
            groups: HashMap::new(),
        }
    }

    /// Register a new group.
    pub fn create_group(&mut self, id: GroupId, name: String, policy: GroupPolicy) {
        self.groups.insert(
            id.0,
            GroupRuntime {
                id,
                name,
                policy,
                sessions: Vec::new(),
            },
        );
    }

    /// Remove a group by id.
    pub fn remove_group(&mut self, id: GroupId) {
        self.groups.remove(&id.0);
    }

    /// Add a session to a group, enforcing `max_sessions`.
    pub fn add_session(
        &mut self,
        group_id: GroupId,
        session_id: SessionId,
    ) -> Result<(), SessionError> {
        let group = self
            .groups
            .get_mut(&group_id.0)
            .ok_or_else(|| SessionError::PolicyViolation("group not found".into()))?;

        if group.sessions.len() >= group.policy.max_sessions as usize {
            return Err(SessionError::PolicyViolation(
                "max_sessions limit reached".into(),
            ));
        }

        group.sessions.push(session_id);
        Ok(())
    }

    /// Remove a session from a group.
    pub fn remove_session(&mut self, group_id: GroupId, session_id: SessionId) {
        if let Some(group) = self.groups.get_mut(&group_id.0) {
            group.sessions.retain(|s| *s != session_id);
        }
    }

    /// Return the `OnEmpty` policy for a group.
    pub fn on_session_empty(&self, group_id: GroupId) -> Option<OnEmpty> {
        self.groups.get(&group_id.0).map(|g| g.policy.on_empty.clone())
    }

    /// Return the `OnOom` policy for a group.
    pub fn on_oom(&self, group_id: GroupId) -> Option<OnOom> {
        self.groups.get(&group_id.0).map(|g| g.policy.on_oom.clone())
    }

    /// Check whether a new session can be created in the given group.
    pub fn can_create_session(&self, group_id: GroupId) -> bool {
        match self.groups.get(&group_id.0) {
            Some(group) => group.sessions.len() < group.policy.max_sessions as usize,
            None => false,
        }
    }

    /// Look up a group by id.
    pub fn get_group(&self, id: GroupId) -> Option<&GroupRuntime> {
        self.groups.get(&id.0)
    }
}

impl Default for GroupManager {
    fn default() -> Self {
        Self::new()
    }
}
