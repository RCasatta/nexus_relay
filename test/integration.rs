use futures_util::{SinkExt, StreamExt};
use jsonrpc_lite::{Id, JsonRpc};
use lwk_wollet::{Wollet, WolletDescriptor};
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

#[tokio::test]
async fn test_publish_proposal() {
    // Note: This test requires the liquidex_make functionality from LWK master branch
    // Current LWK release (0.9.0) does not include liquidex_make, but the master branch should

    let elementsd_exe = env::var("ELEMENTSD_EXEC").expect("ELEMENTSD_EXEC must be set");
    let (elementsd, zmq_port) = launch_elementsd(elementsd_exe);
    let base_url = elementsd.rpc_url().to_string();

    let test_node = TestNode::new(elementsd);

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
    let (signer_a, wollet_a) = Wollet::test_wallet().unwrap();
    let (signer_b, wollet_b) = Wollet::test_wallet().unwrap();

    // ===== Fund both wallets =====

    let address_a = wollet_a.address(None).unwrap(); // Get next address from wallet A
    let address_b = wollet_b.address(None).unwrap(); // Get next address from wallet B
                                                     // Fund the wallets
    let txid_a = test_node
        .send_to_address(&address_a.address(), 10.0)
        .unwrap();
    let txid_b = test_node
        .send_to_address(&address_b.address(), 10.0)
        .unwrap();

    // Generate a block to confirm transactions
    test_node.generate_to_address(1, &funding_address).unwrap();

    // Update wallet states (sync with blockchain)
    // This requires setting up electrum/esplora client or using Elements RPC
    // let update_a = Update::from_rpc(...)?; // Get updates from elements node
    // let update_b = Update::from_rpc(...)?;
    // wallet_a.apply_update(update_a)?;
    // wallet_b.apply_update(update_b)?;

    // ===== TODO: Issue asset X in wallet B =====
    // This involves using LWK's asset issuance functionality
    /*
    use lwk_wollet::TxBuilder;

    // Create asset issuance transaction in wallet B
    let asset_amount = 1000_u64; // Amount of asset to issue
    let asset_address = wallet_b.address(None)?; // Address to receive the asset

    let mut tx_builder = TxBuilder::new();

    // Add issuance to the transaction
    // Note: The exact API for issue_asset needs to be confirmed from LWK master
    let issuance_output = tx_builder
        .add_issuance(
            asset_amount,
            &asset_address,
            None, // No token amount
            None, // No token address
        )?;

    let asset_id = issuance_output.asset; // Get the generated asset ID

    // Build the transaction
    let pset = tx_builder.finish(&wallet_b)?;

    // Sign and finalize the transaction
    let signed_pset = signer_b.sign(&pset)?;
    let tx = signed_pset.extract_tx()?;

    // Broadcast via elements node
    let txid_issue = test_node.broadcast_tx(&tx)?;

    // Generate block to confirm issuance
    test_node.generate_to_address(1, &funding_address)?;

    // Update wallet B to see the new asset
    let update_b = Update::from_rpc(...)?;
    wallet_b.apply_update(update_b)?;
    */

    // ===== TODO: Create LiquiDEX proposal using TxBuilder liquidex_make from wallet A =====
    // This is the core functionality that needs liquidex_make from LWK master
    /*
    use lwk_wollet::{TxBuilder, LiquidexProposal};

    // Create liquidex proposal from wallet A
    // Wallet A wants to trade L-BTC for asset X at a feasible rate
    let mut tx_builder = TxBuilder::new();

    let lbtc_amount = 500_000_u64; // Amount of L-BTC to offer (0.005 BTC)
    let asset_amount_wanted = 100_u64; // Amount of asset X wanted (feasible 5:1 ratio)

    // Get L-BTC asset ID (should be the network's native asset)
    let lbtc_asset_id = &Network::ElementsRegtest.policy_asset(); // or similar method

    // Create the proposal using liquidex_make
    // Note: This is the key functionality that needs to be available in LWK master
    let proposal = tx_builder
        .liquidex_make(
            lbtc_amount,                    // Amount offering
            lbtc_asset_id,                  // L-BTC asset ID (native asset)
            asset_amount_wanted,            // Amount wanted
            &asset_id,                      // Asset ID of asset X from issuance
            // Additional parameters may be needed based on actual API
        )?
        .finish(&wallet_a)?;

    // The proposal should be a LiquidexProposal<Unvalidated>
    // It can then be wrapped in our Proposal struct for the nexus relay
    let nexus_proposal = crate::jsonrpc::Proposal { proposal };
    */

    // Connect to the WebSocket server for nexus relay testing
    let ws_url = format!("ws://127.0.0.1:{}", port);
    let (ws_stream, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .expect("Failed to connect to WebSocket");

    let (mut ws_sender, mut ws_receiver) = ws_stream.split();

    // ===== TODO: Subscribe to the pair the proposal was created on =====
    // This uses the existing pair subscription functionality
    /*
    use serde_json::json;

    let id = 12345;

    // Create pair subscription manually using the JSON structure
    let subscribe_message = json!({
        "id": id,
        "jsonrpc": "2.0",
        "method": "subscribe",
        "params": {
            "pair": {
                "input": lbtc_asset_id.to_string(),  // L-BTC asset ID
                "output": asset_id.to_string()       // Asset X ID
            }
        }
    });

    use tokio_tungstenite::tungstenite::Message as TungsteniteMessage;
    ws_sender
        .send(TungsteniteMessage::Text(subscribe_message.to_string()))
        .await
        .expect("Failed to send subscribe message");

    // Wait for subscription confirmation
    if let Some(Ok(TungsteniteMessage::Text(text))) = ws_receiver.next().await {
        println!("Subscribe response: {}", text);
        let response = NexusResponse::new_subscribed(id);
        let jsonrpc = JsonRpc::parse(&text).unwrap();
        assert_eq!(jsonrpc, response.into());
    } else {
        panic!("Failed to receive subscription confirmation");
    }
    */

    // ===== TODO: Send the proposal via the relay =====
    /*
    let publish_id = 54321;
    let publish_message = NexusRequest::new_publish_proposal(publish_id, nexus_proposal).to_string();

    ws_sender
        .send(TungsteniteMessage::Text(publish_message))
        .await
        .expect("Failed to send proposal");

    // Wait for publish confirmation
    if let Some(Ok(TungsteniteMessage::Text(text))) = ws_receiver.next().await {
        println!("Publish response: {}", text);
        let response = NexusResponse::new_published(publish_id);
        let jsonrpc = JsonRpc::parse(&text).unwrap();
        assert_eq!(jsonrpc, response.into());
    } else {
        panic!("Failed to receive publish confirmation");
    }
    */

    // ===== TODO: Retrieve the proposal from the relay =====
    /*
    // The subscribed client should receive the proposal notification
    if let Some(Ok(TungsteniteMessage::Text(text))) = ws_receiver.next().await {
        println!("Received proposal: {}", text);

        let jsonrpc = JsonRpc::parse(&text).unwrap();
        assert_eq!(jsonrpc.get_id(), Some(jsonrpc_lite::Id::Num(-1))); // Notification

        // Extract and validate the proposal
        if let Some(result) = jsonrpc.get_result() {
            let received_proposal: lwk_wollet::LiquidexProposal<lwk_wollet::Unvalidated> =
                serde_json::from_value(result.clone()).expect("Failed to parse proposal");

            // Validate the proposal matches what we sent
            assert_eq!(received_proposal.input().asset, lbtc_asset_id);
            assert_eq!(received_proposal.output().asset, asset_id);
            assert_eq!(received_proposal.input().satoshi, lbtc_amount);
            assert_eq!(received_proposal.output().satoshi, asset_amount_wanted);
        } else {
            panic!("Expected proposal in notification result");
        }
    } else {
        panic!("Failed to receive proposal notification");
    }
    */

    // ===== TODO: Accept the proposal in wallet B =====
    /*
    // Wallet B receives the proposal and decides to accept it
    // This involves creating a taking transaction using liquidex_take

    use lwk_wollet::TxBuilder;

    let mut take_builder = TxBuilder::new();

    // Create the take transaction
    // Note: liquidex_take API needs to be confirmed from LWK master
    let take_pset = take_builder
        .liquidex_take(&received_proposal)?
        .finish(&wallet_b)?;

    // Sign and finalize the take transaction
    let signed_take = signer_b.sign(&take_pset)?;
    let final_tx = signed_take.extract_tx()?;

    // Broadcast the trade transaction
    let txid_trade = test_node.broadcast_tx(&final_tx)?;

    // Generate block to confirm the trade
    test_node.generate_to_address(1, &funding_address)?;

    // Update both wallets to see the completed trade
    let update_a = Update::from_rpc(...)?;
    let update_b = Update::from_rpc(...)?;
    wallet_a.apply_update(update_a)?;
    wallet_b.apply_update(update_b)?;

    // Verify the trade completed successfully
    let wallet_a_balances = wallet_a.balance()?;
    let wallet_b_balances = wallet_b.balance()?;

    // Wallet A should now have asset X
    assert!(wallet_a_balances.get(&asset_id).unwrap_or(&0) >= &asset_amount_wanted);
    // Wallet B should now have the L-BTC
    assert!(wallet_b_balances.get(lbtc_asset_id).unwrap_or(&0) >= &lbtc_amount);

    println!("✓ LiquiDEX trade completed successfully!");
    */

    // ===== CURRENT WORKING IMPLEMENTATION =====
    // For now, just ensure the basic relay infrastructure is working
    // This part works with the current codebase

    println!("test_publish_proposal: Skeleton implementation completed");
    println!(
        "TODO: Implement wallet creation, funding, asset issuance, and liquidex functionality"
    );
    println!("TODO: This test requires the liquidex_make and liquidex_take functionality from LWK master branch");
    println!("TODO: Key missing APIs: TxBuilder::liquidex_make(), TxBuilder::liquidex_take()");
    println!("TODO: Also need proper wallet creation, funding, and asset issuance workflows");

    // Test basic ping to ensure the relay is running
    let id = 99999;
    let ping_message = NexusRequest::new_ping(id).to_string();

    use tokio_tungstenite::tungstenite::Message as TungsteniteMessage;
    ws_sender
        .send(TungsteniteMessage::Text(ping_message))
        .await
        .expect("Failed to send ping");

    if let Some(Ok(TungsteniteMessage::Text(text))) = ws_receiver.next().await {
        println!("Ping response: {}", text);
        let jsonrpc = JsonRpc::parse(&text).unwrap();
        let expected_response = NexusResponse::new_pong(id);
        assert_eq!(jsonrpc, expected_response.into());
        println!("✓ Nexus relay is running and responding correctly");
        println!("✓ Test infrastructure is ready for liquidex implementation");
    } else {
        panic!("Failed to receive ping response");
    }
}
