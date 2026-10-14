"""Native ownership DTOs must never invent SMS consent from pairing."""
from ._validation import fields
STATES = {"unknown", "pending", "verified_native", "expired", "revoked", "attempts_exceeded"}


def valid_contact(kind, value):
    if kind == "contact_pairing":
        return (fields(value, ("id", "owner_identity_id", "peer_identity_id", "state"))
                and isinstance(value["state"], str) and value["state"] in STATES and type(value.get("expires_at")) is int
                and value["expires_at"] >= 0
                and ("challenge" not in value or isinstance(value["challenge"], str)))
    return (fields(value, ("peer_identity_id", "ownership"), ("pairing_id",))
            and isinstance(value["ownership"], str) and value["ownership"] in STATES and value.get("sms_consent") is False
            and "verified_at" in value and (value["verified_at"] is None
                or (type(value["verified_at"]) is int and value["verified_at"] >= 0)))
