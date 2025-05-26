#!/usr/bin/env python3
import zmq
import time
import sys

context = zmq.Context()
socket = context.socket(zmq.PUB)
socket.bind("tcp://127.0.0.1:29000")

# ZMQ needs a moment to establish connections
time.sleep(1)

tx_hex = "0200000001010000000000000000000000000000000000000000000000000000000000000000ffffffff0401650101ffffffff020125b251070e29ca19043cf33ccd7324e2ddab03ecc4ae0b5e77c4fc0e5cf6c95a01000000000000000000016a0125b251070e29ca19043cf33ccd7324e2ddab03ecc4ae0b5e77c4fc0e5cf6c95a01000000000000000000266a24aa21a9ed94f15ed3a62165e4a0b99699cc28b48e19cb5bc1b1f47155db62d63f1e047d45000000000000012000000000000000000000000000000000000000000000000000000000000000000000000000"

if len(sys.argv) > 2:
    topic = sys.argv[1]
    message = sys.argv[2]
else:
    topic = b"rawtx"
    message = bytes.fromhex(tx_hex)  # Convert hex string to binary bytes

print(f"Sending to topic: {topic}")
socket.send_multipart([topic, message])
print(f"Sent message: {message}")

# Give it time to deliver before closing
time.sleep(1) 