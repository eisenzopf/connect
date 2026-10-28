#!/usr/bin/env python3
"""Import one fixture CA into the fresh Linux container's normal NSS user DB."""
import ctypes as c
import hashlib
import json
from pathlib import Path
import sys


def main():
    if sys.platform != "linux" or len(sys.argv) != 2:
        raise ValueError("Linux fixture CA path required")
    certificate = Path(sys.argv[1]).read_bytes()
    if not 1 <= len(certificate) <= 65536:
        raise ValueError("Expected one bounded DER CA")
    database_path = Path.home() / ".pki/nssdb"
    database_path.mkdir(parents=True, mode=0o700, exist_ok=False)
    nss = c.CDLL("libnss3.so")

    class Item(c.Structure):
        _fields_ = [("type", c.c_int), ("data", c.POINTER(c.c_ubyte)), ("len", c.c_uint)]

    class Trust(c.Structure):
        _fields_ = [("ssl", c.c_uint), ("email", c.c_uint), ("object", c.c_uint)]

    def function(name, result, arguments):
        value = getattr(nss, name)
        value.restype, value.argtypes = result, arguments
        return value

    def check(value):
        if value != 0:
            raise RuntimeError("Fixture NSS import refused")

    version = function("NSS_GetVersion", c.c_char_p, [])().decode()
    if version != "3.98":
        raise ValueError("Expected the pinned owned image's NSS 3.98")
    check(function("NSS_InitReadWrite", c.c_int, [c.c_char_p])(("sql:" + str(database_path)).encode()))
    try:
        slot = function("PK11_GetInternalKeySlot", c.c_void_p, [])()
        if not slot:
            raise RuntimeError("No private NSS slot")
        try:
            if function("PK11_NeedUserInit", c.c_int, [c.c_void_p])(slot):
                check(function("PK11_InitPin", c.c_int, [c.c_void_p, c.c_char_p, c.c_char_p])(slot, None, b""))
            check(function("PK11_Authenticate", c.c_int, [c.c_void_p, c.c_int, c.c_void_p])(slot, 1, None))
        finally:
            function("PK11_FreeSlot", None, [c.c_void_p])(slot)
        database = function("CERT_GetDefaultCertDB", c.c_void_p, [])()
        if not database:
            raise RuntimeError("No private certificate DB")
        buffer = (c.c_ubyte * len(certificate)).from_buffer_copy(certificate)
        item = Item(0, buffer, len(certificate))
        items = (c.POINTER(Item) * 1)(c.pointer(item))
        certificates = c.POINTER(c.c_void_p)()
        check(function("CERT_ImportCerts", c.c_int, [c.c_void_p, c.c_int, c.c_uint,
              c.POINTER(c.POINTER(Item)), c.POINTER(c.POINTER(c.c_void_p)), c.c_int, c.c_int,
              c.c_char_p])(database, 3, 1, items, c.byref(certificates), 1, 1, b"Connect owned fixture CA"))
        try:
            trust = Trust()
            check(function("CERT_DecodeTrustString", c.c_int, [c.POINTER(Trust), c.c_char_p])(c.byref(trust), b"C,,"))
            if (trust.ssl, trust.email, trust.object) != (24, 0, 0):
                raise RuntimeError("Unexpected website CA trust")
            check(function("CERT_ChangeCertTrust", c.c_int, [c.c_void_p, c.c_void_p, c.POINTER(Trust)])(
                database, certificates[0], c.byref(trust)))
        finally:
            function("CERT_DestroyCertArray", None, [c.POINTER(c.c_void_p), c.c_uint])(certificates, 1)
    finally:
        check(function("NSS_Shutdown", c.c_int, [])())
    print(json.dumps({"normal_user_ca_import": True, "nss_version": version,
                      "root_sha256": hashlib.sha256(certificate).hexdigest()}))


if __name__ == "__main__":
    main()
