//! Tracks which callers are on which node.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct NodeInfo {
    pub node: u16,
    pub peer: Option<SocketAddr>,
    pub handle: Option<String>,
    pub location: Option<String>,
    pub activity: String,
    pub connected_at: Instant,
}

/// The node table, shared by every session.
#[derive(Clone)]
pub struct Nodes {
    slots: Arc<Mutex<Vec<Option<NodeInfo>>>>,
}

impl Nodes {
    pub fn new(max_nodes: u16) -> Self {
        Self {
            slots: Arc::new(Mutex::new(vec![None; usize::from(max_nodes)])),
        }
    }

    /// Take the lowest free node, or `None` if the board is full.
    pub fn claim(&self, peer: Option<SocketAddr>) -> Option<NodeGuard> {
        let mut slots = self.slots.lock().ok()?;
        let index = slots.iter().position(Option::is_none)?;
        let node = u16::try_from(index + 1).ok()?;
        slots[index] = Some(NodeInfo {
            node,
            peer,
            handle: None,
            location: None,
            activity: "Logging in".into(),
            connected_at: Instant::now(),
        });
        Some(NodeGuard {
            nodes: self.clone(),
            node,
        })
    }

    /// Everyone currently connected.
    pub fn online(&self) -> Vec<NodeInfo> {
        self.slots
            .lock()
            .map(|s| s.iter().flatten().cloned().collect())
            .unwrap_or_default()
    }

    fn update(&self, node: u16, f: impl FnOnce(&mut NodeInfo)) {
        if let Ok(mut slots) = self.slots.lock()
            && let Some(Some(info)) = slots.get_mut(usize::from(node) - 1)
        {
            f(info);
        }
    }
}

/// A claimed node. The node is freed when this is dropped.
pub struct NodeGuard {
    nodes: Nodes,
    node: u16,
}

impl NodeGuard {
    pub fn number(&self) -> u16 {
        self.node
    }

    pub fn set_user(&self, handle: &str, location: Option<&str>) {
        self.nodes.update(self.node, |info| {
            info.handle = Some(handle.to_string());
            info.location = location.map(str::to_string);
        });
    }

    pub fn set_activity(&self, activity: &str) {
        self.nodes
            .update(self.node, |info| info.activity = activity.to_string());
    }
}

impl Drop for NodeGuard {
    fn drop(&mut self) {
        if let Ok(mut slots) = self.nodes.slots.lock()
            && let Some(slot) = slots.get_mut(usize::from(self.node) - 1)
        {
            *slot = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claims_lowest_free_node_and_frees_on_drop() {
        let nodes = Nodes::new(2);
        let a = nodes.claim(None).unwrap();
        let b = nodes.claim(None).unwrap();
        assert_eq!((a.number(), b.number()), (1, 2));
        assert!(nodes.claim(None).is_none());
        drop(a);
        let c = nodes.claim(None).unwrap();
        assert_eq!(c.number(), 1);
        c.set_user("Clayton", Some("Texas"));
        assert_eq!(nodes.online().len(), 2);
        assert!(
            nodes
                .online()
                .iter()
                .any(|n| n.handle.as_deref() == Some("Clayton"))
        );
    }
}
