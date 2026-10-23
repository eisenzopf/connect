import type { ContextRecord } from "./record_context.js";
import type { Operation, Outcome } from "./types.js";

/** Application policy carried by Connect control over UCTP, not a new transport. */
export const TASK_PROFILE = "connect-task/1" as const;
export type TaskState =
    | "ready"
    | "running"
    | "waiting"
    | "completed"
    | "cancelled"
    | "failed"
    | "expired";
export type EffectState =
    | "prepared"
    | "dispatched"
    | "succeeded"
    | "failed"
    | "uncertain"
    | "cancelled";
export type TaskPermission =
    | "resolve"
    | "capabilities"
    | "send"
    | "history"
    | "context_view"
    | "record_get"
    | "context_deliveries"
    | "inbox"
    | "request_status"
    | "artifact_begin"
    | "artifact_append"
    | "artifact_finish"
    | "artifact_get"
    | "artifact_delete"
    | "session_invite"
    | "session_accept"
    | "session_reject"
    | "session_cancel"
    | "session_end"
    | "session_get"
    | "call_start"
    | "call_status"
    | "call_end"
    | "sip_dial"
    | "sip_status"
    | "sip_end"
    | "task_cancel";
export interface TaskSmsEnrollment {
    source: string;
    grant_id: string;
    purpose: string;
}
export interface TaskScope {
    operations: TaskPermission[];
    recipients: string[];
    sms_enrollments?: TaskSmsEnrollment[];
}
export interface TaskDefinition {
    conversation_id: string;
    scope: TaskScope;
    expires_at: number;
    initiating_event_id?: string | null;
}
export interface TaskRecord {
    id: string;
    owner_id: string;
    conversation_id: string;
    scope: TaskScope;
    state: TaskState;
    revision: number;
    expires_at: number;
    initiating_event_id: string | null;
    created_at: number;
    updated_at: number;
    reason: string | null;
}
/** A coordination lease is not permission to repeat a possibly dispatched effect. */
export interface TaskStepClaim {
    task_id: string;
    step_id: string;
    worker_id: string;
    revision: number;
    expires_at: number;
}
export interface TaskEffect {
    id: string;
    task_id: string;
    step_id: string;
    request_id: string;
    state: EffectState;
    outcome: Outcome | null;
    error?: { code: string; message: string } | null;
}
export interface TaskReply {
    event_id: string;
    conversation_id: string;
    task_id: string | null;
    reply_to: string | null;
    assigned_by: string | null;
    consumed: boolean;
}
export type TaskEventData =
    | {
          kind: "state";
          data: { state: TaskState; revision: number; reason: string | null };
      }
    | { kind: "reply"; data: TaskReply }
    | { kind: "effect"; data: TaskEffect }
    | { kind: "claim"; data: TaskStepClaim };
export interface TaskEvent {
    id: string;
    task_id: string;
    actor_id: string;
    sequence: number;
    created_at: number;
    data: TaskEventData;
}
/** Retained context is data. It cannot expand task authority or choose a route. */
export interface TaskSource {
    source_id: string;
    kind: string;
    actor_id: string;
    created_at: number | null;
    provenance: string;
    state: "available" | "deleted" | "expired" | "unavailable";
    body: string | null;
    record?: ContextRecord;
}
export type TaskToolResult =
    | { status: "completed"; task_id: string; outcome: Outcome }
    | { status: "pending"; task_id: string; effect_id: string | null }
    | { status: "uncertain"; task_id: string; effect_id: string };
export type TaskOutcome =
    | { kind: "record"; data: TaskRecord }
    | { kind: "replies"; data: { items: TaskReply[]; next: string | null } }
    | { kind: "inbox"; data: { events: TaskEvent[]; next: string | null } }
    | {
          kind: "recall";
          data: { sources: TaskSource[]; next: string | null; omitted: number };
      }
    | { kind: "effect"; data: TaskEffect }
    | { kind: "claim"; data: TaskStepClaim | null }
    | { kind: "tool"; data: TaskToolResult }
    | {
          kind: "reply_accepted";
          data: { request_id: string; message_id: string; reply: TaskReply };
      };
/** Inputs contain neither an owner selector nor a secret/invocation binding. */
export type TaskCommand =
    | { action: "create"; request_id: string; definition: TaskDefinition }
    | { action: "get"; task_id: string }
    | {
          action: "cancel";
          request_id: string;
          task_id: string;
          reason?: string | null;
      }
    | {
          action: "transition";
          request_id: string;
          task_id: string;
          expected_revision: number;
          state: TaskState;
          reason?: string | null;
      }
    | {
          action: "replies";
          conversation_id: string;
          after?: string | null;
          limit: number;
      }
    | {
          action: "assign_reply";
          request_id: string;
          task_id: string;
          event_id: string;
          reply_to?: string | null;
      }
    | {
          action: "reply";
          request_id: string;
          conversation_id: string;
          reply_to: string;
          body: string;
      }
    | {
          action: "inbox" | "wait";
          task_id: string;
          after?: string | null;
          limit: number;
          wait_ms: number;
      }
    | {
          action: "consume_reply";
          request_id: string;
          task_id: string;
          event_id: string;
          expected_revision: number;
      }
    | {
          action: "invoke";
          request_id: string;
          task_id: string;
          step_id: string;
          operation: Extract<Operation, { op: TaskPermission }>;
          reply_to?: string | null;
          wait_for_reply?: boolean;
      }
    | {
          action: "recall";
          task_id: string;
          after?: string | null;
          limit: number;
      }
    | { action: "effect_get"; task_id: string; effect_id: string }
    | {
          action: "claim";
          request_id: string;
          task_id: string;
          step_id: string;
          worker_id: string;
          expected_revision?: number | null;
      }
    | { action: "claim_get"; task_id: string; step_id: string }
    | {
          action: "release_claim";
          request_id: string;
          task_id: string;
          step_id: string;
          worker_id: string;
          expected_revision: number;
      };
