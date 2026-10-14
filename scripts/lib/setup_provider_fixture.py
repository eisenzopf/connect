"""Public setup CLI catalog/preview/readiness proof with owned loopback fixtures.

No live account, provisioning, purchase, model, device or Keychain calls occur.
Only Vapi discovery uses HTTP; its origin is selected explicitly as loopback.
Telnyx resource metadata is supplied locally and never discovered from a provider.
"""
import hashlib
import http.server
import json
import threading
import time

CHECKS = (
    'selected-vapi-loopback-catalog', 'exact-selected-reuse',
    'unknown-capability-remains-gap', 'partial-catalog-cannot-approve-create',
    'supplied-telnyx-metadata-consent-and-rtp-gaps', 'number-purchase-refused',
    'duplicate-metadata-refused', 'rejected-authentication-refused',
    'missing-selected-telnyx-ref-denied-before-http',
    'rejected-loopback-api-redacts-provider-body',
    'directional-callback-stale-auth-readiness', 'ambiguous-readiness-refused',
)
CANARIES = ('fixture-only', 'ambient-vapi-must-not-be-used',
    'ambient-telnyx-must-not-be-used', 'raw-provider-key-must-not-export',
    'raw-password-must-not-export')


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def private_json(path, value):
    with path.open('x') as stream:
        stream.write(json.dumps(value, indent=2) + '\n')
    path.chmod(0o600)


class CatalogServer(http.server.ThreadingHTTPServer):
    daemon_threads = True


