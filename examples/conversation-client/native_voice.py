#!/usr/bin/env python3
"""Microphone/speaker voice through Connect's public native UCTP/QUIC SDK.

Install the optional sounddevice package in a virtual environment. Supply an
already accepted audio Session and its existing conversation; this example
never guesses identity, starts a conversation, or changes provider routing.
Audio stays in bounded memory queues and is not recorded.
"""
import argparse
import json
import queue
import select
import sys
import threading

from connect_sdk import NativeAudioClient, NativeClient
from pcmu import decode, encode


def offer(queue_, frame):
    """Drop the oldest queued frame when the audio device falls behind."""
    try:
        queue_.put_nowait(frame)
    except queue.Full:
        try:
            queue_.get_nowait()
        except queue.Empty:
            pass
        try:
            queue_.put_nowait(frame)
        except queue.Full:
            pass


def run(args):
    try:
        import sounddevice
    except ImportError:
        raise SystemExit('Install optional microphone support: pip install sounddevice')
    capture = queue.Queue(maxsize=8)
    playback = queue.Queue(maxsize=25)
    stop = threading.Event()
    failures = queue.Queue()
    counts = {'sent_frames': 0, 'received_frames': 0, 'device_warnings': 0}

    def callback(input_, output, frames, time_, status):
        if status:
            counts['device_warnings'] += 1
        if frames != 160:
            output[:] = bytes(len(output))
            return
        offer(capture, bytes(input_))
        try:
            output[:] = playback.get_nowait()
        except queue.Empty:
            output[:] = bytes(320)

    with NativeAudioClient(
        args.config, args.conversation, args.session, args.participant,
        executable=args.executable, timeout=30,
    ) as audio:
        def sender():
            try:
                while not stop.is_set():
                    try:
                        frame = capture.get(timeout=.2)
                    except queue.Empty:
                        continue
                    audio.send_pcmu(encode(frame))
                    counts['sent_frames'] += 1
            except Exception as error:
                if not stop.is_set():
                    failures.put(str(error))
                    stop.set()

        def receiver():
            try:
                while not stop.is_set():
                    offer(playback, decode(audio.receive(timeout=30)))
                    counts['received_frames'] += 1
            except Exception as error:
                if not stop.is_set():
                    failures.put(str(error))
                    stop.set()

        threads = [threading.Thread(target=task, daemon=True) for task in (sender, receiver)]
        try:
            with sounddevice.RawStream(
                samplerate=8000, blocksize=160, channels=1, dtype='int16',
                callback=callback,
            ):
                for thread in threads:
                    thread.start()
                print('Voice connected through native QUIC. Speak now; press Enter to hang up.', flush=True)
                while not stop.is_set():
                    readable, _, _ = select.select([sys.stdin], [], [], .2)
                    if readable:
                        sys.stdin.readline()
                        break
        finally:
            stop.set()
            audio.close()
            for thread in threads:
                if thread.ident is not None:
                    thread.join(timeout=2)
    counts['ended_by'] = 'local' if failures.empty() else 'remote-or-transport'
    print(json.dumps(counts), flush=True)
    if not failures.empty():
        print('Audio ended: ' + failures.get(), file=sys.stderr)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--config', required=True)
    parser.add_argument('--conversation', required=True)
    parser.add_argument('--session', required=True)
    parser.add_argument('--participant', required=True)
    parser.add_argument('--executable', default='connect')
    args = parser.parse_args()
    try:
        run(args)
    except KeyboardInterrupt:
        pass
    finally:
        # Ending an audio Session preserves its parent Conversation.
        with NativeClient(args.config, args.executable) as client:
            client.session_end('microphone-end-' + args.session, args.conversation,
                               args.session, 'microphone-example-ended')


if __name__ == '__main__':
    main()
