"""Independent public-SDK checks of source, effect and replay behavior."""
import base64
import hashlib
import json
from .common import ProofFailure, require, save


class Observer:
    def histories(self, NativeClient, name):
        histories, pagination = {}, {}
        with NativeClient(self.human_config, str(self.binary)) as peer:
            for key, cid in self.conversations.items():
                after, messages, pages = None, [], []
                for _ in range(3):
                    page = peer.history(cid, after, 100)
                    batch = page["messages"]
                    pages.append({"count": len(batch), "next_present": page.get("next") is not None})
                    require(len(batch) <= 100, "bounded-history-page")
                    if not batch:
                        require(page.get("next") is None, "empty-terminal-history-page")
                        break
                    messages.extend(batch)
                    following = page.get("next")
                    require(isinstance(following, str) and following != after, "advancing-history-cursor")
                    after = following
                else:
                    raise ProofFailure("bounded-history-pagination-exhausted")
                require(len(messages) <= 100 and len({m["id"] for m in messages}) == len(messages),
                        "bounded-complete-deduplicated-history")
                histories[key] = {"messages": messages, "next": None}
                pagination[key] = pages
        save(self.directory / (name + "-history-pagination.json"), pagination)
        save(self.directory / (name + "-history.json"), histories)
        return histories


    def drafts(self, NativeClient, histories):
        found = {}
        with NativeClient(self.human_config, str(self.binary)) as peer:
            for name, history in histories.items():
                proposals = []
                for message in history["messages"]:
                    try:
                        body = json.loads(message["body"])
                    except (ValueError, TypeError):
                        continue
                    if isinstance(body, dict) and body.get("tag") == "draft-review":
                        proposals.append((message, body))
                require(len(proposals) == 1, "exactly-one-draft-question-per-conversation")
                message, body = proposals[0]
                require(message["sender"] == self.agent_id, "actual-agent-draft-question")
                require(body.get("title") == f"proposal-{name}.txt", "correct-draft-name")
                require(body.get("source_message_id") == self.facts[name]["message_id"], "draft-original-source")
                descriptor = body["artifact"]
                require(isinstance(descriptor, dict), "actual-ready-artifact-descriptor")
                chunk = peer.artifact_get(self.conversations[name], descriptor["id"], 0, 32768)
                data = base64.b64decode(chunk["data"], validate=True)
                actual = chunk["artifact"]
                require(actual["state"] == descriptor["state"] == "ready"
                        and actual["owner_id"] == self.agent_id
                        and actual["conversation_id"] == self.conversations[name],
                        "actual-ready-owned-conversation-artifact")
                require(chunk["eof"] and 20 < len(data) <= 8192, "complete-useful-bounded-artifact")
                require(hashlib.sha256(data).hexdigest() == actual["sha256"] == descriptor["sha256"],
                        "artifact-byte-digest")
                require(len(data) == actual["size"] == descriptor["size"], "artifact-byte-size")
                text = data.decode("utf-8")
                require(all(value in text for value in self.facts[name].values()), "artifact-retained-conversation-facts")
                other = "b" if name == "a" else "a"
                require(self.facts[other]["code"] not in text, "no-cross-conversation-fact-substitution")
                found[name] = {"conversation_id": self.conversations[name], "question_id": message["id"],
                               "task_id": body["task_id"], "artifact_id": descriptor["id"],
                               "sha256": actual["sha256"], "size": len(data),
                               "source_message_id": body["source_message_id"]}
                download = self.directory / f"proposal-{name}-{hashlib.sha256(data).hexdigest()}.txt"
                if not download.exists():
                    with download.open("xb") as stream:
                        stream.write(data)
        require(found["a"]["task_id"] != found["b"]["task_id"], "two-distinct-owned-tracking-tasks")
        return found


    def task_records(self, NativeClient, expected, name):
        records = {}
        with NativeClient(self.agent_config, str(self.binary)) as agent:
            for key, draft in self.observations.items():
                record = agent.negotiate_task(draft["task_id"])
                require(record["owner_id"] == self.agent_id and record["conversation_id"] == self.conversations[key],
                        "explicit-task-owner-and-conversation")
                if key in expected:
                    require(record["state"] == expected[key], "tracked-state-without-invented-approval")
                records[key] = record
        save(self.directory / (name + "-tasks.json"), records)
        return records


    def verify_completion(self, NativeClient, histories):
        source_reply = self.reply["data"]["message_id"]
        tagged = {}
        for name, history in histories.items():
            tagged[name] = []
            for message in history["messages"]:
                if message["sender"] != self.agent_id:
                    continue
                try:
                    body = json.loads(message["body"])
                except (ValueError, TypeError):
                    continue
                if isinstance(body, dict):
                    tagged[name].append((message, body))
        followups = [(m, b) for m, b in tagged["a"] if b.get("tag") == "review-followup"]
        incoming = [(m, b) for m, b in tagged["b"] if b.get("tag") == "incoming-answer"]
        require(len(followups) == len(incoming) == 1, "one-useful-followup-and-one-independent-answer")
        _, followup = followups[0]
        require(followup.get("task_id") == self.observations["a"]["task_id"]
                and followup.get("reviewer_code") == self.review_code
                and followup.get("source_reply_message_id") == source_reply
                and followup.get("source_message_id") == self.facts["a"]["message_id"],
                "attributed-review-source-and-new-code")
        require(all(self.facts["a"][field] in followup.get("body", "") for field in ("project", "slot", "code")),
                "review-useful-retained-context")
        _, answer = incoming[0]
        require(answer.get("request_id") == self.incoming_request
                and answer.get("source_message_id") == self.incoming["message_id"]
                and answer.get("context_source_message_id") == self.facts["b"]["message_id"],
                "independent-answer-incoming-and-context-sources")
        require(all(self.facts["b"][field] in answer.get("body", "") for field in ("project", "slot", "code")),
                "independent-useful-retained-context")
        require(not any(body.get("tag") == "review-followup" for _, body in tagged["b"]),
                "independent-contact-does-not-approve-task")
        require(self.drafts(NativeClient, histories) == self.observations, "unchanged-artifacts-and-questions-after-resume")
        records = self.task_records(NativeClient, {"b": "waiting"}, "after-resume")
        require(records["a"]["state"] not in ("cancelled", "failed", "expired"), "answered-task-not-falsely-rejected")
        with NativeClient(self.agent_config, str(self.binary)) as agent:
            agent.negotiate_task(self.observations["a"]["task_id"])
            replies = agent.task({"action": "replies", "conversation_id": self.conversations["a"],
                                  "after": None, "limit": 100})
            assigned = [r for r in replies["data"]["items"] if r["event_id"] == self.reply["data"]["reply"]["event_id"]]
            require(len(assigned) == 1
                    and assigned[0]["task_id"] == self.observations["a"]["task_id"],
                    "original-reply-attribution-retained-by-own-task")
            require(assigned[0]["consumed"] is True, "explicitly-consumed-handled-reply")
            journal = agent.task({"action": "wait", "task_id": self.observations["a"]["task_id"],
                                  "after": None, "limit": 100, "wait_ms": 0})
            followup_id = followups[0][0]["id"]
            observed = self.model_followup_invocation
            effects = [event["data"]["data"] for event in journal["data"]["events"]
                       if event["data"]["kind"] == "effect"
                       and event["data"]["data"]["request_id"] == observed["arguments"]["request_id"]
                       and event["data"]["data"]["step_id"] == observed["arguments"]["step_id"]]
            require(effects and len({effect["id"] for effect in effects}) == 1,
                    "one-original-followup-effect-in-journal")
            effect = agent.task({"action": "effect_get", "task_id": self.observations["a"]["task_id"],
                                 "effect_id": effects[0]["id"]})["data"]
            observed = self.model_followup_invocation
            require(effect["state"] == "succeeded"
                    and observed["arguments"]["reply_to"] == self.reply["data"]["reply"]["event_id"]
                    and observed["arguments"]["task_id"] == self.observations["a"]["task_id"]
                    and effect["request_id"] == observed["arguments"]["request_id"]
                    and effect["step_id"] == observed["arguments"]["step_id"]
                    and effect["outcome"]["data"]["message_id"] == followup_id
                    and observed["structured"]["data"]["data"]["outcome"] == effect["outcome"],
                    "one-succeeded-followup-correlates-actual-call-and-exact-attributed-reply")
            save(self.directory / "review-reply-attribution.json", replies)
            save(self.directory / "review-followup-effect.json", effect)
            self.report["reply_semantics"] = {
                "reply_to": "authorizes/correlates the tracked effect; it does not implicitly consume the inbox marker",
                "consumed_marker": assigned[0]["consumed"],
                "explicit_consume": "explicit normal MCP consume command, independently observed through the public SDK",
                "followup_effect_id": effect["id"], "exact_reply_event_id": observed["arguments"]["reply_to"],
            }
        # Replay only the original public reviewer requests. This cannot inject
        # another model answer or turn an independent message into approval.
        with NativeClient(self.human_config, str(self.binary)) as peer:
            peer.negotiate_tasks("connect:model-review@local", self.conversations["a"])
            replay = peer.task({"action": "reply", "request_id": self.reply_request,
                                "conversation_id": self.conversations["a"],
                                "reply_to": self.observations["a"]["question_id"], "body": self.reply_body})
            require(replay["data"]["message_id"] == source_reply
                    and replay["data"]["reply"]["event_id"] == self.reply["data"]["reply"]["event_id"],
                    "original-review-request-replayed-without-duplicate")
            replay_incoming = peer.send("incoming-send-" + self.run_id, self.conversations["b"], self.incoming_body)
            require(replay_incoming == self.incoming, "original-independent-send-replayed-without-duplicate")
        require(self.histories(NativeClient, "after-peer-replay") == histories, "no-extra-messages-on-request-replay")
        self.assert_installation_unchanged()
        require(all(c["interface"][0] != "task-access" for c in self.commands), "no-task-grant-setup-or-renewal")
        self.report.update(
            status="passed", actual_inference=True,
            one_credential_across_two_authorized_conversations=True,
            artifacts_verified_before_after_restart=self.observations,
            same_exact_host_session_after_server_restart=True,
            first_turn=self.first_turn, resumed_turn=self.resumed_turn,
            source_backed_review_followup_count=1, source_backed_independent_incoming_answer_count=1,
            unanswered_review_task_still_waiting=True, original_reply_attribution_retained_once=True,
            public_request_replay_preserves_messages=True,
            no_task_grant_issuance_or_renewal=True,
            continuation="explicit exact saved session resume; delivered continuation prompt, without new user-supplied facts or reconstructed context",
            limits=["Local regular identity profile only; prior scoped-host/provider/SMS proofs retain their original recorded source and scope.",
                    "Explicit host turn/resume, not an always-running scheduler or unattended mailbox service.",
                    "No live voice/SMS/provider operation, remote deployment or security change."],
        )