class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_GET(self):
        self.server.requests.append({'method': 'GET', 'path': self.path})
        if self.path != '/assistant' or self.headers.get('Authorization') != 'Bearer fixture-only':
            self.send_error(404)
            return
        value = [{'id': 'assistant-1', 'name': 'fixture-assistant', 'model': {}, 'voice': {},
            'api_key': CANARIES[3], 'configuration': {'password': CANARIES[4]}}]
        if self.server.reject:
            value = {'error': CANARIES[3]}
        body = json.dumps(value).encode()
        self.send_response(401 if self.server.reject else 200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        self.server.requests.append({'method': self.command, 'path': self.path})
        self.send_error(405)

    do_DELETE = do_POST
    do_PATCH = do_POST
    do_PUT = do_POST


def provider_setup(cli, work):
    """Exercise the selected executable and return only redacted observations."""
    work.mkdir(mode=0o700)
    refs = work / 'references.toml'
    refs.write_text("version=1\n[references.VAPI_PRIVATE_KEY]\nstorage='environment'\nvariable='CONNECT_FIXTURE_SELECTED_KEY'\n")
    refs.chmod(0o600)
    server = CatalogServer(('127.0.0.1', 0), Handler)
    server.requests = []
    server.reject = False
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    env = dict(cli.env, CONNECT_SECRET_REFERENCES=str(refs),
        CONNECT_FIXTURE_SELECTED_KEY=CANARIES[0], VAPI_PRIVATE_KEY=CANARIES[1],
        TELNYX_API_KEY=CANARIES[2],
        CONNECT_VAPI_API_BASE_URL='http://127.0.0.1:' + str(server.server_port))
    checks = []

    def call(name, arguments, accepted=True):
        result = cli.run(*arguments, accepted=accepted, env=env)
        assert not any(value in result.stdout + result.stderr for value in CANARIES)
        checks.append({'name': name, 'status': 'passed'})
        return result

    def preview(name, catalog, intent, accepted=True):
        catalog_path = work / (name + '-catalog.json')
        intent_path = work / (name + '-intent.json')
        private_json(catalog_path, catalog)
        private_json(intent_path, intent)
        before = {path: digest(path) for path in (catalog_path, intent_path, refs)}
        calls = len(server.requests)
        result = call(name, ['setup', 'provider-preview', '--catalog', catalog_path,
            '--intent', intent_path], accepted)
        assert all(digest(path) == checksum for path, checksum in before.items())
        assert len(server.requests) == calls
        return json.loads(result.stdout) if accepted else None

    try:
        destination = work / 'vapi-only.json'
        call(CHECKS[0], ['setup', 'provider-discover', '--provider', 'vapi', '--output', destination])
        catalog = json.loads(destination.read_text())
        assert catalog['authenticated'] and not catalog['complete']
        assert catalog['resources'] == [{'kind': 'assistant', 'id': 'assistant-1',
            'name': 'fixture-assistant', 'capabilities': ['chat', 'voice']}]
        assert not any(value in destination.read_text() for value in CANARIES)
        assert destination.stat().st_mode & 0o777 == 0o600
        reuse = {'profile': 'connect-provider-setup/1', 'selections': [
            {'action': 'reuse', 'kind': 'assistant', 'id': 'assistant-1', 'required_capabilities': ['chat']}]}
        result = preview(CHECKS[1], catalog, reuse)
        assert result['gaps'] == [] and not result['purchases'] and not result['public_ingress_created']
        missing = {'profile': reuse['profile'], 'selections': [
            dict(reuse['selections'][0], required_capabilities=['video'])]}
        assert 'video' in ' '.join(preview(CHECKS[2], catalog, missing)['gaps'])
        create = {'profile': reuse['profile'], 'selections': [
            {'action': 'create', 'kind': 'assistant', 'name': 'new-assistant'}]}
        preview(CHECKS[3], catalog, create, False)
        complete = {'authenticated': True, 'complete': True, 'resources': [
            {'kind': 'number', 'id': 'number-1', 'name': 'fixture', 'capabilities': ['voice']},
            {'kind': 'trunk', 'id': 'trunk-1', 'name': 'fixture', 'capabilities': ['sip']}]}
        telnyx = {'profile': reuse['profile'], 'selections': [
            {'action': 'reuse', 'kind': 'number', 'id': 'number-1', 'required_capabilities': ['sms']},
            {'action': 'reuse', 'kind': 'trunk', 'id': 'trunk-1', 'required_capabilities': ['sip']}]}
        gaps = preview(CHECKS[4], complete, telnyx)
        assert not gaps['purchases'] and not gaps['public_ingress_created']
        assert all(value in ' '.join(gaps['gaps']) for value in ('enrollment', 'signed-callback', 'RTP', 'sms'))
        preview(CHECKS[5], complete, {'profile': reuse['profile'], 'selections': [
            {'action': 'create', 'kind': 'number', 'name': 'new-number'}]}, False)
        preview(CHECKS[6], dict(catalog, resources=catalog['resources'] * 2), reuse, False)
        preview(CHECKS[7], dict(catalog, authenticated=False), reuse, False)
        absent = work / 'absent-telnyx.json'
        before = len(server.requests)
        call(CHECKS[8], ['setup', 'provider-discover', '--provider', 'telnyx', '--output', absent], False)
        assert not absent.exists() and len(server.requests) == before
        server.reject = True
        rejected = work / 'rejected-vapi.json'
        call(CHECKS[9], ['setup', 'provider-discover', '--provider', 'vapi', '--output', rejected], False)
        assert not rejected.exists()
        server.reject = False
        now = int(time.time())
        observations = [
            {'prerequisite': 'callback', 'direction': 'inbound', 'state': 'ready', 'observed_at': now - 10, 'valid_until': now + 3600},
            {'prerequisite': 'authentication', 'direction': 'outbound', 'state': 'rejected', 'observed_at': now - 10, 'valid_until': now + 3600},
            {'prerequisite': 'rtp', 'direction': 'outbound', 'state': 'ready', 'observed_at': now - 10, 'valid_until': now - 1}]
        obs = work / 'observations.json'
        private_json(obs, observations)
        before, checksum = len(server.requests), digest(obs)
        actions = json.loads(call(CHECKS[10], ['setup', 'readiness', '--observations', obs]).stdout)
        assert len(actions) == 14 and len(server.requests) == before and digest(obs) == checksum
        states = {(row['prerequisite'], row['direction']): row for row in actions}
        assert states['callback', 'inbound']['state'] == 'ready'
        assert states['callback', 'outbound']['state'] == 'unknown'
        assert states['authentication', 'outbound']['state'] == 'rejected'
        assert states['rtp', 'outbound']['state'] == 'stale'
        assert all(states[kind, direction]['state'] == 'unknown'
            for kind in ('sip_registration', 'media', 'sms_eligibility') for direction in ('inbound', 'outbound'))
        assert 'not delivery proof' in states['callback', 'inbound']['next_action']
        assert 'not SMS consent' in states['sms_eligibility', 'outbound']['next_action']
        ambiguous = work / 'ambiguous-observations.json'
        private_json(ambiguous, observations + [observations[0]])
        call(CHECKS[11], ['setup', 'readiness', '--observations', ambiguous], False)
        assert server.requests == [{'method': 'GET', 'path': '/assistant'}] * 2
        return {'status': 'passed', 'checks': checks, 'provider_http_methods': ['GET'],
            'provider_http_count': 2, 'resource_mutations': 0, 'real_account_requests': 0,
            'preview_and_readiness_inputs_unchanged': True,
            'scope': 'public CLI with read-only Vapi loopback and supplied Telnyx metadata; no live account or provisioning effects'}
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=2)
