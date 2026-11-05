//! One hash-bound, genuinely decoding libvpx child, retained across commit.
use super::*;
use connect_client::{sdk::VideoReceiveEpoch, NativeVideoFrame};
use sha2::{Digest, Sha256};
use std::process::Stdio;
use tokio::{
    io::AsyncReadExt,
    process::Command,
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

pub(super) struct Decoded {
    pub ordinal: u32,
    pub keyframe: bool,
    pub epoch: VideoReceiveEpoch,
    pub arrived: tokio::time::Instant,
    pub finished: tokio::time::Instant,
    pub timestamp_rtp: u32,
    pub luma_sum: u32,
    pub encoded_sha256: String,
}
struct Request {
    ordinal: u32,
    frame: NativeVideoFrame,
    epoch: VideoReceiveEpoch,
    arrived: tokio::time::Instant,
    deadline: tokio::time::Instant,
}
type Reply = std::result::Result<Decoded, &'static str>;
#[derive(Clone, serde::Serialize)]
pub(super) struct Joined {
    pub joined: bool,
    pub clean: bool,
    pub cancelled: bool,
    pub forced: bool,
    pub original_pid: u32,
    pub exit_code: Option<i32>,
}
pub(super) struct Decoder {
    requests: mpsc::Sender<Request>,
    replies: mpsc::Receiver<Reply>,
    stop: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<Result<Joined>>>,
    pending: usize,
    submitted: u32,
    joined: Option<Joined>,
    original_pid: u32,
}
async fn decode(
    input: &mut tokio::process::ChildStdin,
    output: &mut tokio::process::ChildStdout,
    r: &Request,
) -> Result<Decoded> {
    let mut header = [0u8; 16];
    header[..4].copy_from_slice(b"D8F1");
    header[4..8].copy_from_slice(&r.ordinal.to_le_bytes());
    header[8..12].copy_from_slice(&(r.frame.encoded.len() as u32).to_le_bytes());
    header[12..].copy_from_slice(&u32::from(r.frame.keyframe).to_le_bytes());
    input.write_all(&header).await?;
    input.write_all(&r.frame.encoded).await?;
    let mut answer = [0u8; 32];
    output.read_exact(&mut answer).await?;
    let word = |at| u32::from_le_bytes(answer[at..at + 4].try_into().unwrap());
    if &answer[..4] != b"V8D1"
        || word(4) != r.ordinal
        || word(8) != 1
        || word(12) != u32::from(r.frame.keyframe)
        || word(16) != 640
        || word(20) != 360
        || word(24) > 255 * 256
        || word(28) != 256
    {
        return Err("actual decoder reply differs from its original complete frame".into());
    }
    Ok(Decoded {
        ordinal: r.ordinal,
        keyframe: r.frame.keyframe,
        epoch: r.epoch.clone(),
        arrived: r.arrived,
        finished: tokio::time::Instant::now(),
        timestamp_rtp: r.frame.timestamp_rtp,
        luma_sum: word(24),
        encoded_sha256: format!("{:x}", Sha256::digest(&r.frame.encoded)),
    })
}
impl Decoder {
    pub async fn open(program: &encoder::Program) -> Result<Self> {
        program.validate()?;
        encoder::verify(&program.executable, &program.executable_sha256, None).await?;
        let mut child = Command::new(&program.executable)
            .arg("--decode-vp8")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .env_clear()
            .env("LANG", "C")
            .kill_on_drop(true)
            .spawn()?;
        let original_pid = child
            .id()
            .ok_or("original decoder process identity missing")?;
        let mut input = child.stdin.take().ok_or("decoder input missing")?;
        let mut output = child.stdout.take().ok_or("decoder output missing")?;
        let (requests, mut incoming) = mpsc::channel::<Request>(8);
        let (reply, replies) = mpsc::channel::<Reply>(8);
        let (stop, mut stopping) = oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            let mut clean = true;
            let mut cancelled = false;
            let mut extra = [0u8; 1];
            loop {
                let r = tokio::select! {biased;
                    _=&mut stopping=>{cancelled=true;break;},
                    extra=output.read(&mut extra)=>{let _=extra;clean=false;break;},
                    request=incoming.recv()=>match request{Some(r)=>r,None=>break},
                };
                // Once submitted, complete this original bounded pipe request
                // before observing stop; no partial reply is abandoned/reissued.
                let result =
                    match tokio::time::timeout_at(r.deadline, decode(&mut input, &mut output, &r))
                        .await
                    {
                        Ok(Ok(value)) => Ok(value),
                        Ok(Err(_)) => Err("original decoder output refused or child closed"),
                        Err(_) => Err("original decoder reply expired"),
                    };
                let failed = result.is_err();
                if reply.try_send(result).is_err() || failed {
                    clean = false;
                    break;
                }
            }
            drop(input);
            // Original process is always joined. EOF is attempted even after
            // cancellation; forced cleanup is retained as unclean evidence.
            let ended = tokio::time::timeout(Duration::from_millis(500), async {
                let (status, tail) = tokio::join!(child.wait(), output.read(&mut extra));
                Ok::<_, std::io::Error>((status?, tail?))
            })
            .await;
            if let Ok(Ok((status, tail))) = ended {
                return Ok(Joined {
                    joined: true,
                    clean: clean && status.success() && tail == 0,
                    cancelled,
                    forced: false,
                    original_pid,
                    exit_code: status.code(),
                });
            }
            let _ = child.start_kill();
            let status = child.wait().await?;
            Ok(Joined {
                joined: true,
                clean: false,
                cancelled,
                forced: true,
                original_pid,
                exit_code: status.code(),
            })
        });
        Ok(Self {
            requests,
            replies,
            stop: Some(stop),
            task: Some(task),
            pending: 0,
            submitted: 0,
            joined: None,
            original_pid,
        })
    }
    pub fn pending(&self) -> bool {
        self.pending > 0
    }
    pub fn submit(
        &mut self,
        frame: NativeVideoFrame,
        epoch: VideoReceiveEpoch,
        arrived: tokio::time::Instant,
        until: tokio::time::Instant,
    ) -> Result<()> {
        let deadline = (arrived + Duration::from_millis(200)).min(until);
        if self.stop.is_none()
            || self.pending >= 8
            || self.submitted >= 1860
            || deadline <= tokio::time::Instant::now()
            || frame.encoded.len() < 3
            || frame.encoded.len() > 256 * 1024
        {
            return Err("original decoder input bound refused".into());
        }
        self.requests
            .try_send(Request {
                ordinal: self.submitted,
                frame,
                epoch,
                arrived,
                deadline,
            })
            .map_err(|_| "original decoder owner busy or stopped")?;
        self.submitted += 1;
        self.pending += 1;
        Ok(())
    }
    pub async fn receive(&mut self) -> Result<Decoded> {
        let value = self
            .replies
            .recv()
            .await
            .ok_or("original decoder reply owner stopped")?;
        self.pending = self
            .pending
            .checked_sub(1)
            .ok_or("unowned decoder output")?;
        Ok(value?)
    }
    pub async fn stop_and_join(&mut self) -> Result<Joined> {
        if let Some(value) = &self.joined {
            return Ok(value.clone());
        }
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        let value = self
            .task
            .as_mut()
            .ok_or("original decoder join already failed")?
            .await;
        self.task = None;
        let value = value??;
        self.joined = Some(value.clone());
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    fn program() -> encoder::Program {
        // Selected component requires the actual freshly built local helper;
        // absence is a failed setup, never a skipped/pass decoder check.
        assert_eq!(std::env::consts::OS, "linux");
        assert_eq!(std::env::consts::ARCH, "aarch64");
        let path = PathBuf::from(
            std::env::var_os("CONNECT_TEST_LIVE_VP8_ENCODER")
                .expect("exact private libvpx helper required"),
        );
        let bytes = std::fs::read(&path).unwrap();
        let root = path.parent().unwrap();
        encoder::Program {
            executable: path.clone(),
            executable_sha256: format!("{:x}", Sha256::digest(bytes)),
            raw_path: root.join("decoder-unused-source.i420"),
            raw_sha256: "0".repeat(64),
            frame_count: 2,
        }
    }
    fn epoch() -> VideoReceiveEpoch {
        VideoReceiveEpoch {
            epoch: "original-epoch".into(),
            incarnation: "original-incarnation".into(),
            revision: 1,
            ssrc: 7,
            first_sequence: 0,
            first_timestamp: 0,
        }
    }
    fn frame(timestamp: u32) -> NativeVideoFrame {
        NativeVideoFrame {
            timestamp_rtp: timestamp,
            keyframe: true,
            encoded: include_bytes!("../black-640x360.vp8").to_vec(),
        }
    }
    #[tokio::test]
    async fn actual_original_vp8_decoder_survives_two_phases_and_joins_naturally() {
        let mut decoder = Decoder::open(&program()).await.unwrap();
        let pid = decoder.original_pid;
        for timestamp in [0, 3000] {
            let now = tokio::time::Instant::now();
            decoder
                .submit(frame(timestamp), epoch(), now, now + Duration::from_secs(1))
                .unwrap();
            let value = decoder.receive().await.unwrap();
            assert_eq!(value.timestamp_rtp, timestamp);
            assert!(value.keyframe);
            assert_eq!(decoder.original_pid, pid);
            assert_eq!(value.epoch, epoch());
        }
        let joined = decoder.stop_and_join().await.unwrap();
        assert!(joined.joined && joined.clean && !joined.forced);
        assert_eq!((joined.original_pid, joined.exit_code), (pid, Some(0)));
        assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
    }
    #[tokio::test]
    async fn corrupt_complete_frame_is_not_readiness_and_failed_original_child_still_joins() {
        let mut decoder = Decoder::open(&program()).await.unwrap();
        let now = tokio::time::Instant::now();
        let bad = NativeVideoFrame {
            timestamp_rtp: 0,
            keyframe: true,
            encoded: vec![0; 16],
        };
        decoder
            .submit(bad, epoch(), now, now + Duration::from_secs(1))
            .unwrap();
        assert!(decoder.receive().await.is_err());
        let joined = decoder.stop_and_join().await.unwrap();
        assert!(joined.joined);
        assert!(!joined.clean);
        assert_ne!(joined.exit_code, Some(0));
    }
    #[tokio::test]
    async fn expired_original_decode_never_submits_or_resets_its_deadline() {
        let mut decoder = Decoder::open(&program()).await.unwrap();
        let now = tokio::time::Instant::now();
        assert!(decoder
            .submit(
                frame(0),
                epoch(),
                now - Duration::from_millis(201),
                now + Duration::from_secs(1)
            )
            .is_err());
        assert_eq!(decoder.submitted, 0);
        assert!(!decoder.pending());
        let joined = decoder.stop_and_join().await.unwrap();
        assert!(joined.joined && joined.clean && !joined.forced);
    }
}
