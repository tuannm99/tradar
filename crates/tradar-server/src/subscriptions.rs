//! Bookkeeping for push subscriptions (Kafka tail, Socket tail): ids,
//! ownership by client, and "everything this client left running".
//!
//! Handles are returned to the caller instead of stopped in here, so the
//! caller can drop its state lock first (stopping aborts tasks; nothing
//! about that needs the lock held).

use std::collections::HashMap;

struct Entry<H, M> {
    client: u64,
    #[cfg_attr(not(any(feature = "socket", feature = "grpc")), allow(dead_code))]
    meta: M,
    handle: H,
}

/// `H` is the running subscription (`TailHandle`), `M` whatever the owner
/// wants to look up by (`()` for Kafka, the saved connection name for
/// Socket).
pub(crate) struct Subscriptions<H, M = ()> {
    prefix: &'static str,
    next: u64,
    entries: HashMap<String, Entry<H, M>>,
    /// client id -> its subscription ids, so a closing socket finds exactly
    /// its own without scanning `entries`.
    by_client: HashMap<u64, Vec<String>>,
}

impl<H, M> Subscriptions<H, M> {
    pub fn new(prefix: &'static str) -> Self {
        Self {
            prefix,
            next: 0,
            entries: HashMap::new(),
            by_client: HashMap::new(),
        }
    }

    /// A fresh `<prefix>-<n>` id. Consumed even if the start then fails.
    pub fn next_id(&mut self) -> String {
        self.next += 1;
        format!("{}-{}", self.prefix, self.next)
    }

    pub fn insert(&mut self, id: String, client: u64, meta: M, handle: H) {
        self.by_client.entry(client).or_default().push(id.clone());
        self.entries.insert(
            id,
            Entry {
                client,
                meta,
                handle,
            },
        );
    }

    #[cfg_attr(not(any(feature = "socket", feature = "grpc")), allow(dead_code))]
    pub fn get(&self, id: &str) -> Option<&H> {
        self.entries.get(id).map(|e| &e.handle)
    }

    #[cfg_attr(not(any(feature = "socket", feature = "grpc")), allow(dead_code))]
    pub fn any_meta(&self, mut f: impl FnMut(&M) -> bool) -> bool {
        self.entries.values().any(|e| f(&e.meta))
    }

    /// Forgets `id`; the caller stops the returned handle.
    pub fn remove(&mut self, id: &str) -> Option<H> {
        let entry = self.entries.remove(id)?;
        if let Some(ids) = self.by_client.get_mut(&entry.client) {
            ids.retain(|s| s != id);
        }
        Some(entry.handle)
    }

    /// Forgets every subscription `client` owns; the caller stops them.
    pub fn remove_client(&mut self, client: u64) -> Vec<H> {
        let Some(ids) = self.by_client.remove(&client) else {
            return Vec::new();
        };
        ids.iter()
            .filter_map(|id| self.entries.remove(id))
            .map(|e| e.handle)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_prefixed_and_increase() {
        let mut subs: Subscriptions<u8> = Subscriptions::new("kafka");
        assert_eq!(subs.next_id(), "kafka-1");
        assert_eq!(subs.next_id(), "kafka-2");
    }

    #[test]
    fn remove_forgets_one_and_leaves_the_clients_other_subscriptions() {
        let mut subs: Subscriptions<&str> = Subscriptions::new("s");
        subs.insert("s-1".into(), 7, (), "a");
        subs.insert("s-2".into(), 7, (), "b");

        assert_eq!(subs.remove("s-1"), Some("a"));
        assert_eq!(subs.remove("s-1"), None);
        assert_eq!(subs.remove_client(7), vec!["b"]);
    }

    #[test]
    fn remove_client_takes_only_that_clients_subscriptions() {
        let mut subs: Subscriptions<&str> = Subscriptions::new("s");
        subs.insert("s-1".into(), 1, (), "mine");
        subs.insert("s-2".into(), 2, (), "theirs");

        assert_eq!(subs.remove_client(1), vec!["mine"]);
        assert_eq!(subs.get("s-2"), Some(&"theirs"));
        assert!(subs.remove_client(1).is_empty());
    }

    #[test]
    fn any_meta_looks_through_every_entry() {
        let mut subs: Subscriptions<u8, String> = Subscriptions::new("s");
        subs.insert("s-1".into(), 1, "web".to_string(), 0);

        assert!(subs.any_meta(|name| name == "web"));
        assert!(!subs.any_meta(|name| name == "other"));
        subs.remove("s-1");
        assert!(!subs.any_meta(|name| name == "web"));
    }
}
