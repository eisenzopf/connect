"""Private phase reports and sampled RSS for an owned local server process."""
import contextlib
import json
import pathlib
import subprocess
import threading
import time


class ProfileReport:
    def __init__(self, directory, initial):
        self.directory = pathlib.Path(directory)
        self.data = dict(initial, status='running', phases=[], observations={})
        self.pending = None
        self.durations = []
        self.rss = []
        self.save()

    def save(self):
        path = self.directory / 'result.json'
        temporary = path.with_suffix('.tmp')
        temporary.write_text(json.dumps(self.data, indent=2) + '\n')
        temporary.replace(path)

    def checkpoint(self, **values):
        self.data['observations'].update(values)
        self.save()

    def latency(self, seconds):
        self.durations.append(seconds * 1000)

    @contextlib.contextmanager
    def phase(self, name):
        print('RUN', name, flush=True)
        started = time.monotonic()
        first = len(self.durations)
        first_rss = len(self.rss)
        entry = {'name': name, 'status': 'running'}
        self.data['phases'].append(entry)
        self.save()
        try:
            yield
            entry['status'] = 'passed'
        except BaseException:
            entry['status'] = 'failed'
            raise
        finally:
            values = sorted(self.durations[first:])
            entry['seconds'] = round(time.monotonic() - started, 3)
            entry['exchanges'] = len(values)
            samples = self.rss[first_rss:]
            entry['server_sampled_peak_rss_bytes'] = max(samples, default=None)
            entry['database_apparent_bytes'] = sum(
                p.stat().st_size for p in (self.directory / 'state').glob('state.sqlite3*')
            )
            if values:
                entry['latency_ms'] = {
                    'p50': round(values[(len(values) - 1) // 2], 3),
                    'p95': round(values[int((len(values) - 1) * .95)], 3),
                    'maximum': round(values[-1], 3),
                }
            self.save()
            print(entry['status'].upper(), name, flush=True)

    @contextlib.contextmanager
    def sample(self, pid):
        stop = threading.Event()

        def observe():
            while not stop.is_set():
                try:
                    text = subprocess.check_output(
                        ['ps', '-o', 'rss=', '-p', str(pid)], text=True, timeout=2,
                        stderr=subprocess.DEVNULL,
                    )
                    self.rss.append(int(text.strip()) * 1024)
                except (ValueError, subprocess.SubprocessError, OSError):
                    pass
                stop.wait(.5)

        worker = threading.Thread(target=observe, daemon=True)
        worker.start()
        try:
            yield
        finally:
            stop.set()
            worker.join(timeout=3)
            assert not worker.is_alive(), 'RSS sampler did not stop'

    def finish(self, status, state, error=None):
        self.data['status'] = status
        self.data['server_rss'] = {
            'samples': len(self.rss), 'sampled_peak_bytes': max(self.rss, default=None),
            'note': 'Owned server PID only, sampled every 0.5s; not an exact kernel high-water mark.',
        }
        self.data['database_bytes'] = {
            path.name: path.stat().st_size for path in state.glob('state.sqlite3*')
        }
        if error:
            self.data['error'] = {'type': type(error).__name__, 'detail': str(error)}
            if getattr(error, '__notes__', None):
                self.data['error']['notes'] = error.__notes__
            if hasattr(error, 'code'):
                self.data['error']['code'] = error.code
            if error.__cause__ is not None:
                self.data['error']['cause_type'] = type(error.__cause__).__name__
            self.data['pending_command'] = self.pending
        self.save()
