"""Public operations shared by native and HTTP clients."""
from abc import ABC, abstractmethod


class Operations(ABC):
    @abstractmethod
    def request(self, operation):
        """Return a validated outcome; mutations always use caller-owned IDs."""
        raise NotImplementedError

    def capabilities(self, address, conversation_id=None):
        """Return authorized configured routes and limits; not a reachability guarantee."""
        return self.request({"op": "capabilities", "address": address,
                             "conversation_id": conversation_id})["data"]

    def browser_binding(self):
        """Return the current credential's configured browser conversation."""
        return self.request({"op": "browser_binding"})["data"]

    def context_deliveries(self, conversation_id, before=None, limit=100):
        """Page newest-first delivery receipts with an opaque older-page cursor."""
        return self.request({"op": "context_deliveries", "conversation_id": conversation_id,
                             "before": before, "limit": limit})["data"]

    def resolve(self, address):
        return self.request({"op": "resolve", "address": address})["data"]

    def create(self, request_id, participants):
        return self.request({"op": "create", "request_id": request_id,
                             "participants": participants})["data"]

    def send(self, request_id, conversation_id, body):
        return self.request({"op": "send", "request_id": request_id,
                             "conversation_id": conversation_id, "body": body})["data"]

    def history(self, conversation_id, after=None, limit=100):
        return self.request({"op": "history", "conversation_id": conversation_id,
                             "after": after, "limit": limit})["data"]

    def inbox(self, cursor=None, limit=100, wait_ms=0):
        return self.request({"op": "inbox", "cursor": cursor,
                             "limit": limit, "wait_ms": wait_ms})["data"]

    def request_status(self, request_id):
        return self.request({"op": "request_status", "request_id": request_id})["data"]

    def provider_status(self, conversation_id):
        return self.request({"op": "provider_status", "conversation_id": conversation_id})["data"]

    def close_conversation(self, request_id, conversation_id):
        return self.request({"op": "close", "request_id": request_id,
                             "conversation_id": conversation_id})["data"]

    def call_start(self, request_id, conversation_id, route_id, destination):
        return self.request({"op": "call_start", "request_id": request_id,
                             "conversation_id": conversation_id, "route_id": route_id,
                             "destination": destination})["data"]

    def call_status(self, conversation_id, call_id):
        return self.request({"op": "call_status", "conversation_id": conversation_id,
                             "call_id": call_id})["data"]

    def call_end(self, request_id, conversation_id, call_id):
        return self.request({"op": "call_end", "request_id": request_id,
                             "conversation_id": conversation_id, "call_id": call_id})["data"]

    def session_invite(self, request_id, conversation_id, participant_id, media):
        return self.request({"op": "session_invite", "request_id": request_id,
                             "conversation_id": conversation_id,
                             "participant_id": participant_id, "media": media})["data"]

    def session_accept(self, request_id, conversation_id, session_id):
        return self.request({"op": "session_accept", "request_id": request_id,
                             "conversation_id": conversation_id, "session_id": session_id})["data"]

    def session_reject(self, request_id, conversation_id, session_id, reason=None):
        return self.request({"op": "session_reject", "request_id": request_id,
                             "conversation_id": conversation_id, "session_id": session_id,
                             "reason": reason})["data"]

    def session_cancel(self, request_id, conversation_id, session_id):
        return self.request({"op": "session_cancel", "request_id": request_id,
                             "conversation_id": conversation_id, "session_id": session_id})["data"]

    def session_end(self, request_id, conversation_id, session_id, reason=None):
        return self.request({"op": "session_end", "request_id": request_id,
                             "conversation_id": conversation_id, "session_id": session_id,
                             "reason": reason})["data"]

    def session_get(self, conversation_id, session_id):
        return self.request({"op": "session_get", "conversation_id": conversation_id,
                             "session_id": session_id})["data"]

    def sip_dial(self, request_id, conversation_id, session_id, destination):
        return self.request({"op": "sip_dial", "request_id": request_id,
                             "conversation_id": conversation_id, "session_id": session_id,
                             "destination": destination})["data"]

    def sip_status(self, conversation_id, sip_leg_id):
        return self.request({"op": "sip_status", "conversation_id": conversation_id,
                             "sip_leg_id": sip_leg_id})["data"]

    def sip_end(self, request_id, conversation_id, sip_leg_id):
        return self.request({"op": "sip_end", "request_id": request_id,
                             "conversation_id": conversation_id,
                             "sip_leg_id": sip_leg_id})["data"]

    def context_view(self, conversation_id, limit=100):
        return self.request({"op": "context_view", "conversation_id": conversation_id,
                             "limit": limit})["data"]

    def availability_set(self, request_id, state, expires_at, preferred_route=None):
        return self.request({"op": "availability_set", "request_id": request_id,
                             "state": state, "preferred_route": preferred_route,
                             "expires_at": expires_at})["data"]

    def availability_get(self, identity_id):
        return self.request({"op": "availability_get", "identity_id": identity_id})["data"]

    def artifact_begin(self, request_id, conversation_id, name, media_type, size, sha256,
                       expires_at=None):
        return self.request({"op": "artifact_begin", "request_id": request_id,
                             "conversation_id": conversation_id, "name": name,
                             "media_type": media_type, "size": size, "sha256": sha256,
                             "expires_at": expires_at})["data"]

    def artifact_append(self, request_id, conversation_id, artifact_id, offset, data):
        return self.request({"op": "artifact_append", "request_id": request_id,
                             "conversation_id": conversation_id, "artifact_id": artifact_id,
                             "offset": offset, "data": data})["data"]

    def artifact_finish(self, request_id, conversation_id, artifact_id):
        return self.request({"op": "artifact_finish", "request_id": request_id,
                             "conversation_id": conversation_id,
                             "artifact_id": artifact_id})["data"]

    def artifact_get(self, conversation_id, artifact_id, offset=0, limit=32768):
        return self.request({"op": "artifact_get", "conversation_id": conversation_id,
                             "artifact_id": artifact_id, "offset": offset,
                             "limit": limit})["data"]

    def artifact_delete(self, request_id, conversation_id, artifact_id):
        return self.request({"op": "artifact_delete", "request_id": request_id,
                             "conversation_id": conversation_id,
                             "artifact_id": artifact_id})["data"]

    def health(self):
        return self.request({"op": "health"})["data"]
