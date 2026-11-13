//! Original context-ready protocol only; never sends or preencodes a frame.
use super::*;

fn spawn(
    binary: &Path,
    raw: &Path,
    executable_sha256: String,
) -> std::result::Result<Encoder, String> {
    let mut child = tokio::process::Command::new(binary)
        .arg(raw)
        .arg(FRAMES.to_string())
        .arg("--ready-v1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("original encoder spawn: {e}"))?;
    // Tokio guarantees requested piped handles after successful spawn.
    Ok(Encoder {
        executable: binary.into(),
        executable_sha256,
        input: child.stdin.take().expect("requested original stdin pipe"),
        output: child.stdout.take().expect("requested original stdout pipe"),
        child,
    })
}
async fn await_ready(owners: &mut [Encoder], until: Instant) -> std::result::Result<(), String> {
    for (index, owner) in owners.iter_mut().enumerate() {
        if Instant::now() >= until {
            return Err(format!("original encoder readiness deadline index={index}"));
        }
        let mut marker = [0u8; 4];
        match timeout_at(until, owner.output.read_exact(&mut marker)).await {
            Err(_) => return Err(format!("original encoder readiness deadline index={index}")),
            Ok(Err(e)) => return Err(format!("original encoder readiness IO index={index}: {e}")),
            Ok(Ok(_)) => (),
        }
        if Instant::now() >= until {
            return Err(format!("original encoder readiness deadline index={index}"));
        }
        if &marker != b"V8R1" {
            return Err(format!("original encoder readiness marker index={index}"));
        }
    }
    Ok(())
}
pub(super) async fn prepare(
    binary: &Path,
    raw: &[std::path::PathBuf],
    owners: &mut Vec<Encoder>,
    until: Instant,
) -> std::result::Result<(), String> {
    if Instant::now() >= until {
        return Err("original encoder readiness deadline before hash".into());
    }
    if raw.len() != 6 || !owners.is_empty() {
        return Err("exact six fresh original encoder inputs required".into());
    }
    let executable_sha256 = file_sha256(binary); // Once; BEFORE any child exists.
    for path in raw {
        if Instant::now() >= until {
            return Err("original encoder readiness deadline before spawn".into());
        }
        owners.push(spawn(binary, path, executable_sha256.clone())?);
    }
    await_ready(owners, until).await
}

pub(super) async fn close_all(owners: Vec<Encoder>) -> Vec<std::result::Result<bool, String>> {
    futures_util::future::join_all(owners.into_iter().map(|owner| async move {
        AssertUnwindSafe(finish_encoder(owner))
            .catch_unwind()
            .await
            .map_err(panic_text)
    }))
    .await
}
#[cfg(test)]
fn fake_child(script: &str) -> std::result::Result<Encoder, String> {
    let mut child = tokio::process::Command::new("python3")
        .args(["-B", "-c", script])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("control original spawn: {e}"))?;
    Ok(Encoder {
        executable: "python3".into(),
        executable_sha256: String::new(),
        input: child.stdin.take().expect("requested control stdin"),
        output: child.stdout.take().expect("requested control stdout"),
        child,
    })
}
#[cfg(test)]
async fn control_case(
    scripts: &[&str],
    remaining: Option<Duration>,
    fail_spawn: bool,
) -> std::result::Result<(), String> {
    let mut owners = Vec::new();
    let work = AssertUnwindSafe(async {
        for script in scripts {
            owners.push(fake_child(script)?);
        }
        let until = remaining.map_or_else(
            || Instant::now() - Duration::from_millis(1),
            |remaining| Instant::now() + remaining,
        );
        await_ready(&mut owners, until).await?;
        if fail_spawn {
            owners.push(spawn(
                Path::new("/this-deliberately-absent-original-encoder"),
                Path::new("unused"),
                String::new(),
            )?);
        }
        Ok::<(), String>(())
    })
    .catch_unwind()
    .await;
    let closed = close_all(owners).await;
    if let Err(panic) = work {
        eprintln!("secondary control child cleanup={closed:?}");
        std::panic::resume_unwind(panic);
    }
    assert!(
        closed.iter().all(|v| matches!(v, Ok(true))),
        "every original control must EOF/wait before verdict: {closed:?}"
    );
    work.unwrap()
}
#[cfg(test)]
pub(super) async fn controls() {
    let valid="import sys; sys.stdout.buffer.write(b'V8R1'); sys.stdout.buffer.flush(); sys.stdin.buffer.read()";
    let wrong="import sys; sys.stdout.buffer.write(b'NOPE'); sys.stdout.buffer.flush(); sys.stdin.buffer.read()";
    let eof = "import os,sys; os.close(1); sys.stdin.buffer.read()";
    let blocked = "import sys; sys.stdin.buffer.read()";
    assert!(
        control_case(&[valid, valid], Some(Duration::from_secs(8)), false)
            .await
            .is_ok()
    );
    assert!(
        control_case(&[valid, wrong], Some(Duration::from_secs(8)), false)
            .await
            .unwrap_err()
            .contains("marker index=1")
    );
    assert!(
        control_case(&[valid, eof], Some(Duration::from_secs(8)), false)
            .await
            .unwrap_err()
            .contains("IO index=1")
    );
    assert!(control_case(&[blocked], None, false)
        .await
        .unwrap_err()
        .contains("deadline index=0"));
    // The ready prefix consumes only its own marker; the blocked original then
    // reaches timeout_at under the SAME absolute cutoff, before all EOF joins.
    assert!(
        control_case(&[valid, blocked], Some(Duration::from_millis(200)), false)
            .await
            .unwrap_err()
            .contains("deadline index=1")
    );
    assert!(control_case(&[valid], Some(Duration::from_secs(8)), true)
        .await
        .unwrap_err()
        .contains("original encoder spawn"));
}
