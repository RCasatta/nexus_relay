use elements::Address;
use elements::{encode::Decodable, Transaction, Txid};
use std::ffi::OsStr;
use std::str::FromStr;
use tokio::runtime::Runtime;

pub struct Client {
    client: reqwest::Client,
    base_url: String,
}

impl Client {
    pub fn new(base_url: String) -> Self {
        Client {
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

        println!("bytes: {:?}", bytes);

        let tx = Transaction::consensus_decode(bytes.as_ref())?;
        Ok(tx)
    }
}

#[cfg(test)]
mod tests {
    use bitcoind::bitcoincore_rpc::RpcApi;
    use bitcoind::{BitcoinD, Conf};
    use elements::Address;
    use elements::Txid;
    use serde_json::Value;
    use std::env;
    use std::ffi::OsStr;
    use std::str::FromStr;
    use tokio::runtime::Runtime;

    use super::Client;

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

        let client = Client::new(elementsd.rpc_url());

        let addr: Value = elementsd
            .client
            .call("getnewaddress", &["label".into(), "p2sh-segwit".into()])
            .unwrap();
        let address = Address::from_str(addr.as_str().unwrap())
            .unwrap()
            .to_string();

        elementsd
            .client
            .call::<Value>("generatetoaddress", &[101.into(), address.into()])
            .unwrap();

        // let block = elementsd.client.get_block(&block_hashes[101]).unwrap();
        // let txid = Txid::from_str(&block.txdata[0].txid().to_string()).unwrap();

        // // Create a tokio runtime
        // let rt = Runtime::new().unwrap();

        // // Use block_on to run the async code
        // let tx = rt.block_on(client.tx(txid)).unwrap();

        // println!("{:?}", tx);
    }
}
