use elements::OutPoint;
use elements::{encode::Decodable, Transaction, Txid};
use log::info;
use serde_json::Value;

pub struct Node {
    client: reqwest::Client,
    base_url: String,
}

impl Node {
    pub fn new(base_url: String) -> Self {
        Node {
            client: reqwest::Client::new(),
            base_url,
        }
    }

    /// GET /rest/tx/<TX-HASH>.<bin|hex|json>
    pub async fn tx(
        &self,
        txid: Txid,
    ) -> Result<Transaction, Box<dyn std::error::Error + Send + Sync>> {
        let base = &self.base_url;
        let url = format!("{base}/rest/tx/{txid}.bin");

        let bytes = self.client.get(&url).send().await?.bytes().await?;

        let tx = Transaction::consensus_decode(bytes.as_ref())?;
        Ok(tx)
    }

    /// GET /rest/getutxos/checkmempool/<TXID>-<N>/<TXID>-<N>/.../<TXID>-<N>.<bin|hex|json>
    pub async fn get_utxos(
        &self,
        outpoint: OutPoint,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let base = &self.base_url;
        let outpoint_str = format!("{}-{}", outpoint.txid, outpoint.vout);
        let url = format!("{base}/rest/getutxos/checkmempool/{outpoint_str}.json");

        let json: Value = self.client.get(&url).send().await?.json().await?;
        info!("{:?}", json);
        Ok(json)
    }

    pub async fn is_spent(
        &self,
        outpoint: OutPoint,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        let json = self.get_utxos(outpoint).await?;
        Ok(json["utxos"]
            .as_array()
            .ok_or("utxos is not an array")?
            .is_empty())
    }
}
