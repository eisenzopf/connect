# RTC relay retirement

The full browser matrix at source 86b6cf7c, run 41209664880, records
decoded UDP-relay EdDSA media and AV passes followed by failure of the original
five-second physical route/capacity drain. The external server/container joins
and zero TURN allocations remain separate observations.

During relay retirement, the actual stack driver stops advancing the media
Core's timers, but continues including their deadlines in its next wakeup.
An expired media deadline can therefore repeatedly take the immediate-timeout
branch without reaching the socket select that receives TURN refresh responses.
The sender-report interceptor retains a periodic deadline through close, so a
retired media deadline need not disappear by itself.

The correction selects the existing three-second relay-close deadline instead
of polling the retired Core. Active gatherer and video-repair deadlines still
participate. Timeout handling takes Core custody only while media is active.
No close grace, allocation success, route retirement or capacity reclamation is
inferred from this source change.

Patch gate6-relay-close-active-timers.patch is reproduced by the official
dependency preparer in fresh frozen78, with all261 pinned patches. Frozen76
remains available for the separate SIP fact-memo run. No local Rust build is
attempted on the nearly full host disks.

The existing source-bound full browser matrix must verify this candidate,
including all four relay cases, original signed Session end, five-second route
and capacity drain, actual peer joins and zero allocations. Its unchanged
audio/video/AV/recovery assertions still apply. Runtime verification is unrun;
the cause of the historical UDP stall and whole G6-054 remain unqualified.


Run41219309028 at ec1d9c46 is source-verified but media remains unrun. Its
cold build finds no compatible cache and reaches the original3,600second limit
with exit-15, no forced kill,512 completed Cargo artifacts and ten required
runtime libraries at opt3. The server library and integration driver are
unfinished. Runtime, bridge and expiry steps are explicitly skipped.

The artifact binds2,878 Git inputs and3,089 prepared frozen78 inputs; it has
no producer executable. GitHub records a saved partial cache, independently
confirmed as cache9805350070. This is compilation progress, not runtime evidence.

Full browser run41224457162 resumes on f3e70fa7 using the same production
sources and recorded patch set. The only Rust difference is the Store snapshot
tests under cfg(test); production libraries, manifests, lock and patches are
identical. Completed artifacts may be reused only through the existing source,
mode, timestamp and patch checks. The one-hour build limit, opt3 runtime,
opt0 driver and every original media/cleanup assertion remain unchanged.
See [the recorded producer result](../evidence/gate-6/rtc-relay-retirement.json).
