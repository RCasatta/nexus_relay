#!/usr/bin/env python3
import zmq
import time
import sys

context = zmq.Context()
socket = context.socket(zmq.PUB)
socket.bind("tcp://127.0.0.1:29000")

# ZMQ needs a moment to establish connections
time.sleep(1)

if len(sys.argv) > 2:
    topic = sys.argv[1]
    message = sys.argv[2]
else:
    topic = b"rawtx"
    message = b"01020304050607080910" # Sample hex data

print(f"Sending to topic: {topic}")
socket.send_multipart([topic, message])
print(f"Sent message: {message}")

# Give it time to deliver before closing
time.sleep(1) 