// Shared admission for real browser matrix owners; no Node or private Store IO.
// The caller retains these actual owners for the full original media programme.
const check = (ok, message) => { if (!ok) throw Error(message); };
const nonempty = values => { check(values.length > 0, "nonempty fixture values"); return {first: values[0], rest: values.slice(1)}; };
const roles = ["participant-1", "participant-2", "participant-3", "participant-4"];
export function browserMatrixPlan(uuid = () => crypto.randomUUID()) {
    const ids = new Set();
    const id = kind => {const value = "matrix_" + kind + "_" + uuid(); check(!ids.has(value), "distinct original fixture IDs"); ids.add(value); return value;};
    const operationNames = ["create", "add", "update", "stale", "remove", "readd", "before", "included", "from4", "after"];
    const operations = Object.fromEntries(operationNames.map(name => [name, id("op")]));
    const messages = Object.fromEntries(["before", "included", "from4", "after"].map(name => [name, id("message")]));
    const sessions = [0, 1].map(index => ({index, id: id("session"), invite: id("op"),
        accepts: roles.slice(1, index === 0 ? 4 : 2).map(role => ({role, id: id("op")})), upgrade: id("op"), unsupported: id("op"), end: id("op")}));
    const routes = [[0, 0], [0, 1], [0, 2], [0, 3], [1, 0], [1, 1]].map(([session, role], index) => ({index, session,
        role: roles[role], connection: id("connection"), audio: id("audio"), video: id("video"),
        offer: id("op"), select: id("op"), end: id("op")}));
    const moves = [0, 4].flatMap(source => [15000, 40000].map((offset, ordinal) => ({source, ordinal, scheduledStartMs: offset,
        operation: id("op"), selection: id("op"), audio: id("audio"), video: id("video"), receiveVideo: id("receive-video"),
        targetRole: ordinal ? "participant-1" : "replacement-1"})));
    return {format: "connect/browser-matrix-plan/1", movementProfile: "gate6-realistic-device-roundtrip-v1",
        programmeMs: 62000, operations, messages, sessions, routes, moves};
}

