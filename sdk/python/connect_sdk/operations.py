"""Public operations shared by native and HTTP clients."""
from .membership_types import MembershipRequest, MembershipOutcome, MembershipSnapshot, MembershipInvitations
from abc import ABC, abstractmethod
from .task_operations import TaskOperations
from .errors import ConnectError


class Operations(TaskOperations, ABC):
    @abstractmethod
    def request(self, operation):
        """Return a validated outcome; mutations always use caller-owned IDs."""
        raise NotImplementedError

    def _event_read(self, operation):
        data=self.request(operation)["data"]
        if data.get("conversation_id")!=operation["conversation_id"] or ("session_id" in operation and data.get("session_id")!=operation["session_id"]):
            raise ConnectError("protocol","Event result scope does not match the request")
        return data

    def conversation_snapshot(self, conversation_id, query=None):
        return self._event_read(dict(op="conversation_snapshot",conversation_id=conversation_id,query={} if query is None else query))

    def conversation_events(self, conversation_id, query=None):
        return self._event_read(dict(op="conversation_events",conversation_id=conversation_id,query={} if query is None else query))

    def session_observations(self, conversation_id, session_id, query=None):
        return self._event_read(dict(op="session_observations",conversation_id=conversation_id,session_id=session_id,query={} if query is None else query))

    def admit_transition(self, request):
        """Admit durable intent; acceptance is not a route/media commit."""
        return self.request({"op": "transition", "request": request})["data"]

    def operation_action(self, request):
        return self.request({"op": "durable_operation", "request": request})["data"]

    def contact_pairing_start(self, request_id, peer_identity_id, expires_at):
        """Start native ownership pairing; no SMS consent is granted."""
        return self.request({"op": "contact_pairing_start", "request_id": request_id,
                             "peer_identity_id": peer_identity_id, "expires_at": expires_at})["data"]

    def contact_pairing_complete(self, request_id, pairing_id, challenge):
        """Complete as the intended authenticated peer using its one-use challenge."""
        return self.request({"op": "contact_pairing_complete", "request_id": request_id,
                             "pairing_id": pairing_id, "challenge": challenge})["data"]

    def contact_pairing_revoke(self, request_id, pairing_id):
        return self.request({"op": "contact_pairing_revoke", "request_id": request_id,
                             "pairing_id": pairing_id})["data"]

    def contact_state(self, peer_identity_id):
        return self.request({"op": "contact_state", "peer_identity_id": peer_identity_id})["data"]

    def capabilities(self, address, conversation_id=None):
        """Return authorized configured routes and limits; not a reachability guarantee."""
        return self.request({"op": "capabilities", "address": address,
                             "conversation_id": conversation_id})["data"]

    def browser_binding(self, conversation_id=None, session_id=None):
        """Discover the configured route or a currently authorized accepted AV session."""
        if (conversation_id is None) != (session_id is None):
            raise ValueError("Select both conversation_id and session_id")
        operation = {"op": "browser_binding"}
        if conversation_id is not None:
            operation.update(conversation_id=conversation_id, session_id=session_id)
        return self.request(operation)["data"]

    def context_deliveries(self, conversation_id, before=None, limit=100):
        """Page newest-first delivery receipts with an opaque older-page cursor."""
        return self.request({"op": "context_deliveries", "conversation_id": conversation_id,
                             "before": before, "limit": limit})["data"]

    def resolve(self, address):
        return self.request({"op": "resolve", "address": address})["data"]

    def create(self,request_id,participants):
        return self.create_conversation(request_id,{"tenant_id":None,"policy":"persistent","idle_close_secs":None,"metadata":None,"initial_participants":[{"identity_id":identity_id,"role":"participant"} for identity_id in sorted(set(participants))]})

    def create_conversation(self,request_id,request):
        value=self.request({"op":"create","request_id":request_id,"request":request})["data"]
        from .protocol import valid_mutation_receipt
        if not valid_mutation_receipt(value.get("receipt"),request_id,value.get("id")):
            raise ConnectError("protocol","Conversation receipt does not match its mutation")
        return value

    def list_conversations(self,query=None):
        return self.request({"op":"conversations","query":{} if query is None else query})["data"]

    def change_membership(self,request: MembershipRequest) -> MembershipOutcome:
        value=self.request({"op":"membership","request":request})["data"]
        from .protocol import valid_mutation_receipt
        if not valid_mutation_receipt(value.get("receipt"),request["operation_id"],request["conversation_id"]):
            raise ConnectError("protocol","Membership receipt does not match its mutation")
        return value

    def membership_snapshot(self,conversation_id: str) -> MembershipSnapshot:
        value=self.request({"op":"membership_snapshot","conversation_id":conversation_id})["data"]
        if value.get("conversation_id")!=conversation_id:raise ConnectError("protocol","Membership snapshot scope mismatch")
        return value

    def membership_invitations(self,after: str | None=None,limit: int=100) -> MembershipInvitations:
        return self.request({"op":"membership_invitations","after":after,"limit":limit})["data"]

    def send(self, request_id, conversation_id, body):
        """Send UTF-8 text to all current members, frozen at acceptance."""
        return self.send_message(request_id, conversation_id, {
            "to": "all", "content_type": "text/plain", "body": body,
            "body_encoding": "utf8", "label": "connect-messages", "attachments": [],
            "in_reply_to_msg": None, "msg_id": None})

    def send_message(self, request_id, conversation_id, message):
        """Send an explicit participant audience and typed body using a caller-owned ID."""
        data = self.request({"op": "send", "request_id": request_id,
                             "conversation_id": conversation_id, "message": message})["data"]
        from .protocol import valid_mutation_receipt
        if data.get("request_id") != request_id or data.get("conversation_id") != conversation_id or not valid_mutation_receipt(data.get("receipt"), request_id, conversation_id):
            raise ConnectError("protocol", "Message receipt does not match its mutation")
        return data

    def message_receipts(self, conversation_id, message_id, after=None, limit=100):
        return self.request({"op": "message_receipts", "conversation_id": conversation_id, "message_id": message_id, "after": after, "limit": limit})["data"]

    def message_receipt(self, request_id, conversation_id, message_id, state):
        return self.request({"op": "message_receipt", "request_id": request_id, "conversation_id": conversation_id, "message_id": message_id, "state": state})["data"]

    def receipt_preferences(self, conversation_id):
        return self.request({"op": "receipt_preferences_get", "conversation_id": conversation_id})["data"]

    def set_receipt_preferences(self, request_id, conversation_id, delivery, read):
        return self.request({"op": "receipt_preferences_set", "request_id": request_id, "conversation_id": conversation_id, "delivery": delivery, "read": read})["data"]

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

    def close_conversation(self,request_id,conversation_id,options=None):
        value=self.request({"op":"close","request_id":request_id,"conversation_id":conversation_id,"options":{"expected_revision":None,"reason_code":None,"reason":None} if options is None else options})["data"]
        from .protocol import valid_mutation_receipt
        if value.get("id")!=conversation_id or not valid_mutation_receipt(value.get("receipt"),request_id,conversation_id):
            raise ConnectError("protocol","Conversation receipt does not match its mutation")
        return value

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
                       expires_at=None, to="all"):
        return self.request({"op": "artifact_begin", "to": to, "request_id": request_id,
                             "conversation_id": conversation_id, "name": name,
                             "media_type": media_type, "size": size, "sha256": sha256,
                             "expires_at": expires_at})["data"]

    def artifact_inspect(self, conversation_id, artifact_id):
        return self.request({"op": "artifact_inspect", "conversation_id": conversation_id, "artifact_id": artifact_id})["data"]

    def artifact_upload_link(self, conversation_id, artifact_id, expires_in=60):
        return self.request({"op": "artifact_upload_link", "conversation_id": conversation_id, "artifact_id": artifact_id, "expires_in": expires_in})["data"]

    def artifact_download_link(self, conversation_id, artifact_id, message_id, expires_in=60):
        return self.request({"op": "artifact_download_link", "conversation_id": conversation_id, "artifact_id": artifact_id, "message_id": message_id, "expires_in": expires_in})["data"]

    def artifact_link_get(self, link, offset=0, limit=32768):
        return self.request({"op": "artifact_link_get", "conversation_id": link["conversation_id"], "artifact_id": link["artifact_id"], "link": link["path"], "offset": offset, "limit": limit})["data"]

    def artifact_link_append(self, request_id, link, offset, data):
        """Base64 chunk. Keep request_id stable when refreshing a link."""
        return self.request({"op": "artifact_link_append", "request_id": request_id, "conversation_id": link["conversation_id"], "artifact_id": link["artifact_id"], "link": link["path"], "offset": offset, "data": data})["data"]

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
