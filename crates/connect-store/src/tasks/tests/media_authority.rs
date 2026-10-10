use super::{grants, media_fixture::*};
use crate::{Scope, SipLegAction, VoiceAction};
use connect_protocol::tasks::*;

#[test]
fn cancelled_tasks_cannot_claim_queued_voice_or_sip_starts() {
    let (f, t) = call_setup();
    f.run(&invoke(&t, "start", call_start(&f, "start")))
        .unwrap();
    cancel(&f, &t);
    assert!(f.store.claim_voice_job().unwrap().is_none());
    let (f, t, s) = sip_setup();
    f.run(&invoke(&t, "start", sip_start(&f, "start", &s)))
        .unwrap();
    cancel(&f, &t);
    assert!(f.store.claim_sip_leg_job().unwrap().is_none());
}
#[test]
fn revoked_invocation_is_rechecked_after_claim_before_voice_dispatch() {
    let (f, t) = call_setup();
    let binding = grants::binding("voice", InvocationKind::Native);
    f.store
        .register_task_binding(&f.key.id, &t.id, &binding)
        .unwrap();
    let grant = f
        .store
        .issue_task_grant(
            &f.key.id,
            "issue",
            &GrantDefinition {
                task_id: t.id.clone(),
                binding: binding.clone(),
                operations: vec![TaskPermission::CallStart],
                expires_at: f.now + 120,
            },
        )
        .unwrap();
    let invocation = f
        .store
        .authenticate_task_grant(grant.expose_secret(), &binding)
        .unwrap();
    f.store
        .execute_task_invocation(&invocation, &invoke(&t, "start", call_start(&f, "start")))
        .unwrap();
    let job = f.store.claim_voice_job().unwrap().unwrap();
    f.store
        .revoke_task_grant(&f.key.id, &grant.grant.id)
        .unwrap();
    assert!(!f.store.voice_dispatch_allowed(&job).unwrap());
}
#[test]
fn cancelled_task_is_rechecked_after_sip_preparation_before_commit() {
    let (f, t, s) = sip_setup();
    f.run(&invoke(&t, "start", sip_start(&f, "start", &s)))
        .unwrap();
    let job = f.store.claim_sip_leg_job().unwrap().unwrap();
    f.store.prepare_sip_leg(&job, "prepared-only").unwrap();
    cancel(&f, &t);
    assert!(f.store.sip_origination_cancelled(&job).unwrap());
    assert!(!f.store.sip_dispatch_allowed(&job).unwrap());
}
#[test]
fn each_physical_start_can_be_admitted_only_once() {
    let (f, _) = call_setup();
    f.store
        .execute(&f.key.id, &call_start(&f, "start"))
        .unwrap();
    let job = f.store.claim_voice_job().unwrap().unwrap();
    assert!(f.store.voice_dispatch_allowed(&job).unwrap());
    assert!(!f.store.voice_dispatch_allowed(&job).unwrap());
    let (f, _, s) = sip_setup();
    f.store
        .execute(&f.key.id, &sip_start(&f, "start", &s))
        .unwrap();
    let job = f.store.claim_sip_leg_job().unwrap().unwrap();
    assert!(f.store.sip_dispatch_allowed(&job).unwrap());
    assert!(!f.store.sip_dispatch_allowed(&job).unwrap());
}
#[test]
fn blocked_task_voice_end_does_not_contaminate_a_new_ordinary_end() {
    let (f, t) = call_setup();
    let call = active_call(&f);
    let end = invoke(&t, "task-end", call_end(&f, "task-end", &call));
    f.run(&end).unwrap();
    cancel(&f, &t);
    assert!(f.store.claim_voice_job().unwrap().is_none());
    f.store
        .execute(&f.key.id, &call_end(&f, "new-end", &call))
        .unwrap();
    let job = f.store.claim_voice_job().unwrap().unwrap();
    assert_eq!(job.action, VoiceAction::End);
    assert!(f.store.voice_dispatch_allowed(&job).unwrap());
}
#[test]
fn blocked_task_sip_end_does_not_contaminate_a_new_ordinary_end() {
    let (f, t, s) = sip_setup();
    let leg = active_sip(&f, &s);
    f.run(&invoke(&t, "task-end", sip_end(&f, "task-end", &leg)))
        .unwrap();
    cancel(&f, &t);
    assert!(f.store.claim_sip_leg_job().unwrap().is_none());
    f.store
        .execute(&f.key.id, &sip_end(&f, "new-end", &leg))
        .unwrap();
    let job = f.store.claim_sip_leg_job().unwrap().unwrap();
    assert_eq!(job.action, SipLegAction::End);
    assert!(f.store.sip_dispatch_allowed(&job).unwrap());
}
#[test]
fn cached_old_end_receipts_cannot_replace_the_current_generation() {
    let (f, _) = call_setup();
    let call = active_call(&f);
    let old = call_end(&f, "old", &call);
    f.store.execute(&f.key.id, &old).unwrap();
    f.store
        .execute(&f.key.id, &call_end(&f, "new", &call))
        .unwrap();
    let before = crate::Store::inspect_snapshot(f.store.path()).unwrap();
    f.store.execute(&f.key.id, &old).unwrap();
    assert_eq!(
        crate::Store::inspect_snapshot(f.store.path()).unwrap(),
        before
    );
    let job = f.store.claim_voice_job().unwrap().unwrap();
    assert!(f.store.voice_dispatch_allowed(&job).unwrap());
}
#[test]
fn a_new_end_supersedes_a_claimed_generation_before_network_dispatch() {
    let (f, _) = call_setup();
    let call = active_call(&f);
    f.store
        .execute(&f.key.id, &call_end(&f, "old", &call))
        .unwrap();
    let old = f.store.claim_voice_job().unwrap().unwrap();
    f.store
        .execute(&f.key.id, &call_end(&f, "new", &call))
        .unwrap();
    assert!(!f.store.voice_dispatch_allowed(&old).unwrap());
    let new = f.store.claim_voice_job().unwrap().unwrap();
    assert_eq!(new.action, VoiceAction::End);
    assert!(f.store.voice_dispatch_allowed(&new).unwrap());
    assert!(!f.store.voice_dispatch_allowed(&old).unwrap());
}
#[test]
fn sip_end_uses_its_issuer_instead_of_the_dialer() {
    let (f, _, s) = sip_setup();
    let leg = active_sip(&f, &s);
    let replacement = f
        .store
        .issue(&f.owner, Scope::operator(), f.now + 3600)
        .unwrap();
    f.store
        .execute(&replacement.id, &sip_end(&f, "end", &leg))
        .unwrap();
    f.store
        .revoke(
            &f.store.authenticate(&f.key.token).unwrap(),
            &replacement.id,
        )
        .unwrap();
    assert!(f.store.claim_sip_leg_job().unwrap().is_none());
}
#[test]
fn expiry_and_membership_loss_deny_before_physical_voice_dispatch() {
    for expiry in [true, false] {
        let (f, t) = call_setup();
        f.run(&invoke(&t, "start", call_start(&f, "start")))
            .unwrap();
        let job = f.store.claim_voice_job().unwrap().unwrap();
        let c = f.store.connection().unwrap();
        if expiry {
            c.execute(
                "UPDATE tasks SET expires_at=?1 WHERE id=?2",
                rusqlite::params![f.now, t.id],
            )
            .unwrap();
        } else {
            c.execute(
                "DELETE FROM participants WHERE conversation_id=?1",
                [&f.cid],
            )
            .unwrap();
        }
        drop(c);
        assert!(!f.store.voice_dispatch_allowed(&job).unwrap());
    }
}
#[test]
fn infrastructure_failure_is_not_recorded_as_an_authorization_denial() {
    let (f, t) = call_setup();
    f.run(&invoke(&t, "start", call_start(&f, "start")))
        .unwrap();
    f.store
        .connection()
        .unwrap()
        .execute_batch("ALTER TABLE task_effects RENAME TO unavailable_effects")
        .unwrap();
    assert_eq!(
        f.store.claim_voice_job().err().unwrap().code,
        "storage-unavailable"
    );
    let state: String = f
        .store
        .connection()
        .unwrap()
        .query_row("SELECT state FROM voice_calls", [], |r| r.get(0))
        .unwrap();
    assert_eq!(state, "pending");
}
#[test]
fn missing_legacy_task_dispatch_metadata_does_not_fall_back_to_account_authority() {
    let (f, t) = call_setup();
    f.run(&invoke(&t, "start", call_start(&f, "start")))
        .unwrap();
    f.store
        .connection()
        .unwrap()
        .execute("DELETE FROM task_effect_links WHERE kind='call'", [])
        .unwrap();
    assert!(f.store.claim_voice_job().unwrap().is_none());
}
#[test]
fn newer_sip_end_supersedes_a_claimed_old_generation_without_dispatching_it() {
    let (f, _, s) = sip_setup();
    let leg = active_sip(&f, &s);
    f.store
        .execute(&f.key.id, &sip_end(&f, "old", &leg))
        .unwrap();
    let old = f.store.claim_sip_leg_job().unwrap().unwrap();
    f.store
        .execute(&f.key.id, &sip_end(&f, "new", &leg))
        .unwrap();
    assert!(!f.store.sip_dispatch_allowed(&old).unwrap());
    let new = f.store.claim_sip_leg_job().unwrap().unwrap();
    assert!(f.store.sip_dispatch_allowed(&new).unwrap());
    assert!(!f.store.sip_dispatch_allowed(&old).unwrap());
}
#[test]
fn rejected_historical_task_end_cannot_become_success_when_another_end_finishes() {
    let (f, t) = call_setup();
    let call = active_call(&f);
    f.run(&invoke(&t, "task-end", call_end(&f, "task-end", &call)))
        .unwrap();
    let actor = f.store.authenticate(&f.key.token).unwrap();
    let replacement = f
        .store
        .issue(&f.owner, Scope::operator(), f.now + 3600)
        .unwrap();
    f.store.revoke(&actor, &f.key.id).unwrap();
    assert!(f.store.claim_voice_job().unwrap().is_none());
    f.store
        .execute(&replacement.id, &call_end(&f, "fresh", &call))
        .unwrap();
    let job = f.store.claim_voice_job().unwrap().unwrap();
    assert!(f.store.voice_dispatch_allowed(&job).unwrap());
    f.store
        .finish_voice_job(&job, "ended", None, None, None)
        .unwrap();
    let effect_id: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT id FROM task_effects WHERE task_id=?1",
            [&t.id],
            |r| r.get(0),
        )
        .unwrap();
    let TaskOutcome::Effect(e) = f
        .store
        .execute_task(
            &replacement.id,
            &TaskRequest::new(TaskCommand::EffectGet {
                task_id: t.id,
                effect_id,
            }),
        )
        .unwrap()
    else {
        panic!("effect")
    };
    assert_eq!(e.state, EffectState::Failed);
}
