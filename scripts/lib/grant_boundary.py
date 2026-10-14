"""Classify observed expiry races without extending authority or request budgets."""
import json
from .mcp_load_http import success


def status_at_boundary(response, started, finished, expiry, task_id):
    """A request admitted before expiry may be denied at the execution recheck.

    Only a measured interval crossing the exact expiry can explain an HTTP200
    forbidden tool outcome. Early denial, post-expiry fresh success, malformed
    results and unrelated errors remain failures. This never changes server policy.
    """
    assert started <= finished, 'Expiry observation wall clock moved backwards'
    code, raw = response
    if code == 401:
        assert finished >= expiry, 'Current grant denied before expiry'
        return 'http-unauthorized'
    assert code == 200, 'Unexpected expiry probe HTTP outcome'
    rpc = json.loads(raw)
    assert rpc.get('jsonrpc') == '2.0' and rpc.get('id') == 1 and 'error' not in rpc, 'Invalid expiry probe correlation'
    result = rpc.get('result')
    assert isinstance(result, dict), 'Missing expiry probe tool outcome'
    if result.get('isError') is True:
        error = result.get('structuredContent')
        assert isinstance(error, dict) and error.get('code') == 'forbidden', 'Unrelated expiry probe tool error'
        assert started < expiry <= finished, 'Forbidden probe did not cross grant expiry'
        return 'execution-expired'
    assert started < expiry, 'Fresh post-expiry request admitted'
    data = success(response)['structuredContent']['data']
    assert data['id'] == task_id, 'Expiry probe task mismatch'
    return 'status-completed'
