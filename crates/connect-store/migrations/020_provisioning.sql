-- Current Gate 6 schema; older candidate state requires explicit backup/reset.
ALTER TABLE identities ADD COLUMN revoked INTEGER NOT NULL DEFAULT 0 CHECK(revoked IN (0,1));
ALTER TABLE identities ADD COLUMN revision INTEGER NOT NULL DEFAULT 0 CHECK(revision BETWEEN 0 AND 9007199254740991);
ALTER TABLE endpoints ADD COLUMN secret_ref TEXT;
ALTER TABLE endpoints ADD COLUMN route_owner_id TEXT REFERENCES identities(id);
CREATE UNIQUE INDEX endpoints_address_claim ON endpoints(tenant_id,transport,lower(address)) WHERE verification_state!='revoked';
CREATE TRIGGER endpoint_owner_immutable BEFORE UPDATE ON endpoints
 WHEN NEW.id!=OLD.id OR NEW.identity_id!=OLD.identity_id OR NEW.tenant_id!=OLD.tenant_id
 OR NEW.device_id!=OLD.device_id OR NEW.transport!=OLD.transport OR NEW.address!=OLD.address
 OR NEW.capabilities_json!=OLD.capabilities_json OR NEW.route_owner_id IS NOT OLD.route_owner_id
 OR (NEW.secret_ref IS NOT OLD.secret_ref AND NOT(NEW.verification_state='revoked' AND NEW.secret_ref IS NULL))
 OR (OLD.verification_state='revoked' AND NEW.verification_state!='revoked')
 BEGIN SELECT RAISE(ABORT,'endpoint ownership is immutable'); END;
PRAGMA user_version=20;
