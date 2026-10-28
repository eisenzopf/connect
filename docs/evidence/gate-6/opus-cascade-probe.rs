//! Real public codec API regression probe. All input is locally generated PCM.
//! Build against the media-core artifact from a guarded normal-feature build.
//! --audio-control changes only the second encoder's application, at32kbit/s.
use rvoip_media_core::{
    codec::{
        audio::{AudioCodec, OpusApplication, OpusCodec, OpusConfig},
        factory::CodecFactory,
    },
    types::{AudioFrame, SampleRate},
};

fn audio_encoder() -> OpusCodec {
    OpusCodec::new(
        SampleRate::Rate48000,
        1,
        OpusConfig {
            application: OpusApplication::Audio,
            bitrate: 32_000,
            ..OpusConfig::default()
        },
    )
    .unwrap()
}

// Independent radix2 FFT with the Web Audio Blackman window. It does not use
// expected frequency to select a bin, and keeps the original35Hz identity bound.
fn peak(samples: &[i16]) -> f64 {
    let n = samples.len();
    let mut re: Vec<f64> = samples
        .iter()
        .enumerate()
        .map(|(i, &s)| {
            let phase = 2.0 * std::f64::consts::PI * i as f64 / n as f64;
            f64::from(s) * (0.42 - 0.5 * phase.cos() + 0.08 * (2.0 * phase).cos())
        })
        .collect();
    let mut im = vec![0.0; n];
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let theta = -2.0 * std::f64::consts::PI / len as f64;
        let (ci, cr) = theta.sin_cos();
        for start in (0..n).step_by(len) {
            let (mut wr, mut wi) = (1.0, 0.0);
            for k in 0..len / 2 {
                let a = start + k;
                let b = a + len / 2;
                let tr = wr * re[b] - wi * im[b];
                let ti = wr * im[b] + wi * re[b];
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                (wr, wi) = (wr * cr - wi * ci, wr * ci + wi * cr);
            }
        }
        len *= 2;
    }
    let mut best = (f64::NEG_INFINITY, 0.0);
    for k in 1..n / 2 {
        let hz = k as f64 * 48_000.0 / n as f64;
        if (100.0..=3000.0).contains(&hz) {
            let energy = re[k] * re[k] + im[k] * im[k];
            if energy > best.0 {
                best = (energy, hz);
            }
        }
    }
    best.1
}

fn score(samples: &[i16], expected: f64) -> (usize, usize, f64, f64) {
    let (mut windows, mut bad, mut low, mut high) = (0, 0, f64::INFINITY, 0.0f64);
    for at in (0..=samples.len() - 2048).step_by(128) {
        if at < 24_000 {
            continue; // The same explicit0.5s codec startup exclusion as the control grid.
        }
        let hz = peak(&samples[at..at + 2048]);
        windows += 1;
        bad += usize::from((hz - expected).abs() >= 35.0);
        low = low.min(hz);
        high = high.max(hz);
    }
    (windows, bad, low, high)
}

fn main() {
    let control = std::env::args().nth(1).as_deref() == Some("--audio-control");
    let mut results = Vec::new();
    let mut failures = 0;
    for hz in [440.0, 660.0] {
        let mut source = audio_encoder();
        let mut factory: Box<dyn AudioCodec> = if control {
            Box::new(audio_encoder())
        } else {
            CodecFactory::create_negotiated_codec(111, "opus", Some(48_000), Some(1), None).unwrap()
        };
        assert_eq!(factory.get_info().sample_rate, 48_000);
        assert_eq!(factory.get_info().channels, 1);
        assert_eq!(factory.get_info().bitrate, 32_000);
        let mut original = Vec::new();
        let mut output = Vec::new();
        for frame in 0..250 {
            let samples = (0..960)
                .map(|i| {
                    let n = frame * 960 + i;
                    let gain = if (n + 5 * 127) % 48_000 < 4800 {
                        0.7
                    } else {
                        0.05
                    };
                    (32767.0 * gain * (2.0 * std::f64::consts::PI * hz * n as f64 / 48_000.0).sin())
                        as i16
                })
                .collect();
            let first = source
                .encode(&AudioFrame::new(samples, 48_000, 1, 0))
                .unwrap();
            let decoded = source.decode(&first).unwrap();
            assert_eq!(decoded.samples.len(), 960);
            original.extend_from_slice(&decoded.samples);
            let second = factory.encode(&decoded).unwrap();
            let decoded = factory.decode(&second).unwrap();
            assert_eq!(decoded.samples.len(), 960);
            output.extend_from_slice(&decoded.samples);
        }
        let (windows, source_bad, _, _) = score(&original, hz);
        let (_, bad, low, high) = score(&output, hz);
        assert_eq!(
            source_bad, 0,
            "generated source did not preserve its own identity"
        );
        failures += bad;
        results.push(format!(
            "{{\"expected_hz\":{hz},\"windows\":{windows},\"source_bad\":{source_bad},\"cascade_bad\":{bad},\"minimum_hz\":{low},\"maximum_hz\":{high}}}"
        ));
    }
    println!(
        "{{\"scope\":\"Actual codec cascade only; no Core mixing, network or browser playout\",\"audio_control\":{control},\"bitrate\":32000,\"cases\":[{}],\"passed\":{}}}",
        results.join(","),
        failures == 0
    );
    if failures > 0 {
        std::process::exit(1);
    }
}
