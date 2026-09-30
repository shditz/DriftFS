use std::collections::HashMap;

use driftfs_core::FileId;

use crate::chunk::ChunkKey;

struct LruNode {
    key: ChunkKey,
    size: u64,
    prev: Option<usize>,
    next: Option<usize>,
}

pub struct LruTracker {
    arena: Vec<Option<LruNode>>,
    free_list: Vec<usize>,
    map: HashMap<ChunkKey, usize>,
    head: Option<usize>,
    tail: Option<usize>,
    current_size_bytes: u64,
    reserved_bytes: u64,
    max_size_bytes: u64,
    low_watermark_bytes: u64,
}

impl LruTracker {
    pub fn new(max_size_bytes: u64) -> Self {
        let low_watermark_bytes = (max_size_bytes * 85) / 100;
        Self {
            arena: Vec::new(),
            free_list: Vec::new(),
            map: HashMap::new(),
            head: None,
            tail: None,
            current_size_bytes: 0,
            reserved_bytes: 0,
            max_size_bytes,
            low_watermark_bytes,
        }
    }

    pub fn current_size(&self) -> u64 {
        self.current_size_bytes
    }

    pub fn reserved_size(&self) -> u64 {
        self.reserved_bytes
    }

    pub fn total_size(&self) -> u64 {
        self.current_size_bytes.saturating_add(self.reserved_bytes)
    }

    pub fn max_size(&self) -> u64 {
        self.max_size_bytes
    }

    pub fn contains(&self, key: &ChunkKey) -> bool {
        self.map.contains_key(key)
    }

    pub fn get_size(&self, key: &ChunkKey) -> Option<u64> {
        self.map
            .get(key)
            .and_then(|&idx| self.arena.get(idx)?.as_ref().map(|n| n.size))
    }

    pub fn touch(&mut self, key: &ChunkKey) {
        if let Some(&idx) = self.map.get(key) {
            self.move_to_tail(idx);
        }
    }

    pub fn prepare_eviction(&mut self, incoming_bytes: u64) -> Vec<ChunkKey> {
        let mut evicted = Vec::new();
        let projected = self
            .current_size_bytes
            .saturating_add(self.reserved_bytes)
            .saturating_add(incoming_bytes);

        if projected <= self.max_size_bytes {
            self.reserved_bytes = self.reserved_bytes.saturating_add(incoming_bytes);
            return evicted;
        }

        while self
            .current_size_bytes
            .saturating_add(self.reserved_bytes)
            .saturating_add(incoming_bytes)
            > self.low_watermark_bytes
        {
            if let Some(key) = self.pop_lru() {
                evicted.push(key);
            } else {
                break;
            }
        }

        self.reserved_bytes = self.reserved_bytes.saturating_add(incoming_bytes);
        evicted
    }

    pub fn cancel_reservation(&mut self, bytes: u64) {
        self.reserved_bytes = self.reserved_bytes.saturating_sub(bytes);
    }

    pub fn insert(&mut self, key: ChunkKey, size: u64) {
        if let Some(&idx) = self.map.get(&key) {
            if let Some(Some(node)) = self.arena.get_mut(idx) {
                self.current_size_bytes = self
                    .current_size_bytes
                    .saturating_sub(node.size)
                    .saturating_add(size);
                node.size = size;
            }
            self.move_to_tail(idx);
            return;
        }

        let new_node = LruNode {
            key: key.clone(),
            size,
            prev: None,
            next: None,
        };

        let idx = if let Some(free_idx) = self.free_list.pop() {
            self.arena[free_idx] = Some(new_node);
            free_idx
        } else {
            let new_idx = self.arena.len();
            self.arena.push(Some(new_node));
            new_idx
        };

        self.link_tail(idx);
        self.map.insert(key, idx);
        self.reserved_bytes = self.reserved_bytes.saturating_sub(size);
        self.current_size_bytes = self.current_size_bytes.saturating_add(size);
    }

    pub fn remove(&mut self, key: &ChunkKey) -> Option<u64> {
        let idx = self.map.remove(key)?;
        let size = self.unlink(idx)?;
        self.free_list.push(idx);
        self.current_size_bytes = self.current_size_bytes.saturating_sub(size);
        Some(size)
    }

    pub fn keys_for_file(&self, file_id: &FileId) -> Vec<ChunkKey> {
        self.map
            .keys()
            .filter(|k| k.file_id == *file_id)
            .cloned()
            .collect()
    }

    pub fn drain_all(&mut self) -> Vec<ChunkKey> {
        let mut keys = Vec::with_capacity(self.map.len());
        while let Some(key) = self.pop_lru() {
            keys.push(key);
        }
        self.reserved_bytes = 0;
        keys
    }

