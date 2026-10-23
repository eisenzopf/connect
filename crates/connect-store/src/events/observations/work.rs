//! Frozen work labels and anti-downgrade contracts outlive buffer pruning.
use super::*;
use crate::TaskObservationBinding;
pub(super) fn current(store: &Store, c: &Connection, route: &Route) -> Result<bool> {
    let Some(work) = &route.work else {
        return Ok(true);
    };
    let checked = (|| {
        work.check(store, c, &route.credential, &route.cid)?;
        let memory = route.memory.as_ref().ok_or_else(Error::forbidden)?;
        if !memory.contains_question(c, &work.descriptor().question_id)? {
            return Err(Error::forbidden());
        }
        Ok(())
    })();
    match checked {
        Ok(()) => Ok(true),
        Err(e) if matches!(e.code.as_str(), "forbidden" | "unauthorized" | "not-found") => {
            Ok(false)
        }
        Err(e) => Err(e),
    }
}
pub(super) fn contract(store: &Store, c: &Connection, route: &Route) -> Result<()> {
    let Some(memory) = &route.memory else {
        // A native provider context ID is reserved even after its transient
        // route is pruned. Generic registration cannot remove memory custody.
        let reserved:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM native_assistant_providers WHERE delivery_id=?1 UNION ALL SELECT 1 FROM private_provider_contexts WHERE delivery_id=?1)",[&route.correlation],|r|r.get(0)).map_err(|_|Error::storage())?;
        return if reserved || route.work.is_some() {
            Err(Error::new(
                "conflict",
                "Captured provider correlation requires its original custody",
            ))
        } else {
            Ok(())
        };
    };
    let identity = memory.identity();
    let work = route
        .work
        .as_ref()
        .map(TaskObservationBinding::contract)
        .transpose()?;
    let old:Option<(String,Option<String>,bool)>=c.query_row("SELECT memory_kind,work_json,requires_activity FROM agent_observation_contracts WHERE runtime_epoch=?1 AND delivery_id=?2",params![store.runtime_epoch(),identity.1],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|_|Error::storage())?;
    if let Some((kind, saved, activity)) = old {
        if kind != identity.0 || saved != work || (activity && route.activity.is_none()) {
            return Err(Error::new(
                "conflict",
                "Captured observation restrictions changed under the original context",
            ));
        }
        if !activity && route.activity.is_some() {
            c.execute("UPDATE agent_observation_contracts SET requires_activity=1 WHERE runtime_epoch=?1 AND delivery_id=?2",params![store.runtime_epoch(),identity.1]).map_err(|_|Error::storage())?;
        }
    } else {
        c.execute("INSERT INTO agent_observation_contracts(runtime_epoch,delivery_id,memory_kind,work_json,requires_activity) VALUES(?1,?2,?3,?4,?5)",params![store.runtime_epoch(),identity.1,identity.0,work,route.activity.is_some()]).map_err(|_|Error::storage())?;
    }
    Ok(())
}
