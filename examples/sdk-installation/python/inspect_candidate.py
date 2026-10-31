"""Run with the installed wheel and matching candidate connect on PATH."""
import asyncio
import json
import sys
from connect_sdk import Client, RequestOptions


async def main():
    if len(sys.argv) != 2:
        raise SystemExit('Usage: python inspect_candidate.py PRIVATE_SDK_CONFIG')
    options = RequestOptions(4000)
    async with Client(sys.argv[1]) as client:
        value = await client.capabilities.refresh(options)
        print(json.dumps({'profile': value.profile, 'feature_count': len(value.features)}))


if __name__ == '__main__':
    asyncio.run(main())
