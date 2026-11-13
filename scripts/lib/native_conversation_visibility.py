"""Exact original programme-owned Conversation visibility, never a count waiver."""
import re


def token(value):
    return isinstance(value, str) and re.fullmatch(r"[A-Za-z0-9._~-]{1,128}", value) is not None


def owned_conversation_ids(observations, proofs, cycles):
    if type(cycles) is not int or cycles not in (1, 3) or len(observations) != cycles or len(proofs) != cycles:
        raise ValueError("Every original programme creation must be present")
    ids, operations = set(), set()
    for observed, proof in zip(observations, proofs):
        created = observed.get("original_created_conversation", {})
        reference = proof.get("reference", {})
        cid, operation = created.get("conversation_id"), created.get("create_operation_id")
        digest = proof.get("reference_sha256")
        if (created.get("format") != "connect/original-native-created-conversation/1"
                or not token(cid) or not token(operation) or cid in ids or operation in operations
                or operation != proof.get("planned_create_operation_id")
                or reference.get("schema") != "connect-operation-reference/1"
                or reference.get("command") != "conversation.create" or reference.get("scope") != {}
                or reference.get("operation_id") != operation
                or not isinstance(digest, str) or re.fullmatch(r"[0-9a-f]{64}", digest) is None
                or created.get("original_reference_sha256") != digest):
            raise ValueError("Original public creation differs from its sealed programme plan/reference")
        ids.add(cid); operations.add(operation)
    return ids


def require_exact_conversation_visibility(visible, expected, owners):
    if (not expected or set(visible) != set(owners) or not owners):
        raise ValueError("Every original independent owner must be observed")
    for rows in visible.values():
        if (not isinstance(rows, list) or len(rows) != len(expected)
                or any(not isinstance(row, dict) or not token(row.get("id"))
                    or row.get("policy") != "persistent" for row in rows)
                or {row["id"] for row in rows} != expected):
            raise ValueError("Current visibility differs from the exact original programme-owned set")
