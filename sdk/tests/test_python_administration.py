"""Explicit administrative owner over synthetic Rust normalization/profile2 replies."""
import asyncio
from dataclasses import FrozenInstanceError
import json
import os
import time

from connect_sdk import (
    AdministrativeClient, Cancellation, Client, MutationOptions, Operation,
    OperationId, RequestOptions, SdkCancelledError, SdkError, TrustedProvisioningCommand,
)
from _python_sdk_fixture import OPTIONS, ROOT, PythonSdkFixture


def draft():
    return {'type':'identity','operation_id':'original','alias':' local ','kind':'human','display_name':'private backend label'}


def saved(command='identity.provision',id='original'):
    return {'schema':'connect-operation-reference/1','operation_id':id,'command':command,'scope':{}}


def mutation(id='original',options=OPTIONS,extensions=None):
    return MutationOptions(OperationId(id),options,extensions)


class TypedPythonAdministration(PythonSdkFixture):
    def administrator(self, config=None):
        self.client(config)
        _,directory=self.clients[-1]
        executable=directory/'connect.mjs'
        executable.write_text('#!/usr/bin/env node\nprocess.env.CONNECT_SDK_FIXTURE_CONFIG='+json.dumps(str(directory/'config.json'))+';\nawait import('+json.dumps((ROOT/'sdk/tests/fixtures/sdk_helper.mjs').as_uri())+');\n')
        executable.chmod(0o700)
        client=AdministrativeClient(directory/'config.json',executable)
        self.clients[-1]=(client,directory)
        return client

    def validations(self,index=-1):
        path=self.clients[index][1]/'admin-validation.jsonl'
        return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []

    async def asyncTearDown(self):
        try:
            for _,directory in self.clients:
                path=directory/'admin-validation.jsonl'
                for value in ([json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []):
                    with self.assertRaises(ProcessLookupError):os.kill(value['pid'],0)
        finally:
            await super().asyncTearDown()

    async def assert_error(self,kind,certainty,action,id=None):
        with self.assertRaises(SdkError) as error:await action
        self.assertEqual((error.exception.kind,error.exception.certainty),(kind,certainty))
        if id:self.assertEqual(error.exception.operation.operation_id,id)
        self.assertNotIn('private',str(error.exception))

    async def test_explicit_admin_profile_normalized_immutable_command_and_json_completion(self):
        client=self.administrator();value=draft();command=client.validate_provisioning(value)
        value['alias']='changed';value['display_name']='changed'
        self.assertEqual((command.value['alias'],command.value['display_name'],command.operation_id),('local','private backend label','original'))
        with self.assertRaises(TypeError):command.value['alias']='forged'
        with self.assertRaises(AttributeError):command.value={}
        exported=command.reference_json();exported['operation_id']='changed'
        self.assertEqual(command.reference_json(),saved())
        exported_command=command.to_json();exported_command['display_name']='changed'
        self.assertEqual(command.value['display_name'],'private backend label')
        self.assertIsNone(client._helper._process);self.assertEqual(self.calls(),[])
        self.assertEqual(self.validations()[0]['argv'],['sdk-validate-provisioning'])
        operation=await client.provision(command,mutation());self.assertIsInstance(operation,Operation)
        result=await operation.wait(OPTIONS)
        self.assertEqual(result.value,{'kind':'identity','identity_id':'created','created':True})
        with self.assertRaises(TypeError):result.value['identity_id']='foreign'
        request=self.calls()[0]
        self.assertEqual(request['profile'],'connect-sdk-administration/2')
        self.assertEqual(request['request']['action']['command']['alias'],'local')
        self.assertEqual(request['request']['action']['type'],'administrative_provision')
        self.assertEqual(json.loads((self.clients[-1][1]/'argv.json').read_text())[0],'sdk-admin-stdio')
        for item in [client,command,operation,operation.reference,result]:
            self.assertNotIn('private backend label',repr(item));self.assertNotIn(str(self.directory),repr(item))

    async def test_admin_operation_original_record_readiness_recovery_and_separate_decisions(self):
        client=self.administrator();operation=await client.provision(client.validate_provisioning(draft()),mutation())
        first=await operation.status(OPTIONS);self.assertEqual(first.key.command,'identity.provision')
        self.assertEqual((await operation.wait_until_ready(OPTIONS)).state,'completed')
        before=len(self.calls());restored=client.operations.restore(operation.reference_json());recovered=client.operations.recover(operation.reference)
        self.assertEqual(len(self.calls()),before)
        self.assertEqual((await restored.status(OPTIONS)).record_id,first.record_id)
        self.assertEqual((await recovered.wait(OPTIONS)).snapshot.record_id,first.record_id)
        cancel=await operation.cancel(mutation('cancel'));receipt=(await cancel.wait(OPTIONS)).value
        self.assertEqual((receipt.target,receipt.decision.operation_id),('original','cancel'))
        reconcile=await operation.reconcile('completed','evidence',mutation('reconcile'))
        self.assertEqual((await reconcile.wait(OPTIONS)).value.evidence_id,'evidence')
        self.assertEqual((await client.operations.recover(cancel.reference).status(OPTIONS)).key.command,'operation.cancel')
        self.assertEqual((await client.operations.restore(cancel.reference_json()).wait(OPTIONS)).value.target,'original')
        self.assertTrue(any(v['argv']==['sdk-validate-provisioning','--reference'] for v in self.validations()))
        self.assertEqual(sum(v['request']['action']['type']=='administrative_provision' for v in self.calls()),1)
        self.assertTrue(all(v['profile']=='connect-sdk-administration/2' for v in self.calls()))

    async def test_participant_and_administrative_facades_and_typed_recovery_remain_distinct(self):
        admin=self.administrator();participant=self.client()
        self.assertFalse(hasattr(admin,'conversations'));self.assertFalse(hasattr(admin,'raw'))
        self.assertFalse(hasattr(participant,'validate_provisioning'));self.assertFalse(hasattr(participant,'provision'))
        original=admin.operations.restore(saved())
        with self.assertRaises(SdkError):participant.operations.recover(original.reference)
        participant_ref=participant.operations.restore(saved('operation.cancel'),participant.operation_kinds.operation_cancel).reference
        with self.assertRaises(SdkError):admin.operations.recover(participant_ref)
        with self.assertRaises(SdkError):admin.operations.recover({'operation_id':'original'})
        self.assertIsNone(admin._helper._process);self.assertIsNone(participant._helper._process)

    async def test_forged_commands_wrong_original_id_extensions_and_precancel_are_local(self):
        client=self.administrator();command=client.validate_provisioning(draft())
        for forged in [None,{}, {'value':command.value},object.__new__(TrustedProvisioningCommand)]:
            await self.assert_error('validation','not-submitted',client.provision(forged,mutation()))
        with self.assertRaises(SdkError):TrustedProvisioningCommand()
        await self.assert_error('validation','not-submitted',client.provision(command,mutation('different')),'different')
        await self.assert_error('validation','not-submitted',client.provision(command,{}))
        await self.assert_error('unsupported','not-submitted',client.provision(command,mutation(extensions={'extra':True})),'original')
        signal=Cancellation();signal.cancel()
        await self.assert_error('cancelled','not-submitted',client.provision(command,mutation(options=RequestOptions(1500,signal))),'original')
        self.assertEqual(self.calls(),[]);self.assertIsNone(client._helper._process)

    async def test_invalid_commands_and_scoped_or_continued_saved_metadata_grant_nothing(self):
        client=self.administrator()
        for value in [None,{}, {**draft(),'type':'unknown'},{**draft(),'operation_id':'bad!'}]:
            with self.assertRaises(SdkError) as observed:client.validate_provisioning(value)
            self.assertEqual((observed.exception.kind,observed.exception.certainty),('validation','not-submitted'))
        for value in [{**saved(),'scope':{'conversation_id':'room'}},{**saved(),'continuation':{}},
                      {**saved(),'command':'message.send'},{**saved(),'operation_id':'bad!'}]:
            with self.assertRaises(SdkError) as observed:client.operations.restore(value)
            self.assertEqual((observed.exception.kind,observed.exception.certainty),('validation','not-submitted'))
        self.assertEqual(self.calls(),[]);self.assertIsNone(client._helper._process)

    async def test_corrupt_local_validator_descriptor_fences_before_managed_submission(self):
        for mode in ['profile','scope','kind-array','foreign-id']:
            client=self.administrator({'adminValidator':mode})
            with self.assertRaises(SdkError) as observed:client.validate_provisioning(draft())
            self.assertEqual((observed.exception.kind,observed.exception.certainty),('protocol','not-submitted'))
            self.assertEqual(self.calls(),[]);self.assertTrue(client._helper._closed)
            await client.close(OPTIONS)

    async def test_admin_response_corruption_retains_submitted_mutation_uncertainty_and_no_retry(self):
        source=self.administrator();command=source.validate_provisioning(draft())
        for mode in ['profile','wrong-id','wrong-reference','duplicate','extra','error-kind-array']:
            client=self.administrator({'mode':mode})
            await self.assert_error('protocol','uncertain',client.provision(command,mutation()),'original')
            await self.assert_error('configuration','not-submitted',client.provision(command,mutation()),'original')
            self.assertEqual(len(self.calls()),1);await client.close(OPTIONS)

    async def test_admin_status_scope_state_and_stable_record_fences_are_read_only(self):
        for mode in ['wrong-scope','state-array','record-change']:
            client=self.administrator({'mode':mode});operation=client.operations.restore(saved())
            if mode=='record-change':await operation.status(OPTIONS)
            await self.assert_error('protocol','read-only',operation.status(OPTIONS),'original')
            await client.close(OPTIONS)

    async def test_admin_sent_deadline_keeps_original_id_and_never_reloads_or_retries(self):
        client=self.administrator({'mode':'partial'});command=client.validate_provisioning(draft())
        await self.assert_error('deadline','uncertain',client.provision(command,mutation(options=RequestOptions(50))),'original')
        await self.assert_error('configuration','not-submitted',client.provision(command,mutation()),'original')
        self.assertEqual(len(self.calls()),1)

    async def test_full_64_kib_normalized_command_fits_owned_wrapper_and_oversize_stays_local(self):
        client=self.administrator();value=draft();value['alias']='local';value['display_name']=''
        overhead=len(json.dumps(value,separators=(',',':'),ensure_ascii=False,sort_keys=True).encode())
        value['display_name']='x'*(65536-overhead)
        command=client.validate_provisioning(value)
        self.assertEqual(len(json.dumps(command.to_json(),separators=(',',':'),ensure_ascii=False,sort_keys=True).encode()),65536)
        self.assertEqual((await client.provision(command,mutation())).reference.operation_id,'original')
        before=len(self.validations());value['display_name']+='x'
        with self.assertRaises(SdkError) as observed:client.validate_provisioning(value)
        self.assertEqual(observed.exception.kind,'validation');self.assertEqual(len(self.validations()),before)
        self.assertEqual(len(self.calls()),1)

    async def test_admin_validator_timeout_is_bounded_and_original_child_joined(self):
        client=self.administrator({'adminValidator':'hang'});start=time.monotonic()
        with self.assertRaises(SdkError) as observed:client.validate_provisioning(draft())
        self.assertEqual((observed.exception.kind,observed.exception.certainty),('configuration','not-submitted'))
        self.assertLess(time.monotonic()-start,3);self.assertEqual(self.calls(),[])

    async def test_admin_startup_failure_remains_unsent_for_mutations_and_read_only_for_status(self):
        source=self.administrator();command=source.validate_provisioning(draft())
        for kind in ['authentication','configuration','trust','deadline']:
            client=self.administrator({'mode':'startup','kind':kind,'certainty':'read-only'})
            await self.assert_error(kind,'not-submitted',client.provision(command,mutation()),'original')
            operation=client.operations.restore(saved())
            await self.assert_error(kind,'read-only',operation.status(OPTIONS),'original')
            self.assertEqual(len(self.calls()),2)

    async def test_admin_task_cancel_retains_original_intent_and_joins_one_owned_helper(self):
        client=self.administrator({'mode':'partial','signalDrainMs':60});command=client.validate_provisioning(draft())
        waiter=asyncio.create_task(client.provision(command,mutation()));await self.sent();waiter.cancel('private cancellation')
        with self.assertRaises(SdkCancelledError) as observed:await waiter
        self.assertEqual((observed.exception.kind,observed.exception.certainty,observed.exception.operation.operation_id),('cancelled','uncertain','original'))
        self.assertTrue(waiter.cancelled());await client.close(OPTIONS)
        directory=self.clients[-1][1]
        self.assertEqual(json.loads((directory/'drained.json').read_text()),{'drained':True})
        self.assertEqual(json.loads((directory/'exit.json').read_text()),{'code':0})
        self.assertEqual(len(self.calls()),1)

    async def test_admin_decisions_keep_distinct_original_target_and_reject_foreign_receipt(self):
        client=self.administrator({'mode':'decision-target'});operation=client.operations.restore(saved())
        with self.assertRaises(SdkError):await operation.cancel(mutation())
        decision=await operation.cancel(mutation('cancel'))
        await self.assert_error('protocol','read-only',decision.wait(OPTIONS),'cancel')

    async def test_saved_command_and_typed_reference_recover_under_fresh_explicit_owner_without_submission(self):
        first=self.administrator();command=first.validate_provisioning(draft());operation=await first.provision(command,mutation())
        reference=operation.reference;await first.close(OPTIONS)
        second=self.administrator();recovered=second.operations.recover(reference)
        self.assertEqual(self.calls(),[]);self.assertEqual(self.validations(),[])
        self.assertEqual(recovered.reference_json(),command.reference_json())
        self.assertEqual((await recovered.status(OPTIONS)).key.operation_id,'original')
        self.assertEqual([v['request']['action']['type'] for v in self.calls()],['administrative_operation'])

    async def test_admin_context_close_and_fixed_config_keep_original_owner_and_no_private_diagnostics(self):
        client=self.administrator()
        async with client:
            operation=await client.provision(client.validate_provisioning(draft()),mutation())
            self.clients[-1][1].joinpath('config.json').write_text('changed invalid config')
            self.assertEqual((await client.operations.recover(operation.reference).status(OPTIONS)).record_id,'record-original')
        self.assertIsNotNone(client._helper._process.returncode)

    async def test_missing_admin_validator_is_a_private_configuration_error_without_startup(self):
        client=AdministrativeClient(self.directory/'private-config',self.directory/'private-executable')
        self.clients.append((client,self.directory/'unused'))
        with self.assertRaises(SdkError) as observed:client.validate_provisioning(draft())
        self.assertEqual((observed.exception.kind,observed.exception.certainty),('configuration','not-submitted'))
        self.assertNotIn('private',str(observed.exception));self.assertIsNone(client._helper._process)
