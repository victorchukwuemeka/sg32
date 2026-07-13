use crate::flat_file_store::FlatFileStore;
use crate::merkle_prover::MerkleTree;
use crate::ring_buffer::SlotRingBuffer;
use crate::stats::SharedStats;
use axum::{extract::State, Json, Router};
use serde::{Deserialize, Serialize};
use solana_sdk::transaction::VersionedTransaction;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct AppState {
    pub ring_buffer: Arc<RwLock<SlotRingBuffer>>,
    pub file_store: Arc<RwLock<FlatFileStore>>,
    pub stats: SharedStats,
}

#[derive(Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub method: String,
    pub params: Option<serde_json::Value>,
    pub id: u64,
}

#[derive(Serialize)]
pub struct JsonRpcResponse<T: Serialize> {
    pub jsonrpc: String,
    pub result: Option<T>,
    pub error: Option<String>,
    pub id: u64,
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", axum::routing::get(index_handler))
        .route("/jsonrpc", axum::routing::post(jsonrpc_handler))
        .route("/stats", axum::routing::get(stats_handler))
        .with_state(state)
}

async fn index_handler() -> (axum::http::StatusCode, [(&'static str, &'static str); 1], &'static str) {
    (axum::http::StatusCode::OK, [("content-type", "text/html")], INDEX_HTML)
}

async fn stats_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let stats = state.stats.read().await;
    Json(serde_json::json!({
        "latest_slot": stats.latest_slot,
        "current_batch": {
            "slot": stats.current_batch.slot,
            "fec_set_index": stats.current_batch.fec_set_index,
            "data_shreds": stats.current_batch.data_shreds,
            "code_shreds": stats.current_batch.code_shreds,
            "num_data": stats.current_batch.num_data,
            "num_code": stats.current_batch.num_code,
        },
        "total_blocks_recovered": stats.total_blocks_recovered,
        "blocks_in_ring_buffer": stats.blocks_in_ring_buffer,
        "files_on_disk": stats.files_on_disk,
        "latest_block_txs": stats.latest_block_txs,
        "latest_block_root": hex::encode(stats.latest_block_root),
    }))
}

fn param_str(params: &Option<serde_json::Value>, i: usize) -> Option<String> {
    params
        .as_ref()
        .and_then(|p| p.as_array()?.get(i)?.as_str().map(String::from))
}

fn param_u64(params: &Option<serde_json::Value>, i: usize) -> Option<u64> {
    params.as_ref().and_then(|p| p.as_array()?.get(i)?.as_u64())
}

fn param_bool(params: &Option<serde_json::Value>, i: usize) -> Option<bool> {
    params.as_ref().and_then(|p| p.as_array()?.get(i)?.as_bool())
}

async fn load_slot_data(
    state: &Arc<AppState>,
    slot: u64,
) -> Option<(Vec<VersionedTransaction>, Arc<MerkleTree>)> {
    let buf = state.ring_buffer.read().await;
    if let Some(data) = buf.get(slot) {
        if data.num_transactions == 0 || data.merkle_tree.is_none() {
            drop(buf);
            return None;
        }
        let entries: Vec<crate::deshredder::Entry> =
            bincode::deserialize(&data.entries).ok()?;
        let tree = data.merkle_tree.clone().unwrap();
        drop(buf);

        let txs: Vec<VersionedTransaction> = entries
            .iter()
            .flat_map(|e| &e.transactions)
            .cloned()
            .collect();
        return Some((txs, tree));
    }
    drop(buf);

    let store = state.file_store.read().await;
    let raw = store.load_slot(slot)?;
    drop(store);

    let result = crate::deshredder::deshred_into_txs(&[raw])?;
    let tree = Arc::new(MerkleTree::new(&result.transactions));
    let txs: Vec<VersionedTransaction> = result
        .entries
        .iter()
        .flat_map(|e| &e.transactions)
        .cloned()
        .collect();
    Some((txs, tree))
}

fn encode_transaction_base64(tx: &VersionedTransaction) -> String {
    use base64::Engine;
    let bytes = bincode::serialize(tx).unwrap_or_default();
    base64::engine::general_purpose::STANDARD.encode(&bytes)
}

fn transaction_json(tx: &VersionedTransaction) -> serde_json::Value {
    let sigs: Vec<String> = tx
        .signatures
        .iter()
        .map(|s| bs58::encode(s.as_ref()).into_string())
        .collect();

    serde_json::json!({
        "signatures": sigs,
        "message": {
            "header": {
                "numRequiredSignatures": 1,
                "numReadonlySignedAccounts": 0,
                "numReadonlyUnsignedAccounts": 0
            },
            "accountKeys": [],
            "recentBlockhash": "",
            "instructions": []
        }
    })
}

