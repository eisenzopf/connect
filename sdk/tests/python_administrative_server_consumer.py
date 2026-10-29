"""Installed trusted Python backend; only explicit isolated host commands."""
import asyncio
from collections.abc import Mapping
import importlib.metadata
import json
import os
from pathlib import Path
import sys
import traceback

import connect_sdk
from connect_sdk import AdministrativeClient, Cancellation, MutationOptions, OperationId, RequestOptions, SdkError

spec=json.loads(Path(sys.argv[1]).read_text())
assert Path.cwd().resolve()==Path(spec['project']).resolve()
assert Path(connect_sdk.__file__).resolve().is_relative_to(Path(sys.prefix).resolve())
assert importlib.metadata.version('connect-sdk')=='0.1.0'
assert 'PYTHONPATH' not in os.environ
options=RequestOptions(4000)
checkpoint='ready'


def plain(value):
    if isinstance(value,Mapping):
        return {k:plain(v) for k,v in value.items()}
    if isinstance(value,(list,tuple)):
        return [plain(v) for v in value]
    return value


def emit(value):
    print(json.dumps(plain(value),separators=(',',':')),flush=True)


async def run():
    global checkpoint
    reader=asyncio.StreamReader(limit=128*1024)
    protocol=asyncio.StreamReaderProtocol(reader)
    transport,_=await asyncio.get_running_loop().connect_read_pipe(lambda:protocol,sys.stdin)
    try:
        async with AdministrativeClient(spec['config']) as client:
            emit({'phase':'ready'})
            while True:
                line=await asyncio.wait_for(reader.readline(),15)
                assert 0<len(line)<=128*1024
                request=json.loads(line)
                checkpoint=request['action']
                if checkpoint=='apply':
                    command=client.validate_provisioning(request['command'])
                    saved=command.reference_json()
                    if isinstance(request['command'].get('alias'),str):
                        original=command.value['alias']
                        request['command']['alias']='mutated-after-capture'
                        assert command.value['alias']==original
                    mutation=MutationOptions(command.operation_id,options)
                    operation=await client.provision(command,mutation)
                    first=await operation.wait(options)
                    assert operation.reference_json()==saved and first.snapshot.state=='completed'
                    try:
                        command.value['operation_id']='foreign'
                    except TypeError:
                        pass
                    else:
                        raise AssertionError('Mutable captured provisioning command')
                    restored=client.operations.restore(saved)
                    again=await restored.wait(options)
                    assert again.snapshot.record_id==first.snapshot.record_id and again.value==first.value
                    assert (await client.operations.recover(operation.reference).status(options)).record_id==first.snapshot.record_id
                    replay=await client.provision(command,mutation)
                    assert (await replay.wait(options)).snapshot.record_id==first.snapshot.record_id
                    emit({'phase':'applied','reference':saved,'value':first.value,'record_id':first.snapshot.record_id})
                elif checkpoint=='read':
                    read=await client.operations.restore(request['reference']).wait(options)
                    if request['expected_record'] is not None:
                        assert read.snapshot.record_id==request['expected_record']
                    emit({'phase':'read','value':read.value,'record_id':read.snapshot.record_id})
                elif checkpoint=='held':
                    command=client.validate_provisioning(request['command'])
                    queued_command=client.validate_provisioning(request['queued'])
                    original_cancel=Cancellation()
                    queued_cancel=Cancellation()
                    original=asyncio.create_task(client.provision(command,MutationOptions(command.operation_id,
                        RequestOptions(4000,original_cancel))))
                    queued=asyncio.create_task(client.provision(queued_command,MutationOptions(queued_command.operation_id,
                        RequestOptions(4000,queued_cancel))))
                    try:
                        emit({'phase':'submitted','reference':command.reference_json()})
                        cancel=json.loads(await asyncio.wait_for(reader.readline(),15))
                        assert cancel=={'action':'cancel-held'}
                        checkpoint='held-cancel'
                        queued_cancel.cancel()
                        try:
                            await queued
                        except SdkError as error:
                            assert (error.kind,error.certainty)==('cancelled','not-submitted')
                            assert error.operation.operation_id==queued_command.operation_id
                        else:
                            raise AssertionError('Cancelled queued command submitted')
                        original_cancel.cancel()
                        try:
                            await original
                        except SdkError as error:
                            assert (error.kind,error.certainty)==('cancelled','uncertain')
                            assert error.operation.operation_id==command.operation_id
                        else:
                            raise AssertionError('Held response accepted after cancellation')
                        emit({'phase':'uncertain','reference':command.reference_json(),
                              'original_id':command.operation_id,'original_certainty':'uncertain',
                              'queued_id':queued_command.operation_id,'queued_certainty':'not-submitted'})
                    finally:
                        original_cancel.cancel()
                        queued_cancel.cancel()
                        await asyncio.gather(original,queued,return_exceptions=True)
                elif checkpoint=='read-refused':
                    operation=client.operations.restore(request['reference'])
                    try:
                        await operation.status(options)
                    except SdkError as error:
                        assert (error.kind=='authentication' and error.certainty in ('refused','read-only')
                            or request['revoked'] and error.kind in ('cancelled','transport-uncertain') and error.certainty=='read-only')
                        assert error.operation.operation_id==operation.reference.operation_id
                        emit({'phase':'read-refused','kind':error.kind,'certainty':error.certainty})
                    else:
                        raise AssertionError('Current administrative authority accepted')
                elif checkpoint=='refuse':
                    command=client.validate_provisioning(request['command'])
                    try:
                        await client.provision(command,MutationOptions(command.operation_id,options))
                    except SdkError as error:
                        assert (error.kind,error.certainty)==(request['kind'],request['certainty'])
                        assert error.operation.operation_id==command.operation_id
                    else:
                        raise AssertionError('Unauthorized provisioning accepted')
                    emit({'phase':'refused'})
                else:
                    assert checkpoint=='close'
                    await client.close(options)
                    def pids(path):
                        return [int(line) for line in path.read_text().splitlines()] if path.exists() else []
                    helpers=pids(Path(spec['pid_file']))
                    validators=pids(Path(spec['pid_file']+'.validators'))
                    assert len(helpers)==1 and len(set(helpers+validators))==len(helpers)+len(validators)
                    for pid in helpers+validators:
                        try:
                            os.kill(pid,0)
                        except ProcessLookupError:
                            pass
                        else:
                            raise AssertionError('Original administrative helper/validator still exists')
                    emit({'phase':'closed','helpers_joined':len(helpers),'validators_joined':len(validators)})
                    break
    finally:
        transport.close()


try:
    asyncio.run(run())
except Exception as error:
    local=[frame for frame in traceback.extract_tb(error.__traceback__) if frame.filename==__file__]
    emit({'phase':'failed','checkpoint':checkpoint if checkpoint in ('ready','apply','read','held','held-cancel','read-refused','refuse','close') else 'invalid',
          'kind':error.kind if isinstance(error,SdkError) else 'consumer-assertion',
          'certainty':error.certainty if isinstance(error,SdkError) else None,
          'line':local[-1].lineno if local else None,'error_type':type(error).__name__})
    raise