export async function browserMatrixAdmission({actors, plan, journal, selectedTransport, startMedia, state={}, afterControl}) {
    check(["webtransport", "wss-webrtc"].includes(selectedTransport), "explicit matrix transport");
    check(typeof journal === "function", "durable original intent journal is required");
    check(plan?.format === "connect/browser-matrix-plan/1" && plan.routes.length === 6 && plan.sessions.length === 2
        && plan.moves.length === 4 && plan.programmeMs === 62000, "complete approved browser matrix plan");
    const names = [...roles, "replacement-1", "controller-1", "controller-2"];
    check(Object.keys(actors).sort().join() === names.sort().join(), "seven explicit browser Device owners");
    check(new Set(roles.map(role => actors[role].identityId)).size === 4, "four independent identities");
    check(new Set(names.map(role => actors[role].deviceId)).size === 7, "seven independently enrolled Devices");
    check(names.filter(name => !roles.includes(name)).every(name => actors[name].identityId === actors["participant-1"].identityId),
        "controllers and replacement belong to the original identity");
    await journal({kind: "plan", plan}); // Must settle before any mutation submission.
    const references = new Map();
    const options = {timeoutMs: 8000}, report = state.report = {checks: [], operations: [], fullProgrammeComplete: false};
    const client = role => actors[role].client;
    const mutation = operationId => ({...options, operationId});
    async function complete(name, operationId, submit, onAccepted) {
        await journal({kind: "intent", name, operationId});
        let operation;
        try {
            operation = await submit(mutation(operationId));
            const reference = operation.referenceJson();
            references.set(name, reference);
            await journal({kind: "accepted", name, operationId, reference});
            if (onAccepted) await onAccepted();
            const result = await operation.wait(options);
            check(result.snapshot.state === "completed", "original operation must complete");
            report.operations.push({name, operationId, state: result.snapshot.state});
            await journal({kind: "completed", name, operationId, reference});
            return result.value;
        } catch (error) {
            await journal({kind: "failure", name, operationId, certainty: error.certainty ?? null,
                reference: operation?.referenceJson() ?? null, operation: error.operation ?? null});
            throw error; // Never resubmit a mutation after uncertainty.
        }
    }
    const refused = async (work, code, reason) => {
        let denied = false;
        try {await work();} catch (error) {check(error.problem?.code === code && error.problem?.reason === reason
            && error.certainty !== "uncertain", "current refusal must be explicit"); denied = true;}
        check(denied, "required current refusal was absent");
    };
    for (const name of names) {
        check(client(name).role === "browser", "actual browser SDK owner required");
        const capability = await client(name).capabilities.refresh(options);
        check(capability.selectedTransport === selectedTransport, "actual selected browser transport");
    }
    report.checks.push("seven-current-browser-owners-on-selected-transport");
    const owner = client("participant-1"), late = client("participant-4");
    const conversation = await complete("create", plan.operations.create, opts => owner.conversations.create({policy: "persistent",
        participants: roles.slice(1, 3).map(role => actors[role].identityId)}, opts));
    const cid = conversation.id;
    state.conversation=conversation;state.createReference=references.get("create");
    const recovered = await owner.operations.restore(references.get("create"), owner.operationKinds.conversationCreate).wait(options);
    check(recovered.snapshot.state === "completed" && recovered.value.id === cid, "recover original create without another mutation");
    report.checks.push("original-browser-operation-reference-recovers-same-conversation");
    const member = (view, role) => {
        const found = view.participants.filter(p => p.identityId === actors[role].identityId && p.active);
        check(found.length === 1, "one current participant per identity"); return found[0];
    };
    const recipients = view => view.participants.filter(p => p.active && p.identityId !== actors["participant-1"].identityId).map(p => p.id);
    const send = async (role, name, targets) => complete(name, plan.operations[name], opts => client(role).conversations.byId(cid)
        .send({messageId: plan.messages[name], to: nonempty(targets), text: "Synthetic matrix " + name}, opts));
    const history = async role => {
        const started = performance.now(), seen = new Set(), messages = new Set(); let cursor;
        for (let page = 0; page < 8; page++) {
            const left = 8000 - (performance.now() - started); check(left > 0, "original history deadline");
            const api = client(role).raw;
            const frames = await api.request(api.validate({v: 2, id: "history_" + crypto.randomUUID(), ts: new Date().toISOString(),
                type: "message.history", cid, payload: {limit: 50, ...(cursor ? {cursor} : {})}}), {timeoutMs: left});
            const ack = frames.filter(f => f.value.type === "ack"); check(ack.length === 1, "one history result");
            for (const frame of frames.filter(f => f.value.type === "message.send")) {
                const id = frame.value.payload.msg_id; check(typeof id === "string" && !messages.has(id), "distinct addressed messages"); messages.add(id);
            }
            cursor = ack[0].value.payload.next_cursor ?? null;
            if (cursor === null) return messages;
            check(typeof cursor === "string" && cursor.length && !seen.has(cursor), "bounded current history continuation"); seen.add(cursor);
        }
        throw Error("history did not terminate within the original page bound");
    };
    let view = await conversation.refresh(options);
    check(view.participants.filter(p => p.active).length === 3, "original membership");
    await send("participant-1", "before", recipients(view));
    await refused(() => late.conversations.byId(cid).refresh(options), 403, "forbidden");
    await complete("add", plan.operations.add, opts => conversation.participants.add({identityId: actors["participant-4"].identityId,
        kind: "human", role: "participant", expectedRevision: view.revision}, opts));
    view = await conversation.refresh(options);
    check(view.participants.filter(p => p.active).length === 4, "four current members");
    check(!(await history("participant-4")).has(plan.messages.before), "late member must not inherit private history");
    await send("participant-1", "included", recipients(view));
    check((await history("participant-4")).has(plan.messages.included), "current addressed member sees included message");
    const fourth = member(view, "participant-4"), oldRevision = view.revision;
    await complete("update", plan.operations.update, opts => conversation.participants.byId(fourth.id)
        .update({role: "reviewer", expectedConversationRevision: oldRevision}, opts));
    await refused(() => conversation.participants.byId(fourth.id).update({role: "participant", expectedConversationRevision: oldRevision},
        mutation(plan.operations.update)), 409, "operation_conflict");
    await refused(() => conversation.participants.byId(fourth.id).update({role: "participant", expectedConversationRevision: oldRevision},
        mutation(plan.operations.stale)), 409, "operation_conflict");
    report.conflict={passed:true,changed_original_intent_refused:true,stale_revision_refused:true};
    view = await conversation.refresh(options);
    await complete("remove", plan.operations.remove, opts => conversation.participants.byId(fourth.id)
        .remove({expectedConversationRevision: view.revision}, opts));
    await refused(() => late.conversations.byId(cid).refresh(options), 403, "forbidden");
    view = await conversation.refresh(options);
    await send("participant-1", "after", recipients(view));
    await complete("readd", plan.operations.readd, opts => conversation.participants.add({identityId: actors["participant-4"].identityId,
        kind: "human", role: "participant", expectedRevision: view.revision}, opts));
    view = await conversation.refresh(options);
    const fourthHistory = await history("participant-4");
    check(!fourthHistory.has(plan.messages.after), "rejoin must not inherit messages sent while removed");
    report.checks.push("participant-add-update-remove-rejoin-and-current-addressed-history");
    const participants = roles.map(role => member(view, role).id);
    const capabilities = {audio: ["opus"], video: ["vp8"]};
    const sessions = [];
    state.sessions=sessions;state.connections=[];state.participants=participants;
    for (const session of plan.sessions) {
        const invited = await complete("invite-" + session.index, session.invite, opts => conversation.sessions.invite({sessionId: session.id,
            to: participants.slice(1, session.index === 0 ? 4 : 2), medium: "voice", intent: "call", capabilities}, opts));
        for (const accept of session.accepts) {
            const current = client(accept.role).conversations.byId(cid).sessions.byId(session.id);
            const snapshot = await current.refresh(options);
            await complete("accept-" + session.index + "-" + accept.role, accept.id, opts => current.accept({capabilities,
                expectedTopologyRevision: snapshot.topologyRevision}, opts));
        }
        const snapshot = await invited.refresh(options);
        await complete("upgrade-" + session.index, session.upgrade, opts => invited.update({action: "medium-upgrade",
            media: nonempty(["audio", "video"])}, snapshot.topologyRevision, opts));
        const upgraded = await invited.refresh(options);
        await refused(() => complete("unsupported-" + session.index, session.unsupported,
            opts => invited.update({action: "medium-upgrade", media: nonempty(["screen"])}, upgraded.topologyRevision, opts)),
            501, "unsupported_feature");
        check(JSON.stringify(await invited.refresh(options)) === JSON.stringify(upgraded), "unsupported medium must preserve the current Session");
        sessions.push(invited);
    }
    report.checks.push("two-concurrent-accepted-sessions-upgraded-to-audio-video-and-unoffered-screen-refused");
    if(afterControl)await afterControl(state);
    const connections = state.connections;
    check(typeof startMedia === "function", "real owned media must start before awaiting offer readiness");
    for (const route of plan.routes) {
        const session = client(route.role).conversations.byId(cid).sessions.byId(plan.sessions[route.session].id);
        const connection = await complete("offer-" + route.index, route.offer, opts => session.connections.offer({connectionId: route.connection,
            streams: nonempty([{id: route.audio, kind: "audio", direction: "sendrecv", codecs: nonempty(["opus"])},
                {id: route.video, kind: "video", direction: "sendrecv", codecs: nonempty(["vp8"])}])}, opts),
            () => startMedia({route, connection: session.connections.byId(route.connection), options}));
        connections.push(connection);
    }
    for (const route of plan.routes) {
        const peers = plan.routes.filter(other => other.session === route.session), at = peers.findIndex(other => other.index === route.index);
        const source = peers[(at + peers.length - 1) % peers.length], connection = connections[route.index];
        const snapshot = await connection.refresh(options);
        check(Number.isSafeInteger(snapshot.streamRevision), "fresh stream revision for original selection");
        await complete("select-" + route.index, route.select, opts => connection.streams.subscribe({expectedStreamRevision: snapshot.streamRevision,
            selectors: nonempty([{stream: source.audio}, {stream: source.video}])}, opts));
    }
    report.checks.push("six-real-browser-connections-with-current-session-scoped-duplex-selections");
    return state;
}


