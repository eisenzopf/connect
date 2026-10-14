"""Select build/runtime prerequisites without inheriting account credentials."""
import os


def fixture_environment(environ=None):
    """Packaging is local; provider and model settings are never prerequisites."""
    selected = os.environ if environ is None else environ
    names = ('PATH', 'HOME', 'USER', 'LOGNAME', 'TMPDIR', 'LANG', 'LC_ALL',
        'CARGO_HOME', 'RUSTUP_HOME', 'CARGO_BUILD_JOBS', 'CARGO_TARGET_DIR',
        'CC', 'CXX', 'AR', 'SDKROOT', 'MACOSX_DEPLOYMENT_TARGET', 'SYSTEMROOT')
    return dict({key: selected[key] for key in names if key in selected},
        PYTHONDONTWRITEBYTECODE='1', CARGO_PROFILE_DEV_DEBUG='0',
        CARGO_PROFILE_TEST_DEBUG='0', CARGO_INCREMENTAL='0')
