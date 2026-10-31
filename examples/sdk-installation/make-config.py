#!/usr/bin/env python3
"""Create a private SDK configuration from a fresh local candidate installation."""
import argparse
import os
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('installation', type=Path)
args = parser.parse_args()
root = args.installation.resolve(strict=True)
native = (root / 'operator.toml').read_text()
application = (root / 'operator.application.toml').read_text()
# Keep relative signing/trust files beside the configuration. Never print keys.
value = 'tenant_id = "tnt_default"\n[control]\n' + native + '\n[application]\n' + application
path = root / 'operator.sdk.toml'
fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
with os.fdopen(fd, 'w') as stream:
    stream.write(value)
    stream.flush()
    os.fsync(stream.fileno())
print('Private SDK configuration created beside the installation credentials.')
