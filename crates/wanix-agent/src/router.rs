//! [`RouterEngine`]: dispatch an agent session to a local or remote engine.
//!
//! A mesh node may run several agent engines: its own local one (a real codex
//! bridge, or the deterministic fake) and one [`RemoteEngine`](crate::RemoteEngine)
//! per imported peer `#agent`. `RouterEngine` composes them behind one
//! [`AgentEngine`]: it holds a default route and any number of named routes, and
//! each [`Self::start_session`] dispatches to the default while
//! [`Self::start_session_on`] targets a named route by name. Because every route
//! is just an `AgentEngine`, "run the agent here" and "run it on node A" are the
//! same call with a different route — Plan 9 cpu's "run there, namespace from
//! here", applied to agents.

use std::collections::BTreeMap;
use std::sync::Arc;

use wanix_fs::{FsError, FsResult};

use crate::engine::{AgentEngine, AgentSession};

/// An [`AgentEngine`] that dispatches sessions to named local or remote engines.
///
/// The `default` route backs the bare [`AgentEngine::start_session`] (so the
/// router drops in anywhere an engine is expected); named routes are reached
/// explicitly with [`Self::start_session_on`]. A route is any `AgentEngine`, so
/// a router can nest routers, mix a local engine with remote ones, or fan out to
/// several peers.
pub struct RouterEngine {
    default: Arc<dyn AgentEngine>,
    routes: BTreeMap<String, Arc<dyn AgentEngine>>,
    label: String,
}

impl RouterEngine {
    /// Builds a router whose default (unnamed) route is `default`.
    #[must_use]
    pub fn new(default: Arc<dyn AgentEngine>) -> Self {
        let label = format!("router(default={})", default.describe());
        Self {
            default,
            routes: BTreeMap::new(),
            label,
        }
    }

    /// Adds a named route `engine`, reachable via [`Self::start_session_on`].
    ///
    /// A common shape is `route("A", RemoteEngine::new(import_of_node_a, …))`, so
    /// `start_session_on("A")` runs the session on node A's `#agent`.
    #[must_use]
    pub fn route(mut self, name: impl Into<String>, engine: Arc<dyn AgentEngine>) -> Self {
        self.routes.insert(name.into(), engine);
        self
    }

    /// Lists the named routes, sorted, for status and discovery.
    #[must_use]
    pub fn route_names(&self) -> Vec<String> {
        self.routes.keys().cloned().collect()
    }

    /// Starts a session on the named route `name`.
    ///
    /// # Errors
    ///
    /// Returns [`FsError::NotFound`] when no route has that name, or the route's
    /// own error when its engine cannot start a session.
    pub fn start_session_on(&self, name: &str) -> FsResult<Arc<dyn AgentSession>> {
        let engine = self.routes.get(name).ok_or(FsError::NotFound)?;
        engine.start_session()
    }
}

impl AgentEngine for RouterEngine {
    fn start_session(&self) -> FsResult<Arc<dyn AgentSession>> {
        // The bare call dispatches to the default route, so the router substitutes
        // for any single engine; named routing is the explicit opt-in.
        self.default.start_session()
    }

    fn describe(&self) -> &str {
        &self.label
    }
}

#[cfg(test)]
mod router_tests;
