//! Remembers when each payment first appeared. ldk-node only keeps the time of the last update,
//! which moves on every status change, so the first-seen time is stored here, beside the node's data.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

struct State {
    seen: HashMap<String, u64>,
    /// True until the first `record_new` call when there was no usable file. Payments that already
    /// exist then get their last update time as an estimate: the real first-seen time is unknown.
    estimate_existing: bool,
}

pub struct FirstSeen {
    path: PathBuf,
    state: Mutex<State>,
}

impl FirstSeen {
    /// A missing file starts empty. A corrupt one is moved aside to `.corrupt` and also starts empty.
    pub fn load(path: PathBuf) -> Self {
        let (seen, estimate_existing) = match fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<HashMap<String, u64>>(&bytes) {
                Ok(seen) => (seen, false),
                Err(err) => {
                    eprintln!("first-seen file is corrupt ({err}); starting over");
                    let _ = fs::rename(&path, path.with_extension("json.corrupt"));
                    (HashMap::new(), true)
                }
            },
            Err(_) => (HashMap::new(), true),
        };
        return Self {
            path,
            state: Mutex::new(State {
                seen,
                estimate_existing,
            }),
        };
    }

    /// Records every `(id, last_update)` not seen before. Known ids are never changed.
    pub fn record_new(&self, payments: impl IntoIterator<Item = (String, u64)>, now: u64) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let estimate = state.estimate_existing;
        state.estimate_existing = false;
        let mut changed = false;
        for (id, last_update) in payments {
            if !state.seen.contains_key(&id) {
                let at = if estimate { last_update.min(now) } else { now };
                state.seen.insert(id, at);
                changed = true;
            }
        }
        if changed {
            self.save(&state.seen);
        }
    }

    /// Records a known first-seen time for an id that has none yet, such as a payment that was
    /// already shown as pending before ldk-node listed it.
    pub fn record_at(&self, id: &str, at: u64) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !state.seen.contains_key(id) {
            state.seen.insert(id.to_string(), at);
            self.save(&state.seen);
        }
    }

    /// The recorded time, or `fallback` for an id that has not been recorded.
    pub fn get(&self, id: &str, fallback: u64) -> u64 {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        return state.seen.get(id).copied().unwrap_or(fallback);
    }

    /// Writes to a temp file and renames it, so a crash never leaves half a file. A failed write is
    /// logged and never fails a request.
    fn save(&self, seen: &HashMap<String, u64>) {
        let tmp = self.path.with_extension("json.tmp");
        let result = serde_json::to_vec(seen)
            .map_err(|e| e.to_string())
            .and_then(|bytes| fs::write(&tmp, bytes).map_err(|e| e.to_string()))
            .and_then(|()| fs::rename(&tmp, &self.path).map_err(|e| e.to_string()));
        if let Err(err) = result {
            eprintln!("could not save first-seen times: {err}");
        }
    }
}

// === Tests

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("first-seen-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        return dir.join("first_seen.json");
    }

    fn entry(id: &str, updated: u64) -> (String, u64) {
        return (id.to_string(), updated);
    }

    #[test]
    fn first_run_estimates_existing_payments_from_their_update_time() {
        let store = FirstSeen::load(temp_path("estimate"));
        store.record_new([entry("a", 100)], 5_000);
        assert_eq!(store.get("a", 0), 100);
        // After that first pass, new payments get the current time.
        store.record_new([entry("b", 200)], 6_000);
        assert_eq!(store.get("b", 0), 6_000);
    }

    #[test]
    fn a_recorded_time_never_changes() {
        let store = FirstSeen::load(temp_path("once"));
        store.record_new([], 1);
        store.record_new([entry("a", 10)], 1_000);
        store.record_new([entry("a", 99)], 2_000);
        assert_eq!(store.get("a", 0), 1_000);
    }

    #[test]
    fn times_survive_a_reload_and_skip_the_estimate() {
        let path = temp_path("reload");
        let store = FirstSeen::load(path.clone());
        store.record_new([], 1);
        store.record_new([entry("a", 10)], 1_000);

        let reloaded = FirstSeen::load(path);
        assert_eq!(reloaded.get("a", 0), 1_000);
        reloaded.record_new([entry("b", 20)], 3_000);
        assert_eq!(reloaded.get("b", 0), 3_000);
    }

    #[test]
    fn a_corrupt_file_is_moved_aside_and_the_store_starts_over() {
        let path = temp_path("corrupt");
        fs::write(&path, b"not json").unwrap();
        let store = FirstSeen::load(path.clone());
        assert_eq!(store.get("a", 7), 7);
        assert!(path.with_extension("json.corrupt").exists());
        store.record_new([entry("a", 50)], 9_000);
        assert_eq!(store.get("a", 0), 50);
    }

    #[test]
    fn unknown_ids_use_the_fallback() {
        let store = FirstSeen::load(temp_path("fallback"));
        assert_eq!(store.get("missing", 42), 42);
    }
}
