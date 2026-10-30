-- Private transport negotiation history. SDP/credentials must never enter the
-- shared stream projection or operation/event result. The native control slot
-- already reserves each parent's one outstanding original attempt.
CREATE TABLE native_webrtc_ice_state (
 parent_record_id TEXT PRIMARY KEY REFERENCES native_connection_lifetimes(operation_record_id),
 connection_id TEXT NOT NULL REFERENCES connections(id),
 connection_generation INTEGER NOT NULL CHECK(connection_generation BETWEEN 1 AND 9007199254740991),
 ice_generation INTEGER NOT NULL CHECK(ice_generation BETWEEN 0 AND 9007199254740991),
 remote_offer TEXT NOT NULL CHECK(length(CAST(remote_offer AS BLOB)) BETWEEN 5 AND 524288 AND substr(remote_offer,1,5)=char(118,61,48,13,10) AND instr(remote_offer,char(0))=0),
 last_record_id TEXT UNIQUE REFERENCES native_stream_operations(operation_record_id),
 CHECK((ice_generation=0 AND last_record_id IS NULL) OR (ice_generation>0 AND last_record_id IS NOT NULL))
);
CREATE TABLE native_webrtc_ice_operations (
 operation_record_id TEXT PRIMARY KEY REFERENCES native_stream_operations(operation_record_id),
 parent_record_id TEXT NOT NULL REFERENCES native_webrtc_ice_state(parent_record_id),
 expected_ice_generation INTEGER NOT NULL CHECK(expected_ice_generation BETWEEN 0 AND 9007199254740990),
 previous_remote_offer TEXT NOT NULL CHECK(length(CAST(previous_remote_offer AS BLOB)) BETWEEN 5 AND 524288 AND substr(previous_remote_offer,1,5)=char(118,61,48,13,10) AND instr(previous_remote_offer,char(0))=0),
 proposed_remote_offer TEXT NOT NULL CHECK(length(CAST(proposed_remote_offer AS BLOB)) BETWEEN 5 AND 524288 AND substr(proposed_remote_offer,1,5)=char(118,61,48,13,10) AND instr(proposed_remote_offer,char(0))=0),
 previous_local_answer TEXT CHECK(previous_local_answer IS NULL OR (length(CAST(previous_local_answer AS BLOB)) BETWEEN 5 AND 524288 AND substr(previous_local_answer,1,5)=char(118,61,48,13,10) AND instr(previous_local_answer,char(0))=0)),
 answer_sdp TEXT CHECK(answer_sdp IS NULL OR (length(CAST(answer_sdp AS BLOB)) BETWEEN 5 AND 524288 AND substr(answer_sdp,1,5)=char(118,61,48,13,10) AND instr(answer_sdp,char(0))=0)),
 committed INTEGER NOT NULL DEFAULT 0 CHECK(committed IN (0,1)),
 CHECK(answer_sdp IS NULL OR previous_local_answer IS NOT NULL),
 CHECK(committed=0 OR answer_sdp IS NOT NULL)
);
CREATE TRIGGER native_webrtc_ice_initial_binding BEFORE INSERT ON native_webrtc_ice_state
 WHEN NOT EXISTS(SELECT 1 FROM native_admission_sources n
 JOIN native_connection_lifetimes l ON l.operation_record_id=n.operation_record_id
 JOIN connections c ON c.id=n.connection_id
 JOIN endpoints e ON e.id=c.endpoint_id
 JOIN connection_offers o ON o.connection_id=c.id AND o.operation_id=n.operation_id
 WHERE n.operation_record_id=NEW.parent_record_id AND n.state='completed'
 AND c.id=NEW.connection_id AND c.generation=NEW.connection_generation
 AND l.generation=c.generation AND l.retired=0 AND l.runtime_epoch=c.observed_runtime_id
 AND c.state IN ('ready','active') AND e.transport='webrtc'
 AND json_extract(o.offer_json,'$.substrate')='websocket+webrtc'
 AND json_extract(o.offer_json,'$.substrate_setup.sdp')=NEW.remote_offer
 AND NEW.ice_generation=0 AND NEW.last_record_id IS NULL)
 BEGIN SELECT RAISE(ABORT,'ICE original source binding mismatch'); END;
