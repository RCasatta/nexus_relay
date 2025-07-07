use futures_util::{SinkExt, StreamExt};
use jsonrpc_lite::{Id, JsonRpc};
use nexus_relay::jsonrpc::{NexusRequest, NexusResponse};
use nexus_relay::node::Node;
use nexus_relay::{async_main, Config, Network};

use bitcoind::bitcoincore_rpc::RpcApi;
use bitcoind::{BitcoinD, Conf};
use elements::encode::Decodable;
use elements::{Address, BlockHash};
use serde_json::{json, Value};
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
    pub fn send_to_address(
        &self,
        address: &Address,
        amount: f64,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let txid: Value = self
            .elementsd
            .client
            .call(
                "sendtoaddress",
                &[address.to_string().into(), amount.into()],
            )
            .unwrap();
        Ok(txid.as_str().unwrap().to_string())
    }
    pub fn get_balance(&self) -> Result<f64, Box<dyn std::error::Error + Send + Sync>> {
        let balance: Value = self.elementsd.client.call("getbalance", &[]).unwrap();
        println!("Raw balance response: {:?}", balance);

        // In Elements, balance might be an object with asset information
        if let Some(balance_num) = balance.as_f64() {
            Ok(balance_num)
        } else if let Some(balance_obj) = balance.as_object() {
            // Try to get the bitcoin/L-BTC balance
            if let Some(btc_balance) = balance_obj.values().next() {
                Ok(btc_balance.as_f64().unwrap_or(0.0))
            } else {
                Ok(0.0)
            }
        } else {
            Ok(0.0)
        }
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
    pub fn rescan_blockchain(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.elementsd
            .client
            .call::<Value>("rescanblockchain", &[])
            .unwrap();
        Ok(())
    }
}

fn launch_elementsd<S: AsRef<OsStr>>(exe: S) -> (BitcoinD, u16) {
    let zmq_port = bitcoind::get_available_port().unwrap();
    let zmq1 = format!("-zmqpubrawblock=tcp://127.0.0.1:{}", zmq_port);
    let zmq2 = format!("-zmqpubrawtx=tcp://127.0.0.1:{}", zmq_port);
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
        zmq1.as_str(),
        zmq2.as_str(),
    ];
    conf.args = args;
    conf.view_stdout = std::env::var("RUST_LOG").is_ok();
    conf.network = "liquidregtest";

    let elementsd = BitcoinD::with_conf(exe, &conf).unwrap();
    (elementsd, zmq_port)
}

