"""Pure lineage/visibility negatives; actual SDK and current authority remain separate."""
import copy
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
from lib.native_conversation_visibility import owned_conversation_ids, require_exact_conversation_visibility


class OwnedConversationSet(unittest.TestCase):
    def fixture(self, cycles=3):
        originals, proofs = [], []
        for index in range(cycles):
            operation, cid, digest = "create_" + str(index), "cid_" + str(index), str(index + 1) * 64
            originals.append(dict(original_created_conversation=dict(
                format="connect/original-native-created-conversation/1", conversation_id=cid,
                create_operation_id=operation, original_reference_sha256=digest)))
            proofs.append(dict(planned_create_operation_id=operation, reference_sha256=digest,
                reference=dict(schema="connect-operation-reference/1", command="conversation.create",
                    scope={}, operation_id=operation)))
        return originals, proofs

    def test_three_original_creations_require_exact_owned_set_for_all_six_current_configs(self):
        originals, proofs = self.fixture()
        expected = owned_conversation_ids(originals, proofs, 3)
        self.assertEqual(expected, {"cid_0", "cid_1", "cid_2"})
        owners = {"controller-1", "controller-2", "participant-2", "participant-3", "participant-4", "replacement-1"}
        visible = {owner:[dict(id=cid, policy="persistent") for cid in reversed(sorted(expected))] for owner in owners}
        require_exact_conversation_visibility(visible, expected, owners)
        # The same exact original snapshot remains required after real primary revocation.
        after = copy.deepcopy(visible)
        require_exact_conversation_visibility(after, expected, owners)
        self.assertEqual(after, visible)

    def test_foreign_missing_wrong_duplicate_room_or_owner_cannot_borrow_three_cycle_count(self):
        originals, proofs = self.fixture(); expected = owned_conversation_ids(originals, proofs, 3)
        valid = {"owner":[dict(id=cid, policy="persistent") for cid in sorted(expected)]}
        for mode in ("foreign", "missing", "wrong", "duplicate", "policy", "owner"):
            with self.subTest(mode=mode):
                visible = copy.deepcopy(valid)
                if mode == "foreign": visible["owner"].append(dict(id="foreign", policy="persistent"))
                elif mode == "missing": visible["owner"].pop()
                elif mode == "wrong": visible["owner"][0]["id"] = "foreign"
                elif mode == "duplicate": visible["owner"][0] = dict(visible["owner"][1])
                elif mode == "policy": visible["owner"][0]["policy"] = "ephemeral"
                else: visible["other"] = visible.pop("owner")
                with self.assertRaises(ValueError): require_exact_conversation_visibility(visible, expected, {"owner"})

    def test_original_create_plan_reference_and_completed_cid_must_stay_bound(self):
        for mode in ("plan", "hash", "command", "scope", "duplicate-cid", "duplicate-operation", "missing"):
            with self.subTest(mode=mode):
                originals, proofs = self.fixture()
                if mode == "plan": proofs[0]["planned_create_operation_id"] = "foreign"
                elif mode == "hash": originals[0]["original_created_conversation"]["original_reference_sha256"] = "f" * 64
                elif mode == "command": proofs[0]["reference"]["command"] = "conversation.close"
                elif mode == "scope": proofs[0]["reference"]["scope"] = dict(conversation_id="foreign")
                elif mode == "duplicate-cid": originals[1]["original_created_conversation"]["conversation_id"] = "cid_0"
                elif mode == "duplicate-operation":
                    originals[1] = copy.deepcopy(originals[0]); proofs[1] = copy.deepcopy(proofs[0])
                else: originals.pop()
                with self.assertRaises(ValueError): owned_conversation_ids(originals, proofs, 3)

    def test_single_cycle_requires_its_exact_one_creation_and_unknown_counts_refuse(self):
        originals, proofs = self.fixture(1)
        expected = owned_conversation_ids(originals, proofs, 1)
        require_exact_conversation_visibility({"owner":[dict(id="cid_0", policy="persistent")]}, expected, {"owner"})
        for count in (0, 2, True, None):
            with self.subTest(count=count), self.assertRaises(ValueError): owned_conversation_ids(originals, proofs, count)


if __name__ == "__main__": unittest.main()
