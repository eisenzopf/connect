#!/usr/bin/env python3
"""Seed one new isolated qualification profile with normal local CA trust."""
import argparse
import ctypes
import hashlib
import json
from pathlib import Path
import sqlite3
import subprocess


CHROME_VERSION = '156.0.8078.48'
NSS_VERSION = '3.130'


def import_firefox_root(runtime, profile, certificate):
    """Use the bundled NSS public API; no app, Keychain or existing profile edit."""
    c = ctypes
    c.CDLL(str(runtime / 'libmozglue.dylib'), mode=c.RTLD_GLOBAL)
    nss = c.CDLL(str(runtime / 'libnss3.dylib'))

    class Item(c.Structure):
        _fields_ = [('type', c.c_int), ('data', c.POINTER(c.c_ubyte)), ('len', c.c_uint)]

    class Trust(c.Structure):
        _fields_ = [('ssl', c.c_uint), ('email', c.c_uint), ('object', c.c_uint)]

    def function(name, result, arguments):
        value = getattr(nss, name)
        value.restype, value.argtypes = result, arguments
        return value

    def check(value, operation):
        if value != 0:
            error = function('PORT_GetError', c.c_int, [])()
            raise RuntimeError(f'{operation} refused: NSS error {error}')

    version = function('NSS_GetVersion', c.c_char_p, [])().decode()
    if version != NSS_VERSION:
        raise ValueError(f'Qualification importer requires NSS {NSS_VERSION}; got {version}')
    check(function('NSS_InitReadWrite', c.c_int, [c.c_char_p])(
        ('sql:' + str(profile)).encode()), 'NSS_InitReadWrite')
    try:
        slot = function('PK11_GetInternalKeySlot', c.c_void_p, [])()
        if not slot:
            raise RuntimeError('No private NSS software slot')
        try:
            if function('PK11_NeedUserInit', c.c_int, [c.c_void_p])(slot):
                check(function('PK11_InitPin', c.c_int, [c.c_void_p, c.c_char_p, c.c_char_p])(
                    slot, None, b''), 'PK11_InitPin')
            check(function('PK11_Authenticate', c.c_int, [c.c_void_p, c.c_int, c.c_void_p])(
                slot, 1, None), 'PK11_Authenticate')
        finally:
            function('PK11_FreeSlot', None, [c.c_void_p])(slot)
        database = function('CERT_GetDefaultCertDB', c.c_void_p, [])()
        if not database:
            raise RuntimeError('No private NSS certificate database')
        buffer = (c.c_ubyte * len(certificate)).from_buffer_copy(certificate)
        item = Item(0, buffer, len(certificate))
        items = (c.POINTER(Item) * 1)(c.pointer(item))
        certificates = c.POINTER(c.c_void_p)()
        check(function('CERT_ImportCerts', c.c_int, [
            c.c_void_p, c.c_int, c.c_uint, c.POINTER(c.POINTER(Item)),
            c.POINTER(c.POINTER(c.c_void_p)), c.c_int, c.c_int, c.c_char_p,
        ])(database, 3, 1, items, c.byref(certificates), 1, 1,
           b'Connect isolated qualification root'), 'CERT_ImportCerts')
        try:
            trust = Trust()
            check(function('CERT_DecodeTrustString', c.c_int, [c.POINTER(Trust), c.c_char_p])(
                c.byref(trust), b'C,,'), 'CERT_DecodeTrustString')
            if (trust.ssl, trust.email, trust.object) != (24, 0, 0):
                raise RuntimeError('Unexpected NSS website-only CA trust mapping')
            check(function('CERT_ChangeCertTrust', c.c_int, [
                c.c_void_p, c.c_void_p, c.POINTER(Trust),
            ])(database, certificates[0], c.byref(trust)), 'CERT_ChangeCertTrust')
        finally:
            function('CERT_DestroyCertArray', None, [c.POINTER(c.c_void_p), c.c_uint])(
                certificates, 1)
    finally:
        check(function('NSS_Shutdown', c.c_int, [])(), 'NSS_Shutdown')
    return version


def create_profile(browser, binary, profile, root_der):
    binary, profile, root_der = map(lambda p: Path(p).absolute(), (binary, profile, root_der))
    # Never accept an existing directory or symlink, including an empty profile.
    if profile.exists() or profile.is_symlink():
        raise FileExistsError(profile)
    certificate = root_der.read_bytes()
    if not 1 <= len(certificate) <= 65536:
        raise ValueError('Expected one bounded DER CA certificate')
    if browser == 'chrome':
        version = subprocess.check_output([str(binary), '--version'], text=True).strip()
        if version != 'Google Chrome ' + CHROME_VERSION:
            raise ValueError(f'Qualification database schema is pinned to Chrome {CHROME_VERSION}')
    elif browser != 'firefox':
        raise ValueError('Use the designated chrome or firefox runtime')
    profile.mkdir(mode=0o700)
    if browser == 'chrome':
        default = profile / 'Default'
        default.mkdir(mode=0o700)
        with sqlite3.connect(default / 'ServerCertificate') as db:
            db.executescript('''
                CREATE TABLE meta(key LONGVARCHAR NOT NULL UNIQUE PRIMARY KEY,value LONGVARCHAR);
                CREATE TABLE certificates(sha256hash_hex TEXT PRIMARY KEY,
                    der_cert BLOB NOT NULL,trust_settings BLOB NOT NULL);
            ''')
            db.executemany('INSERT INTO meta VALUES(?,?)', [
                ('version', 1), ('last_compatible_version', 1), ('mmap_status', -1),
            ])
            # Chromium 156 CertificateMetadata.trust (field 1) contains
            # CertificateTrust.trust_type (field 1), TRUSTED enum value 3.
            db.execute('INSERT INTO certificates VALUES(?,?,?)', (
                hashlib.sha256(certificate).hexdigest(), certificate, bytes.fromhex('0a020803')))
    else:
        version = import_firefox_root(binary.parent, profile, certificate)
    return {'browser': browser, 'version': version, 'profile': str(profile),
            'root_sha256': hashlib.sha256(certificate).hexdigest()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('browser', choices=['chrome', 'firefox'])
    parser.add_argument('binary', type=Path)
    parser.add_argument('profile', type=Path)
    parser.add_argument('root_der', type=Path)
    arguments = parser.parse_args()
    print(json.dumps(create_profile(**vars(arguments))))


if __name__ == '__main__':
    main()
