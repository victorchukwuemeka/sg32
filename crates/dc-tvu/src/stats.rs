use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Default)]
pub struct FecBatchStats {
    pub slot: u64,
    pub fec_set_index: u32,
    pub data_shreds: usize,
    pub code_shreds: usize,
    pub num_data: usize,
    pub num_code: usize,
}

#[derive(Debug, Clone, Default)]
pub struct PipelineStats {
    pub latest_slot: u64,
    pub current_batch: FecBatchStats,
    pub total_blocks_recovered: u64,
    pub blocks_in_ring_buffer: usize,
    pub files_on_disk: usize,
    pub latest_block_txs: usize,
    pub latest_block_root: [u8; 32],
}

pub type SharedStats = Arc<RwLock<PipelineStats>>;

pub fn new_shared_stats() -> SharedStats {
    Arc::new(RwLock::new(PipelineStats::default()))
}
