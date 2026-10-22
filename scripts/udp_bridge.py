import socket
import select
import sys

EXT_IP = "172.23.20.217"
EXT_PORT = 54521
QEMU_IP = "127.0.0.1"
QEMU_PORT = 54521

ext_sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
ext_sock.bind((EXT_IP, EXT_PORT))

local_sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)

print(f"UDP Bridge running: {EXT_IP}:{EXT_PORT} <-> {QEMU_IP}:{QEMU_PORT}", flush=True)

client_map = {}

try:
    while True:
        r, _, _ = select.select([ext_sock, local_sock], [], [], 1.0)
        for s in r:
            if s is ext_sock:
                data, addr = ext_sock.recvfrom(65535)
                client_map["client"] = addr
                print(f"[EXT->QEMU] {len(data)} bytes from {addr}", flush=True)
                local_sock.sendto(data, (QEMU_IP, QEMU_PORT))
            elif s is local_sock:
                data, _ = local_sock.recvfrom(65535)
                target = client_map.get("client")
                if target:
                    print(f"[QEMU->EXT] {len(data)} bytes to {target}", flush=True)
                    ext_sock.sendto(data, target)
except KeyboardInterrupt:
    pass
