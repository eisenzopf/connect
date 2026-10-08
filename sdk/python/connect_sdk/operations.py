"""Public operations shared by native and HTTP clients."""
from abc import ABC, abstractmethod


class Operations(ABC):
    @abstractmethod
    def request(self, operation):
        """Return a validated outcome; mutations always use caller-owned IDs."""
        raise NotImplementedError

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

    def health(self):
        return self.request({"op": "health"})["data"]


