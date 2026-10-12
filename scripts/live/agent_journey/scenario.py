"""Public CLI setup and two-conversation review scenario."""
import json
import subprocess
import time
import tomllib
import uuid
from .common import require, read_json, sha256, save, write_private


class Scenario:
    def setup(self):
        raw = subprocess.run([str(self.codex), "--version"], env=self.model_env,
                             capture_output=True, timeout=self.remaining(15), check=True)
        version = raw.stdout.decode().strip()
        require(len(version) < 300 and "codex" in version.lower(), "selected-codex-version")
        self.report["codex_version"] = version
        self.command("init", ["init", "--directory", self.state, "--example", "review-loop",
                              "--bind", f"127.0.0.1:{self.quic_port}"])
        identity = json.loads(self.command("create-agent", [
            "agent-access", "create-identity", "--config", self.state / "operator.toml",
            "--server-config", self.state / "server.toml", "--alias", "model-review",
            "--request-id", "setup-agent-" + self.run_id,
        ]))
        self.agent_id = identity["identity_id"]
        human = tomllib.loads((self.state / "reviews.toml").read_text())
        self.human_id = human["identity_id"]
        self.agent_config, self.human_config = self.state / "model.toml", self.state / "human.toml"
        for name, actor, output, options in (
            ("issue-agent", self.agent_id, self.agent_config, ["--voice", "--tasks"]),
            ("issue-human", self.human_id, self.human_config, []),
        ):
            self.command(name, ["issue", "--config", self.state / "operator.toml",
                               "--server-config", self.state / "server.toml",
                               "--identity-id", actor, "--recipients", "*", "--output", output,
                               *options])
        with (self.state / "server.toml").open("a") as stream:
            stream.write(f'\n[mcp]\nbind = "127.0.0.1:{self.mcp_port}"\n')
        self.command("export-agent", ["agent-access", "export-codex", "--config", self.agent_config,
                                      "--server-config", self.state / "server.toml",
                                      "--mcp-address", f"127.0.0.1:{self.mcp_port}",
                                      "--output", self.installation])
        self.initial_manifest = read_json(self.installation / "install.json")
        self.initial_config_sha256 = sha256(self.installation / "agent.toml")
        self.assert_installation_unchanged()
        self.start_server("server-first")


    def assert_installation_unchanged(self):
        manifest = read_json(self.installation / "install.json")
        require(manifest == self.initial_manifest, "same-agent-installation-manifest")
        require(manifest["profile"] == "connect-codex-agent-install/1", "regular-installation-mode")
        require(manifest["identity_id"] == self.agent_id, "selected-agent-identity")
        require(sha256(self.installation / "agent.toml") == self.initial_config_sha256,
                "same-agent-key-config")
        selected = tomllib.loads(self.agent_config.read_text())
        installed = tomllib.loads((self.installation / "agent.toml").read_text())
        require(selected["credential"] == installed["credential"], "same-selected-normal-key")
        require(not installed["credential_id"].startswith("task-"), "not-a-task-principal")
        require(not any("grant" in p.name or "renew" in p.name
                        for p in self.installation.iterdir()), "no-task-grant-or-renewal-files")
        require(manifest["credential_expires_at"] > int(time.time()) + 24 * 60 * 60,
                "ordinary-longer-lived-credential")
        self.report["identity_id"] = self.agent_id
        self.report["credential_id"] = installed["credential_id"]
        self.report["installation_manifest_sha256"] = sha256(self.installation / "install.json")


    def seed(self, NativeClient):
        with NativeClient(self.human_config, str(self.binary)) as peer:
            for name, project, slot in (("a", "Aster", "Friday 11:45"), ("b", "Juniper", "Tuesday 14:20")):
                cid = peer.create("conversation-" + name + "-" + self.run_id, [self.agent_id])["id"]
                code = name.upper() + "-" + uuid.uuid4().hex[:12]
                body = f"Project {project}: the agreed review slot is {slot}. Retain reference code {code}. Use these facts in this conversation's proposal."
                message = peer.send("seed-" + name + "-" + self.run_id, cid, body)
                self.conversations[name] = cid
                self.facts[name] = {"project": project, "slot": slot, "code": code,
                                    "message_id": message["message_id"]}
        save(self.directory / "seed-facts.json", self.facts)
        prompt = f"""Use your installed $connect skill. Every Connect communication action must use its installed ordinary MCP tools. You have one agent credential across authorized conversations; no task grant or renewal is needed. You may use your own local computational tools to read the installed skill, write drafts under ./draft-work, and calculate UTF-8 byte lengths, SHA-256 and base64 for artifact upload. Calculate these values yourself from the actual draft bytes; no test harness supplies them. Local artifact preparation is allowed, but do not use a shell/Python/SDK/HTTP/Store communication shortcut. Do not read sibling directories, private operator/model/agent configuration files, account credentials or environment secrets. Read the installed skill and current tool schema, and inspect your ordinary identity with connect_inspect.

Two existing conversations with reviewer {self.human_id} need separate proposals:
A: conversation_id {self.conversations['a']}, artifact name proposal-a.txt.
B: conversation_id {self.conversations['b']}, artifact name proposal-b.txt.

Read each conversation's Connect context to recover its project, review slot, reference code and original source message ID. Those facts are not in this prompt. Compose a useful brief UTF-8 proposal for each, including those exact facts and that source ID. Upload real bytes using artifact_begin/append/finish with text/plain, correct byte size and SHA-256. Keep each proposal within 8 KiB.

For each conversation create ONE optional tracking task using connect_task_create (request_id + conversation_id suffice). Send ONE tracked approval question using connect_task_send with wait_for_reply=true. Message body must be valid JSON with tag="draft-review", title="proposal-a.txt" or "proposal-b.txt", task_id=the returned task ID, artifact=the full returned ready artifact descriptor, question=your concise approval question, and source_message_id=that conversation's original fact message ID. Preserve the logical request/step IDs and task references. Use a bounded task wait; if unanswered, leave the task waiting and finish this host turn. Do not reupload, recreate tasks or send another approval question on later resume.

On an explicit later resume, poll connect_inbox for independent incoming messages, read retained context as needed, and use connect_task_wait to inspect replies for both tracked tasks. If a task has an attributed reviewer reply, send ONE tracked useful final follow-up using that task's received journal event ID as reply_to. Its JSON body must have tag="review-followup", task_id, reviewer_code=the new code from the reply, source_reply_message_id=that reply message's ID, source_message_id=the original fact message ID, and body=your concrete accepted proposal summary with the original project/slot/code. After the confirmed send, explicitly call connect_task_consume_reply with that same received journal event ID, a stable request_id, task_id, and the current expected_revision from connect_task_status. Correlation does not consume a reply. Only after confirmed consumption, you may mark that task completed using its newly current revision.

Also handle independently delivered messages with JSON action="contact_assistant" through ordinary communication. Such a message is not a task approval. For each, send ONE ordinary connect_send answer in the same conversation with JSON tag="incoming-answer", request_id=that incoming body's request_id, source_message_id=the incoming message ID, context_source_message_id=the original fact message ID, and body=the requested useful answer citing the retained project/slot/reference code. Use a separate stable Connect mutation request_id. Leave an unanswered review task waiting; never consume another task's reply or invent approval. Persist/reconcile original mutation IDs across uncertainty. Do not claim success from prose instead of actual Connect tool receipts.
"""
        write_private(self.directory / "first-prompt.txt", prompt)


    def deliver_after_restart(self, NativeClient):
        self.review_code = "APPROVE-" + uuid.uuid4().hex[:12]
        self.reply_request = "review-reply-" + self.run_id
        self.reply_body = f"Approve this proposal. Include reviewer code {self.review_code} in one follow-up and preserve its original facts. This approves only this specific question."
        self.incoming_request = "independent-contact-" + uuid.uuid4().hex
        self.incoming_body = json.dumps({"action": "contact_assistant", "request_id": self.incoming_request,
                                        "body": "Please remind me of this conversation's project, agreed review slot and reference code using its retained context, and cite both the original fact message and this request. This independent request does not approve the pending proposal."})
        with NativeClient(self.human_config, str(self.binary)) as peer:
            peer.negotiate_tasks("connect:model-review@local", self.conversations["a"])
            self.reply = peer.task({"action": "reply", "request_id": self.reply_request,
                                    "conversation_id": self.conversations["a"],
                                    "reply_to": self.observations["a"]["question_id"], "body": self.reply_body})
            require(self.reply["kind"] == "reply_accepted"
                    and self.reply["data"]["reply"]["task_id"] == self.observations["a"]["task_id"],
                    "public-reply-attributed-to-exact-question-task")
            self.incoming = peer.send("incoming-send-" + self.run_id, self.conversations["b"], self.incoming_body)
        save(self.directory / "human-review-reply.json", self.reply)
        save(self.directory / "independent-incoming.json", self.incoming)


    def run(self, NativeClient):
        self.setup()
        self.seed(NativeClient)
        print(json.dumps({"private_work": str(self.directory), "phase": "actual-regular-agent-drafts"}), flush=True)
        self.first_turn = self.model_turn("model-first", True)
        self.observations = self.drafts(NativeClient, self.histories(NativeClient, "before-restart"))
        save(self.directory / "verified-drafts.json", self.observations)
        self.task_records(NativeClient, {"a": "waiting", "b": "waiting"}, "before-restart")
        self.stop_server("server-first")
        self.start_server("server-restarted")
        require(self.drafts(NativeClient, self.histories(NativeClient, "after-restart")) == self.observations,
                "restart-preserves-draft-bytes-questions-and-sources")
        self.deliver_after_restart(NativeClient)
        self.resumed_turn = self.model_turn("model-resume", False)
        self.verify_completion(NativeClient, self.histories(NativeClient, "after-resume"))