    fn pop_lru(&mut self) -> Option<ChunkKey> {
        let head_idx = self.head?;
        let node = self.arena.get_mut(head_idx)?.take()?;
        let key = node.key;
        let size = node.size;

        self.head = node.next;
        if let Some(new_head) = self.head {
            if let Some(Some(head_node)) = self.arena.get_mut(new_head) {
                head_node.prev = None;
            }
        } else {
            self.tail = None;
        }

        self.map.remove(&key);
        self.free_list.push(head_idx);
        self.current_size_bytes = self.current_size_bytes.saturating_sub(size);

        Some(key)
    }

    fn move_to_tail(&mut self, idx: usize) {
        if self.tail == Some(idx) {
            return;
        }

        let (prev, next) = match self.arena.get(idx) {
            Some(Some(n)) => (n.prev, n.next),
            _ => return,
        };

        if let Some(p) = prev {
            if let Some(Some(prev_node)) = self.arena.get_mut(p) {
                prev_node.next = next;
            }
        } else {
            self.head = next;
        }

        if let Some(n) = next {
            if let Some(Some(next_node)) = self.arena.get_mut(n) {
                next_node.prev = prev;
            }
        }

        self.link_tail(idx);
    }

    fn link_tail(&mut self, idx: usize) {
        let old_tail = self.tail;
        if let Some(Some(node)) = self.arena.get_mut(idx) {
            node.prev = old_tail;
            node.next = None;
        }

        if let Some(t) = old_tail {
            if let Some(Some(tail_node)) = self.arena.get_mut(t) {
                tail_node.next = Some(idx);
            }
        } else {
            self.head = Some(idx);
        }

        self.tail = Some(idx);
    }

    fn unlink(&mut self, idx: usize) -> Option<u64> {
        let node = self.arena.get_mut(idx)?.take()?;
        let prev = node.prev;
        let next = node.next;

        if let Some(p) = prev {
            if let Some(Some(prev_node)) = self.arena.get_mut(p) {
                prev_node.next = next;
            }
        } else {
            self.head = next;
        }

        if let Some(n) = next {
            if let Some(Some(next_node)) = self.arena.get_mut(n) {
                next_node.prev = prev;
            }
        } else {
            self.tail = prev;
        }

        Some(node.size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_key(id: &str, idx: u64) -> ChunkKey {
        ChunkKey::new(FileId(id.into()), "1", idx)
    }

    #[test]
    fn insert_and_eviction_watermark() {
        let mut lru = LruTracker::new(100);
        let k1 = make_key("f1", 0);
        let k2 = make_key("f1", 1);
        let k3 = make_key("f1", 2);

        lru.insert(k1.clone(), 40);
        lru.insert(k2.clone(), 40);
        assert_eq!(lru.current_size(), 80);

        let evicted = lru.prepare_eviction(30);
        assert_eq!(evicted.len(), 1);
        assert_eq!(evicted[0], k1);

        lru.insert(k3.clone(), 30);
        assert_eq!(lru.current_size(), 70);
        assert!(!lru.contains(&k1));
        assert!(lru.contains(&k2));
        assert!(lru.contains(&k3));
    }

    #[test]
    fn touch_promotes_mru() {
        let mut lru = LruTracker::new(100);
        let k1 = make_key("f1", 0);
        let k2 = make_key("f1", 1);
        let k3 = make_key("f1", 2);

        lru.insert(k1.clone(), 40);
        lru.insert(k2.clone(), 40);
        lru.touch(&k1);

        let evicted = lru.prepare_eviction(30);
        assert_eq!(evicted.len(), 1);
        assert_eq!(evicted[0], k2);
        lru.insert(k3, 30);

        assert!(lru.contains(&k1));
        assert!(!lru.contains(&k2));
    }

    #[test]
    fn remove_and_drain() {
        let mut lru = LruTracker::new(100);
        let k1 = make_key("f1", 0);
        let k2 = make_key("f1", 1);

        lru.insert(k1.clone(), 30);
        lru.insert(k2.clone(), 30);

        assert_eq!(lru.remove(&k1), Some(30));
        assert_eq!(lru.current_size(), 30);

        let drained = lru.drain_all();
        assert_eq!(drained, vec![k2]);
        assert_eq!(lru.current_size(), 0);
    }

    #[test]
    fn concurrent_reservation_prevents_overshoot() {
        let mut lru = LruTracker::new(100);
        let k1 = make_key("f1", 0);
        lru.insert(k1.clone(), 60);

        let evicted_a = lru.prepare_eviction(30);
        assert!(evicted_a.is_empty());
        assert_eq!(lru.reserved_size(), 30);
        assert_eq!(lru.total_size(), 90);

        let evicted_b = lru.prepare_eviction(20);
        assert_eq!(evicted_b, vec![k1]);
        assert_eq!(lru.current_size(), 0);
        assert_eq!(lru.reserved_size(), 50);

        let ka = make_key("fa", 0);
        lru.insert(ka, 30);
        assert_eq!(lru.current_size(), 30);
        assert_eq!(lru.reserved_size(), 20);

        lru.cancel_reservation(20);
        assert_eq!(lru.current_size(), 30);
        assert_eq!(lru.reserved_size(), 0);
    }
}
