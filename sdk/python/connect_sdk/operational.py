"""Closed administrator diagnostic DTOs; no identifiers or private payload fields."""
POOLS = frozenset(["quic-ingress","native-quic","webtransport","websocket","http-connections","http-requests","http-callback-connections","http-callback-requests","https-application-connections","https-application-requests","https-application-handshakes","https-administration-connections","https-administration-requests","https-administration-handshakes","https-callbacks-connections","https-callbacks-requests","https-callbacks-handshakes","https-health-connections","https-health-requests","https-health-handshakes","record-https-connections","record-https-requests","record-https-handshakes","mcp-requests","mcp-connections"])
LIMITS = ["message_bytes","page_size","journal_events","operation_receipts","message_storage_bytes","participants","wait_ms","request_timeout_ms","connections","sessions","artifact_file_bytes","artifact_bytes","artifact_chunk_bytes"]
OPERATIONS = ["retained","accepted","preparing","ready","committing","interrupted","unknown"]
MAX = 9007199254740991
def closed(value, keys):
    return isinstance(value, dict) and set(value) == set(keys)
def counter(value):
    return type(value) is int and 0 <= value <= MAX
def counters(value, keys):
    return closed(value, keys) and all(counter(value[k]) for k in keys)
def u32(value):
    return counter(value) and value <= 4294967295
def pool(value):
    if not closed(value, ("kind", "state", "usage")) or not isinstance(value["kind"], str) or value["kind"] not in POOLS:
        return False
    if value["state"] in ("busy", "retired"):
        return value["usage"] is None
    usage = value["usage"]
    return (value["state"] == "available" and closed(usage, ("used", "limit", "accepted", "rejected", "draining"))
        and u32(usage["used"]) and u32(usage["limit"]) and counter(usage["accepted"])
        and counter(usage["rejected"]) and type(usage["draining"]) is bool)
def valid_operational(value):
    if (not closed(value, ("observed_at", "limits", "operations", "runtime", "resource_pools"))
        or not counter(value["observed_at"]) or not counters(value["limits"], LIMITS)
        or not counters(value["operations"], OPERATIONS)):
        return False
    if value["operations"]["retained"] < min(MAX, sum(value["operations"][k] for k in OPERATIONS if k != "retained")):
        return False
    runtime = value["runtime"]
    if runtime is not None and not (closed(runtime, ("physical_owners", "physical_capacity", "owned_tasks")) and all(u32(v) for v in runtime.values())):
        return False
    pools = value["resource_pools"]
    return pools is None or (isinstance(pools, list) and len(pools) <= 32 and all(pool(v) for v in pools) and len({v["kind"] for v in pools}) == len(pools))
