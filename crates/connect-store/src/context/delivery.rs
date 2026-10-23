//! Read-only captured destinations and control observations. Never infer a
//! historical generation from a mutable current Connection projection.
use super::*;
use connect_protocol::{
    ContextDeliveryDestination, ContextDeliveryLifecycle, ContextDeliveryPhase,
    ContextDeliveryProblem, ContextDeliveryPurpose,
};
use rusqlite::OptionalExtension;
pub(super) fn metadata(c: &Connection, receipt: &mut ContextDelivery) -> Result<()> {
    let row:Option<(String,String,u64,Option<String>,bool,bool,bool,bool,Option<bool>,Option<bool>,bool,String,Option<String>)>=c.query_row(
        "SELECT 'public_session',p.connection_id,p.generation,p.problem,p.creation_started,p.created_at IS NOT NULL,(p.export_started=1 OR EXISTS(SELECT 1 FROM native_assistant_initializations i WHERE i.delivery_id=p.delivery_id AND i.started=1)),p.delivered_at IS NOT NULL,p.end_started,p.end_uncertain,p.closed_at IS NOT NULL,CASE WHEN EXISTS(SELECT 1 FROM native_assistant_initializations i WHERE i.delivery_id=p.delivery_id) THEN 'initialization' ELSE 'post_creation' END,NULL FROM native_assistant_providers p WHERE p.delivery_id=?1
         UNION ALL
         SELECT 'private_consultation',p.connection_id,(SELECT json_extract(peer.value,'$.generation') FROM json_each(p.selection_json,'$.private') peer WHERE json_extract(peer.value,'$.connection_id')=p.connection_id),CASE WHEN d.state='uncertain' THEN 'unknown' ELSE NULL END,p.started,p.created_at IS NOT NULL,p.export_started,p.delivered_at IS NOT NULL,NULL,NULL,p.closed_at IS NOT NULL,'post_creation',NULL FROM private_provider_contexts p JOIN context_deliveries d ON d.id=p.delivery_id WHERE p.delivery_id=?1
         UNION ALL
         SELECT 'public_session',p.connection_id,p.generation,u.problem,0,p.created_at IS NOT NULL,u.export_started,u.delivered_at IS NOT NULL,p.end_started,p.end_uncertain,p.closed_at IS NOT NULL,'update',p.delivery_id FROM native_assistant_context_updates u JOIN native_assistant_providers p ON p.delivery_id=u.instance_delivery_id WHERE u.delivery_id=?1",
        [&receipt.id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?,r.get(10)?,r.get(11)?,r.get(12)?)),
    ).optional().map_err(|_|Error::storage())?;
    if let Some((
        purpose,
        connection_id,
        generation,
        problem,
        creation_admitted,
        creation_confirmed,
        export_admitted,
        delivery_confirmed,
        cleanup_admitted,
        cleanup_uncertain,
        closed,
        phase,
        parent_delivery_id,
    )) = row
    {
        let purpose = match purpose.as_str() {
            "public_session" => ContextDeliveryPurpose::PublicSession,
            "private_consultation" => ContextDeliveryPurpose::PrivateConsultation,
            _ => return Err(Error::storage()),
        };
        if generation == 0 || generation > 9_007_199_254_740_991 {
            return Err(Error::storage());
        }
        receipt.destination = Some(ContextDeliveryDestination {
            connection_id,
            generation,
            purpose,
        });
        receipt.problem = problem
            .map(|problem| match problem.as_str() {
                "rejected" => Ok(ContextDeliveryProblem::Rejected),
                "truncated" => Ok(ContextDeliveryProblem::Truncated),
                "incomplete" => Ok(ContextDeliveryProblem::Incomplete),
                "unknown" => Ok(ContextDeliveryProblem::Unknown),
                _ => Err(Error::storage()),
            })
            .transpose()?;
        receipt.lifecycle = Some(ContextDeliveryLifecycle {
            phase: match phase.as_str() {
                "initialization" => ContextDeliveryPhase::Initialization,
                "post_creation" => ContextDeliveryPhase::PostCreation,
                "update" => ContextDeliveryPhase::Update,
                _ => return Err(Error::storage()),
            },
            parent_delivery_id,
            creation_admitted,
            creation_confirmed,
            export_admitted,
            delivery_confirmed,
            cleanup_admitted,
            cleanup_uncertain,
            closed,
        });
    }
    Ok(())
}
