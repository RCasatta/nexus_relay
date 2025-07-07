#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("JSON-RPC error: {0}")]
    JsonRpcLite(#[from] jsonrpc_lite::Error),

    #[error("error: {0}")]
    JsonRpc(#[from] crate::jsonrpc::Error),

    #[error("elements error: {0}")]
    Elements(#[from] elements::encode::Error),
}
