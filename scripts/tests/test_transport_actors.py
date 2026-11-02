"""Enrollment plans preserve originals and cannot expand authority on resume."""
import copy
import json
from pathlib import Path
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
from lib.transport_actors import CONTROLLERS, DEVICES, OPERATIONS, ROLES, encoded, exact, new_plan, validate_plan
from lib.transport_fixture import FixtureError


class ActorPlanBoundary(unittest.TestCase):
    def setUp(self):
        self.binding = dict(cell_id="fixture-cell", binary_sha256="b" * 64, instance_sha256="a" * 64,
            admin_config_sha256="c" * 64, operations_sha256="d" * 64, source_revision="e" * 40)
        self.plan = new_plan(self.binding, 2000)

    def test_saved_plan_roundtrip_retains_all_original_ids_and_replacement_identity(self):
        saved = json.loads(encoded(self.plan))
        validate_plan(saved, self.binding, 2000, 1000)
        self.assertEqual(saved, self.plan)
        self.assertEqual([v["role"] for v in saved["identities"]], list(ROLES))
        self.assertEqual([v["name"] for v in saved["devices"]], list(DEVICES))
        self.assertEqual(saved["devices"][-1]["role"], "participant-1")
        originals = [v["operation_id"] for v in saved["identities"] + saved["devices"]]
        self.assertEqual(len(set(originals)), 9)

    def test_changed_issuer_candidate_scope_or_expiry_refuses_resume(self):
        for field in self.binding:
            with self.subTest(field=field):
                binding = dict(self.binding, **{field: "changed"})
                with self.assertRaises(FixtureError):
                    validate_plan(self.plan, binding, 2000, 1000)
        for ceiling, now in [(1999, 1000), (2000, 2000)]:
            with self.assertRaises(FixtureError):
                validate_plan(self.plan, self.binding, ceiling, now)

    def test_duplicate_operation_or_replacement_role_change_refuses(self):
        for mutate in [lambda p:p["devices"][-1].update(role="participant-2"),
                lambda p:p["devices"][1].update(operation_id=p["devices"][0]["operation_id"]),
                lambda p:p["identities"].pop(), lambda p:p.update(extra="private")]:
            plan = copy.deepcopy(self.plan)
            mutate(plan)
            with self.assertRaises(FixtureError):
                validate_plan(plan, self.binding, 2000, 1000)


    def test_media_controller_mode_preserves_four_identities_and_seven_original_device_commands(self):
        plan = new_plan(self.binding, 2000, movement_controllers=True)
        saved = json.loads(encoded(plan))
        validate_plan(saved, self.binding, 2000, 1000, movement_controllers=True)
        self.assertEqual(saved, plan)
        self.assertEqual([v["name"] for v in saved["devices"]], list(DEVICES + CONTROLLERS))
        self.assertEqual(len(saved["identities"]), 4)
        self.assertEqual([v["role"] for v in saved["devices"][-2:]], ["participant-1"] * 2)
        originals = [v["operation_id"] for v in saved["identities"] + saved["devices"]]
        self.assertEqual(len(set(originals)), 11)

    def test_saved_mode_cannot_expand_or_drop_controller_devices_on_resume(self):
        media = new_plan(self.binding, 2000, movement_controllers=True)
        with self.assertRaises(FixtureError):
            validate_plan(self.plan, self.binding, 2000, 1000, movement_controllers=True)
        with self.assertRaises(FixtureError):
            validate_plan(media, self.binding, 2000, 1000)
        for mutate in [lambda p:p["devices"][-1].update(role="participant-2"),
                lambda p:p["devices"][-1].update(operation_id=p["devices"][-2]["operation_id"]),
                lambda p:p["devices"].reverse(), lambda p:p["devices"].pop()]:
            plan = copy.deepcopy(media)
            mutate(plan)
            with self.assertRaises(FixtureError):
                validate_plan(plan, self.binding, 2000, 1000, movement_controllers=True)

    def test_controller_mode_is_an_explicit_boolean(self):
        for mode in [None, 0, 1, "true"]:
            with self.assertRaises(FixtureError):
                new_plan(self.binding, 2000, movement_controllers=mode)

    def test_exact_publication_never_replaces_existing_intent_or_symlink(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "plan.json"
            exact(path, encoded(self.plan))
            exact(path, encoded(self.plan))
            with self.assertRaises(FixtureError):
                exact(path, b"changed")
            self.assertEqual(path.read_bytes(), encoded(self.plan))
            link = Path(temp) / "linked.json"
            link.symlink_to(path)
            with self.assertRaises(FixtureError):
                exact(link, encoded(self.plan))

    def test_qualification_scope_excludes_admin_and_external_communications(self):
        self.assertFalse(set(OPERATIONS) & {"admin", "*", "installation.observe", "call_start", "sip_dial", "task"})
        self.assertTrue({"connection.replace", "session.update", "conversation.participant.remove", "stream.subscribe", "connection.end"} <= set(OPERATIONS))


if __name__ == "__main__":
    unittest.main()
