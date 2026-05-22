use std::sync::Arc;

pub struct SlotData {
    pub slot: u64,
    pub parent_slot: u64,
    pub entries: Vec<u8>,
    pub num_transactions: usize,
    pub merkle_root: Option<[u8; 32]>,
}

pub struct SlotRingBuffer {
    slots: Vec<Option<Arc<SlotData>>>,
    capacity: usize,
    head: u64,
    tail: u64,
}

impl SlotRingBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            slots: vec![None; capacity],
            capacity,
            head: 0,
            tail: 0,
        }
    }

    pub fn put(&mut self, slot_data: SlotData) {
        let slot = slot_data.slot;
        let idx = (slot % self.capacity as u64) as usize;
        self.slots[idx] = Some(Arc::new(slot_data));

        if slot >= self.head + self.capacity as u64 {
            self.head = slot - self.capacity as u64 + 1;
        }
        if slot >= self.tail {
            self.tail = slot + 1;
        }
    }

    pub fn get(&self, slot: u64) -> Option<Arc<SlotData>> {
        if slot < self.head || slot >= self.tail {
            return None;
        }
        let idx = (slot % self.capacity as u64) as usize;
        self.slots[idx].as_ref().and_then(|data| {
            if data.slot == slot {
                Some(data.clone())
            } else {
                None
            }
        })
    }
}
