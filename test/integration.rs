use futures_util::{SinkExt, StreamExt};
use jsonrpc_lite::{Id, JsonRpc};
use lwk_wollet::asyncr::EsploraClientBuilder;
use lwk_wollet::{ElementsNetwork, LiquidexProposal, Wollet};
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
use tokio_tungstenite::tungstenite::Message;

pub struct TestNode<'a> {
    elementsd: &'a BitcoinD,
}

impl<'a> TestNode<'a> {
    pub fn new(elementsd: &'a BitcoinD) -> Self {
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

    let test_node = TestNode::new(&elementsd);
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

    let test_node = TestNode::new(&elementsd);

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
        let mut result = jsonrpc.get_result().unwrap().clone();
        assert!(result.get("tx_hex").is_some());
        if let Value::Object(ref mut map) = result {
            map.remove("tx_hex");
        }
        assert_eq!(jsonrpc.get_id(), Some(Id::Num(-1)));
        assert_eq!(result, json!({ "txid": txid, "where": "block" }));
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

#[tokio::test]
async fn test_publish_proposal() {
    let network = ElementsNetwork::default_regtest();
    let elementsd_exe = env::var("ELEMENTSD_EXEC").expect("ELEMENTSD_EXEC must be set");
    let (elementsd, zmq_port) = launch_elementsd(elementsd_exe);
    let base_url = elementsd.rpc_url().to_string();

    let waterfalls =
        waterfalls::test_env::launch_with_node(elementsd, waterfalls::Family::Elements).await;

    let test_node = TestNode::new(waterfalls.node());

    // Rescan blockchain to recognize initialfreecoins
    test_node.rescan_blockchain().unwrap();

    let port = bitcoind::get_available_port().unwrap();

    let config = Config {
        base_url: base_url.clone(),
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

    // ===== Create two LWK wallets (Wallet A and Wallet B) =====

    // Create signers for both wallets
    let (signer_a, mut wollet_a) = Wollet::test_wallet().unwrap();
    let (signer_b, mut wollet_b) = Wollet::test_wallet().unwrap();

    // ===== Fund both wallets =====

    let address_a = wollet_a.address(None).unwrap();
    let address_b = wollet_b.address(None).unwrap();
    let _ = test_node
        .send_to_address(&address_a.address(), 10.0)
        .unwrap();
    let _ = test_node
        .send_to_address(&address_b.address(), 10.0)
        .unwrap();

    test_node.generate_to_address(1, &funding_address).unwrap();

    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    let mut waterfalls_client = EsploraClientBuilder::new(waterfalls.base_url(), network)
        .waterfalls(true)
        .build();
    let update_a = waterfalls_client
        .full_scan(&wollet_a)
        .await
        .unwrap()
        .unwrap();
    let update_b = waterfalls_client
        .full_scan(&wollet_b)
        .await
        .unwrap()
        .unwrap();

    wollet_a.apply_update(update_a).unwrap();
    wollet_b.apply_update(update_b).unwrap();

    // ===== Issue asset X in wallet B =====
    let asset_amount = 1000_u64; // Amount of asset to issue

    // Add issuance to the transaction
    // Note: The exact API for issue_asset needs to be confirmed from LWK master
    let mut pset = wollet_b
        .tx_builder()
        .issue_asset(
            asset_amount,
            None, // Use a wallet address
            0,    // No reissuance token amount
            None, // No token address
            None, // No contract
        )
        .unwrap()
        .finish()
        .unwrap();

    // Sign and finalize the transaction
    use lwk_common::Signer;
    signer_b.sign(&mut pset).unwrap();
    wollet_b.finalize(&mut pset).unwrap();
    let tx = pset.extract_tx().unwrap();

    // Broadcast via elements node
    let _txid_issue = waterfalls_client.broadcast(&tx).await.unwrap();

    // Generate block to confirm issuance
    test_node.generate_to_address(1, &funding_address).unwrap();

    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    let update_b = waterfalls_client
        .full_scan(&wollet_b)
        .await
        .unwrap()
        .unwrap();

    wollet_b.apply_update(update_b).unwrap();

    // Verify the trade completed successfully
    let balance_a = wollet_a.balance().unwrap();
    let balance_b = wollet_b.balance().unwrap();

    println!("Wallet A initial balance: {:?}", balance_a);
    println!("Wallet B initial balance: {:?}", balance_b);

    // ===== Create LiquiDEX proposal using TxBuilder liquidex_make from wallet B =====

    // Get the asset ID from the issuance transaction using the Node
    let assets = wollet_b.assets_owned().unwrap();
    let issued_asset_id = assets
        .into_iter()
        .filter(|asset| &network.policy_asset() != asset)
        .next()
        .unwrap();

    let utxos = wollet_b.utxos().unwrap();
    let outpoint_to_trade = utxos
        .into_iter()
        .filter(|u| u.unblinded.asset == issued_asset_id)
        .next()
        .unwrap();

    // Store values for later validation
    let lbtc_asset_id = network.policy_asset();
    let lbtc_amount = 10_000u64;
    let asset_amount_wanted = outpoint_to_trade.unblinded.value;

    // Create liquidex proposal from wallet B
    // Wallet B wants to trade asset X for 10_000 Lsats
    let receive_address = wollet_b.address(None).unwrap();
    let mut pset = wollet_b
        .tx_builder()
        .liquidex_make(
            outpoint_to_trade.outpoint,
            receive_address.address(),
            lbtc_amount,
            network.policy_asset(),
        )
        .unwrap()
        .finish()
        .unwrap();
    signer_b.sign(&mut pset).unwrap();
    let proposal = LiquidexProposal::from_pset(&pset).unwrap();

    // The proposal should be a LiquidexProposal<Unvalidated>
    // It can then be wrapped in our Proposal struct for the nexus relay
    let nexus_proposal = nexus_relay::jsonrpc::Proposal(proposal);

    // Connect to the WebSocket server for nexus relay testing
    let ws_url = format!("ws://127.0.0.1:{}", port);
    let (ws_stream, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .expect("Failed to connect to WebSocket");

    let (mut ws_sender, mut ws_receiver) = ws_stream.split();

    // ===== TODO: Subscribe to the pair the proposal was created on =====
    // This uses the existing pair subscription functionality

    use serde_json::json;

    let id = 12345;

    // Create pair subscription manually using the JSON structure
    // The proposal trades issued_asset for L-BTC, so we subscribe to that pair
    let subscribe_message = json!({
        "id": id,
        "jsonrpc": "2.0",
        "method": "subscribe",
        "params": {
            "pair": {
                "input": issued_asset_id.to_string(),       // Asset X ID (what the proposal offers)
                "output": network.policy_asset().to_string() // L-BTC asset ID (what the proposal wants)
            }
        }
    });

    ws_sender
        .send(Message::Text(subscribe_message.to_string()))
        .await
        .expect("Failed to send subscribe message");

    // Wait for subscription confirmation
    if let Some(Ok(Message::Text(text))) = ws_receiver.next().await {
        println!("Subscribe response: {}", text);
        let response = NexusResponse::new_subscribed(id);
        let jsonrpc = JsonRpc::parse(&text).unwrap();
        assert_eq!(jsonrpc, response.into());
    } else {
        panic!("Failed to receive subscription confirmation");
    }

    // ===== Send the proposal via the relay =====
    // Note: This section is commented out because we don't have a real proposal yet
    // The liquidex_make API is not available in the current LWK version
    // Once the API is available, this would create and send a real proposal

    let publish_id = 54321;
    let publish_message =
        NexusRequest::new_publish_proposal(publish_id, nexus_proposal).to_string();

    ws_sender
        .send(Message::Text(publish_message))
        .await
        .expect("Failed to send proposal");

    // The subscribed client should receive the proposal notification first
    // (this happens immediately when the proposal is published)
    let received_proposal: lwk_wollet::LiquidexProposal<lwk_wollet::Validated> =
        if let Some(Ok(Message::Text(text))) = ws_receiver.next().await {
            let jsonrpc = JsonRpc::parse(&text).unwrap();
            assert_eq!(jsonrpc.get_id(), Some(jsonrpc_lite::Id::Num(-1))); // Notification

            // Extract and validate the proposal
            if let Some(result) = jsonrpc.get_result() {
                let proposal: lwk_wollet::LiquidexProposal<lwk_wollet::Unvalidated> =
                    serde_json::from_value(result.clone()).expect("Failed to parse proposal");

                let validated_proposal = proposal.insecure_validate().unwrap();
                // Validate the proposal matches what we sent
                // The proposal offers the issued asset and wants L-BTC
                assert_eq!(validated_proposal.input().asset, issued_asset_id); // Input: issued asset
                assert_eq!(validated_proposal.output().asset, lbtc_asset_id); // Output: L-BTC
                assert_eq!(validated_proposal.input().amount, asset_amount_wanted); // Input amount: issued asset amount
                assert_eq!(validated_proposal.output().amount, lbtc_amount); // Output amount: L-BTC amount wanted
                validated_proposal
            } else {
                panic!("Expected proposal in notification result");
            }
        } else {
            panic!("Failed to receive proposal notification");
        };

    // Now wait for publish confirmation response
    if let Some(Ok(Message::Text(text))) = ws_receiver.next().await {
        println!("Publish response: {}", text);
        let response = NexusResponse::new_published(publish_id);
        let jsonrpc = JsonRpc::parse(&text).unwrap();
        assert_eq!(jsonrpc, response.into());
    } else {
        panic!("Failed to receive publish confirmation");
    }

    // ===== Accept the proposal in wallet A =====
    // Wallet A receives the proposal and decides to accept it
    // This involves creating a taking transaction using liquidex_take

    println!("Accepting the LiquiDEX proposal...");

    // Create the take transaction
    let mut take_pset = wollet_a
        .tx_builder()
        .liquidex_take(vec![received_proposal])
        .unwrap()
        .finish()
        .unwrap();

    // Sign the take transaction
    signer_a.sign(&mut take_pset).unwrap();
    wollet_a.finalize(&mut take_pset).unwrap();
    let take_tx = take_pset.extract_tx().unwrap();

    // Broadcast the trade transaction
    let txid_trade = waterfalls_client.broadcast(&take_tx).await.unwrap();
    println!("Trade transaction broadcasted: {}", txid_trade);

    // Generate block to confirm the trade
    test_node.generate_to_address(1, &funding_address).unwrap();
    println!("Block generated to confirm trade");

    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    // Update both wallets to see the completed trade
    let update_a = waterfalls_client
        .full_scan(&wollet_a)
        .await
        .unwrap()
        .unwrap();
    let update_b = waterfalls_client
        .full_scan(&wollet_b)
        .await
        .unwrap()
        .unwrap();

    wollet_a.apply_update(update_a).unwrap();
    wollet_b.apply_update(update_b).unwrap();

    // Verify the trade completed successfully
    let balance_a = wollet_a.balance().unwrap();
    let balance_b = wollet_b.balance().unwrap();

    println!("Wallet A full balance: {:?}", balance_a);
    println!("Wallet B full balance: {:?}", balance_b);

    // Wallet A should now have the issued asset
    let asset_balance_a = balance_a.get(&issued_asset_id).unwrap_or(&0);
    println!(
        "Wallet A asset balance: {} (expected at least: {})",
        asset_balance_a, asset_amount_wanted
    );

    // Wallet B should now have the L-BTC
    let lbtc_balance_b = balance_b.get(&lbtc_asset_id).unwrap_or(&0);
    println!(
        "Wallet B L-BTC balance: {} (expected at least: {})",
        lbtc_balance_b, lbtc_amount
    );

    // Check if the trade actually happened by looking at total L-BTC amounts
    let total_lbtc_a = balance_a.get(&lbtc_asset_id).unwrap_or(&0);
    let total_lbtc_b = balance_b.get(&lbtc_asset_id).unwrap_or(&0);
    println!(
        "Total L-BTC - Wallet A: {}, Wallet B: {}",
        total_lbtc_a, total_lbtc_b
    );

    // The trade should have transferred assets between wallets
    // Wallet A should have received the issued asset
    // Wallet B should have received the L-BTC
    assert!(
        *asset_balance_a >= asset_amount_wanted,
        "Wallet A should have received the issued asset"
    );
    assert!(
        *lbtc_balance_b >= lbtc_amount,
        "Wallet B should have received the L-BTC"
    );

    println!("✓ LiquiDEX trade completed successfully!");

    waterfalls.shutdown().await;
}
