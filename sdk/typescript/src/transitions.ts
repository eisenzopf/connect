/** Durable intent; admission does not prove media readiness or completion. */
export type OperationState =
    | "accepted"
    | "preparing"
    | "ready"
    | "committing"
    | "completed"
    | "cancelled"
    | "failed"
    | "interrupted"
    | "unknown";
export type TransitionIntent =
    | {
          type: "session.consult.return";
          consultation_record_id: string;
          retained_connid: string;
      }
    | {
          type: "session.consult";
          retained_connid: string;
          private_participants: string[];
          hold_treatment?: "silence" | "configured-media";
      }
    | {
          type: "session.handoff";
          source_participant: string;
          target_participant: string;
          retained_connid: string;
          candidate_connid?: string | null;
          outgoing: "leave" | "observe";
          role_scope?: "conversation" | "session";
          expected_conversation_revision?: number | null;
      }
    | {
          type: "connection.replace";
          endpoint_id?: string | null;
          candidate_connid?: string | null;
          retained_connid: string;
      };
export interface TransitionRequest {
    operation_id: string;
    conversation_id: string;
    session_id: string;
    connection_id?: string | null;
    expected_revision: number;
    expires_at: number;
    intent: TransitionIntent;
}
export type OperationAction =
    | { action: "get"; operation_id: string }
    | {
          action: "cancel";
          operation_id: string;
          target_operation_id: string;
          reason?: string | null;
      }
    | {
          action: "reconcile";
          operation_id: string;
          target_operation_id: string;
          outcome: "completed" | "cancelled" | "failed";
          evidence_id: string;
      };
export interface OperationEffect {
    id: string;
    provider: string;
    correlation_key: string;
    idempotency: "none" | "provider-key" | "status-query";
    state:
        | "prepared"
        | "submitting"
        | "confirmed"
        | "failed"
        | "cancelled"
        | "unknown";
    attempt: number;
    compensates_effect_id: string | null;
    submission_generation: number;
}
export interface OperationRecord {
    operation_id: string;
    record_id: string;
    command_type: string;
    conversation_id: string | null;
    session_id: string | null;
    connection_id: string | null;
    expected_revision: number | null;
    state: OperationState;
    revision: number;
    expires_at: number | null;
    created_at: number;
    updated_at: number;
    cancel_requested: boolean;
    result: Record<string, unknown> | null;
    problem?: Record<string, unknown>;
    effects: OperationEffect[];
    causal_event_ids: string[];
    events_omitted: number;
}
