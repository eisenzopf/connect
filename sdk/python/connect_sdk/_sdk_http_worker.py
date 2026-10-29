"""One private, one-shot HTTP exchange. Parent owns deadline, cancellation and PID."""
from __future__ import annotations

from dataclasses import asdict
import http.client
import json
import socket
import ssl
import sys
import time

from ._sdk_helper import _encode, _parse
from ._sdk_http_protocol import MAX_RESPONSE, PROFILE, _Endpoint, _reply, _request
from .client import _object
from .sdk_types import SdkError,identifier


def _exchange(endpoint, frame, body, key, timeout_ms):
    deadline=time.monotonic()+timeout_ms/1000
    timeout=timeout_ms/1000
    connection = None
    sent=False
    response=None
    try:
        connection = (http.client.HTTPSConnection(endpoint.host,endpoint.port,timeout=timeout,context=ssl.create_default_context())
                      if endpoint.transport=="remote-https" else http.client.HTTPConnection(endpoint.host,endpoint.port,timeout=timeout))
        connection.connect()
        if time.monotonic()>=deadline:
            raise TimeoutError()
        sent=True
        connection.request("POST","/v2/commands",body,{"Content-Type":"application/json","Authorization":"Bearer "+endpoint.credential,"Accept":"application/json"})
        response=connection.getresponse()
        if 300<=response.status<400 or response.getheader("content-type","").split(";",1)[0].strip().lower()!="application/json":
            raise ValueError()
        headers=response.getheaders()
        if sum(len(k)+len(v)+4 for k,v in headers)>16384:
            raise ValueError()
        lengths=response.headers.get_all("content-length",[])
        transfers=response.headers.get_all("transfer-encoding",[])
        if (len(lengths)>1 or len(transfers)>1 or lengths and transfers
                or lengths and (not lengths[0].isascii() or not lengths[0].isdecimal() or int(lengths[0])>MAX_RESPONSE)
                or transfers and transfers[0].strip().lower()!="chunked"
                or response.getheader("content-encoding","identity").strip().lower()!="identity"):
            raise ValueError()
        raw=response.read(MAX_RESPONSE+1)
        if len(raw)>MAX_RESPONSE or lengths and len(raw)!=int(lengths[0]):
            raise ValueError()
        if time.monotonic()>=deadline:
            raise TimeoutError()
        frames=_reply(_parse(raw,True),frame,response.status,key)
        if time.monotonic()>=deadline:
            raise TimeoutError()
        return {"requestId":frame["id"],"envelopes":frames}
    except SdkError:
        raise
    except ssl.SSLCertVerificationError:
        raise SdkError("trust","not-submitted" if key is not None else "read-only",key) from None
    except (TimeoutError,socket.timeout):
        raise SdkError("deadline","uncertain" if sent and key is not None else "not-submitted" if key is not None else "read-only",key) from None
    except (OSError,http.client.HTTPException):
        raise SdkError("transport-uncertain","uncertain" if sent and key is not None else "not-submitted" if key is not None else "read-only",key) from None
    except (ValueError,TypeError,UnicodeError,RecursionError):
        raise SdkError("protocol","uncertain" if key is not None else "read-only",key) from None
    finally:
        if response is not None:
            response.close()
        if connection is not None:
            connection.close()


def main():
    raw=sys.stdin.buffer.readline(128*1024+1)
    if not raw.endswith(b"\n") or len(raw)>128*1024:
        return 1
    job=_parse(raw,True)
    if (not _object(job,("profile","id","request")) or job["profile"]!=PROFILE or not identifier(job["id"])
            or not _object(job["request"],("timeout_ms","action"))):
        return 1
    request=job["request"];action=request["action"]
    if (type(request["timeout_ms"]) is not int or not 1<=request["timeout_ms"]<=60000
            or not _object(action,("type","config","envelope")) or action["type"]!="http_request"
            or not _object(action["config"],("endpoint","credential","transport"))):
        return 1
    key=None;phase="startup"
    try:
        endpoint=_Endpoint.parse(**action["config"])
        frame,body,key=_request(action["envelope"])
        phase="request"
        result={"Ok":_exchange(endpoint,frame,body,key,request["timeout_ms"])}
    except SdkError as error:
        result={"Err":{"kind":error.kind,"certainty":error.certainty,"operation":error.operation._wire() if error.operation is not None else None,
                       "problem":asdict(error.problem) if error.problem is not None else None,"observation":None}}
    response={"profile":PROFILE,"id":job["id"],"phase":phase,"result":result}
    _,encoded=_encode(response,MAX_RESPONSE+1024,True)
    sys.stdout.buffer.write(encoded+b"\n");sys.stdout.buffer.flush()
    # The parent must consume the complete reply before process exit can win
    # its independent EOF watcher. No second request is accepted on this owner.
    sys.stdin.buffer.read(1)
    return 0


if __name__=="__main__":
    try:
        raise SystemExit(main())
    except (ValueError,TypeError,UnicodeError,RecursionError,OSError):
        raise SystemExit(1) from None
