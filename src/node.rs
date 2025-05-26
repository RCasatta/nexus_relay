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
    async fn get_utxos(
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

#[cfg(test)]
mod tests {
    use super::Node;
    use bitcoind::bitcoincore_rpc::RpcApi;
    use bitcoind::{BitcoinD, Conf};
    use elements::encode::Decodable;
    use elements::{Address, BlockHash};
    use serde_json::Value;
    use std::env;
    use std::ffi::OsStr;
    use std::str::FromStr;
    use tokio::runtime::Runtime;
    pub struct TestNode {
        elementsd: BitcoinD,
    }

    impl TestNode {
        pub fn new(elementsd: BitcoinD) -> Self {
            Self { elementsd }
        }
        pub fn get_new_address(&self) -> Result<Address, Box<dyn std::error::Error + Send + Sync>> {
            let addr: Value = self
                .elementsd
                .client
                .call("getnewaddress", &["label".into(), "p2sh-segwit".into()])
                .unwrap();
            let address = Address::from_str(addr.as_str().unwrap()).unwrap();
            Ok(address)
        }
        pub fn generate_to_address(
            &self,
            block_num: u64,
            address: &Address,
        ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            self.elementsd
                .client
                .call::<Value>(
                    "generatetoaddress",
                    &[block_num.into(), address.to_string().into()],
                )
                .unwrap();
            Ok(())
        }
        pub fn get_block_hash(
            &self,
            block_num: u64,
        ) -> Result<BlockHash, Box<dyn std::error::Error + Send + Sync>> {
            let block: Value = self
                .elementsd
                .client
                .call("getblockhash", &[block_num.into()])
                .unwrap();
            let string = block.as_str().unwrap();
            let block_hash = BlockHash::from_str(string).unwrap();
            Ok(block_hash)
        }
        pub fn get_block(
            &self,
            block_hash: BlockHash,
        ) -> Result<elements::Block, Box<dyn std::error::Error + Send + Sync>> {
            let block: Value = self
                .elementsd
                .client
                .call("getblock", &[block_hash.to_string().into(), 0.into()])
                .unwrap();
            let string = block.as_str().unwrap();
            let bytes = hex::decode(string).unwrap();
            let block = elements::Block::consensus_decode(&bytes[..]).unwrap();
            Ok(block)
        }
    }

    fn launch_elementsd<S: AsRef<OsStr>>(exe: S) -> BitcoinD {
        let mut conf = Conf::default();
        let args = vec![
            "-fallbackfee=0.0001",
            "-dustrelayfee=0.00000001",
            "-chain=liquidregtest",
            "-initialfreecoins=2100000000",
            "-validatepegin=0",
            "-acceptdiscountct=1",
            "-txindex=1",
            "-rest=1",
        ];
        conf.args = args;
        conf.view_stdout = std::env::var("RUST_LOG").is_ok();
        conf.network = "liquidregtest";

        BitcoinD::with_conf(exe, &conf).unwrap()
    }

    #[test]
    fn test_launch_elementsd() {
        let elementsd_exe = env::var("ELEMENTSD_EXEC").expect("ELEMENTSD_EXEC must be set");
        let elementsd = launch_elementsd(elementsd_exe);
        let node = Node::new(elementsd.rpc_url());

        let test_node = TestNode::new(elementsd);

        let address = test_node.get_new_address().unwrap();

        test_node.generate_to_address(101, &address).unwrap();

        let block_hash = test_node.get_block_hash(101).unwrap();
        let block = test_node.get_block(block_hash).unwrap();
        let txid = block.txdata[0].txid();

        // Create a tokio runtime
        let rt = Runtime::new().unwrap();

        // Use block_on to run the async code
        let tx = rt.block_on(node.tx(txid)).unwrap();

        let outpoint = elements::OutPoint {
            txid: txid,
            vout: 1,
        };

        let _utxos = rt.block_on(node.get_utxos(outpoint)).unwrap();
        let is_spent = rt.block_on(node.is_spent(outpoint)).unwrap();
        assert_eq!(tx.txid(), block.txdata[0].txid());
        assert!(is_spent);
    }
}
