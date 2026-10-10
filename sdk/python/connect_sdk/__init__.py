"""Connect SDK: native verified UCTP/QUIC or explicit loopback HTTP/JSON."""
from .errors import ConnectError
from .native import NativeClient
from .native_audio import NativeAudioClient
from .http import HttpClient

__all__ = ["ConnectError", "NativeClient", "NativeAudioClient", "HttpClient", "TASK_PROFILE", "TaskCommand", "TaskDefinition", "TaskOutcome", "TaskRecord", "TaskScope"]

from .task_types import TASK_PROFILE, TaskCommand, TaskDefinition, TaskOutcome, TaskRecord, TaskScope
