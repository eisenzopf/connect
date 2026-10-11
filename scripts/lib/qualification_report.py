"""Source-stable private reports for explicit offline qualification phases."""
import hashlib
import json
import os
import re
import subprocess

from .local_process import ROOT


def source_inputs():
    paths = subprocess.check_output(
        ['git', 'ls-files', '--cached', '--others', '--exclude-standard', '-z'], cwd=ROOT).split(b'\0')
    return {os.fsdecode(p): hashlib.sha256((ROOT / os.fsdecode(p)).read_bytes()).hexdigest()
            for p in sorted(filter(None, paths)) if not p.startswith(b'docs/')
            and (ROOT / os.fsdecode(p)).is_file()}


def save(directory, report):
    path = directory / 'result.json'
    temporary = path.with_suffix('.tmp')
    temporary.write_text(json.dumps(report, indent=2) + '\n')
    temporary.replace(path)


def environment(codex_binary=None):
    """Build tools need a home/cache; fixtures receive no inherited API tokens."""
    names = ('PATH', 'HOME', 'USER', 'LOGNAME', 'TMPDIR', 'LANG', 'LC_ALL',
             'CARGO_HOME', 'RUSTUP_HOME', 'CC', 'CXX', 'AR', 'SDKROOT',
             'MACOSX_DEPLOYMENT_TARGET', 'SYSTEMROOT')
    result = {key: os.environ[key] for key in names if key in os.environ}
    result.update(PYTHONPATH=str(ROOT / 'sdk/python'), RUSTDOCFLAGS='-D warnings',
                  CONNECT_BINARY=str(ROOT / 'target/debug/connect'),
                  CONNECT_TEST_BINARY=str(ROOT / 'target/debug/connect'), CARGO_TERM_COLOR='never',
                  CARGO_PROFILE_DEV_DEBUG='0', CARGO_PROFILE_TEST_DEBUG='0', CARGO_INCREMENTAL='0')
    if codex_binary:
        result['CONNECT_TEST_CODEX_BINARY'] = str(codex_binary)
    return result


def versions(env, codex_binary=None):
    commands = {'rustc': ['rustc', '--version'], 'cargo': ['cargo', '--version'],
                'node': ['node', '--version'], 'npm': ['npm', '--version']}
    if codex_binary:
        commands['codex'] = [str(codex_binary), '--version']
    return {name: subprocess.check_output(command, cwd=ROOT, env=env, text=True,
                                          stderr=subprocess.STDOUT, timeout=15).strip()
            for name, command in commands.items()}


IGNORED = {
    'installed_runtime_discovers_skill_and_real_mcp_with_current_authority',
    'persisted_call_precedes_dispatch_and_exact_resume_reconciles_one_effect',
    'interrupted_before_dispatch_replays_original_program_after_listener_restart',
    'current_revocation_after_intent_persistence_denies_dispatch_and_required_startup',
    'two_installed_sessions_resume_bound_reply_after_server_restart_and_replay_one_follow_up',
    'packaged_host_renews_original_session_and_continues_reply_after_restart_without_repeating_effect',
    'overlapping_drafts_preserve_bytes_context_and_bound_reply_after_restart',
    'profile_one_and_four_concurrent_sessions_for_sixty_seconds',
    'retired_setups_reuse_capacity_through_the_full_session_limit',
}


def check_test_results(result, directory):
    """Record test totals and refuse an unaccounted ignored workspace check."""
    text = (directory / result['log']).read_text(errors='replace')
    totals = re.findall(r'test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored;', text)
    if totals:
        result['rust_tests'] = dict(zip(('passed', 'failed', 'ignored'),
                                       map(sum, zip(*[map(int, row) for row in totals]))))
    if result['status'] != 'passed':
        return
    python_expected = {'agent-incoming-unit': 15, 'agent-incoming-public': 1, 'agent-incoming-host': 1}
    if result['name'] in python_expected:
        runs = re.findall(r'^Ran (\d+) tests? in ', text, re.M)
        count = python_expected[result['name']]
        if runs != [str(count)] or '\nOK\n' not in text or 'skipped=' in text:
            raise ValueError('Incoming example phase did not execute every required Python check')
        result['python_tests'] = {'passed': count, 'failed': 0, 'ignored': 0}
        return
    expected = {'workspace': None, 'installed-host': 7, 'media-pressure': 1}
    if result['name'] not in expected:
        return
    if not totals:
        raise ValueError('Required Rust phase has no completed test summary')
    counts = result['rust_tests']
    if counts['failed']:
        raise ValueError('Rust test failures contradict phase success')
    if result['name'] == 'workspace':
        ignored = set(re.findall(r'^test (\S+) \.\.\. ignored', text, re.M))
        if ignored != IGNORED or counts['ignored'] != len(IGNORED):
            raise ValueError('Workspace ignored tests differ from the explicit follow-up phases')
        result['ignored_follow_up'] = sorted(ignored)
    elif counts['passed'] != expected[result['name']] or counts['ignored']:
        raise ValueError('Explicit ignored-test phase did not execute every required check')