// The admission-only caller requests one terminal intent here. Future full media
// callers retain these same owners until the unchanged programme terminal.
export async function retireBrowserMatrixAdmission({state,actors,plan,journal,now=()=>performance.now(),terminalMs,reason='matrix-admission-complete'}) {
    const terminal=terminalMs??now(),cutoff=terminal+5000;
    const left=()=>{const timeoutMs=Math.floor(cutoff-now());check(timeoutMs>0,"original browser teardown deadline");return {timeoutMs};};
    const ends=await Promise.allSettled(state.connections.map(async(connection,index)=>{
        const original=plan.routes[index].end;
        const snapshot=await connection.refresh(left());
        await journal({kind:"end-intent",operationId:original,connectionId:connection.id,expectedStreamRevision:snapshot.streamRevision});
        const operation=await connection.end({reasonCode:0,reason,expectedStreamRevision:snapshot.streamRevision},
            {...left(),operationId:original});
        await journal({kind:"end-accepted",operationId:original,reference:operation.referenceJson()});
        const result=await operation.wait(left());check(result.snapshot.state==="completed","original browser end must complete");
    }));
    const sessionEnds=await Promise.allSettled(state.sessions.map(async(session,index)=>{
        const original=plan.sessions[index].end;
        const snapshot=await session.refresh(left());
        await journal({kind:"session-end-intent",operationId:original,sessionId:session.id,
            expectedTopologyRevision:snapshot.topologyRevision});
        const operation=await session.end({reasonCode:0,reason,expectedTopologyRevision:snapshot.topologyRevision},
            {...left(),operationId:original});
        await journal({kind:"session-end-accepted",operationId:original,reference:operation.referenceJson()});
        const result=await operation.wait(left());check(result.snapshot.state==="completed","original Session end must complete");
    }));
    const closes=await Promise.allSettled(Object.values(actors).map(async actor=>{
        try {await actor.client.close({timeoutMs:Math.max(1,Math.floor(cutoff-now()))});return {deadlineMet:true};}
        catch(error){
            // A missed acceptance deadline remains failed. Retain and join the
            // original cleanup; this is another wait, never another mutation.
            await actor.client.close({timeoutMs:60000});return {deadlineMet:false};
        }
    }));
    const elapsedMs=now()-terminal;
    const passed=elapsedMs<=5000&&ends.every(value=>value.status==="fulfilled")&&sessionEnds.every(value=>value.status==="fulfilled")
        &&closes.every(value=>value.status==="fulfilled"&&value.value.deadlineMet);
    return {passed,elapsedMs,originalEndsCompleted:ends.filter(value=>value.status==="fulfilled").length,
        originalSessionsEnded:sessionEnds.filter(value=>value.status==="fulfilled").length,
        originalClientsClosed:closes.filter(value=>value.status==="fulfilled").length,
        scope:"Admission component terminal only; no ten-second idle resource restoration or full media programme claim"};
}
