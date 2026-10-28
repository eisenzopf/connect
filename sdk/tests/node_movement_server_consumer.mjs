// Installed public Node controller; the independent native peers own all media.
import assert from 'node:assert/strict';
import {readFile, realpath} from 'node:fs/promises';
import {createInterface} from 'node:readline';
import {fileURLToPath} from 'node:url';
import {NodeClient, NodeSdkError} from '@rudeless/connect';
const spec = JSON.parse(await readFile(process.argv[2], 'utf8'));
assert.ok((await realpath(fileURLToPath(import.meta.resolve('@rudeless/connect')))).startsWith(spec.project + '/node_modules/'));
const opts = {timeoutMs: 4000}, clients = [];
const open = () => {const client = new NodeClient({config: spec.config, executable: spec.executable});clients.push(client);return client;};
const emit = value => process.stdout.write(JSON.stringify(value) + '\n');
const lines = createInterface({input: process.stdin});
const topology = value => ({retained: value.retained.map(c => c.id), admitted: value.admitted.map(c => c.id),
  retired: [...value.retired], topology_revision: value.topologyRevision, conversation_revision: value.conversationRevision ?? null});
const summary = value => {
  for (const c of [...value.retained, ...value.admitted]) assert.equal(c.nativeNegotiation, undefined);
  if (spec.mode === 'handoff') {
    const role = value.roleChange;assert.ok(role);
    return {topology: topology(value), role_change: {participant_revision: role.participantRevision,
      role_scope: role.roleScope, outgoing: role.outgoing, source_participant: role.sourceParticipant,
      target_participant: role.targetParticipant, cleanup: role.cleanup}};
  }
  assert.equal(value.connection.nativeNegotiation, undefined);
  const b = value.binding;
  return {topology: topology(value), connection_id: value.connection.id, native_owned: false,
    binding: {generation: b.generation, participant_id: b.participantId, membership_epoch: b.membershipEpoch,
      endpoint_id: b.endpointId, device_id: b.deviceId}};
};
async function stopped(operation, state) {
  assert.equal((await operation.status(opts)).state, state);
  await assert.rejects(operation.wait(opts), error => {
    assert.ok(error instanceof NodeSdkError);assert.equal(error.certainty, 'recorded');
    assert.equal(error.observation.state, state);return true;
  });
}
let client, operation, kind, saved, checkpoint = 'register';
try {
  client = open();
  if (spec.mode === 'move-recovery') {
    checkpoint = 'recover';kind = client.operationKinds.connection_replace;
    operation = client.operations.restore(spec.detail.reference, kind);
    await stopped(operation, spec.detail.state);
    emit({phase: 'stopped', state: spec.detail.state, certainty: 'recorded'});
  } else {
    assert.ok(['handoff', 'move'].includes(spec.mode));
    const command = spec.detail.command;
    await client.conversations.byId(command.conversation_id).refresh(opts);
    emit({phase: 'registered'});
  }
  for await (const line of lines) {
    assert.ok(line.length <= 4096);
    const request = JSON.parse(line);checkpoint = request.action;
    if (request.action === 'begin') {
      const c = spec.detail.command, i = c.intent;
      const session = client.conversations.byId(c.conversation_id).sessions.byId(c.session_id);
      const extensions = {...c.payload_extensions};
      const mutation = {...opts, operationId: c.operation_id, extensions};
      if (spec.mode === 'handoff') {
        assert.equal(i.type, 'session.handoff');
        assert.equal(extensions.consultation_record_id, spec.detail.consultation.continuation.record_id);
        delete extensions.consultation_record_id;
        kind = client.operationKinds.session_handoff;
        const original = client.operations.restore(spec.detail.consultation, client.operationKinds.session_consult);
        operation = await session.handoff({via: c.connection_id, from: i.source_participant, to: i.target_participant,
          retain: i.retained_connid, ...(i.candidate_connid == null ? {} : {candidate: i.candidate_connid}),
          outgoing: i.outgoing, role: i.role_scope === 'session' ? {scope: 'session'} : {scope: 'conversation', expectedConversationRevision: i.expected_conversation_revision},
          expectedTopologyRevision: c.expected_revision, consultation: original}, mutation);
      } else {
        assert.equal(spec.mode, 'move');assert.equal(i.type, 'connection.replace');
        assert.notEqual(i.endpoint_id != null, i.candidate_connid != null);
        kind = client.operationKinds.connection_replace;
        operation = await session.connections.byId(c.connection_id).move({retain: i.retained_connid,
          target: i.endpoint_id == null ? {candidate: i.candidate_connid} : {endpoint: i.endpoint_id},
          expectedTopologyRevision: c.expected_revision}, mutation);
        assert.notEqual((await operation.status(opts)).state, 'completed');
      }
      saved = operation.referenceJson();
      assert.equal(operation.reference.operationId, c.operation_id);
      assert.equal(operation.reference.scope.conversationId, c.conversation_id);
      assert.equal(operation.reference.scope.sessionId, c.session_id);
      assert.equal(operation.reference.scope.connectionId, c.connection_id);
      emit({phase: 'accepted', reference: saved});
    } else if (request.action === 'completed') {
      const completion = await operation.wait(opts), value = summary(completion.value);
      const fresh = open(), restored = fresh.operations.restore(saved, kind);
      const replay = await restored.wait(opts);
      assert.equal(replay.snapshot.recordId, completion.snapshot.recordId);
      assert.deepEqual(summary(replay.value), value);
      await fresh.close(opts);
      emit({phase: 'completed', recovered: true, summary: value});
    } else if (request.action === 'stopped') {
      if (request.original_revoked) {
        await assert.rejects(operation.status(opts), error => {
          assert.ok(error instanceof NodeSdkError);assert.ok(['read-only', 'refused'].includes(error.certainty));
          assert.ok(['cancelled', 'authentication', 'transport-uncertain', 'application'].includes(error.kind));return true;
        });
      } else await stopped(operation, request.state);
      emit({phase: 'stopped', state: request.state, original_revoked: request.original_revoked});
    } else {
      assert.equal(request.action, 'close');
      for (const active of clients) await active.close(opts);
      const pids = (await readFile(spec.pid_file, 'utf8')).trim().split('\n').filter(Boolean).map(Number);
      assert.equal(new Set(pids).size, pids.length);
      for (const pid of pids) {assert.ok(Number.isSafeInteger(pid) && pid > 0);assert.throws(() => process.kill(pid, 0), error => error.code === 'ESRCH');}
      emit({phase: 'closed', helpers_joined: pids.length});break;
    }
  }
 } catch (error) {
  const at = /movement-consume\.mjs:(\d+):(\d+)/.exec(String(error?.stack));
  emit({phase: 'failed', checkpoint: ['register', 'recover', 'begin', 'completed', 'stopped', 'close'].includes(checkpoint) ? checkpoint : 'invalid',
    kind: error instanceof NodeSdkError ? error.kind : 'consumer-assertion', certainty: error instanceof NodeSdkError ? error.certainty : null,
    line: at ? Number(at[1]) : null, column: at ? Number(at[2]) : null});
  process.exitCode = 1;
} finally {
  for (const active of clients) await active.close(opts).catch(() => {process.exitCode = 1;});
  lines.close();
}