// ── Standard Solana JSON-RPC methods ────────────────────────────────────────

async fn handle_get_slot(
    state: Arc<AppState>,
    _params: Option<serde_json::Value>,
    id: u64,
) -> Json<JsonRpcResponse<serde_json::Value>> {
    let stats = state.stats.read().await;
    let live = stats.latest_slot;
    drop(stats);

    let slot = if live > 0 {
        live
    } else {
        state.file_store.read().await.latest_slot().unwrap_or(0)
    };

    if slot == 0 {
        return Json(JsonRpcResponse {
            jsonrpc: "2.0".into(),
            result: None,
            error: Some("no slots available".into()),
            id,
        });
    }
    Json(JsonRpcResponse {
        jsonrpc: "2.0".into(),
        result: Some(serde_json::json!(slot)),
        error: None,
        id,
    })
}

async fn handle_get_block(
    state: Arc<AppState>,
    params: Option<serde_json::Value>,
    id: u64,
) -> Json<JsonRpcResponse<serde_json::Value>> {
    let slot = param_u64(&params, 0).unwrap_or(0);
    let encoding = param_str(&params, 1).unwrap_or_else(|| "base64".into());

    let (txs, _tree) = match load_slot_data(&state, slot).await {
        Some(v) => v,
        None => {
            return Json(JsonRpcResponse {
                jsonrpc: "2.0".into(),
                result: None,
                error: Some(format!("slot {} not found", slot)),
                id,
            })
        }
    };

    let tx_objects: Vec<serde_json::Value> = txs
        .iter()
        .map(|tx| {
            let tx_field = match encoding.as_str() {
                "json" => transaction_json(tx),
                _ => serde_json::json!([encode_transaction_base64(tx)]),
            };
            serde_json::json!({
                "transaction": tx_field,
                "meta": null,
                "version": "legacy",
            })
        })
        .collect();

    let buf = state.ring_buffer.read().await;
    let parent_slot = buf
        .get(slot)
        .map(|d| d.parent_slot)
        .unwrap_or(slot.saturating_sub(1));
    drop(buf);

    Json(JsonRpcResponse {
        jsonrpc: "2.0".into(),
        result: Some(serde_json::json!({
            "blockhash": null,
            "previousBlockhash": null,
            "parentSlot": parent_slot,
            "transactions": tx_objects,
            "blockTime": null,
            "blockHeight": null,
            "numTransactions": txs.len(),
        })),
        error: None,
        id,
    })
}

async fn handle_get_latest_slot(
    state: Arc<AppState>,
    id: u64,
) -> Json<JsonRpcResponse<serde_json::Value>> {
    let live = state.stats.read().await.latest_slot;

    let slot = if live > 0 {
        live
    } else {
        state.file_store.read().await.latest_slot().unwrap_or(0)
    };

    if slot == 0 {
        return Json(JsonRpcResponse {
            jsonrpc: "2.0".into(),
            result: None,
            error: Some("no slots available".into()),
            id,
        });
    }

    Json(JsonRpcResponse {
        jsonrpc: "2.0".into(),
        result: Some(serde_json::json!({
            "slot": slot,
        })),
        error: None,
        id,
    })
}

// ── Trustless custom methods ───────────────────────────────────────────────

async fn handle_get_proof(
    state: Arc<AppState>,
    params: Option<serde_json::Value>,
    id: u64,
) -> Json<JsonRpcResponse<serde_json::Value>> {
    let slot = param_u64(&params, 0).unwrap_or(0);
    let tx_index = param_u64(&params, 1).unwrap_or(0) as usize;

    let (_txs, tree) = match load_slot_data(&state, slot).await {
        Some(v) => v,
        None => {
            return Json(JsonRpcResponse {
                jsonrpc: "2.0".into(),
                result: None,
                error: Some(format!("slot {} not found", slot)),
                id,
            })
        }
    };

    let leaf = tree.leaves.get(tx_index).copied().unwrap_or([0u8; 32]);
    let proof = tree.prove(tx_index).unwrap_or_default();
    let verified = MerkleTree::verify(&tree.root, &leaf, &proof, tx_index);

    Json(JsonRpcResponse {
        jsonrpc: "2.0".into(),
        result: Some(serde_json::json!({
            "slot": slot,
            "tx_index": tx_index,
            "leaf": hex::encode(leaf),
            "proof": proof.iter().map(hex::encode).collect::<Vec<_>>(),
            "root": hex::encode(tree.root),
            "verified": verified,
        })),
        error: None,
        id,
    })
}

