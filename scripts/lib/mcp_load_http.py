"""Raw HTTP stress probes for the existing stateless MCP endpoint, not an SDK."""
import concurrent.futures
import http.client
import json
import threading
import time

BODY_BYTES = 512 * 1024
ADMISSIONS = 32


def encode(name, arguments):
    return json.dumps({'jsonrpc': '2.0', 'id': 1, 'method': 'tools/call',
                       'params': {'name': name, 'arguments': arguments}}, separators=(',', ':')).encode()


def success(response):
    assert response[0] == 200, response[0]
    rpc = json.loads(response[1])
    assert rpc['jsonrpc'] == '2.0' and rpc['id'] == 1 and 'error' not in rpc
    result = rpc['result']
    assert result.get('isError') is not True, result.get('structuredContent')
    return result


class Endpoint:
    def __init__(self, address, token):
        self.address, self.token = address, token

    def headers(self):
        return {'Authorization': 'Bearer ' + self.token, 'Content-Type': 'application/json',
                'Accept': 'application/json, text/event-stream', 'MCP-Protocol-Version': '2025-11-25',
                'Connection': 'close'}

    def request(self, body, chunked=False):
        connection = http.client.HTTPConnection(self.address, timeout=12)
        try:
            headers = self.headers()
            if chunked:
                headers['Transfer-Encoding'] = 'chunked'
                body = iter([body[:BODY_BYTES // 2], body[BODY_BYTES // 2:]])
            connection.request('POST', '/mcp', body=body, headers=headers, encode_chunked=chunked)
            response = connection.getresponse()
            value = response.read(1024 * 1024 + 1)
            assert len(value) <= 1024 * 1024, 'Unexpected oversized MCP response'
            assert self.token.encode() not in value, 'Bearer leaked in response'
            return response.status, value
        finally:
            connection.close()

    def stalled(self, body, *, admission_deadline):
        """Hold an admitted body, rather than assuming a TCP flush admitted it.

        Hyper sends 100 Continue when the authenticated handler polls the body,
        after its admission permit is held. Consume only that interim header:
        HTTPConnection.getresponse() would skip it and wait for the final 504.
        """
        remaining = admission_deadline - time.monotonic()
        assert remaining > 0, 'MCP admission confirmation exceeded two seconds'
        connection = http.client.HTTPConnection(self.address, timeout=remaining)
        try:
            connection.putrequest('POST', '/mcp')
            for name, value in self.headers().items():
                connection.putheader(name, value)
            connection.putheader('Content-Length', str(len(body)))
            connection.putheader('Expect', '100-continue')
            started = time.monotonic()
            connection.endheaders()
            admitted_body(connection, admission_deadline)
            connection.send(body[:-1])
            return connection, started
        except BaseException:
            connection.close()
            raise


def admitted_body(connection, deadline):
    """Require the bounded interim response before sending a held body."""
    header = bytearray()
    while not header.endswith(b'\r\n\r\n'):
        assert len(header) < 4096, 'Oversized MCP admission response'
        remaining = deadline - time.monotonic()
        assert remaining > 0, 'MCP admission confirmation exceeded two seconds'
        connection.sock.settimeout(remaining)
        byte = connection.sock.recv(1)
        assert byte, 'MCP connection closed before admission confirmation'
        header.extend(byte)
    status_line = bytes(header).split(b'\r\n', 1)[0].split(b' ', 2)
    assert len(status_line) >= 2 and status_line[:2] == [b'HTTP/1.1', b'100'], (
        'Stalled MCP request was not admitted', status_line[1:2])
    connection.timeout = 12
    connection.sock.settimeout(12)


def burst(call, report):
    """Synchronize client starts, without claiming all executions overlap."""
    barrier = threading.Barrier(ADMISSIONS)

    def measured(index):
        barrier.wait(timeout=5)
        started = time.monotonic()
        result = call(index)
        return result, time.monotonic() - started

    with concurrent.futures.ThreadPoolExecutor(max_workers=ADMISSIONS) as executor:
        futures = [executor.submit(measured, index) for index in range(ADMISSIONS)]
        results = [future.result(timeout=15) for future in futures]
    for _, elapsed in results:
        report.latency(elapsed)
    return [result for result, _ in results]
