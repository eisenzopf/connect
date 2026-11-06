import assert from 'node:assert/strict';

/** One original create recovery before any media ownership. All opens use the
 * unchanged enrolled configuration; recovery never submits a mutation. */
export async function nativeControlRecovery({clients,configuration,configDigest,openClient,helpers,helperExists,
    complete,history,originalReference,conversationId,expectedHistory,expectedParticipants,options,observe,check,stage,report}){
    Object.assign(report,{format:'connect/native-independent-control-recovery/1',passed:false,mutation_replayed:false,
        new_helper_created:false,original_create_reference_preserved:false});
    const originalOwner=clients.get('participant-1'),reserve=clients.get('replacement-1');
    const originalReferenceJson=JSON.stringify(originalReference),originalReferenceValue=JSON.parse(originalReferenceJson);
    report.original_creator_config_sha256=configuration['participant-1'].sha256;
    assert.ok(originalOwner&&reserve,'Both original enrolled owners are required');
    const boundedFailure=(error,at)=>({stage:at,name:error.name,kind:error.kind??null,certainty:error.certainty??null});
    let currentStage='reserve-original-join',recovery,recoveryJoined=false,creatorJoined=false,reserveJoined=false,originalFailure,cleanupFailure;
    const enter=value=>{currentStage=value;stage(value);};
    async function closeOwned(role,owner,expectedBefore,expectedAfter){
        const before=await helpers();assert.equal(before.size,expectedBefore,'Exact original live helper set before close');
        await owner.close({timeoutMs:5000});
        const after=await helpers(),removed=[...before].filter(pid=>!after.has(pid));
        assert.equal(after.size,expectedAfter,'Exact original live helper set after close');assert.equal(removed.length,1);
        assert.equal(await helperExists(removed[0]),false,'Original physical helper must actually exit');
        assert.equal(clients.get(role),owner,'Close must retain its exact original client');clients.delete(role);
        return removed[0];
    }
    try{
        enter('reserve-original-join');await closeOwned('replacement-1',reserve,5,4);reserveJoined=true;
        report.initial_helper_joined_before_workload=true;
        report.original_device_config_sha256=configuration['replacement-1'].sha256;
        check('reserved-device-initial-helper-joins-before-sixty-second-workload');
        enter('creator-original-join');await closeOwned('participant-1',originalOwner,4,3);creatorJoined=true;
        report.original_creator_helper_joined=true;
        enter('recovery-original-config');
        assert.equal(await configDigest(configuration['replacement-1'].path),configuration['replacement-1'].sha256);
        const before=await helpers();assert.equal(before.size,3);
        recovery=openClient(configuration['replacement-1'].path);clients.set('replacement-1',recovery);
        enter('recovery-current-authority');assert.equal((await recovery.capabilities.refresh(options)).selectedTransport,'quic');
        const opened=await helpers(),added=[...opened].filter(pid=>!before.has(pid));
        assert.equal(opened.size,4);assert.equal(added.length,1);assert.equal(await helperExists(added[0]),true);
        report.new_helper_created=true;report.current_recovery_authority_verified=true;
        observe('independent-recovery-helper',opened);
        check('independent-device-explicitly-activates-original-config-after-original-sdk-owner-joins');
        enter('recovery-original-create-reference');
        const restoredOperation=recovery.operations.restore(originalReference,recovery.operationKinds.conversation_create);
        assert.deepEqual(restoredOperation.referenceJson(),originalReferenceValue);
        assert.equal(JSON.stringify(originalReference),originalReferenceJson);report.original_create_reference_preserved=true;
        const conversation=await complete(restoredOperation);assert.equal(conversation.id,conversationId);
        enter('recovery-current-membership');
        const snapshot=await conversation.refresh(options),active=snapshot.participants.filter(p=>p.active);
        assert.equal(active.length,4);assert.deepEqual(active.map(p=>p.id).sort(),[...expectedParticipants].sort());
        report.current_membership_verified=true;
        enter('recovery-original-history-audiences');assert.deepEqual(await history(recovery),[...expectedHistory].sort());
        report.original_history_audiences_verified=true;
        check('independent-device-recovers-original-create-and-history-after-original-helper-close');
    }catch(error){originalFailure=error;report.failure=boundedFailure(error,currentStage);}
    finally{
        if(recovery){
            try{enter('recovery-original-helper-join');await closeOwned('replacement-1',recovery,4,3);recoveryJoined=true;
                assert.equal(await configDigest(configuration['replacement-1'].path),configuration['replacement-1'].sha256);
                report.recovery_helper_joined_before_workload=true;
            }catch(error){cleanupFailure=error;report.cleanup_failure=boundedFailure(error,currentStage);}
        }
    }
    // A failed recovery proof remains failed, but it need not erase a later
    // independent media observation when every old owner has actually joined.
    if(!(reserveJoined&&creatorJoined&&(!recovery||recoveryJoined)))
        throw originalFailure??cleanupFailure??Error('Original recovery owners must join before media');
    enter('creator-original-config-reactivation');
    assert.equal(await configDigest(configuration['participant-1'].path),configuration['participant-1'].sha256);
    const before=await helpers();assert.equal(before.size,3);
    const creator=openClient(configuration['participant-1'].path);clients.set('participant-1',creator);
    assert.equal((await creator.capabilities.refresh(options)).selectedTransport,'quic');
    const live=await helpers(),added=[...live].filter(pid=>!before.has(pid));
    assert.equal(live.size,4);assert.equal(added.length,1);assert.equal(await helperExists(added[0]),true);
    assert.equal(await configDigest(configuration['participant-1'].path),configuration['participant-1'].sha256);
    report.creator_reactivated_on_original_config=true;report.creator_current_authority_verified=true;
    report.active_helpers_before_session_workload=live.size;observe('creator-reactivated-before-session-workload',live);
    report.passed=!report.failure&&!report.cleanup_failure&&report.new_helper_created===true
        &&report.original_create_reference_preserved===true&&report.current_membership_verified===true
        &&report.original_history_audiences_verified===true&&report.recovery_helper_joined_before_workload===true;
    return creator;
}