async fn handle_get_transaction_by_index(
    state: Arc<AppState>,
    params: Option<serde_json::Value>,
    id: u64,
) -> Json<JsonRpcResponse<serde_json::Value>> {
    let slot = param_u64(&params, 0).unwrap_or(0);
    let tx_index = param_u64(&params, 1).unwrap_or(0) as usize;

    let (txs, tree) = match load_slot_data(&state, slot).await {
        Some(v) => v,
        None => {
            return Json(JsonRpcResponse {
                jsonrpc: "2.0".into(),
                result: None,
                error: Some(format!("slot {} not found", slot)),
                id,
            })
        }
    };

    let tx = match txs.get(tx_index) {
        Some(t) => t,
        None => {
            return Json(JsonRpcResponse {
                jsonrpc: "2.0".into(),
                result: None,
                error: Some(format!("tx_index {} out of range (max {})", tx_index, txs.len().saturating_sub(1))),
                id,
            })
        }
    };

    let tx_bytes = bincode::serialize(tx).unwrap_or_default();
    let leaf = tree.leaves.get(tx_index).copied().unwrap_or([0u8; 32]);
    let proof = tree.prove(tx_index).unwrap_or_default();
    let verified = MerkleTree::verify(&tree.root, &leaf, &proof, tx_index);

    Json(JsonRpcResponse {
        jsonrpc: "2.0".into(),
        result: Some(serde_json::json!({
            "slot": slot,
            "tx_index": tx_index,
            "transaction": hex::encode(&tx_bytes),
            "leaf": hex::encode(leaf),
            "proof": proof.iter().map(hex::encode).collect::<Vec<_>>(),
            "root": hex::encode(tree.root),
            "verified": verified,
        })),
        error: None,
        id,
    })
}

async fn handle_get_block_with_proofs(
    state: Arc<AppState>,
    params: Option<serde_json::Value>,
    id: u64,
) -> Json<JsonRpcResponse<serde_json::Value>> {
    let slot = param_u64(&params, 0).unwrap_or(0);
    let with_tx_data = param_bool(&params, 1).unwrap_or(true);

    let (txs, tree) = match load_slot_data(&state, slot).await {
        Some(v) => v,
        None => {
            return Json(JsonRpcResponse {
                jsonrpc: "2.0".into(),
                result: None,
                error: Some(format!("slot {} not found", slot)),
                id,
            })
        }
    };

    let tx_objects: Vec<serde_json::Value> = txs
        .iter()
        .enumerate()
        .map(|(i, tx)| {
            let leaf = tree.leaves.get(i).copied().unwrap_or([0u8; 32]);
            let proof = tree.prove(i).unwrap_or_default();
            let tx_bytes = bincode::serialize(tx).unwrap_or_default();
            let mut obj = serde_json::json!({
                "index": i,
                "signatures": tx.signatures.iter().map(|s| bs58::encode(s.as_ref()).into_string()).collect::<Vec<_>>(),
                "leaf": hex::encode(leaf),
                "proof": proof.iter().map(hex::encode).collect::<Vec<_>>(),
            });
            if with_tx_data {
                obj["transaction"] = serde_json::json!(hex::encode(&tx_bytes));
            }
            obj
        })
        .collect();

    let buf = state.ring_buffer.read().await;
    let parent_slot = buf
        .get(slot)
        .map(|d| d.parent_slot)
        .unwrap_or(slot.saturating_sub(1));
    drop(buf);

    Json(JsonRpcResponse {
        jsonrpc: "2.0".into(),
        result: Some(serde_json::json!({
            "slot": slot,
            "parentSlot": parent_slot,
            "num_transactions": txs.len(),
            "merkle_root": hex::encode(tree.root),
            "transactions": tx_objects,
        })),
        error: None,
        id,
    })
}

// ── Handler dispatch ───────────────────────────────────────────────────────

async fn jsonrpc_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<JsonRpcRequest>,
) -> Json<JsonRpcResponse<serde_json::Value>> {
    match req.method.as_str() {
        // Standard Solana methods
        "getSlot" => handle_get_slot(state, req.params, req.id).await,
        "getBlock" => handle_get_block(state, req.params, req.id).await,
        "getLatestSlot" => handle_get_latest_slot(state, req.id).await,

        // Trustless extensions
        "getProof" => handle_get_proof(state, req.params, req.id).await,
        "getTransactionByIndex" => handle_get_transaction_by_index(state, req.params, req.id).await,
        "getBlockWithProofs" => handle_get_block_with_proofs(state, req.params, req.id).await,

        _ => Json(JsonRpcResponse {
            jsonrpc: "2.0".into(),
            result: None,
            error: Some(format!("unknown method: {}", req.method)),
            id: req.id,
        }),
    }
}

const INDEX_HTML: &str = include_str!("../../../app/index.html");
