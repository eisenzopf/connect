// Installed external consumer. Only the exported Node API controls the scene.
import assert from 'node:assert/strict';
import {readFile, realpath} from 'node:fs/promises';
import {createInterface} from 'node:readline';
import {fileURLToPath} from 'node:url';
import {NodeClient, NodeSdkError, CanonicalHttpClient, CanonicalError} from '@rudeless/connect';

const spec = JSON.parse(await readFile(process.argv[2], 'utf8'));
assert.ok((await realpath(fileURLToPath(import.meta.resolve('@rudeless/connect')))).startsWith(spec.project + '/node_modules/'));
const opts = {timeoutMs: 4000};
const emit = value => process.stdout.write(JSON.stringify(value) + '\n');
const clients = [];
const open = () => {
  const client = new NodeClient({config: spec.config, executable: spec.executable});
  clients.push(client);
  return client;
};
const joined = async () => {
  const pids = (await readFile(spec.pid_file, 'utf8')).trim().split('\n').filter(Boolean).map(Number);
  assert.equal(new Set(pids).size, pids.length);
  for (const pid of pids) {
    assert.ok(Number.isSafeInteger(pid) && pid > 0);
    assert.throws(() => process.kill(pid, 0), error => error.code === 'ESRCH');
  }
  return pids.length;
};
const lines = createInterface({input: process.stdin});
let client, operation, saved, record;
let checkpoint = 'submit', observedKind = null, observedCertainty = null;
const http = new CanonicalHttpClient({endpoint: spec.http, credential: spec.credential});
try {
  client = open();
  const d = spec.detail;
  if (spec.mode === 'consult') {
    operation = await client.conversations.byId(d.cid).sessions.byId(d.sid).consult({
      via: d.via, retain: d.retain,
      with: {first: d.participants[0], rest: d.participants.slice(1)},
      hold: d.hold, expectedTopologyRevision: d.revision,
    }, {...opts, operationId: d.operation_id});
    saved = operation.referenceJson();
    const snapshot = await operation.status(opts);
    assert.notEqual(snapshot.state, 'completed');
    assert.ok(snapshot.recordId);
    record = snapshot.recordId;
    assert.equal(operation.reference.scope.conversationId, d.cid);
    assert.equal(operation.reference.scope.sessionId, d.sid);
    assert.equal(operation.reference.scope.connectionId, d.via);
    emit({phase: 'submitted', reference: saved});
  } else {
    assert.equal(spec.mode, 'return');
    const original = client.operations.restore(d.reference, client.operationKinds.session_consult);
    checkpoint = 'return-submit';
    operation = await original.returnToRetained({expectedTopologyRevision: d.revision}, {...opts, operationId: 'return-private-native'});
    saved = operation.referenceJson();
    assert.equal(saved.continuation.retained_connection_id, d.reference.continuation.retained_connection_id);
    assert.equal(saved.continuation.consultation_record_id, d.reference.continuation.record_id);
    checkpoint = 'return-wait';
    const completion = await operation.wait(opts);
    assert.equal(completion.value.id, original.reference.scope.sessionId);
    record = completion.snapshot.recordId;
    assert.ok(record);
    checkpoint = 'return-recover';
    const recovered = client.operations.restore(saved, client.operationKinds.session_consult_return);
    assert.equal((await recovered.wait(opts)).snapshot.recordId, record);
    emit({phase: 'returned', reference: saved, original_record_recovered: true});
  }
  for await (const line of lines) {
    assert.ok(line.length <= 4096);
    const request = JSON.parse(line);
    checkpoint = request.action;
    if (request.action === 'completed') {
      assert.equal(spec.mode, 'consult');
      const completion = await operation.wait(opts);
      assert.equal(completion.snapshot.recordId, record);
      assert.equal(completion.value.topologyRevision, request.revision);
      assert.equal(completion.value.session.id, spec.detail.sid);
      assert.deepEqual(completion.value.retained.map(c => c.id).sort(), [spec.detail.via, spec.detail.retain].sort());
      assert.deepEqual(completion.value.admitted.map(c => c.id), ['private-consultant']);
      assert.deepEqual(completion.value.retired, []);
      // Handles are observations; they grant no native negotiation custody.
      assert.equal(completion.value.admitted[0].nativeNegotiation, undefined);
      await assert.rejects(operation.returnToRetained({expectedTopologyRevision: request.revision}, {...opts, operationId: 'node-return-without-grant'}), error => {
        assert.ok(error instanceof NodeSdkError);
        assert.equal(error.certainty, 'refused');
        assert.equal(error.problem.code, 403);
        assert.equal(error.operation.operationId, 'node-return-without-grant');
        assert.equal(error.operation.command, 'session.consult.return');
        return true;
      });
      const fresh = open();
      const restored = fresh.operations.restore(saved, fresh.operationKinds.session_consult);
      const replay = await restored.wait(opts);
      assert.equal(replay.snapshot.recordId, record);
      assert.equal(replay.value.topologyRevision, request.revision);
      assert.deepEqual(replay.value.admitted.map(c => c.id), ['private-consultant']);
      assert.deepEqual(replay.value.retained.map(c => c.id).sort(), [spec.detail.via, spec.detail.retain].sort());
      await fresh.close(opts);
      emit({phase: 'completed', record_id: record, fresh_observation: true, return_without_grant_refused: true});
    } else if (request.action === 'revoked') {
      await assert.rejects(operation.status(opts), error => {
        observedKind = error instanceof NodeSdkError ? error.kind : null;
        observedCertainty = error instanceof NodeSdkError ? error.certainty : null;
        assert.ok(error instanceof NodeSdkError);
        assert.ok(['refused', 'read-only'].includes(error.certainty) &&
          (['authentication', 'cancelled', 'transport-uncertain'].includes(error.kind) || error.certainty === 'refused' && error.kind === 'application'));
        return true;
      });
      // A current authenticated HTTP read must independently reject the token.
      // A native transport closure alone cannot establish this authorization fact.
      const scope = operation.reference.scope;
      await assert.rejects(http.request('operation.get', {operation_id: operation.reference.operationId},
        {cid: scope.conversationId, sid: scope.sessionId, connid: scope.connectionId}, opts), error => {
        assert.ok(error instanceof CanonicalError);assert.equal(error.kind, 'authentication');return true;
      });
      emit({phase: 'revoked', current_authority_refused: true, http_current_authority_refused: true});
    } else {
      assert.equal(request.action, 'close');
      for (const active of clients) await active.close(opts);
      emit({phase: 'closed', mode: spec.mode, helpers_joined: await joined()});
      break;
    }
  }
} catch (error) {
  const at = /consultation-consume\.mjs:(\d+):(\d+)/.exec(String(error?.stack));
  const known = ['submit', 'return-submit', 'return-wait', 'return-recover', 'completed', 'revoked', 'close'].includes(checkpoint) ? checkpoint : 'invalid';
  emit({phase: 'failed', checkpoint: known,
    kind: error instanceof NodeSdkError ? error.kind : 'consumer-assertion',
    certainty: error instanceof NodeSdkError ? error.certainty : null, observedKind, observedCertainty,
    line: at ? Number(at[1]) : null, column: at ? Number(at[2]) : null});
  process.exitCode = 1;
} finally {
  for (const active of clients) await active.close(opts).catch(() => {process.exitCode = 1;});
  await http.close(opts).catch(() => {process.exitCode = 1;});
  lines.close();
}
