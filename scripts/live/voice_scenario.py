"""Bound speech fixtures before allocating media or contacting any server."""
import json
import math
import wave


def load_scenario(path, encode):
    with path.open("rb") as source:
        data = source.read(65537)
    if len(data) > 65536:
        raise ValueError("Scenario exceeds 64 KiB")
    scenario = json.loads(data)
    turns = scenario.get("turns") if isinstance(scenario, dict) else None
    if not isinstance(turns, list) or not 1 <= len(turns) <= 6:
        raise ValueError("Use one to six turns bounded by 120 seconds")
    prompts, duration = [], 2.0
    for turn in turns:
        if not isinstance(turn, dict) or not isinstance(turn.get("wav"), str):
            raise ValueError("Every turn requires a WAV path")
        wait = turn.get("wait_seconds", 10)
        if type(wait) not in (int, float) or not math.isfinite(wait) or not 0 <= wait <= 20:
            raise ValueError("Wait per turn must be between zero and twenty seconds")
        wait_for_reply = turn.get("wait_for_reply", False)
        if not isinstance(wait_for_reply, bool):
            raise ValueError("wait_for_reply must be boolean")
        with wave.open(str(path.parent / turn["wav"]), "rb") as source:
            if (source.getframerate(), source.getnchannels(), source.getsampwidth()) != (8000, 1, 2):
                raise ValueError("Prompts must be mono signed 16-bit 8 kHz WAV")
            frames = source.getnframes()
            if not 1 <= frames <= 8000 * 118:
                raise ValueError("Prompt exceeds the audio budget or is empty")
            raw = source.readframes(frames)
            if len(raw) != frames * 2:
                raise ValueError("Truncated WAV prompt")
        data = encode(raw)
        data += b"\xff" * (-len(data) % 160)
        duration += len(data) / 8000 + wait + (12 if wait_for_reply else 0)
        if duration > 120:
            raise ValueError("Scenario exceeds the 120-second audio budget")
        prompts.append(([data[i:i + 160] for i in range(0, len(data), 160)], int(wait * 50), wait_for_reply))
    return prompts