CREATE TRIGGER native_webrtc_ice_operation_binding BEFORE INSERT ON native_webrtc_ice_operations
 WHEN NOT EXISTS(SELECT 1 FROM native_stream_operations n
 JOIN operations o ON o.record_id=n.operation_record_id
 JOIN native_webrtc_ice_state s ON s.parent_record_id=n.parent_record_id
 WHERE n.operation_record_id=NEW.operation_record_id AND n.parent_record_id=NEW.parent_record_id
 AND n.generation=s.connection_generation AND o.connection_id=s.connection_id
 AND n.closed=0 AND n.effect_started=0 AND o.state='accepted' AND o.command_type='connection.update'
 AND json_extract(o.intent_json,'$.payload.action')='ice-restart'
 AND json_extract(o.intent_json,'$.payload.details.expected_ice_generation')=NEW.expected_ice_generation
 AND json_extract(o.intent_json,'$.payload.details.setup.kind')='websocket+webrtc'
 AND json_extract(o.intent_json,'$.payload.details.setup.sdp')=NEW.proposed_remote_offer
 AND s.ice_generation=NEW.expected_ice_generation AND s.remote_offer=NEW.previous_remote_offer
 AND NEW.previous_local_answer IS NULL AND NEW.answer_sdp IS NULL AND NEW.committed=0)
 BEGIN SELECT RAISE(ABORT,'ICE original operation binding mismatch'); END;
CREATE TRIGGER native_webrtc_ice_operation_immutable BEFORE UPDATE ON native_webrtc_ice_operations
 WHEN NEW.operation_record_id!=OLD.operation_record_id OR NEW.parent_record_id!=OLD.parent_record_id
 OR NEW.expected_ice_generation!=OLD.expected_ice_generation
 OR NEW.previous_remote_offer!=OLD.previous_remote_offer OR NEW.proposed_remote_offer!=OLD.proposed_remote_offer
 OR (OLD.previous_local_answer IS NOT NULL AND NEW.previous_local_answer IS NOT OLD.previous_local_answer)
 OR (OLD.answer_sdp IS NOT NULL AND NEW.answer_sdp IS NOT OLD.answer_sdp)
 OR NEW.committed<OLD.committed
 OR NOT EXISTS(SELECT 1 FROM native_stream_operations n JOIN operation_effects e ON e.id=n.effect_id
  WHERE n.operation_record_id=OLD.operation_record_id AND n.effect_started=1
  AND e.operation_record_id=n.operation_record_id AND e.provider='native-stream'
  AND (NEW.committed=0 OR e.state='confirmed'))
 BEGIN SELECT RAISE(ABORT,'ICE original attempt is immutable'); END;
CREATE TRIGGER native_webrtc_ice_generation_commit BEFORE UPDATE ON native_webrtc_ice_state
 WHEN NEW.parent_record_id!=OLD.parent_record_id OR NEW.connection_id!=OLD.connection_id
 OR NEW.connection_generation!=OLD.connection_generation OR NEW.ice_generation!=OLD.ice_generation+1
 OR NOT EXISTS(SELECT 1 FROM native_webrtc_ice_operations i JOIN native_stream_operations n ON n.operation_record_id=i.operation_record_id
  JOIN operation_effects e ON e.id=n.effect_id
  WHERE i.operation_record_id=NEW.last_record_id AND i.parent_record_id=OLD.parent_record_id
  AND i.committed=1 AND i.expected_ice_generation=OLD.ice_generation
  AND i.previous_remote_offer=OLD.remote_offer AND i.proposed_remote_offer=NEW.remote_offer
  AND n.generation=OLD.connection_generation AND e.operation_record_id=i.operation_record_id AND e.state='confirmed')
 BEGIN SELECT RAISE(ABORT,'ICE generation requires original confirmed outcome'); END;
CREATE TRIGGER native_webrtc_ice_state_retained BEFORE DELETE ON native_webrtc_ice_state
 BEGIN SELECT RAISE(ABORT,'ICE original generation remains reserved'); END;
CREATE TRIGGER native_webrtc_ice_operation_retained BEFORE DELETE ON native_webrtc_ice_operations
 BEGIN SELECT RAISE(ABORT,'ICE original operation remains reserved'); END;
PRAGMA user_version=116;
