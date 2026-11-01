import test from "node:test";
import assert from "node:assert/strict";
import { hasLiveMediaBinding } from "./ui/media-session.mjs";

test("current media survives consented Session progress but stops on actual scope, presence or lifetime loss", () => {
    const binding = { conversationId: "conversation-1", sessionId: "session-1", connectionId: "connection-1" };
    const view = { conversation: { id: binding.conversationId, closed: false },
        connections: [{ id: binding.connectionId, sessionId: binding.sessionId, state: "ready" }],
        sessions: [{ id: binding.sessionId, state: "accepted", appearance: "present" }] };
    for (const state of ["accepted", "ready", "active"]) {
        view.sessions[0].state = state;
        assert.equal(hasLiveMediaBinding(view, binding), true, state);
    }
    for (const change of [
        v => { v.sessions[0].state = "ending"; }, v => { v.sessions[0].state = "ended"; },
        v => { v.sessions[0].state = "inviting"; }, v => { v.sessions[0].appearance = "invited"; },
        v => { v.sessions[0].id = "another-session"; }, v => { v.connections[0].id = "another-connection"; },
        v => { v.connections[0].state = "retired"; }, v => { v.conversation.id = "another-conversation"; },
        v => { v.conversation.closed = true; },
    ]) {
        const invalid = structuredClone(view); change(invalid);
        assert.equal(hasLiveMediaBinding(invalid, binding), false);
    }
});
