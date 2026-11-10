//! Keeps count of who is in each door, so a door with `max_callers` set
//! lets only that many callers in at once.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// A caller inside a door.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Occupant {
    pub node: u16,
    pub handle: String,
}

/// Who is in which door, shared by every session.
#[derive(Clone, Default)]
pub struct DoorSeats {
    doors: Arc<Mutex<HashMap<String, Vec<Occupant>>>>,
}

impl DoorSeats {
    pub fn new() -> Self {
        Self::default()
    }

    /// Take a seat in door `id`. `limit` 0 means no limit. When the door is
    /// full, returns the callers already in it.
    pub fn enter(
        &self,
        id: &str,
        limit: u32,
        node: u16,
        handle: &str,
    ) -> Result<DoorSeat, Vec<Occupant>> {
        let key = id.to_ascii_lowercase();
        let mut doors = self.doors.lock().unwrap_or_else(|e| e.into_inner());
        let inside = doors.entry(key.clone()).or_default();
        if limit > 0 && inside.len() >= limit as usize {
            return Err(inside.clone());
        }
        inside.push(Occupant {
            node,
            handle: handle.to_string(),
        });
        Ok(DoorSeat {
            seats: self.clone(),
            key,
            node,
        })
    }

    /// Callers currently in door `id`.
    pub fn inside(&self, id: &str) -> Vec<Occupant> {
        let doors = self.doors.lock().unwrap_or_else(|e| e.into_inner());
        doors
            .get(&id.to_ascii_lowercase())
            .cloned()
            .unwrap_or_default()
    }
}

/// A taken seat. It is given back when this is dropped, so a caller who
/// hangs up mid-game frees the door too.
pub struct DoorSeat {
    seats: DoorSeats,
    key: String,
    node: u16,
}

impl Drop for DoorSeat {
    fn drop(&mut self) {
        let mut doors = self.seats.doors.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(inside) = doors.get_mut(&self.key) {
            if let Some(i) = inside.iter().position(|o| o.node == self.node) {
                inside.remove(i);
            }
            if inside.is_empty() {
                doors.remove(&self.key);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_caller_door_turns_away_a_second_caller() {
        let seats = DoorSeats::new();
        let first = seats.enter("tw2002", 1, 1, "Clayton").unwrap();
        let busy = seats.enter("TW2002", 1, 2, "Guest").err().unwrap();
        assert_eq!(
            busy,
            vec![Occupant {
                node: 1,
                handle: "Clayton".into()
            }]
        );
        // Other doors aren't affected.
        let _lord = seats.enter("lord", 1, 2, "Guest").unwrap();
        drop(first);
        assert!(seats.inside("tw2002").is_empty());
        let _second = seats.enter("tw2002", 1, 2, "Guest").unwrap();
    }

    #[test]
    fn no_limit_lets_everyone_in() {
        let seats = DoorSeats::new();
        let _a = seats.enter("lord", 0, 1, "A").unwrap();
        let _b = seats.enter("lord", 0, 2, "B").unwrap();
        assert_eq!(seats.inside("lord").len(), 2);
    }
}
