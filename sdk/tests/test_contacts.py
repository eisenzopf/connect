"""Malformed native ownership responses fail closed without echoing secrets."""
import json
import unittest
from connect_sdk import ConnectError
from connect_sdk.protocol import decode_response

class ContactResponses(unittest.TestCase):
    def decode(self, kind, data):
        return decode_response(json.dumps({"profile":"connect-control/1", "result":{"Ok":{"kind":kind,"data":data}}}), kind)

    def test_contact_cannot_infer_sms_consent_or_accept_malformed_ownership(self):
        contact = {"peer_identity_id":"peer","ownership":"verified_native","pairing_id":"pairing","verified_at":1,"sms_consent":False}
        self.assertEqual(self.decode("contact", contact)["data"], contact)
        for field, values in {"sms_consent":[True, 0, None, "false"], "ownership":["verified_phone", None, {}], "verified_at":[True, -1, "now"], "pairing_id":[0, {}]}.items():
            for value in values:
                with self.subTest(field=field, value=value), self.assertRaises(ConnectError) as error:
                    self.decode("contact", {**contact,field:value,"private":"private-response-secret"})
                self.assertEqual(error.exception.code, "protocol")
                self.assertNotIn("private-response-secret", str(error.exception))

    def test_pairing_rejects_bad_state_expiry_and_challenge_without_echo(self):
        pairing = {"id":"pairing","owner_identity_id":"owner","peer_identity_id":"peer","expires_at":1,"state":"pending","challenge":"private-response-secret"}
        self.assertEqual(self.decode("contact_pairing", pairing)["data"], pairing)
        for field, value in [("state","verified_phone"),("expires_at",True),("expires_at",-1),("challenge",{})]:
            with self.subTest(field=field), self.assertRaises(ConnectError) as error:
                self.decode("contact_pairing", {**pairing,field:value})
            self.assertEqual(error.exception.code, "protocol")
            self.assertNotIn("private-response-secret", str(error.exception))

if __name__ == "__main__": unittest.main()
