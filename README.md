# NexusRelay: A WebSocket Pub/Sub Relay for Wallet Coordination

A WebSocket server with a simple publish/subscribe text based message protocol useful for the following use-cases:

- Publish Liquidex swap proposals and to subscribe to receive proposals from others. To make and take proposal you can use [LWK](https://github.com/blockstream/lwk)
- Share PSETs for signing in multisignature setups.
- Create multisignature setups.
- Listen for transaction activity on addresses.

Note in the most generic case this is a simple relay mechanism or a "dumb pipe" providing a transport layer which can be encrypted via ssl but it leaves other security implementation and details to the clients.

In some specific cases, like publishing a Liquidex Proposal, a PSET or subscribbing to address, there is some validation provided by the server.

## Building and Running

Build and run the server:

```bash
cargo build
cargo run
```

## Protocol

The server uses [JSON-RPC 2.0](https://www.jsonrpc.org/specification) with some restriction outlined below over WebSocket.

### JSON-RPC Method

There are the following methods:

* `publish`
* `subscribe`
* `unsubscribe`

### JSON-RPC Id

Id must be an positive integer (which is a restriction over JSON-RPC)

The only exception are notifications sent from the server that use `-1`. Notification are message sent spontanously (for example when an event is triggered for an active subscription) from the server tha don't require a response.
We do this because JSON-RPC [notifications](https://www.jsonrpc.org/specification#notification) don't hold a value.

### JSON-RPC Params

Params must be a Map with a single value, which is another restriction in comparison to JSON-RPC.
This is done to discriminate the subcategory of the method.

## Methods

### Ping/Pong

Request:

```json
{"jsonrpc": "2.0", "method": "publish", "id": 1, "params": {"ping": null} }
```

Response:
```json
{"jsonrpc": "2.0", "result": "pong", "id": 1 }
```

### Subscribe

There are various different subscription:

* `ping` a special subscribe returning a pong to the caller
* `any` subscribe on a general`topic` without any filtering from the nexus relay
* `address` subscribe to the given unconfidential address to be informed about transaction containing this address entering the mempool or being confirmed in a block
* `txid` subscribe to the transaction id to receive information when the transaction enter the mempool or it's confirmed in a block
* `wallet` subscribe to pset created for wallet identified by `<wallet_id>`
* `proposal` subscribe to receive liquidex proposals involving `<buy_asset_id>` and `<sell_asset_id>`

Any:

```json
{"jsonrpc": "2.0", "method": "subscribe", "id": 1, "params": {"any": {"topic":"the-topic"}}}
```

Pset:

https://github.com/bitcoin/bips/blob/master/bip-0370.mediawiki#unique-identification

### Publish

There are 3 possible publishing:

* `any` general one
* `proposal` publish a liquidex proposal
* `pset` publish a pset


#### Publish proposal


```json
{"jsonrpc": "2.0", "method": "ping", "id": 1, "params": {"proposal": $PROPOSAL_JSON }
```


Where `$PROPOSAL_JSON` has the following format (removed tx hex for brevity):

```json
{
    "version": 1,
    "tx": "<txhex>",
    "inputs": [{
        "asset":"6921c799f7b53585025ae8205e376bfd2a7c0571f781649fb360acece252a6a7",
        "asset_blinder":"5bfa3c033ed871572f3012eda1d2c9ae2ec662be07ed267a09f1b44c06eee86f",
        "satoshi":10000,
        "blind_value_proof":"2000000000000027101988645f8abc1ef9086b5d8e4c41c0e42475b412fc22245779f597007c5f48cb9c61ff6535866ef5150b326bf69c0d8c291f8c8e6afc015311ed405b74baf9d4"
    }],
    "outputs": [{
        "asset":"f13806d2ab6ef8ba56fc4680c1689feb21d7596700af1871aef8c2d15d4bfd28",
        "asset_blinder":"1e8c347f88a3e46e97a9a6556710812137a10b939983aef8a8f95da466abfae5",
        "satoshi":10000,
        "blind_value_proof":"200000000000002710c833ef127398975f30746b9ba1774f982d82dc658926f7d41fd52ff6cc34bc2282b168c578e337f667ad4c567ed4c39744021fc25486df3013ff2aa8b5b88a74"
    }],
    "scalars": [
        "95e27208708a7b57e0ec3b562bbe0dd56548bb9d0359d7f9a8604dfd9ec02eba"
    ]
}
```

The topic of this proposal is automatically inferred by the proposal json, and the server is doing the following validations:

- Input exists and it's unspent
- Liquidex validation rules (see method [validate](https://github.com/Blockstream/lwk/blob/16ec78caf4ba212d38de89446dc519deaba61567/lwk_wollet/src/liquidex.rs#L227) on LiqudexProposal)

#### Subcribe proposal

```json
{
    "jsonrpc": "2.0",
    "method": "subscribe",
    "id": 1,
    "params": {
        "pair": {
            "input": "6921c799f7b53585025ae8205e376bfd2a7c0571f781649fb360acece252a6a7",
            "output": "f13806d2ab6ef8ba56fc4680c1689feb21d7596700af1871aef8c2d15d4bfd28"
        }
    }
}
```

## Testing

You can test the WebSocket server using tools like `websocat`:

### Ping

To test with Liquid asset pairs:

```bash
# Terminal 1: ping
$ echo '{"jsonrpc": "2.0", "method": "ping", "id": 1 }' | websocat ws://127.0.0.1:8080
{"jsonrpc": "2.0", "result": "pong", "id": 1 }
```

## Address

### Subscribe address

unconfidential address:  

```json
{
    "jsonrpc": "2.0",
    "method": "subscribe",
    "id": 1,
    "params": {
        "address": "ert1qxdyjf6h5d6qxap4n2dap97q4j5ps6ua82znu3z"
    }
}
```

### Notification on address seen

```json
{
    "jsonrpc":"2.0",
    "result":
    {
        "address":"ert1qxdyjf6h5d6qxap4n2dap97q4j5ps6ua82znu3z","tx_hex":"0200000001023...","txid":"46b65e2f9f0086bddb6adcda9d796707e423c2c7e7a0575836865001e42505d6","where":"mempool"
    },
    "id":-1
}
```