#[test]
fn test_launch_elementsd() {
    let elementsd_exe = env::var("ELEMENTSD_EXEC").expect("ELEMENTSD_EXEC must be set");
    let (elementsd, _) = launch_elementsd(elementsd_exe);
    let node = Node::new(elementsd.rpc_url());

    let test_node = TestNode::new(elementsd);
    let address = test_node.get_new_address().unwrap();

    test_node.generate_to_address(101, &address).unwrap();

    // Check balance and generate more blocks if needed
    let balance = test_node.get_balance().unwrap();
    println!("Wallet balance after 101 blocks: {}", balance);

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

#[tokio::test]
async fn test_publish_from_zmq() {
    let elementsd_exe = env::var("ELEMENTSD_EXEC").expect("ELEMENTSD_EXEC must be set");
    let (elementsd, zmq_port) = launch_elementsd(elementsd_exe);
    let base_url = elementsd.rpc_url().to_string();

    let test_node = TestNode::new(elementsd);

    // Rescan blockchain to recognize initialfreecoins
    test_node.rescan_blockchain().unwrap();

    let port = bitcoind::get_available_port().unwrap();

    let config = Config {
        base_url,
        zmq_endpoint: format!("tcp://127.0.0.1:{}", zmq_port),
        network: Network::ElementsRegtest,
        port,
    };

    tokio::spawn(async move {
        async_main(config).await.unwrap();
    });

    // Give the server a moment to start up
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Generate some initial blocks to get funds
    let funding_address = test_node.get_new_address().unwrap();
    test_node
        .generate_to_address(101, &funding_address)
        .unwrap();

    // Check balance and generate more blocks if needed
    let balance = test_node.get_balance().unwrap();
    assert!(balance > 0.0);

    // Get a new address to subscribe to (different from funding address)
    let target_address = test_node.get_new_address().unwrap();
    let target_address_str = target_address.to_unconfidential().to_string();

    // Connect to the WebSocket server
    let ws_url = format!("ws://127.0.0.1:{}", port);
    let (ws_stream, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .expect("Failed to connect to WebSocket");

    let (mut ws_sender, mut ws_receiver) = ws_stream.split();

    // Subscribe to the target address
    let id = 12345;
    let subscribe_message =
        NexusRequest::new_address_subscribe(id, &target_address_str).to_string();

    println!("Subscribe message: {}", subscribe_message);

    use tokio_tungstenite::tungstenite::Message as TungsteniteMessage;
    ws_sender
        .send(TungsteniteMessage::Text(subscribe_message))
        .await
        .expect("Failed to send subscribe message");

    // Wait for subscription confirmation
    if let Some(Ok(TungsteniteMessage::Text(text))) = ws_receiver.next().await {
        println!("Subscribe response: {}", text);
        let response = NexusResponse::new_subscribed(id);
        let jsonrpc = JsonRpc::parse(&text).unwrap();
        assert_eq!(jsonrpc, response.into());
    } else {
        assert!(false);
    }

    // Now send funds to the target address (this should trigger a rawtx ZMQ notification)
    let txid = test_node.send_to_address(&target_address, 1.0).unwrap();
    println!(
        "Sent transaction to address: {}",
        target_address.to_string()
    );

    // Wait for the address notification from ZMQ
    if let Some(Ok(TungsteniteMessage::Text(text))) = ws_receiver.next().await {
        println!("Received message: {}", text);
        let jsonrpc = JsonRpc::parse(&text).unwrap();
        assert_eq!(jsonrpc.get_id(), Some(Id::Num(-1)));
        assert_eq!(
            jsonrpc.get_result(),
            Some(&json!({ "address": target_address_str, "where": "mempool" }))
        );
    } else {
        assert!(false);
    }

    // note we are not waiting for the txid seen in mempoolnotification from ZMQ, because we are not subscribed to it

    // Subscribe to the transaction ID
    let id = 54321;
    let subscribe_txid_message = NexusRequest::new_txid_subscribe(id, &txid).to_string();
    println!("Subscribe to txid message: {}", subscribe_txid_message);

    ws_sender
        .send(TungsteniteMessage::Text(subscribe_txid_message))
        .await
        .expect("Failed to send txid subscribe message");

    // Wait for subscription confirmation
    if let Some(Ok(TungsteniteMessage::Text(text))) = ws_receiver.next().await {
        println!("Txid subscribe response: {}", text);
        let response = NexusResponse::new_subscribed(id);
        let jsonrpc = JsonRpc::parse(&text).unwrap();
        assert_eq!(jsonrpc, response.into());
    } else {
        assert!(false);
    }

    // Generate a block to confirm the transaction
    test_node.generate_to_address(1, &funding_address).unwrap();
    println!("Generated block to confirm transaction");

    if let Some(Ok(TungsteniteMessage::Text(text))) = ws_receiver.next().await {
        println!("Received txid confirmation message: {}", text);

        // Check if this is a RESULT message containing our txid
        let jsonrpc = JsonRpc::parse(&text).unwrap();
        assert_eq!(jsonrpc.get_id(), Some(Id::Num(-1)));
        assert_eq!(
            jsonrpc.get_result(),
            Some(&json!({ "txid": txid, "where": "block" }))
        );
    } else {
        assert!(false);
    }

    if let Some(Ok(TungsteniteMessage::Text(text))) = ws_receiver.next().await {
        println!("Received address confirmation message: {}", text);

        // Check if this is a RESULT message containing our txid
        let jsonrpc = JsonRpc::parse(&text).unwrap();
        assert_eq!(jsonrpc.get_id(), Some(Id::Num(-1)));
        assert_eq!(
            jsonrpc.get_result(),
            Some(&json!({ "address": target_address_str, "where": "block" }))
        );
    } else {
        assert!(false);
    }
}
