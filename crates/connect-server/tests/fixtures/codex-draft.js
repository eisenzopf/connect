// Exercise actual artifact tools; share the returned handle in the question.
const begin = await invoke('connect_artifact_begin', {
  request_id: 'draft-begin', step_id: 'draft', name: draft.name,
  media_type: 'text/plain', size: draft.size, sha256: draft.sha256,
});
const artifactId = begin.data.outcome.data.id;
await invoke('connect_artifact_append', {
  request_id: 'draft-append', step_id: 'draft', artifact_id: artifactId,
  offset: 0, data: draft.data,
});
const finished = await invoke('connect_artifact_finish', {
  request_id: 'draft-finish', step_id: 'draft', artifact_id: artifactId,
});
const question = await invoke('connect_send', {
  request_id: 'workflow-question', step_id: 'review', wait_for_reply: true,
  body: JSON.stringify({title: draft.name, artifact: finished.data.outcome.data}),
});
const task = await invoke('connect_task_status', {});
const journal = await invoke('connect_task_wait', {limit: 100, wait_ms: 0});
text(JSON.stringify({question, task, journal}));
