import socket
import base64
import os
import json
import urllib.request
import sys
import struct

def cdp_eval(port, expr):
    # 1. Get websocket URL
    targets = json.loads(urllib.request.urlopen(f"http://127.0.0.1:{port}/json").read())
    ws_url = targets[0]["webSocketDebuggerUrl"]
    # ws://127.0.0.1:port/devtools/page/...
    path = ws_url.split(f":{port}")[1]

    # 2. Connect raw socket
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    s.connect(("127.0.0.1", int(port)))

    # 3. Perform WS Handshake
    key = base64.b64encode(os.urandom(16)).decode('utf-8')
    handshake = (
        f"GET {path} HTTP/1.1\r\n"
        f"Host: 127.0.0.1:{port}\r\n"
        f"Upgrade: websocket\r\n"
        f"Connection: Upgrade\r\n"
        f"Sec-WebSocket-Key: {key}\r\n"
        f"Sec-WebSocket-Version: 13\r\n\r\n"
    )
    s.sendall(handshake.encode())

    # Read handshake response
    resp = b""
    while b"\r\n\r\n" not in resp:
        resp += s.recv(1024)

    # 4. Send CDP message
    msg = json.dumps({
        "id": 1,
        "method": "Runtime.evaluate",
        "params": {
            "expression": expr,
            "awaitPromise": True,
            "returnByValue": True
        }
    }).encode()

    # Frame header: FIN (0x80) | opcode text (0x01) = 0x81
    # Mask bit (0x80)
    length = len(msg)
    mask = os.urandom(4)
    if length < 126:
        header = bytes([0x81, 0x80 | length])
    elif length < 65536:
        header = bytes([0x81, 0x80 | 126]) + struct.pack("!H", length)
    else:
        header = bytes([0x81, 0x80 | 127]) + struct.pack("!Q", length)

    masked_msg = bytes(b ^ mask[i % 4] for i, b in enumerate(msg))
    s.sendall(header + mask + masked_msg)

    # 5. Read response frame
    data = b""
    while True:
        chunk = s.recv(4096)
        if not chunk:
            break
        data += chunk
        # Simple frame parsing: find payload
        if len(data) >= 2:
            payload_len = data[1] & 0x7f
            offset = 2
            if payload_len == 126:
                if len(data) < 4: continue
                payload_len = struct.unpack("!H", data[2:4])[0]
                offset = 4
            elif payload_len == 127:
                if len(data) < 10: continue
                payload_len = struct.unpack("!Q", data[2:10])[0]
                offset = 10
            if len(data) >= offset + payload_len:
                payload = data[offset:offset+payload_len]
                s.close()
                res = json.loads(payload.decode('utf-8', errors='replace'))
                if "result" in res and "result" in res["result"]:
                    return res["result"]["result"].get("value", res["result"]["result"])
                return res

if __name__ == "__main__":
    port = sys.argv[1]
    expr = sys.argv[2]
    result = cdp_eval(port, expr)
    print(json.dumps(result, indent=2) if isinstance(result, (dict, list)) else result)
