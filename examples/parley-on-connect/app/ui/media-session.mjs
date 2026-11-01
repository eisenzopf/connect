// Session lifecycle only. Current membership, appearance, physical generation
// and packet authorization are still checked by the installed SDK/backend.
export function sessionAllowsMedia(state) { return ["accepted", "ready", "active"].includes(state); }

export function hasLiveMediaBinding(view, binding) {
    return view?.conversation?.id === binding?.conversationId && view.conversation.closed === false &&
        Array.isArray(view.connections) && view.connections.some(c => c.id === binding.connectionId &&
            c.sessionId === binding.sessionId && ["ready", "active"].includes(c.state)) &&
        Array.isArray(view.sessions) && view.sessions.some(s => s.id === binding.sessionId &&
            sessionAllowsMedia(s.state) && s.appearance === "present");
}
