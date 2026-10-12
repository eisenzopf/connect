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

    def stalled(self, body):
        connection = http.client.HTTPConnection(self.address, timeout=12)
        try:
            connection.putrequest('POST', '/mcp')
            for name, value in self.headers().items():
                connection.putheader(name, value)
            connection.putheader('Content-Length', str(len(body)))
            started = time.monotonic()
            connection.endheaders(body[:-1])
            return connection, started
        except BaseException:
            connection.close()
            raise


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
