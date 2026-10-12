"""Choose shipped application files separately from the qualification fixtures."""
import os
from pathlib import Path

SOURCE = Path(__file__).resolve().parents[3]
RUNTIME = Path(os.environ.get("CONNECT_TEST_RUNTIME_ROOT", SOURCE)).resolve()


def verify_imports(binary):
    """An installed test must not silently import the checkout's SDK or app."""
    if "CONNECT_TEST_RUNTIME_ROOT" not in os.environ:
        return
    import connect_sdk
    import incoming
    expected = ((connect_sdk, RUNTIME / "sdk/python/connect_sdk"),
                (incoming, RUNTIME / "examples/agent-presence/incoming"))
    for module, directory in expected:
        if not Path(module.__file__).resolve().is_relative_to(directory):
            raise AssertionError(f"{module.__name__} loaded outside the selected runtime")
    if Path(binary).resolve() != (RUNTIME / "bin/connect").resolve():
        raise AssertionError("Native helper does not belong to the selected runtime")
