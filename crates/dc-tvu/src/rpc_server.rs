use crate::flat_file_store::FlatFileStore;
use crate::merkle_prover::MerkleTree;
use crate::ring_buffer::SlotRingBuffer;
use axum::{extract::State, Json, Router};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct AppState {
    pub ring_buffer: Arc<RwLock<SlotRingBuffer>>,
    pub file_store: Arc<RwLock<FlatFileStore>>,
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

#[derive(Serialize)]
pub struct SlotResponse {
    pub slot: u64,
    pub parent_slot: u64,
    pub num_transactions: usize,
    pub merkle_root: Option<[u8; 32]>,
}

#[derive(Serialize)]
pub struct ProofResponse {
    pub slot: u64,
    pub tx_index: usize,
    pub leaf: [u8; 32],
    pub proof: Vec<[u8; 32]>,
    pub root: [u8; 32],
    pub verified: bool,
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/jsonrpc", axum::routing::post(jsonrpc_handler))
        .with_state(state)
}

async fn jsonrpc_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<JsonRpcRequest>,
) -> Json<JsonRpcResponse<serde_json::Value>> {
    match req.method.as_str() {
        "getSlot" => handle_get_slot(state, req.params, req.id).await,
        "getProof" => handle_get_proof(state, req.params, req.id).await,
        "getLatestSlot" => handle_get_latest_slot(state, req.id).await,
        "getBlock" => handle_get_block(state, req.params, req.id).await,
        _ => Json(JsonRpcResponse {
            jsonrpc: "2.0".into(),
            result: None,
            error: Some(format!("unknown method: {}", req.method)),
            id: req.id,
        }),
    }
}

async fn handle_get_slot(
    state: Arc<AppState>,
    params: Option<serde_json::Value>,
    id: u64,
) -> Json<JsonRpcResponse<serde_json::Value>> {
    let slot = params
        .and_then(|p| p.as_array()?.first()?.as_u64())
        .unwrap_or(0);

    let buf = state.ring_buffer.read().await;
    let result = buf.get(slot);

    match result {
        Some(data) => Json(JsonRpcResponse {
            jsonrpc: "2.0".into(),
            result: Some(serde_json::json!({
                "slot": data.slot,
                "parent_slot": data.parent_slot,
                "num_transactions": data.num_transactions,
                "merkle_root": data.merkle_root.map(hex::encode),
            })),
            error: None,
            id,
        }),
        None => Json(JsonRpcResponse {
            jsonrpc: "2.0".into(),
            result: None,
            error: Some(format!("slot {} not found", slot)),
            id,
        }),
    }
}

async fn handle_get_latest_slot(
    state: Arc<AppState>,
    id: u64,
) -> Json<JsonRpcResponse<serde_json::Value>> {
    let buf = state.ring_buffer.read().await;
    let latest = buf.latest_slot().unwrap_or(0);

    let result = buf.get(latest);
    match result {
        Some(data) => Json(JsonRpcResponse {
            jsonrpc: "2.0".into(),
            result: Some(serde_json::json!({
                "slot": data.slot,
                "merkle_root": data.merkle_root.map(hex::encode),
                "num_transactions": data.num_transactions,
            })),
            error: None,
            id,
        }),
        None => Json(JsonRpcResponse {
            jsonrpc: "2.0".into(),
            result: None,
            error: Some("no slots available".into()),
            id,
        }),
    }
}

async fn handle_get_proof(
    state: Arc<AppState>,
    params: Option<serde_json::Value>,
    id: u64,
) -> Json<JsonRpcResponse<serde_json::Value>> {
    let arr = params
        .and_then(|p| p.as_array().map(|a| a.clone()))
        .unwrap_or_default();
    let slot = arr.first().and_then(|v| v.as_u64()).unwrap_or(0);
    let tx_index = arr
        .get(1)
        .and_then(|v| v.as_u64())
        .map(|i| i as usize)
        .unwrap_or(0);

    let buf = state.ring_buffer.read().await;
    let hot = buf.get(slot);

    if let Some(data) = hot {
        if let Some(ref tree) = data.merkle_tree {
            let leaf = tree.leaves.get(tx_index).copied().unwrap_or([0u8; 32]);
            let proof = tree.prove(tx_index).unwrap_or_default();
            let verified = MerkleTree::verify(&tree.root, &leaf, &proof, tx_index);
            return Json(JsonRpcResponse {
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
            });
        }
    }
    drop(buf);

    let store = state.file_store.read().await;
    let raw = store.load_slot(slot);
    drop(store);

    match raw {
        Some(data) => {
            let result = crate::deshredder::deshred_into_txs(&[data]);
            match result {
                Some(dr) => {
                    let tree = MerkleTree::new(&dr.transactions);
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
                None => Json(JsonRpcResponse {
                    jsonrpc: "2.0".into(),
                    result: None,
                    error: Some("failed to deserialize slot data".into()),
                    id,
                }),
            }
        }
        None => Json(JsonRpcResponse {
            jsonrpc: "2.0".into(),
            result: None,
            error: Some(format!("slot {} not found (cold)", slot)),
            id,
        }),
    }
}

async fn handle_get_block(
    state: Arc<AppState>,
    params: Option<serde_json::Value>,
    id: u64,
) -> Json<JsonRpcResponse<serde_json::Value>> {
    let slot = params
        .and_then(|p| p.as_array()?.first()?.as_u64())
        .unwrap_or(0);

    let buf = state.ring_buffer.read().await;
    let hot = buf.get(slot);

    let raw = if let Some(data) = hot {
        Some(data.entries.clone())
    } else {
        drop(buf);
        let store = state.file_store.read().await;
        match store.load_slot(slot) {
            Some(bytes) => {
                let result = crate::deshredder::deshred_into_txs(&[bytes]);
                result.map(|r| bincode::serialize(&r.entries).unwrap_or_default())
            }
            None => None,
        }
    };

    match raw {
        Some(entries_bytes) => {
            let entries: Vec<crate::deshredder::Entry> =
                bincode::deserialize(&entries_bytes).unwrap_or_default();
            let count: usize = entries.iter().map(|e| e.transactions.len()).sum();
            Json(JsonRpcResponse {
                jsonrpc: "2.0".into(),
                result: Some(serde_json::json!({
                    "slot": slot,
                    "num_entries": entries.len(),
                    "num_transactions": count,
                })),
                error: None,
                id,
            })
        }
        None => Json(JsonRpcResponse {
            jsonrpc: "2.0".into(),
            result: None,
            error: Some(format!("slot {} not found", slot)),
            id,
        }),
    }
}
