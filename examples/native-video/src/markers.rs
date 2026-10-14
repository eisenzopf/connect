//! Bounded generated-marker code shared by the example's audio and video.
pub const PROGRAM: &str = "connect-av-cadence/1";

pub fn frames() -> Vec<u32> {
    fn visit(at: usize, period: usize, digits: &mut [u8; 19], code: &mut Vec<u8>) {
        if at > 6 {
            if 6 % period == 0 {
                code.extend_from_slice(&digits[1..=period]);
            }
        } else {
            digits[at] = digits[at - period];
            visit(at + 1, period, digits, code);
            for value in digits[at - period] + 1..3 {
                digits[at] = value;
                visit(at + 1, at, digits, code);
            }
        }
    }
    let mut code = Vec::with_capacity(729);
    visit(1, 1, &mut [0; 19], &mut code);
    let mut at = 9;
    let mut frames = Vec::with_capacity(512);
    for value in code {
        if at >= 300 * 30 {
            break;
        }
        frames.push(at);
        at += [15, 27, 39][usize::from(value)];
    }
    assert!(frames.len() <= 512);
    frames
}

pub fn expression(seconds: u64) -> String {
    fn lookup(frames: &[u32]) -> String {
        match frames.len() {
            0 => "0".into(),
            1 => format!(
                "between(t,{:.9},{:.9})",
                f64::from(frames[0]) / 30.0,
                f64::from(frames[0]) / 30.0 + 0.1 - 0.000001
            ),
            count => {
                let middle = count / 2;
                format!(
                    "if(lt(t,{:.9}),{},{})",
                    f64::from(frames[middle]) / 30.0,
                    lookup(&frames[..middle]),
                    lookup(&frames[middle..])
                )
            }
        }
    }
    assert!(seconds <= 300);
    let selected: Vec<_> = frames()
        .into_iter()
        .take_while(|at| u64::from(*at) < seconds * 30)
        .collect();
    // if() evaluates one branch: audio samples perform logarithmic lookup,
    // rather than evaluating hundreds of pulse comparisons every sample.
    lookup(&selected)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn code_is_bounded_and_has_no_repeated_six_interval_word() {
        let frames = frames();
        assert_eq!(&frames[..10], &[9, 24, 39, 54, 69, 84, 99, 126, 141, 156]);
        let intervals: Vec<_> = frames.windows(2).map(|pair| pair[1] - pair[0]).collect();
        let words: std::collections::HashSet<_> = intervals.windows(6).collect();
        assert_eq!(words.len(), intervals.len() - 5);
        assert!(intervals.iter().all(|gap| matches!(gap, 15 | 27 | 39)));
        assert!(expression(300).len() < 65536);
        assert_eq!(PROGRAM, "connect-av-cadence/1");
    }
}
