"""Owned loopback CONNECT rejector. No TLS, DNS lookup, or upstream connection."""
import json
import os
from pathlib import Path
import selectors
import signal
import socket
import sys
import tempfile
import time


def publish(path, value):
    with tempfile.NamedTemporaryFile(mode='w', dir=path.parent, delete=False) as stream:
        json.dump(value, stream)
        temporary = stream.name
    os.replace(temporary, path)


def main():
    if len(sys.argv) != 2:
        raise ValueError('Expected one owned output directory')
    root = Path(sys.argv[1])
    if root.resolve() != root or not root.is_dir():
        raise ValueError('Expected canonical owned output directory')
    ready, state_path = root/'ready.json', root/'state.json'
    if ready.exists() or state_path.exists():
        raise ValueError('Proxy proof already exists')
    listener, dead = socket.socket(), socket.socket()
    listener.bind(('127.0.0.1', 0)); listener.listen(8)
    dead.bind(('127.0.0.1', 0))  # Reserved but deliberately not listening.
    start = time.monotonic()
    state = dict(pid=os.getpid(), probe=0, telemetry=0, github=0, unexpected=0,
                 afterRejectBytes=0, incomplete=0, observations=[], stopped=False)
    stopping = False
    def stop(_signal, _frame):
        nonlocal stopping
        stopping = True
    signal.signal(signal.SIGTERM, stop)
    selector = selectors.DefaultSelector()
    selector.register(listener, selectors.EVENT_READ, 'client')
    selector.register(sys.stdin, selectors.EVENT_READ, 'owner')
    publish(state_path, state)
    publish(ready, dict(pid=os.getpid(), host='127.0.0.1', port=listener.getsockname()[1], deadPort=dead.getsockname()[1]))
    try:
        while not stopping and time.monotonic()-start < 30*60:
            for key, _events in selector.select(0.2):
                if key.data == 'owner':
                    if not os.read(sys.stdin.fileno(), 1):
                        stopping = True
                    continue
                client, _address = listener.accept()
                with client:
                    client.settimeout(2)
                    data = b''
                    kind = 'unexpected'
                    try:
                        while b'\r\n\r\n' not in data and len(data) < 8192:
                            chunk = client.recv(min(4096, 8192-len(data)))
                            if not chunk:
                                break
                            data += chunk
                        head, separator, extra = data.partition(b'\r\n\r\n')
                        first = head.split(b'\r\n', 1)[0]
                        allowed = {b'CONNECT capture.invalid:443 HTTP/1.1': 'probe',
                                   b'CONNECT us.i.posthog.com:443 HTTP/1.1': 'telemetry',
                                   b'CONNECT github.com:443 HTTP/1.1': 'github'}
                        if separator:
                            kind = allowed.get(first, 'unexpected')
                        else:
                            state['incomplete'] += 1
                        state[kind] += 1
                        state['afterRejectBytes'] += len(extra)
                        client.sendall(b'HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n')
                        client.shutdown(socket.SHUT_WR)
                        # A rejected CONNECT must not proceed to TLS or a request body.
                        tail = client.recv(1)
                        state['afterRejectBytes'] += len(tail)
                    except (OSError, TimeoutError):
                        state['incomplete'] += 1
                    if len(state['observations']) < 32:
                        state['observations'].append(dict(kind=kind, elapsedMs=int((time.monotonic()-start)*1000)))
                    publish(state_path, state)
    finally:
        state['stopped'] = True
        publish(state_path, state)
        selector.close(); listener.close(); dead.close()


if __name__ == '__main__':
    main()
