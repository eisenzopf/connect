//! A finite fixture encoder owns pipe IO independently of cancellable control waits.
//! Neither a source descriptor nor a keyframe request grants media authority.
use super::*;
use sha2::{Digest, Sha256};
use std::{os::unix::fs::MetadataExt, process::Stdio};
use tokio::{
    io::AsyncReadExt,
    process::Command,
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

const RAW_FRAME: u64 = 640 * 360 * 3 / 2;
const MAX_FRAME: usize = 256 * 1024;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Program {
    pub executable: PathBuf,
    pub executable_sha256: String,
    pub raw_path: PathBuf,
    pub raw_sha256: String,
    pub frame_count: u32,
}
impl Program {
    pub fn validate(&self) -> Result<()> {
        let private = |path: &std::path::Path| {
            path.is_absolute()
                && path.components().any(|part| part.as_os_str() == ".local")
                && !path
                    .components()
                    .any(|part| matches!(part, std::path::Component::ParentDir))
                && !path.as_os_str().as_encoded_bytes().contains(&0)
        };
        let digest = |value: &str| {
            value.len() == 64
                && value
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        };
        if !(1..=1860).contains(&self.frame_count)
            || !private(&self.executable)
            || !private(&self.raw_path)
            || self.executable == self.raw_path
            || !digest(&self.executable_sha256)
            || !digest(&self.raw_sha256)
        {
            return Err("invalid finite private live encoder programme".into());
        }
        Ok(())
    }
}

pub(super) async fn verify(
    path: &std::path::Path,
    expected: &str,
    raw_size: Option<u64>,
) -> Result<()> {
    let before = tokio::fs::symlink_metadata(path).await?;
    let limit = raw_size.unwrap_or(16 * 1024 * 1024);
    if !before.is_file()
        || before.file_type().is_symlink()
        || before.len() > limit
        || raw_size.is_some_and(|size| before.len() != size || before.mode() & 0o077 != 0)
        || (raw_size.is_none() && before.mode() & 0o111 == 0)
    {
        return Err("live encoder input mode or size refused".into());
    }
    let mut file = tokio::fs::File::open(path).await?;
    let mut hash = Sha256::new();
    let mut buffer = vec![0; 64 * 1024];
    let mut total = 0u64;
    loop {
        let count = file.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > limit {
            return Err("live encoder input grew beyond bound".into());
        }
        hash.update(&buffer[..count]);
    }
    let after = tokio::fs::symlink_metadata(path).await?;
    let identity = |value: &std::fs::Metadata| {
        (
            value.dev(),
            value.ino(),
            value.len(),
            value.mode(),
            value.mtime(),
            value.mtime_nsec(),
            value.ctime(),
            value.ctime_nsec(),
        )
    };
    if total != before.len()
        || identity(&before) != identity(&after)
        || format!("{:x}", hash.finalize()) != expected
    {
        return Err("live encoder input changed or digest mismatched".into());
    }
    Ok(())
}

pub(super) struct Frame {
    pub index: u32,
    pub bytes: Vec<u8>,
}
type Reply = std::result::Result<Frame, &'static str>;
struct Request {
    index: u32,
    force: bool,
    deadline: tokio::time::Instant,
    reply: oneshot::Sender<Reply>,
}
struct Pending {
    index: u32,
    reply: oneshot::Receiver<Reply>,
}
#[derive(Clone, serde::Serialize)]
pub(super) struct Joined {
    pub joined: bool,
    pub clean: bool,
    pub cancelled: bool,
}
pub(super) struct Encoder {
    requests: mpsc::Sender<Request>,
    stop: Option<oneshot::Sender<bool>>,
    task: Option<JoinHandle<Result<Joined>>>,
    pending: Option<Pending>,
    previous: Option<u32>,
    count: u32,
    joined: Option<Joined>,
    #[cfg(test)]
    pid: u32,
}

async fn encode(
    input: &mut tokio::process::ChildStdin,
    output: &mut tokio::process::ChildStdout,
    request: &Request,
) -> Result<Frame> {
    let mut command = [0; 8];
    command[..4].copy_from_slice(&request.index.to_le_bytes());
    command[4..].copy_from_slice(&u32::from(request.force).to_le_bytes());
    input.write_all(&command).await?;
    let mut header = [0; 16];
    output.read_exact(&mut header).await?;
    let word = |at| u32::from_le_bytes(header[at..at + 4].try_into().unwrap());
    let length = word(8) as usize;
    let key = word(12);
    if &header[..4] != b"V8F1"
        || word(4) != request.index
        || !(3..=MAX_FRAME).contains(&length)
        || key > 1
        || (request.force && key != 1)
    {
        return Err("live encoder reply header differs from original request".into());
    }
    let mut bytes = vec![0; length];
    output.read_exact(&mut bytes).await?;
    if video::key(&bytes) != (key == 1)
        || (key == 1
            && (length < 10
                || bytes[3..6] != [0x9d, 1, 0x2a]
                || u16::from_le_bytes([bytes[6], bytes[7]]) & 0x3fff != 640
                || u16::from_le_bytes([bytes[8], bytes[9]]) & 0x3fff != 360))
    {
        return Err("live encoder keyframe profile refused".into());
    }
    Ok(Frame {
        index: request.index,
        bytes,
    })
}

impl Encoder {
    pub async fn open(program: &Program) -> Result<Self> {
        program.validate()?;
        verify(&program.executable, &program.executable_sha256, None).await?;
        verify(
            &program.raw_path,
            &program.raw_sha256,
            Some(u64::from(program.frame_count) * RAW_FRAME),
        )
        .await?;
        let mut child = Command::new(&program.executable)
            .arg(&program.raw_path)
            .arg(program.frame_count.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .env_clear()
            .env("LANG", "C")
            .kill_on_drop(true)
            .spawn()?;
        #[cfg(test)]
        let pid = child.id().ok_or("encoder process has no identity")?;
        let mut input = child.stdin.take().ok_or("encoder input missing")?;
        let mut output = child.stdout.take().ok_or("encoder output missing")?;
        let (requests, mut incoming) = mpsc::channel::<Request>(1);
        let (stop, mut stopping) = oneshot::channel::<bool>();
        let task = tokio::spawn(async move {
            let mut clean = true;
            let mut cancelled = false;
            let mut extra = [0; 1];
            loop {
                let request = tokio::select! {
                    biased;
                    pending = &mut stopping => { cancelled = matches!(pending, Ok(true)); clean = matches!(pending, Ok(false)); break; },
                    extra = output.read(&mut extra) => { let _ = extra; clean = false; break; },
                    request = incoming.recv() => match request { Some(request) => request, None => { clean = false; break; } },
                };
                let result = tokio::select! {
                    biased;
                    _ = &mut stopping => { cancelled = true; Err("live encoder cancelled with original request pending") },
                    result = tokio::time::timeout_at(request.deadline, encode(&mut input, &mut output, &request)) =>
                        match result { Ok(Ok(frame)) => Ok(frame), _ => Err("live encoder original reply failed or expired") },
                };
                let failed = result.is_err();
                if request.reply.send(result).is_err() || failed {
                    clean = false;
                    break;
                }
            }
            drop(input);
            // Clean EOF is bounded. Malformed replies/cancellation immediately
            // kill, but success is never returned until the actual child exits.
            if clean {
                let finished = tokio::time::timeout(Duration::from_millis(500), async {
                    let (status, tail) = tokio::join!(child.wait(), output.read(&mut extra));
                    Ok::<_, std::io::Error>(status?.success() && tail? == 0)
                })
                .await;
                match finished {
                    Ok(Ok(true)) => {
                        return Ok(Joined {
                            joined: true,
                            clean: true,
                            cancelled: false,
                        })
                    }
                    _ => clean = false,
                }
            }
            let _ = child.start_kill();
            child.wait().await?;
            Ok(Joined {
                joined: true,
                clean,
                cancelled,
            })
        });
        Ok(Self {
            requests,
            stop: Some(stop),
            task: Some(task),
            pending: None,
            previous: None,
            count: program.frame_count,
            joined: None,
            #[cfg(test)]
            pid,
        })
    }

    pub fn pending(&self) -> bool {
        self.pending.is_some()
    }

    pub fn request(
        &mut self,
        index: u32,
        force: bool,
        deadline: tokio::time::Instant,
    ) -> Result<()> {
        let now = tokio::time::Instant::now();
        if self.pending.is_some()
            || self.stop.is_none()
            || index >= self.count
            || self.previous.is_some_and(|previous| index <= previous)
            || deadline <= now
            || deadline - now > Duration::from_millis(200)
        {
            return Err("live encoder original request bound refused".into());
        }
        let (reply, receiver) = oneshot::channel();
        self.requests
            .try_send(Request {
                index,
                force,
                deadline,
                reply,
            })
            .map_err(|_| "live encoder owner stopped or busy")?;
        self.pending = Some(Pending {
            index,
            reply: receiver,
        });
        self.previous = Some(index);
        Ok(())
    }

    // Await the retained receiver by reference. Cancelling this wait does not
    // cancel pipe IO, consume the reply, reset its deadline or issue a command.
    pub async fn receive(&mut self) -> Result<Frame> {
        let pending = self.pending.as_mut().ok_or("no original encoder request")?;
        let result = (&mut pending.reply).await;
        let index = pending.index;
        self.pending = None;
        let frame = result.map_err(|_| "live encoder owner ended without a reply")??;
        if frame.index != index {
            return Err("live encoder original reply changed".into());
        }
        Ok(frame)
    }

    pub async fn stop_and_join(&mut self) -> Result<Joined> {
        if let Some(joined) = &self.joined {
            return Ok(joined.clone());
        }
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(self.pending.is_some());
        }
        let result = self
            .task
            .as_mut()
            .ok_or("encoder join already failed")?
            .await;
        self.task = None;
        let joined = result??;
        self.pending = None;
        self.joined = Some(joined.clone());
        Ok(joined)
    }
}

#[cfg(test)]
pub(super) fn test_program(frame_count: u32) -> Program {
    Program {
        executable: "/private/.local/encoder".into(),
        executable_sha256: "a".repeat(64),
        raw_path: "/private/.local/original.i420".into(),
        raw_sha256: "b".repeat(64),
        frame_count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    struct Fixture {
        directory: PathBuf,
        program: Program,
    }
    impl Fixture {
        fn new() -> Self {
            assert_eq!(std::env::consts::OS, "linux");
            assert_eq!(std::env::consts::ARCH, "aarch64");
            let directory = std::env::current_dir()
                .unwrap()
                .join(".local/gate-6/replacement-live-encoder")
                .join(format!(
                    "{}-{}",
                    std::process::id(),
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_nanos()
                ));
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
            let executable = PathBuf::from(
                std::env::var_os("CONNECT_LIVE_ENCODER_TEST_BINARY")
                    .expect("runner must build the actual C source"),
            );
            let raw_path = directory.join("original.i420");
            let mut raw = vec![128; RAW_FRAME as usize * 12];
            for index in 0..12 {
                raw[index * RAW_FRAME as usize..index * RAW_FRAME as usize + 640 * 360]
                    .fill(60 + 3 * index as u8);
            }
            std::fs::write(&raw_path, &raw).unwrap();
            std::fs::set_permissions(&raw_path, std::fs::Permissions::from_mode(0o600)).unwrap();
            let program = Program {
                executable_sha256: format!(
                    "{:x}",
                    Sha256::digest(std::fs::read(&executable).unwrap())
                ),
                executable,
                raw_sha256: format!("{:x}", Sha256::digest(&raw)),
                raw_path,
                frame_count: 12,
            };
            Self { directory, program }
        }
        fn gone(&self, pid: u32) {
            assert!(
                !PathBuf::from(format!("/proc/{pid}")).exists(),
                "actual encoder PID remains live"
            );
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }

    #[tokio::test]
    async fn live_encoder_original_reply_survives_control_wait_and_decodes_fresh_frame() {
        let fixture = Fixture::new();
        let mut encoder = Encoder::open(&fixture.program).await.unwrap();
        let pid = encoder.pid;
        encoder
            .request(
                0,
                true,
                tokio::time::Instant::now() + Duration::from_millis(200),
            )
            .unwrap();
        tokio::select! {
            biased;
            _ = std::future::ready(()) => {},
            _ = encoder.receive() => panic!("control should finish before this wait"),
        }
        assert!(encoder.pending());
        assert!(encoder
            .request(
                1,
                false,
                tokio::time::Instant::now() + Duration::from_millis(200)
            )
            .is_err());
        let first = encoder.receive().await.unwrap();
        assert_eq!(first.index, 0);
        assert!(video::key(&first.bytes));
        encoder
            .request(
                7,
                true,
                tokio::time::Instant::now() + Duration::from_millis(200),
            )
            .unwrap();
        let frame = encoder.receive().await.unwrap();
        assert_eq!(frame.index * 3000, 21000);
        assert!(video::key(&frame.bytes));
        assert!(encoder
            .request(
                7,
                false,
                tokio::time::Instant::now() + Duration::from_millis(200)
            )
            .is_err());
        let mut ivf = vec![0; 32];
        ivf[..4].copy_from_slice(b"DKIF");
        ivf[6..8].copy_from_slice(&32u16.to_le_bytes());
        ivf[8..12].copy_from_slice(b"VP80");
        ivf[12..14].copy_from_slice(&640u16.to_le_bytes());
        ivf[14..16].copy_from_slice(&360u16.to_le_bytes());
        ivf[16..20].copy_from_slice(&30u32.to_le_bytes());
        ivf[20..24].copy_from_slice(&1u32.to_le_bytes());
        ivf[24..28].copy_from_slice(&1u32.to_le_bytes());
        ivf.extend((frame.bytes.len() as u32).to_le_bytes());
        ivf.extend(7u64.to_le_bytes());
        ivf.extend(frame.bytes);
        let path = fixture.directory.join("fresh.ivf");
        std::fs::write(&path, ivf).unwrap();
        let decoded = std::process::Command::new("ffmpeg")
            .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-i"])
            .arg(path)
            .args([
                "-vf",
                "crop=16:16:0:0",
                "-pix_fmt",
                "yuv420p",
                "-frames:v",
                "1",
                "-f",
                "rawvideo",
                "pipe:1",
            ])
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(decoded.status.success());
        assert_eq!(decoded.stdout.len(), 384);
        let mean = decoded.stdout[..256]
            .iter()
            .map(|v| f64::from(*v))
            .sum::<f64>()
            / 256.0;
        assert!(
            (mean - 81.0).abs() <= 3.0,
            "independent decoder must recover original frame7 marker"
        );
        let joined = encoder.stop_and_join().await.unwrap();
        assert!(joined.joined && joined.clean && !joined.cancelled);
        fixture.gone(pid);
    }

    #[tokio::test]
    async fn live_encoder_partial_reply_survives_cancelled_wait_without_repeated_command() {
        let fixture = Fixture::new();
        let executable = fixture.directory.join("partial-reply-owner");
        // The controlled child exposes only framing/lifetime behavior. The
        // separate actual-C/FFmpeg case supplies the codec-content proof.
        let script = r#"#!/usr/bin/python3
import os,struct,time
root=os.path.dirname(__file__)
request=os.read(0,8)
assert len(request)==8
index,force=struct.unpack('<II',request)
assert index==7 and force==1
header=struct.pack('<4sIII',b'V8F1',index,10,1)
os.write(1,header[:9])
open(root+'/partial-ready','x').close()
while not os.path.exists(root+'/release-reply'): time.sleep(.001)
os.write(1,header[9:]+bytes([4,0,0,157,1,42,128,2,104,1]))
assert os.read(0,8)==b'', 'original command was reissued'
"#;
        std::fs::write(&executable, script).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut program = fixture.program.clone();
        program.executable = executable;
        program.executable_sha256 = format!("{:x}", Sha256::digest(script));
        let mut encoder = Encoder::open(&program).await.unwrap();
        let pid = encoder.pid;
        let original_deadline = tokio::time::Instant::now() + Duration::from_millis(200);
        encoder.request(7, true, original_deadline).unwrap();
        let partial = fixture.directory.join("partial-ready");
        let control = async {
            tokio::time::timeout(Duration::from_millis(100), async {
                while !tokio::fs::try_exists(&partial).await.unwrap() {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            })
            .await
            .unwrap();
        };
        tokio::select! {
            _ = control => {},
            _ = encoder.receive() => panic!("partial reply must remain pending"),
        }
        assert!(encoder.pending());
        assert!(encoder.request(7, true, original_deadline).is_err());
        tokio::fs::write(fixture.directory.join("release-reply"), [])
            .await
            .unwrap();
        let frame = encoder.receive().await.unwrap();
        assert_eq!(frame.index, 7);
        assert_eq!(frame.bytes, [4, 0, 0, 157, 1, 42, 128, 2, 104, 1]);
        let joined = encoder.stop_and_join().await.unwrap();
        assert!(joined.joined && joined.clean && !joined.cancelled);
        fixture.gone(pid);
    }

    #[tokio::test]
    async fn live_encoder_failed_or_cancelled_reply_joins_actual_process_and_refuses_reissue() {
        let fixture = Fixture::new();
        for mode in ["partial", "foreign", "oversize", "cancel"] {
            let executable = fixture.directory.join(format!("controlled-{mode}"));
            let script = format!("#!/usr/bin/python3\nimport os,struct,time\nr=os.read(0,8)\ni=struct.unpack('<I',r[:4])[0]\nh=struct.pack('<4sIII',b'V8F1',i+{}, {},1)\nos.write(1,h[:{}])\ntime.sleep(30)\n",
                u32::from(mode == "foreign"), if mode == "oversize" { 262145 } else { 10 }, if mode == "partial" { 9 } else { 16 });
            std::fs::write(&executable, &script).unwrap();
            std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
            let mut program = fixture.program.clone();
            program.executable = executable;
            program.executable_sha256 = format!("{:x}", Sha256::digest(script));
            let mut encoder = Encoder::open(&program).await.unwrap();
            let pid = encoder.pid;
            encoder
                .request(
                    0,
                    true,
                    tokio::time::Instant::now() + Duration::from_millis(200),
                )
                .unwrap();
            if mode == "cancel" {
                tokio::time::sleep(Duration::from_millis(30)).await;
            } else {
                assert!(encoder.receive().await.is_err());
            }
            let joined = encoder.stop_and_join().await.unwrap();
            assert!(joined.joined && !joined.clean);
            assert_eq!(joined.cancelled, mode == "cancel");
            fixture.gone(pid);
            assert!(encoder
                .request(
                    1,
                    false,
                    tokio::time::Instant::now() + Duration::from_millis(200)
                )
                .is_err());
        }
        let mut wrong = fixture.program.clone();
        wrong.raw_sha256 = "0".repeat(64);
        assert!(Encoder::open(&wrong).await.is_err());
        let linked = fixture.directory.join("symlink.i420");
        std::os::unix::fs::symlink(&fixture.program.raw_path, &linked).unwrap();
        wrong = fixture.program.clone();
        wrong.raw_path = linked;
        assert!(Encoder::open(&wrong).await.is_err());
    }
}
